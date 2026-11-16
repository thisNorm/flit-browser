"""Check sign-in-style native CEF popups in owned normal and private Flit windows."""

import argparse
from collections import Counter
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sys
import tempfile
import threading
import time

sys.dont_write_bytecode = True
from windows_downloads import click_attachment, sha256
from windows_smoke import GO, OwnedProcess, WORKSPACE, Win32, require
from windows_windows import PRIVATE_WINDOW, Window, seed_profile


TITLE = "Flit popup fixture"
OPENER = b'''<!doctype html><title>Flit popup fixture</title>
<link rel="icon" href="data:,">
<style>body { margin:0 } button { width:100vw; height:100vh }</style>
<button type="button">Open synthetic sign-in</button>
<script>
const features = 'popup,width=420,height=360';
let popup;
addEventListener('message', event => {
  if (event.origin === location.origin && event.source === popup &&
      event.data === 'flit-synthetic-sign-in') fetch('/received');
});
document.querySelector('button').onclick = event => {
  if (!event.isTrusted || !navigator.userActivation.isActive) {
    fetch('/untrusted-click');
    return;
  }
  if (popup && !popup.closed) return;
  fetch('/clicked');
  popup = window.open('/popup', 'flit-sign-in', features);
  fetch(popup ? '/gesture-opened' : '/gesture-blocked');
};
const activated = navigator.userActivation.isActive;
const scriptPopup = window.open('/blocked', 'flit-script-popup', features);
fetch(!activated && (scriptPopup === null || scriptPopup.closed)
      ? '/script-blocked' : '/script-not-blocked');
addEventListener('pageshow', () => {
  requestAnimationFrame(() => requestAnimationFrame(() => fetch('/ready')));
});
</script>'''
POPUP = b'''<!doctype html><title>Flit synthetic sign-in</title>
<link rel="icon" href="data:,">
<script>
if (window.opener) {
  window.opener.postMessage('flit-synthetic-sign-in', location.origin);
  fetch('/release').then(response => { if (response.status === 204) window.close(); });
} else {
  fetch('/missing-opener');
}
</script>'''
PAGES = {"/": OPENER, "/popup": POPUP, "/blocked": b"<!doctype html><title>Unexpected popup</title>"}
ERRORS = {
    "/script-not-blocked": "The load-time window.open did not report being blocked without activation",
    "/blocked": "The load-time window.open navigated to its target",
    "/untrusted-click": "The CEF button click had no trusted user activation",
    "/gesture-blocked": "The trusted window.open returned null",
    "/missing-opener": "The popup loaded with no window.opener",
}
EVENTS = {"/ready", "/script-blocked", "/clicked", "/gesture-opened", "/received", "/release", "/favicon.ico", *ERRORS}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def handle(self):
        try:
            super().handle()
        except ConnectionError:
            pass  # Closing the browser may cancel a pending fixture request.

    def reject(self):
        self.send_error(403)

    do_CONNECT = reject

    def do_GET(self):
        site = self.server
        # Use the seeded loopback proxy for both contexts; never forward requests.
        if not self.path.startswith(site.origin + "/"):
            self.reject()
            return
        route = self.path[len(site.origin):]
        if route not in PAGES and route not in EVENTS:
            self.send_error(404)
            return
        with site.lock:
            site.hits[route] += 1
        if route == "/release" and not site.release.wait(45):
            self.send_error(409)
            return
        body = PAGES.get(route, b"")
        self.send_response(200 if body else 204)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)


@contextmanager
def fixture():
    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as site:
        site.origin = f"http://127.0.0.1:{site.server_port}"
        site.hits, site.lock, site.release = Counter(), threading.Lock(), threading.Event()
        thread = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.release.set()
            site.shutdown()
            thread.join()


def top_levels(process):
    win, found, pids = process.win, set(), set(process.pids())

    @win.callback
    def visit(hwnd, _):
        if win.visible(hwnd) and win.pid(hwnd) in pids:
            found.add(hwnd)
        return True

    win.check(win.enum(visit, 0))
    return found


def check_requests(site):
    with site.lock:
        hits = site.hits.copy()
    for route, error in ERRORS.items():
        require(not hits[route], error)
    return hits


def check_opener(browser):
    require(browser.tab_count() == 1 and browser.selected() == 0
            and browser.address() == browser.site.origin + "/" and browser.has_page_title(TITLE),
            "The popup attempt changed the opener's tab or page")


