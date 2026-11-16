"""Exercise native bookmark HTML import and export in a fresh, owned Flit profile."""

import argparse
from contextlib import contextmanager
import ctypes as C
from ctypes import wintypes as W
from html import escape
from html.parser import HTMLParser
import json
from pathlib import Path
import sys
import tempfile

sys.dont_write_bytecode = True
from windows_bookmarks import SAVE_BOOKMARK, TITLE, wait_manager
from windows_download_close import owned_windows, post
from windows_downloads import sha256
from windows_smoke import Browser, OwnedProcess, WORKSPACE, Win32, api, fixture, page_counts, require


IMPORT, EXPORT = 174, 175
FOLDER, NESTED = "가져온 즐겨찾기", "하위 폴더 — 東京"
FIRST, SECOND = "첫 링크 & Café", "두 번째 링크 — 日本語"
HEADER = "<!DOCTYPE NETSCAPE-Bookmark-file-1>"


def pick_file(browser, command, path, evidence):
    win, process = browser.win, browser.process
    user = C.WinDLL("user32", use_last_error=True)
    ancestor = api(user, "GetAncestor", W.HWND, W.HWND, W.UINT)
    control_id = api(user, "GetDlgCtrlID", C.c_int, W.HWND)
    window_text = api(user, "GetWindowTextW", C.c_int, W.HWND, W.LPWSTR, C.c_int)
    enum_children = api(user, "EnumChildWindows", W.BOOL, W.HWND, win.callback, W.LPARAM)
    edit_ids = (1148, 1152) if command == IMPORT else (1001, 1148, 1152)
    require(not owned_windows(process, "#32770"), "An owned dialog was already open")
    post(process, browser.hwnd, 0x0111, command)  # WM_COMMAND
    evidence.update(command=command, requested_path=str(path))

    def title(hwnd):
        text = C.create_unicode_buffer(256)
        window_text(hwnd, text, len(text))
        return text.value

    def edit_text(hwnd):
        require(win.pid(hwnd) == process.pid, "File picker edit is no longer owned")
        text = C.create_unicode_buffer(max(1024, len(str(path)) + 2))
        win.message(hwnd, 0x000D, len(text), C.addressof(text))  # WM_GETTEXT
        return text.value

    def ready():
        dialogs = owned_windows(process, "#32770")
        if not dialogs:
            return None
        require(len(dialogs) == 1, f"Expected one owned file picker, got {dialogs}")
        dialog = dialogs[0]
        owner = ancestor(dialog, 3)  # GA_ROOTOWNER
        require(owner == browser.hwnd, f"File picker root owner is {owner}, expected {browser.hwnd}")
        edits = {}

        @win.callback
        def visit(hwnd, _):
            if (win.pid(hwnd) == process.pid and win.visible(hwnd) and win.enabled(hwnd)
                    and win.class_name(hwnd) == "Edit"):
                edits.setdefault(control_id(hwnd), []).append(hwnd)
            return True

        enum_children(dialog, visit, 0)
        evidence.update(dialog=dialog, dialog_title=title(dialog), root_owner=owner, visible_edit_ids=sorted(edits))
        button = win.get_item(dialog, 1)  # IDOK, Open or Save.
        if not (button and win.pid(button) == process.pid and win.visible(button)
                and win.class_name(button) == "Button"):
            return None
        # The modern pickers nest the edit under different Shell controls.
        for cid in edit_ids:
            if len(edits.get(cid, ())) == 1:
                evidence["filename_control_id"] = cid
                return dialog, edits[cid][0], button

    dialog, entry, button = browser.wait(f"ready native file picker for command {command}", ready, 15)
    evidence.update(initial_filename=edit_text(entry), entry=entry, confirm_button=button)
    value = C.create_unicode_buffer(str(path))
    require(win.pid(entry) == process.pid, "File picker edit is no longer owned")
    if command == EXPORT:
        # WM_SETTEXT changes the displayed text without updating the Shell's save filename.
        win.message(entry, 0x00B1, 0, -1)  # EM_SETSEL: select all.
        win.message(entry, 0x00C2, 1, C.addressof(value))  # EM_REPLACESEL
    else:
        require(win.message(entry, 0x000C, 0, C.addressof(value)), "File picker refused WM_SETTEXT")
    readback = evidence["selected_path"] = edit_text(entry)
    require(readback == str(path), f"File picker path did not match: {readback!r}")
    browser.wait("file picker confirmation enabled", lambda: win.visible(button) and win.enabled(button))
    post(process, button, 0x00F5)  # BM_CLICK on the actual owned button.
    evidence["bm_click_posted"] = True
    try:
        browser.wait("native file picker closed after confirmation", lambda: not win.visible(dialog))
    except RuntimeError:
        try:
            evidence["remaining_dialogs"] = [dict(hwnd=hwnd, title=title(hwnd))
                                             for hwnd in owned_windows(process, "#32770")]
            if win.pid(entry) == process.pid:
                evidence["filename_after_click"] = edit_text(entry)
        except (OSError, RuntimeError) as error:
            evidence["picker_diagnostic_error"] = str(error)
        raise
    evidence["closed"] = True


