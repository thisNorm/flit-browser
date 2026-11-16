"""Check that a background beforeunload veto preserves all tabs in an owned Flit window."""

import argparse
from collections import Counter
from contextlib import contextmanager
import ctypes as C
from ctypes import wintypes as W
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sys
import tempfile
import threading
import time
from urllib.parse import unquote

sys.dont_write_bytecode = True
from windows_download_close import owned_windows, post
from windows_downloads import click_attachment, sha256
from windows_session import loaded_panels, select, titles
from windows_smoke import Browser, NEW_TAB, OwnedProcess, WORKSPACE, Win32, api, page_counts, require, until
from windows_windows import seed_profile


SCRATCH = WORKSPACE / "work" / "flit-0.6.0" / "veto-audit"
PATHS = ("/plain", "/protected", "/active")
LABELS = ("Veto audit: plain background", "Veto audit: protected draft", "Veto audit: active tab")
WARNING = "Leave this page? Unsaved changes may be lost."
DRAFT = "flit-veto-unsaved-field"
WM_CLOSE, WM_COMMAND = 0x0010, 0x0111
IDYES, IDNO = 6, 7

PROTECTED = '''
<style>
body { margin:0; height:100vh; display:grid; grid-template-rows:1fr 1fr }
input { position:absolute; top:48%; left:12px }
</style>
<button id="arm" type="button">Enter unsaved draft</button>
<input id="draft" aria-label="Unsaved draft" autocomplete="off">
<button id="read" type="button">Read draft field</button>
<script>
const field = document.querySelector('#draft');
document.querySelector('#arm').addEventListener('click', event => {
  if (!event.isTrusted || !navigator.userActivation.hasBeenActive) return;
  field.value = 'DRAFT_MARKER';
  addEventListener('beforeunload', event => {
    event.preventDefault();
    event.returnValue = true;
  });
  fetch('/armed');
});
document.querySelector('#read').onclick = event => {
  if (event.isTrusted) fetch('/draft?' + encodeURIComponent(field.value));
};
</script>'''.replace("DRAFT_MARKER", DRAFT)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def handle(self):
        try:
            super().handle()
        except ConnectionError:
            pass  # Closing the owned browser can cancel a request.

    def reply(self, status, body=b""):
        self.send_response(status)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_CONNECT(self):
        self.reply(403)

    def do_GET(self):
        site = self.server
        if not self.path.startswith(site.origin + "/"):
            self.reply(403)
            return
        path, _, query = self.path[len(site.origin):].partition("?")
        if path in PATHS:
            title = LABELS[PATHS.index(path)]
            body = (f'<!doctype html><title>{title}</title><link rel="icon" href="data:,">'
                    + (PROTECTED if path == "/protected" else f"<h1>{title}</h1>")
                    + "<script>addEventListener('pageshow', () => requestAnimationFrame(() => "
                      "requestAnimationFrame(() => fetch('/shown' + location.pathname))));</script>").encode("ascii")
        elif path in {"/armed", "/draft", "/favicon.ico", *("/shown" + path for path in PATHS)}:
            body = b""
        else:
            self.reply(404)
            return
        with site.lock:
            site.hits[path] += 1
            if path == "/draft":
                site.drafts.append(unquote(query))
        self.reply(200 if body else 204, body)


