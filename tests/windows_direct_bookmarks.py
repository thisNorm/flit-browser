"""Exercise direct Chrome and Edge bookmark import on an interactive Windows desktop."""

import argparse
from contextlib import contextmanager
import ctypes as C
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
from unittest.mock import patch

sys.dont_write_bytecode = True
from windows_bookmark_transfer import native_bookmarks, one_bookmark
from windows_bookmarks import BOOKMARKS, MANAGER, SAVE_BOOKMARK, SELECT_FIRST_TAB, TITLE, wait_manager
from windows_download_close import owned_windows, post
from windows_multi_session import clean_scratch
from windows_smoke import Browser, OwnedProcess, WORKSPACE, Win32, api, fixture, page_counts, require


SOURCES = {"Chrome": (185, "Google/Chrome"), "Edge": (186, "Microsoft/Edge")}
DIALOG_TITLE = "Import bookmarks to Flit"
WM_COMMAND, IDYES, IDNO = 0x0111, 6, 7
BAR, OTHER, MOBILE = "Bookmarks bar", "Other bookmarks", "Mobile bookmarks"
FOLDER, NESTED = "즐겨찾기 & Café", "하위 폴더 — 東京"
LITERAL_TITLE = "Quotes \"'\\ & </script><b>서울</b> ${literal}"


def write_source(bookmarks, source, *, merge=False):
    def link(name, url):
        return {"type": "url", "name": name, "url": url}

    def folder(name, *children):
        return {"type": "folder", "name": name, "children": children}

    host = f"{source.lower()}.flit-bookmarks.test"
    top = link(LITERAL_TITLE, f"https://{host}:8443/guides/a%20b?next=%2Fdeep&n=2#overview")
    nested = link("Nested HTTP link", f"http://{host}/nested/page?word=two%20words&n=1#details")
    other = link("Another HTTPS link", f"https://{host}:8443/guides/other?mode=full#section")
    new = link("New link on reimport", f"https://{host}:8443/guides/new/{bookmarks.name}?next=%2Fdeep#update")
    rejected = [
        link("Username and password", f"https://fixture-user:fixture-pass@{host}/discard"),
        link("Username only", f"http://fixture-user@{host}/discard"),
        link("JavaScript", "javascript:void(0)"),
        link("Data URL", "data:text/plain,synthetic"),
        link("File URL", "file:///C:/flit-synthetic-bookmark.txt"),
    ]
    roots = {
        "bookmark_bar": folder(BAR, folder(FOLDER, top, folder(NESTED, nested, *([new] if merge else [])),
                                            *rejected)),
        "other": folder(OTHER, other),
        "synced": folder(MOBILE),
    }
    payload = json.dumps({"version": 1, "roots": roots}, ensure_ascii=False).encode("utf-8")
    bookmarks.parent.mkdir(parents=True, exist_ok=True)
    bookmarks.write_bytes(payload)
    expected = [(top, (BAR, FOLDER)), (nested, (BAR, FOLDER, NESTED)), (other, (OTHER,))]
    if merge:
        expected.append((new, (BAR, FOLDER, NESTED)))
    return payload, expected


def confirm(browser, command, answer, report):
    win, process = browser.win, browser.process
    user = C.WinDLL("user32", use_last_error=True)
    get_owner = api(user, "GetWindow", W.HWND, W.HWND, W.UINT)
    get_text = api(user, "GetWindowTextW", C.c_int, W.HWND, W.LPWSTR, C.c_int)
    require(not owned_windows(process, "#32770"), "An owned dialog was already open")
    report.update(command=command, answer="Yes" if answer == IDYES else "No")
    post(process, browser.hwnd, WM_COMMAND, command)
    dialogs = browser.wait(DIALOG_TITLE, lambda: owned_windows(process, "#32770"))
    require(len(dialogs) == 1, f"Expected one owned confirmation, got {dialogs}")
    dialog, = dialogs
    title = C.create_unicode_buffer(256)
    get_text(dialog, title, len(title))
    owner = get_owner(dialog, 4)  # GW_OWNER
    report.update(dialog_title=title.value, dialog=dialog, owner=owner, browser=browser.hwnd)
    require(title.value == DIALOG_TITLE and owner == browser.hwnd,
            "Unexpected confirmation title or owner")
    require(not win.enabled(browser.hwnd), "Confirmation is not modal to the Flit window")
    buttons = {cid: win.get_item(dialog, cid) for cid in (IDYES, IDNO)}
    require(all(hwnd and win.pid(hwnd) == process.pid and win.visible(hwnd) and win.enabled(hwnd)
                and win.class_name(hwnd) == "Button" for hwnd in buttons.values()),
            "Confirmation does not have owned, enabled Yes and No buttons")
    post(process, dialog, WM_COMMAND, answer, buttons[answer])  # BN_CLICKED; no foreground input.
    browser.wait("confirmation dismissed", lambda: not win.visible(dialog) and win.enabled(browser.hwnd))
    report["dismissed"] = True


