"""Verify sidebar bookmark activation and session restore in an owned Flit window."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
from unittest.mock import patch

sys.dont_write_bytecode = True
from windows_bookmark_transfer import native_bookmarks
from windows_bookmarks import SAVE_BOOKMARK, wait_manager
from windows_multi_session import clean_scratch
from windows_session import titles
from windows_smoke import GO, OwnedProcess, TABS, WORKSPACE, Win32, require
from windows_tab_order import LB_GETITEMRECT, LB_ITEMFROMPOINT, MK_LBUTTON, WM_LBUTTONDOWN, WM_LBUTTONUP
from windows_windows import CLOSE_TAB, Window, fixture, seed_profile


ORDINARY, BOOKMARK, INSIDE_BOOKMARK = "normal-seed", "normal-peer", "normal-after-private"
BOOKMARK_LIST = 194
LB_GETCURSEL, LB_GETCOUNT = 0x0188, 0x018B
GWL_STYLE, LBS_NOSEL = -16, 0x4000


def loads(site, page):
    path = site.paths[page]
    return len(site.seen(path)), len(site.seen(path + "/ready"))


def active_view(browser, page, *, after=None):
    site, win = browser.site, browser.win

    def ready():
        if after is not None and not all(now > before for now, before in zip(loads(site, page), after)):
            return None
        if browser.address() != site.origin + site.paths[page] or not browser.has_page_title(site.title(page)):
            return None
        panel = browser.page_panel()
        view = win.cef_view(panel) if panel else None
        if view and win.visible(view):
            require(win.pid(panel) == win.pid(view) == browser.process.pid, "CEF view is no longer owned")
            return panel, view

    return browser.wait(f"{page} address, title and visible CEF view", ready)


def bind_bookmark_list(browser):
    win = browser.win

    def ready():
        hwnd = win.get_item(browser.hwnd, BOOKMARK_LIST)
        if hwnd and win.visible(hwnd):
            require(win.pid(hwnd) == browser.process.pid and win.class_name(hwnd).casefold() == "listbox",
                    "HWND 194 is not an owned bookmark ListBox")
            if win.message(hwnd, LB_GETCOUNT) == 1:
                return hwnd

    browser.controls[BOOKMARK_LIST] = browser.wait("one visible bookmark in ListBox 194", ready)


def click_first_row(browser, control, *, close=False):
    win, hwnd = browser.win, browser.controls[control]
    browser.process.alive()
    require(win.pid(hwnd) == browser.process.pid and win.visible(hwnd) and win.enabled(hwnd),
            f"ListBox {control} is not owned, visible and enabled")
    with win.window_coordinates(hwnd):
        row = W.RECT()
        require(browser.message(control, LB_GETITEMRECT, 0, C.addressof(row)) != -1,
                f"Could not get row 0 of ListBox {control}")
        _, _, width, height = win.rect(hwnd, client=True)
        x = row.right - 10 if close else row.left + (row.right - row.left) // 4
        y = (row.top + row.bottom) // 2
        require(row.left <= x < row.right and row.top <= y < row.bottom
                and 0 <= x < min(width, 32768) and 0 <= y < min(height, 32768),
                f"Row 0 of ListBox {control} has no visible click point")
        point = x | (y << 16)
        require(browser.message(control, LB_ITEMFROMPOINT, 0, point) == 0,
                f"Click point missed row 0 of ListBox {control}")
        win.check(win.post(hwnd, WM_LBUTTONDOWN, MK_LBUTTON, point))
        win.check(win.post(hwnd, WM_LBUTTONUP, 0, point))


def check_sidebar(browser, check, *, bookmark_active=False):
    check["ordinary_rows"] = titles(browser)
    check["bookmark_count"] = browser.message(BOOKMARK_LIST, LB_GETCOUNT)
    require(check["ordinary_rows"] == (browser.site.title(ORDINARY),),
            f"ListBox 108 should contain only the ordinary tab: {check['ordinary_rows']}")
    require(check["bookmark_count"] == 1, "ListBox 194 should retain the saved bookmark")
    if bookmark_active:
        if browser.win.get_style(browser.controls[BOOKMARK_LIST], GWL_STYLE) & LBS_NOSEL:
            check["bookmark_selected"] = "unavailable (LBS_NOSEL)"
        else:
            browser.wait("bookmark row 0 selected in ListBox 194",
                         lambda: browser.message(BOOKMARK_LIST, LB_GETCURSEL) == 0)
            check["bookmark_selected"] = True


def switch_and_reselect(browser, bookmark_view, check, page=BOOKMARK):
    before = loads(browser.site, page)
    click_first_row(browser, TABS)
    ordinary_view = active_view(browser, ORDINARY)
    check_sidebar(browser, check.setdefault("ordinary", {"cef": ordinary_view}))
    panel, view = bookmark_view
    win = browser.win
    require(win.pid(panel) == win.pid(view) == browser.process.pid
            and win.cef_view(panel) == view and not win.visible(view),
            "Switching to the ordinary tab did not keep the original bookmark CEF view alive and hidden")

    click_first_row(browser, BOOKMARK_LIST)
    current = active_view(browser, page)
    after = loads(browser.site, page)
    check.update(cef=current, loads_before=before, loads_after=after)
    require(current == bookmark_view, "Re-clicking the bookmark replaced its CEF view")
    require(after == before, "Re-clicking the bookmark reloaded its document or reran the ready probe")
    check_sidebar(browser, check, bookmark_active=True)
    return ordinary_view


def exercise(exe, profile, site, report):
    seed_profile(profile, site.origin)
    args = [f"--flit-profile={profile}"]
    bookmark_url = site.origin + site.paths[BOOKMARK]
    ordinary_url = site.origin + site.paths[ORDINARY]
    win, launches, checks = Win32(), report["launches"], report["checks"]

    with OwnedProcess(win, exe, [*args, f"--flit-url={bookmark_url}"], launches.setdefault("seed", {})) as process:
        browser = Window(process, site)
        active_view(browser, BOOKMARK, after=(0, 0))
        browser.command(SAVE_BOOKMARK)
        wait_manager(browser, "Bookmarks — Page saved")
        process.normal_close(browser.hwnd)
    saved = report["saved_bookmarks"] = native_bookmarks(profile)
    require(len(saved) == 1 and saved[0]["url"] == bookmark_url and saved[0]["title"] == site.title(BOOKMARK),
            f"Expected exactly one saved fixture bookmark, got {saved}")

    before_bookmark = loads(site, BOOKMARK)
    with OwnedProcess(win, exe, [*args, f"--flit-url={ordinary_url}"], launches.setdefault("activate", {})) as process:
        browser = Window(process, site)
        ordinary_view = active_view(browser, ORDINARY, after=(0, 0))
        bind_bookmark_list(browser)
        check_sidebar(browser, checks.setdefault("initial", {"cef": ordinary_view}))
        require(loads(site, BOOKMARK) == before_bookmark, "Bookmark loaded before its first sidebar click")

        click_first_row(browser, BOOKMARK_LIST)
        bookmark_view = active_view(browser, BOOKMARK, after=before_bookmark)
        require(bookmark_view[0] != ordinary_view[0] and bookmark_view[1] != ordinary_view[1],
                "Bookmark activation reused the ordinary tab's CEF view")
        check_sidebar(browser, checks.setdefault("activated", {"cef": bookmark_view}), bookmark_active=True)
        require(switch_and_reselect(browser, bookmark_view, checks.setdefault("reselected", {})) == ordinary_view,
                "Switching back did not retain the original ordinary CEF view")

        browser.edit_address(site.origin + site.paths[INSIDE_BOOKMARK])
        browser.click(GO)
        navigated = active_view(browser, INSIDE_BOOKMARK, after=(0, 0))
        require(switch_and_reselect(browser, navigated, checks.setdefault("navigation_retained", {}),
                                    INSIDE_BOOKMARK) == ordinary_view,
                "Navigating within a bookmark changed the ordinary CEF view")

        browser.command(CLOSE_TAB)  # Ctrl+W dispatches WM_COMMAND 106.
        require(active_view(browser, ORDINARY) == ordinary_view, "Ctrl+W did not return to the ordinary view")
        browser.wait("closed bookmark CEF view destroyed", lambda: win.pid(bookmark_view[1]) != process.pid)
        check_sidebar(browser, checks.setdefault("closed", {"cef": ordinary_view}))

        before_reopen = loads(site, BOOKMARK)
        click_first_row(browser, BOOKMARK_LIST)
        reopened = active_view(browser, BOOKMARK, after=before_reopen)
        check_sidebar(browser, checks.setdefault("reopened", {"cef": reopened}), bookmark_active=True)
        process.normal_close(browser.hwnd)

    before_restore = loads(site, BOOKMARK)
    with OwnedProcess(win, exe, args, launches.setdefault("restore", {})) as process:
        browser = Window(process, site)
        restored = active_view(browser, BOOKMARK, after=before_restore)
        bind_bookmark_list(browser)
        check_sidebar(browser, checks.setdefault("restored", {"cef": restored}), bookmark_active=True)
        switch_and_reselect(browser, restored, checks.setdefault("restored_reselected", {}))
        process.normal_close(browser.hwnd)
    before_adoption = loads(site, BOOKMARK)
    with OwnedProcess(win, exe, [*args, f"--flit-url={bookmark_url}"], launches.setdefault("already_open", {})) as process:
        browser = Window(process, site)
        same_view = active_view(browser, BOOKMARK, after=before_adoption)
        requests = loads(site, BOOKMARK)
        bind_bookmark_list(browser)
        require(browser.tab_count() == 1, "Expected one ordinary view before adoption")
        click_first_row(browser, BOOKMARK_LIST)
        browser.wait("ordinary view moved to bookmark list",
                     lambda: browser.tab_count() == 0 and browser.message(BOOKMARK_LIST, LB_GETCURSEL) == 0)
        require(active_view(browser, BOOKMARK) == same_view and loads(site, BOOKMARK) == requests,
                "Opening an already loaded bookmarked address created a second view or reloaded it")
        click_first_row(browser, BOOKMARK_LIST, close=True)
        browser.wait("window stays open with a blank tab after the last bookmark page closes",
                     lambda: not browser.address() and browser.tab_count() == 1
                     and browser.message(BOOKMARK_LIST, LB_GETCURSEL) == -1)
        require(browser.message(BOOKMARK_LIST, LB_GETCOUNT) == 1, "The close button removed the saved bookmark")
        checks["existing_tab_reused_and_closed"] = True
        process.normal_close(browser.hwnd)

    require(native_bookmarks(profile) == saved, "Closing, reopening or restarting changed the saved bookmark")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="workspace parent for the disposable profile")
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    if (sys.platform != "win32" or exe.name.casefold() != "flit.exe"
            or not exe.is_file() or not exe.with_name("Flit.dll").is_file()):
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")
    if not work.is_relative_to(WORKSPACE):
        parser.error(f"--work must be inside {WORKSPACE}")

    report = {"result": "fail", "exe": str(exe), "launches": {}, "checks": {}}
    try:
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-sidebar-bookmark-", dir=work)
        try:
            root = Path(scratch.name).resolve(strict=True)
            require(root.parent == work, "Unexpected disposable profile parent")
            with patch.dict(os.environ, LOCALAPPDATA=str(root / "localappdata")), fixture() as site:
                try:
                    exercise(exe, root / "profile", site, report)
                finally:
                    report["page_loads"] = {page: loads(site, page) for page in (ORDINARY, BOOKMARK, INSIDE_BOOKMARK)}
        finally:
            clean_scratch(scratch, report)
        require("scratch_cleanup_error" not in report, "Could not remove the disposable profile")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2, ensure_ascii=True))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
