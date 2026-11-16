"""Exercise Flit search on Windows using an owned instance and a non-forwarding proxy."""

import argparse
from collections import Counter
from contextlib import contextmanager
import ctypes as C
from ctypes import wintypes as W
from hashlib import file_digest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sys
import tempfile
import threading

sys.dont_write_bytecode = True
from windows_multi_session import clean_scratch
from windows_shortcuts import GuiThreadInfo, selection
from windows_smoke import ADDRESS, OwnedProcess, Win32, api, require
from windows_windows import PRIVATE_WINDOW, Window, seed_profile


FOCUS_ADDRESS, FOCUS_SEARCH = 113, 141
ENTER, ESCAPE = 0x0D, 0x1B
ENGINES = {
    176: ("duckduckgo", "duckduckgo.com"),
    177: ("google", "www.google.com"),
    178: ("naver", "search.naver.com"),
    179: ("bing", "www.bing.com"),
}
TERM = "example.test"
WATCHED = {host for _, host in ENGINES.values()} | {TERM}
PAGES = {path: "Flit search fixture " + path for path in ("/start", "/normal")}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def respond(self, status, body=b""):
        try:
            self.send_response(status)
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if body:
                self.wfile.write(body)
        except ConnectionError:
            pass  # Chromium can abandon a connection when its tab or window changes.

    def do_CONNECT(self):
        # Deny the tunnel before TLS starts. Record only the hostname, never a query.
        host = self.path.partition(":")[0].lower()
        if host in WATCHED:
            with self.server.lock:
                self.server.connects[host] += 1
        self.respond(403)

    def do_GET(self):
        site = self.server
        path = self.path.removeprefix(site.origin)
        if self.path != site.origin + path or path not in PAGES:
            self.respond(403)
            return
        with site.lock:
            site.pages[path] += 1
        self.respond(200, f'<!doctype html><title>{PAGES[path]}</title><link rel="icon" href="data:,">'.encode())


