"""Verify Flit 0.24.0's Recently closed dispatch in an owned Windows process.

Posts the same WM_COMMAND as the native menu selection. Menu labels and layout
are tested separately. Run against a staged bundle; JSON evidence goes to stdout.
"""

import argparse
from hashlib import file_digest
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import time

sys.dont_write_bytecode = True
from windows_multi_session import clean_scratch
from windows_session import send, titles
from windows_smoke import Handler, NEW_TAB, OwnedProcess, TAB_TITLE, WORKSPACE, Win32, fixture, require
from windows_windows import NEW_WINDOW, PRIVATE_WINDOW, WM_COMMAND, Window, seed_profile


REOPEN_LAST, REOPEN_RECENT = 136, 187
TAB_MENU_ACTION, TAB_PIN = 0x8005, 1
OLDER_ID, NEWER_ID, PRIVATE_ID = 1, 2, 3  # The only closes before the dispatch checks.
START, OLDER, NEWER, PRIVATE = "/start", "/tab", "/second", "/first"
LABELS = {"": "New tab", START: "Flit smoke: start", OLDER: TAB_TITLE,
          NEWER: "Flit smoke: second", PRIVATE: "Flit smoke: first"}
UNCHANGED_SECONDS = 0.5


class LoopbackHandler(Handler):
    def do_GET(self):
        # Accept both direct requests and absolute targets from the non-forwarding proxy.
        origin = self.server.origin
        if self.path.startswith(origin + "/"):
            self.path = self.path[len(origin):]
        super().do_GET()


def requests(site):
    counts = site.report()["requests"]
    counts.pop("/favicon.ico", None)  # Chromium may request this asynchronously.
    return counts


def tab_state(window):
    window.owned()
    try:
        rows = titles(window)
    except RuntimeError as error:
        if str(error) != "Could not read tab label":
            raise
        return None  # The owner can rebuild its ListBox between the count and label messages.
    return {"rows": rows, "selected": window.selected(), "address": window.address()}


def expected_tabs(site, paths, selected):
    path = paths[selected]
    return {"rows": tuple(LABELS[p] for p in paths), "selected": selected,
            "address": site.origin + path if path else ""}


def wait_tabs(window, paths, selected):
    expected = expected_tabs(window.site, paths, selected)
    window.wait(f"tabs {paths!r}, selected row {selected}", lambda: tab_state(window) == expected)


def add(window, path):
    window.click(NEW_TAB)
    window.navigate(path)


def exercise(normal, evidence):
    site, process = normal.site, normal.process
    windows = {"normal": normal}
    evidence["windows"] = opened = {"normal": {"hwnd": normal.hwnd, "pid": process.pid}}
    evidence["checks"] = checks = {}

    def snapshot():
        handles = Window.handles(process)
        states = {name: tab_state(window) for name, window in windows.items() if window.hwnd in handles}
        if any(state is None for state in states.values()):
            return None
        return {"hwnds": sorted(handles), "windows": states, "requests": requests(site)}

    def reopen(label, sender, command, history_id=0, *, restore=None, timeout=12):
        window = windows[sender]
        before = window.wait("tab rows settled before reopen", snapshot)
        expected = before
        require(set(before["hwnds"]) == {w.hwnd for w in windows.values()}, f"{label}: unexpected owned windows")
        if restore:
            target, paths, selected = restore
            path = paths[selected]
            loads = before["requests"].copy()
            for route in (path, "/shown" + path):
                loads[route] = loads.get(route, 0) + 1
            expected = {"hwnds": before["hwnds"], "windows": before["windows"] | {
                target: expected_tabs(site, paths, selected)}, "requests": loads}
        check = checks[label] = {"sender": sender, "hwnd": window.hwnd, "message": WM_COMMAND,
                                 "wparam": command, "lparam": history_id, "before": before, "expected": expected}
        started = time.monotonic()
        send(window, WM_COMMAND, command, history_id, post=True)

        def observed():
            after = snapshot()
            if after is None:
                return False
            check["after"] = after
            if restore:
                return after == expected
            require(after == before, f"{label}: a rejected command changed the tabs or loaded a page")
            return time.monotonic() - started >= UNCHANGED_SECONDS

        try:
            window.wait(f"{label} after WM_COMMAND {command} lparam={history_id}", observed, timeout)
        finally:
            check["elapsed_seconds"] = round(time.monotonic() - started, 3)

    normal.page(START, require_request=True)
    add(normal, OLDER)
    wait_tabs(normal, (START, OLDER), 1)
    send(normal, TAB_MENU_ACTION, 2, TAB_PIN)  # The first new tab has key 2.
    wait_tabs(normal, (OLDER, START), 0)
    add(normal, NEWER)
    wait_tabs(normal, (OLDER, START, NEWER), 2)
    normal.close_row(0)
    wait_tabs(normal, (START, NEWER), 1)
    normal.close_row(1)
    wait_tabs(normal, (START,), 0)

    windows["normal_peer"] = peer = normal.open_window(NEW_WINDOW, "normal_peer", opened)
    peer.blank()
    reopen("older_returns_to_its_normal_owner", "normal_peer", REOPEN_RECENT, OLDER_ID,
           restore=("normal", (OLDER, START), 0), timeout=3)
    reopen("used_id_does_not_reopen_another_tab", "normal_peer", REOPEN_RECENT, OLDER_ID)

    windows["private_owner"] = private = normal.open_window(PRIVATE_WINDOW, "private_owner", opened)
    private.blank(private=True)
    add(private, PRIVATE)
    wait_tabs(private, ("", PRIVATE), 1)
    private.close_row(1)
    private.blank(private=True, all_blank=False)
    windows["private_peer"] = fresh = normal.open_window(PRIVATE_WINDOW, "private_peer", opened)
    fresh.blank(private=True)

    # Keep both histories populated so a rejection cannot pass against an empty source stack.
    reopen("normal_rejects_private_id", "normal", REOPEN_RECENT, PRIVATE_ID)
    reopen("private_rejects_normal_id", "private_owner", REOPEN_RECENT, NEWER_ID)
    reopen("fresh_private_rejects_other_private_id", "private_peer", REOPEN_RECENT, PRIVATE_ID)
    reopen("private_restores_own_id", "private_owner", REOPEN_RECENT, PRIVATE_ID,
           restore=("private_owner", ("", PRIVATE), 1))
    # The newer tab was row 1 when it closed; row 0 is now pinned again.
    reopen("legacy_restores_remaining_newer", "normal", REOPEN_LAST,
           restore=("normal", (OLDER, NEWER, START), 1))

    for window in (fresh, private, peer):
        window.close_window()
    process.normal_close(normal.hwnd)


