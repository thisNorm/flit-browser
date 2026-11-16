"""Exercise sidebar tab search in a real Flit Win32 window with a disposable session.

The Ctrl+Shift+A command and search keys target only the test process's HWNDs.
The seeded Chromium profile uses the loopback fixture as a non-forwarding proxy.
"""

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
from windows_session import loaded_panels, send, titles
from windows_shortcuts import GuiThreadInfo
from windows_smoke import (
    Browser, Handler, OwnedProcess, TAB_FILTER, TAB_TITLE, TABS, WORKSPACE, Win32,
    api, fixture, require,
)
from windows_windows import WM_COMMAND, seed_profile


TAB_LABEL, FOCUS_FILTER = 111, 114
VK_RETURN, VK_ESCAPE, VK_UP, VK_DOWN = 0x0D, 0x1B, 0x26, 0x28
FIRST, SOURCE, TARGET = "/start", "/tab", "/second"
LABELS = ("fixture-special first", TAB_TITLE, "fixture-special second")
TARGET_TITLE = "Flit smoke: second"
QUERY = "fixture-special 127.0.0.1"


class LoopbackHandler(Handler):
    def do_GET(self):
        # The proxy sends absolute request targets; only the fixture origin is served.
        if not self.path.startswith(self.server.origin + "/"):
            self.send_error(403)
            return
        self.path = self.path[len(self.server.origin):]
        super().do_GET()


def document_requests(site):
    requests = site.report()["requests"]
    requests.pop("/favicon.ico", None)  # Chromium may fetch the icon asynchronously.
    return requests


