"""Check native Chromium password CSV import and restart in an owned Flit profile.

Run with --exe PATH/Flit.exe on an interactive Windows desktop. Use --route to
open the same settings page by address on builds without the new menu command.
"""

import argparse
from contextlib import closing, contextmanager
import csv
import ctypes as C
from ctypes import wintypes as W
from hashlib import file_digest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import re
import secrets
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory
import threading
from urllib.parse import urlsplit

sys.dont_write_bytecode = True
from windows_download_close import owned_windows, post
from windows_extensions import root_owner, uia
from windows_multi_session import clean_scratch
from windows_smoke import ADDRESS, Browser, OwnedProcess, WORKSPACE, Win32, api, require
from windows_windows import seed_profile


MANAGER = "chrome://password-manager"
SETTINGS = MANAGER + "/settings"
PASSWORDS, IMPORT_PASSWORDS = 137, 181
HOST, USERNAME = "127.0.0.1", "flit-csv-import-fixture"
FORM_TITLE = "Flit synthetic CSV autofill"
ACTIVATE = "Activate synthetic form"
FORM = f"""<!doctype html><meta charset=utf-8><title>{FORM_TITLE}</title>
<link rel=icon href=data:,>
<form method=post>
  <label>Username <input id=user name=username autocomplete=username></label>
  <label>Password <input id=pw name=password type=password autocomplete=current-password></label>
  <button type=submit>Sign in</button>
</form>
<button type=button>{ACTIVATE}</button>
<script>
const user = document.querySelector('#user'), pw = document.querySelector('#pw');
document.querySelector('form').addEventListener('submit', e => e.preventDefault());
setInterval(() => {{
  const checks = [user.value === {json.dumps(USERNAME)},
                  user.matches(':-webkit-autofill'), pw.matches(':-webkit-autofill')];
  document.title = {json.dumps(FORM_TITLE)} + ' ' + checks.map(Number).join('');
}}, 100);
</script>""".encode("utf-8")


class LoginPage(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reject(self):
        self.send_error(403)

    do_CONNECT = do_POST = reject

    def do_GET(self):
        if self.path not in ("/login", self.server.origin + "/login"):
            return self.reject()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(FORM)))
        self.end_headers()
        self.wfile.write(FORM)


@contextmanager
def fixture():
    with ThreadingHTTPServer((HOST, 0), LoginPage) as site:
        site.origin = f"http://{HOST}:{site.server_port}"
        thread = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.shutdown()
            thread.join()


def navigate(browser, url):
    browser.edit_address(url)
    browser.press(ADDRESS, 0x0D)


def descendants(win, parent):
    for hwnd in win.visible_children(parent):
        yield hwnd
        yield from descendants(win, hwnd)


def find_button(rows, matches):
    buttons = [row for row in rows if row["type"] == "ControlType.Button"
               and row["enabled"] and matches(row)]
    require(len(buttons) <= 1, "Ambiguous visible Chromium button")
    return next(iter(buttons), None)


def click_web_button(browser, button, on_page):
    """Post the click to the owned renderer at the UIA control's center."""
    require(on_page(), "Active page changed before the Chromium button click")
    win, process = browser.win, browser.process
    panel = browser.wait("visible owned page panel", browser.page_panel)
    widgets = [hwnd for hwnd in descendants(win, panel)
               if win.class_name(hwnd) == "Chrome_RenderWidgetHostHWND"]
    require(len(widgets) == 1, "Expected one visible Chromium render widget")
    widget = widgets[0]
    user = C.WinDLL("user32", use_last_error=True)
    is_child = api(user, "IsChild", W.BOOL, W.HWND, W.HWND)
    require(win.pid(browser.hwnd) == win.pid(panel) == win.pid(widget) == process.pid
            and is_child(browser.hwnd, panel) and is_child(panel, widget), "Render widget is not owned")
    rect = button.get("rect")
    require(rect and len(rect) == 4 and min(rect[2:]) > 0, "Chromium button has no visible bounds")
    x, y, width, height = rect
    point = W.POINT(round(x + width / 2), round(y + height / 2))
    screen_to_client = api(user, "ScreenToClient", W.BOOL, W.HWND, C.POINTER(W.POINT))
    with win.window_coordinates(widget):
        win.check(screen_to_client(widget, C.byref(point)))
        _, _, right, bottom = win.rect(widget, client=True)
        require(on_page() and is_child(panel, widget) and win.visible(widget)
                and 0 <= point.x < right and 0 <= point.y < bottom,
                "Chromium button is outside the active owned renderer")
        coords = point.x | (point.y << 16)
        post(process, widget, 0x0201, 1, coords)  # WM_LBUTTONDOWN / MK_LBUTTON
        post(process, widget, 0x0202, 0, coords)  # WM_LBUTTONUP


