"""Check that a deferred tab close is cleared when a held download is interrupted."""

import argparse
from contextlib import closing
import json
from pathlib import Path
import socket
import sqlite3
import sys
import tempfile
from threading import Event

sys.dont_write_bytecode = True
from windows_download_close import CHUNK, NAME, ROUTE, HeldAttachment, fixture, post
from windows_download_detached import IDYES, PENDING, WM_CLOSE, answer, dialog_info, dialogs, quiet, tabs
from windows_downloads import TITLE, click_attachment, seed_profile, sha256
from windows_smoke import Browser, GO, NEW_TAB, OwnedProcess, Win32, require, until


CONTENT_LENGTH = 2 * len(CHUNK)


class InterruptedAttachment(HeldAttachment):
    # Chromium needs HTTP/1.1 and a strong validator to treat a short body as an error.
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        site = self.server
        if self.path != site.origin + ROUTE:
            return super().do_GET()
        site.hits[ROUTE] += 1
        if site.release.is_set():
            self.send_error(503)  # A retry cannot fetch the missing bytes.
            return
        try:
            self.send_response(200)
            self.send_header("Connection", "close")
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Disposition", f'attachment; filename="{NAME}"')
            self.send_header("Content-Length", str(CONTENT_LENGTH))
            self.send_header("ETag", '"flit-held-attachment"')
            self.end_headers()
            site.written = self.wfile.write(CHUNK)
            if site.release.wait(45):
                self.connection.shutdown(socket.SHUT_WR)
                site.truncated.set()
        except ConnectionError:
            pass


def exercise(process, site, downloads, report):
    browser = Browser(process, site)
    browser.wait("source attachment page", lambda: site.hits["/ready"] and browser.has_page_title(TITLE))
    require(not site.hits[ROUTE], "Attachment requested before the click")
    click_attachment(browser, 0)
    try:
        browser.wait("attachment request", lambda: site.hits[ROUTE], 2)
    except RuntimeError:
        process.alive()
        if not site.hits[ROUTE]:
            click_attachment(browser, 0)

    partial = downloads / (NAME + ".crdownload")

    def held():
        return not site.release.is_set() and partial.is_file() and partial.stat().st_size == len(CHUNK)

    browser.wait("held partial attachment in scratch", held)
    peer = site.origin + "/#survivor"
    normal = {"labels": [TITLE, TITLE], "selected": 1, "address": peer}
    pending = dict(normal, labels=[PENDING + TITLE, TITLE])
    browser.click(NEW_TAB)
    browser.wait("second tab selected", lambda: browser.tab_count() == 2 and browser.selected() == 1)
    browser.edit_address(peer)
    browser.click(GO)
    browser.wait("peer page loaded", lambda: site.hits["/ready"] >= 2 and browser.page_panel() and tabs(browser) == normal)
    report["before_close"] = tabs(browser)
    require(held() and site.hits[ROUTE] == 1 and not dialogs(process), "The held close fixture was not ready")

    browser.close_row(0)
    native = browser.wait("source tab warning", lambda: dialogs(process), 6)
    require(len(native) == 1, f"Expected one source tab warning: {native}")
    warning = report["tab_warning"] = dialog_info(browser, native[0])
    require("after the downloads finish" in warning["message"].lower(), "Warning did not offer deferred close")
    answer(browser, warning, "Closing this tab", IDYES)

    def observe(key):
        require(not dialogs(process), "Unexpected native dialog")
        state = report[key] = tabs(browser)
        require(len(state["labels"]) == 2 and state["selected"] == 1 and state["address"] == peer,
                f"Source or active peer was lost: {state}")
        return state

    browser.wait("deferred source row", lambda: observe("scheduled") == pending, 6)
    require(held() and not site.truncated.is_set(), "Download stopped before the planned interruption")
    site.release.set()
    browser.wait("fixture sends EOF after the first chunk", site.truncated.is_set, 5)
    browser.wait("reservation clears while both tabs remain", lambda: observe("after_interruption") == normal, 15)
    quiet(process, 1)
    require(observe("after_interruption") == normal, "Tab state changed after the interruption")
    report["final_file_absent"] = not (downloads / NAME).exists()
    require(report["final_file_absent"], "Interrupted attachment was published as a final file")

    post(process, browser.hwnd, WM_CLOSE)
    require(quiet(process, 15, process.exited), "Flit did not exit after the interruption")
    code = report["launch"]["exit_code"] = process.win.native.GetExitCodeProcess(process.process)
    require(code == 0, f"Flit exited with 0x{code:08x}")
    until("empty owned process job", lambda: not process.pids(), 8)
    report["closed_without_stale_warning"] = True


def interrupted_history(profile, url):
    history = profile / "Default" / "History"
    if not history.is_file():
        return []
    with closing(sqlite3.connect(history.as_uri() + "?mode=ro", uri=True)) as db:
        db.row_factory = sqlite3.Row
        return [dict(row) for row in db.execute("""
            SELECT d.state, d.interrupt_reason, d.received_bytes, d.total_bytes
            FROM downloads d JOIN downloads_url_chains u ON d.id = u.id AND u.chain_index = 0
            WHERE u.url = ?
        """, (url,))]


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="flit-download-interrupted-", dir=work) as scratch:
        root = Path(scratch)
        report.update(scratch=scratch, launch={})
        profile, downloads = root / "profile", root / "attachments"
        downloads.mkdir()
        with fixture() as site:
            site.RequestHandlerClass = InterruptedAttachment
            site.truncated, site.written = Event(), 0
            try:
                seed_profile(profile, downloads, site.origin)
                args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/"]
                with OwnedProcess(Win32(), exe, args, report["launch"]) as process:
                    exercise(process, site, downloads, report)
                require(report["launch"].get("leftover_pids") == [], "Owned process job was not emptied")
                report["final_file_absent"] = not (downloads / NAME).exists()
                require(report["final_file_absent"], "Final attachment appeared during shutdown")
                rows = report["history_rows"] = interrupted_history(profile, site.origin + ROUTE)
                # Chromium can omit interrupted initial transfers from History.
                require(all(row["state"] == 4 and row["interrupt_reason"] not in (0, 40, 41, 50) for row in rows),
                        f"Unexpected completion or cancellation in Chromium History: {rows}")
            finally:
                report["http_requests"] = dict(site.hits)
                report["response"] = {"content_length": CONTENT_LENGTH, "bytes_written": site.written,
                                      "truncated": site.truncated.is_set()}
    report["scratch_removed"] = not root.exists()
    require(report["scratch_removed"], "Scratch cleanup failed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", required=True, type=Path, help="parent for disposable profile and downloads")
    args = parser.parse_args()
    exe = args.exe.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.lower() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")
    report = {"result": "fail", "exe": str(exe)}
    try:
        report["sha256"] = {path.name: sha256(path) for path in (exe, dll)}
        run(exe, args.work.resolve(), report)
        require(report["sha256"] == {path.name: sha256(path) for path in (exe, dll)},
                "Staged payload changed during the test")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