def native_bookmarks(profile):
    def walk(node, folders):
        if node["type"] == "url":
            yield dict(url=node["url"], title=node["name"], folders=folders, guid=node["guid"])
        else:
            for child in node.get("children", ()):
                yield from walk(child, (*folders, node["name"]))

    data = json.loads((profile / "Default" / "Bookmarks").read_text(encoding="utf-8"))
    return [row for root, node in data["roots"].items() for child in node["children"]
            for row in walk(child, (root,))]


def one_bookmark(rows, url, title, folders=()):
    matches = [row for row in rows if row["url"] == url]
    require(len(matches) == 1, f"Expected one native bookmark for {url}; got {len(matches)}")
    row = matches[0]
    require(row["title"] == title, f"Unexpected native title for {url}: {row['title']!r}")
    if folders:
        require(row["folders"][-len(folders):] == folders, f"Wrong folder path for {url}: {row['folders']!r}")
    return row


def write_fixture(path, first, second):
    path.write_text(f'''{HEADER}
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3>{escape(FOLDER)}</H3>
    <DL><p>
        <DT><A HREF="{escape(first)}">{escape(FIRST)}</A>
        <DT><H3>{escape(NESTED)}</H3>
        <DL><p>
            <DT><A HREF="{escape(second)}">{escape(SECOND)}</A>
        </DL><p>
    </DL><p>
</DL><p>
''', encoding="utf-8")


