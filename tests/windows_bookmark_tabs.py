"""Verify Bookmark all tabs through Win32 and Chromium's native bookmark file."""

import argparse
import json
import os
from pathlib import Path
import re
import sys
from tempfile import TemporaryDirectory
from unittest.mock import patch
from urllib.parse import urlsplit

sys.dont_write_bytecode = True
import windows_direct_bookmarks as direct
from windows_bookmark_transfer import native_bookmarks
from windows_bookmarks import BOOKMARKS, TITLE, wait_manager
from windows_download_close import owned_windows
from windows_multi_session import clean_scratch
from windows_session import loaded_panels
from windows_smoke import Handler, OwnedProcess, WORKSPACE, Win32, fixture, page_counts, require
from windows_windows import PRIVATE_WINDOW, Window, seed_profile


BOOKMARK_ALL_TABS = 189
DIALOG_TITLE = "Bookmark all tabs"
PARKED_REQUEST = "/start?view=full&next=%2Fdeep"
PARKED_TITLE = "보관한 탭 — 서울"


class BookmarkHandler(Handler):
    def do_GET(self):
        target = urlsplit(self.path)
        if target.netloc and f"{target.scheme}://{target.netloc}" != self.server.origin:
            self.send_error(403)
            return
        if target.query:
            with self.server.lock:
                self.server.hits[f"{target.path}?{target.query}"] += 1
        self.path = target.path
        super().do_GET()


def confirm(browser, answer, report):
    wait = browser.wait

    def wait_for_confirmation(label, check, timeout=12):
        if label != DIALOG_TITLE:
            return wait(label, check, timeout)
        dialogs = wait(label, check, 3)
        if len(dialogs) == 1:
            dialog, = dialogs
            require(len(list(browser.win.visible_children(dialog, "Button"))) == 2,
                    "Expected only Yes and No buttons")
            default = browser.win.message(dialog, 0x0400)  # DM_GETDEFID
            report["default_button"] = default & 0xFFFF
            require(default == ((0x534B << 16) | direct.IDNO), "Confirmation did not default to No")
        return dialogs

    with patch.object(direct, "DIALOG_TITLE", DIALOG_TITLE), patch.object(browser, "wait", wait_for_confirmation):
        direct.confirm(browser, BOOKMARK_ALL_TABS, answer, report)


def bookmark_roots(profile):
    path = profile / "Default" / "Bookmarks"
    return json.loads(path.read_text(encoding="utf-8"))["roots"] if path.is_file() else {}


def verify_saved(profile, expected, report):
    children = bookmark_roots(profile).get("other", {}).get("children", [])
    require(len(children) == 1 and children[0]["type"] == "folder",
            "Expected one new native folder directly under Other bookmarks")
    folder = report["folder"] = children[0]["name"]
    require(re.fullmatch(r"Saved tabs - \d{4}-\d{2}-\d{2} \d{2}:\d{2}", folder),
            f"Unexpected saved tabs folder name: {folder!r}")
    require(len(children[0]["children"]) == len(expected), "Wrong number of direct bookmarks in the folder")
    rows = report["bookmarks"] = native_bookmarks(profile)
    require([(row["url"], row["title"], row["folders"]) for row in rows] ==
            [(url, title, ("other", folder)) for url, title in expected],
            "Native bookmarks differ in count, order, URL, title, or placement")
    require(all(row["guid"] for row in rows), "A saved bookmark lacks a native GUID")


def exercise(exe, root, site, report):
    profile = root / "profile"
    seed_profile(profile, site.origin)
    start = site.origin + "/start"
    parked = site.origin + PARKED_REQUEST + "#details"
    expected = [(parked, PARKED_TITLE), (start, TITLE)]
    entries = [dict(url=url, title=title) for url, title in
               [*expected, (start, "Duplicate title"), ("about:blank", "")]]
    (profile / "session.json").write_text(json.dumps({"entries": entries, "selected": 1}), encoding="utf-8")
    win = Win32()
    for stage, answer in (("decline", direct.IDNO), ("accept", direct.IDYES)):
        step = report[stage] = {"launch": {}, "confirmation": {}}
        baseline = page_counts(site, "/start")
        with OwnedProcess(win, exe, [f"--flit-profile={profile}"], step["launch"]) as process:
            browser = Window(process, site)
            browser.page("/start", require_request=True, baseline=baseline)
            browser.wait("active fixture title", lambda: browser.has_page_title(TITLE))
            panel = browser.wait("active fixture panel", browser.page_panel)
            require(browser.tab_count() == len(entries) and browser.selected() == 1,
                    "The synthetic session did not restore all four tabs with the expected selection")
            require(loaded_panels(browser) == 1 and not site.count(PARKED_REQUEST),
                    "A background tab loaded before the bookmark command")
            confirm(browser, answer, step["confirmation"])
            if answer == direct.IDNO:
                require(browser.address() == start and browser.selected() == 1
                        and browser.tab_count() == len(entries) and browser.page_panel() == panel,
                        "Declining changed the active page or tab list")
                private = browser.open_window(PRIVATE_WINDOW, "private", step)
                private.blank(private=True)
                private.navigate("/first")
                private.command(BOOKMARK_ALL_TABS)
                require(not owned_windows(process, "#32770") and private.address() == site.origin + "/first"
                        and private.tab_count() == 1, "Bookmark all tabs acted in the private window")
                private.close_window()
            else:
                step["manager"] = wait_manager(browser, f"Bookmarks — Saved {len(expected)} tabs")
            process.normal_close(browser.hwnd)

        step["parked_requests"] = site.count(PARKED_REQUEST)
        require(not step["parked_requests"], "Bookmark all tabs requested the parked URL")
        if answer == direct.IDNO:
            require(not any(node["children"] for node in bookmark_roots(profile).values()),
                    "Declining or invoking the command in private wrote bookmarks")
        else:
            verify_saved(profile, expected, step)
            reopened = step["after_restart"] = {"launch": {}}
            with OwnedProcess(win, exe, [f"--flit-profile={profile}"], reopened["launch"]) as process:
                browser = Window(process, site)
                browser.command(BOOKMARKS)
                reopened["manager"] = wait_manager(browser, "Bookmarks", "북마크")
                process.normal_close(browser.hwnd)
            verify_saved(profile, expected, reopened)
            require(reopened["bookmarks"] == step["bookmarks"], "Restart changed the saved bookmark identities")
            require(not site.count(PARKED_REQUEST), "Restart requested the parked URL")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="workspace parent for a disposable profile")
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    if (sys.platform != "win32" or not exe.is_relative_to(WORKSPACE)
            or exe.name.casefold() != "flit.exe" or not exe.is_file() or not exe.with_name("Flit.dll").is_file()):
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll inside the workspace")
    if not work.is_relative_to(WORKSPACE):
        parser.error(f"--work must be inside {WORKSPACE}")

    report = {"result": "fail", "exe": str(exe)}
    try:
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-bookmark-tabs-", dir=work)
        root = Path(scratch.name).resolve(strict=True)
        require(root.parent == work, "Unexpected disposable profile parent")
        try:
            with patch.dict(os.environ, LOCALAPPDATA=str(root / "localappdata")), fixture() as site:
                site.RequestHandlerClass = BookmarkHandler
                try:
                    exercise(exe, root, site, report)
                finally:
                    report["fixture"] = site.report()
        finally:
            clean_scratch(scratch, report)
            report["scratch_removed"] = not root.exists()
        require(report["scratch_removed"] and "scratch_cleanup_error" not in report,
                "Disposable scratch was not removed")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2, ensure_ascii=True))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
