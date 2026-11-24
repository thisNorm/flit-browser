"""Check native tab memory controls on a Windows display."""

import argparse
import ctypes as C
from ctypes import wintypes as W
from hashlib import file_digest
import json
from pathlib import Path
import re
import sys
from tempfile import TemporaryDirectory

sys.dont_write_bytecode = True
from windows_close_cancel import LABELS, PATHS, answer, fixture
from windows_download_close import owned_windows, post
from windows_downloads import seed_profile
from windows_free_memory import FREE_MEMORY, IDNO, IDYES, SIDEBAR_STATUS, WM_COMMAND
from windows_multi_session import clean_scratch
from windows_session import loaded_panels, select, send, titles
from windows_smoke import Browser, NEW_TAB, OwnedProcess, SIDEBAR_TOGGLE, TABS, Win32, api, page_counts, require


FREE_BUTTON = 191
WM_MOUSEHOVER, WM_MOUSELEAVE = 0x02A1, 0x02A3
WM_LBUTTONDOWN, WM_LBUTTONUP, MK_LBUTTON = 0x0201, 0x0202, 0x0001
LB_GETITEMRECT, LB_ITEMFROMPOINT = 0x0198, 0x01A9
RENDERER = re.compile(r"[1-9][0-9]* MiB · (shared )?renderer")  # Memory-first engine shares renderers.
NO_RENDERER = "Not loaded · no renderer"


def control(browser, cid, class_name):
    win = browser.win
    hwnd = win.get_item(browser.hwnd, cid)
    require(hwnd and win.pid(hwnd) == browser.process.pid and win.class_name(hwnd).casefold() == class_name.casefold(),
            f"Missing owned native {class_name} control {cid}")
    browser.controls[cid] = hwnd
    return hwnd


def row_point(browser, index, dpi, *, unload=False):
    row = W.RECT()
    require(browser.message(TABS, LB_GETITEMRECT, index, C.addressof(row)) != -1,
            f"Could not read tab row {index}")
    # Hit the center of the unload glyph, left of the close zone (rightmost 28 DIP).
    x = row.right - (40 * dpi + 48) // 96 if unload else (row.left + row.right) // 2
    y = (row.top + row.bottom) // 2
    _, _, width, height = browser.win.rect(browser.controls[TABS], client=True)
    require(row.left <= x < row.right and row.top <= y < row.bottom
            and 0 <= x < min(width, 32768) and 0 <= y < min(height, 32768),
            f"Point ({x}, {y}) is outside visible tab row {index} at {dpi} DPI")
    point = x | (y << 16)
    require(browser.message(TABS, LB_ITEMFROMPOINT, 0, point) == index,
            f"Point ({x}, {y}) did not hit tab row {index}")
    return point


def confirmation(browser, trigger, caption, choice, unchanged):
    win, process = browser.win, browser.process
    require(not owned_windows(process, "#32770"), "An owned dialog was already open")
    trigger()  # Post the trigger so the modal loop cannot block the test.
    dialogs = browser.wait(f"owned {caption} confirmation", lambda: owned_windows(process, "#32770"))
    require(len(dialogs) == 1, f"Expected one owned confirmation, got {dialogs}")
    dialog = dialogs[0]
    get_window = api(C.WinDLL("user32", use_last_error=True), "GetWindow", W.HWND, W.HWND, W.UINT)
    require(win.pid(dialog) == process.pid and get_window(dialog, 4) == browser.hwnd,
            "Confirmation belongs to a different window")

    def text(hwnd):
        require(win.pid(hwnd) == process.pid, "Confirmation control is no longer owned")
        value = C.create_unicode_buffer(1024)
        win.message(hwnd, 0x000D, len(value), C.addressof(value))  # WM_GETTEXT
        return value.value

    actual = text(dialog)
    require(actual == caption, f"Expected {caption!r}, got {actual!r}")
    require(win.message(dialog, 0x0400) == 0x534B0007, f"{caption} did not default to No")  # DM_GETDEFID
    # Exclude the question icon; only the text should match between dialogs.
    message = " ".join(filter(None, (text(child) for child in win.visible_children(dialog, "Static")
                                    if win.get_style(child, -16) & 0x1F != 0x03)))  # SS_TYPEMASK, SS_ICON
    require(message, f"{caption} is missing its warning")
    unchanged(f"while {caption} is open")
    answer(browser, dialog, choice, {})
    browser.wait("confirmation dismissed", lambda: not win.visible(dialog))
    return message


