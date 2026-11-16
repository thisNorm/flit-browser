"""Check the native Downloads button with held loopback attachments."""

import argparse
from base64 import b64encode
import hashlib
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
from threading import Event

sys.dont_write_bytecode = True
from windows_download_close import CHUNK, NAME, ROUTE, fixture
from windows_download_interrupted import InterruptedAttachment
from windows_downloads import TITLE, click_attachment, seed_profile, sha256
from windows_multi_session import clean_scratch
from windows_smoke import ADDRESS, NEW_TAB, SIDEBAR_TOGGLE, TABS, Browser, OwnedProcess, Win32, require


DOWNLOADS = 190
IDLE, ACTIVE = "Downloads", "Downloads (1 active)"
URL = "chrome://downloads/"
PAGE_TITLES = ("Downloads", "Download history", "다운로드", "다운로드 기록")


def expect_name(browser, label, expected, names):
    def matches():
        names[label] = browser.control_text(DOWNLOADS)
        return names[label] == expected

    browser.wait(f"{label} Downloads caption {expected!r}", matches)
    # Read UI Automation only on the owned native button, never in Chromium's page.
    script = f"""
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$button = [Windows.Automation.AutomationElement]::FromHandle([IntPtr][long]{browser.controls[DOWNLOADS]}).Current
if ($button.ProcessId -ne {browser.process.pid}) {{ throw 'Downloads button is no longer owned' }}
$button.Name
"""
    result = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand",
         b64encode(script.encode("utf-16le")).decode("ascii")],
        stdin=subprocess.DEVNULL, capture_output=True, text=True, errors="replace",
        timeout=8, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    require(result.returncode == 0, f"Downloads accessibility query failed: {result.stderr.strip()}")
    name = names[label] = result.stdout.rstrip("\r\n")
    require(name == expected, f"{label} Downloads accessible name: expected {expected!r}, got {name!r}")


