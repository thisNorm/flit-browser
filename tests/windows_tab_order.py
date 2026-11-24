"""Verify Flit's tab reorder and duplication in an owned Win32 process and loopback profile."""

import argparse
import ctypes as C
from ctypes import wintypes as W
from hashlib import file_digest
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

sys.dont_write_bytecode = True
from windows_multi_session import clean_scratch
from windows_session import loaded_panels, send, titles
from windows_smoke import (
    Browser, NEW_TAB, OwnedProcess, TAB_FILTER, TAB_TITLE, TABS, WORKSPACE,
    Win32, fixture, page_counts, require,
)


WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP, MK_LBUTTON = 0x0200, 0x0201, 0x0202, 0x0001
LB_GETITEMRECT, LB_ITEMFROMPOINT = 0x0198, 0x01A9
TAB_MENU_ACTION = 0x8005  # WM_APP + 5
TAB_PIN, TAB_DUPLICATE, TAB_MOVE_UP, TAB_MOVE_DOWN = 1, 2, 7, 8

START, PINNED, ACTIVE = "/start", "/tab", "/second"
LABELS = {START: "Flit smoke: start", PINNED: TAB_TITLE, ACTIVE: "Flit smoke: second"}
BEFORE, AFTER = (PINNED, START, ACTIVE), (PINNED, ACTIVE, START)
DUPLICATED = (PINNED, PINNED, ACTIVE, ACTIVE, START)
DUPLICATE_SELECTION = 3


def check_ui(browser, order, panel, evidence, *, selected=None):
    if selected is None:
        selected = order.index(ACTIVE)
    evidence.update(rows=titles(browser), selected=browser.selected(),
                    address=browser.address(), active_panel=browser.page_panel())
    expected = tuple(LABELS[path] for path in order)
    require(evidence["rows"] == expected, f"Expected ListBox order {expected}, got {evidence['rows']}")
    require(evidence["selected"] == selected
            and evidence["address"] == browser.site.origin + order[selected]
            and evidence["active_panel"] == panel, f"Active tab changed: {evidence}")


def row_point(browser, index, quarter, dpi):
    row = W.RECT()
    require(browser.message(TABS, LB_GETITEMRECT, index, C.addressof(row)) != -1,
            f"Could not read tab row {index}")
    _, _, width, height = browser.win.rect(browser.controls[TABS], client=True)
    x = (row.left + row.right) // 2
    y = row.top + (row.bottom - row.top) * quarter // 4
    require(row.left <= x < row.right - 28 * dpi // 96 and row.top <= y < row.bottom
            and 0 <= x < min(width, 32768) and 0 <= y < min(height, 32768),
            f"Point ({x}, {y}) is outside the visible body of tab row {index}")
    require(browser.message(TABS, LB_ITEMFROMPOINT, 0, x | (y << 16)) == index,
            f"Point ({x}, {y}) did not hit tab row {index}")
    return x, y