def exercise(browser, checks):
    site, win = browser.site, browser.win
    filter_hwnd, tabs_hwnd = browser.controls[TAB_FILTER], browser.controls[TABS]
    label_hwnd = win.get_item(browser.hwnd, TAB_LABEL)
    require(label_hwnd and win.pid(label_hwnd) == browser.process.pid, "Missing owned tab count label")
    browser.controls[TAB_LABEL] = label_hwnd
    thread = win.get_pid(browser.hwnd, None)
    get_gui_info = api(C.WinDLL("user32", use_last_error=True), "GetGUIThreadInfo",
                       W.BOOL, W.DWORD, C.POINTER(GuiThreadInfo))
    require(thread and win.pid(browser.hwnd) == browser.process.pid, "Missing owned GUI thread")

    def focus_search(label):
        send(browser, WM_COMMAND, FOCUS_FILTER)
        info = GuiThreadInfo(C.sizeof(GuiThreadInfo))
        win.check(get_gui_info(thread, C.byref(info)))
        checks[label] = {"sidebar_visible": bool(win.visible(filter_hwnd) and win.visible(tabs_hwnd)),
                         "window_active": info.active == browser.hwnd, "filter_focused": info.focus == filter_hwnd}
        require(checks[label]["sidebar_visible"], f"{label}: search command did not show the sidebar")
        if checks[label]["window_active"]:
            require(checks[label]["filter_focused"], f"{label}: active window did not focus the filter")

    def page_state():
        return {"address": browser.address(), "title": browser.title(), "panel": browser.page_panel(),
                "loaded_panels": loaded_panels(browser), "requests": document_requests(site)}

    def stable(label, page, search=None):
        deadline = time.monotonic() + 0.35

        def sample():
            native_page = page_state()
            text, rows, highlight = browser.control_text(TAB_FILTER), titles(browser), browser.selected()
            heading = browser.control_text(TAB_LABEL)
            checks[label] = {"page": native_page, "query": text, "rows": rows,
                             "highlight": highlight, "heading": heading}
            require(native_page == page, f"{label}: expected page state {page}, got {native_page}")
            if search is not None:
                require((text, rows, highlight) == search, f"{label}: expected search state {search}, got {checks[label]}")
            if len(rows) < len(LABELS):
                require(heading == f"{len(rows)}/{len(LABELS)}", f"{label}: unexpected tab count label {heading!r}")
            return time.monotonic() >= deadline

        browser.wait(label, sample, 3)

    browser.page(SOURCE, require_request=True)
    browser.wait("restored active title and page panel", lambda: browser.has_page_title(TAB_TITLE) and browser.page_panel())
    require(not win.visible(filter_hwnd) and not win.visible(tabs_hwnd), "Sidebar did not restore closed")
    initial = page_state()
    require(initial["loaded_panels"] == 1 and set(initial["requests"]) == {SOURCE, "/shown" + SOURCE},
            f"Only the active session tab should load at startup: {initial}")
    stable("restored_session", initial, ("", LABELS, 1))

    focus_search("shortcut_command_reveals_sidebar")
    browser.set_text(TAB_FILTER, QUERY)
    matches = (LABELS[0], LABELS[2])
    stable("title_and_address_words_highlight_first", initial, (QUERY, matches, 0))
    for label, key, index in (("down", VK_DOWN, 1), ("up", VK_UP, 0), ("down_to_target", VK_DOWN, 1)):
        browser.press(TAB_FILTER, key)
        stable(label, initial, (QUERY, matches, index))

    # Filtered row 1 is session row 2; session row 1 is the untouched source.
    browser.page(TARGET, lambda: browser.press(TAB_FILTER, VK_RETURN), require_request=True)
    browser.wait("highlighted tab title", lambda: browser.has_page_title(TARGET_TITLE))
    target = page_state()
    expected_requests = initial["requests"] | {TARGET: 1, "/shown" + TARGET: 1}
    require(target["panel"] and target["panel"] != initial["panel"] and target["loaded_panels"] == 2
            and target["requests"] == expected_requests, f"Enter must load only the highlighted tab once: {target}")
    all_rows = (*LABELS[:2], TARGET_TITLE)
    stable("enter_clears_and_activates_target", target, ("", all_rows, 2))

    focus_search("reopen_for_escape")
    query = "smoke: 127.0.0.1"  # The active tab matches, but appears second.
    browser.set_text(TAB_FILTER, query)
    stable("first_highlight_even_when_active_matches", target, (query, (TAB_TITLE, TARGET_TITLE), 0))
    browser.press(TAB_FILTER, VK_ESCAPE)
    stable("escape_clears_without_activation", target, ("", all_rows, 2))

    focus_search("reopen_for_no_results")
    query = "no-matching-fixture-tab"
    browser.set_text(TAB_FILTER, query)
    stable("zero_results", target, (query, (), -1))
    browser.press(TAB_FILTER, VK_RETURN)
    stable("zero_results_enter_does_not_navigate", target)
    return expected_requests


def run(exe, profile, report):
    with fixture() as site:
        site.RequestHandlerClass = LoopbackHandler
        seed_profile(profile, site.origin)
        session = {"restore": True, "sidebar_open": False, "selected": 1, "entries": [
            {"url": site.origin + path, "title": label, "pinned": False}
            for path, label in zip((FIRST, SOURCE, TARGET), LABELS)
        ]}
        (profile / "session.json").write_text(json.dumps(session), encoding="utf-8")
        try:
            with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}"], report["launch"]) as process:
                browser = Browser(process, site)
                requests = exercise(browser, report["checks"])
                process.normal_close(browser.hwnd)
            require(document_requests(site) == requests, "A page was requested during test shutdown")
        finally:
            report["fixture"] = site.report()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="workspace scratch directory for the disposable profile")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Browser runtime verification requires Windows")

    report = {"result": "fail", "checks": {}, "launch": {}}
    try:
        exe, work = args.exe.resolve(), args.work.resolve()
        require(exe.is_file(), f"Missing executable: {exe}")
        require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-tab-search-", dir=work)
        try:
            profile = Path(scratch.name) / "profile"
            report.update(exe=str(exe), profile=str(profile))
            run(exe, profile, report)
        finally:
            clean_scratch(scratch, report)
        require("scratch_cleanup_error" not in report, "Could not remove the disposable profile")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
