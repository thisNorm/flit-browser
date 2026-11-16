"""Check multi-window session recovery using one disposable Flit job at a time."""

import argparse
from hashlib import file_digest
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import time

sys.dont_write_bytecode = True
from windows_session import loaded_panels, select, send, titles
from windows_smoke import NEW_TAB, OwnedProcess, WORKSPACE, Win32, require, until
from windows_windows import (
    NEW_WINDOW, NORMAL_TITLE, PRIVATE_WINDOW, WM_COMMAND, Window, fixture, seed_profile, visit,
)


SCRATCH = WORKSPACE / "work" / "windows-multi-session"
EXIT_FLIT, RECOVERY_COPY = 172, 173
TAB_MENU_ACTION, TAB_PIN = 0x8005, 1
NORMALS = {
    "first": (("normal-peer", "normal-seed"), 1),
    "second": (("normal-restart", "normal-after-private"), 0),
}
PRIVATE_PAGE = "private-empty"


def expected_window(site, pages, selected):
    return {
        "entries": [
            {"url": site.origin + site.paths[page], "title": site.title(page), "pinned": index == 0}
            for index, page in enumerate(pages)
        ],
        "selected": selected,
    }


def read_snapshot(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, PermissionError, UnicodeDecodeError, json.JSONDecodeError):
        return None


def matches(state, expected):
    if not isinstance(state, dict) or not isinstance(extra := state.get("additional_windows", []), list):
        return False
    fields = ("entries", "selected")
    # HWND creation/activation order can vary; each window's tab order cannot.
    windows = [{key: item.get(key) for key in fields} for item in [state, *extra] if isinstance(item, dict)]
    return len(windows) == 1 + len(extra) == len(expected) and all(item in windows for item in expected)


def await_snapshot(owner, path, expected, evidence, key, *, changed_from=None):
    def saved():
        evidence[key] = state = read_snapshot(path)
        if not matches(state, expected):
            return False
        try:
            return changed_from is None or path.stat().st_mtime_ns != changed_from
        except (FileNotFoundError, PermissionError):
            return False

    owner.wait(f"{key}: {len(expected)} windows in a fresh session.json (entries / additional_windows)", saved, 8)


def check_snapshot(path, expected, evidence, key):
    evidence[key] = state = read_snapshot(path)
    require(matches(state, expected), f"{key}: unexpected {path.name} contents: {state}")
    return state


def requests(site, page):
    path = site.paths[page]
    return len(site.seen(path)), len(site.seen(path + "/ready"))


def idle_requests(site):
    pages = [tabs[1 - selected] for tabs, selected in NORMALS.values()]
    return {page: requests(site, page) for page in [*pages, PRIVATE_PAGE]}


def check_idle(site, before, evidence):
    evidence["idle_requests"] = {"before": before, "after": idle_requests(site)}
    require(evidence["idle_requests"]["after"] == before, "A background or private page requested content during recovery")


def check_window(window, site, label, before, evidence):
    pages, index = NORMALS[label]
    active = pages[index]
    caption = f"{NORMAL_TITLE} — {site.title(active)}"
    window.wait(f"{label}: selected page rendered in restored HWND", lambda: (
        all(new > old for new, old in zip(requests(site, active), before[active]))
        and window.title() == caption and window.address() == site.origin + site.paths[active]
        and window.page_panel()
    ))
    evidence[label] = observed = {
        "hwnd": window.hwnd, "address": window.address(), "selected": window.selected(),
        "titles": titles(window), "loaded_panels": loaded_panels(window),
    }
    require(observed["titles"] == tuple(map(site.title, pages)) and observed["selected"] == index,
            f"{label}: restored tab order, titles or selection changed: {observed}")
    require(observed["loaded_panels"] == 1, f"{label}: expected exactly one loaded CEF view")


