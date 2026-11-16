"""Check one synthetic unpacked MV3 extension in a staged Windows Flit.

Use --exe PATH/Flit.exe on an interactive Windows desktop, one run at a time.
Prints JSON evidence. Uses a new temporary profile and no global input.
Requires Python 3.11+ and Windows PowerShell (UI Automation); no Pillow.
"""

import argparse
from base64 import b64encode
from collections import Counter
from contextlib import contextmanager
import ctypes as C
from ctypes import wintypes as W
from hashlib import file_digest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
import traceback
from urllib.parse import parse_qs, urlsplit

sys.dont_write_bytecode = True
from windows_download_close import owned_windows, post
from windows_multi_session import clean_scratch
from windows_session import select, send
from windows_smoke import Browser, GO, OwnedProcess, Win32, api, require
from windows_windows import seed_profile


NAME = "Flit Synthetic MV3 Probe"
TITLE = "Flit synthetic MV3 loopback page"
TARGET, OTHER_PATH = "/flit-synthetic-mv3/target", "/flit-synthetic-mv3/outside"
MARKER, WORKER_REPLY = "flit-synthetic-content-ok", "flit-synthetic-worker-ok"
EXTENSIONS = "chrome://extensions/"
REMOVE_TITLE = f'Remove "{NAME}"?'

PAGE = b"""<!doctype html><title>Flit synthetic MV3 loopback page</title>
<link rel="icon" href="data:,"><main>Flit synthetic MV3 loopback page</main>
<script>
let last;
function report() {
  const node = document.getElementById('flit-synthetic-mv3-marker');
  const pair = [node?.textContent || '', node?.dataset.worker || ''];
  const current = JSON.stringify(pair);
  if (current === last) return;
  last = current;
  fetch('/observation?' + new URLSearchParams({
    probe: location.search.slice(1), marker: pair[0], worker: pair[1]
  }));
}
new MutationObserver(report).observe(document.body, {subtree:true, childList:true, attributes:true});
addEventListener('load', () => setTimeout(report, 500));
</script>"""


class Proxy(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, status, body=b""):
        try:
            self.send_response(status)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if body:
                self.wfile.write(body)
        except ConnectionError:
            pass

    def deny(self):
        with self.server.lock:
            self.server.requests["denied_proxy_requests"] += 1
        self.reply(403)

    do_CONNECT = deny

    def do_GET(self):
        site = self.server
        target = urlsplit(self.path)
        origin = f"{target.scheme}://{target.netloc}"
        if origin not in (site.origin, site.other_origin):
            self.deny()
        elif target.path in (TARGET, OTHER_PATH) and target.query:
            with site.lock:
                site.requests[self.path] += 1
            self.reply(200, PAGE)
        elif target.path == "/observation":
            values = parse_qs(target.query, keep_blank_values=True)
            probe, marker, worker = (values.get(key, [""])[0] for key in ("probe", "marker", "worker"))
            with site.lock:
                site.observations.setdefault(probe, []).append(dict(origin=origin, marker=marker, worker=worker))
            self.reply(204)
        else:
            self.reply(404)


@contextmanager
def fixture():
    # Serve two synthetic origins through a fixed proxy that never forwards.
    with ThreadingHTTPServer(("127.0.0.1", 0), Proxy) as site, socket.socket() as other:
        other.bind(("127.0.0.1", 0))
        site.origin = f"http://127.0.0.1:{site.server_port}"
        site.other_origin = f"http://127.0.0.1:{other.getsockname()[1]}"
        site.lock, site.requests, site.observations = threading.Lock(), Counter(), {}
        thread = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.shutdown()
            thread.join()