def pick_file(browser, path):
    win, process = browser.win, browser.process
    control_id = api(C.WinDLL("user32", use_last_error=True), "GetDlgCtrlID", C.c_int, W.HWND)

    def ready():
        dialogs = owned_windows(process, "#32770")
        require(len(dialogs) <= 1, "Expected only the Chromium file picker")
        if not dialogs:
            return None
        dialog = dialogs[0]
        require(root_owner(browser, dialog) == browser.hwnd, "File picker has another root owner")
        button = win.get_item(dialog, 1)  # IDOK / Open.
        if not (button and win.pid(button) == process.pid and win.visible(button)
                and win.class_name(button) == "Button"):
            return None
        edits = [hwnd for hwnd in descendants(win, dialog) if win.pid(hwnd) == process.pid
                 and win.enabled(hwnd) and win.class_name(hwnd) == "Edit"]
        for cid in (1148, 1152):  # Explorer-style filename Edit, including nested shell controls.
            entries = [hwnd for hwnd in edits if control_id(hwnd) == cid]
            if len(entries) == 1:
                return dialog, entries[0], button

    dialog, entry, button = browser.wait("owned native Open dialog and filename field", ready, 15)
    require(win.pid(entry) == process.pid, "File picker filename field is no longer owned")
    value = C.create_unicode_buffer(str(path))
    require(win.message(entry, 0x000C, 0, C.addressof(value)), "File picker refused the fixture path")
    readback = C.create_unicode_buffer(len(value) + 1)
    win.message(entry, 0x000D, len(readback), C.addressof(readback))
    require(readback.value == str(path), "File picker did not read back the fixture path")
    browser.wait("owned Open button enabled", lambda: win.visible(button) and win.enabled(button))
    require(root_owner(browser, dialog) == browser.hwnd, "File picker root owner changed")
    post(process, button, 0x00F5)  # BM_CLICK on the owned Open button.
    browser.wait("owned file picker closed", lambda: not win.visible(dialog))


def verify_entry(browser, report):
    def on_page():
        url = urlsplit(browser.address())
        return url.scheme == "chrome" and url.netloc == "password-manager"

    def list_row():
        if on_page() and browser.page_panel():
            return find_button(uia(browser, browser.hwnd), lambda row: (
                row["id"] == "seePasswordDetails" and HOST in (row["name"] or "")))

    browser.wait("visible password-manager entry for the fixture host", list_row, 20)
    report["host_visible"] = True


def verify_store(profile, login):
    database = (profile / "Default" / "Login Data").resolve(strict=True)
    require(database.is_relative_to(profile), "Password fixture database is outside the disposable profile")
    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db:
        matches, = db.execute(
            "SELECT count(*) FROM logins WHERE origin_url = ? AND username_value = ? "
            "AND length(password_value) > 0", (login, USERNAME),
        ).fetchone()
    require(matches == 1, "Expected one complete synthetic credential in the disposable profile")
    return True


def autofill_probe(browser, url, report):
    def on_page():
        return browser.address() == url

    def observe():
        if on_page():
            match = re.search(re.escape(FORM_TITLE) + r" ([01])([01])([01])$", browser.title())
            if match:
                username, username_autofilled, password_autofilled = (flag == "1" for flag in match.groups())
                report.update(form_observed=True, username_matches=username,
                              username_autofilled=username_autofilled, password_autofilled=password_autofilled)
                return username and username_autofilled and password_autofilled

    navigate(browser, url)
    try:
        browser.wait("synthetic login form to report", lambda: observe() or report["form_observed"], 8)
        if observe():
            return
        button = browser.wait("synthetic form activation button", lambda: on_page() and find_button(
            uia(browser, browser.hwnd), lambda row: row["name"] == ACTIVATE), 5)
        click_web_button(browser, button, on_page)
        browser.wait("autofill after a click without typing", observe, 6)
    except (OSError, RuntimeError, subprocess.TimeoutExpired):
        # Optional: policy or renderer activation can suppress page-observable autofill.
        browser.process.alive()