def check_layout(browser):
    win, hwnd = browser.win, browser.hwnd
    address, button = browser.controls[ADDRESS], browser.controls[DOWNLOADS]
    menu = win.get_item(hwnd, 117)  # Flit menu
    require(menu and win.pid(menu) == browser.process.pid, "Missing owned menu button")
    with win.window_coordinates(hwnd):
        left, top, right, bottom = win.rect(hwnd)
        dpi = win.dpi(hwnd)
        width, height = right - left, bottom - top
        narrow = max(560 * dpi // 96, width * 2 // 3)
        flags = 0x0016  # SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE.
        if narrow < width:
            win.check(win.set_window_pos(hwnd, None, 0, 0, narrow, height, flags))
        try:
            for collapsed in (False, True):
                if collapsed:
                    browser.click(SIDEBAR_TOGGLE)
                    browser.wait("sidebar collapsed", lambda: not win.visible(browser.controls[TABS]))
                order = (address, browser.controls[NEW_TAB], button, menu) if collapsed else (address, button, menu)
                bounds = [win.rect(control) for control in order]
                require(all(win.visible(control) for control in order)
                        and all(first[2] <= second[0] for first, second in zip(bounds, bounds[1:]))
                        and bounds[-1][2] < win.rect(hwnd)[2],
                        "Downloads button overlaps the nearby toolbar controls")
            browser.click(SIDEBAR_TOGGLE)
            browser.wait("sidebar expanded", lambda: win.visible(browser.controls[TABS]))
        finally:
            if narrow < width:
                win.check(win.set_window_pos(hwnd, None, 0, 0, width, height, flags))
    return {"dpi": dpi, "width_px": min(narrow, width), "sidebar_expanded_and_collapsed": True}


def exercise(browser, site, downloads, report, *, interrupt):
    win, pid = browser.win, browser.process.pid
    browser.wait("fixture page through the loopback test proxy",
                 lambda: site.hits["/ready"] and browser.has_page_title(TITLE))
    button = win.get_item(browser.hwnd, DOWNLOADS)
    require(button and win.pid(button) == pid and win.class_name(button).casefold() == "button"
            and win.visible(button), "Missing visible owned Win32 Button 190")
    browser.controls[DOWNLOADS] = button
    names = report["names"] = {}
    expect_name(browser, "idle", IDLE, names)

    require(not site.hits[ROUTE], "Attachment requested before clicking the fixture link")
    click_attachment(browser, 0)
    try:
        browser.wait("attachment request", lambda: site.hits[ROUTE], 2)
    except RuntimeError:
        browser.process.alive()
        if not site.hits[ROUTE]:
            click_attachment(browser, 0)
    final, partial = downloads / NAME, downloads / (NAME + ".crdownload")
    browser.wait("held attachment bytes", lambda: partial.is_file() and partial.stat().st_size == len(CHUNK))
    require(site.hits[ROUTE] == 1, "Expected exactly one held attachment request")
    expect_name(browser, "active", ACTIVE, names)

    def downloads_visible():
        address, title = browser.address(), browser.title()
        panel = browser.page_panel()
        report["page"] = {"url": address, "title": title, "panel": panel}
        return (address == URL and title.partition(" — ")[2] in PAGE_TITLES and panel
                and win.pid(panel) == pid and any(win.pid(child) == pid for child in win.visible_children(panel)))

    browser.click(DOWNLOADS)
    browser.wait("Downloads URL, title and visible owned CEF panel from one click", downloads_visible)
    tab_count = browser.tab_count()
    browser.click(DOWNLOADS)
    require(browser.address() == URL and browser.tab_count() == tab_count,
            "Clicking Downloads again duplicated the Downloads tab")
    require(not site.release.is_set() and partial.is_file() and browser.control_text(DOWNLOADS) == ACTIVE,
            "Downloads page did not open while the attachment was still held")

    site.release.set()
    if interrupt:
        browser.wait("fixture truncates the attachment", site.truncated.is_set)
    else:
        browser.wait("fixture file completes", lambda: final.is_file() and not partial.exists())
        require(sha256(final) == hashlib.sha256(CHUNK * 2).hexdigest(), "Completed file differs from the fixture")
    expect_name(browser, "after_interruption" if interrupt else "after_completion", IDLE, names)
    if not interrupt:
        report["layout"] = check_layout(browser)
    browser.process.normal_close(browser.hwnd)
    if interrupt:
        require(not final.exists(), "Interrupted attachment was finalized")
    report["final_file_present"] = final.is_file()


def run(exe, work, report, *, interrupt):
    scratch = TemporaryDirectory(prefix="flit-downloads-toolbar-", dir=work)
    try:
        with fixture() as site:
            root = Path(scratch.name)
            profile, downloads = root / "profile", root / "attachments"
            downloads.mkdir()
            if interrupt:
                site.RequestHandlerClass = InterruptedAttachment
                site.truncated = Event()
            seed_profile(profile, downloads, site.origin)
            args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/"]
            try:
                with OwnedProcess(Win32(), exe, args, report.setdefault("launch", {})) as process:
                    exercise(Browser(process, site), site, downloads, report, interrupt=interrupt)
            finally:
                report["http_requests"] = dict(site.hits)
    finally:
        clean_scratch(scratch, report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="parent for disposable profiles and attachments")
    args = parser.parse_args()
    exe = args.exe.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.lower() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")
    report = {"result": "fail", "exe": str(exe), "cases": {}}
    try:
        work = args.work.resolve()
        work.mkdir(parents=True, exist_ok=True)
        report["sha256"] = {path.name: sha256(path) for path in (exe, dll)}
        for case, interrupt in (("completion", False), ("interruption", True)):
            item = report["cases"][case] = {}
            run(exe, work, item, interrupt=interrupt)
        require(report["sha256"] == {path.name: sha256(path) for path in (exe, dll)},
                "Staged payload changed during the test")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
