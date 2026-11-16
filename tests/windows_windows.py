"""Exercise normal and private Flit windows through owned Win32 controls and loopback."""

import argparse
from collections import Counter
from contextlib import closing, contextmanager
from hashlib import file_digest
from http.cookies import SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import secrets
import sqlite3
import subprocess
import sys
import tempfile
import threading
from urllib.parse import unquote, urlsplit

sys.dont_write_bytecode = True
from windows_session import loaded_panels
from windows_smoke import Browser, GO, NEW_TAB, OwnedProcess, WORKSPACE, Win32, require


SCRATCH = WORKSPACE / "work" / "flit-0.6.0" / "windows-test"
WINDOW_CLASS = "FlitBrowserWindow"
NORMAL_TITLE, PRIVATE_TITLE = "Flit Browser", "Flit Browser — Private"
NEW_WINDOW, PRIVATE_WINDOW, CLOSE_TAB = 161, 162, 106
WM_COMMAND, WM_CLOSE = 0x0111, 0x0010
COOKIES = {"normal": "flit_windows_normal", "private": "flit_windows_private"}
SEEDS = {"normal-seed", "private-seed"}
PAGES = (
    "normal-seed", "normal-peer", "private-empty", "private-seed", "private-peer",
    "normal-after-private", "private-survivor", "private-reset", "normal-restart",
)

PROBE = """<script type=module>
const marker = %s, key = 'flit-windows-storage';
const route = location.pathname;
let observation;
try {
  if (marker !== null) {
    localStorage.setItem(key, marker);
    const asset = await fetch(route + '/asset');
    if (!asset.ok || (await asset.text()) !== marker) throw Error('fixture cacheable asset failed');
  }
  observation = {storage: localStorage.getItem(key)};
} catch (error) {
  observation = {error: String(error)};
}
await fetch(route + '/ready?' + encodeURIComponent(JSON.stringify(observation)), {cache: 'no-store'});
</script>"""


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reject(self):
        with self.server.lock:
            self.server.transport["rejected"] += 1
        try:
            self.send_response(403)
            self.send_header("Content-Length", "0")
            self.end_headers()
        except ConnectionError:
            pass  # Chromium can abandon background requests as its window closes.

    do_CONNECT = reject

    def do_GET(self):
        site = self.server
        target = urlsplit(self.path)
        if (target.scheme, target.netloc) != ("http", site.authority):
            self.reject()
            return

        path = target.path
        page = site.routes.get(path)
        parent, _, suffix = path.rpartition("/")
        content_type, cache, cookie = "text/plain", "no-store", None
        if page and not target.query:
            mode = page.split("-")[0]
            marker = site.markers[mode] if page in SEEDS else None
            if marker:
                cookie = f"{COOKIES[mode]}={marker}; Max-Age=86400; Path=/; HttpOnly; SameSite=Lax"
            body = (f'<!doctype html><meta charset=utf-8><title>{site.title(page)}</title>'
                    f'<link rel=icon href="data:,"><h1>{site.title(page)}</h1>'
                    + PROBE % json.dumps(marker)).encode("ascii")
            content_type = "text/html; charset=utf-8"
            site.observe(path, self.headers)
        elif suffix == "ready" and parent in site.routes:
            try:
                observation = json.loads(unquote(target.query))
                if not isinstance(observation, dict) or set(observation) not in ({"storage"}, {"error"}):
                    raise ValueError("unexpected observation")
            except ValueError:
                self.send_error(400)
                return
            site.observe(path, self.headers, **observation)
            body = b""
        elif suffix == "asset" and site.routes.get(parent) in SEEDS:
            body = site.markers[site.routes[parent].split("-")[0]].encode("ascii")
            cache = "public, max-age=600"
            site.observe(path, self.headers)
        elif path == "/favicon.ico":
            body = b""
        else:
            self.send_error(404)
            return

        self.send_response(200 if body else 204)
        self.send_header("Content-Type", content_type)
        self.send_header("Cache-Control", cache)
        if cookie:
            self.send_header("Set-Cookie", cookie)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)


