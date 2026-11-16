"""Check lazy startup window restore through owned Win32 messages and a loopback fixture."""

import argparse
import ctypes as C
from ctypes import wintypes as W
from hashlib import file_digest
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import time

sys.dont_write_bytecode = True
from windows_multi_session import clean_scratch
from windows_session import loaded_panels, send, titles
from windows_smoke import OwnedProcess, WORKSPACE, Win32, require, until
from windows_windows import NORMAL_TITLE, Window, fixture, seed_profile


SCRATCH = WORKSPACE / "work" / "windows-deferred-windows"
SELECTED = {"primary": 1, "extra-a": 1, "extra-b": 0}
PATHS = {name: tuple(f"/{name}/tab-{index}" for index in range(2)) for name in SELECTED}
WM_ACTIVATE, WA_INACTIVE, WA_ACTIVE = 0x0006, 0, 1
MINIMIZED = 1 << 16
QUIET_SECONDS = 1.0


def window_state(window):
    try:
        rows = titles(window)
    except RuntimeError as error:
        if str(error) != "Could not read tab label":
            raise
        rows = None  # The ListBox can rebuild between LB_GETCOUNT and LB_GETTEXT.
    return {
        "address": window.address(), "caption": window.title(), "selected": window.selected(),
        "titles": rows, "loaded_panels": loaded_panels(window), "visible_panel": bool(window.page_panel()),
    }


def exercise(process, site, stored, report):
    active = {name: PATHS[name][index] for name, index in SELECTED.items()}
    labels = {site.origin + path: name for name, path in active.items()}
    observations = report["observations"] = {}
    messages = report["messages"] = []
    owned = report["windows"] = {}

    def three_handles():
        handles = Window.handles(process)
        report["visible_hwnds"] = sorted(handles)
        require(len(handles) <= len(SELECTED), f"Unexpected owned HWNDs: {sorted(handles)}")
        return handles if len(handles) == len(SELECTED) else None

    handles = until("three visible restored HWNDs", three_handles, 35, process.alive)
    windows = {}
    for hwnd in sorted(handles):
        window = Window(process, site, hwnd)
        name = window.wait(f"saved address in HWND 0x{hwnd:x}", lambda: labels.get(window.address()))
        require(name not in windows, f"Multiple restored HWNDs for {name}")
        windows[name] = window
        owned[name] = {"hwnd": hwnd, "pid": window.win.pid(hwnd)}

    def observe(stage, *loaded):
        expected_requests, expected_windows = {}, {}
        for name, index in SELECTED.items():
            live = name in loaded
            rows = tuple(
                site.title(site.routes[path]) if live and i == index else stored[path]
                for i, path in enumerate(PATHS[name])
            )
            expected_windows[name] = {
                "address": site.origin + active[name], "caption": f"{NORMAL_TITLE} — {rows[index]}",
                "selected": index, "titles": rows, "loaded_panels": int(live), "visible_panel": live,
            }
            for path in PATHS[name]:
                expected_requests[path] = expected_requests[path + "/ready"] = int(live and path == active[name])
        phase = observations[stage] = {
            "expected_windows": expected_windows, "expected_requests": expected_requests,
        }
        ready_since = None

        def check():
            nonlocal ready_since
            visible = Window.handles(process)
            phase["visible_hwnds"] = sorted(visible)
            require(visible == handles, f"{stage}: a restored HWND appeared, disappeared or became hidden")
            phase["windows"] = states = {name: window_state(window) for name, window in windows.items()}
            with site.lock:
                phase["requests"] = counts = {path: len(site.requests.get(path, ())) for path in expected_requests}
            # Fail on eager loads even while titles and selections are still settling.
            for path, count in counts.items():
                require(count <= expected_requests[path],
                        f"{stage}: unexpected request count for {path}: {count}, expected {expected_requests[path]}")
            for name, state in states.items():
                require(state["loaded_panels"] <= int(name in loaded)
                        and (name in loaded or not state["visible_panel"]),
                        f"{stage}: unexpected CEF panel in {name}: {state}")
            if states != expected_windows or counts != expected_requests:
                ready_since = None
                return False
            now = time.monotonic()
            if ready_since is None:
                ready_since = now
            return now - ready_since >= QUIET_SECONDS

        until(f"{stage}: restored UI and fixture requests unchanged for {QUIET_SECONDS}s", check, 20, process.alive)

    def activation(name, wparam):
        window = windows[name]
        window.owned()
        send(window, WM_ACTIVATE, wparam)
        messages.append({"window": name, "hwnd": window.hwnd, "message": WM_ACTIVATE, "wparam": wparam, "lparam": 0})

    observe("startup", "primary")
    user = C.WinDLL("user32", use_last_error=True)
    show = user.ShowWindow
    show.argtypes, show.restype = (W.HWND, C.c_int), W.BOOL
    iconic = user.IsIconic
    iconic.argtypes, iconic.restype = (W.HWND,), W.BOOL
    for name in ("extra-a", "extra-b"):
        activation(name, WA_INACTIVE)
        window = windows[name]
        window.owned()
        show(window.hwnd, 7)  # SW_SHOWMINNOACTIVE
        window.wait("owned window is actually minimized", lambda: iconic(window.hwnd))
        activation(name, WA_ACTIVE | MINIMIZED)
    observe("ignored_activation", "primary")
    for name in ("extra-a", "extra-b"):
        window = windows[name]
        window.owned()
        show(window.hwnd, 4)  # SW_SHOWNOACTIVATE
        window.wait("owned window restored without activation", lambda: not iconic(window.hwnd))
    activation("extra-a", WA_ACTIVE)
    observe("selected_activation", "primary", "extra-a")
    activation("extra-a", WA_INACTIVE)
    activation("extra-a", WA_ACTIVE)
    activation("extra-a", WA_ACTIVE)
    observe("repeat_activation", "primary", "extra-a")