def exercise(browser, checks):
    browser.page(START, require_request=True)
    source_panels = {}
    for index, path in enumerate((PINNED, ACTIVE), start=1):
        browser.click(NEW_TAB)
        browser.wait(f"new tab selected at row {index}", lambda: browser.selected() == index)
        browser.navigate(path)
        source_panels[path] = browser.wait(f"source tab panel for {path}", browser.page_panel)
    browser.wait("three titled tabs", lambda: titles(browser) == (LABELS[START], LABELS[PINNED], LABELS[ACTIVE]))
    browser.wait("three loaded tab panels", lambda: loaded_panels(browser) == 3)
    panel = source_panels[ACTIVE]
    send(browser, TAB_MENU_ACTION, 2, TAB_PIN)  # Second created tab; keys do not follow row indices.
    check_ui(browser, BEFORE, panel, checks.setdefault("initial", {"loaded_panels": loaded_panels(browser)}))

    def drag(label, before, after, source, target, *, lower):
        trace = checks[label] = {}
        win, hwnd = browser.win, browser.controls[TABS]
        require(win.pid(hwnd) == browser.process.pid and win.visible(hwnd), "Tab ListBox is not visible and owned")
        with win.window_coordinates(hwnd):
            dpi = max(win.check(win.dpi(hwnd)), 96)
            x0, y0 = row_point(browser, before.index(source), 2, dpi)
            x1, y1 = row_point(browser, before.index(target), 3 if lower else 1, dpi)
            require(abs(y1 - y0) >= 16 * dpi // 96, "Drag must travel well beyond the threshold")
            trace.update(dpi=dpi, from_client=(x0, y0), to_client=(x1, y1))
            start, end = x0 | (y0 << 16), x1 | (y1 << 16)
            # Synchronous sends let us observe the active tab before releasing the button.
            browser.message(TABS, WM_LBUTTONDOWN, MK_LBUTTON, start)
            try:
                check_ui(browser, before, panel, trace.setdefault("down", {}))
                browser.message(TABS, WM_MOUSEMOVE, MK_LBUTTON, end)
                check_ui(browser, before, panel, trace.setdefault("held", {}))
            finally:
                browser.message(TABS, WM_LBUTTONUP, 0, end)
        check_ui(browser, after, panel, trace.setdefault("released", {}))

    drag("drag_down", BEFORE, AFTER, START, ACTIVE, lower=True)
    drag("drag_up", AFTER, BEFORE, START, ACTIVE, lower=False)
    drag("unpinned_cannot_cross", BEFORE, BEFORE, START, PINNED, lower=False)
    drag("pinned_cannot_cross", BEFORE, BEFORE, PINNED, START, lower=True)

    # Dispatch the same command as the menu using /start's original tab key.
    for label, action, order in (("menu_down", TAB_MOVE_DOWN, AFTER), ("menu_up", TAB_MOVE_UP, BEFORE)):
        send(browser, TAB_MENU_ACTION, 1, action)
        check_ui(browser, order, panel, checks.setdefault(label, {"action": action}))

    # Hide the pinned row so visible row indices differ from the underlying tab order.
    browser.set_text(TAB_FILTER, ": s")
    check_ui(browser, BEFORE[1:], panel, checks.setdefault("filtered", {}))
    drag("filtered_drag", BEFORE[1:], AFTER[1:], START, ACTIVE, lower=True)
    browser.set_text(TAB_FILTER, "")
    check_ui(browser, AFTER, panel, checks.setdefault("final", {}))

    duplicates = (
        ("duplicate_pinned", 2, PINNED, (PINNED, PINNED, ACTIVE, START), 1),
        ("duplicate_unpinned", 3, ACTIVE, DUPLICATED, DUPLICATE_SELECTION),
    )
    for label, key, path, order, selected in duplicates:
        trace = checks[label] = {"source_key": key, "source_panel": source_panels[path]}
        browser.page(path, lambda: send(browser, TAB_MENU_ACTION, key, TAB_DUPLICATE),
                     require_request=True)
        duplicate_panel = browser.wait("duplicate tab panel", browser.page_panel)
        check_ui(browser, order, duplicate_panel, trace, selected=selected)
        require(duplicate_panel != source_panels[path],
                f"Duplication selected the original tab panel: {trace}")


def run(exe, profile, evidence):
    win = Win32()
    checks, launches = evidence["checks"], evidence["launches"]
    args = [f"--flit-profile={profile}"]
    with fixture() as site:
        try:
            with OwnedProcess(win, exe, [*args, f"--flit-url={site.origin}{START}"], launches.setdefault("reorder", {})) as process:
                browser = Browser(process, site)
                exercise(browser, checks)
                process.normal_close(browser.hwnd)

            saved = evidence["session"] = json.loads((profile / "session.json").read_text(encoding="utf-8"))
            entries = [{"url": site.origin + path, "title": LABELS[path], "pinned": path == PINNED} for path in DUPLICATED]
            require(saved["entries"] == entries and saved["selected"] == DUPLICATE_SELECTION and saved["restore"] is True,
                    "Session JSON did not retain the final tab order, titles, pins and active selection")

            baseline = page_counts(site, ACTIVE)
            with OwnedProcess(win, exe, args, launches.setdefault("restart", {})) as process:
                browser = Browser(process, site)
                browser.page(ACTIVE, require_request=True, baseline=baseline)
                panel = browser.wait("restored active tab panel", browser.page_panel)
                check_ui(browser, DUPLICATED, panel, checks.setdefault("restored", {}), selected=DUPLICATE_SELECTION)
                process.normal_close(browser.hwnd)
        finally:
            evidence["fixture"] = site.report()


def dll_sha256(exe):
    with exe.with_name("Flit.dll").open("rb") as dll:
        return file_digest(dll, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, default=WORKSPACE / "work" / "windows-tab-order",
                        help="workspace scratch directory for the disposable profile")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Browser runtime verification requires Windows")
    evidence = {"result": "fail", "checks": {}, "launches": {}}
    try:
        exe, work = args.exe.resolve(), args.work.resolve()
        evidence["exe"] = str(exe)
        require(exe.name.casefold() == "flit.exe" and exe.is_file(), f"Missing staged Flit.exe: {exe}")
        require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
        evidence["dll_sha256"] = dll_sha256(exe)
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-tab-order-", dir=work)
        try:
            profile = Path(scratch.name) / "profile"
            evidence["profile"] = str(profile)
            run(exe, profile, evidence)
        finally:
            clean_scratch(scratch, evidence)
        require("scratch_cleanup_error" not in evidence, "Could not remove the disposable profile")
        require(dll_sha256(exe) == evidence["dll_sha256"], "Staged Flit.dll changed during the run")
        evidence["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(evidence, indent=2))
    return int(evidence["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
