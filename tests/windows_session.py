"""Check clean session restore in an owned Flit instance with a fresh profile.

Run with --exe PATH/Flit.exe --work WORKSPACE/scratch. Uses only localhost pages.
Pin, tab selection and the startup preference are exercised through Win32 commands.
"""

import argparse
from contextlib import contextmanager
import ctypes as C
import json
from pathlib import Path
import sys
import tempfile

from windows_smoke import (
    Browser, NEW_TAB, OwnedProcess, TAB_TITLE, TABS, WORKSPACE, Win32,
    fixture, page_counts, require, until,
)


PATHS = ("/tab", "/start", "/second")  # Order after pinning the second opened tab.
TITLES = (TAB_TITLE, "Flit smoke: start", "Flit smoke: second")


def send(browser, msg, wparam, lparam=0, *, post=False):
    browser.process.alive()
    require(browser.win.pid(browser.hwnd) == browser.process.pid, "Main window is no longer owned")
    if post:
        browser.win.check(browser.win.post(browser.hwnd, msg, wparam, lparam))
    else:
        browser.win.message(browser.hwnd, msg, wparam, lparam)


def select(browser, index):
    send(browser, 0x0111, 151 + index)  # WM_COMMAND, SELECT_TAB_FIRST + index
    browser.wait(f"selected tab {index}", lambda: browser.selected() == index)


def titles(browser):
    result = []
    for index in range(browser.tab_count()):
        text = C.create_unicode_buffer(512)  # Up to 240 Unicode scalars in a tab label.
        require(browser.message(TABS, 0x0189, index, C.addressof(text)) >= 0, "Could not read tab label")
        result.append(text.value)
    return tuple(result)


def loaded_panels(browser):
    # Include hidden tab panels: a materialized CEF view attaches a child HWND.
    count, panel = 0, None
    while panel := browser.win.find_child(browser.hwnd, panel, "Static", None):
        count += bool(browser.win.find_child(panel, None, None, None))
    return count


def session_is(profile, site, paths, selected=0, restore=True):
    state = json.loads((profile / "session.json").read_text(encoding="utf-8"))
    require([item["url"] for item in state["entries"]] == [site.origin + path for path in paths],
            f"Wrong persisted URLs/order: {state}")
    require(state["selected"] == selected and state["restore"] is restore,
            f"Wrong persisted selection/preference: {state}")
    require([item["pinned"] for item in state["entries"]] == [path == "/tab" for path in paths],
            f"Wrong persisted pin state: {state}")
    return state


