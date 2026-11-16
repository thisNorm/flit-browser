"""Exercise Prepare sites against isolated Chromium bookmark files in an owned Flit window."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import sys
import tempfile
from threading import Thread
import time
from unittest.mock import patch
from urllib.parse import urlsplit

sys.dont_write_bytecode = True
from windows_download_close import owned_windows, post
from windows_multi_session import clean_scratch
from windows_search import Handler as RejectingHandler, Proxy
from windows_smoke import GO, NEW_TAB, TAB_FILTER, TABS, WORKSPACE, OwnedProcess, Win32, api, require
from windows_windows import Window, seed_profile


SOURCES = {"Chrome": (182, "Google/Chrome"), "Edge": (183, "Microsoft/Edge")}
DIALOG_TITLE = "Prepare sites in Flit"
WM_COMMAND, IDYES, IDNO = 0x0111, 6, 7
SITE_LIMIT, QUIET_SECONDS = 12, .6
FIXTURE_SUFFIX = ".flit-prepare.test"


def fixture_host(host):
    return host in {"127.0.0.1", "127.0.0.2", "localhost"} or host.endswith(FIXTURE_SUFFIX)


def fixture_title(route):
    return f"Flit prepare sites fixture: {route}"


class Handler(RejectingHandler):
    def do_CONNECT(self):
        if fixture_host(self.path.partition(":")[0].lower()):
            with self.server.lock:
                self.server.connects["fixture"] += 1
        self.respond(403)

    def do_GET(self):
        site = self.server
        route = site.routes.get(self.path)
        host = urlsplit(self.path).hostname or self.headers.get("Host", "").partition(":")[0].lower()
        if fixture_host(host):
            with site.lock:
                site.pages[route or "rejected"] += 1
        if route is None:
            self.respond(403)
            return
        self.respond(200, (f'<!doctype html><title>{fixture_title(route)}</title>'
                           '<link rel="icon" href="data:,">').encode("ascii"))


def seed_bookmarks(path, source, earlier, start, prepared):
    host = f"portal.{source.lower()}{FIXTURE_SUFFIX}"
    allowed = {prepared, f"https://{host}/", f"http://{host}/", f"https://{host}:8443/"}
    inputs = [
        f"https://{host.upper()}:443/deep?omit=1#fragment",
        f"https://{host}/another-bookmark",
        f"https://fixture@userinfo.{source.lower()}{FIXTURE_SUFFIX}/discard",
        "javascript:void(0)",
        earlier.rsplit("/", 1)[0] + "/was-already-open?omit=1#fragment",
        start.rsplit("/", 1)[0] + "/same-active-origin",
        prepared + "saved/deep?omit=1#fragment",
        f"http://{host}:80/deep?omit=1#fragment",
        f"https://{host}:8443/deep?omit=1#fragment",
    ]
    if source == "Chrome":
        for index in range(SITE_LIMIT):
            origin = f"https://cap-{index}.chrome{FIXTURE_SUFFIX}/"
            allowed.add(origin)
            inputs.append(origin + "deep?omit=1#fragment")
    rows = [{"type": "url", "id": str(index), "name": f"Synthetic site {index}", "url": url}
            for index, url in enumerate(inputs, 10)]

    def folder(id_, name, children):
        return {"type": "folder", "id": str(id_), "name": name, "children": children}

    roots = {
        "bookmark_bar": folder(1, "Bookmarks bar", rows[:2] + [folder(4, "Nested", rows[2:-1])]),
        "other": folder(2, "Other bookmarks", rows[-1:]),
        "synced": folder(3, "Mobile bookmarks", []),
    }
    path.parent.mkdir(parents=True)
    path.write_text(json.dumps({"version": 1, "roots": roots}), encoding="utf-8")
    return allowed


def bookmark_stamp(path):
    try:
        if path.resolve(strict=True) == path:
            stat = path.stat()
            return stat.st_ino, stat.st_mtime_ns, path.read_bytes()
    except OSError:
        pass


def confirm(browser, command, answer, report, check_pending):
    win, process = browser.win, browser.process
    user = C.WinDLL("user32", use_last_error=True)
    get_owner = api(user, "GetWindow", W.HWND, W.HWND, W.UINT)
    get_text = api(user, "GetWindowTextW", C.c_int, W.HWND, W.LPWSTR, C.c_int)
    require(not owned_windows(process, "#32770"), "An owned dialog was already open")
    report.update(command=command, answer="Yes" if answer == IDYES else "No")
    post(process, browser.hwnd, WM_COMMAND, command)
    dialogs = browser.wait(DIALOG_TITLE, lambda: owned_windows(process, "#32770"), 8)
    require(len(dialogs) == 1, "Expected exactly one owned Prepare sites dialog")
    dialog, = dialogs
    caption = C.create_unicode_buffer(256)
    get_text(dialog, caption, len(caption))
    require(caption.value == DIALOG_TITLE and get_owner(dialog, 4) == browser.hwnd,
            "Unexpected dialog title or owner")  # GW_OWNER
    buttons = {cid: win.get_item(dialog, cid) for cid in (IDYES, IDNO)}
    require(all(hwnd and win.pid(hwnd) == process.pid and win.visible(hwnd) and win.enabled(hwnd)
                and win.class_name(hwnd) == "Button" for hwnd in buttons.values()), "Missing owned Yes or No button")
    require(not win.enabled(browser.hwnd) and not owned_windows(process, "#32768"),
            "Expected direct modal confirmation for the single synthetic Default profile")
    report["owned_yes_no_dialog"] = True
    check_pending()
    post(process, dialog, WM_COMMAND, answer, buttons[answer])  # BN_CLICKED
    browser.wait("Prepare sites confirmation dismissed", lambda: (
        win.pid(dialog) != process.pid and win.enabled(browser.hwnd)
    ))
    report["dismissed"] = True


def loaded_views(browser):
    # Include hidden CEF views; an empty tab shell is not a loaded panel.
    win, panel, views = browser.win, None, {}
    while panel := win.find_child(browser.hwnd, panel, "Static", None):
        if child := win.find_child(panel, None, None, None):
            views[panel] = child
    return views


def exercise(exe, profile, site, source, earlier, start, prepared, allowed, report):
    seed_profile(profile, site.origin)
    expected_added = min(SITE_LIMIT, len(allowed))
    report["expected_added"] = expected_added
    args = [f"--flit-profile={profile}", f"--flit-url={earlier}"]
    earlier_hits = site.snapshot()[0]["earlier"]
    with OwnedProcess(Win32(), exe, args, report.setdefault("launch", {})) as process:
        browser = Window(process, site)

        def rendered(url, route, before):
            return (site.snapshot()[0][route] > before and browser.address() == url
                    and browser.has_page_title(fixture_title(route)))

        browser.wait("earlier existing page", lambda: rendered(earlier, "earlier", earlier_hits))
        start_hits = site.snapshot()[0]["start"]
        browser.click(NEW_TAB)
        browser.wait("second tab selected", lambda: browser.tab_count() == 2 and browser.selected() == 1)
        browser.edit_address(start)
        browser.click(GO)
        browser.wait("active existing page", lambda: rendered(start, "start", start_hits))
        panel = browser.wait("active CEF panel", browser.page_panel)
        views, before = loaded_views(browser), site.snapshot()
        require(len(views) == 2 and panel in views, "Expected both original views to be loaded")
        require(not before[0]["prepared"], "Prepared fixture loaded before either command")

        def parked(evidence, added, *, in_progress=False):
            evidence.update(added=browser.tab_count() - 2, selected=browser.selected(),
                            original_page=browser.page_panel() == panel and browser.address() == start
                            and browser.has_page_title(fixture_title("start")),
                            unchanged_views=loaded_views(browser) == views,
                            no_fixture_traffic=site.snapshot() == before)
            require(evidence["selected"] == 1 and evidence["original_page"], "The active page changed before selection")
            require(evidence["unchanged_views"], "A CEF view was added or replaced before selection")
            require(evidence["no_fixture_traffic"], "A fixture site was requested before selection")
            actual = evidence["added"]
            require(actual == added or (in_progress and 0 <= actual < added),
                    f"Unexpected prepared tab count: got {actual}, expected {added}")
            return actual == added

        for answer, label, added in ((IDNO, "cancel", 0), (IDYES, "prepare", expected_added)):
            stage = report[label] = {"pending": {}, "after": {}}
            confirm(browser, SOURCES[source][0], answer, stage, lambda: parked(stage["pending"], 0))
            browser.wait(f"{source} {label}: {added} prepared tabs", lambda: parked(stage["after"], added, in_progress=True))
            deadline = time.monotonic() + QUIET_SECONDS
            browser.wait(f"{source} {label}: quiet parked state", lambda: (
                parked(stage["after"], added) and time.monotonic() >= deadline
            ), 3)
            stage["quiet_ms"] = int(QUIET_SECONDS * 1000)

        selected_url = start
        if source == "Edge":
            # Filter to the HTTP loopback row so selection does not assume bookmark ordering.
            browser.set_text(TAB_FILTER, urlsplit(prepared).netloc)
            browser.wait("only the prepared loopback row", lambda: browser.tab_count() == 1 and browser.selected() == -1)
            require(browser.page_panel() == panel and loaded_views(browser) == views and site.snapshot() == before,
                    "Filtering the list loaded the prepared site")
            require(browser.message(TABS, 0x0186, 0) == 0, "Could not select the prepared row")  # LB_SETCURSEL
            post(process, browser.hwnd, WM_COMMAND, (1 << 16) | TABS, browser.controls[TABS])  # LBN_SELCHANGE
            browser.wait("prepared root rendered in a new CEF panel after selection", lambda: (
                rendered(prepared, "prepared", before[0]["prepared"])
                and (visible := browser.page_panel()) and visible not in views and len(loaded_views(browser)) == 3
            ))
            browser.set_text(TAB_FILTER, "")
            report["loaded_only_after_selection"] = True
            selected_url = prepared

        process.normal_close(browser.hwnd)

    # The live list count above also catches bad rows the session writer might drop.
    state = json.loads((profile / "session.json").read_text(encoding="utf-8"))
    urls = [entry["url"] for entry in state["entries"]]
    parked_urls = set(urls) - {earlier, start}
    saved = report["session"] = {
        "original_tabs_kept": earlier in urls and start in urls,
        "exact_count": len(urls) == expected_added + 2,
        "unique_normalized_origins": len(urls) == len(set(urls)) and parked_urls <= allowed
        and len(parked_urls) == expected_added,
        "expected_page_selected": 0 <= state["selected"] < len(urls) and urls[state["selected"]] == selected_url,
    }
    require(all(saved.values()), f"{source}: unexpected saved tab count, URLs, or selection")


def run(exe, work, report):
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    work.mkdir(parents=True, exist_ok=True)
    scratch = tempfile.TemporaryDirectory(prefix="flit-prepare-sites-", dir=work)
    root = Path(scratch.name)
    local = root / "localappdata"
    sources, stamps = {}, {}
    try:
        with Proxy() as site:
            site.RequestHandlerClass = Handler
            earlier = f"http://127.0.0.2:{site.server_port}/earlier"
            start = site.origin + "/start"
            prepared = f"http://localhost:{site.server_port}/"
            site.routes = {earlier: "earlier", start: "start", prepared: "prepared"}
            for name, (_, vendor) in SOURCES.items():
                path = local / vendor / "User Data" / "Default" / "Bookmarks"
                sources[name] = seed_bookmarks(path, name, earlier, start, prepared)
                stamp = bookmark_stamp(path)
                require(stamp is not None, "Could not snapshot the synthetic Bookmarks file")
                stamps[name] = (path, stamp)

            thread = Thread(target=site.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
            thread.start()
            try:
                # OwnedProcess inherits this override; the machine/user environment is untouched.
                with patch.dict(os.environ, LOCALAPPDATA=str(local)):
                    for name, allowed in sources.items():
                        exercise(exe, root / f"flit-{name.lower()}", site, name, earlier, start, prepared,
                                 allowed, report.setdefault("sources", {}).setdefault(name, {}))
            finally:
                site.shutdown()
                thread.join()
    finally:
        report["source_bookmarks_unchanged"] = {name: bookmark_stamp(path) == stamp
                                                for name, (path, stamp) in stamps.items()}
        clean_scratch(scratch, report)
        report["scratch_removed"] = not root.exists()
    require(all(report["source_bookmarks_unchanged"].values()), "A synthetic source Bookmarks file changed")
    require(report["scratch_removed"] and "scratch_cleanup_error" not in report, "Disposable scratch was not removed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", required=True, type=Path, help="workspace parent for disposable profiles")
    args = parser.parse_args()
    exe = args.exe.resolve()
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not exe.is_file() or not exe.with_name("Flit.dll").is_file():
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll")

    report = {"result": "fail"}
    try:
        run(exe, args.work.resolve(), report)
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
