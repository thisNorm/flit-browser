"""Check keyboard and mouse use of Exact address suggestions on a disposable profile."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import json
from pathlib import Path
from tempfile import TemporaryDirectory

from windows_bookmarks import SAVE_BOOKMARK, wait_manager
from windows_exact_ui import popups, row, surfaces, watch
from windows_multi_session import clean_scratch
from windows_session import select
from windows_shortcuts import GuiThreadInfo
from windows_smoke import (
    ADDRESS, NEW_TAB, TAB_TITLE, WORKSPACE, Browser, OwnedProcess, Win32, api, fixture, require,
)


FOCUS_ADDRESS = 113
WM_COMMAND = 0x0111
BUTTON = "ControlType.Button"


def address_has_focus(browser):
    info = GuiThreadInfo(C.sizeof(GuiThreadInfo))
    thread = browser.win.get_pid(browser.hwnd, None)
    gui_info = api(C.WinDLL("user32", use_last_error=True), "GetGUIThreadInfo", W.BOOL, W.DWORD,
                   C.POINTER(GuiThreadInfo))
    return thread and gui_info(thread, C.byref(info)) and info.focus == browser.controls[ADDRESS]


def type_address(browser, text):
    browser.win.message(browser.hwnd, WM_COMMAND, FOCUS_ADDRESS)
    browser.wait("address edit has keyboard focus", lambda: address_has_focus(browser))
    browser.edit_address(text)


def suggestion(browser, title):
    popup = browser.wait("suggestions popup below the address field", lambda: next(iter(popups(browser)), None))
    child = browser.wait("Exact suggestions surface", lambda: next(surfaces(browser, popup), None))
    tree = watch(browser, child, (title,), title, lambda tree: row(tree, title, BUTTON))
    return popup, child, row(tree, title, BUTTON)


def click_suggestion(browser, child, item):
    win = browser.win
    with win.window_coordinates(child):
        left, top, _, _ = win.rect(child)
        x, y, width, height = item["rect"]
        px, py = int(x + width / 2 - left), int(y + height / 2 - top)
        _, _, right, bottom = win.rect(child, client=True)
        require(0 < px < right and 0 < py < bottom, "Suggestion is outside its Exact surface")
        point = px | (py << 16)
        win.message(child, 0x0201, 1, point)  # WM_LBUTTONDOWN, MK_LBUTTON
        require(address_has_focus(browser), "Clicking a suggestion stole the address focus")
        require(win.visible(child), "Clicking a suggestion dismissed it before invocation")
        win.message(child, 0x0202, 0, point)  # WM_LBUTTONUP


def check(exe, scratch, result):
    win = Win32()
    profile = scratch / "profile"
    with fixture() as site, OwnedProcess(win, exe, [f"--flit-profile={profile}"], {}) as process:
        browser = Browser(process, site)
        browser.navigate("/start")
        browser.wait("first page title", lambda: browser.has_page_title("Flit smoke: start"))
        browser.click(NEW_TAB)
        browser.wait("second tab selected", lambda: browser.selected() == 1)
        browser.navigate("/tab")
        browser.wait("second page title", lambda: browser.has_page_title(TAB_TITLE))

        result["stage"] = "existing tab by keyboard"
        type_address(browser, "smoke start")
        popup, _, _ = suggestion(browser, "Flit smoke: start")
        browser.press(ADDRESS, 0x1B)  # Escape
        browser.wait("Escape closes suggestions", lambda: not win.visible(popup))
        require(browser.address() == "smoke start" and address_has_focus(browser),
                "Escape changed the draft or lost address focus")

        browser.edit_address("smoke star")
        suggestion(browser, "Flit smoke: start")
        before = site.count("/start")
        browser.press(ADDRESS, 0x28)  # Down
        require(browser.selected() == 1 and site.count("/start") == before,
                "Moving the highlight activated or reloaded the candidate")
        browser.press(ADDRESS, 0x0D)  # Enter
        browser.wait("Enter switches to the already loaded tab", lambda: (
            browser.address() == site.origin + "/start" and browser.selected() == 0 and not popups(browser)
        ))
        require(browser.tab_count() == 2 and site.count("/start") == before,
                "Deferred suggestions duplicated or reloaded an existing tab")
        result["tab_by_keyboard"] = True

        result["stage"] = "saved bookmark by mouse"
        browser.navigate("/second")
        browser.wait("bookmark page title", lambda: browser.has_page_title("Flit smoke: second"))
        browser.win.message(browser.hwnd, WM_COMMAND, SAVE_BOOKMARK)
        wait_manager(browser, "Bookmarks — Page saved")
        select(browser, 0)
        browser.navigate("/start")
        # Chromium writes its bookmark file after the WebUI operation returns.
        bookmark_file = profile / "Default" / "Bookmarks"
        browser.wait("bookmarked address is persisted", lambda: bookmark_file.exists()
                     and site.origin + "/second" in bookmark_file.read_text(encoding="utf-8"), timeout=12)
        type_address(browser, "smoke second")
        _, child, item = suggestion(browser, "Flit smoke: second")
        browser.page("/second", lambda: click_suggestion(browser, child, item))
        require(not popups(browser), "Bookmark selection left the popup open")
        result["bookmark_by_mouse"] = True

        result["stage"] = "explicit search"
        type_address(browser, "smoke orchid")
        popup, _, _ = suggestion(browser, TAB_TITLE)
        browser.edit_address("? smoke orchid")
        browser.wait("explicit search dismisses matching tab suggestions", lambda: not win.visible(popup))
        process.normal_close(browser.hwnd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--work", required=True, type=Path)
    args = parser.parse_args()
    work = args.work.resolve()
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    work.mkdir(parents=True, exist_ok=True)
    result = {"result": "fail"}
    scratch = TemporaryDirectory(prefix="address-suggestions-", dir=work)
    try:
        try:
            check(args.exe.resolve(strict=True), Path(scratch.name), result)
        finally:
            clean_scratch(scratch, result)
        require("scratch_cleanup_error" not in result, "Could not clean the disposable profile")
        result["result"] = "pass"
    except Exception as error:
        result["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(result, indent=2))
    return int(result["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
