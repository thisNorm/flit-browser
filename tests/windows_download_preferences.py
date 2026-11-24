"""Exercise Flit's Windows download prompt using an English profile and a loopback attachment."""

import argparse
from base64 import b64encode
import ctypes as C
from ctypes import wintypes as W
import json
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from windows_devtools import title, tools_windows
from windows_download_close import post
from windows_downloads import ATTACHMENTS, TITLE, click_attachment, fixture, seed_profile, sha256
from windows_smoke import Browser, OwnedProcess, Win32, api, require


DOWNLOAD_PREFERENCES = 193
CLOSE_TAB = 106
PROMPT_LABEL = "Ask where to save each file before downloading"
ROUTE = ATTACHMENTS[0][0]

# Chromium exposes the WebUI switch's checked state and default action through TogglePattern.
UIA = r"""
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$condition = [Windows.Automation.PropertyCondition]::new(
    [Windows.Automation.AutomationElement]::NameProperty, $label)
$clock = [Diagnostics.Stopwatch]::StartNew()
$before = $null
do {
    $root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$target)
    if (!$root -or $root.Current.ProcessId -ne $ownerPid) { throw 'Settings HWND is no longer owned' }
    $matches = @(foreach ($item in $root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)) {
        try {
            $v = $item.Current
            if ($v.ProcessId -ne $ownerPid -or $v.IsOffscreen -or !$v.IsEnabled) { continue }
            $pattern = $null
            if ($item.TryGetCurrentPattern([Windows.Automation.TogglePattern]::Pattern, [ref]$pattern)) {
                $pattern
            }
        } catch [Windows.Automation.ElementNotAvailableException] {}
    })
    if ($matches.Count -gt 1) { throw "Ambiguous Settings switch: $label" }
    if ($matches.Count -eq 1) {
        $toggle = $matches[0]
        $state = [string]$toggle.Current.ToggleState
        if ($null -eq $before) {
            if ($state -cne 'Off') { throw "Expected $label to start Off, got $state" }
            if ($root.Current.ProcessId -ne $ownerPid) { throw 'Settings HWND is no longer owned' }
            $before = $state
            $toggle.Toggle()
        } elseif ($state -ceq 'On') {
            ConvertTo-Json -Compress -InputObject @{before = $before; after = $state}
            exit 0
        }
    }
    Start-Sleep -Milliseconds 100
} while ($clock.Elapsed.TotalSeconds -lt 15)
throw "Timed out waiting for the English Settings switch '$label' to be available and On (before: $before)"
"""


def enable_prompt(browser, hwnd):
    process, win = browser.process, browser.win
    process.alive()
    require(win.pid(hwnd) == process.pid and win.visible(hwnd), "Expected a visible owned Flit HWND")
    command = f"$target = [long]{hwnd}; $ownerPid = {process.pid}; $label = '{PROMPT_LABEL}';\n" + UIA
    result = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-Mta", "-EncodedCommand",
         b64encode(command.encode("utf-16le")).decode("ascii")],
        stdin=subprocess.DEVNULL, capture_output=True, encoding="utf-8", errors="replace",
        timeout=22, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    require(result.returncode == 0, f"Settings UI Automation failed: {result.stderr.strip()}")
    states = json.loads(result.stdout)
    require(states == {"before": "Off", "after": "On"}, f"Unexpected switch states: {states!r}")
    return states


def owned_dialogs(process):
    win, pids, dialogs = process.win, process.pids(), []

    @win.callback
    def visit(hwnd, _):
        if win.pid(hwnd) in pids and win.visible(hwnd) and win.class_name(hwnd) == "#32770":
            dialogs.append(hwnd)
        return True

    win.check(win.enum(visit, 0))
    return dialogs


