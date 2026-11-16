"""Exercise Chromium DevTools in a separate window without a debug listener."""

import argparse
import ctypes as C
from hashlib import sha256
import json
from pathlib import Path
import tempfile

from windows_smoke import Browser, OwnedProcess, Win32, fixture, require


DEVTOOLS, DEVTOOLS_CONSOLE, NEW_TAB, CLOSE_TAB = 163, 164, 105, 106
WM_COMMAND, WM_CLOSE = 0x0111, 0x0010


def title(win, hwnd):
    text = C.create_unicode_buffer(1024)
    win.message(hwnd, 0x000D, len(text), C.addressof(text))
    return text.value


def tools_windows(browser):
    win, result = browser.win, {}

    @win.callback
    def visit(hwnd, _):
        if (hwnd != browser.hwnd and win.pid(hwnd) == browser.process.pid
                and win.visible(hwnd) and win.class_name(hwnd).startswith("Chrome_WidgetWin_")):
            result[hwnd] = title(win, hwnd)
        return True

    win.check(win.enum(visit, 0))
    return result


def command(browser, action):
    require(browser.win.pid(browser.hwnd) == browser.process.pid, "Flit main window is no longer owned")
    browser.win.check(browser.win.post(browser.hwnd, WM_COMMAND, action, 0))


def open_tools(browser, action, path):
    command(browser, action)
    target = browser.site.origin.removeprefix("http://") + path

    def match():
        for hwnd, label in tools_windows(browser).items():
            if "devtools" in label.casefold() and target in label:
                return hwnd, label

    return browser.wait(f"DevTools window inspecting {path}", match, timeout=4)


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-devtools-", dir=work)) / "profile"
    launch = report["launch"] = {}
    with fixture() as site:
        win = Win32()
        with OwnedProcess(win, exe, [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"], launch) as process:
            browser = Browser(process, site)
            browser.page("/start", require_request=True)
            require(not tools_windows(browser), "An auxiliary Chromium window opened before requesting DevTools")
            report["first_tools_title"] = open_tools(browser, DEVTOOLS, "/start")[1]
            require(browser.tab_count() == 1 and browser.has_page_title("Flit smoke: start"),
                    "Opening tools changed the inspected tab")

            command(browser, DEVTOOLS)
            browser.wait("DevTools toggled closed", lambda: not tools_windows(browser), timeout=5)
            require(browser.has_page_title("Flit smoke: start"), "Closing DevTools changed the inspected tab")

            command(browser, NEW_TAB)
            browser.wait("two tabs", lambda: browser.tab_count() == 2)
            browser.navigate("/second")
            report["console_tools_title"] = open_tools(browser, DEVTOOLS_CONSOLE, "/second")[1]
            command(browser, CLOSE_TAB)
            browser.wait("inspected tab closed without leaving DevTools", lambda: (
                browser.tab_count() == 1 and not tools_windows(browser)
                and browser.has_page_title("Flit smoke: start")
            ), timeout=5)

            open_tools(browser, DEVTOOLS, "/start")
            process.normal_close(browser.hwnd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    exe = args.exe.resolve(strict=True)
    report = {"result": "fail", "dll_sha256": sha256(exe.with_name("Flit.dll").read_bytes()).hexdigest()}
    try:
        run(exe, args.work.resolve(), report)
        report["result"] = "pass"
    except (RuntimeError, OSError) as error:
        report["error"] = str(error)
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
