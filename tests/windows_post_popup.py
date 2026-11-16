"""Verify real POST forms keep their method and body when targeting a new surface."""

import argparse
from http.server import BaseHTTPRequestHandler
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import time

sys.dont_write_bytecode = True
from windows_downloads import click_attachment, seed_profile, sha256
from windows_multi_session import clean_scratch
from windows_popup import top_levels
from windows_session import select, titles
from windows_smoke import Browser, OwnedProcess, WORKSPACE, Win32, fixture, require


KINDS = ("self", "blank")
FORMS = {f"/form/{kind}": kind for kind in KINDS}
RESULTS = {f"/result/{kind}" for kind in KINDS}
EVENTS = {
    route for kind in KINDS for route in
    (f"/shown/form/{kind}", f"/shown/result/{kind}", f"/trusted/{kind}", f"/wrong-context/result/{kind}")
}
# Keep the trusted native submit from work/flit-0.30.0/followup-audit/check-post-target.py.
FORM = '''<!doctype html><title>Form audit {kind}</title>
<link rel="icon" href="data:,">
<style>body {{margin:0}} button {{width:100vw;height:100vh}}</style>
<form action="/result/{kind}" method="post" target="_{kind}">
<input type="hidden" name="demo" value="fixture"><button>Send test value</button></form>
<script>
document.querySelector('button').addEventListener('click', event => {{
    if (event.isTrusted && navigator.userActivation.isActive)
        fetch('/trusted/{kind}', {{keepalive: true}});
}});
addEventListener('pageshow', () => fetch('/shown' + location.pathname));
</script>'''
RESULT_TITLE = "POST received"
RESULT = '''<!doctype html><title>POST received</title><h1>OK</h1>
<link rel="icon" href="data:,">
<script>
addEventListener('pageshow', () => {
    const path = window === window.top && window.opener === null ? '/shown' : '/wrong-context';
    fetch(path + location.pathname);
});
</script>'''


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def handle(self):
        try:
            super().handle()
        except ConnectionError:
            pass

    def respond(self, status, body=""):
        payload = body.encode("ascii")
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if payload:
            self.wfile.write(payload)

    def do_CONNECT(self):
        self.respond(403)

    def route(self):
        # The profile uses this fixture as a fixed proxy. Nothing is forwarded.
        origin = self.server.origin
        return self.path[len(origin):] if self.path.startswith(origin + "/") else ""

    def do_GET(self):
        route = self.route()
        path = route.partition("?")[0]
        if path in RESULTS:
            with self.server.lock:
                self.server.hits[f"GET {path}"] += 1
            self.respond(405, "<!doctype html><title>Method not allowed</title><h1>POST is required</h1>")
        elif route in FORMS or route in EVENTS:
            with self.server.lock:
                self.server.hits[route] += 1
            if route in FORMS:
                self.respond(200, FORM.format(kind=FORMS[route]))
            else:
                self.respond(204)
        else:
            self.respond(403)

    def do_POST(self):
        route = self.route()
        path = route.partition("?")[0]
        if path not in RESULTS:
            self.respond(403)
            return
        expected = b"demo=fixture"
        valid = False
        try:
            if route == path and self.headers.get("Content-Length") == str(len(expected)):
                valid = self.rfile.read(len(expected)) == expected
        finally:
            with self.server.lock:
                self.server.hits[f"POST {path}"] += 1
                if valid:
                    self.server.hits[f"valid body {path}"] += 1
        if valid:
            self.respond(200, RESULT)
        else:
            self.respond(400)


def require_post(hits, kind):
    path = f"/result/{kind}"
    post, valid, get = (hits.get(f"{key} {path}", 0) for key in ("POST", "valid body", "GET"))
    require((post, valid, get) == (1, 1, 0),
            f"{kind}: expected one POST with demo=fixture and no GET; "
            f"got POST={post}, matching body={valid}, GET={get}")
    require(hits.get(f"/trusted/{kind}", 0) == 1, f"{kind}: expected exactly one trusted submit click")


def visible_page(browser, path, title):
    return browser.address() == browser.site.origin + path and browser.has_page_title(title) and browser.page_panel()


