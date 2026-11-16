"""Check that only the visible CEF page resizes and hidden pages catch up on selection."""

import argparse
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

sys.dont_write_bytecode = True
from windows_multi_session import clean_scratch
from windows_session import select
from windows_smoke import BACK, FORWARD, NEW_TAB, WORKSPACE, Browser, OwnedProcess, Win32, fixture, require


def page_child(browser):
    panel = browser.wait("visible CEF panel", browser.page_panel)
    child = browser.win.find_child(panel, None, None, None)
    require(child and browser.win.pid(child) == browser.process.pid, "Could not identify owned CEF HWND")
    return child


def check(exe, scratch, result):
    win = Win32()
    with fixture() as site, OwnedProcess(win, exe, [f"--flit-profile={scratch / 'profile'}"], {}) as process:
        browser = Browser(process, site)
        browser.navigate("/start")
        browser.navigate("/second")
        browser.wait("Back is available in the first tab", lambda: win.enabled(browser.controls[BACK]))
        first = page_child(browser)
        browser.click(NEW_TAB)
        browser.wait("second tab selected", lambda: browser.selected() == 1)
        browser.navigate("/tab")
        second = page_child(browser)

        def viewport(hwnd):
            return win.rect(hwnd, client=True)[2:]

        with win.window_coordinates(browser.hwnd):
            require(not win.visible(first) and win.visible(second), "First page must be hidden")
            before = result["before_px"] = [viewport(first), viewport(second)]
            left, top, right, bottom = win.rect(browser.hwnd)
            # SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE; resize only the owned window.
            win.check(win.set_window_pos(browser.hwnd, None, 0, 0, right - left + 120, bottom - top, 0x0016))
            def grown():
                size = viewport(second)
                return size if size[0] > before[1][0] else None

            new_size = browser.wait("visible browser grows", grown)
            result["after_resize_px"] = [viewport(first), viewport(second)]
            require(not win.visible(first) and viewport(first) == before[0], "Hidden CEF viewport also resized")
            select(browser, 0)
            browser.wait("first loaded page visible again", lambda: (
                browser.address() == site.origin + "/second" and win.visible(first)
            ))
            require(page_child(browser) == first, "Switching tabs recreated the CEF page")
            result["after_selection_px"] = viewport(first)
            require(viewport(first) == new_size and not win.visible(second), "Selected page retained stale bounds")
        browser.page("/start", lambda: browser.click(BACK))
        browser.page("/second", lambda: browser.click(FORWARD))
        result["back_and_forward_preserved"] = True
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
    scratch = TemporaryDirectory(prefix="viewport-", dir=work)
    try:
        try:
            check(args.exe.resolve(), Path(scratch.name), result)
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
