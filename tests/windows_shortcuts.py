"""Run real Flit shortcuts from the address bar and page on an idle Windows desktop.

Only the shortcuts use SendInput; fixture text and selections are seeded with
targeted messages. The candidate gets a new temporary profile and a loopback page.
Run from an unlocked foreground terminal and leave the desktop idle. Windows
has no atomic foreground-check-and-send API.
Exit 2 means keyboard checks were blocked by desktop focus or a held key.
"""

import argparse
import ctypes as C
from ctypes import wintypes as W
from pathlib import Path
import sys
import tempfile
import time

from windows_smoke import ADDRESS, Browser, OwnedProcess, WORKSPACE, Win32, api, fixture, require


FOCUS_ADDRESS, FIND_TEXT, FIND_STATUS = 113, 125, 126
EM_GETSEL, EM_SETSEL = 0x00B0, 0x00B1
VK_BACK, VK_TAB, VK_CONTROL, VK_DELETE, VK_F5, VK_F11 = 0x08, 0x09, 0x11, 0x2E, 0x74, 0x7A
MODIFIERS = (VK_CONTROL, 0x10, 0x12, 0x5B, 0x5C)  # Ctrl, Shift, Alt, left/right Win
KEYUP, EXTENDED = 0x0002, 0x0001


class GuiThreadInfo(C.Structure):
    _fields_ = [("size", W.DWORD), ("flags", W.DWORD), ("active", W.HWND), ("focus", W.HWND),
                ("capture", W.HWND), ("menu", W.HWND), ("move", W.HWND), ("caret", W.HWND),
                ("caret_rect", W.RECT)]


class KeyboardInput(C.Structure):
    _fields_ = [("vk", W.WORD), ("scan", W.WORD), ("flags", W.DWORD), ("time", W.DWORD),
                ("extra", C.c_size_t)]


class MouseInput(C.Structure):
    # SendInput requires the size and alignment of the whole INPUT union.
    _fields_ = [("dx", W.LONG), ("dy", W.LONG), ("data", W.DWORD), ("flags", W.DWORD),
                ("time", W.DWORD), ("extra", C.c_size_t)]


class InputData(C.Union):
    _fields_ = [("keyboard", KeyboardInput), ("mouse", MouseInput)]


class Input(C.Structure):
    _fields_ = [("type", W.DWORD), ("data", InputData)]


class KeyboardUnavailable(RuntimeError):
    pass


class OwnedKeyboard:
    def __init__(self, win):
        self.win = win
        user = C.WinDLL("user32", use_last_error=True)
        self.foreground = api(user, "GetForegroundWindow", W.HWND)
        self.set_foreground = api(user, "SetForegroundWindow", W.BOOL, W.HWND)
        self.gui_info = api(user, "GetGUIThreadInfo", W.BOOL, W.DWORD, C.POINTER(GuiThreadInfo))
        self.is_child = api(user, "IsChild", W.BOOL, W.HWND, W.HWND)
        self.key_state = api(user, "GetAsyncKeyState", W.SHORT, C.c_int)
        self.send_input = api(user, "SendInput", W.UINT, W.UINT, C.POINTER(Input), C.c_int)
        self.previous = self.foreground()
        self.previous_pid = win.pid(self.previous) if self.previous else 0

    def activate(self, browser):
        self.browser = browser
        self.address = browser.controls[ADDRESS]
        if self.owns_foreground():
            return
        foreground = self.foreground()
        if not foreground:
            raise KeyboardUnavailable("No foreground window on this desktop; no input sent")
        if foreground != self.previous:
            raise KeyboardUnavailable("Foreground changed during startup; no input sent")
        require(self.win.pid(browser.hwnd) == browser.process.pid, "Flit window is no longer owned")
        if not self.set_foreground(browser.hwnd) and not self.owns_foreground():
            raise KeyboardUnavailable("Windows did not give Flit foreground; no input sent")
        browser.wait("Flit to become the owned foreground window", self.owns_foreground, 2)

    def owns_foreground(self):
        hwnd = self.browser.hwnd
        return self.win.pid(hwnd) == self.browser.process.pid and self.foreground() == hwnd

    def focused(self, hwnd, *, descendant=False):
        info = GuiThreadInfo(C.sizeof(GuiThreadInfo))
        return (self.gui_info(0, C.byref(info)) and not (info.flags & 0x1E)  # Menu or window move/resize.
                and info.active == self.browser.hwnd and info.focus
                and (self.is_child(hwnd, info.focus) if descendant else info.focus == hwnd)
                and self.win.pid(info.focus) == self.browser.process.pid
                and self.owns_foreground())

    def focus_address(self):
        if not self.owns_foreground():
            raise KeyboardUnavailable("Flit lost foreground; no further input sent")
        self.win.message(self.browser.hwnd, 0x0111, FOCUS_ADDRESS)  # WM_COMMAND, setup only.

    def press(self, label, key, *, ctrl=True, panel=None):
        keys = (VK_CONTROL, key) if ctrl else (key,)

        def event(vk, flags):
            flags |= EXTENDED if vk == VK_DELETE else 0
            return Input(1, InputData(keyboard=KeyboardInput(vk, 0, flags)))

        def focused():
            return self.focused(panel or self.address, descendant=panel is not None)

        events = [event(vk, 0) for vk in keys] + [event(vk, KEYUP) for vk in reversed(keys)]
        batch = (Input * len(events))(*events)
        if any(self.key_state(vk) < 0 for vk in (*MODIFIERS, key)):
            raise KeyboardUnavailable(f"{label}: a key or modifier is already held; no input sent")
        if not self.owns_foreground():
            raise KeyboardUnavailable(f"{label}: Flit lost foreground; no input sent")
        C.set_last_error(0)
        if not focused():
            raise RuntimeError(f"{label}: expected {'page' if panel else 'address'} focus; no input sent")
        sent = self.send_input(len(batch), batch, C.sizeof(Input))
        if sent != len(batch):
            error = C.get_last_error()
            if sent:
                # Release only keys this batch put down and only while the same owned control has focus.
                pending = events[max(sent, len(events) - sent):]
                releases = (Input * len(pending))(*pending)
                if focused():
                    self.send_input(len(releases), releases, C.sizeof(Input))
            raise RuntimeError(f"{label}: SendInput accepted {sent}/{len(batch)} events (Win32 error {error})")

    def restore(self, browser):
        hwnd = self.previous
        if (hwnd and hwnd != browser.hwnd and self.previous_pid
                and self.win.pid(hwnd) == self.previous_pid and self.win.visible(hwnd)
                and self.win.enabled(hwnd) and self.win.pid(browser.hwnd) == browser.process.pid
                and self.foreground() == browser.hwnd):
            if not self.set_foreground(hwnd):
                print("Windows did not restore the previous foreground window.", file=sys.stderr)


