"""Verify bulk background unload without touching the active tab, a pin, or an unsaved draft."""

import argparse
from hashlib import file_digest
import json
from pathlib import Path
import tempfile

from windows_close_cancel import DRAFT, answer, click_and_observe, close_dialog
from windows_download_close import owned_windows, post
from windows_downloads import seed_profile
from windows_session import loaded_panels, select, titles
from windows_smoke import Browser, NEW_TAB, OwnedProcess, WORKSPACE, Win32, require, until
from windows_close_cancel import fixture


FREE_MEMORY, WM_COMMAND = 167, 0x0111
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
    with OwnedProcess(
        Win32(), exe, [f"--flit-profile={profile}", f"--flit-url={site.origin}/plain"], launch,
    ) as process:
        browser = Browser(process, site)
        def visit(path, action=None):
            before = browser.site.count("/shown" + path)
            if action:
                action()
            browser.wait(f"fixture {path} shown", lambda: browser.address() == site.origin + path
                         and site.count("/shown" + path) > before)

        browser.wait("initial plain fixture shown", lambda: browser.address() == site.origin + "/plain"
                     and site.count("/shown/plain") > 0)
        panels = [browser.wait("releasable page panel", browser.page_panel)]
        for path in ("/protected", "/plain", "/active"):
            browser.click(NEW_TAB)
            def navigate():
                browser.edit_address(site.origin + path)
                browser.click(107)
            visit(path, navigate)
            panels.append(browser.wait(f"page panel for {path}", browser.page_panel))
            if path == "/protected":
                click_and_observe(browser, 0, "/armed")
        browser.win.message(browser.hwnd, 0x8005, 3, 1)  # Pin the second /plain tab.
        before = list(titles(browser))
        win = browser.win
        child = lambda panel: win.find_child(panel, None, None, None)
        views = [child(panel) for panel in panels]
        require(all(views) and loaded_panels(browser) == 4 and browser.selected() == 3, "Fixture did not load four tabs")
        report["initial_loaded_tabs"] = loaded_panels(browser)

        confirm(browser, IDNO)
        require([child(panel) for panel in panels] == views, "Cancelling changed the loaded tabs")
        confirm(browser, IDYES)
        browser.wait("releasable background browser gone", lambda: not child(panels[0]), 6)
        browser.wait("all other views retained", lambda: [child(panel) for panel in panels[1:]] == views[1:], 5)
        require(list(titles(browser)) == before and browser.selected() == 3, "Bulk release changed the tab list or selection")
        require(browser.page_panel() == panels[3] and not owned_windows(process, "#32770"), "Active page or dialogs changed")
        require(loaded_panels(browser) == 3, "One tab should have released its web view")
        report["loaded_tabs_after_release"] = loaded_panels(browser)

        select(browser, 2)
        browser.wait("protected view visible again", lambda: browser.page_panel() == panels[1])
        click_and_observe(browser, 1, "/draft")
        require(site.drafts == [DRAFT], "Draft was lost while releasing other tabs")
        report["draft_preserved"] = True
        require(site.count("/protected") == 1, "Draft tab reloaded")

        visit("/plain", lambda: select(browser, 1))
        require(browser.page_panel() == panels[0] and child(panels[0]), "Released tab did not reload on selection")
        report["released_tab_reloaded"] = True
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
