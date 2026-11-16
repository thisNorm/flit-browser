"""Verify recently closed tabs and windows in an owned Flit process."""

import argparse
from hashlib import file_digest
import json
from pathlib import Path
import tempfile

from windows_session import loaded_panels, titles
from windows_smoke import NEW_TAB, OwnedProcess, TAB_TITLE, WORKSPACE, Win32, fixture, require
from windows_windows import Window

REOPEN = 136
PIN_TAB = 0x8005


def add(browser, path):
    browser.click(NEW_TAB)
    browser.navigate(path)


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-reopen-", dir=work)) / "profile"
    errors = report["mismatches"] = []
    report["launch"] = launch = {}
    with fixture() as site, OwnedProcess(
        Win32(), exe, [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"], launch,
    ) as process:
        main = Window(process, site)
        main.page("/start", require_request=True)
        add(main, "/tab")
        add(main, "/second")
        main.wait("three titled tabs", lambda: titles(main) == ("Flit smoke: start", TAB_TITLE, "Flit smoke: second"))
        main.close_row(1)
        main.wait("middle tab closed", lambda: main.tab_count() == 2)
        main.page("/tab", lambda: main.command(REOPEN))
        report["middle"] = list(titles(main))
        if main.selected() != 1 or report["middle"] != ["Flit smoke: start", TAB_TITLE, "Flit smoke: second"]:
            errors.append("Reopening a middle tab did not restore its position")

        # Its reopened key is four; pin and close it regardless of where it was restored.
        main.win.message(main.hwnd, PIN_TAB, 4, 1)
        main.wait("tab pinned first", lambda: titles(main)[0] == TAB_TITLE)
        main.close_row(0)
        main.wait("pinned tab closed", lambda: main.tab_count() == 2)
        main.page("/tab", lambda: main.command(REOPEN))
        report["pinned"] = list(titles(main))
        if main.selected() != 0 or report["pinned"] != [TAB_TITLE, "Flit smoke: start", "Flit smoke: second"]:
            errors.append("Reopening a pinned tab lost its position or pin")

        # Cross-window reopen targets the still-open original owner.
        main.close_row(main.selected())
        main.wait("pinned tab closed again", lambda: main.tab_count() == 2)
        peer = main.open_window(161, "secondary normal window", {})
        peer.command(REOPEN)
        try:
            main.wait("pinned tab returned to its owner",
                      lambda: main.tab_count() == 3 and main.selected() == 0 and titles(main)[0] == TAB_TITLE,
                      3)
        except RuntimeError:
            errors.append("Reopen from another window did not restore a tab in its original window")

        peer.navigate("/first")
        add(peer, "/edit")
        peer.close_window()
        prior = Window.handles(process)
        main.command(REOPEN)
        try:
            new_hwnd = main.wait("secondary window reopened", lambda: next(iter(Window.handles(process) - prior), None), 3)
        except RuntimeError:
            errors.append("Reopen did not restore a closed secondary window")
        else:
            restored = Window(process, site, new_hwnd)
            restored.wait("secondary window selected tab", lambda: restored.address() == site.origin + "/edit")
            report["window"] = {"rows": list(titles(restored)), "selected": restored.selected(),
                                "materialized": loaded_panels(restored)}
            if (restored.tab_count(), restored.selected(), loaded_panels(restored)) != (2, 1, 1) or report["window"]["rows"] != ["Flit smoke: first", "Flit smoke: edit"]:
                errors.append("Reopened window did not retain two tabs and load only the active one")
            restored.close_window()

        # Closing a private window must never add its pages to normal reopen history.
        private = main.open_window(162, "private window", {})
        private.navigate("/prime")
        private_tab_requests = site.count("/prime")
        private.close_window()
        main.command(REOPEN)
        if not errors:
            # The last normal window is still available; the private close
            # must not replace that item.
            main.wait("normal window still reopens after a private close",
                      lambda: len(Window.handles(process)) == 2, 3)
        for hwnd in Window.handles(process):
            reopened = Window(process, site, hwnd)
            if any("prime" in item for item in titles(reopened)) or reopened.address() == site.origin + "/prime":
                errors.append("A private page entered normal reopen history")
            if hwnd != main.hwnd:
                reopened.close_window()
        require(site.count("/prime") == private_tab_requests, "A private page was loaded during normal reopen")
        process.normal_close(main.hwnd)
        saved = json.loads((profile / "session.json").read_text(encoding="utf-8"))
        report["pin_persisted"] = bool(saved["entries"] and saved["entries"][0]["pinned"])
        if not report["pin_persisted"]:
            errors.append("The recovered pinned tab was not pinned in the saved session")
    return not errors


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    report = {"result": "fail"}
    try:
        work = args.work.resolve()
        require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
        exe = args.exe.resolve(strict=True)
        with exe.with_suffix(".dll").open("rb") as dll:
            report["dll_sha256"] = file_digest(dll, "sha256").hexdigest()
        if run(exe, work, report):
            report["result"] = "pass"
    except (RuntimeError, OSError) as error:
        report["error"] = str(error)
    print(json.dumps(report, indent=2))
    raise SystemExit(int(report["result"] != "pass"))