def run(exe, work, evidence):
    require(exe.is_file(), f"Missing executable: {exe}")
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-session-", dir=work)) / "profile"
    profile.mkdir()
    evidence["profile"] = str(profile)
    win = Win32()

    with fixture() as site:
        @contextmanager
        def launch(name, path=None):
            report = evidence["launches"][name] = {}
            args = [f"--flit-profile={profile}"]
            if path:
                args.append(f"--flit-url={site.origin}{path}")
            with OwnedProcess(win, exe, args, report) as process:
                browser = Browser(process, site)
                try:
                    yield browser
                except Exception:
                    if not process.exited() and win.pid(browser.hwnd) == process.pid:
                        try:
                            process.normal_close(browser.hwnd)
                        except Exception as error:
                            report["close_error"] = str(error)
                    raise
                else:
                    if not report.get("final_tab_closed"):
                        process.normal_close(browser.hwnd)

        def counts(paths):
            return {path: page_counts(site, path) for path in paths}

        def at_home(browser):
            browser.wait("one blank startup tab", lambda: (
                browser.tab_count() == 1 and browser.selected() == 0 and not browser.address()
            ))
            require(loaded_panels(browser) == 0, "Blank startup materialized a page")

        try:
            with launch("seed", "/start") as browser:
                browser.page("/start", require_request=True)
                for index, path in enumerate(("/tab", "/second"), start=1):
                    browser.click(NEW_TAB)
                    browser.wait(f"new tab {index}", lambda: browser.selected() == index)
                    browser.navigate(path)
                send(browser, 0x8005, 2, 1)  # WM_APP + 5, key of the second opened tab, TAB_PIN.
                select(browser, 1)
                browser.wait("pinned order and fixture titles", lambda: titles(browser) == TITLES)
                require(browser.address() == site.origin + "/start", "Wrong tab selected after pin")
            state = session_is(profile, site, PATHS, selected=1)
            require(tuple(item["title"] for item in state["entries"]) == TITLES, "Tab titles were not saved")

            before = counts(PATHS)
            background = ("/tab", "/second")
            with launch("lazy_restore") as browser:
                browser.page("/start", require_request=True, baseline=before["/start"])
                require(browser.selected() == 1 and titles(browser) == TITLES,
                        "Restore changed tab order, titles or selection")
                report = browser.process.report
                report["panels_before_selection"] = loaded_panels(browser)
                report["requests_before_selection"] = counts(background)
                require(report["panels_before_selection"] == 1, "A background page was materialized at startup")
                require(counts(background) == {path: before[path] for path in background},
                        "A background page requested content before selection")
                browser.page("/tab", lambda: select(browser, 0), require_request=True)
                report["panels_after_selection"] = loaded_panels(browser)
                require(report["panels_after_selection"] == 2, "Selected background page did not materialize")
            require(page_counts(site, "/second") == before["/second"], "Never selected tab loaded during shutdown")
            session_is(profile, site, PATHS, selected=0)

            before = counts((*PATHS, "/first"))
            with launch("explicit_url", "/first") as browser:
                browser.page("/first", require_request=True, baseline=before["/first"])
                require(browser.tab_count() == 1 and browser.selected() == 0, "Explicit URL restored other tabs")
                require(counts(PATHS) == {path: before[path] for path in PATHS}, "Explicit startup loaded stored URLs")
                send(browser, 0x0111, 145)  # WM_COMMAND, TOGGLE_RESTORE.
            session_is(profile, site, (), restore=False)

            before = page_counts(site, "/first")
            with launch("restore_disabled_and_final_tab_close") as browser:
                at_home(browser)
                send(browser, 0x0111, 145)
                browser.navigate("/restart")
                send(browser, 0x0111, 106, post=True)  # WM_COMMAND, CLOSE_TAB.
                process = browser.process
                until("exit after closing final tab", process.exited, 20)
                code = win.native.GetExitCodeProcess(process.process)
                process.report["exit_code"] = code
                until("empty process job after final tab close", lambda: not process.pids(), 8)
                require(code == 0, f"Final tab closure exited with 0x{code:08x}")
                process.report["final_tab_closed"] = True
            require(page_counts(site, "/first") == before, "Disabled restore loaded the stored URL")
            session_is(profile, site, ())

            with launch("empty_restore") as browser:
                at_home(browser)

            evidence["launches"]["interrupted"] = report = {}
            with OwnedProcess(win, exe, [f"--flit-profile={profile}", "--flit-url=" + site.origin + "/first"], report) as process:
                browser = Browser(process, site)
                browser.page("/first", require_request=True)
                browser.click(NEW_TAB)
                browser.navigate("/second")

                def saved():
                    path = profile / "session.json"
                    if not path.is_file():
                        return False
                    state = json.loads(path.read_text(encoding="utf-8"))
                    return (state["selected"] == 1 and
                            [entry["url"] for entry in state["entries"]] ==
                            [site.origin + "/first", site.origin + "/second"])

                browser.wait("recent session snapshot before an unexpected exit", saved)
                # Simulate an abrupt exit of the disposable browser, never the user's app.
                win.check(win.kill_job(process.job, 1))
                until("empty interrupted test job", lambda: not process.pids(), 8)
                report["snapshot_recovered_after_abrupt_exit"] = True

            before = counts(("/first", "/second"))
            with launch("recover_interrupted_session") as browser:
                browser.page("/second", require_request=True, baseline=before["/second"])
                require(browser.tab_count() == 2 and browser.selected() == 1 and loaded_panels(browser) == 1,
                        "Recovery did not select only the previously active tab")
                require(page_counts(site, "/first") == before["/first"], "Background recovery loaded a page")
        finally:
            evidence["fixture"] = site.report()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Browser runtime verification requires Windows")
    evidence = {"result": "fail", "launches": {}}
    try:
        run(args.exe.resolve(), args.work.resolve(), evidence)
        evidence["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(evidence, indent=2, ensure_ascii=True))
    return int(evidence["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