def exercise(browser, report):
    win, process, site = browser.win, browser.process, browser.site
    button = control(browser, FREE_BUTTON, "Button")  # First gate: baseline must fail here.
    require(browser.control_text(FREE_BUTTON) == "Free memory…", "Wrong caption for Free memory button 191")
    status = control(browser, SIDEBAR_STATUS, "Static")
    tabs = browser.controls[TABS]
    report["dpi"] = dpi = win.dpi(tabs)
    require(dpi > 0, "Could not read the list's DPI")

    browser.wait("initial blank tab", lambda: browser.tab_count() == 1 and browser.selected() == 0)
    blank = titles(browser)[0]
    require(not loaded_panels(browser), "Initial blank tab should have no view")

    def child(panel):
        return win.find_child(panel, None, None, None)

    panels, views = [], []
    for index, (path, label) in enumerate(zip(PATHS, LABELS), start=1):
        browser.click(NEW_TAB)
        browser.wait(f"new tab {index} selected", lambda: browser.selected() == index)
        browser.navigate(path)
        browser.wait(f"title of {path}", lambda: browser.has_page_title(label))
        panel = browser.wait(f"view panel for {path}", browser.page_panel)
        view = browser.wait(f"attached view for {path}", lambda: child(panel))
        require(win.pid(view) == process.pid and panel not in panels, f"Wrong view for {path}")
        panels.append(panel)
        views.append(view)
    target, pinned, active = panels
    send(browser, 0x8005, 3, 1)  # TAB_MENU_ACTION, third tab's key (the second page), TAB_PIN.
    expected = (LABELS[1], blank, LABELS[0], LABELS[2])
    browser.wait("pinned row followed by blank, target and active", lambda: titles(browser) == expected)
    require(loaded_panels(browser) == 3, "Fixture did not load exactly three views")

    active_counts = page_counts(site, PATHS[2])

    def unchanged(step):
        require(titles(browser) == expected and browser.selected() == 3,
                f"{step}: changed tab count, order, labels or selection")
        require(browser.address() == site.origin + PATHS[2] and browser.page_panel() == active
                and child(active) == views[2] and page_counts(site, PATHS[2]) == active_counts,
                f"{step}: changed the active page")

    def bounds_clear():
        _, _, width, height = win.rect(browser.hwnd, client=True)
        bounds = {cid: browser.in_client(browser.controls[cid]) for cid in (TABS, SIDEBAR_STATUS, FREE_BUTTON)}
        page_left = browser.in_client(active)[0]
        for cid, (left, top, right, bottom) in bounds.items():
            require(win.visible(browser.controls[cid]) and 0 <= left < right <= min(width, page_left)
                    and 0 <= top < bottom <= height, f"Control {cid} is outside the expanded sidebar: {bounds}")
        require(bounds[TABS][3] <= bounds[SIDEBAR_STATUS][1]
                and bounds[SIDEBAR_STATUS][3] <= bounds[FREE_BUTTON][1],
                f"Footer controls overlap the tab list or each other: {bounds}")
        return {"client_width_px": width, "controls_px": bounds}

    with win.window_coordinates(browser.hwnd):
        left, top, right, bottom = win.rect(browser.hwnd)
        width, height = right - left, bottom - top
        report["sidebar_bounds"] = {"default": bounds_clear()}
        narrow = max(560 * dpi // 96, width * 2 // 3)
        if narrow < width:
            flags = 0x0016  # SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE.
            win.check(win.set_window_pos(browser.hwnd, None, 0, 0, narrow, height, flags))
            try:
                report["sidebar_bounds"]["narrow"] = bounds_clear()
            finally:
                win.check(win.set_window_pos(browser.hwnd, None, 0, 0, width, height, flags))

    report["hover"] = captions = {}

    def hover(label, index, pattern, counts):
        read = lambda: browser.control_text(SIDEBAR_STATUS)
        browser.wait(f"footer counts {counts!r} before {label}", lambda: read() == counts)
        with win.window_coordinates(tabs):
            # Deliver the TrackMouseEvent result without arming tracking for the real cursor.
            browser.message(TABS, WM_MOUSEHOVER, 0, row_point(browser, index, dpi))

        def matches():
            captions[label] = read()
            return re.fullmatch(pattern, captions[label])

        browser.wait(f"{label}: hover caption matching {pattern!r}", matches)
        unchanged(f"hovering {label}")
        browser.message(TABS, WM_MOUSELEAVE)
        browser.wait(f"{label}: restored footer counts {counts!r}", lambda: read() == counts)

    hover("inactive", 2, RENDERER, "3 live · 0 on demand")
    hover("active", 3, RENDERER, "3 live · 0 on demand")
    hover("blank", 1, NO_RENDERER, "3 live · 0 on demand")
    with win.window_coordinates(tabs):
        browser.message(TABS, WM_MOUSEHOVER, 0, row_point(browser, 2, dpi))
    read = lambda: browser.control_text(SIDEBAR_STATUS)
    browser.wait("renderer caption before deactivation", lambda: RENDERER.fullmatch(read()))
    send(browser, 0x0006, 0)  # WM_ACTIVATE, WA_INACTIVE.
    browser.wait("window deactivation clears the preview", lambda: read() == "3 live · 0 on demand")
    report["inactive_window_preview_hidden"] = True
    before = page_counts(site, PATHS[0])

    def click_unload():
        with win.window_coordinates(tabs):
            point = row_point(browser, 2, dpi, unload=True)
            report["unload_point_px"] = (point & 0xFFFF, point >> 16)
            post(process, tabs, WM_LBUTTONDOWN, MK_LBUTTON, point)
            post(process, tabs, WM_LBUTTONUP, 0, point)

    confirmation(browser, click_unload, "Unload tab?", IDYES, unchanged)
    browser.wait("inactive browser view detached", lambda: not child(target) and not win.pid(views[0]))
    require(loaded_panels(browser) == 2, "Row unload did not release exactly one view")
    unchanged("after row unload")
    hover("deferred", 2, NO_RENDERER, "2 live · 1 on demand")
    require(page_counts(site, PATHS[0]) == before, "Deferred page loaded without selection")

    browser.page(PATHS[0], lambda: select(browser, 2), require_request=True)
    browser.wait("selected target recreated its view", lambda: browser.page_panel() == target and child(target))
    reload_view = child(target)
    report["reload_requests"] = page_counts(site, PATHS[0])
    select(browser, 3)
    browser.wait("active page visible again", lambda: browser.page_panel() == active)
    unchanged("after selecting and reloading the deferred page")

    legacy = confirmation(browser, lambda: send(browser, WM_COMMAND, FREE_MEMORY, post=True),
                          "Free tab memory?", IDNO, unchanged)
    require(child(target) == reload_view and child(pinned) == views[1] and loaded_panels(browser) == 3,
            "Declining the existing bulk command changed a loaded view")
    browser.wait("enabled footer Free memory button", lambda: win.visible(button) and win.enabled(button))
    require(win.pid(button) == process.pid, "Footer button is no longer owned")
    via_button = confirmation(browser, lambda: send(browser, WM_COMMAND, FREE_BUTTON, button, post=True),
                              "Free tab memory?", IDYES, unchanged)  # BN_CLICKED = 0 in HIWORD(wParam).
    require(via_button == legacy, "Footer button changed the existing bulk confirmation")
    browser.wait("bulk button released the inactive unpinned view", lambda: not child(target))
    require(child(pinned) == views[1] and loaded_panels(browser) == 2, "Bulk button did not preserve the pin")
    unchanged("after bulk unload")
    report["bulk_preserved_active_and_pin"] = True

    browser.click(SIDEBAR_TOGGLE)
    browser.wait("collapsed sidebar hides its list, status and Free memory button",
                 lambda: not any(map(win.visible, (tabs, status, button))))
    unchanged("after collapsing the sidebar")
    report["collapsed_footer_hidden"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="parent directory for temporary fixture data")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    report = {"result": "fail", "launch": {}}
    try:
        exe, work = args.exe.resolve(), args.work.resolve()
        dll = exe.with_name("Flit.dll")
        require(exe.name.casefold() == "flit.exe" and exe.is_file() and dll.is_file(),
                "--exe must point to staged Flit.exe beside Flit.dll")
        with dll.open("rb") as stream:
            report["dll_sha256"] = file_digest(stream, "sha256").hexdigest()
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-tab-memory-ui-", dir=work)
        try:
            root = Path(scratch.name)
            profile, downloads = root / "profile", root / "downloads"
            downloads.mkdir()
            with fixture() as site:
                seed_profile(profile, downloads, site.origin)
                with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}"], report["launch"]) as process:
                    browser = Browser(process, site)
                    exercise(browser, report)
                    process.normal_close(browser.hwnd)
        finally:
            clean_scratch(scratch, report)
        require("scratch_cleanup_error" not in report, "Could not remove disposable fixture data")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