def open_popup(browser, before, attempt, evidence):
    process, site, win = browser.process, browser.site, browser.win
    evidence["clicks_posted"] = 1
    click_attachment(browser, 0)  # The only button covers the whole viewport.

    def clicked():
        return check_requests(site)["/clicked"] >= attempt

    try:
        browser.wait("trusted click in opener DOM", clicked, 2)
    except RuntimeError:
        process.alive()
        if not clicked():
            # A newly shown render widget can miss its first posted mouse message.
            evidence["clicks_posted"] += 1
            click_attachment(browser, 0)
            browser.wait("trusted click in opener DOM after retry", clicked, 3)

    def opened():
        hits = check_requests(site)
        current = top_levels(process)
        new = current - before
        require(before <= current and len(new) <= 1, "Unexpected owned windows after clicking")
        check_opener(browser)
        if new and all(hits[route] == attempt for route in ("/popup", "/gesture-opened", "/received", "/release")):
            return next(iter(new))

    hwnd = browser.wait("native popup and sign-in marker from its WindowProxy", opened)
    name, pid = win.class_name(hwnd), win.pid(hwnd)
    require(name.startswith("Chrome_WidgetWin_") and pid in process.pids(),
            f"New HWND 0x{hwnd:x} is not an owned Chrome popup: {name!r}")
    evidence.update({"hwnd": hwnd, "class": name, "pid": pid})


def run(exe, scratch, private, report):
    launch = report["launch"] = {}
    with fixture() as site:
        profile = scratch / "profile"
        seed_profile(profile, site.origin)
        win = Win32()
        try:
            with OwnedProcess(win, exe, [f"--flit-profile={profile}"], launch) as process:
                normal = Window(process, site)
                normal.blank()
                browser = normal.open_window(PRIVATE_WINDOW, "private_window", report) if private else normal
                if private:
                    browser.blank(private=True)
                before = top_levels(process)
                report["opener_hwnd"] = browser.hwnd
                browser.edit_address(site.origin + "/")
                browser.click(GO)

                def ready():
                    hits = check_requests(site)
                    return hits["/ready"] and hits["/script-blocked"] and browser.has_page_title(TITLE) and browser.page_panel()

                browser.wait("load-time popup rejection and visible opener page", ready)
                check_opener(browser)
                require(top_levels(process) == before and not check_requests(site)["/popup"],
                        "A popup appeared before the click")
                report["script_popup_blocked"] = True

                first = report["self_close"] = {}
                open_popup(browser, before, 1, first)
                site.release.set()
                browser.wait("popup window.close() destroyed its HWND", lambda: win.pid(first["hwnd"]) != first["pid"])
                require(top_levels(process) == before, "Self-close changed another owned window")
                check_opener(browser)
                first["closed"] = True

                site.release.clear()
                second = report["parent_close"] = {}
                open_popup(browser, before, 2, second)
                if private:
                    browser.close_window()
                    normal.wait("parent WM_CLOSE destroyed its popup", lambda: win.pid(second["hwnd"]) != second["pid"])
                    require(top_levels(process) == before - {browser.hwnd}, "Closing the private opener affected another window")
                    normal.blank()
                process.normal_close(normal.hwnd)
                require(win.pid(second["hwnd"]) != second["pid"], "Popup survived closing its opener")
                second["closed"] = True
                check_requests(site)
        finally:
            with site.lock:
                report["requests"] = dict(site.hits)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires a staged Windows Flit.exe with its sibling Flit.dll")
    if not work.is_relative_to(WORKSPACE):
        parser.error(f"--work must be inside {WORKSPACE}")
    work.mkdir(parents=True, exist_ok=True)
    report = {"result": "pass", "exe": str(exe), "dll_sha256": sha256(dll)}
    for mode in ("normal", "private"):
        case = report[mode] = {"result": "fail"}
        try:
            scratch = tempfile.TemporaryDirectory(prefix="flit-popup-", dir=work)
            try:
                run(exe, Path(scratch.name), mode == "private", case)
            finally:
                try:
                    try:
                        scratch.cleanup()
                    except PermissionError:
                        # Windows can still report a sharing violation just after forced job teardown.
                        case["scratch_cleanup_retried"] = True
                        time.sleep(.25)
                        scratch.cleanup()
                except OSError as error:
                    case["scratch_cleanup_error"] = f"{type(error).__name__}: {error}"
                    case["scratch_retained"] = scratch.name
            require(sha256(dll) == report["dll_sha256"], "Staged DLL changed during the test")
            require("scratch_cleanup_error" not in case, "Could not remove the isolated scratch profile")
            case["result"] = "pass"
        except (RuntimeError, OSError) as error:
            report["result"] = "fail"
            case["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