def write_extension(folder, origin):
    folder.mkdir()
    exact = json.dumps(origin + TARGET)
    content, worker = "flit-synthetic-content.js", "flit-synthetic-worker.js"
    manifest = {
        "manifest_version": 3, "name": NAME, "version": "1.0",
        "content_scripts": [{
            "matches": [f"http://127.0.0.1{TARGET}*"],
            "include_globs": [origin + TARGET + "?*"], "js": [content], "run_at": "document_idle",
        }],
        "background": {"service_worker": worker},
    }
    (folder / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    (folder / content).write_text(f"""if (location.origin + location.pathname === {exact}) {{
  const node = document.createElement('strong');
  node.id = 'flit-synthetic-mv3-marker';
  node.textContent = '{MARKER}';
  document.body.append(node);
  chrome.runtime.sendMessage('flit-synthetic-probe').then(reply => {{
    if (reply === '{WORKER_REPLY}') node.dataset.worker = reply;
  }}).catch(() => {{}});
}}
""", encoding="utf-8")
    (folder / worker).write_text(f"""chrome.runtime.onMessage.addListener((request, sender, reply) => {{
  if (request === 'flit-synthetic-probe' && sender.url?.startsWith({exact} + '?')) reply('{WORKER_REPLY}');
}});
""", encoding="utf-8")


def probe(browser, report, label, *, expected=False, origin=None, path=TARGET):
    report["stage"] = label
    site = browser.site
    origin = origin or site.origin
    url = f"{origin}{path}?{label}"
    browser.edit_address(url)
    browser.click(GO)
    browser.wait(f"{label} fixture committed", lambda: browser.address() == url and browser.has_page_title(TITLE))

    def observed():
        with site.lock:
            rows = site.observations.get(label, ())
            if expected:
                return dict(origin=origin, marker=MARKER, worker=WORKER_REPLY) in rows
            require(not any(row["marker"] or row["worker"] for row in rows),
                    f"{label}: unexpected content script or worker response: {rows}")
            return any(row["origin"] == origin for row in rows)

    browser.wait(f"{label} content/worker observation", observed)
    if not expected:
        deadline = time.monotonic() + .75
        while time.monotonic() < deadline:
            browser.process.alive()
            observed()
            time.sleep(.05)
        observed()
    report["checks"][label] = "signals present" if expected else "signals absent"


UIA = r"""
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$target)
if (!$root -or $root.Current.ProcessId -ne $ownerPid) { throw 'UIA root is not owned' }
$items = $root.FindAll([Windows.Automation.TreeScope]::Descendants,
                       [Windows.Automation.Condition]::TrueCondition)
if ($confirmTitle) {
    $matchedPrompt = $root.Current.Name -ceq $confirmTitle
    $buttons = @(foreach ($item in $items) {
        try {
            $v = $item.Current
            if ($v.ProcessId -ne $ownerPid -or $v.IsOffscreen) { continue }
            if ($v.Name -ceq $confirmTitle) { $matchedPrompt = $true }
            if ($v.ControlType -eq [Windows.Automation.ControlType]::Button -and
                $v.Name -ceq 'Remove' -and $v.IsEnabled) { $item }
        } catch [Windows.Automation.ElementNotAvailableException] {}
    })
    if (!$matchedPrompt -or $buttons.Count -ne 1) {
        throw 'Expected the synthetic extension confirmation with a unique Remove button'
    }
    $button = $buttons[0]
    $pattern = $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
    if ($root.Current.ProcessId -ne $ownerPid -or $button.Current.ProcessId -ne $ownerPid) {
        throw 'UIA ownership changed before invocation'
    }
    $pattern.Invoke()
    '{"action":"UIA InvokePattern"}'
} else {
    $rows = foreach ($item in $items) {
        try {
            $v = $item.Current
            if ($v.ProcessId -ne $ownerPid -or $v.IsOffscreen) { continue }
            $rect = $v.BoundingRectangle
            @{
                name = $v.Name; id = $v.AutomationId; enabled = $v.IsEnabled
                type = $v.ControlType.ProgrammaticName
                rect = if (!$rect.IsEmpty) { @($rect.X, $rect.Y, $rect.Width, $rect.Height) }
            }
        } catch [Windows.Automation.ElementNotAvailableException] {}
    }
    ConvertTo-Json -Compress -Depth 4 -InputObject @($rows)
}
"""


def uia(browser, hwnd, *, confirm=False):
    process, win = browser.process, browser.win
    process.alive()
    require(win.pid(hwnd) == process.pid and win.visible(hwnd), "UIA root is not a visible owned HWND")
    title = f"$confirmTitle = '{REMOVE_TITLE}';" if confirm else "$confirmTitle = $null;"
    command = f"$target = [long]{hwnd}; $ownerPid = {process.pid}; " + title + UIA
    result = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand",
         b64encode(command.encode("utf-16le")).decode("ascii")],
        stdin=subprocess.DEVNULL, capture_output=True, encoding="utf-8", errors="replace",
        timeout=10, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    require(result.returncode == 0, f"UIA {'Remove invocation' if confirm else 'snapshot'} failed: {result.stderr.strip()}")
    return json.loads(result.stdout)


