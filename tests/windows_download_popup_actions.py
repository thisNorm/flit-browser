"""Exercise native Exact Downloads actions with held loopback attachments."""

import argparse
import hashlib
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import time

sys.dont_write_bytecode = True
from windows_download_close import owned_windows
from windows_downloads import TITLE, seed_profile, sha256
from windows_downloads_controls import BODY, CANCEL, COMPLETE, HEAD, NAMES, fixture, held, start_attachment
from windows_exact_ui import BUTTON, DOWNLOADS, TEXT, open_panel, popups, row, uia, watch
from windows_multi_session import clean_scratch
from windows_smoke import Browser, OwnedProcess, Win32, require


ACTIONS = ("Pause", "Resume", "Cancel")
CANCELED_DETAILS = ("Canceled", "Stopped")


def open_downloads(browser):
    popup, surface, _ = open_panel(browser, DOWNLOADS, "Downloads", "Show all downloads")
    return popup, surface


def close_downloads(browser, popup):
    browser.click(DOWNLOADS)
    browser.wait("owned Downloads popover dismissed",
                 lambda: not popups(browser) and browser.win.pid(popup) != browser.process.pid)


def wait_row(browser, surface, filename, *, actions=(), details=()):
    expected = sorted(actions)

    def matches(tree):
        visible_actions = sorted(item["name"] for item in tree["rows"] if item["kind"] == BUTTON)
        return (row(tree, filename, TEXT) and visible_actions == expected
                and (not details or any(row(tree, detail, TEXT) for detail in details)))

    return watch(browser, surface, (filename, *ACTIONS, *details),
                 f"{filename}: controls {actions}, detail {details}", matches)


def invoke(browser, surface, name):
    snapshot = uia(browser, surface, (name,))
    item = row(snapshot, name, BUTTON)
    require(item, f"{name} button is not visible in the owned Exact panel")
    win = browser.win
    with win.window_coordinates(surface):
        left, top, _, _ = win.rect(surface)
        x, y, width, height = item["rect"]
        px, py = int(x + width / 2 - left), int(y + height / 2 - top)
        _, _, right, bottom = win.rect(surface, client=True)
        require(0 < px < right and 0 < py < bottom, f"{name} is outside the client area")
        point = px | (py << 16)
        win.message(surface, 0x0201, 1, point)
        win.message(surface, 0x0202, 0, point)


def check_private_downloads(browser):
    class PrivateWindow(Browser):
        def find_window(self):
            return next((h for h in owned_windows(self.process, "FlitBrowserWindow")
                         if h != browser.hwnd), None)

    win = browser.win
    win.message(browser.hwnd, 0x0111, 162)  # Private window
    private = PrivateWindow(browser.process, browser.site)
    popup, surface = open_downloads(private)
    empty = "Nothing downloaded in this session"
    names = (empty, *NAMES.values())
    tree = watch(private, surface, names, "empty private Downloads list",
                 lambda tree: row(tree, empty, TEXT))
    require(not any(row(tree, name, TEXT) for name in NAMES.values()),
            "General-window download appeared in the private panel")
    close_downloads(private, popup)
    win.check(win.post(private.hwnd, 0x0010, 0, 0))  # WM_CLOSE to the private window only
    browser.wait("private window closed", lambda: not win.visible(private.hwnd))


