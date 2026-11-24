"""Exercise the Windows Exact Settings and toolbar surfaces in an owned Flit job.

Run on an available interactive desktop with a staged bundle:
    python tests/windows_exact_ui.py --exe PATH/Flit.exe [--extension-fixture]
--self-check parses the PowerShell and loads the UIA assemblies without opening Flit.
"""

import argparse
from base64 import b64encode
import ctypes as C
from ctypes import wintypes as W
from hashlib import file_digest
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

sys.dont_write_bytecode = True
from windows_download_close import owned_windows
from windows_multi_session import clean_scratch
from windows_session import select, send
from windows_smoke import Browser, OwnedProcess, Win32, api, require
from windows_windows import fixture, seed_profile


SETTINGS, MENU, DOWNLOADS, EXTENSIONS, VAULT = 122, 117, 190, 200, 215
SETTINGS_URL = "flit://settings"
SURFACE, POPUP = "FlitExactSurface", "FlitPopover"
RADIO, BUTTON, TEXT = "ControlType.RadioButton", "ControlType.Button", "ControlType.Text"
EXTENSION_MARKER = "Flit synthetic extension popup"
LOCALES = {
    "en-US": ("Settings", "Menu", "Downloads", "Extensions", "Show all downloads",
              "Manage extensions", "15 min", "30 min"),
    "ko": ("설정", "메뉴", "다운로드", "확장 프로그램", "모든 다운로드 보기",
           "확장 프로그램 관리", "15분", "30분"),
}


UIA_ASSEMBLIES = """
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
"""
UIA_QUERY = r"""
function RectArray($r) {
    if (!$r.IsEmpty) { $r.X; $r.Y; $r.Width; $r.Height }
}
$root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr][long]$spec.hwnd)
if (!$root -or $root.Current.ProcessId -ne $spec.pid) { throw 'Wrong UIA owner' }
$targets = [Collections.Generic.List[Windows.Automation.AutomationElement]]::new()
$rows = @(foreach ($item in $root.FindAll([Windows.Automation.TreeScope]::Descendants,
                                         [Windows.Automation.Condition]::TrueCondition)) {
    try {
        $v = $item.Current
        if ($v.ProcessId -ne $spec.pid -or $v.IsOffscreen -or
            $spec.names -cnotcontains $v.Name) { continue }
        $kind = $v.ControlType.ProgrammaticName
        if ($v.Name -ceq $spec.target -and $kind -ceq $spec.kind) {
            $targets.Add($item)
        }
        $selected = $null
        $pattern = $null
        if ($kind -ceq 'ControlType.RadioButton' -and
            $item.TryGetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)) {
            $selected = $pattern.Current.IsSelected
        }
        @{name=$v.Name; kind=$kind; focused=$v.HasKeyboardFocus; selected=$selected
          rect=@(RectArray $v.BoundingRectangle)}
    } catch [Windows.Automation.ElementNotAvailableException] {}
})
if ($spec.action) {
    if ($targets.Count -ne 1) { throw 'UIA action target is missing or ambiguous' }
    $item = $targets[0]
    if ($item.Current.ProcessId -ne $spec.pid -or !$item.Current.IsEnabled) {
        throw 'UIA action target is unavailable'
    }
    switch ($spec.action) {
        'focus' { $item.SetFocus() }
        'select' { $item.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select() }
        'invoke' { $item.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() }
        default { throw 'Invalid UIA action' }
    }
    '{"dispatched":true}'
} else {
    $v = $root.Current
    ConvertTo-Json -Compress -Depth 5 -InputObject @{
        root = if ($spec.names -ccontains $v.Name) { $v.Name } else { $null }
        rect = @(RectArray $v.BoundingRectangle)
        rows = $rows
    }
}
"""


def ps_literal(text):
    return "'" + text.replace("'", "''") + "'"


def powershell(script):
    encoded = b64encode(script.encode("utf-16le")).decode("ascii")
    result = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded],
        stdin=subprocess.DEVNULL, capture_output=True, encoding="utf-8",
        timeout=10, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    require(result.returncode == 0, "Windows PowerShell UIA operation failed")
    return json.loads(result.stdout)