def web_controls(browser):
    require(browser.address() == EXTENSIONS, "The active tab is not chrome://extensions/")
    return uia(browser, browser.hwnd)


def button(rows, control_id):
    matches = [row for row in rows if row["id"] == control_id
               and row["type"] == "ControlType.Button" and row["enabled"]]
    require(len(matches) <= 1, f"Ambiguous Extensions control {control_id}")
    return next(iter(matches), None)


def fixture_card(rows):
    names = [row["name"] for row in rows if row["id"] == "name" and row["type"] == "ControlType.Text"]
    require(not names or names == [NAME], f"Expected only our synthetic extension card; found {names}")
    return bool(names)


def card_button(browser, control_id, *, state=None):
    def find():
        rows = web_controls(browser)
        if fixture_card(rows):
            item = button(rows, control_id)
            if item and (state is None or item["name"].split(",", 1)[0] == state):
                return item

    return browser.wait(f"synthetic card {control_id} {state or ''}".strip(), find)


def click_web(browser, control):
    win, process = browser.win, browser.process
    require(browser.address() == EXTENSIONS, "Extensions tab changed before WebUI click")
    panel = browser.wait("owned Extensions page panel", browser.page_panel)
    user = C.WinDLL("user32", use_last_error=True)
    is_child = api(user, "IsChild", W.BOOL, W.HWND, W.HWND)
    require(win.pid(browser.hwnd) == win.pid(panel) == process.pid and is_child(browser.hwnd, panel),
            "Extensions panel is not an owned descendant of the Flit window")

    def widgets(parent):
        for child in win.visible_children(parent):
            if win.class_name(child) == "Chrome_RenderWidgetHostHWND":
                yield child
            else:
                yield from widgets(child)

    matches = list(widgets(panel))
    require(len(matches) == 1, f"Expected one visible Extensions render widget, got {matches}")
    widget = matches[0]
    require(win.pid(widget) == process.pid and is_child(panel, widget),
            "Extensions render widget is not an owned descendant of the active panel")
    bounds = control["rect"]
    require(isinstance(bounds, list) and len(bounds) == 4, f"No UIA bounds for {control['id']}")
    x, y, width, height = bounds
    require(width > 0 and height > 0, f"Empty UIA bounds for {control['id']}")
    point = W.POINT(round(x + width / 2), round(y + height / 2))
    screen_to_client = api(user, "ScreenToClient", W.BOOL, W.HWND, C.POINTER(W.POINT))
    with win.window_coordinates(widget):
        win.check(screen_to_client(widget, C.byref(point)))
        left, top, right, bottom = win.rect(widget, client=True)
        require(left <= point.x < right and top <= point.y < bottom, f"{control['id']} is outside its owned widget")
        require(is_child(browser.hwnd, panel) and is_child(panel, widget),
                "Extensions widget ancestry changed before click")
        packed = point.x | (point.y << 16)
        post(process, widget, 0x0201, 1, packed)  # WM_LBUTTONDOWN, MK_LBUTTON
        post(process, widget, 0x0202, 0, packed)  # WM_LBUTTONUP


def open_extensions(browser):
    send(browser, 0x0111, 123)  # WM_COMMAND, Flit Extensions menu item.
    browser.wait("chrome://extensions committed",
                 lambda: browser.address() == EXTENSIONS
                 and any(browser.has_page_title(name) for name in ("Extensions", "확장 프로그램")))
    return browser.selected()