class Proxy(ThreadingHTTPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.origin = f"http://127.0.0.1:{self.server_port}"
        self.pages, self.connects = Counter(), Counter()
        self.lock = threading.Lock()

    def get_request(self):
        conn, address = super().get_request()
        conn.settimeout(3)
        return conn, address

    def snapshot(self):
        with self.lock:
            return self.pages.copy(), self.connects.copy()


@contextmanager
def proxy():
    with Proxy() as site:
        thread = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.shutdown()
            thread.join()


def address_is(browser, expected, selected=None):
    def matches():
        return browser.address() == expected and (selected is None or selection(browser) == selected)

    browser.wait(f"address {expected!r}" + (f" with selection {selected}" if selected else ""), matches, 4)


def search_mode(browser):
    browser.command(FOCUS_SEARCH)
    address_is(browser, "? ", (2, 2))
    # Read the owned GUI thread, independently of which application is foreground.
    info = GuiThreadInfo(C.sizeof(GuiThreadInfo))
    tid = browser.win.get_pid(browser.hwnd, None)
    get_info = api(C.WinDLL("user32", use_last_error=True), "GetGUIThreadInfo", W.BOOL, W.DWORD, C.POINTER(GuiThreadInfo))
    browser.win.check(tid and get_info(tid, C.byref(info)))
    require(info.focus == browser.controls[ADDRESS], "WM_COMMAND 141 did not focus the address edit")


def page(browser, path, before, *, submit=False):
    url = browser.site.origin + path
    if submit:
        browser.edit_address(url)
        browser.press(ADDRESS, ENTER)
    browser.wait(f"rendered loopback URL {path}", lambda: (
        browser.site.snapshot()[0][path] > before[path]
        and browser.has_page_title(PAGES[path]) and browser.address() == url
    ))


def submit_search(browser, command, before, report, label):
    _, host = ENGINES[command]
    # Observe a bounded quiet interval while the draft is still uncommitted.
    require(not threading.Event().wait(.6) and browser.site.snapshot() == before,
            f"{label}: navigation before Enter")
    browser.press(ADDRESS, ENTER)
    started = browser.wait(f"{label}: CONNECT to {host}", lambda: (
        (hits := browser.site.snapshot()[1] - before[1]) or None
    ))
    require(set(started) == {host}, f"{label}: expected {host}, got {dict(started)}")
    report[label] = {"before_enter_quiet_ms": 600, "connect_hosts": dict(started)}


def saved_engine(profile, command):
    state = json.loads((profile / "session.json").read_text(encoding="utf-8"))
    actual = state.get("search_engine", "duckduckgo")
    require(actual == ENGINES[command][0], f"Unexpected persisted search_engine: {actual!r}")
    return actual


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    scratch = tempfile.TemporaryDirectory(prefix="flit-search-", dir=work)
    report["scratch"] = scratch.name
    try:
        exercise(exe, Path(scratch.name) / "profile", report)
    finally:
        clean_scratch(scratch, report)
    require("scratch_cleanup_error" not in report, "Disposable test profile was not removed")


def exercise(exe, profile, report):
    win = Win32()
    report.update(launches={}, checks={}, session={})
    checks = report["checks"]
    with proxy() as site:
        seed_profile(profile, site.origin)
        args = [f"--flit-profile={profile}"]

        @contextmanager
        def launch(label, *extra):
            evidence = report["launches"][label] = {}
            with OwnedProcess(win, exe, [*args, *extra], evidence) as process:
                browser = Window(process, site)
                yield browser
                process.normal_close(browser.hwnd)

        try:
            before = site.snapshot()[0]
            with launch("fresh", f"--flit-url={site.origin}/start") as browser:
                page(browser, "/start", before)
                current = site.origin + "/start"
                before = site.snapshot()

                draft = site.origin + "/normal"
                browser.edit_address(draft)
                browser.command(FOCUS_ADDRESS)
                address_is(browser, draft, (0, len(draft)))
                browser.edit_address("? " + TERM)
                browser.command(FOCUS_ADDRESS)
                address_is(browser, current, (0, len(current)))
                search_mode(browser)
                browser.edit_address("? " + TERM)
                browser.press(ADDRESS, ESCAPE)
                address_is(browser, current)
                page(browser, "/normal", before[0], submit=True)
                require(site.snapshot()[1] == before[1], "Address or cancel commands started a search")
                checks.update(ctrl_l_preserved_draft=True, ctrl_l_reset_search=True, esc_restored_url=True,
                              normal_url_rendered=True)

                before = site.snapshot()
                search_mode(browser)
                browser.edit_address("? discarded draft")
                search_mode(browser)
                browser.edit_address("? " + TERM)
                submit_search(browser, 176, before, checks, "fresh_default_after_repeated_141")

                # Leave a loopback page in the saved session so restart cannot replay a search.
                page(browser, "/normal", site.snapshot()[0], submit=True)
                browser.command(178)
            report["session"]["after_selection"] = saved_engine(profile, 178)

            before = site.snapshot()[0]
            with launch("restored") as browser:
                page(browser, "/normal", before)
                before = site.snapshot()
                browser.command(FOCUS_ADDRESS)
                browser.edit_address("? " + TERM)  # Explicit search without WM_COMMAND 141.
                submit_search(browser, 178, before, checks, "restored_direct_prefix")

                private = browser.open_window(PRIVATE_WINDOW, "private", report["launches"]["restored"])
                private.blank(private=True)
                before = site.snapshot()
                private.command(179)  # Private must ignore the attempt to change the shared preference.
                search_mode(private)
                private.edit_address("? " + TERM)
                submit_search(private, 178, before, checks, "private_after_bing_command")
                private.close_window()
                page(browser, "/normal", site.snapshot()[0], submit=True)
            report["session"]["after_private"] = saved_engine(profile, 178)
        finally:
            pages, connects = site.snapshot()
            report.update(proxy_pages=dict(pages), proxy_connect_hosts=dict(connects))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe alongside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="parent for a disposable test profile")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    exe = args.exe.resolve()
    dll = exe.with_name("Flit.dll")
    if exe.name.casefold() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("--exe must point to Flit.exe alongside Flit.dll")

    def hashes():
        result = {}
        for path in (exe, dll):
            with path.open("rb") as stream:
                result[path.name] = file_digest(stream, "sha256").hexdigest()
        return result

    report = {"result": "fail", "exe": str(exe)}
    try:
        report["sha256"] = hashes()
        try:
            run(exe, args.work.resolve(), report)
        finally:
            report["sha256_after"] = hashes()
            if "scratch" in report:
                report["scratch_removed"] = not Path(report["scratch"]).exists()
        require(report["sha256"] == report["sha256_after"], "Staged Flit.exe or Flit.dll changed during the test")
        require(report["scratch_removed"], "Disposable test profile was not removed")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
