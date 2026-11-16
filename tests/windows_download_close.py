"""Check Flit's close warning with a held loopback download and a fresh profile."""

import argparse
from collections import Counter
from contextlib import closing, contextmanager
import ctypes as C
from ctypes import wintypes as W
import hashlib
from http.server import ThreadingHTTPServer
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import time

sys.dont_write_bytecode = True
from windows_downloads import ATTACHMENTS, Handler, TITLE, click_attachment, seed_profile, sha256
from windows_smoke import Browser, GO, NEW_TAB, OwnedProcess, Win32, api, require, until


CHUNK = b"Flit download close fixture\n" * (1 << 15)
ROUTE, NAME, _ = ATTACHMENTS[0]
WARNING = "Download in progress"
IDNO = 7


class HeldAttachment(Handler):
    def do_GET(self):
        if self.path != self.server.origin + ROUTE:
            return super().do_GET()
        self.server.hits[ROUTE] += 1
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Disposition", f'attachment; filename="{NAME}"')
        self.send_header("Content-Length", str(2 * len(CHUNK)))
        self.end_headers()
        try:
            self.wfile.write(CHUNK)
            if self.server.release.wait(45):
                self.wfile.write(CHUNK)
        except (ConnectionResetError, ConnectionAbortedError, BrokenPipeError):
            pass


@contextmanager
def fixture():
    with ThreadingHTTPServer(("127.0.0.1", 0), HeldAttachment) as site:
        site.origin = f"http://127.0.0.1:{site.server_port}"
        site.hits, site.release = Counter(), threading.Event()
        thread = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.release.set()
            site.shutdown()
            thread.join()


def owned_windows(process, class_name):
    win, result = process.win, []

    @win.callback
    def visit(hwnd, _):
        if win.pid(hwnd) == process.pid and win.visible(hwnd) and win.class_name(hwnd) == class_name:
            result.append(hwnd)
        return True

    win.check(win.enum(visit, 0))
    return result


def post(process, hwnd, msg, wparam=0, lparam=0):
    require(process.win.pid(hwnd) == process.pid, "Win32 target is no longer owned")
    process.win.check(process.win.post(hwnd, msg, wparam, lparam))


def decline_close(browser, target):
    process, win = browser.process, browser.win
    dialog = browser.wait(f"{target} download warning", lambda: next(iter(owned_windows(process, "#32770")), None))
    user = C.WinDLL("user32", use_last_error=True)
    get_owner = api(user, "GetWindow", W.HWND, W.HWND, W.UINT)
    get_text = api(user, "GetWindowTextW", C.c_int, W.HWND, W.LPWSTR, C.c_int)

    def text(hwnd):
        value = C.create_unicode_buffer(1024)
        get_text(hwnd, value, len(value))
        return value.value

    require(get_owner(dialog, 4) == browser.hwnd and text(dialog) == WARNING, "Unexpected close dialog or owner")
    detail = " ".join(text(child) for child in win.visible_children(dialog, "Static"))
    require("Downloads in progress: 1" in detail and f"Closing this {target}" in detail, detail)
    default = win.message(dialog, 0x0400)  # DM_GETDEFID
    require(default == (0x534B << 16) | IDNO, f"Default button is not No: {default:#x}")
    no = win.get_item(dialog, IDNO)
    require(no and win.pid(no) == process.pid, "Missing owned No button")
    post(process, dialog, 0x0111, IDNO, no)  # WM_COMMAND / BN_CLICKED
    browser.wait(f"declined {target} close", lambda: dialog not in owned_windows(process, "#32770"))
    return detail.strip()


def without_dialog(process, check):
    require(not owned_windows(process, "#32770"), "An unexpected close confirmation appeared")
    return check()