def new_target(browser, old_windows, old_tabs):
    process, win = browser.process, browser.win
    current = top_levels(process)
    added = current - old_windows
    tab_delta = browser.tab_count() - old_tabs
    require(old_windows <= current and tab_delta in (0, 1) and len(added) + tab_delta <= 1,
            "The submit replaced an owned surface or created multiple destinations")
    if added:
        hwnd, = added
        name, pid = win.class_name(hwnd), win.pid(hwnd)
        require(name.startswith("Chrome_WidgetWin_") and pid in process.pids(),
                f"New HWND 0x{hwnd:x} is not an owned native Chromium window: {name!r}")
        return {"kind": "native_popup", "hwnd": hwnd, "class": name, "pid": pid}
    if tab_delta:
        rows = titles(browser)
        if RESULT_TITLE in rows:
            return {"kind": "sidebar_tab", "index": rows.index(RESULT_TITLE)}


def exercise(browser, report):
    site, process = browser.site, browser.process
    for kind in KINDS:
        form, result = f"/form/{kind}", f"/result/{kind}"
        form_title = f"Form audit {kind}"
        browser.navigate(form)
        browser.wait("form title and page view", lambda: visible_page(browser, form, form_title))
        source, tabs, windows = browser.selected(), browser.tab_count(), top_levels(process)

        def submitted():
            return site.count(f"/trusted/{kind}") or site.count(f"POST {result}") or site.count(f"GET {result}")

        click_attachment(browser, 0)  # The audited form's button covers the viewport.
        retry_at = time.monotonic() + 2
        browser.wait("submit or input retry deadline", lambda: submitted() or time.monotonic() >= retry_at, 3)
        if not submitted() and visible_page(browser, form, form_title):
            click_attachment(browser, 0)
        browser.wait(f"trusted submit ({kind})", lambda: site.count(f"/trusted/{kind}"))
        browser.wait(f"result request ({kind})", lambda: site.count(f"POST {result}") or site.count(f"GET {result}"))
        require_post(site.report()["requests"], kind)

        def shown():
            require(not site.count(f"/wrong-context{result}"), f"{kind}: result is framed or has a window.opener")
            return site.count(f"/shown{result}")

        browser.wait(f"rendered top-level POST result ({kind})", shown)
        if kind == "self":
            browser.wait("same-tab POST result", lambda: visible_page(browser, result, RESULT_TITLE))
            require(browser.selected() == source and browser.tab_count() == tabs and top_levels(process) == windows,
                    "The same-tab control opened or replaced a surface")
            continue

        target = report["destination"] = browser.wait("new POST destination", lambda: new_target(browser, windows, tabs))
        if target["kind"] == "sidebar_tab":
            select(browser, target["index"])
            browser.wait("sidebar POST result", lambda: visible_page(browser, result, RESULT_TITLE))
            rows = titles(browser)
            require(rows.count(form_title) == 1, "The source tab was lost or duplicated")
            select(browser, rows.index(form_title))
        browser.wait("original form still present", lambda: visible_page(browser, form, form_title))

    process.normal_close(browser.hwnd)
    hits = site.report()["requests"]
    for kind in KINDS:
        require_post(hits, kind)
    require(hits.get("/form/blank") == hits.get("/shown/form/blank") == 1,
            "The source form did not stay loaded after the new-target submit")


def run(exe, scratch, report):
    with fixture() as site:
        site.RequestHandlerClass = Handler
        profile = scratch / "profile"
        seed_profile(profile, scratch / "downloads", site.origin)
        try:
            with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}"], report["launch"]) as process:
                exercise(Browser(process, site), report)
        finally:
            report["requests"] = site.report()["requests"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="parent for the disposable profile")
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")
    if not work.is_relative_to(WORKSPACE):
        parser.error(f"--work must be inside {WORKSPACE}")

    report = {"result": "fail", "exe": str(exe), "launch": {}}
    try:
        report["dll_sha256"] = sha256(dll)
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-post-popup-", dir=work)
        try:
            run(exe, Path(scratch.name), report)
        finally:
            clean_scratch(scratch, report)
        require("scratch_cleanup_error" not in report, "Disposable profile cleanup failed")
        require(sha256(dll) == report["dll_sha256"], "Staged DLL changed during the test")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