def run(exe, directory, route, schema, report):
    profile = directory / "profile"
    csv_file = directory / "Flit synthetic passwords.csv"
    with fixture() as site:
        seed_profile(profile, site.origin)  # Fixed, non-forwarding loopback proxy; no existing profile.
        login = site.origin + "/login"
        with csv_file.open("x", encoding="utf-8", newline="") as stream:
            writer = csv.writer(stream)
            password = secrets.token_urlsafe(24)
            if schema == "firefox":
                writer.writerow(("url", "username", "password", "httpRealm", "formActionOrigin", "guid",
                                 "timeCreated", "timeLastUsed", "timePasswordChanged"))
                writer.writerow((login, USERNAME, password, "", site.origin,
                                 "{00000000-0000-4000-8000-000000000021}", "1", "1", "1"))
            else:
                writer.writerow(("name", "url", "username", "password", "note"))
                writer.writerow((HOST, login, USERNAME, password, ""))

        args = [f"--flit-profile={profile}", "--flit-url=about:blank"]
        win = Win32()
        with OwnedProcess(win, exe, args, report["jobs"].setdefault("import", {})) as process:
            browser = Browser(process, site)
            report["stage"] = "open password import settings"
            require(not owned_windows(process, "#32770"), "An owned dialog was already open")
            if route:
                navigate(browser, SETTINGS)
            else:
                post(process, browser.hwnd, 0x0111, IMPORT_PASSWORDS)

            def on_settings():
                return browser.address().rstrip("/") == SETTINGS

            button = browser.wait("visible Select file button on Chromium settings", lambda: (
                on_settings() and browser.page_panel() and find_button(uia(browser, browser.hwnd),
                    lambda row: row["name"] == "Select file" or row["id"] == "selectFileButtonLinkRow")), 20)
            report["stage"] = "select synthetic CSV in native file picker"
            require(not owned_windows(process, "#32770"), "File picker opened before Select file was clicked")
            click_web_button(browser, button, on_settings)
            pick_file(browser, csv_file)
            report["native_file_picker"] = True

            report["stage"] = "verify native import and saved entry"
            view = browser.wait("Chromium import result with View passwords", lambda: on_settings() and find_button(
                uia(browser, browser.hwnd), lambda row: row["id"] == "viewPasswordsButton"
                or row["name"] == "View passwords"), 20)
            click_web_button(browser, view, on_settings)
            verify_entry(browser, report["after_import"])
            report["stage"] = "normal close after import"
            process.normal_close(browser.hwnd)

        report["after_import"]["stored_entry_matches"] = verify_store(profile, login)
        csv_file.unlink(missing_ok=True)

        with OwnedProcess(win, exe, args, report["jobs"].setdefault("restart", {})) as process:
            browser = Browser(process, site)
            report["stage"] = "verify saved entry after restart"
            post(process, browser.hwnd, 0x0111, PASSWORDS)
            verify_entry(browser, report["after_restart"])
            report["stage"] = "observe standard same-origin form autofill"
            autofill_probe(browser, login, report["autofill"])
            report["stage"] = "normal close after restart"
            process.normal_close(browser.hwnd)
        report["after_restart"]["stored_entry_matches"] = verify_store(profile, login)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe beside Flit.dll and libcef.dll")
    parser.add_argument("--work", type=Path, default=WORKSPACE / "work" / "windows-password-import")
    parser.add_argument("--route", action="store_true", help="navigate directly to settings (0.20 baseline)")
    parser.add_argument("--schema", choices=("chromium", "firefox"), default="chromium")
    args = parser.parse_args()
    exe = args.exe.resolve()
    if (sys.platform != "win32" or exe.name.casefold() != "flit.exe"
            or not all(exe.with_name(name).is_file() for name in ("Flit.exe", "Flit.dll", "libcef.dll"))):
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll and libcef.dll")

    report = {"result": "fail", "stage": "setup", "entrypoint": "route" if args.route else "menu 181",
              "schema": args.schema,
              "jobs": {}, "native_file_picker": False,
              "after_import": {"host_visible": False, "stored_entry_matches": False},
              "after_restart": {"host_visible": False, "stored_entry_matches": False},
              "autofill": {"form_observed": False, "username_matches": False,
                           "username_autofilled": False, "password_autofilled": False}}
    try:
        with exe.with_name("Flit.dll").open("rb") as dll:
            report["dll_sha256"] = file_digest(dll, "sha256").hexdigest()
        work = args.work.resolve()
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-password-import-", dir=work)
        try:
            directory = Path(scratch.name).resolve(strict=True)
            require(directory.parent == work, "Unexpected disposable profile parent")
            run(exe, directory, args.route, args.schema, report)
        finally:
            clean_scratch(scratch, report)
        report["stage"] = "scratch cleanup"
        require("scratch_cleanup_error" not in report, "Could not remove the disposable profile and CSV")
        report.update(result="pass", stage="complete")
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
