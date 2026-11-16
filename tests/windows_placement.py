"""Check Win32 session geometry in one owned Flit job with a disposable profile."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

sys.dont_write_bytecode = True
from windows_multi_session import EXIT_FLIT, clean_scratch, read_snapshot, wait_stopped
from windows_session import loaded_panels, send
from windows_smoke import OwnedProcess, WORKSPACE, Win32, api, require, until
from windows_windows import WM_COMMAND, Window, fixture, seed_profile


SCRATCH = WORKSPACE / "work" / "windows-placement"
PAGES = {"primary": "normal-seed", "maximized": "normal-peer", "stale": "normal-restart"}
WM_SYSCOMMAND, SC_RESTORE = 0x0112, 0xF120
SWP_NOZORDER, SWP_NOACTIVATE = 0x0004, 0x0010


class MonitorInfo(C.Structure):
    _fields_ = [("size", W.DWORD), ("monitor", W.RECT), ("work", W.RECT), ("flags", W.DWORD)]


def placement(x, y, width, height, maximized=False):
    return dict(x=x, y=y, width=width, height=height, maximized=maximized)


def edges(saved):
    x, y = saved["x"], saved["y"]
    return x, y, x + saved["width"], y + saved["height"]


def placements(path):
    if state := read_snapshot(path):
        return {window["entries"][0]["url"]: window.get("placement")
                for window in [state, *state.get("additional_windows", [])]}


def run(exe, profile, report):
    win = Win32()
    user = C.WinDLL("user32", use_last_error=True)
    from_point = api(user, "MonitorFromPoint", W.HANDLE, W.POINT, W.DWORD)
    from_window = api(user, "MonitorFromWindow", W.HANDLE, W.HWND, W.DWORD)
    monitor_info = api(user, "GetMonitorInfoW", W.BOOL, W.HANDLE, C.POINTER(MonitorInfo))
    is_zoomed = api(user, "IsZoomed", W.BOOL, W.HWND)

    def work_bounds(monitor):
        info = MonitorInfo(C.sizeof(MonitorInfo))
        win.check(monitor_info(win.check(monitor), C.byref(info)))
        return info.work.left, info.work.top, info.work.right, info.work.bottom

    # Measure and move the HWNDs in the same physical coordinates as Flit.
    dpi_context = win.check(win.thread_dpi_context(C.c_void_p(-4)))
    try:
        left, top, right, bottom = work_bounds(from_point(W.POINT(0, 0), 1))
        width, height = right - left, bottom - top
        primary = placement(left + 20, top + 20, width - 80, height - 80)
        maximized = placement(left + 40, top + 40, width - 120, height - 120, True)
        stale = placement(200_000, 200_000, 32_000, 20_000)
        with fixture() as site:
            seed_profile(profile, site.origin)
            current, backup = profile / "session.json", profile / "session.previous.json"
            urls = {label: site.origin + site.paths[page] for label, page in PAGES.items()}
            seeded = dict(zip(urls.values(), (primary, maximized, stale)))
            states = [
                {"entries": [{"url": url, "title": label}], "selected": 0, "placement": seeded[url]}
                for label, url in urls.items()
            ]
            current.write_text(json.dumps({**states[0], "additional_windows": states[1:]}), encoding="utf-8")

            with OwnedProcess(win, exe, [f"--flit-profile={profile}"], report) as process:
                def restored_handles():
                    found = Window.handles(process)
                    require(len(found) <= 3, f"Unexpected extra Flit windows: {found}")
                    return found if len(found) == 3 else None

                handles = until("three restored Flit HWNDs", restored_handles, 35, process.alive)
                windows = {}
                labels = {url: label for label, url in urls.items()}
                for hwnd in handles:
                    window = Window(process, site, hwnd)
                    label = window.wait("restored tab address", lambda: labels.get(window.address()))
                    require(label not in windows, f"Duplicate restored window: {label}")
                    windows[label] = window
                first, second, detached = (windows[label] for label in PAGES)
                first.wait("primary page ready", lambda: site.seen(site.paths[PAGES["primary"]] + "/ready"))
                report["restored_primary"] = win.rect(first.hwnd)
                require(report["restored_primary"] == edges(primary) and not is_zoomed(first.hwnd),
                        f"Primary bounds differ from the session: {report['restored_primary']}")
                require(is_zoomed(second.hwnd), "Secondary did not reopen maximized")
                require(placements(backup) == seeded, "Startup copy lost a window's saved placement")

                report["stale_rect"] = x1, y1, x2, y2 = win.rect(detached.hwnd)
                work = work_bounds(from_window(detached.hwnd, 2))
                require(work[0] <= x1 < x2 <= work[2] and work[1] <= y1 < y2 <= work[3],
                        f"Saved position from a detached display is outside the work area {work}")
                first.wait("startup save after correcting the off-screen window",
                           lambda: (placements(current) or {}).get(urls["stale"])
                           == placement(x1, y1, x2 - x1, y2 - y1))

                moved = {**primary, "x": primary["x"] + 15, "width": primary["width"] - 10}
                first.owned()
                win.check(win.set_window_pos(first.hwnd, None, moved["x"], moved["y"],
                                            moved["width"], moved["height"], SWP_NOZORDER | SWP_NOACTIVATE))
                first.wait("move and resize saved without tab activity",
                           lambda: (placements(current) or {}).get(urls["primary"]) == moved)
                live = placements(current)
                require(live.get(urls["maximized"]) == maximized,
                        "Saving a maximized HWND lost its normal bounds")
                for label in ("maximized", "stale"):
                    require(not loaded_panels(windows[label]) and not site.seen(site.paths[PAGES[label]]),
                            f"{label} background window loaded before activation")

                send(second, WM_SYSCOMMAND, SC_RESTORE)
                second.wait("secondary unmaximized", lambda: not is_zoomed(second.hwnd))
                report["unmaximized_secondary"] = win.rect(second.hwnd)
                require(report["unmaximized_secondary"] == edges(maximized),
                        f"Unmaximizing lost the saved normal bounds: {report['unmaximized_secondary']}")
                send(first, WM_COMMAND, EXIT_FLIT, post=True)
                wait_stopped(process, "Exit Flit")
    finally:
        win.check(win.thread_dpi_context(dpi_context))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", default=SCRATCH, type=Path, help=f"directory inside {SCRATCH}")
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    if (sys.platform != "win32" or exe.name.casefold() != "flit.exe"
            or not exe.is_file() or not exe.with_name("Flit.dll").is_file()):
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")
    if not work.is_relative_to(SCRATCH):
        parser.error(f"--work must be inside {SCRATCH}")
    report = {"result": "fail"}
    try:
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-", dir=work)
        try:
            run(exe, Path(scratch.name) / "profile", report)
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