class Site(ThreadingHTTPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.authority = f"127.0.0.1:{self.server_port}"
        self.origin = f"http://{self.authority}"
        self.markers = {mode: f"flit-{mode}-{secrets.token_hex(12)}" for mode in COOKIES}
        self.paths = {page: f"/{self.markers[page.split('-')[0]]}/{page}" for page in PAGES}
        self.routes = {path: page for page, path in self.paths.items()}
        self.lock = threading.Lock()
        self.requests = {}
        self.transport = Counter()

    @staticmethod
    def title(page):
        return f"Flit windows fixture: {page}"

    def get_request(self):
        conn, address = super().get_request()
        conn.settimeout(3)
        return conn, address

    def observe(self, path, headers, **probe):
        incoming = SimpleCookie()
        for cookie in headers.get_all("Cookie", []):
            incoming.load(cookie)
        values = {mode: incoming[name].value for mode, name in COOKIES.items() if name in incoming}
        with self.lock:
            self.transport["proxy"] += 1
            self.requests.setdefault(path, []).append({"cookies": values, **probe})

    def seen(self, path):
        with self.lock:
            return tuple(self.requests.get(path, ()))


@contextmanager
def fixture():
    with Site() as site:
        worker = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
        worker.start()
        try:
            yield site
        finally:
            site.shutdown()
            worker.join(2)


def seed_profile(profile, origin):
    # Match the downloads test: fixed loopback proxy with no implicit bypass; never forward.
    default = profile / "Default"
    default.mkdir(parents=True)
    (profile / ".flit-browser-profile").write_bytes(b"Flit Browser profile v1\n")
    proxy = {"mode": "fixed_servers", "server": origin, "bypass_list": "<-loopback>"}
    (profile / "Local State").write_text(json.dumps({"proxy": proxy, "dns_over_https": {"mode": "off"}}),
                                         encoding="utf-8")
    (default / "Preferences").write_text(json.dumps({"proxy": proxy, "net": {"network_prediction_options": 2}}),
                                        encoding="utf-8")


class Window(Browser):
    def __init__(self, process, site, hwnd=None):
        self.target = hwnd
        super().__init__(process, site)

    @staticmethod
    def handles(process):
        win = process.win
        found = set()

        @win.callback
        def visit(hwnd, _):
            if win.pid(hwnd) == process.pid and win.visible(hwnd) and win.class_name(hwnd) == WINDOW_CLASS:
                found.add(hwnd)
            return True

        win.check(win.enum(visit, 0))
        return found

    def find_window(self):
        handles = self.handles(self.process)
        if self.target is not None:
            return self.target if self.target in handles else None
        return next(iter(handles)) if len(handles) == 1 else None

    def owned(self):
        self.process.alive()
        require(self.win.pid(self.hwnd) == self.process.pid and self.win.class_name(self.hwnd) == WINDOW_CLASS,
                f"Flit no longer owns HWND 0x{self.hwnd:x}")

    def title(self):
        self.owned()
        return super().title()

    def command(self, cid):
        self.owned()
        self.win.message(self.hwnd, WM_COMMAND, cid, 0)

    def blank(self, private=False, tabs=1, *, all_blank=True):
        caption = PRIVATE_TITLE if private else NORMAL_TITLE
        self.wait(f"{caption!r} blank tab {tabs}", lambda: (
            self.title() == caption and not self.address()
            and self.tab_count() == tabs and self.selected() == tabs - 1
        ))
        require(not self.page_panel(), f"Blank tab in {caption!r} has a visible CEF child")
        if all_blank:
            require(loaded_panels(self) == 0, f"Blank {caption!r} window has an attached CEF child")

    def open_window(self, cid, label, report):
        before = self.handles(self.process)
        self.command(cid)

        def added():
            current = self.handles(self.process)
            require(before <= current, f"An existing Flit HWND disappeared when opening {label}")
            new = current - before
            require(len(new) <= 1, f"WM_COMMAND {cid} opened multiple Flit HWNDs: {sorted(new)}")
            return next(iter(new), None)

        hwnd = self.wait(f"new owned {label} HWND after WM_COMMAND {cid}", added, 8)
        window = Window(self.process, self.site, hwnd)
        report[label] = {"hwnd": hwnd, "pid": self.win.pid(hwnd), "command": cid}
        return window

    def close_window(self):
        self.owned()
        before = self.handles(self.process) - {self.hwnd}
        require(before, "Use normal_close for the final owned window")
        self.win.check(self.win.post(self.hwnd, WM_CLOSE, 0, 0))
        self.wait(f"destruction of HWND 0x{self.hwnd:x} after WM_CLOSE",
                  lambda: self.win.pid(self.hwnd) != self.process.pid, 15)
        require(self.handles(self.process) == before, "Closing a window affected another Flit HWND")


def visit(window, page, mode, checks, *, start=False):
    site = window.site
    path = site.paths[page]
    private = page.startswith("private-")
    caption = f"{PRIVATE_TITLE if private else NORMAL_TITLE} — {site.title(page)}"
    if not start:
        require(not site.seen(path), f"Fixture page {page} loaded before its target HWND was instructed")
        window.owned()
        window.edit_address(site.origin + path)
        window.click(GO)
    window.wait(f"{page} document, JS report and caption in HWND 0x{window.hwnd:x}", lambda: (
        site.seen(path + "/ready") and window.title() == caption and window.address() == site.origin + path
    ))
    panel = window.wait(f"{page} CEF child attached to its HWND", window.page_panel)
    require(window.win.pid(panel) == window.process.pid, f"Page panel for {page} is not owned")
    value = site.markers[mode] if mode else None
    expected = {mode: value} if mode else {}
    # The seed's navigation must precede its own Set-Cookie; its JS callback must carry it.
    expected_navigation = {} if page in SEEDS else expected
    navigation, probes = site.seen(path), site.seen(path + "/ready")
    checks[page] = {"hwnd": window.hwnd, "title": caption, "navigation": navigation, "probes": probes}
    require(navigation and all(item == {"cookies": expected_navigation} for item in navigation),
            f"{page}: expected navigation cookies {expected_navigation}, got {navigation}")
    require(all(item == {"cookies": expected, "storage": value} for item in probes),
            f"{page}: expected JS storage {value!r} and cookies {expected}, got {probes}")


def renderer_pids(process):
    # A failed/incomplete CIM lookup cannot prove there was no renderer.
    for _ in range(2):
        pids = process.pids()
        require(process.pid in pids, "Flit root is not in the owned job")
        where = " OR ".join(f"ProcessId={pid}" for pid in pids)
        query = (f"$p = Get-CimInstance Win32_Process -Filter '{where}' -ErrorAction Stop; "
                 r"foreach ($item in $p) { $kind = if (!$item.CommandLine) {'unknown'} "
                 r"elseif ($item.CommandLine -match '(^|\s)--type=renderer(\s|$)') {'renderer'} "
                 r"else {'other'}; '{0}:{1}' -f $item.ProcessId, $kind }")
        result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", query],
                                stdin=subprocess.DEVNULL, capture_output=True, text=True,
                                creationflags=subprocess.CREATE_NO_WINDOW, timeout=10)
        require(result.returncode == 0, f"Owned renderer lookup failed: {result.stderr.strip()}")
        roles = {int(pid): kind for pid, kind in (line.split(":", 1) for line in result.stdout.splitlines())}
        current = set(process.pids())
        if current <= roles.keys():
            require(process.pid in current and all(roles[pid] in {"other", "renderer"} for pid in current),
                    "An owned process command line was unavailable")
            return sorted(pid for pid in current if roles[pid] == "renderer")
    raise RuntimeError("Owned processes changed during renderer inspection")


