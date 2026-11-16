"""Regression: defer closing the source tab until its held download completes."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import time

sys.dont_write_bytecode = True
from windows_download_close import CHUNK, IDNO, NAME, ROUTE, WARNING, fixture, owned_windows, post
from windows_downloads import TITLE, click_attachment, seed_profile, sha256
from windows_smoke import Browser, GO, NEW_TAB, OwnedProcess, TABS, Win32, api, require, until


WM_CLOSE, WM_COMMAND = 0x0010, 0x0111
IDYES, EXIT_FLIT = 6, 172
PENDING = "After download — "


def tabs(browser):
    labels = []
    for index in range(browser.tab_count()):
        value = C.create_unicode_buffer(1024)
        require(browser.message(TABS, 0x0189, index, C.addressof(value)) >= 0, "Could not read tab label")
        labels.append(value.value)
    return {"labels": labels, "selected": browser.selected(), "address": browser.address()}


def dialogs(process):
    return owned_windows(process, "#32770")


def chrome_windows(process):
    return {hwnd for cls in ("Chrome_WidgetWin_0", "Chrome_WidgetWin_1", "Chrome_WidgetWin_2")
            for hwnd in owned_windows(process, cls)}


def dialog_info(browser, hwnd):
    process, win = browser.process, browser.win
    require(win.pid(hwnd) == process.pid, "Close dialog is no longer owned")
    user = C.WinDLL("user32", use_last_error=True)
    get_owner = api(user, "GetWindow", W.HWND, W.HWND, W.UINT)
    get_text = api(user, "GetWindowTextW", C.c_int, W.HWND, W.LPWSTR, C.c_int)
    title = C.create_unicode_buffer(1024)
    get_text(hwnd, title, len(title))
    parts = []
    for child in win.visible_children(hwnd, "Static"):
        if win.pid(child) == process.pid:
            value = C.create_unicode_buffer(1024)
            win.message(child, 0x000D, len(value), C.addressof(value))  # WM_GETTEXT
            if value.value:
                parts.append(value.value)
    return {"hwnd": hwnd, "owner": get_owner(hwnd, 4), "class": win.class_name(hwnd),
            "caption": title.value, "message": "\n".join(parts)}


def answer(browser, warning, action, button_id):
    process, win, hwnd = browser.process, browser.win, warning["hwnd"]
    require(warning["class"] == "#32770" and warning["caption"] == WARNING
            and warning["owner"] == browser.hwnd, f"Unexpected close dialog: {warning}")
    require("Downloads in progress: 1" in warning["message"].splitlines()
            and action in warning["message"], f"Wrong download warning: {warning}")
    require(win.message(hwnd, 0x0400) == (0x534B << 16) | IDNO, "Default close answer is not No")
    button = win.get_item(hwnd, button_id)
    require(button and win.pid(button) == process.pid and win.class_name(button) == "Button",
            f"Missing owned button {button_id}")
    post(process, hwnd, WM_COMMAND, button_id, button)  # BN_CLICKED
    warning["answer"] = "Yes" if button_id == IDYES else "No"
    browser.wait("answered warning dismissed", lambda: not win.visible(hwnd) and win.enabled(browser.hwnd))


def quiet(process, seconds, stop=None):
    end = time.monotonic() + seconds
    while True:
        require(not dialogs(process), "Unexpected or repeated native dialog")
        if stop and stop():
            return True
        remaining = end - time.monotonic()
        if remaining <= 0:
            return False
        time.sleep(min(0.05, remaining))


def close_attempt(browser, report, key, msg, command=0):
    process = browser.process
    require(not dialogs(process), "A native dialog was open before the close request")
    before = chrome_windows(process)
    attempt = report[key] = {"flit_warnings": 0}
    post(process, browser.hwnd, msg, command)

    def outcome():
        native = dialogs(process)
        if native:
            attempt["dialogs"] = [dialog_info(browser, hwnd) for hwnd in native]
            attempt["flit_warnings"] = sum(
                warning["caption"] == WARNING for warning in attempt["dialogs"])
            return "flit_warning" if attempt["flit_warnings"] else "native_confirmation"
        if process.exited() or process.win.pid(browser.hwnd) != process.pid:
            return "window_closed"
        new = chrome_windows(process) - before
        if new:
            attempt["dialogs"] = [dialog_info(browser, hwnd) for hwnd in sorted(new)]
            return "chromium_window"

    def observe():
        nonlocal observed
        observed = outcome()
        return observed

    observed = None
    end = time.monotonic() + 6
    while not observe() and time.monotonic() < end:
        time.sleep(0.05)
    attempt["outcome"] = observed or "no_warning_observed"
    if observed == "flit_warning":
        require(len(attempt["dialogs"]) == 1, f"Expected one close warning: {attempt}")
        answer(browser, attempt["dialogs"][0], "Exiting Flit" if command else "Closing this window", IDNO)
        quiet(process, 0.35)
    # Any Chromium confirmation is evidence, not a Flit guard. Leave it unanswered;
    # OwnedProcess can cleanly contain a baseline that did not exit on its own.
    return observed == "flit_warning"


def exercise(process, site, downloads, report):
    browser = Browser(process, site)
    browser.wait("source page", lambda: site.hits["/ready"] and browser.has_page_title(TITLE))
    require(not site.hits[ROUTE], "Attachment requested before the click")
    click_attachment(browser, 0)
    try:
        browser.wait("attachment request", lambda: site.hits[ROUTE], 2)
    except RuntimeError:
        process.alive()
        if not site.hits[ROUTE]:
            click_attachment(browser, 0)
    partial = downloads / (NAME + ".crdownload")
    final = downloads / NAME

    def held():
        return not site.release.is_set() and partial.is_file() and partial.stat().st_size == len(CHUNK)

    browser.wait("held attachment body", held)
    report["held_bytes"] = len(CHUNK)
    pids = report["launch"]["pids_while_held"] = process.pids()
    require(process.pid in pids, "Flit root missing from its job")
    peer = site.origin + "/#survivor"
    browser.click(NEW_TAB)
    browser.wait("second tab selected", lambda: browser.tab_count() == 2 and browser.selected() == 1)
    browser.edit_address(peer)
    browser.click(GO)
    browser.wait("ordinary second tab loaded", lambda: site.hits["/ready"] >= 2
                 and browser.has_page_title(TITLE) and browser.address() == peer and browser.page_panel())
    before = report["before_yes"] = tabs(browser)
    require(before == {"labels": [TITLE, TITLE], "selected": 1, "address": peer} and held()
            and site.hits[ROUTE] == 1 and not dialogs(process), "The close fixture was not ready")

    browser.close_row(0)
    native = browser.wait("source tab warning", lambda: dialogs(process), 6)
    require(len(native) == 1, f"Expected one source tab warning: {native}")
    tab_warning = report["tab_warning"] = dialog_info(browser, native[0])
    answer(browser, tab_warning, "Closing this tab", IDYES)
    pending = {"labels": [PENDING + TITLE, TITLE], "selected": 1, "address": peer}
    survivor = {"labels": [TITLE], "selected": 0, "address": peer}

    def after_yes():
        current = report["after_yes"] = tabs(browser)
        return current if current in (pending, survivor) else None

    current = browser.wait("deferred source row or immediate removal", after_yes, 6)
    require(not site.release.is_set(), "Body released before the window close attempt")
    window_warned = close_attempt(browser, report, "window_x", WM_CLOSE)
    report["body_released_at_x"] = site.release.is_set()
    require(current == pending,
            "Source tab closed before download completion; see window_x for the held-close outcome")
    require(window_warned and tabs(browser) == pending and held(), "Window X did not protect the held download")
    require(close_attempt(browser, report, "exit_flit", WM_COMMAND, EXIT_FLIT)
            and tabs(browser) == pending and held(), "Exit Flit did not protect the held download")
    require("after the downloads finish" in tab_warning["message"].lower(), "Yes did not offer to defer the tab close")

    site.release.set()
    browser.wait("final attachment and no partial file", lambda: final.is_file() and not partial.exists())
    size, digest = final.stat().st_size, sha256(final)
    report["completed_file"] = {"name": NAME, "bytes": size, "sha256": digest}
    require(size == 2 * len(CHUNK) and digest == hashlib.sha256(CHUNK * 2).hexdigest()
            and site.hits[ROUTE] == 1, "Final download did not match the sole fixture request")
    browser.wait("only the original tab closes automatically", lambda: tabs(browser) == survivor)
    report["after_completion"] = tabs(browser)
    quiet(process, 2)  # Chromium can rename the file before its terminal download callback.
    post(process, browser.hwnd, WM_CLOSE)
    require(quiet(process, 15, process.exited), "Flit did not exit after the completed download")
    code = report["launch"]["exit_code"] = process.win.native.GetExitCodeProcess(process.process)
    require(code == 0, f"Flit exited with 0x{code:08x}")
    until("empty owned process job", lambda: not process.pids(), 8)
    report["closed_without_stale_warning"] = True


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    scratch = tempfile.TemporaryDirectory(prefix="flit-download-detached-", dir=work)
    root = Path(scratch.name)
    report.update(scratch=str(root), launch={})
    try:
        profile, downloads = root / "profile", root / "attachments"
        downloads.mkdir()
        with fixture() as site:
            try:
                seed_profile(profile, downloads, site.origin)
                args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/"]
                with OwnedProcess(Win32(), exe, args, report["launch"]) as process:
                    try:
                        exercise(process, site, downloads, report)
                    except (Exception, KeyboardInterrupt) as error:
                        report["error"] = f"{type(error).__name__}: {error}"
                        raise
            finally:
                report["http_requests"] = dict(site.hits)
    finally:
        try:
            scratch.cleanup()
        except OSError as error:
            report["scratch_cleanup_error"] = str(error)
        report["scratch_removed"] = not root.exists()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", required=True, type=Path, help="parent for a disposable profile and downloads")
    args = parser.parse_args()
    exe = args.exe.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.lower() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")
    report = {"result": "fail", "exe": str(exe)}
    try:
        report["sha256"] = {path.name: sha256(path) for path in (exe, dll)}
        run(exe, args.work.resolve(), report)
        require(report["scratch_removed"] and "scratch_cleanup_error" not in report, "Scratch cleanup failed")
        require(report["launch"].get("leftover_pids") == [], "Owned process job was not emptied")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        message = f"{type(error).__name__}: {error}"
        if report.setdefault("error", message) != message:
            report["unwind_error"] = message
    if "sha256" in report:
        try:
            report["sha256_after"] = {path.name: sha256(path) for path in (exe, dll)}
            require(report["sha256"] == report["sha256_after"], "Staged payload changed during the test")
        except (Exception, KeyboardInterrupt) as error:
            report["payload_error"] = f"{type(error).__name__}: {error}"
            report["result"] = "fail"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