def verify_store(profile, original, wrapper, expected, report):
    rows = report["bookmarks"] = native_bookmarks(profile)
    require(one_bookmark(rows, original["url"], TITLE) == original, "Original Flit bookmark changed")
    require(len(rows) == 1 + len(expected), "Unexpected, unsafe, or duplicate native bookmark")
    imported = [one_bookmark(rows, node["url"], node["name"], ("other", wrapper, *path))
                for node, path in expected]
    require(all(row["guid"] for row in imported), "Imported bookmarks are missing native GUIDs")

    def folders(node, path=()):
        for child in node.get("children", ()):
            if child["type"] == "folder":
                child_path = (*path, child["name"])
                yield child_path
                yield from folders(child, child_path)

    store = json.loads((profile / "Default" / "Bookmarks").read_text(encoding="utf-8"))
    actual = report["other_folder_paths"] = sorted(folders(store["roots"]["other"]))
    expected_paths = [(), (BAR,), (BAR, FOLDER), (BAR, FOLDER, NESTED), (OTHER,), (MOBILE,)]
    require(actual == sorted((wrapper, *path) for path in expected_paths),
            "Other bookmarks contains a missing, misplaced, or duplicate folder")
    return imported


def exercise(exe, root, site, source, report):
    command, vendor = SOURCES[source]
    local, profile = root / "localappdata", root / "flit-profile"
    bookmarks = (local / vendor / "User Data" / "Default"
                 / ("AccountBookmarks" if source == "Chrome" else "Bookmarks"))
    start = site.origin + "/start"
    args = [f"--flit-profile={profile}", f"--flit-url={start}"]
    wrapper = f"From {source} - Default"
    win = Win32()
    report.update(localappdata=str(local), source_bookmarks=str(bookmarks), launches={})

    @contextmanager
    def launch(stage):
        report["stage"] = stage
        baseline = page_counts(site, "/start")
        # OwnedProcess inherits this process-only override; no real browser profile is used.
        with patch.dict(os.environ, LOCALAPPDATA=str(local)):
            with OwnedProcess(win, exe, args, report["launches"].setdefault(stage, {})) as process:
                browser = Browser(process, site)
                browser.page("/start", require_request=True, baseline=baseline)
                browser.wait("fixture page title", lambda: browser.has_page_title(TITLE))
                yield browser
                process.normal_close(browser.hwnd)

    payload, expected = write_source(bookmarks, source)
    source_payloads = {bookmarks: payload}
    with launch("seed_and_decline") as browser:
        post(browser.process, browser.hwnd, WM_COMMAND, SAVE_BOOKMARK)
        wait_manager(browser, "Bookmarks — Page saved")
        post(browser.process, browser.hwnd, WM_COMMAND, SELECT_FIRST_TAB)
        browser.wait("return to the fixture page", lambda: browser.address() == start)
        tab_count = browser.tab_count()
        confirm(browser, command, IDNO, report.setdefault("decline", {}))
        require(browser.address() == start and browser.tab_count() == tab_count,
                "Declining import changed the active page or tabs")
    rows = native_bookmarks(profile)
    original = report["original"] = one_bookmark(rows, start, TITLE)
    require(len(rows) == 1 and original["guid"], "Declining import did not leave just the original bookmark")

    def accept(stage, current, added, reuse=False):
        step = report[stage] = {}
        with launch(stage) as browser:
            if reuse:
                post(browser.process, browser.hwnd, WM_COMMAND, BOOKMARKS)
                wait_manager(browser, "Bookmarks", "북마크")
            tab_count = browser.tab_count()
            confirm(browser, command, IDYES, step)
            # This completion title is emitted by the manager after the last native API call.
            step["manager"] = wait_manager(browser, f"Bookmarks — Imported {added} bookmarks")
            require(step["manager"]["url"] == MANAGER, "Import did not use the native bookmark manager")
            if reuse:
                require(browser.tab_count() == tab_count, "Reimport opened another bookmark manager tab")
        step["source_unchanged"] = all(path.read_bytes() == data for path, data in source_payloads.items())
        require(step["source_unchanged"], "A synthetic bookmark source was changed by Flit")
        return verify_store(profile, original, wrapper, current, step)

    initial = accept("import", expected, added=len(expected))
    source_payloads[bookmarks], expected = write_source(bookmarks, source, merge=True)
    if source == "Chrome":
        # Each file has a new link in the same folder as the shared links.
        legacy = bookmarks.with_name("Bookmarks")
        source_payloads[legacy], legacy_expected = write_source(legacy, source, merge=True)
        report["legacy_bookmarks"] = str(legacy)
        expected.append(legacy_expected[-1])
    merged = accept("merge", expected, added=len(expected) - len(initial), reuse=True)
    require(merged[:len(initial)] == initial, "Reimport replaced an existing imported bookmark")
    report["stage"] = "complete"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="workspace parent for disposable profiles")
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    if (sys.platform != "win32" or exe.name.casefold() != "flit.exe"
            or not exe.is_file() or not exe.with_name("Flit.dll").is_file()):
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")
    if not work.is_relative_to(WORKSPACE):
        parser.error(f"--work must be inside {WORKSPACE}")

    report = {"result": "fail", "exe": str(exe), "sources": {}}
    try:
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-direct-bookmarks-", dir=work)
        root = Path(scratch.name).resolve(strict=True)
        require(root.parent == work, "Unexpected disposable profile parent")
        try:
            with fixture() as site:
                for source in SOURCES:
                    exercise(exe, root / source.lower(), site, source, report["sources"].setdefault(source, {}))
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