def root_owner(browser, hwnd):
    require(browser.win.pid(hwnd) == browser.process.pid, "Dialog is not owned by the Flit process")
    ancestor = api(C.WinDLL("user32", use_last_error=True), "GetAncestor", W.HWND, W.HWND, W.UINT)
    return ancestor(hwnd, 3)  # GA_ROOTOWNER


def install(browser, folder, report):
    report["stage"] = "extensions WebUI"
    tab = open_extensions(browser)
    report["extensions"] = {"url": browser.address(), "title": browser.title(), "tab": tab}

    def developer_controls():
        rows = web_controls(browser)
        return rows if button(rows, "devMode") else None

    rows = browser.wait("Extensions Developer mode control", developer_controls)
    require(not fixture_card(rows), "A fresh profile should not contain the synthetic extension")
    report["stage"] = "enable developer mode"
    if not button(rows, "loadUnpacked"):
        click_web(browser, button(rows, "devMode"))
    load = browser.wait("Load unpacked button", lambda: button(web_controls(browser), "loadUnpacked"))
    report["stage"] = "click Load unpacked"
    click_web(browser, load)
    win, process = browser.win, browser.process
    report["stage"] = "wait for folder picker controls"
    picker = report["extensions"]["picker"] = {}

    def ready():
        dialogs = owned_windows(process, "#32770")
        require(len(dialogs) <= 1, f"Expected one owned folder picker, got {dialogs}")
        if not dialogs:
            return None
        hwnd = dialogs[0]
        owner = root_owner(browser, hwnd)
        require(owner == browser.hwnd, "Folder picker has another root owner")
        entry, choose = win.get_item(hwnd, 1152), win.get_item(hwnd, 1)
        picker.update(hwnd=hwnd, root_owner=owner, edit=entry, select=choose)
        if (entry and choose and win.pid(entry) == win.pid(choose) == process.pid
                and win.class_name(entry) == "Edit" and win.class_name(choose) == "Button"
                and win.visible(entry) and win.enabled(entry) and win.visible(choose)):
            return hwnd, entry, choose

    hwnd, entry, choose = browser.wait("owned folder picker with Edit 1152 and Select 1", ready)
    report["stage"] = "set and read back synthetic folder"
    require(win.pid(entry) == process.pid, "Folder picker edit is no longer owned")
    path = C.create_unicode_buffer(str(folder))
    require(win.message(entry, 0x000C, 0, C.addressof(path)), "Folder picker refused the synthetic path")
    readback = C.create_unicode_buffer(len(path))
    win.message(entry, 0x000D, len(readback), C.addressof(readback))
    picker["selected_folder"] = readback.value
    require(readback.value == str(folder), "Folder picker path readback did not match")
    report["stage"] = "confirm synthetic folder"
    browser.wait("owned folder picker Select enabled",
                 lambda: win.pid(choose) == process.pid and win.visible(choose) and win.enabled(choose))
    post(process, choose, 0x00F5)  # BM_CLICK on the owned Select button.
    picker["bm_click_posted"] = True
    browser.wait("folder picker closed after selecting the synthetic extension", lambda: not win.visible(hwnd))
    picker["closed"] = True
    report["stage"] = "verify installed extension card"
    card_button(browser, "enableToggle", state="On")
    report["checks"]["load_unpacked"] = "synthetic card enabled"
    return tab


