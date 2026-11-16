"""Verify sidebar counts and bulk unload while retaining the active tab, a pin, and an unsaved draft."""

import argparse
from hashlib import file_digest
import json
from pathlib import Path
import tempfile

from windows_close_cancel import DRAFT, PATHS, answer, click_and_observe, close_dialog, fixture
from windows_download_close import owned_windows, post
from windows_downloads import seed_profile
from windows_session import loaded_panels, select, titles
from windows_smoke import (
    Browser, GO, NEW_TAB, OwnedProcess, SIDEBAR_TOGGLE, TABS, WORKSPACE, Win32, require, until,
)


FREE_MEMORY, SIDEBAR_STATUS, WM_COMMAND = 167, 188, 0x0111
IDYES, IDNO = 6, 7


def confirm(browser, button):
    process, win = browser.process, browser.win
    post(process, browser.hwnd, WM_COMMAND, FREE_MEMORY)
    dialogs = browser.wait("owned Free tab memory confirmation", lambda: owned_windows(process, "#32770"), 5)
    require(len(dialogs) == 1, "Expected one free-memory confirmation")
    dialog = dialogs[0]
    require(win.message(dialog, 0x0400) == 0x534B0007, "Confirmation did not default to No")
    target = win.get_item(dialog, button)
    require(target and win.pid(target) == process.pid, "Confirmation button was not owned")
    post(process, dialog, WM_COMMAND, button, target)
    browser.wait("confirmation dismissed", lambda: not win.visible(dialog))


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-free-memory-", dir=work)) / "profile"
    report["launch"] = launch = {}
    with fixture() as site:
        seed_profile(profile, profile.parent / "downloads", site.origin)
        exercise(exe, profile, site, launch, report)


def exercise(exe, profile, site, launch, report):
    with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}"], launch) as process:
        browser = Browser(process, site)
        win = browser.win
        footer = win.get_item(browser.hwnd, SIDEBAR_STATUS)
        require(footer and win.pid(footer) == process.pid and win.class_name(footer).casefold() == "static",
                "Expected owned native Static sidebar footer (188)")
        browser.controls[SIDEBAR_STATUS] = footer
        report["sidebar_footer"] = footer_checks = {}

        def check_footer(step, expected):
            def matches():
                footer_checks[step] = browser.control_text(SIDEBAR_STATUS)
                return win.visible(footer) and footer_checks[step] == expected

            browser.wait(f"{step}: visible sidebar footer {expected!r}", matches)

        def visit(path, action=None):
            before = browser.site.count("/shown" + path)
            if action is None:
                browser.edit_address(site.origin + path)
                browser.click(GO)
            else:
                action()
            browser.wait(f"fixture {path} shown", lambda: browser.address() == site.origin + path
                         and site.count("/shown" + path) > before)

        browser.wait("initial blank tab", lambda: browser.tab_count() == 1 and browser.selected() == 0)
        check_footer("empty", "No pages loaded")
        require(loaded_panels(browser) == 0, "Initial blank tab created a view")
        visit("/plain")
        panels = [browser.wait("releasable page panel", browser.page_panel)]
        for path in ("/protected", "/plain", "/active"):
            browser.click(NEW_TAB)
            if path == "/active":
                check_footer("new_blank", "3 live · 0 on demand")
                require(browser.tab_count() == 4 and browser.selected() == 3 and not browser.page_panel()
                        and loaded_panels(browser) == 3, "New blank tab changed the loaded views")
            visit(path)
            panels.append(browser.wait(f"page panel for {path}", browser.page_panel))
            if path == "/protected":
                click_and_observe(browser, 0, "/armed")
        browser.win.message(browser.hwnd, 0x8005, 3, 1)  # Pin the second /plain tab.
        before = list(titles(browser))
        child = lambda panel: win.find_child(panel, None, None, None)
        views = [child(panel) for panel in panels]
        require(all(views) and loaded_panels(browser) == 4 and browser.selected() == 3, "Fixture did not load four tabs")
        report["initial_loaded_tabs"] = loaded_panels(browser)
        check_footer("all_loaded", "4 live · 0 on demand")

        confirm(browser, IDNO)
        require([child(panel) for panel in panels] == views, "Cancelling changed the loaded tabs")
        check_footer("after_no", footer_checks["all_loaded"])
        confirm(browser, IDYES)
        browser.wait("releasable background browser gone", lambda: not child(panels[0]), 6)
        browser.wait("all other views retained", lambda: [child(panel) for panel in panels[1:]] == views[1:], 5)
        require(list(titles(browser)) == before and browser.selected() == 3, "Bulk release changed the tab list or selection")
        require(browser.page_panel() == panels[3] and not owned_windows(process, "#32770"), "Active page or dialogs changed")
        require(loaded_panels(browser) == 3, "One tab should have released its web view")
        report["loaded_tabs_after_release"] = loaded_panels(browser)
        check_footer("after_yes", "3 live · 1 on demand")

        def page_state():
            return (browser.selected(), browser.address(), browser.page_panel(),
                    [child(panel) for panel in panels], [site.count(path) for path in PATHS])

        released_state = page_state()
        browser.click(SIDEBAR_TOGGLE)
        browser.wait("sidebar and footer hidden", lambda: not win.visible(browser.controls[TABS])
                     and not win.visible(footer))
        require(page_state() == released_state, "Collapsing the sidebar changed or loaded a fixture page")
        footer_checks["collapsed"] = "hidden"
        browser.click(SIDEBAR_TOGGLE)
        browser.wait("sidebar reopened", lambda: win.visible(browser.controls[TABS]))
        check_footer("reopened", footer_checks["after_yes"])
        require(page_state() == released_state, "Reopening the sidebar changed or loaded a fixture page")

        select(browser, 2)
        browser.wait("protected view visible again", lambda: browser.page_panel() == panels[1])
        click_and_observe(browser, 1, "/draft")
        require(site.drafts == [DRAFT], "Draft was lost while releasing other tabs")
        report["draft_preserved"] = True
        require(site.count("/protected") == 1, "Draft tab reloaded")

        visit("/plain", lambda: select(browser, 1))
        require(browser.page_panel() == panels[0] and child(panels[0]), "Released tab did not reload on selection")
        report["released_tab_reloaded"] = True
        check_footer("after_reload", footer_checks["all_loaded"])
        # Use the existing test fixture's beforeunload UI to exit normally.
        answer(browser, close_dialog(browser, {}), IDYES, {})
        until("normal exit after approving protected page close", process.exited, 12)
        launch["exit_code"] = win.native.GetExitCodeProcess(process.process)
        require(launch["exit_code"] == 0, "Flit did not close normally")
        until("empty owned process job", lambda: not process.pids(), 8)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    report = {"result": "fail"}
    try:
        exe, work = args.exe.resolve(strict=True), args.work.resolve()
        require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
        with exe.with_suffix(".dll").open("rb") as dll:
            report["dll_sha256"] = file_digest(dll, "sha256").hexdigest()
        run(exe, work, report)
        report["result"] = "pass"
    except (RuntimeError, OSError) as error:
        report["error"] = str(error)
    print(json.dumps(report, indent=2))
    raise SystemExit(int(report["result"] != "pass"))
