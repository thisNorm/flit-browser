"""Check site information uses the committed origin and opens its settings."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import json
from pathlib import Path
import subprocess
import tempfile
from urllib.parse import parse_qs, urlsplit
from windows_devtools import tools_windows
from windows_download_close import owned_windows, post
from windows_smoke import Browser, GO, OwnedProcess, Win32, WORKSPACE, api, fixture, require


SITE_INFO, SETTINGS, WM_COMMAND, IDYES = 166, 122, 0x0111, 6


def settings_address(browser, hwnd):
    require(browser.win.pid(hwnd) == browser.process.pid, "Settings window is no longer owned")
    helper = Path(__file__).with_name("read_address.ps1")
    result = subprocess.run(["powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive", "-File",
                             str(helper), "-Hwnd", str(hwnd), "-OwnerPid", str(browser.process.pid)],
                            capture_output=True, check=True, timeout=8, creationflags=subprocess.CREATE_NO_WINDOW)
    address = result.stdout.decode("utf-8-sig").strip()
    return address if address.startswith("chrome://") or not address else "chrome://" + address


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-site-info-", dir=work)) / "profile"
    launch = report["launch"] = {}
    with fixture() as site:
        win = Win32()
        with OwnedProcess(win, exe, [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"], launch) as process:
            browser = Browser(process, site)
            browser.page("/start", require_request=True)
            button = win.get_item(browser.hwnd, SITE_INFO)
            require(button and win.pid(button) == process.pid and win.enabled(button), "Site information button missing")

            # Open Settings first to verify the site-specific route reuses its window.
            post(process, browser.hwnd, WM_COMMAND, SETTINGS)
            def native_settings():
                return next(((hwnd, title) for hwnd, title in tools_windows(browser).items()
                             if "settings" in title.casefold() or "\uc124\uc815" in title), None)

            settings_hwnd, _ = browser.wait("general settings window", native_settings)
            browser.edit_address(site.origin.replace("127.0.0.1", "localhost") + "/not-submitted")
            post(process, button, 0x00F5)  # BM_CLICK, on the owned toolbar button only.
            dialog = browser.wait("site information dialog", lambda: next(iter(owned_windows(process, "#32770")), None))
            user = C.WinDLL("user32", use_last_error=True)
            owner = api(user, "GetWindow", W.HWND, W.HWND, W.UINT)
            read_text = api(user, "GetWindowTextW", C.c_int, W.HWND, W.LPWSTR, C.c_int)
            enum_children = api(user, "EnumChildWindows", W.BOOL, W.HWND, win.callback, W.LPARAM)
            require(owner(dialog, 4) == browser.hwnd, "Site information dialog is not owned by Flit")

            def labels():
                text = []

                @win.callback
                def read(child, _):
                    value = C.create_unicode_buffer(1024)
                    if win.pid(child) == process.pid:
                        if not read_text(child, value, len(value)):
                            win.message(child, 0x000D, len(value), C.addressof(value))
                        if value.value:
                            text.append(value.value)
                    return True

                enum_children(dialog, read, 0)
                return " ".join(text)

            report["dialog"] = browser.wait("site information labels", labels, timeout=3)
            require(site.origin in report["dialog"] and "not encrypted" in report["dialog"]
                    and "localhost" not in report["dialog"], "Dialog did not describe the committed HTTP page")

            post(process, dialog, WM_COMMAND, IDYES, win.get_item(dialog, IDYES))

            def settings():
                address = settings_address(browser, settings_hwnd)
                route = urlsplit(address)
                return address if route.path == "/content/siteDetails" and parse_qs(route.query) == {"site": [site.origin]} else None

            report["settings"] = browser.wait("per-site settings route in the same window", settings)
            require(browser.tab_count() == 1 and browser.selected() == 0, "Site settings altered the Flit tab list")

            route = "chrome://settings/content"
            browser.edit_address(route)
            browser.click(GO)
            report["typed_settings"] = browser.wait("typed route in the same settings window",
                                                    lambda: settings_address(browser, settings_hwnd) == route)
            browser.page("/start")
            require(browser.tab_count() == 1 and browser.selected() == 0, "Typed settings replaced the Flit tab")
            process.normal_close(browser.hwnd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    report = {"result": "fail"}
    try:
        work = args.work.resolve()
        require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
        run(args.exe.resolve(strict=True), work, report)
        report["result"] = "pass"
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        report["error"] = str(error)
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
