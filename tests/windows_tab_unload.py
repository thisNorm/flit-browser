"""Exercise Flit's manual tab unload in an owned Win32 process and profile."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import json
from pathlib import Path
import sys
import tempfile

sys.dont_write_bytecode = True
from windows_session import select, send, titles
from windows_smoke import (
    Browser, NEW_TAB, OwnedProcess, TAB_TITLE, WORKSPACE, Win32,
    api, fixture, page_counts, require,
)


WM_COMMAND = 0x0111
TAB_MENU_ACTION = 0x8005  # WM_APP + 5
TAB_PIN, TAB_UNLOAD = 1, 5
IDYES, IDNO = 6, 7
TARGET_KEY = 2  # Second tab created in this window; pinning only changes its position.
LABELS = (TAB_TITLE, "Flit smoke: start")


def answer_unload(browser, button_id, evidence):
    win, process = browser.win, browser.process
    get_window = api(C.WinDLL("user32", use_last_error=True), "GetWindow", W.HWND, W.HWND, W.UINT)

    def text(hwnd):
        value = C.create_unicode_buffer(1024)
        win.message(hwnd, 0x000D, len(value), C.addressof(value))  # WM_GETTEXT
        return value.value

    def dialog_window():
        matches = []

        @win.callback
        def visit(hwnd, _):
            if (win.pid(hwnd) == process.pid and win.visible(hwnd) and win.class_name(hwnd) == "#32770"
                    and get_window(hwnd, 4) == browser.hwnd and text(hwnd) == "Unload tab?"):
                matches.append(hwnd)
            return True

        win.check(win.enum(visit, 0))
        return matches[0] if matches else None

    send(browser, TAB_MENU_ACTION, TARGET_KEY, TAB_UNLOAD, post=True)
    dialog = browser.wait("owned Unload tab? confirmation", dialog_window, 5)
    button = win.get_item(dialog, button_id)
    require(win.pid(dialog) == win.pid(button) == process.pid, "Unload dialog or button is no longer owned")
    require(win.get_item(dialog, IDYES) and win.get_item(dialog, IDNO), "Unload dialog is missing Yes or No")
    message = " ".join(filter(None, (text(child) for child in win.visible_children(dialog, "Static"))))
    default = win.message(dialog, 0x0400)  # DM_GETDEFID
    evidence.append({"message": message, "default_no": default == 0x534B0007, "button_id": button_id})
    win.check(win.post(dialog, WM_COMMAND, button_id, button))
    browser.wait("owned confirmation dismissed", lambda: not win.visible(dialog))


def run(exe, work, evidence):
    require(exe.is_file(), f"Missing executable: {exe}")
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-unload-", dir=work)) / "profile"
    profile.mkdir()
    evidence.update(profile=str(profile), launch={}, confirmations=[])
    with fixture() as site:
        args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"]
        try:
            with OwnedProcess(Win32(), exe, args, evidence["launch"]) as process:
                browser = Browser(process, site)
                browser.page("/start", require_request=True)
                start_panel = browser.wait("initial page panel", browser.page_panel)
                browser.click(NEW_TAB)
                browser.wait("second tab selected", lambda: browser.selected() == 1)
                browser.navigate("/tab")
                target_panel = browser.wait("target page panel", browser.page_panel)
                win = browser.win
                target_view = win.find_child(target_panel, None, None, None)
                require(target_panel != start_panel and target_view, "Target CEF view was not attached")

                # The same command on the active page must return without confirmation or teardown.
                send(browser, TAB_MENU_ACTION, TARGET_KEY, TAB_UNLOAD)
                require(browser.selected() == 1 and win.find_child(target_panel, None, None, None) == target_view,
                        "Unload changed the active tab")
                send(browser, TAB_MENU_ACTION, TARGET_KEY, TAB_PIN)
                select(browser, 1)

                def background_unchanged():
                    require(titles(browser) == LABELS and browser.selected() == 1,
                            "Unload changed tab count, title, order or selection")
                    require(browser.address() == site.origin + "/start" and browser.page_panel() == start_panel,
                            "Unload changed the foreground page")

                browser.wait("pinned target and foreground title", lambda: titles(browser) == LABELS)
                background_unchanged()
                before = page_counts(site, "/tab")
                evidence["before_unload"] = before

                answer_unload(browser, IDNO, evidence["confirmations"])
                background_unchanged()
                require(win.find_child(target_panel, None, None, None) == target_view
                        and page_counts(site, "/tab") == before, "No unloaded or reloaded the background page")
                evidence["cancel_kept_view"] = True

                answer_unload(browser, IDYES, evidence["confirmations"])
                browser.wait("background CEF view detached", lambda: not win.find_child(target_panel, None, None, None))
                background_unchanged()
                require(win.pid(target_panel) == process.pid and not win.pid(target_view),
                        "Unload did not retain the outer panel and destroy its original CEF view")
                # A deferred page is no longer eligible. This also leaves a turn before selection.
                send(browser, TAB_MENU_ACTION, TARGET_KEY, TAB_UNLOAD)
                require(page_counts(site, "/tab") == before, "Unloaded page reloaded before selection")
                evidence["while_unloaded"] = page_counts(site, "/tab")

                browser.page("/tab", lambda: send(browser, WM_COMMAND, 151, post=True), require_request=True)
                browser.wait("selected target title and recreated view", lambda: (
                    browser.selected() == 0 and browser.has_page_title(TAB_TITLE)
                    and browser.page_panel() == target_panel
                ))
                require(titles(browser) == LABELS, "Reload changed the tab list")
                process.normal_close(browser.hwnd)
        finally:
            evidence["fixture"] = site.report()

        after = page_counts(site, "/tab")
        require(after == (before[0] + 1, before[1] + 1), f"Expected one reload and pageshow, got {before} → {after}")
        evidence["after_selection"] = after
        session = json.loads((profile / "session.json").read_text(encoding="utf-8"))
        require(session["selected"] == 0 and session["entries"] == [
            {"url": site.origin + "/tab", "title": TAB_TITLE, "pinned": True},
            {"url": site.origin + "/start", "title": LABELS[1], "pinned": False},
        ], f"Unload and reload changed saved tab metadata: {session}")
        evidence["session"] = session

    for confirmation in evidence["confirmations"]:
        require(confirmation["default_no"], "The unload confirmation did not default to No")
        message = confirmation["message"].casefold()
        require("reload" in message and "unsaved" in message and ("lost" in message or "lose" in message),
                f"Confirmation must warn about reload and loss of unsaved changes: {confirmation['message']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--work", required=True, type=Path)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Browser runtime verification requires Windows")
    evidence = {"result": "fail"}
    try:
        run(args.exe.resolve(), args.work.resolve(), evidence)
        evidence["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(evidence, indent=2))
    return int(evidence["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