class Site(ThreadingHTTPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.origin = f"http://127.0.0.1:{self.server_port}"
        self.lock, self.hits, self.drafts = threading.Lock(), Counter(), []

    def count(self, path):
        with self.lock:
            return self.hits[path]


@contextmanager
def fixture():
    with Site() as site:
        thread = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.shutdown()
            thread.join()


def click_and_observe(browser, index, path):
    click_attachment(browser, index)
    try:
        browser.wait(f"trusted fixture click {path}", lambda: browser.site.count(path), 2)
    except RuntimeError:
        browser.process.alive()
        if not browser.site.count(path):
            click_attachment(browser, index)
            browser.wait(f"trusted fixture click {path} after retry", lambda: browser.site.count(path), 3)


def active_view(browser):
    panel = browser.page_panel()
    if panel and (view := browser.win.find_child(panel, None, None, None)):
        return panel, view


def close_dialog(browser, evidence):
    process, win = browser.process, browser.win
    require(not owned_windows(process, "#32770"), "An owned dialog was already open")
    post(process, browser.hwnd, WM_CLOSE)
    dialogs = browser.wait("owned beforeunload dialog after WM_CLOSE", lambda: owned_windows(process, "#32770"), 8)
    require(len(dialogs) == 1, f"Expected one close dialog, got {dialogs}")
    dialog = dialogs[0]
    get_owner = api(C.WinDLL("user32", use_last_error=True), "GetWindow", W.HWND, W.HWND, W.UINT)
    require(get_owner(dialog, 4) == browser.hwnd, "Close dialog is not owned by the tested Flit window")

    def text(hwnd):
        require(win.pid(hwnd) == process.pid, "Close dialog control is no longer owned")
        value = C.create_unicode_buffer(512)
        win.message(hwnd, 0x000D, len(value), C.addressof(value))  # WM_GETTEXT
        return value.value

    evidence.update(hwnd=dialog, message=" ".join(filter(None, map(text, win.visible_children(dialog, "Static")))))
    require(evidence["message"] == WARNING, f"Unexpected dialog text: {evidence['message']!r}")
    return dialog


def answer(browser, dialog, button_id, evidence):
    win, process = browser.win, browser.process
    require(win.pid(dialog) == process.pid, "Close dialog is no longer owned")
    button = win.get_item(dialog, button_id)
    require(button and win.pid(button) == process.pid and win.class_name(button) == "Button",
            f"Close dialog is missing owned button {button_id}")
    post(process, dialog, WM_COMMAND, button_id, button)  # BN_CLICKED
    evidence["answer"] = "No" if button_id == IDNO else "Yes"


def run(exe, profile, report):
    with fixture() as site:
        seed_profile(profile, site.origin)
        launch = report["launch"] = {}
        args = [f"--flit-profile={profile}", f"--flit-url={site.origin}{PATHS[0]}"]
        try:
            with OwnedProcess(Win32(), exe, args, launch) as process:
                browser = Browser(process, site)
                win = browser.win
                views = report["views_before_close"] = []
                for index, path in enumerate(PATHS):
                    if index:
                        browser.click(NEW_TAB)
                        browser.wait(f"tab {index} selected", lambda: browser.selected() == index)
                        browser.navigate(path)
                    else:
                        browser.page(path, require_request=True)
                    identity = browser.wait(f"visible CEF view for {path}", lambda: active_view(browser))
                    require(win.pid(identity[1]) in process.pids() and identity not in views,
                            f"Missing distinct owned CEF view for {path}")
                    views.append(identity)
                    if path == "/protected":
                        click_and_observe(browser, 0, "/armed")

                browser.wait("three loaded tab rows with the third selected",
                             lambda: titles(browser) == LABELS and browser.selected() == 2)
                pids = process.pids()
                require(all(win.pid(view) in pids for _, view in views), "A tab view closed before WM_CLOSE")
                report["rows_before_close"] = titles(browser)
                report["page_counts_before_close"] = before = {path: page_counts(site, path) for path in PATHS}

                veto = report["veto"] = {}
                dialog = close_dialog(browser, veto)
                veto["loaded_panels_at_prompt"] = loaded_panels(browser)
                veto["views_at_prompt"] = [(panel, win.find_child(panel, None, None, None)) for panel, _ in views]
                require(veto["loaded_panels_at_prompt"] == len(PATHS) and veto["views_at_prompt"] == views,
                        "The beforeunload prompt appeared after a loaded tab view was lost or replaced")
                answer(browser, dialog, IDNO, veto)
                browser.wait("No dismissed the dialog and kept the Flit window enabled", lambda: (
                    not win.visible(dialog) and win.pid(browser.hwnd) == process.pid and win.enabled(browser.hwnd)
                ))
                report["rows_after_no"] = rows = titles(browser)
                require(rows == LABELS, f"No lost or reordered tab rows: {rows!r}")
                report["loaded_panels_after_no"] = loaded_panels(browser)
                require(report["loaded_panels_after_no"] == len(PATHS), "No lost a loaded tab view")
                for index, (path, label) in enumerate(zip(PATHS, LABELS)):
                    select(browser, index)
                    browser.wait(f"{path} selected in its original CEF view", lambda: (
                        browser.address() == site.origin + path and browser.has_page_title(label)
                        and active_view(browser) == views[index]
                    ))
                    if path == "/protected":
                        click_and_observe(browser, 1, "/draft")
                with site.lock:
                    report["draft_readback"] = draft = site.drafts[0]
                require(draft == DRAFT, f"No lost the protected draft: {draft!r}")
                report["page_counts_after_no"] = after = {path: page_counts(site, path) for path in PATHS}
                require(after == before, f"No reloaded a fixture page: {before!r} -> {after!r}")
                require(titles(browser) == LABELS, "Selecting the retained rows changed the tab list")

                confirm = report["confirm"] = {}
                answer(browser, close_dialog(browser, confirm), IDYES, confirm)
                until("exit after approving the second WM_CLOSE", process.exited, 20)
                launch["exit_code"] = code = win.native.GetExitCodeProcess(process.process)
                until("empty job after Yes", lambda: not process.pids(), 8)
                require(code == 0, f"Flit exited with 0x{code:08x} after Yes")
                session = report["session_after_yes"] = json.loads((profile / "session.json").read_text(encoding="utf-8"))
                entries = [{"url": site.origin + path, "title": label, "pinned": False}
                           for path, label in zip(PATHS, LABELS)]
                require(isinstance(session, dict) and session.get("entries") == entries
                        and session.get("selected") == len(PATHS) - 1 and not session.get("additional_windows"),
                        f"Clean close after Yes did not save the original tabs and selection: {session!r}")
        finally:
            with site.lock:
                report["requests"] = dict(site.hits)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, default=SCRATCH)
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires a staged Windows Flit.exe with its sibling Flit.dll")
    if not work.is_relative_to(SCRATCH):
        parser.error(f"--work must be inside {SCRATCH}")
    report = {"result": "fail", "exe": str(exe), "dll_sha256": sha256(dll)}
    try:
        work.mkdir(parents=True, exist_ok=True)
        scratch = tempfile.TemporaryDirectory(prefix="flit-close-cancel-", dir=work)
        try:
            run(exe, Path(scratch.name) / "profile", report)
        finally:
            try:
                try:
                    scratch.cleanup()
                except PermissionError:
                    report["scratch_cleanup_retried"] = True
                    time.sleep(.25)
                    scratch.cleanup()
            except OSError as error:
                report["scratch_cleanup_error"] = str(error)
                report["scratch_retained"] = scratch.name
        require("scratch_cleanup_error" not in report, "Could not remove the isolated scratch profile")
        require(sha256(dll) == report["dll_sha256"], "Staged DLL changed during the test")
        report["result"] = "pass"
    except (RuntimeError, OSError, ValueError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