def run(exe, profile, report):
    with fixture() as site:
        site.RequestHandlerClass = LoopbackHandler
        seed_profile(profile, site.origin)
        args = [f"--flit-profile={profile}", f"--flit-url={site.origin}{START}"]
        try:
            with OwnedProcess(Win32(), exe, args, report["launch"]) as process:
                exercise(Window(process, site), report)
            saved = report["session"] = json.loads((profile / "session.json").read_text(encoding="utf-8"))
            entries = [{"url": site.origin + path, "title": LABELS[path], "pinned": path == OLDER}
                       for path in (OLDER, NEWER, START)]
            require(saved["entries"] == entries and saved["selected"] == 1 and not saved.get("additional_windows"),
                    "Saved session did not retain the restored tab order, pin, selection, and normal URLs")
            totals = {route: count for path, count in ((START, 1), (OLDER, 2), (NEWER, 2), (PRIVATE, 2))
                      for route in (path, "/shown" + path)}
            require(requests(site) == totals, f"Unexpected fixture loads: expected {totals}, got {requests(site)}")
        finally:
            report["fixture"] = site.report()


def sha256(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="workspace-staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="workspace subdirectory for a disposable profile")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")

    report = {"result": "fail", "launch": {}}
    try:
        exe, work = args.exe.resolve(strict=True), args.work.resolve()
        require(exe.name.casefold() == "flit.exe" and exe.is_file() and exe.is_relative_to(WORKSPACE),
                f"--exe must be a staged Flit.exe inside {WORKSPACE}")
        dll = exe.with_name("Flit.dll").resolve(strict=True)
        require(dll.is_file() and dll.parent == exe.parent, "Expected a staged Flit.dll beside Flit.exe")
        require(work != WORKSPACE and work.is_relative_to(WORKSPACE), f"--work must be below {WORKSPACE}")
        report.update(exe=str(exe), dll_sha256=sha256(dll), exe_sha256=sha256(exe))
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-recently-closed-", dir=work)
        try:
            root = Path(scratch.name).resolve(strict=True)
            require(root.parent == work, "Unexpected disposable profile parent")
            profile = root / "profile"
            report["profile"] = str(profile)
            run(exe, profile, report)
        finally:
            clean_scratch(scratch, report)
        require("scratch_cleanup_error" not in report, "Could not remove the disposable profile")
        require(sha256(dll) == report["dll_sha256"] and sha256(exe) == report["exe_sha256"],
                "Staged Flit binaries changed during the run")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
