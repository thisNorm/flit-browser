"""Check Flit's native Stop/Reload button against a held loopback script."""

import argparse
from collections import Counter
from contextlib import contextmanager
from hashlib import file_digest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sys
import tempfile
from threading import Event, Lock, Thread

sys.dont_write_bytecode = True
from windows_shortcuts import OwnedKeyboard
from windows_smoke import GO, RELOAD, WORKSPACE, Browser, OwnedProcess, Win32, require
from windows_windows import seed_profile

RELOADED = "Flit reload completed"


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        site = self.server
        if not self.path.startswith(site.origin + "/"):
            return self.send_error(403)  # Seeded loopback proxy; never forwards.
        path = self.path[len(site.origin):]
        gate = site.gates.get(path)
        if gate:
            body, kind = b"/* held script */", "application/javascript"
        elif path + ".js" in site.gates:
            with site.lock:
                site.visits[path] += 1
                first = site.visits[path] == 1
            markup = (f'<script async src="{path}.js"></script>' if first
                      else f"""<body onload='document.title="{RELOADED}"'>""")
            body, kind = f"<!doctype html><title>Flit {path}</title>{markup}".encode(), "text/html"
        else:
            return self.send_error(404)

        self.send_response(200)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")  # Don't let Chromium drain for keep-alive reuse.
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body[:1] if gate else body)
        if not gate:
            return
        started, cancelled = gate
        started.set()
        self.connection.settimeout(45)
        try:
            if self.connection.recv(1):
                return
        except TimeoutError:
            return  # Server timeout is not evidence of client cancellation.
        except (ConnectionAbortedError, ConnectionResetError):
            pass
        cancelled.set()


@contextmanager
def fixture():
    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as site:
        site.origin = f"http://127.0.0.1:{site.server_port}"
        site.gates = {path: (Event(), Event()) for path in ("/stop.js", "/escape.js", "/shortcut.js")}
        site.visits, site.lock = Counter(), Lock()
        thread = Thread(target=site.serve_forever, args=(0.05,), daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.shutdown()
            thread.join()


def cancel(browser, path, action):
    site = browser.site
    started, cancelled = site.gates[path + ".js"]
    url, title = site.origin + path, "Flit " + path
    browser.wait(f"{path} committed with its subresource stalled",
                 lambda: started.is_set() and browser.has_page_title(title) and browser.address() == url)
    wait_button(browser, "Stop")
    require(site.visits[path] == 1 and not cancelled.is_set(), f"{path} was not still on its first held load")
    action()
    browser.wait(f"{path} peer disconnect while its response is incomplete", cancelled.is_set, 8)
    wait_button(browser, "Reload")
    require(site.visits[path] == 1 and browser.has_page_title(title) and browser.address() == url,
            f"Stopping {path} unexpectedly reloaded or left the page")


def wait_button(browser, name):
    browser.wait(f"{name} on the visible native button", lambda: (
        browser.win.visible(browser.controls[RELOAD])
        and browser.control_text(RELOAD).split()[:1] == [name]  # Win32 name source (WM_GETTEXT).
    ), 6)


def escape_page(browser, keyboard):
    panel = browser.wait("visible CEF panel", browser.page_panel)
    browser.wait("owned CEF page keyboard focus", lambda: keyboard.focused(panel, descendant=True), 5)
    keyboard.press("Escape from CEF", 0x1B, ctrl=False, panel=panel)


def run(exe, work, report, escape=False):
    win = Win32()
    keyboard = OwnedKeyboard(win) if escape else None
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-stop-", dir=work)) / "profile"
    report["launch"] = {}
    with fixture() as site:
        seed_profile(profile, site.origin)
        launch = [f"--flit-profile={profile}", f"--flit-url={site.origin}/stop"]
        with OwnedProcess(win, exe, launch, report["launch"]) as process:
            browser = Browser(process, site)
            cancel(browser, "/stop", lambda: browser.click(RELOAD))
            report["stop_closed_incomplete_response"] = True
            browser.click(RELOAD)
            browser.wait("fresh /stop request and DOM load after Reload", lambda: (
                site.visits["/stop"] == 2 and browser.has_page_title(RELOADED)
                and browser.address() == site.origin + "/stop"
            ))
            wait_button(browser, "Reload")
            report["reload_requests"] = site.visits["/stop"]
            browser.edit_address(site.origin + "/shortcut")
            browser.click(GO)
            browser.wait("shortcut test starts a held request", site.gates["/shortcut.js"][0].is_set)
            wait_button(browser, "Stop")
            browser.press(RELOAD, 0x74)  # VK_F5 stays Reload while the toolbar displays Stop.
            browser.wait("F5 reloads the held page", lambda: (
                site.visits["/shortcut"] == 2 and browser.has_page_title(RELOADED)
            ))
            report["f5_reloads_while_stop_visible"] = True
            if keyboard:
                try:
                    keyboard.activate(browser)
                    browser.edit_address(site.origin + "/escape")
                    browser.click(GO)  # Go gives focus to CEF.
                    cancel(browser, "/escape", lambda: escape_page(browser, keyboard))
                    report["page_escape_closed_incomplete_response"] = True
                finally:
                    keyboard.restore(browser)
            else:
                browser.edit_address(site.origin + "/escape")
                browser.click(GO)
                cancel(browser, "/escape", lambda: browser.press(RELOAD, 0x1B))
                report["toolbar_escape_closed_incomplete_response"] = True
            process.normal_close(browser.hwnd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe")
    parser.add_argument("--work", type=Path, required=True, help="workspace directory for test profiles")
    parser.add_argument("--escape", action="store_true", help="also test Escape on an idle, unlocked desktop")
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    if (sys.platform != "win32" or exe.name.lower() != "flit.exe"
            or not exe.is_file() or not exe.with_name("Flit.dll").is_file()):
        parser.error("requires a staged Windows Flit.exe next to Flit.dll")
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    with exe.with_name("Flit.dll").open("rb") as dll:
        report = {"result": "fail", "dll_sha256": file_digest(dll, "sha256").hexdigest()}
    try:
        run(exe, work, report, args.escape)
        report["result"] = "pass"
    except (OSError, RuntimeError) as error:
        report["error"] = str(error)
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
