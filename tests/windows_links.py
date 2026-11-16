"""Test literal web links on first launch, relaunch, and cold session restore."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from windows_session import loaded_panels, titles
from windows_smoke import Browser, OwnedProcess, TAB_TITLE, Win32, WORKSPACE, fixture, page_counts, require


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-link-", dir=work)) / "profile"
    args = [f"--flit-profile={profile}", "--"]
    win = Win32()
    report["launch"] = {}
    with fixture() as site:
        first = site.origin + "/start"
        second = site.origin + "/second"
        with OwnedProcess(win, exe, [*args, first], report["launch"]) as process:
            browser = Browser(process, site)
            browser.page("/start", require_request=True)
            require(browser.tab_count() == 1, "Cold launch did not open exactly one tab")
            relaunch = {}
            report["second_launch"] = relaunch
            with OwnedProcess(process.win, exe, [*args, second], relaunch) as launcher:
                browser.page("/second", require_request=True)
                browser.wait("link forwarded as one new selected tab",
                             lambda: browser.tab_count() == 2 and browser.selected() == 1)
                browser.wait("the second launcher exited", launcher.exited, timeout=8)
                require(not launcher.pids(), "Relaunch left a second process tree")
                relaunch["exit_code"] = launcher.win.native.GetExitCodeProcess(launcher.process)
            require(browser.win.pid(browser.hwnd) == process.pid, "External link replaced the original browser window")
            process.normal_close(browser.hwnd)

        before = {path: page_counts(site, path) for path in ("/start", "/second")}
        cold = report["cold_restore"] = {"saved_page_counts_before_launch": before}
        with OwnedProcess(win, exe, [*args, site.origin + "/tab"], cold) as process:
            browser = Browser(process, site)
            browser.page("/tab", require_request=True)
            browser.wait("three tabs with the external link selected",
                         lambda: browser.tab_count() == 3 and browser.selected() == 2)
            expected = ("Flit smoke: start", "Flit smoke: second", TAB_TITLE)
            browser.wait("saved rows followed by the external link", lambda: titles(browser) == expected)
            cold["row_titles"] = titles(browser)
            cold["selected"] = browser.selected()
            cold["loaded_panels"] = loaded_panels(browser)
            cold["saved_page_counts_before_selection"] = {path: page_counts(site, path) for path in before}
            require(cold["loaded_panels"] == 1, "Cold link launch materialized a saved tab")
            require(cold["saved_page_counts_before_selection"] == before,
                    "Cold link launch fetched or rendered a saved tab before selection")
            process.normal_close(browser.hwnd)
        require(all(page_counts(site, path) == counts for path, counts in before.items()),
                "An unselected saved tab loaded during shutdown")
        report["requests"] = site.report()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    exe, work = args.exe.resolve(strict=True), args.work.resolve()
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    report = {"result": "fail", "dll_sha256": hashlib.sha256(exe.with_name("Flit.dll").read_bytes()).hexdigest()}
    try:
        run(exe, work, report)
        report["result"] = "pass"
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    raise SystemExit(int(report["result"] != "pass"))
