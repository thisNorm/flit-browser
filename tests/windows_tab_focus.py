"""Check mouse and keyboard focus handoff with two loaded loopback tabs."""

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
from windows_session import titles
from windows_shortcuts import GuiThreadInfo
from windows_smoke import Browser, NEW_TAB, OwnedProcess, TAB_FILTER, TAB_TITLE, TABS, WORKSPACE, Win32, api, fixture, require
from windows_tab_order import MK_LBUTTON, WM_LBUTTONDOWN, WM_LBUTTONUP, dll_sha256, row_point


def exercise(b, checks):
    win, tabs = b.win, b.controls[TABS]
    pid = W.DWORD()
    thread = win.get_pid(b.hwnd, C.byref(pid))
    require(thread and pid.value == b.process.pid, "Could not get the owned browser GUI thread")
    user = C.WinDLL("user32", use_last_error=True)
    gui_info = api(user, "GetGUIThreadInfo", W.BOOL, W.DWORD, C.POINTER(GuiThreadInfo))
    is_child = api(user, "IsChild", W.BOOL, W.HWND, W.HWND)
    checks["window"] = {"hwnd": b.hwnd, "listbox": tabs, "gui_thread": thread}

    paths = ("/start", "/tab")
    b.page(paths[0], require_request=True)
    first = b.wait("first visible CEF panel", b.page_panel)
    b.click(NEW_TAB)
    b.wait("second row selected", lambda: b.selected() == 1)
    b.navigate(paths[1])
    second = b.wait("second visible CEF panel", lambda: (p := b.page_panel()) and p != first and p)
    b.wait("both titled tabs", lambda: titles(b) == ("Flit smoke: start", TAB_TITLE))
    panels = (first, second)
    checks["loaded_panels"] = panels

    def expect(label, index, *, page, keep_list_focus=False):
        trace = checks[label] = {"expected_row": index, "expected_focus": "page" if page else "listbox"}
        stable_since = None

        def sample():
            nonlocal stable_since
            selected, address, panel = b.selected(), b.address(), b.page_panel()
            info = GuiThreadInfo(C.sizeof(GuiThreadInfo))
            C.set_last_error(0)
            ok = gui_info(thread, C.byref(info))
            error = None if ok else C.get_last_error()
            focus = info.focus
            in_page = bool(panel and focus and is_child(panel, focus) and win.pid(focus) == b.process.pid)
            trace.update(row=selected, address=address, panel=panel, hwnd_focus=focus,
                         page_focus=in_page, listbox_focus=focus == tabs, gui_error=error,
                         browser_active=info.active == b.hwnd, listbox_capture=info.capture == tabs)
            if keep_list_focus and ok and focus:
                require(focus == tabs, f"{label} moved focus off the ListBox: {trace}")
            matched = (ok and (in_page if page else focus == tabs) and selected == index
                       and panel == panels[index] and address == b.site.origin + paths[index])
            if not keep_list_focus:
                return matched
            # Leave time for an asynchronous focus change after the arrow selection.
            if not matched:
                stable_since = None
                return False
            now = time.monotonic()
            if stable_since is None:
                stable_since = now
            return now - stable_since >= 0.2

        b.wait(label, sample, 5)

    def click_row(label, index, before):
        require(win.pid(tabs) == b.process.pid and win.visible(tabs), "Tab ListBox is not visible and owned")
        with win.window_coordinates(tabs):
            x, y = row_point(b, index, 2, max(win.check(win.dpi(tabs)), 96))
            point = x | (y << 16)
            checks[label] = {"client_point": [x, y]}
            try:
                b.message(TABS, WM_LBUTTONDOWN, MK_LBUTTON, point)
                expect(f"{label}: mouse down", before, page=False)
            finally:
                b.message(TABS, WM_LBUTTONUP, 0, point)
        expect(f"{label}: mouse up", index, page=True)

    click_row("click active tab", 1, 1)  # 0.16 leaves focus on the ListBox here.
    click_row("click inactive tab", 0, 1)

    # Escape in the filter gives the ListBox focus without an ordinary row click.
    b.press(TAB_FILTER, 0x1B)  # VK_ESCAPE
    expect("keyboard setup", 0, page=False)
    for label, key, index in (("Down", 0x28, 1), ("Up", 0x26, 0)):
        b.press(TABS, key)
        expect(label, index, page=False, keep_list_focus=True)
    b.press(TABS, 0x0D)  # VK_RETURN; the helper also sends translated WM_CHAR.
    expect("Enter", 0, page=True)


def run(exe, profile, evidence):
    with fixture() as site:
        try:
            args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"]
            with OwnedProcess(Win32(), exe, args, evidence["launch"]) as process:
                b = Browser(process, site)
                exercise(b, evidence["checks"])
                process.normal_close(b.hwnd)
        finally:
            evidence["fixture"] = site.report()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, default=WORKSPACE / "work" / "windows-tab-focus",
                        help="workspace scratch directory for the disposable profile")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Browser runtime verification requires Windows")
    evidence = {"result": "fail", "checks": {}, "launch": {}}
    try:
        exe, work = args.exe.resolve(), args.work.resolve()
        evidence["exe"] = str(exe)
        require(exe.name.casefold() == "flit.exe" and exe.is_file(), f"Missing staged Flit.exe: {exe}")
        require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
        evidence["dll_sha256"] = dll_sha256(exe)
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-tab-focus-", dir=work)
        try:
            profile = Path(scratch.name) / "profile"
            evidence["profile"] = str(profile)
            run(exe, profile, evidence)
        finally:
            clean_scratch(scratch, evidence)
        require("scratch_cleanup_error" not in evidence, "Could not remove the disposable profile")
        evidence["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(evidence, indent=2))
    return int(evidence["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