class ExportHTML(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links, self.folders, self.pending = [], [], None

    def handle_starttag(self, tag, attrs):
        if tag in ("a", "h3"):
            self.pending = tag, dict(attrs).get("href"), []

    def handle_data(self, data):
        if self.pending:
            self.pending[2].append(data)

    def handle_endtag(self, tag):
        if self.pending and tag == self.pending[0]:
            _, url, chunks = self.pending
            title = "".join(chunks)
            if tag == "a":
                self.links.append((url, title))
            else:
                self.folders.append(title)
            self.pending = None


def exported(path, expected, evidence):
    try:
        data = path.read_bytes()
        text = data.decode("utf-8-sig")
    except (FileNotFoundError, PermissionError, UnicodeDecodeError):
        return False  # The native asynchronous writer may still be creating the file.
    parsed = ExportHTML()
    parsed.feed(text)
    parsed.close()
    evidence.update(bytes=len(data), links=parsed.links, folders=parsed.folders)
    return (text.startswith(HEADER) and all(parsed.links.count(link) == 1 for link in expected)
            and all(folder in parsed.folders for folder in (FOLDER, NESTED)))


def run(exe, scratch, evidence):
    profile = scratch / "profile"
    profile.mkdir()
    source, output = scratch / "import 북마크.html", scratch / "export 북마크.html"
    evidence.update(profile=str(profile), source=str(source), output=str(output), launches={})
    with fixture() as site:
        start, first, second = (site.origin + route for route in ("/start", "/first", "/second"))
        write_fixture(source, first, second)
        win = Win32()

        @contextmanager
        def launch(name, target=profile):
            args = [f"--flit-profile={target}", f"--flit-url={start}"]
            baseline = page_counts(site, "/start")
            with OwnedProcess(win, exe, args, evidence["launches"].setdefault(name, {})) as process:
                browser = Browser(process, site)
                browser.page("/start", require_request=True, baseline=baseline)
                browser.wait("fixture page title", lambda: browser.has_page_title(TITLE))
                yield browser
                process.normal_close(browser.hwnd)

        def verify_bookmarks(target):
            rows = native_bookmarks(target)
            existing = one_bookmark(rows, start, TITLE)
            top = one_bookmark(rows, first, FIRST, (FOLDER,))
            nested = one_bookmark(rows, second, SECOND, (FOLDER, NESTED))
            require(nested["folders"][:-1] == top["folders"], "Imported folder paths do not share the same parent")
            return [existing, top, nested]

        def import_file(name, target, path, expected_urls):
            def write_seen():
                try:
                    urls = {row["url"] for row in native_bookmarks(target)}
                    return expected_urls <= urls
                except (FileNotFoundError, PermissionError, json.JSONDecodeError):
                    return False

            imported = evidence[name] = {}
            with launch(name, target) as browser:
                pick_file(browser, IMPORT, path, imported)
                try:
                    browser.wait("import worker and Chromium bookmark write", write_seen)
                    imported["write_seen_before_close"] = True
                except RuntimeError:
                    browser.process.alive()
                    imported["write_seen_before_close"] = False  # Shutdown may flush the write.
            return verify_bookmarks(target)

        with launch("seed") as browser:
            post(browser.process, browser.hwnd, 0x0111, SAVE_BOOKMARK)
            wait_manager(browser, "Bookmarks — Page saved")
        before = evidence["original"] = one_bookmark(native_bookmarks(profile), start, TITLE)
        require(before["guid"], "The original bookmark has no Chromium GUID")

        saved = evidence["after_import"] = import_file("import", profile, source, {first, second})
        require(saved[0] == before, "The original bookmark changed during import")

        with launch("relaunch_export") as browser:
            require(not output.exists(), "Export target must be a fresh scratch path")
            export = evidence["export"] = {}
            pick_file(browser, EXPORT, output, export)
            expected = [(row["url"], row["title"]) for row in saved]
            browser.wait("exported HTML with all three bookmarks", lambda: exported(output, expected, export), 15)
        evidence["after_relaunch"] = verify_bookmarks(profile)
        require(evidence["after_relaunch"] == saved, "Native bookmark identities changed after relaunch")

        roundtrip = scratch / "roundtrip-profile"
        roundtrip.mkdir()
        evidence["roundtrip_profile"] = str(roundtrip)
        evidence["after_reimport"] = import_file("reimport", roundtrip, output, {start, first, second})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32":
        parser.error("requires Windows")
    if (not exe.is_relative_to(WORKSPACE) or exe.name.casefold() != "flit.exe"
            or not exe.is_file() or not dll.is_file()):
        parser.error("expected a staged Flit.exe next to Flit.dll inside the workspace")
    if not work.is_relative_to(WORKSPACE):
        parser.error("expected a scratch directory inside the workspace")
    work.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="flit-bookmark-transfer-", dir=work))
    evidence = {"exe": str(exe), "result": "fail"}
    try:
        evidence["dll_sha256"] = sha256(dll)
        run(exe, scratch, evidence)
        require(sha256(dll) == evidence["dll_sha256"], "Staged Flit.dll changed during the test")
        evidence["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
        evidence["scratch_entries"] = sorted(path.name for path in scratch.iterdir())
    report = json.dumps(evidence, indent=2, ensure_ascii=True)
    out = scratch / "evidence.json"
    out.write_text(report + "\n", encoding="utf-8")
    print(report)
    print(f"Evidence: {out}")
    return int(evidence["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
