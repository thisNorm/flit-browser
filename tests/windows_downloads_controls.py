"""Exercise Chromium's downloads controls in a staged Flit on Windows.

Run with --exe PATH/Flit.exe --work PATH on an interactive desktop. The test
uses an owned browser job, a disposable profile, and a nonforwarding loopback
proxy. Pause/Resume covers an active transfer; an interrupted download or a
new HTTP Range request is not required. Show in folder is never invoked.
"""

import argparse
from collections import Counter
from contextlib import contextmanager
import hashlib
from http.server import ThreadingHTTPServer
import json
from pathlib import Path
import re
import sys
import tempfile
import threading
import time

sys.dont_write_bytecode = True
import windows_extensions as extensions
from windows_downloads import Handler, TITLE, click_attachment, seed_profile, sha256
from windows_multi_session import clean_scratch
from windows_session import select, send
from windows_smoke import Browser, OwnedProcess, Win32, require


DOWNLOADS = "chrome://downloads/"
COMPLETE, CANCEL = "/first", "/second"  # The existing fixture page's two links.
NAMES = {COMPLETE: "flit-controls-complete.txt", CANCEL: "flit-controls-cancel.txt"}
HEAD, TAIL = b"A" * (64 * 1024), b"B" * (32 * 1024)
BODY = HEAD + TAIL
ETAG = '"flit-downloads-controls-v1"'