def cancel_save_as(browser, evidence):
    process, win = browser.process, browser.win
    ancestor = api(C.WinDLL("user32", use_last_error=True), "GetAncestor", W.HWND, W.HWND, W.UINT)

    def ready():
        dialogs = owned_dialogs(process)
        if not dialogs:
            return None
        require(len(dialogs) == 1, f"Expected one owned download picker, got {dialogs}")
        dialog = dialogs[0]
        owner = ancestor(dialog, 3)  # GA_ROOTOWNER, including the embedded tab's native window.
        caption = title(win, dialog)
        evidence.update(hwnd=dialog, root_owner=owner, caption=caption)
        require(owner == browser.hwnd, "Save picker does not belong to the original Flit window")
        # The native picker uses the Windows display language.
        if not caption:
            return None
        require(caption.casefold() in ("save as", "다른 이름으로 저장"), f"Unexpected picker title: {caption!r}")
        cancel = win.get_item(dialog, 2)  # IDCANCEL
        if (cancel and win.pid(cancel) == win.pid(dialog) and win.class_name(cancel) == "Button"
                and win.visible(cancel) and win.enabled(cancel)):
            return dialog, cancel

    dialog, cancel = browser.wait("owned Save As dialog with a Cancel button", ready, 15)
    require(win.pid(dialog) == win.pid(cancel) in process.pids() and ancestor(dialog, 3) == browser.hwnd
            and win.get_item(dialog, 2) == cancel, "Save picker ownership changed before Cancel")
    win.check(win.post(dialog, 0x0111, 2, cancel))  # WM_COMMAND / IDCANCEL / BN_CLICKED.
    browser.wait("Save As dismissed by Cancel", lambda: not win.visible(dialog) and win.enabled(browser.hwnd))
    evidence["cancelled"] = True


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="flit-download-preferences-", dir=work))
    profile, downloads = scratch / "profile", scratch / "attachments"
    downloads.mkdir()
    prefs = profile / "Default" / "Preferences"
    report.update(work=str(scratch), preferences_path=str(prefs), downloads_path=str(downloads), launch={})
    with fixture() as site:
        seed_profile(profile, downloads, site.origin)
        local_state = profile / "Local State"
        state = json.loads(local_state.read_text(encoding="utf-8"))
        state["intl"] = {"app_locale": "en-US"}
        local_state.write_text(json.dumps(state), encoding="utf-8")
        url = site.origin + "/"
        try:
            with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}", f"--flit-url={url}"],
                              report["launch"]) as process:
                browser = Browser(process, site)

                def original_page():
                    return browser.address() == url and browser.has_page_title(TITLE)

                browser.wait("ordinary attachment page through the loopback proxy",
                             lambda: site.hits["/ready"] and original_page())
                tabs = browser.tab_count(), browser.selected()
                require(not tools_windows(browser), "An owned Chromium window was already open")
                post(process, browser.hwnd, 0x0111, DOWNLOAD_PREFERENCES)  # WM_COMMAND
                # Download settings open as a Flit tab with the Flit skin, not a Chromium window.
                browser.wait("download Settings tab", lambda: browser.address() == "chrome://settings/downloads")
                require(not tools_windows(browser), "Download Settings opened a separate Chromium window")
                settings = report["settings"] = {"hwnd": browser.hwnd, "switch": PROMPT_LABEL}
                settings.update(enable_prompt(browser, browser.page_panel()))
                post(process, browser.hwnd, 0x0111, CLOSE_TAB)  # WM_COMMAND, back to the original tab.
                browser.wait("Settings tab closed", lambda: browser.tab_count() == tabs[0])
                require(tabs == (browser.tab_count(), browser.selected()) and original_page(),
                        "Download Settings changed the ordinary Flit tab")

                require(not owned_dialogs(process) and not any(site.hits[route] for route, _, _ in ATTACHMENTS),
                        "A dialog or attachment request was already present before the download click")
                click_attachment(browser, 0)
                try:
                    browser.wait("attachment GET from the ordinary tab", lambda: site.hits[ROUTE], 2)
                except RuntimeError:
                    process.alive()
                    if not site.hits[ROUTE]:
                        click_attachment(browser, 0)
                browser.wait("attachment GET from the ordinary tab", lambda: site.hits[ROUTE])
                cancel_save_as(browser, report.setdefault("save_as", {}))
                process.normal_close(browser.hwnd)
        finally:
            report["http_requests"] = dict(site.hits)

        require([site.hits[route] for route, _, _ in ATTACHMENTS] == [1, 0], "Expected exactly one fixture attachment GET")
        report["files_after_cancel"] = sorted(path.name for path in downloads.iterdir())
        require(not report["files_after_cancel"], "Cancel left files in the isolated download directory")
        prompt = json.loads(prefs.read_text(encoding="utf-8")).get("download", {}).get("prompt_for_download")
        report["prompt_for_download_after_close"] = prompt
        require(prompt is True, "Default/Preferences did not persist download.prompt_for_download=true")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", required=True, type=Path, help="parent for the fresh profile and attachments")
    args = parser.parse_args()
    exe = args.exe.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires interactive Windows and a staged Flit.exe beside Flit.dll")
    report = {"result": "fail", "exe": str(exe)}
    try:
        report["sha256"] = {path.name: sha256(path) for path in (exe, dll)}
        run(exe, args.work.resolve(), report)
        require(report["sha256"] == {path.name: sha256(path) for path in (exe, dll)},
                "Staged payload changed during the test")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