def inspect_profile(profile, site, report):
    default = profile / "Default"
    session = json.loads((profile / "session.json").read_text(encoding="utf-8"))
    report["session_urls"] = [entry["url"] for entry in session["entries"]]
    normal_url = site.origin + site.paths["normal-after-private"]
    require(report["session_urls"] == [normal_url], f"Unexpected restored session: {report['session_urls']}")

    def rows(path, sql, values):
        require(path.is_file(), f"Missing Chromium profile database: {path.name}")
        # Only read after clean exit; immutable mode creates no journal or sidecar.
        with closing(sqlite3.connect(path.resolve().as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
            return [row[0] for row in db.execute(sql, values)]

    normal_prefix, private_prefix = (f"{site.origin}/{site.markers[mode]}/%" for mode in COOKIES)
    history = report["fixture_history"] = rows(
        default / "History", "SELECT url FROM urls WHERE url LIKE ? OR url LIKE ? ORDER BY url",
        (normal_prefix, private_prefix),
    )
    require(normal_url in history, "Normal visit did not reach Chromium History")
    require(not any(url.startswith(private_prefix[:-1]) for url in history), f"Private visits in History: {history}")
    cookies = report["fixture_cookie_names"] = rows(
        default / "Network" / "Cookies", "SELECT name FROM cookies WHERE name IN (?, ?) ORDER BY name",
        tuple(COOKIES.values()),
    )
    require(cookies == [COOKIES["normal"]], f"Unexpected persisted Chromium cookies: {cookies}")

    # Also catch literal private URLs, cache/storage data, and temporary session traces in the scratch profile.
    markers = [value.encode(encoding) for value in (site.markers["private"], COOKIES["private"])
               for encoding in ("ascii", "utf-16-le")]
    leaks = report["files_with_private_markers"] = []
    for file in profile.rglob("*"):
        if not file.is_file() or file.is_symlink() or not file.resolve().is_relative_to(profile):
            continue
        with file.open("rb") as stream:
            tail = b""
            while chunk := stream.read(1024 * 1024):
                block = tail + chunk
                if any(marker in block for marker in markers):
                    leaks.append(str(file.relative_to(profile)))
                    break
                tail = block[-max(map(len, markers)):]
    require(not leaks, f"Private fixture markers written to the profile: {leaks}")


def run(exe, work, report):
    require(work.is_relative_to(SCRATCH), f"--work must be within {SCRATCH}")
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-windows-", dir=work)) / "profile"
    windows, checks = {}, {}
    report.update(profile=str(profile), windows=windows, pages=checks, launch={}, restart={})
    with fixture() as site:
        report["origin"] = site.origin
        seed_profile(profile, site.origin)
        args = [f"--flit-profile={profile}"]
        win = Win32()
        try:
            with OwnedProcess(win, exe, args, report["launch"]) as process:
                normal = Window(process, site)
                windows["normal"] = {"hwnd": normal.hwnd, "pid": win.pid(normal.hwnd)}
                normal.blank()
                peer = normal.open_window(NEW_WINDOW, "normal_peer", windows)
                peer.blank()
                private = normal.open_window(PRIVATE_WINDOW, "private", windows)
                private.blank(private=True)
                private.command(NEW_TAB)
                private.blank(private=True, tabs=2)
                blank_pids = report["blank_renderer_pids"] = renderer_pids(process)
                require(not blank_pids, f"Renderer started before any blank tab navigated: {blank_pids}")
                private.command(CLOSE_TAB)
                private.blank(private=True)
                require(not site.requests, "Loopback fixture received page requests during the blank-tab checks")

                visit(normal, "normal-seed", "normal", checks)
                loaded_pids = report["loaded_renderer_pids"] = renderer_pids(process)
                require(loaded_pids, "Renderer lookup found no renderer for the loaded normal fixture")
                visit(peer, "normal-peer", "normal", checks)
                visit(private, "private-empty", None, checks)
                visit(private, "private-seed", "private", checks)
                private_peer = private.open_window(PRIVATE_WINDOW, "private_peer", windows)
                private_peer.blank(private=True)
                visit(private_peer, "private-peer", "private", checks)

                peer.close_window()
                visit(normal, "normal-after-private", "normal", checks)
                private.close_window()
                visit(private_peer, "private-survivor", "private", checks)
                private_peer.close_window()
                require(Window.handles(process) == {normal.hwnd}, "Private HWNDs survived the end of the session")

                fresh = normal.open_window(PRIVATE_WINDOW, "private_reopened", windows)
                fresh.blank(private=True)
                visit(fresh, "private-reset", None, checks)
                fresh.close_window()
                process.normal_close(normal.hwnd)

            report["profile_checks"] = {}
            inspect_profile(profile, site, report["profile_checks"])

            # Reopening the same disposable profile proves normal storage survives a full shutdown.
            with OwnedProcess(win, exe, [*args, "--flit-url=" + site.origin + site.paths["normal-restart"]],
                              report["restart"]) as process:
                normal = Window(process, site)
                visit(normal, "normal-restart", "normal", checks, start=True)
                process.normal_close(normal.hwnd)
        finally:
            with site.lock:
                report["transport"] = dict(site.transport)
                report["fixture_hits"] = {site.routes.get(path, path): len(items)
                                          for path, items in site.requests.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe alongside Flit.dll")
    parser.add_argument("--work", type=Path, default=SCRATCH, help="scratch subdirectory for a fresh test profile")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    exe = args.exe.resolve()
    dll = exe.with_name("Flit.dll")
    if exe.name.casefold() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("--exe must point to Flit.exe alongside Flit.dll")

    def hashes():
        result = {}
        for path in (exe, dll):
            with path.open("rb") as stream:
                result[path.name] = file_digest(stream, "sha256").hexdigest()
        return result

    report = {"result": "fail", "exe": str(exe)}
    try:
        report["sha256"] = hashes()
        try:
            run(exe, args.work.resolve(), report)
        finally:
            report["sha256_after"] = hashes()
        require(report["sha256"] == report["sha256_after"], "Flit.exe or Flit.dll changed during the run")
        report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
