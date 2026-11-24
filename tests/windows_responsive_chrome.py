"""Check the Win32 chrome at 530, 700 and 940 client DIPs on the current monitor."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import time

sys.dont_write_bytecode = True
from windows_multi_session import clean_scratch
from windows_shortcuts import FOCUS_ADDRESS, GuiThreadInfo, VK_TAB
from windows_smoke import (
    ADDRESS, BACK, FORWARD, GO, NEW_TAB, RELOAD, SIDEBAR_TOGGLE, TABS, TAB_FILTER,
    Browser, OwnedProcess, Win32, api, capture_window, fixture, require,
)


# Native control and command IDs from src/window.rs.
MENU, FIND, FIND_TEXT, FIND_NEXT, FIND_PREVIOUS, FIND_CLOSE = 117, 124, 125, 127, 128, 129
BOOKMARK, SITE_INFO, DOWNLOADS, EXTENSIONS, VAULT = 160, 166, 190, 200, 215
CONTROL_NAMES = {
    SIDEBAR_TOGGLE: "sidebar toggle", BACK: "back", FORWARD: "forward", RELOAD: "reload",
    SITE_INFO: "site info", ADDRESS: "address", BOOKMARK: "bookmark", GO: "Go",
    NEW_TAB: "new tab", EXTENSIONS: "extensions", VAULT: "vault", DOWNLOADS: "downloads", MENU: "menu",
    FIND_TEXT: "find edit", FIND_PREVIOUS: "find previous", FIND_NEXT: "find next", FIND_CLOSE: "find close",
}
NAV_START = (SIDEBAR_TOGGLE, BACK, FORWARD, RELOAD, SITE_INFO, ADDRESS, BOOKMARK, GO)
NAV_END = (EXTENSIONS, VAULT, DOWNLOADS, MENU)
FIND_ROW = (FIND_TEXT, FIND_PREVIOUS, FIND_NEXT, FIND_CLOSE)
WIDTHS = (530, 700, 940)
WM_COMMAND, WM_CHAR, EM_GETMARGINS = 0x0111, 0x0102, 0x00D4


def intersects(a, b):
    return max(a[0], b[0]) < min(a[2], b[2]) and max(a[1], b[1]) < min(a[3], b[3])


class ResponsiveBrowser(Browser):
    def __init__(self, process, site):
        super().__init__(process, site)
        win = self.win
        user = C.WinDLL("user32", use_last_error=True)
        self.child_at = api(user, "ChildWindowFromPointEx", W.HWND, W.HWND, W.POINT, W.UINT)
        self.gui_info = api(user, "GetGUIThreadInfo", W.BOOL, W.DWORD, C.POINTER(GuiThreadInfo))
        awareness = api(user, "GetAwarenessFromDpiAwarenessContext", C.c_int, C.c_void_p)
        require(awareness(win.window_dpi_context(self.hwnd)) == 2,
                "Flit HWND must be per-monitor DPI-aware to measure physical client pixels")
        self.dwm_attribute = api(C.WinDLL("dwmapi"), "DwmGetWindowAttribute", C.c_long,
                                 W.HWND, W.DWORD, C.c_void_p, W.DWORD)
        self.thread = win.check(win.get_pid(self.hwnd, None))
        for cid, name in CONTROL_NAMES.items():
            child = win.get_item(self.hwnd, cid)
            expected = "edit" if cid in (ADDRESS, FIND_TEXT) else "button"
            require(child and win.pid(child) == process.pid and win.class_name(child).casefold() == expected,
                    f"Missing owned {expected} for {name} (id {cid})")
            self.controls[cid] = child

    def resize_client(self, width_dip):
        win, hwnd = self.win, self.hwnd
        require(win.pid(hwnd) == self.process.pid, "Resize target is no longer owned")
        for _ in range(3):
            dpi = win.check(win.dpi(hwnd))
            target = (width_dip * dpi + 48) // 96
            width = win.rect(hwnd, client=True)[2]
            if width == target:
                return
            left, top, right, bottom = win.rect(hwnd)
            # Preserve height and position without activating or reordering the window.
            win.check(win.set_window_pos(hwnd, None, 0, 0, right - left + target - width, bottom - top, 0x0016))
        dpi = win.check(win.dpi(hwnd))
        width = win.rect(hwnd, client=True)[2]
        require(width == (width_dip * dpi + 48) // 96,
                f"Could not resize the client to {width_dip} DIP (got {width}px at {dpi} DPI)")

    def caption_bounds(self):
        bounds = W.RECT()
        if self.dwm_attribute(self.hwnd, 5, C.byref(bounds), C.sizeof(bounds)) < 0:  # DWMWA_CAPTION_BUTTON_BOUNDS
            return None
        left, top, right, bottom = self.in_client(self.hwnd)
        if (0 <= bounds.left < bounds.right <= right - left
                and 0 <= bounds.top < bounds.bottom <= bottom - top):
            return left + bounds.left, top + bounds.top, left + bounds.right, top + bounds.bottom
        return None

    def bounds(self, ids):
        return {CONTROL_NAMES[cid]: self.in_client(self.controls[cid]) for cid in ids}

    def settled_layout(self, width_dip, expanded, panel):
        win = self.win
        previous, stable_since = None, 0
        toolbar = (*NAV_START, NEW_TAB, *NAV_END)

        def poll():
            nonlocal previous, stable_since
            dpi = win.check(win.dpi(self.hwnd))
            client = win.rect(self.hwnd, client=True)
            if (client[2] != (width_dip * dpi + 48) // 96 or self.page_panel() != panel
                    or any(bool(win.visible(self.controls[cid])) != expanded for cid in (TABS, TAB_FILTER))):
                previous = None
                return None
            layout = {
                "dpi": dpi, "client_px": client, "caption_px": self.caption_bounds(),
                "page_px": self.in_client(panel), "controls_px": self.bounds(toolbar),
            }
            now = time.monotonic()
            if layout != previous:
                previous, stable_since = layout, now
            # Visibility changes before the sidebar finishes sliding; wait for stable bounds.
            return layout if now - stable_since >= 0.4 else None

        return self.wait(f"stable {width_dip} DIP client and {'expanded' if expanded else 'collapsed'} sidebar", poll)

    def target(self, cid, bounds, client, dpi, label):
        win, child = self.win, self.controls[cid]
        name = CONTROL_NAMES[cid]
        require(win.pid(child) == self.process.pid and win.visible(child), f"{label}: {name} is missing or hidden")
        require(cid in (BACK, FORWARD) or win.enabled(child), f"{label}: {name} is disabled")
        left, top, right, bottom = bounds[name]
        require(0 <= left < right <= client[2] and 0 <= top < bottom <= client[3],
                f"{label}: {name} {bounds[name]} is outside client {client}")
        if cid not in (ADDRESS, FIND_TEXT):
            require(min(right - left, bottom - top) * 96 >= 24 * dpi,
                    f"{label}: {name} {bounds[name]} is smaller than 24×24 DIP at {dpi} DPI")
        point = W.POINT((left + right) // 2, (top + bottom) // 2)
        require(self.child_at(self.hwnd, point, 0x0001) == child,  # CWP_SKIPINVISIBLE
                f"{label}: {name} is covered at client point ({point.x}, {point.y})")

    def check_row(self, ids, bounds, client, dpi, caption, label):
        names = [CONTROL_NAMES[cid] for cid in ids]
        row = [bounds[name] for name in names]
        require(max(r[1] for r in row) < min(r[3] for r in row), f"{label}: controls do not share a row: {row}")
        for first, second in zip(names, names[1:]):
            require(bounds[first][2] <= bounds[second][0],
                    f"{label}: {first} {bounds[first]} overlaps or follows {second} {bounds[second]}")
        for cid, name in zip(ids, names):
            self.target(cid, bounds, client, dpi, label)
            require(caption is None or not intersects(bounds[name], caption),
                    f"{label}: {name} {bounds[name]} overlaps the native caption buttons {caption}")

    def expect_focus(self, cid):
        info = GuiThreadInfo(C.sizeof(GuiThreadInfo))
        child = self.controls[cid]
        self.wait(f"focus on {CONTROL_NAMES[cid]} (id {cid})",
                  lambda: self.gui_info(self.thread, C.byref(info)) and info.focus == child, timeout=3)

    def tab_through(self, ids, trace):
        self.expect_focus(ids[0])
        trace.append(CONTROL_NAMES[ids[0]])
        for current, following in zip(ids, ids[1:]):
            self.press(current, VK_TAB)
            self.expect_focus(following)
            trace.append(CONTROL_NAMES[following])


def check_layout(browser, expanded, case):
    label = f"{case['width_dip']} DIP {case['sidebar']} at {case['dpi']} DPI"
    bounds, client, dpi, caption = (case[key] for key in ("controls_px", "client_px", "dpi", "caption_px"))
    stacked = caption is not None and bounds["sidebar toggle"][1] >= caption[3]
    tail = NAV_END if expanded or stacked else (NEW_TAB, *NAV_END)
    browser.check_row((*NAV_START, *tail), bounds, client, dpi, caption, label)
    if expanded or stacked:
        browser.target(NEW_TAB, bounds, client, dpi, label)
        add = bounds[CONTROL_NAMES[NEW_TAB]]
        require((caption is None or not intersects(add, caption)) and not intersects(add, case["page_px"])
                and all(not intersects(add, bounds[CONTROL_NAMES[cid]]) for cid in (*NAV_START, *NAV_END)),
                f"{label}: new-tab button {add} overlaps the toolbar, caption or page")
    if case["width_dip"] == WIDTHS[0]:
        require(case["address_text_px"] * 96 >= 110 * dpi,
                f"{label}: editable address is only {case['address_text_dip']} DIP wide (minimum 110)")


def exercise_minimum(browser, case, path):
    win = browser.win
    expected = browser.site.origin + path
    win.message(browser.hwnd, WM_COMMAND, FOCUS_ADDRESS)
    browser.expect_focus(ADDRESS)
    # Ctrl+L's command selects the old URL. WM_CHAR uses the native edit path.
    for char in expected:
        browser.message(ADDRESS, WM_CHAR, ord(char), 1)
    require(browser.address() == expected, f"Could not replace the full address with {expected}")
    browser.tab_through((ADDRESS, BOOKMARK, GO), case.setdefault("address_tab_order", []))
    browser.page(path, lambda: browser.click(GO), require_request=True)
    browser.wait(f"HTTP fixture title for {path}", lambda: browser.has_page_title(f"Flit smoke: {path[1:]}"))
    require(browser.tab_count() == 1, "Go opened a second tab")
    case["navigated_to"] = expected

    win.message(browser.hwnd, WM_COMMAND, FIND)
    case["find_controls_px"] = browser.bounds(FIND_ROW)
    browser.check_row(FIND_ROW, case["find_controls_px"], case["client_px"], case["dpi"], case["caption_px"],
                      f"{case['sidebar']} find bar")
    browser.tab_through(FIND_ROW, case.setdefault("find_tab_order", []))
    browser.click(FIND_CLOSE)
    browser.wait("find controls to hide", lambda: all(not win.visible(browser.controls[cid]) for cid in FIND_ROW))


def run(exe, profile, capture_dir, report):
    require(exe.is_file(), f"Flit executable does not exist: {exe}")
    win = Win32()
    with fixture() as site:
        args = [f"--flit-profile={profile}", "--flit-agent-launch", f"--flit-url={site.origin}/start"]
        try:
            with OwnedProcess(win, exe, args, report["launch"]) as process:
                browser = ResponsiveBrowser(process, site)
                browser.page("/start", require_request=True)
                panel = browser.wait("owned visible HTTP page panel", browser.page_panel)
                require(win.pid(panel) == process.pid, "HTTP page panel is not owned")
                with win.window_coordinates(browser.hwnd):
                    for width in WIDTHS:
                        browser.resize_client(width)
                        for expanded in (False, True):
                            mode = "expanded" if expanded else "collapsed"
                            case = {"width_dip": width, "sidebar": mode}
                            report["layouts"].append(case)
                            if bool(win.visible(browser.controls[TABS])) != expanded:
                                browser.click(SIDEBAR_TOGGLE)
                            case.update(browser.settled_layout(width, expanded, panel))
                            margins = browser.message(ADDRESS, EM_GETMARGINS)
                            inner_width = win.rect(browser.controls[ADDRESS], client=True)[2]
                            case["address_text_px"] = inner_width - (margins & 0xFFFF) - ((margins >> 16) & 0xFFFF)
                            case["address_text_dip"] = round(case["address_text_px"] * 96 / case["dpi"], 1)
                            if capture_dir:
                                case["capture"] = capture_window(browser, capture_dir / f"{width}-dip-{mode}.png")
                            check_layout(browser, expanded, case)
                            if width == WIDTHS[0]:
                                exercise_minimum(browser, case, "/second" if expanded else "/first")
                process.normal_close(browser.hwnd)
        finally:
            report["fixture"] = site.report()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", required=True, type=Path, help="parent directory for the disposable profile")
    parser.add_argument("--capture-dir", type=Path, help="write new owned-window PNGs here if PrintWindow works")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    report = {"result": "fail", "launch": {}, "layouts": []}
    try:
        work = args.work.resolve()
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="responsive-chrome-", dir=work)
        try:
            run(args.exe.resolve(), Path(scratch.name) / "profile", args.capture_dir, report)
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