def self_check():
    script = UIA_ASSEMBLIES + f"""
$source = {ps_literal(UIA_QUERY)}
$tokens = $null
$errors = $null
$null = [System.Management.Automation.Language.Parser]::ParseInput(
    $source, [ref]$tokens, [ref]$errors)
if ($errors.Count) {{ throw 'UIA query failed to parse' }}
$null = [Windows.Automation.SelectionItemPattern]::Pattern
$null = [Windows.Automation.InvokePattern]::Pattern
$null = [Windows.Automation.AutomationElement]::HasKeyboardFocusProperty
'{{"result":"pass","check":"PowerShell parser and UIAutomation assemblies"}}'
"""
    return powershell(script)


def uia(browser, hwnd, names, *, action=None, target=None, kind=None):
    browser.process.alive()
    pid = browser.process.pid
    require(browser.win.pid(hwnd) == pid and browser.win.visible(hwnd), "UIA HWND is not owned and visible")
    spec = dict(hwnd=hwnd, pid=pid, names=list(names), target=target, action=action, kind=kind)
    script = UIA_ASSEMBLIES + "$spec = ConvertFrom-Json " + ps_literal(json.dumps(spec)) + "\n" + UIA_QUERY
    return powershell(script)


def row(snapshot, name, kind):
    found = [item for item in snapshot["rows"] if item["name"] == name and item["kind"] == kind]
    return found[0] if len(found) == 1 and len(found[0]["rect"]) == 4 else None


def watch(browser, hwnd, names, label, predicate):
    def ready():
        snapshot = uia(browser, hwnd, names)
        return snapshot if predicate(snapshot) else None

    return browser.wait(label, ready, 14)


def radio_state(snapshot, selected, other):
    yes, no = row(snapshot, selected, RADIO), row(snapshot, other, RADIO)
    return yes and no and yes["selected"] is True and no["selected"] is False


def focus_radio(browser, hwnd, name):
    require(uia(browser, hwnd, (name,), action="focus", target=name, kind=RADIO)["dispatched"],
            "UIA did not dispatch focus")
    focused = watch(browser, hwnd, (name,), "focused UIA radio",
                    lambda tree: (row(tree, name, RADIO) or {}).get("focused"))
    require(row(focused, name, RADIO)["selected"] is False, "Focus selected the radio before activation")


def press(browser, hwnd, key):
    win, pid = browser.win, browser.process.pid
    require(win.pid(hwnd) == pid and win.visible(hwnd), "Key target is not owned and visible")
    params = 1 | (win.map_key(key, 0) << 16)
    win.check(win.post(hwnd, 0x0100, key, params))  # WM_KEYDOWN
    up = win.post(hwnd, 0x0101, key, params | (3 << 30))  # WM_KEYUP
    if not up:
        # Escape can destroy the owned popup between the two posts.
        require(key == 0x1B and win.pid(hwnd) != pid, "Could not post WM_KEYUP to the owned window")


def surfaces(browser, parent):
    win, pid = browser.win, browser.process.pid
    for child in win.visible_children(parent):
        if win.pid(child) != pid:
            continue
        if win.class_name(child) == SURFACE:
            yield child
        else:
            yield from surfaces(browser, child)


def settings(browser, title, *, cold=False):
    send(browser, 0x0111, SETTINGS, post=cold)

    def ready():
        children = list(surfaces(browser, browser.hwnd))
        return (children[0] if len(children) == 1 and browser.address() == SETTINGS_URL
                and browser.has_page_title(title) and browser.tab_count() == 2
                and browser.selected() == 1 else None)

    return browser.wait("one owned Settings tab and Exact HWND", ready, 20)


def read_state(profile):
    try:
        state = json.loads((profile / "session.json").read_text(encoding="utf-8"))
    except (FileNotFoundError, PermissionError, json.JSONDecodeError):
        return {}
    return state


def preferences(state):
    return state.get("search_engine", "duckduckgo"), state.get("memory_saver_minutes")


def fixture_ready(browser, url, title):
    path = browser.site.paths["normal-peer"]
    browser.wait("loopback fixture tab", lambda: (
        browser.site.seen(path + "/ready") and browser.address() == url
        and browser.has_page_title(title) and browser.tab_count() == 1
        and browser.selected() == 0 and not list(surfaces(browser, browser.hwnd))), 12)