def exercise(browser, downloads, report):
    site = browser.site
    url = site.origin + "/"
    browser.wait("loopback attachment page", lambda: (
        site.hits["/ready"] and browser.address() == url and browser.has_page_title(TITLE)
    ))
    source_tab = browser.selected(), browser.tab_count()

    report["stage"] = "Pause and Resume"
    partial, final = start_attachment(browser, downloads, COMPLETE, 0)
    popup, surface = open_downloads(browser)
    report["stage"] = "initial transfer actions"
    wait_row(browser, surface, final.name, actions=("Pause", "Cancel"))
    report["stage"] = "Pause and show paused state"
    invoke(browser, surface, "Pause")
    wait_row(browser, surface, final.name, actions=("Resume", "Cancel"), details=("Paused",))
    require(not site.release.is_set() and held(partial) and not final.exists(),
            "The completion fixture was not held at Pause")

    # Release the rest of the response while paused so a UI-only pause cannot pass.
    site.release.set()
    browser.wait("fixture sent remaining bytes while paused", site.tail_sent.is_set, 5)
    deadline = time.monotonic() + 0.75
    while True:
        require(held(partial) and not final.exists(), "The partial file advanced while paused")
        if time.monotonic() >= deadline:
            break
        time.sleep(0.05)
    report["paused_bytes"] = len(HEAD)

    report["stage"] = "Resume and show completed state"
    invoke(browser, surface, "Resume")
    wait_row(browser, surface, final.name, details=("96.0 KiB · Just now",))
    browser.wait("Resume finalized the attachment",
                 lambda: final.is_file() and not partial.exists())
    digest = sha256(final)
    require(final.stat().st_size == len(BODY) and digest == hashlib.sha256(BODY).hexdigest(),
            "Resumed file differs from the loopback attachment")
    report["completed"] = {"filename": final.name, "bytes": len(BODY), "sha256": digest}
    close_downloads(browser, popup)

    report["stage"] = "Cancel"
    cancel_started = time.monotonic()
    partial, canceled = start_attachment(browser, downloads, CANCEL, 1)
    popup, surface = open_downloads(browser)
    wait_row(browser, surface, canceled.name, actions=("Pause", "Cancel"))
    require(held(partial) and not canceled.exists(), "The cancel fixture was not held at Cancel")
    invoke(browser, surface, "Cancel")
    tree = wait_row(browser, surface, canceled.name, details=CANCELED_DETAILS)
    browser.wait("Cancel removed the partial with no final file",
                 lambda: not partial.exists() and not canceled.exists())
    # The shared fixture keeps the cancel response open for 45 seconds.
    require(time.monotonic() - cancel_started < 45, "Cancel must settle before the held response can time out")
    with site.lock:
        require(site.hits[CANCEL] == 1, "Expected one held request for the canceled attachment")
    report["canceled"] = {
        "filename": canceled.name,
        "detail": next(detail for detail in CANCELED_DETAILS if row(tree, detail, TEXT)),
        "partial_absent": True,
        "final_absent": True,
    }
    close_downloads(browser, popup)
    require(browser.address() == url and (browser.selected(), browser.tab_count()) == source_tab,
            "Downloads actions changed the source tab")
    report["stage"] = "private download isolation"
    check_private_downloads(browser)
    report["private_list_isolated"] = True


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    scratch = TemporaryDirectory(prefix="flit-download-popup-actions-", dir=work)
    root = Path(scratch.name)
    try:
        with fixture() as site:
            profile, downloads = root / "profile", root / "attachments"
            downloads.mkdir()
            seed_profile(profile, downloads, site.origin)
            local_state = profile / "Local State"
            state = json.loads(local_state.read_text(encoding="utf-8"))
            state["intl"] = {"app_locale": "en-US"}
            local_state.write_text(json.dumps(state), encoding="utf-8")

            launch_args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/"]
            try:
                with OwnedProcess(Win32(), exe, launch_args, report["launch"]) as process:
                    browser = Browser(process, site)
                    exercise(browser, downloads, report)
                    report["stage"] = "close owned browser"
                    process.normal_close(browser.hwnd)
                require({path.name for path in downloads.iterdir()} == {NAMES[COMPLETE]},
                        "Expected only the resumed file in the disposable download directory")
            finally:
                with site.lock:
                    report["attachment_requests"] = list(site.attachment_requests)
    finally:
        clean_scratch(scratch, report)
    require(not root.exists() and report["launch"].get("leftover_pids") == [], "Test cleanup failed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe")
    parser.add_argument("--work", required=True, type=Path, help="parent for the disposable profile and downloads")
    args = parser.parse_args()
    exe = args.exe.resolve()
    bundle = (exe, *(exe.with_name(name) for name in ("Flit.dll", "FlitExact.dll", "libcef.dll")))
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not all(path.is_file() for path in bundle):
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll, FlitExact.dll and libcef.dll")

    report = {"result": "fail", "stage": "setup", "launch": {}}
    try:
        run(exe, args.work.resolve(), report)
        report.update(result="pass", stage="complete")
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