class HeldAttachments(Handler):
    def do_GET(self):
        site = self.server
        route = self.path.removeprefix(site.origin)
        if route not in NAMES or self.path != site.origin + route:
            return super().do_GET()

        # Cap all attachment writes, including retry requests, at 4 * 96 KiB.
        with site.lock:
            site.hits[route] += 1
            allowed = site.hits[route] <= 2
        if not allowed:
            self.send_error(503)
            return

        requested = self.headers.get("Range")
        range_header = requested if self.headers.get("If-Range", ETAG) == ETAG else None
        start = 0
        if range_header is not None:
            match = re.fullmatch(r"bytes=([0-9]{1,6})-", range_header)
            if not match or int(match[1]) >= len(BODY):
                self.send_response(416)
                self.send_header("Content-Range", f"bytes */{len(BODY)}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            start = int(match[1])

        status = 200 if range_header is None else 206
        with site.lock:
            site.attachment_requests.append({"route": route, "range": requested, "status": status})
        self.protocol_version = "HTTP/1.1"
        try:
            self.send_response(status)
            self.send_header("Connection", "close")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Disposition", f'attachment; filename="{NAMES[route]}"')
            self.send_header("Content-Length", str(len(BODY) - start))
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("ETag", ETAG)
            if status == 206:
                self.send_header("Content-Range", f"bytes {start}-{len(BODY) - 1}/{len(BODY)}")
            self.end_headers()
            if start < len(HEAD):
                self.wfile.write(BODY[start:len(HEAD)])
            if route == CANCEL:
                site.stopping.wait(45)  # The canceled transfer can never receive its tail.
            elif site.release.wait(45) and not site.stopping.is_set():
                self.wfile.write(BODY[max(start, len(HEAD)):])
                site.tail_sent.set()
        except ConnectionError:
            pass  # Chromium can close its socket before the held response ends.


@contextmanager
def fixture():
    with ThreadingHTTPServer(("127.0.0.1", 0), HeldAttachments) as site:
        site.origin = f"http://127.0.0.1:{site.server_port}"
        site.hits, site.lock, site.attachment_requests = Counter(), threading.Lock(), []
        site.release, site.tail_sent, site.stopping = (threading.Event() for _ in range(3))
        worker = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
        worker.start()
        try:
            yield site
        finally:
            site.stopping.set()
            site.release.set()
            site.shutdown()
            worker.join()


def has_bounds(row):
    rect = row["rect"]
    return rect and rect[2] > 0 and rect[3] > 0


def visible_text(rows, *names):
    return any(has_bounds(row) and row["name"].strip("\u202a\u202c").strip() in names for row in rows)


def control(rows, control_id, *labels):
    matches = [row for row in rows if row["id"] == control_id and row["enabled"] and has_bounds(row)
               and row["type"] in ("ControlType.Button", "ControlType.MenuItem")]
    require(len(matches) <= 1, f"Ambiguous Downloads control: {control_id}")
    if matches and (not labels or matches[0]["name"].strip() in labels):
        return matches[0]


def held(path):
    try:
        return path.stat().st_size == len(HEAD)
    except FileNotFoundError:
        return False


def start_attachment(browser, downloads, route, index):
    site = browser.site
    final = downloads / NAMES[route]
    partial = final.with_suffix(final.suffix + ".crdownload")
    require(not site.hits[route], f"{final.name} was requested before the link click")
    click_attachment(browser, index)
    try:
        browser.wait(f"{final.name} request", lambda: site.hits[route], 2)
    except RuntimeError:
        browser.process.alive()
        if not site.hits[route]:
            click_attachment(browser, index)
    browser.wait(f"held partial for {final.name}", lambda: held(partial))
    require(site.hits[route] == 1 and not final.exists(), f"Expected one held transfer for {final.name}")
    return partial, final


def exercise(browser, downloads, report):
    site = browser.site

    def ui(label, check):
        report["stage"] = label
        rows = []

        def ready():
            nonlocal rows
            rows = extensions.web_controls(browser, expected_url=DOWNLOADS)
            return check(rows)

        try:
            return browser.wait(label, ready)
        except (Exception, KeyboardInterrupt):
            report["last_uia"] = rows
            raise

    def menu_action(filename, control_id, *labels):
        # Only one item is active. A completed item uses the quick folder button.
        more = ui(f"{filename}: visible filename and More actions", lambda rows:
                  visible_text(rows, filename) and not control(rows, "cancel") and control(rows, "more-actions"))
        click(more)
        return ui(f"{filename}: enabled {labels[0]}", lambda rows: control(rows, control_id, *labels))

    def click(control):
        extensions.click_web(browser, control, expected_url=DOWNLOADS)

    report["stage"] = "start completion fixture"
    browser.wait("loopback attachment page", lambda: site.hits["/ready"] and browser.has_page_title(TITLE))
    source = browser.selected()
    partial, final = start_attachment(browser, downloads, COMPLETE, 0)

    report["stage"] = "open Chromium Downloads"
    send(browser, 0x0111, 121)  # WM_COMMAND: existing Downloads menu command.
    browser.wait("chrome://downloads/ committed in another tab", lambda: browser.address() == DOWNLOADS
                 and browser.selected() != source and any(browser.has_page_title(title) for title in
                 ("Downloads", "Download history", "다운로드", "다운로드 기록")))
    downloads_tab = browser.selected()
    report["downloads_page"] = {"command": 121, "url": browser.address(), "title": browser.title()}

    completed = report["completed"] = {"filename": final.name}
    completed["pause"] = pause = menu_action(final.name, "pause-or-resume", "Pause", "일시중지")
    click(pause)
    completed["resume"] = resume = menu_action(final.name, "pause-or-resume", "Resume", "다시 시작")

    report["stage"] = "verify paused file after the server sends the tail"
    site.release.set()
    browser.wait("fixture sent the remaining bytes while paused", site.tail_sent.is_set, 5)
    deadline = time.monotonic() + .75
    while True:
        require(held(partial) and not final.exists(), "The download advanced while paused")
        if time.monotonic() >= deadline:
            break
        time.sleep(.05)
    completed["paused_bytes"] = len(HEAD)
    click(resume)

    completed["show_in_folder"] = ui("completed filename and enabled Show in folder", lambda rows:
                                     visible_text(rows, final.name) and control(rows, "quick-show-in-folder"))
    require(not partial.exists() and final.is_file() and final.stat().st_size == len(BODY)
            and sha256(final) == hashlib.sha256(BODY).hexdigest(), "Completed file differs from the fixture")
    completed.update(bytes=len(BODY), sha256=sha256(final))

    report["stage"] = "start cancellation fixture"
    select(browser, source)
    browser.wait("source tab restored", lambda: browser.address() == site.origin + "/" and browser.has_page_title(TITLE))
    partial, canceled = start_attachment(browser, downloads, CANCEL, 1)
    select(browser, downloads_tab)
    browser.wait("Downloads tab restored", lambda: browser.address() == DOWNLOADS)
    cancelled = report["cancelled"] = {"filename": canceled.name}
    cancelled["cancel"] = cancel = menu_action(canceled.name, "cancel", "Cancel", "취소")
    click(cancel)
    ui("canceled filename and status in the same item", lambda rows:
       any(row["id"] == "title-area" and has_bounds(row) and
           row["name"].strip() in (f"{canceled.name} Canceled", f"{canceled.name} 취소됨") for row in rows))
    browser.wait("no canceled partial or final file", lambda: not partial.exists() and not canceled.exists())
    cancelled.update(status_visible=True, partial_absent=True, final_file_absent=True)


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    scratch = tempfile.TemporaryDirectory(prefix="flit-downloads-controls-", dir=work)
    root = Path(scratch.name)
    try:
        with fixture() as site:
            profile, downloads = root / "profile", root / "attachments"
            downloads.mkdir()
            seed_profile(profile, downloads, site.origin)
            report["launch"] = {}
            args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/"]
            try:
                with OwnedProcess(Win32(), exe, args, report["launch"]) as process:
                    browser = Browser(process, site)
                    exercise(browser, downloads, report)
                    report["stage"] = "close owned browser"
                    process.normal_close(browser.hwnd)
                require({path.name for path in downloads.iterdir()} == {NAMES[COMPLETE]},
                        "Unexpected file in the isolated download directory after exit")
            finally:
                with site.lock:
                    report.update(http_requests=dict(site.hits), attachment_requests=list(site.attachment_requests))
    finally:
        clean_scratch(scratch, report)
        report["scratch_removed"] = not root.exists()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe beside Flit.dll and libcef.dll")
    parser.add_argument("--work", required=True, type=Path, help="parent for disposable profile and attachments")
    args = parser.parse_args()
    exe = args.exe.resolve()
    files = (exe, exe.with_name("Flit.dll"), exe.with_name("libcef.dll"))
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not all(path.is_file() for path in files):
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll and libcef.dll")
    report = {"result": "fail", "exe": str(exe), "stage": "setup"}
    try:
        report["sha256"] = {path.name: sha256(path) for path in files[:2]}
        run(exe, args.work.resolve(), report)
        require(report["scratch_removed"] and report["launch"].get("leftover_pids") == [], "Test cleanup failed")
        report.update(result="pass", stage="complete")
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