def verify_resize(browser, hwnd, title):
    win = browser.win
    with win.window_coordinates(hwnd):
        before = win.rect(hwnd, client=True)[2:]
        dpi = win.dpi(hwnd)
        left, top, right, bottom = win.rect(browser.hwnd)
        width, height = right - left, bottom - top
        narrowed = min(width, max(600 * dpi // 96, width - 120))
        shortened = min(height, max(470 * dpi // 96, height - 90))
        if (narrowed, shortened) == (width, height):
            return {"dpi": dpi, "status": "skipped_already_compact"}
        win.check(win.set_window_pos(browser.hwnd, None, 0, 0, narrowed, shortened, 0x16))

    def changed():
        with win.window_coordinates(hwnd):
            size = win.rect(hwnd, client=True)[2:]
        return size if size != before else None

    after = browser.wait("Exact client resize", changed)
    watch(browser, hwnd, (title,), "UIA bounds updated after resize", lambda tree: (
        row(tree, title, TEXT) and len(tree["rect"]) == 4
        and all(abs(observed - pixels) <= 2 for observed, pixels in zip(tree["rect"][2:], after))))
    return {"dpi": dpi, "dpi_after": win.dpi(hwnd), "before_px": before, "after_px": after}


def get_toolbar(browser, cid):
    win = browser.win
    hwnd = win.get_item(browser.hwnd, cid)
    require(hwnd and win.pid(hwnd) == browser.process.pid and win.visible(hwnd)
            and win.class_name(hwnd) == "Button", "Missing owned native toolbar button")
    browser.controls[cid] = hwnd


def popups(browser):
    return owned_windows(browser.process, POPUP)


def open_panel(browser, cid, title, action, action_kind=BUTTON):
    get_toolbar(browser, cid)
    require(not popups(browser), "Previous Flit popover is still visible")
    browser.click(cid)
    get_owner = api(C.WinDLL("user32", use_last_error=True), "GetWindow", W.HWND, W.HWND, W.UINT)

    def ready():
        windows = popups(browser)
        if len(windows) != 1 or get_owner(windows[0], 4) != browser.hwnd:  # GW_OWNER
            return None
        children = list(surfaces(browser, windows[0]))
        return (windows[0], children[0]) if len(children) == 1 else None

    popup, child = browser.wait("owned native toolbar popover", ready)
    tree = watch(browser, child, (title, action), "localized native heading and action",
                 lambda tree: row(tree, title, TEXT) and row(tree, action, action_kind))
    return popup, child, tree["root"] == title


def check_panels(browser, labels, result):
    title, menu, downloads, extensions, show_all, manage, _, _ = labels
    vault, secure_required = (
        ("로그인 보관함", "보안 연결이 필요합니다") if title == "설정"
        else ("Flit Vault", "Secure site required")
    )
    for label, cid, heading, action, action_kind in (
        ("menu", MENU, menu, title, BUTTON),
        ("downloads", DOWNLOADS, downloads, show_all, BUTTON),
        ("extensions", EXTENSIONS, extensions, manage, BUTTON),
        ("vault", VAULT, vault, secure_required, TEXT),
    ):
        result["stage"] = f"native_{label}"
        popup, child, root_name_matches = open_panel(browser, cid, heading, action, action_kind)
        if cid == DOWNLOADS:
            browser.click(cid)
            dismissal = "toolbar_toggle"
        else:
            press(browser, child, 0x1B)
            dismissal = "Escape_to_Exact_child"
        browser.wait("native popover is dismissed and its HWND destroyed",
                     lambda: not popups(browser) and browser.win.pid(popup) != browser.process.pid)
        result["native_panels"][label] = {
            "heading": heading, "root_name_matches": root_name_matches, "dismissed_by": dismissal,
        }


def install_extension_fixture(browser, folder, result):
    from windows_extensions import NAME, install

    folder.mkdir()
    (folder / "manifest.json").write_text(json.dumps({
        "manifest_version": 3, "name": NAME, "version": "1.0",
        "action": {"default_popup": "popup.html"},
    }), encoding="utf-8")
    (folder / "popup.html").write_text(
        f"<!doctype html><title>{EXTENSION_MARKER}</title><button>{EXTENSION_MARKER}</button>",
        encoding="utf-8",
    )
    result["stage"] = "install_synthetic_extension"
    tab = install(browser, folder, {"checks": {}})
    select(browser, 0)
    browser.close_row(tab)
    browser.wait("extensions management tab closed", lambda: browser.tab_count() == 1)
    result["extension_fixture"] = "installed_in_Chromium"


def verify_extension_fixture(browser, labels, result):
    from windows_extensions import NAME

    result["stage"] = "synthetic_extension_toolbar_row"
    _, child, _ = open_panel(browser, EXTENSIONS, labels[3], NAME)
    result["stage"] = "synthetic_extension_invoke"
    require(uia(browser, child, (NAME,), action="invoke", target=NAME, kind=BUTTON)["dispatched"],
            "UIA did not dispatch the synthetic extension row")

    def cef_popup():
        windows = popups(browser)
        if len(windows) == 1 and not list(surfaces(browser, windows[0])):
            if any(browser.win.pid(x) == browser.process.pid for x in browser.win.visible_children(windows[0])):
                return windows[0]
        return None

    result["stage"] = "synthetic_extension_cef_popup"
    popup = browser.wait("Chromium popup replacing the Exact extension list", cef_popup)
    result["stage"] = "synthetic_extension_popup_contents"
    watch(browser, popup, (EXTENSION_MARKER,), "synthetic Chromium popup button",
          lambda tree: row(tree, EXTENSION_MARKER, BUTTON))
    result["stage"] = "synthetic_extension_dismissal"
    press(browser, browser.hwnd, 0x1B)
    browser.wait("extension popup parent destroyed by owner Escape",
                 lambda: browser.win.pid(popup) != browser.process.pid and not popups(browser))
    result["extension_cef_parent_destroyed"] = True
    result["extension_fixture"] = "loaded_via_Chromium_and_popup_dismissed"


def job_result(raw, report, key):
    report[key] = {name: raw[name] for name in
                   ("normal_close", "wm_close_posted", "exit_code", "cleanup_needed") if name in raw}
    if "leftover_pids" in raw:
        report[key]["remaining_process_count"] = len(raw["leftover_pids"])


def test_locale(exe, profile, site, locale, run_extension, result):
    labels = LOCALES[locale]
    title, *_, after_choice, before_choice = labels
    path = site.paths["normal-peer"]
    url, fixture_title = site.origin + path, site.title("normal-peer")
    seed_profile(profile, site.origin)
    local = profile / "Local State"
    state = json.loads(local.read_text(encoding="utf-8"))
    state["intl"] = {"app_locale": locale}
    local.write_text(json.dumps(state), encoding="utf-8")
    expected = "google", 15
    launch = [f"--flit-profile={profile}"]
    result.update(stage="launch", native_panels={},
                  extension_fixture="pending" if run_extension else "not_requested")
    job = {}
    try:
        with OwnedProcess(Win32(), exe, [*launch, f"--flit-url={url}"], job) as process:
            browser = Browser(process, site)
            result["stage"] = "first_settings_open"
            fixture_ready(browser, url, fixture_title)
            browser.wait("default search engine and Memory Saver in the fixture profile",
                         lambda: preferences(read_state(profile)) == ("duckduckgo", 30))
            hwnd = settings(browser, title, cold=True)
            names = (title, "DuckDuckGo", "Google", before_choice, after_choice)
            watch(browser, hwnd, names, "Settings heading and both current UIA radio choices",
                  lambda tree: row(tree, title, TEXT) and radio_state(tree, "DuckDuckGo", "Google")
                  and radio_state(tree, before_choice, after_choice))
            result["initial"] = {"search_engine": "duckduckgo", "memory_saver_minutes": 30}

            result["stage"] = "uia_search_engine"
            focus_radio(browser, hwnd, "Google")
            require(uia(browser, hwnd, ("Google",), action="select", target="Google", kind=RADIO)["dispatched"],
                    "UIA did not dispatch the Search engine radio selection")
            watch(browser, hwnd, names, "Google selected and DuckDuckGo unselected",
                  lambda tree: radio_state(tree, "Google", "DuckDuckGo"))
            browser.wait("search engine persisted to the fixture profile",
                         lambda: preferences(read_state(profile)) == ("google", 30))
            result["search_engine"] = {"focus": True, "SelectionItem.Select": True, "persisted": "google"}

            result["stage"] = "keyboard_memory_saver"
            focus_radio(browser, hwnd, after_choice)
            press(browser, hwnd, 0x20)  # VK_SPACE
            watch(browser, hwnd, names, "Memory Saver selected by Space",
                  lambda tree: radio_state(tree, after_choice, before_choice))
            browser.wait("Memory Saver persisted to the fixture profile",
                         lambda: preferences(read_state(profile)) == expected)
            result["memory_saver"] = {"focus": True, "WM_key": "Space", "persisted_minutes": 15}

            result["stage"] = "repeat_and_resize_settings"
            require(settings(browser, title) == hwnd, "Repeated Settings open replaced the active HWND")
            select(browser, 0)
            require(settings(browser, title) == hwnd, "Settings reselect replaced the existing HWND")
            watch(browser, hwnd, names, "reopened Settings radio state",
                  lambda tree: radio_state(tree, "Google", "DuckDuckGo")
                  and radio_state(tree, after_choice, before_choice))
            result["repeated_open_reused_tab_and_surface"] = True
            result["stage"] = "settings_resize"
            result["resize"] = verify_resize(browser, hwnd, title)

            result["stage"] = "close_settings"
            browser.close_row(1)
            fixture_ready(browser, url, fixture_title)
            check_panels(browser, labels, result)
            if run_extension:
                install_extension_fixture(browser, profile.parent / "extension-fixture", result)
                verify_extension_fixture(browser, labels, result)
            fixture_ready(browser, url, fixture_title)
            result["stage"] = "first_clean_exit"
            process.normal_close(browser.hwnd)
        require(preferences(read_state(profile)) == expected, "Preferences changed during clean shutdown")
    finally:
        job_result(job, result, "first_job")

    result["stage"] = "relaunch_from_same_disposable_profile"
    job = {}
    try:
        with OwnedProcess(Win32(), exe, launch, job) as process:
            browser = Browser(process, site)
            fixture_ready(browser, url, fixture_title)
            hwnd = settings(browser, title, cold=True)
            watch(browser, hwnd, names, "persisted radio choices after process relaunch",
                  lambda tree: radio_state(tree, "Google", "DuckDuckGo")
                  and radio_state(tree, after_choice, before_choice))
            browser.close_row(1)
            fixture_ready(browser, url, fixture_title)
            result["stage"] = "relaunch_clean_exit"
            process.normal_close(browser.hwnd)
        state = read_state(profile)
        require(preferences(state) == expected and [tab["url"] for tab in state.get("entries", ())] == [url],
                "Final profile did not retain the two choices and just the fixture tab")
        result.update(stage="pass", reload_verified=True, result="pass")
    finally:
        job_result(job, result, "relaunch_job")


def bundle_hashes(exe):
    hashes = {}
    for path in (exe, exe.with_name("Flit.dll"), exe.with_name("FlitExact.dll")):
        with path.open("rb") as stream:
            hashes[path.name] = file_digest(stream, "sha256").hexdigest()
    return hashes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, help="staged Flit.exe beside Flit.dll and FlitExact.dll")
    parser.add_argument("--work", type=Path, help="optional existing directory for disposable profiles")
    parser.add_argument("--locale", action="append", choices=LOCALES, help="repeatable, defaults to English and Korean")
    parser.add_argument("--extension-fixture", action="store_true", help="also install and open a synthetic MV3 popup in English")
    parser.add_argument("--self-check", action="store_true", help="check QA prerequisites without launching GUI")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    if args.self_check:
        try:
            print(json.dumps(self_check()))
            return 0
        except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
            print(json.dumps({"result": "fail", "check": "self-check", "error": type(error).__name__}))
            return 1

    locales = tuple(dict.fromkeys(args.locale or LOCALES))
    if args.extension_fixture and "en-US" not in locales:
        parser.error("--extension-fixture requires the English run for Chromium's folder picker")
    if not args.exe or args.exe.name.casefold() != "flit.exe":
        parser.error("--exe must point to the staged Flit.exe")
    exe = args.exe.resolve()
    report = {"result": "fail", "runs": {}}
    try:
        report["stage"] = "validate_bundle"
        report["sha256"] = bundle_hashes(exe)
        report["stage"] = "disposable_profiles"
        scratch = TemporaryDirectory(prefix="flit-exact-ui-", dir=args.work)
        cleanup = {}
        try:
            with fixture() as site:
                for locale in locales:
                    result = report["runs"][locale] = {"result": "fail", "stage": "seed_profile"}
                    try:
                        test_locale(exe, Path(scratch.name) / locale, site, locale,
                                    args.extension_fixture and locale == "en-US", result)
                    except Exception as error:
                        result["error"] = type(error).__name__
        finally:
            clean_scratch(scratch, cleanup)
            report["scratch_removed"] = not Path(scratch.name).exists()
        report["stage"] = "verify_bundle_and_results"
        require(report["sha256"] == bundle_hashes(exe), "Candidate bundle changed during the run")
        if report["scratch_removed"] and all(run["result"] == "pass" for run in report["runs"].values()):
            report.update(result="pass", stage="pass")
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = type(error).__name__
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