def recovered_windows(process, site, before, evidence):
    expected = {
        site.origin + site.paths[pages[selected]]: label
        for label, (pages, selected) in NORMALS.items()
    }

    def two_handles():
        handles = Window.handles(process)
        require(len(handles) <= 2, f"Expected only two restored normal HWNDs; got {sorted(handles)}")
        return handles if len(handles) == 2 else None

    handles = until("two restored normal windows", two_handles, 35, process.alive)
    windows = {}
    evidence["windows"] = observed = {}
    for hwnd in sorted(handles):
        window = Window(process, site, hwnd)
        label = window.wait(f"saved selected address in HWND 0x{hwnd:x}", lambda: expected.get(window.address()))
        require(label not in windows, f"Multiple HWNDs restored {label}")
        send(window, 0x0006, 1)  # WM_ACTIVATE, WA_ACTIVE with a zero minimize word.
        check_window(window, site, label, before, observed)
        windows[label] = window
    require(Window.handles(process) == handles, "Restoring the session added or lost a window")
    check_idle(site, before["idle"], evidence)
    return windows


def wait_stopped(process, label, expected_code=0):
    until(f"process exit after {label}", process.exited, 20)
    until(f"empty job after {label}", lambda: not process.pids(), 8)
    process.report["exit_code"] = code = process.win.native.GetExitCodeProcess(process.process)
    require(code == expected_code, f"{label} exited with 0x{code:08x}, expected {expected_code}")


def run(exe, profile, report):
    primary, previous = profile / "session.json", profile / "session.previous.json"
    win = Win32()
    launches = report["launches"] = {}
    with fixture() as site:
        seed_profile(profile, site.origin)
        args = [f"--flit-profile={profile}"]
        first, second = [expected_window(site, *spec) for spec in NORMALS.values()]
        expected = (first, second)

        def launch(label):
            launches[label] = evidence = {}
            return OwnedProcess(win, exe, args, evidence)

        def before_restart():
            before = {pages[index]: requests(site, pages[index]) for pages, index in NORMALS.values()}
            return {**before, "idle": idle_requests(site)}

        with launch("interrupted") as process:
            first_window = Window(process, site)
            evidence = process.report
            checks = evidence["seed_pages"] = {}
            for label, (pages, index) in NORMALS.items():
                window = first_window if label == "first" else first_window.open_window(NEW_WINDOW, label, evidence)
                window.blank()
                visit(window, pages[1], "normal", checks)
                window.command(NEW_TAB)
                window.blank(tabs=2, all_blank=False)
                visit(window, pages[0], "normal", checks)
                send(window, TAB_MENU_ACTION, 2, TAB_PIN)  # The second created tab moves to the front.
                select(window, index)
            private = first_window.open_window(PRIVATE_WINDOW, "private", evidence)
            private.blank(private=True)
            visit(private, PRIVATE_PAGE, None, checks)
            require(len(Window.handles(process)) == 3, "Expected two normal windows and one private window")
            await_snapshot(first_window, primary, expected, evidence, "before_crash")
            require(not previous.exists(), "A recovery copy appeared before the first restart")
            # Terminate only the job that was created suspended for this scratch profile.
            win.check(win.kill_job(process.job, 1))
            wait_stopped(process, "owned job termination", expected_code=1)

        interrupted_mtime = primary.stat().st_mtime_ns
        before = before_restart()
        with launch("crash_recovery_and_exit") as process:
            evidence = process.report
            windows = recovered_windows(process, site, before, evidence)
            backup = check_snapshot(previous, expected, evidence, "startup_recovery_copy")
            first_window = windows["first"]
            await_snapshot(first_window, primary, expected, evidence, "restored_live_pins",
                           changed_from=interrupted_mtime)
            windows["second"].close_window()  # WM_CLOSE, the same close path as the title bar X.
            await_snapshot(first_window, primary, (first,), evidence, "after_window_x")
            check_idle(site, before["idle"], evidence)
            require(read_snapshot(previous) == backup, "The recovery copy changed during the live session")

            manual_before = before_restart()
            from_copy = first_window.open_window(RECOVERY_COPY, "manual_recovery_copy", evidence)
            from_copy.wait("saved address in the backup window",
                           lambda: from_copy.address() == site.origin + site.paths["normal-restart"])
            require(loaded_panels(from_copy) == 0 and requests(site, "normal-restart") == manual_before["normal-restart"],
                    "Opening the backup loaded a background window before activation")
            send(from_copy, 0x0006, 1)  # Activate before checking the restored page.
            check_window(from_copy, site, "second", manual_before, evidence["manual_recovery_copy"])
            check_idle(site, manual_before["idle"], evidence["manual_recovery_copy"])
            await_snapshot(first_window, primary, expected, evidence, "after_manual_copy")
            handles = Window.handles(process)
            first_window.command(RECOVERY_COPY)
            require(Window.handles(process) == handles, "Recovery Copy duplicated windows already open")
            # Change a pin immediately before Exit so the saved state must come from the
            # live windows, even if the normal recovery timer has not fired.
            send(first_window, TAB_MENU_ACTION, 1, TAB_PIN, post=True)
            send(first_window, WM_COMMAND, EXIT_FLIT, post=True)
            wait_stopped(process, "Exit Flit")
            check_idle(site, before["idle"], evidence)

        unpinned = {**first, "entries": [{**first["entries"][0], "pinned": False}, first["entries"][1]]}
        check_snapshot(primary, (unpinned, second), report, "after_explicit_exit")
        require(read_snapshot(previous) == backup, "Exit Flit changed the startup recovery copy")

        # Both jobs have stopped. Damage only the primary of this run's disposable profile.
        primary.write_bytes(b'{"entries":[')
        before = before_restart()
        with launch("corrupt_primary") as process:
            evidence = process.report
            windows = recovered_windows(process, site, before, evidence)
            require(read_snapshot(previous) == backup, "Corrupt primary overwrote the valid recovery copy")
            await_snapshot(windows["first"], primary, expected, evidence, "repaired_primary")
            windows["second"].close_window()
            process.normal_close(windows["first"].hwnd)
            check_idle(site, before["idle"], evidence)
        check_snapshot(primary, (first,), report, "after_last_window_x")


