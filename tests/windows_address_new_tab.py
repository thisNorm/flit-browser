"""Verify address submission to a foreground tab in an owned Flit window."""

import argparse
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import time

sys.dont_write_bytecode = True
from windows_multi_session import clean_scratch
from windows_search import ENGINES, PAGES, TERM, page, proxy
from windows_session import loaded_panels, select
from windows_smoke import ADDRESS, OwnedProcess, WORKSPACE, Win32, require
from windows_tab_order import dll_sha256
from windows_windows import Window, seed_profile


FOCUS_ADDRESS, GO_NEW_TAB, CLOSE_TAB = 113, 180, 106
WM_SYSKEYDOWN, WM_SYSCHAR, VK_RETURN = 0x0104, 0x0106, 0x0D
ALT_ENTER = 0x201C0001  # Alt context, Enter scan code, one keystroke.
START, TARGET = "/start", "/normal"


def exercise(browser, checks):
    site = browser.site
    source_url = site.origin + START
    target_url = site.origin + TARGET
    page(browser, START, {START: 0})
    source_panel = browser.wait("visible source panel", browser.page_panel)
    source_requests = site.snapshot()[0][START]

    def unchanged_source():
        require(site.snapshot()[0][START] == source_requests, "Original page was requested again")

    def original(label, tabs):
        browser.wait(label, lambda: (
            browser.tab_count() == tabs and browser.selected() == 0
            and browser.page_panel() == source_panel and browser.address() == source_url
            and browser.has_page_title(PAGES[START])
        ), 6)
        unchanged_source()
        checks[label] = True

    def foreground(label):
        def new_panel():
            panel = browser.page_panel()
            if (browser.tab_count() == 2 and browser.selected() == 1
                    and loaded_panels(browser) == 2 and panel and panel != source_panel):
                return panel

        panel = browser.wait(label, new_panel, 6)
        unchanged_source()
        checks[label] = True
        return panel

    def edit(draft):
        browser.command(FOCUS_ADDRESS)
        browser.edit_address(draft)
        require(browser.address() == draft, f"Expected current address edit to contain {draft!r}")

    def alt_enter(*, repeated=False):
        key = ALT_ENTER | ((1 << 30) if repeated else 0)
        browser.message(ADDRESS, WM_SYSKEYDOWN, VK_RETURN, key)
        browser.message(ADDRESS, WM_SYSCHAR, VK_RETURN, key)

    def quiet(label, pages, draft=None):
        deadline = time.monotonic() + 0.5

        def unchanged():
            require(browser.tab_count() == 1 and browser.selected() == 0
                    and browser.page_panel() == source_panel and browser.has_page_title(PAGES[START]),
                    f"{label}: original tab was not left active")
            require(site.snapshot()[0] == pages, f"{label}: a loopback page was requested")
            if draft is not None:
                require(browser.address() == draft, f"{label}: address draft changed")
            return time.monotonic() >= deadline

        browser.wait(label, unchanged, 2)
        checks[label] = True

    original("initial_source", 1)
    edit(target_url)
    before = site.snapshot()[0]
    require(not before[TARGET], "Editing requested the destination before submission")
    browser.command(GO_NEW_TAB)
    target_panel = foreground("command_180_new_foreground_tab")
    page(browser, TARGET, before)
    checks["destination_rendered"] = True
    select(browser, 0)
    original("switch_restores_source", 2)
    select(browser, 1)
    browser.wait("same destination tab after switching back", lambda: (
        browser.page_panel() == target_panel and browser.address() == target_url
    ))
    browser.command(CLOSE_TAB)
    original("close_restores_source", 1)

    # A valid repeat must be ignored. An initial Alt+Enter then submits the same draft.
    draft = "? " + TERM
    edit(draft)
    pages, connects = site.snapshot()
    alt_enter(repeated=True)
    quiet("repeated_alt_enter_ignored", pages, draft)
    require(site.snapshot()[1] == connects, "Repeated Alt+Enter started a search")
    alt_enter()
    foreground("alt_enter_search_new_foreground_tab")
    _, host = ENGINES[176]  # Fresh profile default is DuckDuckGo.
    started = browser.wait("search CONNECT through loopback proxy", lambda: site.snapshot()[1] - connects)
    require(set(started) == {host}, f"Explicit search reached the wrong host: {dict(started)}")
    checks["search_connect_hosts"] = dict(started)
    browser.command(CLOSE_TAB)
    original("close_search_restores_source", 1)

    edit("")
    pages = site.snapshot()[0]
    browser.command(GO_NEW_TAB)
    quiet("empty_draft_did_not_open_tab", pages)
    unchanged_source()


def run(exe, profile, report):
    with proxy() as site:
        try:
            seed_profile(profile, site.origin)
            args = [f"--flit-profile={profile}", f"--flit-url={site.origin}{START}"]
            with OwnedProcess(Win32(), exe, args, report["launch"]) as process:
                browser = Window(process, site)
                exercise(browser, report["checks"])
                process.normal_close(browser.hwnd)
        finally:
            pages, connects = site.snapshot()
            report.update(proxy_pages=dict(pages), proxy_connect_hosts=dict(connects))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, default=WORKSPACE / "work" / "windows-address-new-tab",
                        help="parent directory for the disposable profile")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    exe, work = args.exe.resolve(), args.work.resolve()
    if exe.name.casefold() != "flit.exe" or not exe.is_file() or not exe.with_name("Flit.dll").is_file():
        parser.error("--exe must point to Flit.exe beside Flit.dll")

    report = {"result": "fail", "exe": str(exe), "checks": {}, "launch": {}}
    try:
        report["dll_sha256"] = dll_sha256(exe)
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-address-new-tab-", dir=work)
        scratch_path = Path(scratch.name).resolve()
        require(scratch_path.parent == work, "Disposable profile directory is outside --work")
        report["scratch"] = str(scratch_path)
        try:
            run(exe, scratch_path / "profile", report)
        finally:
            clean_scratch(scratch, report)
            report["scratch_removed"] = not scratch_path.exists()
            report["dll_sha256_after"] = dll_sha256(exe)
        require(report["scratch_removed"] and "scratch_cleanup_error" not in report,
                "Disposable test profile was not removed")
        require(report["dll_sha256"] == report["dll_sha256_after"], "Staged Flit.dll changed during the test")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
