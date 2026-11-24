"""Exercise split view with loopback pages and a disposable Flit profile."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

sys.dont_write_bytecode = True
from windows_free_memory import IDYES, WM_COMMAND, confirm
from windows_multi_session import clean_scratch
from windows_session import select, send
from windows_shortcuts import GuiThreadInfo
from windows_smoke import (
    BACK, FORWARD, NEW_TAB, SIDEBAR_TOGGLE, TABS, WORKSPACE, Browser, OwnedProcess, Win32,
    api, fixture, page_counts, require,
)
from windows_tab_order import MK_LBUTTON, TAB_MENU_ACTION, WM_LBUTTONDOWN, WM_LBUTTONUP, row_point
from windows_tab_unload import TAB_UNLOAD


TAB_CLOSE = 4
TAB_SPLIT_WITH = 11
SPLIT_VIEW, SPLIT_DIVIDER = 213, 214
WM_KEYDOWN, VK_RIGHT = 0x0100, 0x27
LEFT_KEY, RIGHT_KEY = 1, 2  # The first two allocated tab keys in the fresh window.
BACKGROUND_KEY = 3


def visible_pages(browser):
    # The single-pane Browser.page_panel() helper is ambiguous in split view.
    win = browser.win
    return {panel: view for panel in win.visible_children(browser.hwnd, "Static")
            if (view := win.cef_view(panel)) and win.visible(view)}


def click_row(browser, index):
    win, tabs = browser.win, browser.controls[TABS]
    require(win.pid(tabs) == browser.process.pid and win.visible(tabs), "Tab ListBox is unavailable")
    x, y = row_point(browser, index, 2, win.check(win.dpi(tabs)))
    point = x | (y << 16)
    browser.message(TABS, WM_LBUTTONDOWN, MK_LBUTTON, point)
    browser.message(TABS, WM_LBUTTONUP, 0, point)


def exercise(browser, checks):
    win, site, pid = browser.win, browser.site, browser.process.pid
    user = C.WinDLL("user32", use_last_error=True)
    gui_info = api(user, "GetGUIThreadInfo", W.BOOL, W.DWORD, C.POINTER(GuiThreadInfo))
    is_child = api(user, "IsChild", W.BOOL, W.HWND, W.HWND)
    owner = W.DWORD()
    thread = win.get_pid(browser.hwnd, C.byref(owner))
    require(thread and owner.value == pid, "Could not identify the owned Flit GUI thread")

    def active(label, row, page, path, *, back, forward=False, focused=False):
        trace = checks[label] = {}

        def matches():
            trace.update(row=browser.selected(), address=browser.address(),
                         back=bool(win.enabled(browser.controls[BACK])),
                         forward=bool(win.enabled(browser.controls[FORWARD])))
            if focused:
                info = GuiThreadInfo(C.sizeof(GuiThreadInfo))
                trace["focus"] = info.focus if gui_info(thread, C.byref(info)) else None
                focus = trace["focus"]
                if not (focus and win.pid(focus) == pid and is_child(page[0], focus)):
                    return False
            return (trace["row"], trace["address"], trace["back"], trace["forward"]) == (
                row, site.origin + path, back, forward,
            )

        browser.wait(label, matches)

    def layout(label, pages, matches):
        trace = checks[label] = {}
        expected = dict(pages)

        def fit():
            shown = visible_pages(browser)
            trace["visible_hwnds"] = list(shown.items())
            if shown != expected:
                return None
            bounds = tuple(browser.in_client(view) for _, view in pages)
            trace["bounds_px"] = bounds
            return bounds if (all(browser.in_client(panel) == rect
                                  for (panel, _), rect in zip(pages, bounds)) and matches(bounds)) else None

        return browser.wait(label, fit)

    def current_page(label):
        def single():
            pages = visible_pages(browser)
            return next(iter(pages.items())) if len(pages) == 1 else None

        panel, view = browser.wait(label, single)
        require(win.pid(panel) == win.pid(view) == pid, f"{label}: CEF window is not owned")
        return panel, view

    def new_page(row, path):
        browser.click(NEW_TAB)
        browser.wait(f"new tab selected at row {row}", lambda: browser.selected() == row)
        browser.navigate(path)
        return current_page(f"CEF viewport for {path}")

    def no_reload(label, before):
        after = {path: page_counts(site, path) for path in before}
        require(after == before, f"{label} reloaded or restored a page: {before} -> {after}")

    def unload_visible(key):
        try:
            send(browser, TAB_MENU_ACTION, key, TAB_UNLOAD)
        except RuntimeError as error:
            raise RuntimeError(f"Manual unload of visible tab key {key} failed to return: {error}") from error

    browser.page("/start", require_request=True)
    browser.navigate("/second")
    left = current_page("initial left CEF viewport")
    active("left has Back history before split", 0, left, "/second", back=True)
    right = new_page(1, "/tab")
    background = new_page(2, "/restart")
    require(len({page[0] for page in (left, right, background)}) == 3
            and len({page[1] for page in (left, right, background)}) == 3,
            "The fixture must have three distinct panel and CEF HWNDs")
    select(browser, 0)
    full, = layout("initial full viewport", (left,), lambda b: b[0][0] < b[0][2] and b[0][1] < b[0][3])
    paths = ("/start", "/second", "/tab", "/first", "/restart")
    before = {path: page_counts(site, path) for path in paths}

    def split(label, *, moved_right_from=None):
        def matches(bounds):
            left_rect, right_rect = bounds
            left_edge, top, right_edge, bottom = full
            left_end, right_start = left_rect[2], right_rect[0]
            aligned = (left_edge == left_rect[0] < left_end <= right_start < right_rect[2] == right_edge
                       and (left_rect[1], left_rect[3]) == (right_rect[1], right_rect[3]) == (top, bottom))
            return aligned and (moved_right_from is None
                                or (left_end > moved_right_from[0][2] and right_start > moved_right_from[1][0]))

        return layout(label, (left, right), matches)

    def unsplit(label, page):
        layout(label, (page,), lambda bounds: bounds == (full,))

    send(browser, TAB_MENU_ACTION, RIGHT_KEY, TAB_SPLIT_WITH)
    split("original CEF HWNDs laid out side by side")
    active("source tab stays active", 0, left, "/second", back=True)
    user = C.WinDLL("user32", use_last_error=True)
    is_iconic = api(user, "IsIconic", W.BOOL, W.HWND)
    show_window = api(user, "ShowWindowAsync", W.BOOL, W.HWND, C.c_int)
    win.check(show_window(browser.hwnd, 6))  # SW_MINIMIZE
    try:
        browser.wait("minimized window hides both Chrome widgets", lambda: is_iconic(browser.hwnd) and
                     all(not win.get_style(view, -16) & 0x10000000 for _, view in (left, right)))
    finally:
        win.check(show_window(browser.hwnd, 9))  # SW_RESTORE
    browser.wait("split window restored", lambda: not is_iconic(browser.hwnd))
    split("restore shows the same split widgets")
    no_reload("Minimizing and restoring both panes", before)
    unload_visible(RIGHT_KEY)

    click_row(browser, 1)
    active("right tab selection updates address, Back and focus", 1, right, "/tab", back=False, focused=True)
    split("selecting right keeps both pages visible")
    unload_visible(LEFT_KEY)
    # An eligible third tab proves bulk unload actually ran.
    confirm(browser, IDYES)
    browser.wait("bulk unload releases the hidden third CEF view",
                 lambda: not win.cef_view(background[0]) and win.pid(background[1]) != pid)
    require(browser.tab_count() == 3, "Bulk unload changed the tab count")
    before_resize = split("manual memory commands preserve both split views")
    active("manual memory commands keep the right pane selected", 1, right, "/tab", back=False)

    divider = win.get_item(browser.hwnd, SPLIT_DIVIDER)
    require(divider and win.pid(divider) == pid and win.visible(divider),
            "Expected visible owned split divider control 214")
    win.message(divider, WM_KEYDOWN, VK_RIGHT)
    split("Right Arrow on the divider grows left and shrinks right", moved_right_from=before_resize)
    active("divider resize keeps the right pane selected", 1, right, "/tab", back=False)
    original = split("both panes before hiding the vertical sidebar")
    browser.click(SIDEBAR_TOGGLE)
    layout("both panes grow when the vertical sidebar is hidden", (left, right), lambda bounds: (
        not win.visible(browser.controls[TABS])
        and bounds[0][0] < original[0][0]
        and bounds[1][2] == original[1][2]
        and bounds[0][2] < bounds[1][0]
    ))
    browser.click(SIDEBAR_TOGGLE)
    layout("reopening the vertical sidebar restores both pane bounds", (left, right), lambda b: b == original)
    no_reload("Splitting, selecting, unloading memory or resizing", before)

    left_counts = {path: page_counts(site, path) for path in ("/start", "/second")}
    browser.navigate("/first")
    active("right has its own Back history", 1, right, "/first", back=True)
    browser.page("/tab", lambda: browser.click(BACK))
    active("right Back returns to its own page", 1, right, "/tab", back=False, forward=True)
    split("right navigation keeps both original CEF views")
    no_reload("Right navigation", left_counts)

    right_counts = {path: page_counts(site, path) for path in ("/tab", "/first")}
    click_row(browser, 0)
    active("left tab selection restores address, Back and focus", 0, left, "/second", back=True, focused=True)
    split("selecting left keeps both pages visible")
    browser.page("/start", lambda: browser.click(BACK))
    active("left Back returns to its own page", 0, left, "/start", back=False, forward=True)
    split("left navigation keeps both original CEF views")
    no_reload("Left selection or navigation", right_counts)

    before = {path: page_counts(site, path) for path in paths}
    click_row(browser, 1)
    active("right history survives left navigation", 1, right, "/tab", back=False, forward=True, focused=True)
    send(browser, WM_COMMAND, SPLIT_VIEW)
    unsplit("toggle out expands the selected right page", right)
    active("right remains selected after toggle out", 1, right, "/tab", back=False, forward=True)
    click_row(browser, 0)
    unsplit("left page uses the same full bounds after toggle out", left)
    active("left keeps its history after toggle out", 0, left, "/start", back=False, forward=True, focused=True)

    send(browser, TAB_MENU_ACTION, RIGHT_KEY, TAB_SPLIT_WITH)
    split("re-entering split reuses the two original CEF views")
    send(browser, TAB_MENU_ACTION, RIGHT_KEY, TAB_CLOSE)
    browser.wait("closing the right pane removes its tab and CEF view",
                 lambda: browser.tab_count() == 2 and win.pid(right[1]) != pid)
    unsplit("closing the right pane expands the left survivor", left)
    active("left stays selected with its history after close", 0, left, "/start", back=False, forward=True)
    no_reload("Selecting, toggling split or closing the other pane", before)
    send(browser, TAB_MENU_ACTION, BACKGROUND_KEY, TAB_SPLIT_WITH)
    browser.wait("split loads the previously unloaded tab", lambda: len(visible_pages(browser)) == 2)
    divider = win.get_item(browser.hwnd, SPLIT_DIVIDER)
    require(win.visible(divider), "New split divider is not visible")
    win.message(divider, WM_KEYDOWN, VK_RIGHT)


def check_restored(browser, result):
    browser.wait("both restored split pages load", lambda: len(visible_pages(browser)) == 2
                 and browser.address() == browser.site.origin + "/start")
    with browser.win.window_coordinates(browser.hwnd):
        bounds = sorted(browser.in_client(view) for view in visible_pages(browser).values())
        left, right = bounds
        left_width, right_width = left[2] - left[0], right[2] - right[0]
        require(left[2] < right[0] and left[1::2] == right[1::2], "Restored panes overlap or are misaligned")
        require(abs(left_width / (left_width + right_width) - 0.55) < 0.01,
                "The saved divider position was not restored")
        require(browser.selected() == 0 and browser.tab_count() == 2, "The saved focus or tab count changed")
        result["restored_bounds_px"] = bounds


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", required=True, type=Path, help="scratch directory inside the checkout")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Split view verification requires an interactive Windows desktop")

    report = {"result": "fail", "checks": {}, "launch": {}}
    try:
        exe, work = args.exe.resolve(strict=True), args.work.resolve()
        require(exe.name.casefold() == "flit.exe" and exe.is_file(), f"Missing staged Flit.exe: {exe}")
        require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="split-view-", dir=work)
        try:
            profile = Path(scratch.name) / "profile"
            with fixture() as site:
                launch_args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"]
                with OwnedProcess(Win32(), exe, launch_args, report["launch"]) as process:
                    browser = Browser(process, site)
                    with browser.win.window_coordinates(browser.hwnd):
                        exercise(browser, report["checks"])
                    process.normal_close(browser.hwnd)
                state = json.loads((profile / "session.json").read_text(encoding="utf-8"))
                report["saved_split"] = state.get("split")
                require(report["saved_split"] == {"left": 0, "right": 1, "ratio": 550},
                        "Saved profile lost the panes, order or divider ratio")
                report["restore_launch"] = {}
                with OwnedProcess(Win32(), exe, launch_args[:1], report["restore_launch"]) as process:
                    browser = Browser(process, site)
                    check_restored(browser, report)
                    process.normal_close(browser.hwnd)
        finally:
            clean_scratch(scratch, report)
        require("scratch_cleanup_error" not in report, "Could not clean the disposable profile")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