def run_exit_veto(exe, profile, report):
    import ctypes as C
    from ctypes import wintypes as W

    import windows_close_cancel as veto
    from windows_download_close import owned_windows
    from windows_smoke import api

    report["launches"] = {"exit_veto": {}}
    evidence = report["launches"]["exit_veto"]
    primary = profile / "session.json"
    with veto.fixture() as site:
        seed_profile(profile, site.origin)
        states = [
            {"entries": [{"url": site.origin + path, "title": title, "pinned": False}], "selected": 0}
            for path, title in zip(veto.PATHS, veto.LABELS)
        ]
        with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}"], evidence) as process:
            first = Window(process, site)
            first.navigate("/plain")
            second = first.open_window(NEW_WINDOW, "second", evidence)
            second.navigate("/active")
            protected = first.open_window(NEW_WINDOW, "third_protected", evidence)
            protected.navigate("/protected")
            veto.click_and_observe(protected, 0, "/armed")
            windows = (first, protected, second)  # Same order as states.
            handles = sorted(window.hwnd for window in windows)
            require(sorted(Window.handles(process)) == handles, "Expected three original normal windows before Exit Flit")
            await_snapshot(first, primary, states, evidence, "before_exit")
            get_owner = api(C.WinDLL("user32", use_last_error=True), "GetWindow", W.HWND, W.HWND, W.UINT)
            for source, label, button in ((first, "decline", veto.IDNO), (protected, "confirm", veto.IDYES)):
                require(not owned_windows(process, "#32770"), "An owned modal dialog was already open")
                if button == veto.IDYES:
                    before_retry_mtime = primary.stat().st_mtime_ns
                send(source, WM_COMMAND, EXIT_FLIT, post=True)
                dialogs = protected.wait("beforeunload confirmation after Exit Flit",
                                         lambda: owned_windows(process, "#32770"), 8)
                require(len(dialogs) == 1 and get_owner(dialogs[0], 4) == protected.hwnd,
                        f"Expected one modal dialog owned by the protected window: {dialogs}")
                dialog = dialogs[0]
                warning = C.create_unicode_buffer(256)
                for child in process.win.visible_children(dialog, "Static"):
                    require(process.win.pid(child) == process.pid, "Dialog control is no longer owned")
                    process.win.message(child, 0x000D, len(warning), C.addressof(warning))  # WM_GETTEXT
                    if warning.value == veto.WARNING:
                        break
                else:
                    raise RuntimeError("Exit Flit dialog did not show the beforeunload warning")
                evidence[label] = {"hwnd": dialog, "warning": warning.value, "remaining_windows": sorted(Window.handles(process))}
                require(evidence[label]["remaining_windows"] == handles,
                        "Exit Flit lost or replaced a window before the beforeunload answer")
                veto.answer(protected, dialog, button, evidence[label])
                if button == veto.IDNO:
                    protected.wait("No dismissed beforeunload and re-enabled the protected HWND", lambda: (
                        not process.win.visible(dialog) and process.win.enabled(protected.hwnd)
                    ))
                    evidence["windows_after_no"] = sorted(Window.handles(process))
                    require(evidence["windows_after_no"] == handles, "Declining Exit Flit lost or replaced a window")
                    for window, state in zip(windows, states):
                        entry = state["entries"][0]
                        require(window.address() == entry["url"] and titles(window) == (entry["title"],)
                                and window.selected() == state["selected"],
                                f"Declining Exit Flit changed the tab in HWND 0x{window.hwnd:x}")
                    veto.click_and_observe(protected, 1, "/draft")
                    with site.lock:
                        evidence["draft_preserved"] = site.drafts[-1] == veto.DRAFT
                    require(evidence["draft_preserved"], "Declining Exit Flit lost the live draft")
                    await_snapshot(protected, primary, states, evidence, "after_veto")
            wait_stopped(process, "confirming Exit Flit after a veto")
        check_snapshot(primary, states, evidence, "after_retry")
        require(primary.stat().st_mtime_ns != before_retry_mtime, "Confirming Exit Flit did not refresh session.json")