def selection(browser):
    packed = browser.message(ADDRESS, EM_GETSEL)
    return packed & 0xFFFF, packed >> 16


def check_address(browser, keyboard):
    keyboard.focus_address()
    cases = (
        ("Ctrl+Backspace, Korean word", VK_BACK, "앞  한글  뒤", 7, 7, "앞  뒤"),
        ("Ctrl+Backspace, selection after emoji", VK_BACK, "가😀한글 나", 3, 5, "가😀 나"),
        ("Ctrl+Delete, Korean word", VK_DELETE, "앞  한글  뒤", 1, 1, "앞  뒤"),
        ("Ctrl+Delete, emoji", VK_DELETE, "가😀나", 1, 1, "가나"),
        ("Ctrl+A, whole UTF-16 selection", ord("A"), "앞😀 한글", 4, 4, "앞😀 한글"),
    )
    for label, key, original, start, end, expected in cases:
        browser.edit_address(original)
        browser.message(ADDRESS, EM_SETSEL, start, end)
        require(selection(browser) == (start, end), f"{label}: could not seed the requested UTF-16 selection")
        keyboard.press(label, key)
        full = (0, len(original.encode("utf-16-le")) // 2) if key == ord("A") else None

        def matches():
            return browser.address() == expected and (full is None or selection(browser) == full)

        browser.wait(f"{label}: expected address {expected!r}" + (f" and selection {full}" if full else ""),
                     matches, 4)
        # TranslateMessage can queue WM_CHAR behind WM_KEYDOWN. Catch a second edit.
        time.sleep(0.1)
        require(matches(), f"{label}: final address {browser.address()!r}, selection {selection(browser)}")
        print(f"PASS: {label}", flush=True)


def check_tabs(browser, keyboard):
    def tabs(label, count, index):
        browser.wait(f"{label}: {count} tabs with index {index} selected",
                     lambda: browser.tab_count() == count and browser.selected() == index, 5)

    for count in range(2, 11):
        keyboard.press("Ctrl+T", ord("T"))
        tabs("Ctrl+T", count, count - 1)
    print("PASS: Ctrl+T opened nine tabs", flush=True)

    for number in range(1, 10):
        label = f"Ctrl+{number}"
        keyboard.focus_address()
        keyboard.press(label, ord(str(number)))
        tabs(label, 10, number - 1 if number < 9 else 9)
    print("PASS: Ctrl+1 through Ctrl+8 selected by position; Ctrl+9 selected tab 10", flush=True)

    keyboard.focus_address()
    keyboard.press("Ctrl+W", ord("W"))
    tabs("Ctrl+W", 9, 8)
    keyboard.focus_address()
    keyboard.press("Ctrl+Tab", VK_TAB)
    tabs("Ctrl+Tab", 9, 0)
    require(browser.address() == browser.site.origin + "/start", "Ctrl+Tab did not return to the original page")
    print("PASS: Ctrl+W closed a tab; Ctrl+Tab wrapped to the original tab", flush=True)


def check_page(browser, keyboard):
    for label, key, ctrl in (("Ctrl+R", ord("R"), True), ("F5", VK_F5, False)):
        keyboard.focus_address()
        browser.page("/start", lambda: keyboard.press(label, key, ctrl=ctrl), require_request=True)
        print(f"PASS: {label} requested and rendered the loopback page again", flush=True)

    for cid, name in ((FIND_TEXT, "edit"), (FIND_STATUS, "static")):
        hwnd = browser.win.get_item(browser.hwnd, cid)
        require(hwnd and browser.win.pid(hwnd) == browser.process.pid
                and browser.win.class_name(hwnd).casefold() == name, f"Missing owned find control {cid}")
        browser.controls[cid] = hwnd
    find = browser.controls[FIND_TEXT]
    require(not browser.win.visible(find), "Find input should start hidden")
    keyboard.focus_address()
    keyboard.press("Ctrl+F", ord("F"))
    browser.wait("Ctrl+F shows and focuses the find input",
                 lambda: browser.win.visible(find) and keyboard.focused(find), 5)
    browser.set_text(FIND_TEXT, "Flit smoke: start")
    browser.wait("Ctrl+F find input reports one page match",
                 lambda: browser.control_text(FIND_STATUS) in ("0/1", "1/1"), 5)
    print("PASS: Ctrl+F opened the focused find input and the fixture had one match", flush=True)


def check_cef_focus(browser, keyboard):
    browser.navigate("/first")  # The product's Go action hands focus to CEF.
    panel = browser.wait("visible CEF panel", browser.page_panel)

    def page_focused():
        return keyboard.focused(panel, descendant=True)

    browser.wait("keyboard focus inside CEF", page_focused, 5)
    frame_mask = 0x00CF0000  # WS_OVERLAPPEDWINDOW
    for fullscreen in (True, False):
        keyboard.press("F11 from CEF", VK_F11, ctrl=False, panel=panel)
        browser.wait(f"F11 {'enters' if fullscreen else 'leaves'} fullscreen and restores page focus", lambda: (
            bool(browser.win.get_style(browser.hwnd, -16) & frame_mask) != fullscreen
            and bool(browser.win.visible(keyboard.address)) != fullscreen
            and page_focused()
        ), 5)
    print("PASS: F11 entered and left fullscreen from CEF focus", flush=True)

    keyboard.press("Ctrl+L from CEF", ord("L"), panel=panel)
    end = len(browser.site.origin + "/first")  # Loopback URL is ASCII.
    browser.wait("Ctrl+L moves focus to the native address and selects its URL",
                 lambda: keyboard.focused(keyboard.address) and selection(browser) == (0, end), 5)
    print("PASS: Ctrl+L moved focus from CEF to the native address", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe inside the workspace")
    exe = parser.parse_args().exe.resolve()
    if sys.platform != "win32":
        parser.error("requires Windows")
    if not exe.is_relative_to(WORKSPACE) or exe.name.casefold() != "flit.exe" or not exe.is_file():
        parser.error(f"expected a staged Flit.exe inside {WORKSPACE}: {exe}")

    win = Win32()
    status = 0
    with tempfile.TemporaryDirectory(prefix="flit-shortcuts-") as profile, fixture() as site:
        keyboard = OwnedKeyboard(win)
        args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"]
        launch = {}
        with OwnedProcess(win, exe, args, launch) as process:
            browser = Browser(process, site)
            try:
                browser.page("/start", require_request=True)
                print(f"PASS: owned Flit PID {process.pid} rendered the loopback fixture", flush=True)
                keyboard.activate(browser)
                check_address(browser, keyboard)
                check_tabs(browser, keyboard)
                check_page(browser, keyboard)
                check_cef_focus(browser, keyboard)
            except KeyboardUnavailable as error:
                print(f"BLOCKED: {error}", file=sys.stderr, flush=True)
                status = 2
            finally:
                try:
                    keyboard.restore(browser)
                finally:
                    process.normal_close(browser.hwnd)
                    print(f"PASS: WM_CLOSE exited PID {process.pid} with code {launch['exit_code']}; owned job is empty",
                          flush=True)
    if status == 0:
        print("PASS: Flit Windows keyboard E2E")
    return status


if __name__ == "__main__":
    raise SystemExit(main())