def completed_row(profile):
    history = profile / "Default" / "History"
    if history.is_file():
        with closing(sqlite3.connect(history.as_uri() + "?mode=ro", uri=True, timeout=0.1)) as db:
            return db.execute("""
                SELECT u.url, d.current_path, d.target_path, d.received_bytes, d.total_bytes
                FROM downloads d JOIN downloads_url_chains u ON d.id = u.id AND u.chain_index = 0
                WHERE d.state = 1 AND d.interrupt_reason = 0 AND d.end_time > 0
            """).fetchone()


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="flit-download-close-", dir=work))
    profile, directory = scratch / "profile", scratch / "attachments"
    directory.mkdir()
    report.update(work=str(scratch), launch={})
    with fixture() as site:
        seed_profile(profile, directory, site.origin)
        args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/"]
        try:
            with OwnedProcess(Win32(), exe, args, report["launch"]) as process:
                browser = Browser(process, site)
                browser.wait("loopback page", lambda: site.hits["/ready"] and browser.has_page_title(TITLE))
                require(not site.hits[ROUTE], "Download requested before the click")
                click_attachment(browser, 0)
                try:
                    browser.wait("attachment request", lambda: site.hits[ROUTE], 2)
                except RuntimeError:
                    process.alive()
                    if not site.hits[ROUTE]:
                        report["retried_click"] = True
                        click_attachment(browser, 0)
                partial = directory / (NAME + ".crdownload")
                browser.wait("held partial file in isolated directory",
                             lambda: partial.is_file() and partial.stat().st_size == len(CHUNK))
                report["ongoing_bytes"] = partial.stat().st_size

                browser.click(NEW_TAB)
                browser.wait("unrelated tab selected", lambda: browser.tab_count() == 2 and browser.selected() == 1)
                browser.edit_address(site.origin + "/")
                browser.click(GO)
                browser.wait("unrelated tab has its own browser", lambda: site.hits["/ready"] == 2 and browser.has_page_title(TITLE))
                browser.close_row(0)
                report["declined_tab_close"] = decline_close(browser, "tab")
                require(browser.tab_count() == 2 and browser.selected() == 1, "Declining closed or switched tabs")

                browser.close_row(1)
                browser.wait("unrelated tab closes silently", lambda: without_dialog(process, lambda: browser.tab_count() == 1))
                report["unrelated_tab_closed"] = True

                post(process, browser.hwnd, 0x0111, 161)  # WM_COMMAND / New window
                other = browser.wait("unrelated window", lambda: next((hwnd for hwnd in owned_windows(process, "FlitBrowserWindow")
                                                                       if hwnd != browser.hwnd), None))
                post(process, other, 0x0010)  # WM_CLOSE
                browser.wait("unrelated window closes silently", lambda: without_dialog(process, lambda: not process.win.pid(other)))
                report["unrelated_window_closed"] = True

                post(process, browser.hwnd, 0x0010)  # WM_CLOSE
                report["declined_window_close"] = decline_close(browser, "window")
                require(browser.tab_count() == 1 and partial.is_file(), "Declining stopped the held download")
                site.release.set()
                path = directory / NAME
                size = 2 * len(CHUNK)
                browser.wait("finished file in isolated directory", lambda: path.is_file() and not partial.exists())
                digest = sha256(path)
                require(digest == hashlib.sha256(CHUNK * 2).hexdigest(), "Final file did not match the fixture")
                report["completed_file"] = {"name": NAME, "bytes": size, "sha256": digest}

                # As in windows_downloads, the final rename can precede Windows quarantine.
                time.sleep(2)
                post(process, browser.hwnd, 0x0010)  # WM_CLOSE
                until("normal exit without repeat warning", lambda: without_dialog(process, process.exited), 15)
                require(process.win.native.GetExitCodeProcess(process.process) == 0, "Flit did not exit cleanly")
                until("empty owned process job", lambda: not process.pids(), 8)
                report["closed_without_repeat"] = True
                row = completed_row(profile)
                require(row is not None, "Chromium did not record a completed download")
                url, current, target, received, total = row
                require(site.hits[ROUTE] == 1 and url == site.origin + ROUTE and received == total == size
                        and Path(current) == Path(target) == path, f"Unexpected completed download record: {row!r}")
                report["completed_history"] = True
        finally:
            report["http_requests"] = dict(site.hits)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--work", required=True, type=Path)
    args = parser.parse_args()
    exe = args.exe.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.lower() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires Windows and a staged Flit.exe next to Flit.dll")
    report = {"result": "fail", "exe": str(exe), "dll_sha256": sha256(dll)}
    try:
        run(exe, args.work.resolve(), report)
        require(sha256(dll) == report["dll_sha256"], "Staged DLL changed during the test")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