def clean_scratch(scratch, report):
    try:
        try:
            scratch.cleanup()
        except PermissionError:
            report["scratch_cleanup_retried"] = True
            time.sleep(.25)
            scratch.cleanup()
    except OSError as error:
        report["scratch_cleanup_error"] = str(error)
        report["scratch_retained"] = scratch.name


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, default=SCRATCH, help="scratch directory for the disposable profile")
    parser.add_argument("--exit-veto", action="store_true", help="run only the optional Exit Flit beforeunload scenario")
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not exe.is_file() or not exe.with_name("Flit.dll").is_file():
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")
    if not work.is_relative_to(SCRATCH):
        parser.error(f"--work must be inside {SCRATCH}")

    report = {"result": "fail", "exe": str(exe)}
    try:
        dll = exe.with_name("Flit.dll")
        with dll.open("rb") as stream:
            report["dll_sha256"] = file_digest(stream, "sha256").hexdigest()
        libcef = exe.with_name("libcef.dll")
        if libcef.is_file():
            with libcef.open("rb") as stream:
                report["libcef_sha256"] = file_digest(stream, "sha256").hexdigest()
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-", dir=work)
        try:
            profile = Path(scratch.name) / "profile"
            report["profile"] = str(profile)
            (run_exit_veto if args.exit_veto else run)(exe, profile, report)
        finally:
            clean_scratch(scratch, report)
        require("scratch_cleanup_error" not in report, "Could not remove the disposable profile")
        with dll.open("rb") as stream:
            require(file_digest(stream, "sha256").hexdigest() == report["dll_sha256"],
                    "Staged Flit.dll changed during the run")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
