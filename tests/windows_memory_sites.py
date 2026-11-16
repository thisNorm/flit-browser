"""Verify site exceptions across windows and restarts using only loopback pages."""

import argparse
from hashlib import file_digest
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

sys.dont_write_bytecode = True
from windows_free_memory import IDYES, confirm
from windows_multi_session import clean_scratch
from windows_session import loaded_panels, send, titles
from windows_smoke import Browser, GO, NEW_TAB, OwnedProcess, Win32, WORKSPACE, fixture, require
from windows_windows import NEW_WINDOW, PRIVATE_WINDOW, Window


TAB_MENU_ACTION, TAB_KEEP_SITE = 0x8005, 6
def page(browser, site, path, host):
    url = site.origin.replace("127.0.0.1", host, 1) + path
    shown = site.count("/shown" + path)
    browser.edit_address(url)
    browser.click(GO)
    browser.wait(f"{host}{path} in the active tab", lambda: (
        browser.address() == url and site.count("/shown" + path) > shown
    ))
    panel = browser.wait(f"visible web panel for {host}{path}", browser.page_panel)
    browser.wait(f"title for {path}", lambda: browser.has_page_title(f"Flit smoke: {path[1:]}"))
    return panel


def demo(site, browser):
    kept = page(browser, site, "/first", "127.0.0.1")
    browser.click(NEW_TAB)
    ordinary = page(browser, site, "/second", "localhost")
    browser.click(NEW_TAB)
    active = page(browser, site, "/start", "localhost")
    require(loaded_panels(browser) == 3, "Fixture did not load three pages")
    return kept, ordinary, active


def run(exe, work, report):
    scratch = TemporaryDirectory(prefix="flit-memory-sites-", dir=work)
    try:
        with fixture() as site:
            exercise(exe, Path(scratch.name) / "profile", site, report)
    finally:
        clean_scratch(scratch, report)
    require("scratch_cleanup_error" not in report, "Could not remove the disposable profile")


def exercise(exe, profile, site, report):
    args = [f"--flit-profile={profile}", "--flit-url=about:blank"]
    observed = report["runs"] = {}
    for label, protected in [("new", True), ("restart", True), ("removed", False)]:
        evidence = observed[label] = {}
        with OwnedProcess(Win32(), exe, args, evidence) as process:
            root = Window(process, site)
            win = process.win
            if label == "new":
                root.blank()
                page(root, site, "/start", "127.0.0.1")
                send(root, TAB_MENU_ACTION, 1, TAB_KEEP_SITE)
                browser = root.open_window(NEW_WINDOW, "another_normal_window", evidence)
            else:
                browser = root
            panels = demo(site, browser)
            child = lambda panel: win.find_child(panel, None, None, None)
            views = [child(panel) for panel in panels]
            labels = titles(browser)
            confirm(browser, IDYES)
            browser.wait("unprotected background site was released", lambda: not child(panels[1]), 6)
            if protected:
                require(child(panels[0]) == views[0], "Excepted background site was released")
            else:
                browser.wait("removed exception is eligible again", lambda: not child(panels[0]), 6)
            require(child(panels[2]) == views[2] and browser.page_panel() == panels[2],
                    "Free memory affected the active tab")
            require(titles(browser) == labels and browser.selected() == 2, "Free memory changed the sidebar")
            evidence["expected_protection"] = protected
            evidence["was_protected"] = child(panels[0]) == views[0]
            evidence["loaded_pages_after"] = loaded_panels(browser)
            if label == "new":
                private = root.open_window(PRIVATE_WINDOW, "private_window", evidence)
                page(private, site, "/first", "127.0.0.1")
                send(private, TAB_MENU_ACTION, 1, TAB_KEEP_SITE)  # Ignored in private mode.
                private.click(NEW_TAB)
                page(private, site, "/start", "localhost")
                confirm(private, IDYES)
                private.wait("private tab was released", lambda: loaded_panels(private) == 1, 6)
                private.close_window()
                evidence["private_mode_separate"] = True
            if label == "restart":
                send(browser, TAB_MENU_ACTION, 1, TAB_KEEP_SITE)
            if browser.hwnd != root.hwnd:
                browser.close_window()
            process.normal_close(root.hwnd)
    require(set(observed) == {"new", "restart", "removed"}, "Missing test steps")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    if sys.platform != "win32" or not work.is_relative_to(WORKSPACE / "work"):
        parser.error("Windows required; --work must be inside the workspace's work directory")
    report = {"result": "fail", "exe": str(exe)}
    try:
        with exe.with_name("Flit.dll").open("rb") as dll:
            report["dll_sha256"] = file_digest(dll, "sha256").hexdigest()
        work.mkdir(parents=True, exist_ok=True)
        run(exe, work, report)
        report["result"] = "pass"
    except (RuntimeError, OSError) as error:
        report["error"] = str(error)
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