def remove(browser, report):
    report["stage"] = "open remove confirmation"
    process, win = browser.process, browser.win
    before = set(owned_windows(process, "Chrome_WidgetWin_1"))
    click_web(browser, card_button(browser, "removeButton"))

    def new_dialog():
        candidates = [hwnd for hwnd in owned_windows(process, "Chrome_WidgetWin_1")
                      if hwnd not in before and root_owner(browser, hwnd) == browser.hwnd]
        require(len(candidates) <= 1, f"Ambiguous owned removal dialogs: {candidates}")
        return next(iter(candidates), None)

    hwnd = browser.wait("owned Chromium Views remove confirmation", new_dialog)
    report["remove"] = {"hwnd": hwnd, "root_owner": browser.hwnd, "class": win.class_name(hwnd)}
    report["stage"] = "identify remove confirmation"

    def prompt_ready():
        rows = uia(browser, hwnd)
        if any(row["name"] == REMOVE_TITLE for row in rows):
            return [row for row in rows if row["type"] == "ControlType.Button"
                    and row["name"] == "Remove" and row["enabled"]]

    buttons = browser.wait("UIA controls for the synthetic removal prompt", prompt_ready)
    require(len(buttons) == 1, "Ambiguous Remove button on the synthetic confirmation")
    report["stage"] = "invoke remove confirmation"
    require(root_owner(browser, hwnd) == browser.hwnd, "Remove confirmation owner changed")
    # Chromium Views handles its own button via UIA, as in the 0.7 release evidence.
    report["remove"].update(uia(browser, hwnd, confirm=True))
    browser.wait("Remove confirmation closed after UIA InvokePattern", lambda: not win.visible(hwnd))
    report["stage"] = "verify card removed"

    def gone():
        rows = web_controls(browser)
        return button(rows, "loadUnpacked") and not fixture_card(rows)

    browser.wait("synthetic extension card gone after Remove", gone)
    report["checks"]["remove_card"] = "synthetic card absent"


def run(exe, directory, site, report):
    profile = directory / "profile"
    folder = directory / "flit-synthetic-unpacked"
    seed_profile(profile, site.origin)
    write_extension(folder, site.origin)
    win = Win32()
    args = [f"--flit-profile={profile}"]

    def launch(name):
        report["stage"] = name
        return OwnedProcess(win, exe, args, report["jobs"].setdefault(name, {}))

    with launch("install_job") as process:
        browser = Browser(process, site)
        probe(browser, report, "before-install")
        page = browser.selected()
        extensions = install(browser, folder, report)
        require(page >= 0 and extensions != page, "Extensions did not open in a separate tab")
        select(browser, page)
        probe(browser, report, "after-install", expected=True)
        probe(browser, report, "other-port", origin=site.other_origin)
        probe(browser, report, "other-path", path=OTHER_PATH)
        for state, next_state, label, expected in (("On", "Off", "disabled", False),
                                                    ("Off", "On", "reenabled", True)):
            report["stage"] = f"set extension {next_state}"
            select(browser, extensions)
            click_web(browser, card_button(browser, "enableToggle", state=state))
            card_button(browser, "enableToggle", state=next_state)
            select(browser, page)
            probe(browser, report, label, expected=expected)
        report["stage"] = "close after install"
        process.normal_close(browser.hwnd)

    with launch("relaunch_job") as process:
        browser = Browser(process, site)
        probe(browser, report, "relaunch", expected=True)
        report["stage"] = "reopen extensions WebUI"
        open_extensions(browser)
        remove(browser, report)
        probe(browser, report, "removed")
        report["stage"] = "close after remove"
        process.normal_close(browser.hwnd)

    with launch("post_removal_job") as process:
        browser = Browser(process, site)
        probe(browser, report, "relaunch-after-removal")
        report["stage"] = "close after removal relaunch"
        process.normal_close(browser.hwnd)


def digest(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll and libcef.dll")
    args = parser.parse_args()
    exe = args.exe.resolve()
    files = (exe, exe.with_name("Flit.dll"), exe.with_name("libcef.dll"))
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not all(path.is_file() for path in files):
        parser.error("requires Windows and a staged Flit.exe beside Flit.dll and libcef.dll")

    report = {"result": "fail", "exe": str(exe), "stage": "setup", "checks": {}, "jobs": {}}
    try:
        report["hashes"] = {path.name: digest(path) for path in files}
        scratch = tempfile.TemporaryDirectory(prefix="flit-synthetic-mv3-")
        try:
            with fixture() as site:
                report.update(scratch=scratch.name, origin=site.origin, other_origin=site.other_origin)
                try:
                    run(exe, Path(scratch.name), site, report)
                finally:
                    with site.lock:
                        report.update(observations=dict(site.observations), requests=dict(site.requests))
        finally:
            clean_scratch(scratch, report)
        report["stage"] = "scratch cleanup"
        require("scratch_cleanup_error" not in report, "Could not remove the disposable profile")
        report.update(result="pass", stage="complete")
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
        report["traceback"] = traceback.format_exc()
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