def run(exe, profile, report):
    with fixture() as site:
        site.routes = {path: path[1:] for tabs in PATHS.values() for path in tabs}
        stored = {path: f"Saved: {page}" for path, page in site.routes.items()}
        seed_profile(profile, site.origin)
        states = {
            name: {
                "entries": [{"url": site.origin + path, "title": stored[path], "pinned": False}
                            for path in PATHS[name]],
                "selected": index,
            }
            for name, index in SELECTED.items()
        }
        report["seed"] = {
            **states["primary"], "additional_windows": [states["extra-a"], states["extra-b"]],
            "restore": True, "defer_background": False,
        }
        (profile / "session.json").write_text(json.dumps(report["seed"]), encoding="utf-8")
        launch = report["launch"] = {}
        try:
            # Job teardown avoids activating the parked HWND by closing its neighbors.
            with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}"], launch) as process:
                exercise(process, site, stored, report)
        finally:
            with site.lock:
                report["fixture_hits"] = {path: len(items) for path, items in site.requests.items()}
        expected = {path: count for path, count in report["observations"]["repeat_activation"]["expected_requests"].items()
                    if count}
        require(report["fixture_hits"] == expected, "Unexpected final fixture request counts")


def hashes(exe, dll):
    result = {}
    for file in (exe, dll):
        with file.open("rb") as stream:
            result[file.name] = file_digest(stream, "sha256").hexdigest()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, default=SCRATCH, help=f"scratch directory within {SCRATCH}")
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")
    if not work.is_relative_to(SCRATCH):
        parser.error(f"--work must be inside {SCRATCH}")

    report = {"result": "fail", "exe": str(exe), "quiet_seconds": QUIET_SECONDS}
    try:
        report["sha256"] = hashes(exe, dll)
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-", dir=work)
        try:
            profile = Path(scratch.name) / "profile"
            report["profile"] = str(profile)
            run(exe, profile, report)
        finally:
            clean_scratch(scratch, report)
        require("scratch_cleanup_error" not in report, "Could not remove the disposable profile")
        require(hashes(exe, dll) == report["sha256"], "Staged Flit binaries changed during the run")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
