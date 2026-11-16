"""Check native Flit attachment downloads on Windows using only a loopback fixture."""

import argparse
from collections import Counter
from contextlib import closing, contextmanager
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import time

sys.dont_write_bytecode = True
from windows_smoke import Browser, OwnedProcess, Win32, require


TITLE = "Flit download fixture"
PAGE = f'''<!doctype html><meta charset="utf-8"><title>{TITLE}</title>
<style>body {{ margin:0; height:100vh; display:grid; grid-template-rows:1fr 1fr; }}
a {{ display:grid; place-items:center; }}</style>
<a href="/first">Download first attachment</a><a href="/second">Download second attachment</a>
<script>addEventListener("pageshow", () => fetch("/ready"));</script>'''.encode("ascii")
ATTACHMENTS = (
    ("/first", "flit-fixture.txt", b"First Flit attachment\n" * 256),
    ("/second", "flit-fixture (1).txt", b"Second Flit attachment\n" * 256),
)
PAYLOADS = {route: data for route, _, data in ATTACHMENTS}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reject(self):
        self.server.hits["rejected_proxy_requests"] += 1
        self.send_error(403)

    do_CONNECT = reject

    def do_GET(self):
        # Only proxy-form requests for this exact loopback origin are served.
        prefix = self.server.origin + "/"
        if not self.path.startswith(prefix):
            self.reject()
            return
        route = self.path[len(prefix) - 1:]
        if route not in ("/", "/ready", "/favicon.ico", *PAYLOADS):
            self.send_error(404)
            return
        self.server.hits[route] += 1
        attachment = route in PAYLOADS
        body = PAGE if route == "/" else PAYLOADS.get(route, b"")
        self.send_response(200 if body else 204)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", "application/octet-stream" if attachment else "text/html; charset=utf-8")
        if attachment:
            self.send_header("Content-Disposition", f'attachment; filename="{ATTACHMENTS[0][1]}"')
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)


@contextmanager
def fixture():
    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as site:
        site.origin = f"http://127.0.0.1:{site.server_port}"
        site.hits = Counter()
        thread = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.shutdown()
            thread.join()


def seed_profile(profile, downloads, origin):
    default = profile / "Default"
    default.mkdir(parents=True)
    (profile / ".flit-browser-profile").write_bytes(b"Flit Browser profile v1\n")
    proxy = {"mode": "fixed_servers", "server": origin, "bypass_list": "<-loopback>"}
    (profile / "Local State").write_text(json.dumps({
        "proxy": proxy, "dns_over_https": {"mode": "off"},
    }), encoding="utf-8")
    (default / "Preferences").write_text(json.dumps({
        "proxy": proxy, "net": {"network_prediction_options": 2}, "safebrowsing": {"enabled": False},
        "download": {"default_directory": str(downloads), "directory_upgrade": True, "prompt_for_download": False},
    }), encoding="utf-8")


def click_attachment(browser, index):
    win, process = browser.win, browser.process

    def widgets(parent):
        for child in win.visible_children(parent):
            if win.class_name(child) == "Chrome_RenderWidgetHostHWND":
                yield child
            else:
                yield from widgets(child)

    panel = browser.wait("visible Flit page panel", browser.page_panel)
    matches = list(widgets(panel))
    require(len(matches) == 1, f"Expected one visible Chromium render widget, got {len(matches)}")
    widget = matches[0]
    with win.window_coordinates(widget):
        _, _, width, height = win.rect(widget, client=True)
        require(width > 0 and height > 0, "Flit render widget has no client area")
        point = (width // 2) | (height * (2 * index + 1) // 4 << 16)
        # Post directly to the owned Chromium HWND; no global mouse or focus changes.
        for message, buttons in ((0x0201, 1), (0x0202, 0)):  # WM_LBUTTONDOWN / WM_LBUTTONUP
            require(win.pid(widget) in process.pids(), "Flit render widget is no longer owned")
            win.check(win.post(widget, message, buttons, point))


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_history(profile, downloads, origin):
    history = profile / "Default" / "History"
    with closing(sqlite3.connect(history.as_uri() + "?mode=ro", uri=True)) as db:
        rows = db.execute('''
            SELECT u.url, d.current_path, d.target_path, d.state, d.interrupt_reason,
                   d.received_bytes, d.total_bytes, d.end_time
            FROM downloads d JOIN downloads_url_chains u ON u.id = d.id AND u.chain_index = 0
            ORDER BY u.url
        ''').fetchall()
    require(len(rows) == len(ATTACHMENTS), f"Expected two download history rows, got {rows!r}")
    for row, (route, name, data) in zip(rows, ATTACHMENTS):
        url, current, target, state, reason, received, total, end = row
        path = downloads / name
        require(url == origin + route and Path(current) == Path(target) == path,
                f"Unexpected download history URL or path: {row!r}")
        # Chromium persists COMPLETE as state 1 and NONE as interrupt reason 0.
        require(state == 1 and reason == 0 and received == total == len(data) and end > 0,
                f"Download history did not record completion: {row!r}")
    return len(rows)


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="flit-downloads-", dir=work))
    profile, downloads = scratch / "profile", scratch / "attachments"
    downloads.mkdir()
    report.update(work=str(scratch), launch={}, files={})
    with fixture() as site:
        seed_profile(profile, downloads, site.origin)
        args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/"]
        try:
            with OwnedProcess(Win32(), exe, args, report["launch"]) as process:
                browser = Browser(process, site)
                browser.wait("attachment page loaded through loopback proxy",
                             lambda: site.hits["/ready"] and browser.has_page_title(TITLE))
                require(not any(site.hits[route] for route in PAYLOADS), "An attachment was requested before clicking")
                for index, (route, name, data) in enumerate(ATTACHMENTS):
                    click_attachment(browser, index)
                    path = downloads / name
                    browser.wait(f"{name} after native click", path.is_file)
                    require(site.hits[route] == 1 and sha256(path) == hashlib.sha256(data).hexdigest(),
                            f"Unexpected request count or bytes for {name}")
                # Chromium renames before Windows quarantine completes; verify completion after clean exit.
                time.sleep(2)
                process.normal_close(browser.hwnd)
        finally:
            report["http_requests"] = dict(site.hits)

        names = sorted(p.name for p in downloads.iterdir())
        require(set(names) == {name for _, name, _ in ATTACHMENTS},
                f"Unexpected files or incomplete download in isolated directory: {names!r}")
        for _, name, data in ATTACHMENTS:
            digest = sha256(downloads / name)
            require(digest == hashlib.sha256(data).hexdigest(), f"{name} was changed or overwritten")
            report["files"][name] = {"bytes": len(data), "sha256": digest}
        report["completed_history_rows"] = check_history(profile, downloads, site.origin)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe")
    parser.add_argument("--work", required=True, type=Path, help="parent directory for a fresh test profile and downloads")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    exe = args.exe.resolve()
    dll = exe.with_name("Flit.dll")
    if exe.name.lower() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("--exe must point to Flit.exe next to Flit.dll")
    report = {"result": "fail", "exe": str(exe)}
    try:
        report["sha256"] = {path.name: sha256(path) for path in (exe, dll)}
        try:
            run(exe, args.work.resolve(), report)
        finally:
            report["sha256_after"] = {path.name: sha256(path) for path in (exe, dll)}
        require(report["sha256"] == report["sha256_after"], "Staged EXE or DLL changed during the test; rerun against a stable payload")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
