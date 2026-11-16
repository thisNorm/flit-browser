"""Exercise an agent-capable Flit.exe over MCP stdio and an owned loopback site."""

import argparse
from contextlib import ExitStack
import ctypes as C
from ctypes import wintypes as W
from hashlib import file_digest
from http.server import BaseHTTPRequestHandler
import json
from pathlib import Path
import os
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
from windows_smoke import JobLimit, OwnedProcess, Win32, api, fixture, require


PROTOCOL = "2025-11-25"
TOOLS = {
    "flit_open": {"url"},
    "flit_list_tabs": set(),
    "flit_snapshot": {"tab"},
    "flit_follow_link": {"tab", "snapshot", "link"},
    "flit_close_tab": {"tab"},
    "flit_sign_in": {"tab"},
    "flit_stop_loading": {"tab"},
}
DOM_TEXT = "렌더링된 한국어 문장: 사과와 포도"
PASSWORD = "FLIT_FIXTURE_DUMMY_VALUE_6D94"
SOURCE_TITLE = "Flit MCP 소스"
TARGET_TITLE = "Flit MCP 링크 도착"
OTHER_TITLE = "Flit MCP 두 번째 탭"
COOKIE = "flit_fixture_session=owned"
PAGES = {
    "/source": f'''<title>{SOURCE_TITLE}</title><p id="dynamic">Waiting for the fixture</p>
<label>Fixture password <input type="password" value="{PASSWORD}"></label>
<a href="/target">상세 보기</a><a href="/other">다른 페이지</a>
<script>fetch('/blocked-post', {{method: 'POST', body: 'fixture'}}).catch(() => {{}});
fetch('/dom-text').then(r => r.text()).then(text => {{
    document.getElementById('dynamic').textContent = text;
}});</script>''',
    "/target": (f'<title>{TARGET_TITLE}</title><h1 id="details">링크를 열었습니다</h1>'
                '<a href="/source">돌아가기</a><a href="#details">이 페이지에서 이동</a>'),
    "/other": f'''<title>{OTHER_TITLE}</title><h1>두 번째 탭의 내용</h1>
<input type="password" disabled><input type="password" hidden>''',
    "/session": '<title>Flit MCP session</title><p>{}</p>',
}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        with self.server.lock:
            self.server.hits["POST " + self.path] += 1
        self.send_error(405)

    def do_GET(self):
        if self.path == "/held-load":
            with self.server.lock:
                self.server.hits[self.path] += 1
            # Send no response; observe the browser closing the outstanding request.
            self.connection.settimeout(15)
            try:
                disconnected = not self.connection.recv(1)
            except ConnectionResetError:
                disconnected = True
            except TimeoutError:
                disconnected = False
            if disconnected:
                with self.server.lock:
                    self.server.hits["disconnected /held-load"] += 1
            return
        if self.path == "/dom-text":
            content_type, body = "text/plain", DOM_TEXT
        elif self.path == "/session":
            has_cookie = COOKIE in self.headers.get("Cookie", "").split("; ")
            content_type, body = "text/html", PAGES[self.path].format(
                "Owned cookie present" if has_cookie else "No session cookie")
        elif self.path in PAGES:
            content_type, body = "text/html", '<!doctype html><meta charset="utf-8">' + PAGES[self.path]
        else:
            self.send_error(404)
            return
        payload = body.encode("utf-8")
        with self.server.lock:
            self.server.hits[self.path] += 1
        self.send_response(200)
        self.send_header("Content-Type", content_type + "; charset=utf-8")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", "no-store")
        if self.path == "/source":
            self.send_header("Set-Cookie", COOKIE + "; Path=/; HttpOnly; SameSite=Lax")
        self.end_headers()
        self.wfile.write(payload)


class JobAccounting(C.Structure):
    _fields_ = [("times", C.c_longlong * 4), ("faults", W.DWORD), ("total", W.DWORD),
                ("active", W.DWORD), ("terminated", W.DWORD)]


class StdioProcess(OwnedProcess):
    def __init__(self, argv, env, child_ends, report):
        import msvcrt

        win = self.win = Win32()
        native = win.native
        self.report = report
        self.process = thread = None
        self.foreground = api(C.WinDLL("user32", use_last_error=True), "GetForegroundWindow", W.HWND)
        handles = [msvcrt.get_osfhandle(end.fileno()) for end in child_ends]
        for handle in handles:
            os.set_handle_inheritable(handle, True)
        startup = subprocess.STARTUPINFO(
            dwFlags=subprocess.STARTF_USESTDHANDLES, hStdInput=handles[0], hStdOutput=handles[1],
            hStdError=handles[2], lpAttributeList={"handle_list": handles},
        )
        self.job = win.check(win.create_job(None, None))
        assigned = False
        try:
            limit = JobLimit()
            limit.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            win.check(win.set_job(self.job, 9, C.byref(limit), C.sizeof(limit)))
            self.process, thread, self.pid, _ = native.CreateProcess(
                str(argv[0]), subprocess.list2cmdline(argv), None, None, True,
                subprocess.CREATE_NO_WINDOW | 0x00000004,  # CREATE_SUSPENDED
                env, str(argv[0].parent), startup,
            )
            win.check(win.assign_job(self.job, self.process))
            assigned = True
            # Drop the parent's copies before resume so pipe EOF reflects the owned process tree.
            for end in child_ends:
                end.close()
            report["pid"] = self.pid
            if win.resume(thread) == 0xFFFFFFFF:
                raise C.WinError(C.get_last_error())
        except BaseException:
            try:
                if self.process and not assigned:
                    native.TerminateProcess(self.process, 1)
                    native.WaitForSingleObject(self.process, 5000)
            finally:
                native.CloseHandle(self.job)
                if self.process:
                    native.CloseHandle(self.process)
            raise
        finally:
            if thread:
                native.CloseHandle(thread)

    def total_processes(self):
        accounting = JobAccounting()
        self.win.check(self.win.query_job(self.job, 1, C.byref(accounting), C.sizeof(accounting), None))
        return accounting.total

    def observe_windows(self):
        pids = self.pids()
        win = self.win
        require(win.pid(self.foreground()) not in pids, "Flit agent took the foreground")
        visible = []

        @win.callback
        def collect(hwnd, _):
            if win.pid(hwnd) in pids and win.visible(hwnd):
                visible.append(hwnd)
            return True

        win.check(win.enum(collect, 0))
        require(not visible, f"Flit agent showed a top-level window: {visible}")


def pipe(stack):
    read, write = os.pipe()
    return (stack.enter_context(os.fdopen(read, "rb", buffering=0)),
            stack.enter_context(os.fdopen(write, "wb", buffering=0)))


def read_available(native, handle, stream):
    try:
        size, _ = native.PeekNamedPipe(handle, 0)
    except BrokenPipeError:
        return b"", True
    return (stream.read(min(size, 8192)) if size else b""), False


class Mcp:
    def __init__(self, process, stdin, stdout, stderr):
        import msvcrt

        self.process, self.stdin = process, stdin
        self.stdout, self.stderr = stdout, stderr
        self.output_handle = msvcrt.get_osfhandle(stdout.fileno())
        self.error_handle = msvcrt.get_osfhandle(stderr.fileno())
        self.output = self.stderr_tail = b""
        self.eof = False
        self.request_id = 0
        self.deadline = time.monotonic() + 80

    def pump(self):
        self.process.observe_windows()
        native = self.process.win.native
        out, self.eof = read_available(native, self.output_handle, self.stdout)
        err, _ = read_available(native, self.error_handle, self.stderr)
        self.output += out
        self.stderr_tail = (self.stderr_tail + err)[-4096:]
        require(len(self.output) <= 256 * 1024, "Oversized or unsolicited MCP stdout")

    def wait(self, label, deadline, check):
        while time.monotonic() < min(self.deadline, deadline):
            self.pump()
            if result := check():
                return result
            time.sleep(0.04)
        raise RuntimeError(f"Timed out waiting for {label}")

    def send(self, method, params=None, *, notify=False, timeout=6, deadline=None):
        packet = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            packet["params"] = params
        if not notify:
            self.request_id += 1
            packet["id"] = self.request_id
        label = f"{method} {params['name']}" if method == "tools/call" else method
        line = json.dumps(packet).encode("utf-8") + b"\n"
        # One small request at a time stays within the input pipe buffer.
        require(len(line) <= 1024 and self.stdin.write(line) == len(line), f"Could not write {label}")
        if notify:
            return None

        def response():
            raw, newline, rest = self.output.partition(b"\n")
            if not newline:
                require(not self.eof, f"MCP stdout ended before {label} replied")
                return None
            self.output = rest
            try:
                message = json.loads(raw.decode("utf-8"))
            except ValueError as error:
                raise RuntimeError(f"Invalid MCP stdout during {label}: {raw[:200]!r}") from error
            require(isinstance(message, dict) and message.get("jsonrpc") == "2.0"
                    and type(message.get("id")) is int and message["id"] == self.request_id
                    and (("result" in message) != ("error" in message))
                    and isinstance(message.get("result", message.get("error")), dict),
                    f"Unexpected MCP response to {label}: {message!r}")
            return message

        end = time.monotonic() + timeout
        return self.wait(label, min(end, deadline) if deadline is not None else end, response)

    def tool(self, name, args, *, reject=False, **timing):
        reply = self.send("tools/call", {"name": name, "arguments": args}, **timing)
        if name == "flit_snapshot":
            require(PASSWORD not in json.dumps(reply, ensure_ascii=False), "Snapshot exposed a password value")
        if reject and "error" in reply:
            error = reply["error"]
            require(isinstance(error, dict) and type(error.get("code")) is int
                    and isinstance(error.get("message"), str), f"{name} returned an invalid RPC error")
            return None
        result = reply.get("result")
        require(isinstance(result, dict), f"{name} returned no tool result: {reply!r}")
        content = result.get("content")
        require(isinstance(content, list) and len(content) == 1 and isinstance(content[0], dict)
                and content[0].get("type") == "text" and isinstance(content[0].get("text"), str),
                f"{name} returned no text content")
        if reject:
            require(result.get("isError") is True and content[0]["text"], f"{name} accepted {args!r}")
            return None
        require(result.get("isError", False) is False, f"{name} failed: {content[0]['text']}")
        data = result.get("structuredContent")
        require(isinstance(data, dict) and json.loads(content[0]["text"]) == data,
                f"{name} returned inconsistent structured content and fallback JSON")
        return data

    def ready_snapshot(self, tab, url, title, text, *, login_form=False):
        end = time.monotonic() + 15
        while time.monotonic() < min(end, self.deadline):
            data = self.tool("flit_snapshot", {"tab": tab}, timeout=13, deadline=end)
            require(data.get("tab") == tab, f"Snapshot changed the tab: {data!r}")
            status = data.get("status")
            require(status in (None, "loading", "ready"), f"Snapshot failed for {url}: {data!r}")
            if status != "loading":
                require(isinstance(data.get("text"), str) and isinstance(data.get("links"), list)
                        and type(data.get("snapshot")) is int, "Ready snapshot lacks text, links, or ID")
                if data.get("url") == url and data.get("title") == title and text in data["text"]:
                    require(data.get("login_form_detected") is login_form, f"Wrong login form flag for {url}")
                    return data
            time.sleep(0.06)
        raise RuntimeError(f"Timed out waiting for rendered text at {url}")

    def finish(self):
        self.stdin.close()
        proc = self.process
        self.wait("clean EOF and empty owned job", time.monotonic() + 18,
                  lambda: self.eof and proc.exited() and not proc.pids())
        require(not self.output, "Unsolicited or incomplete MCP stdout after stdin EOF")
        code = proc.win.native.GetExitCodeProcess(proc.process)
        proc.report["exit_code"] = code
        require(code == 0, f"Flit returned 0x{code:08x} after stdin EOF")


def handshake(mcp):
    init = mcp.send("initialize", {"protocolVersion": PROTOCOL, "capabilities": {},
                                   "clientInfo": {"name": "flit-windows-fixture", "version": "1"}})
    init = init.get("result", {})
    require(init.get("protocolVersion") == PROTOCOL and init.get("serverInfo", {}).get("name")
            and isinstance(init.get("capabilities", {}).get("tools"), dict), f"Invalid initialize: {init!r}")
    mcp.send("notifications/initialized", notify=True)
    tools = mcp.send("tools/list").get("result", {}).get("tools", [])
    require(len(tools) == len(TOOLS) and {tool["name"] for tool in tools} == set(TOOLS),
            "tools/list did not advertise exactly the expected tools")
    for tool in tools:
        schema, keys = tool["inputSchema"], TOOLS[tool["name"]]
        require(schema.get("type") == "object" and set(schema.get("properties", {})) == keys
                and set(schema.get("required", [])) == keys and schema.get("additionalProperties") is False,
                f"Wrong input schema for {tool['name']}")


def exercise(mcp, peer, site, report):
    handshake(mcp)

    def list_tabs():
        tabs = mcp.tool("flit_list_tabs", {}).get("tabs")
        require(isinstance(tabs, list), "flit_list_tabs did not return a tab list")
        return tabs

    require(list_tabs() == [], "Expected no agent tabs before the first open")
    mcp.tool("flit_open", {"url": "data:text/plain,flit-fixture"}, reject=True)
    mcp.tool("flit_stop_loading", {"tab": 1}, reject=True)
    require(list_tabs() == [], "A rejected URL created an agent tab")
    report["processes_before_first_open"] = mcp.process.total_processes()
    require(report["processes_before_first_open"] == 1 and not site.report()["requests"],
            "Agent started child processes or made fixture requests before the first open")

    source, target, other = (site.origin + path for path in ("/source", "/target", "/other"))

    def navigation(result, url, tab=None):
        ident = result.get("tab")
        require(type(ident) is int and (tab is None or ident == tab) and result.get("url") == url
                and result.get("status") in ("loading", "ready"), f"Unexpected navigation: {result!r}")
        return ident

    report["stage"] = "render and inspect two tabs"
    first = navigation(mcp.tool("flit_open", {"url": source}, timeout=27), source)
    second = navigation(mcp.tool("flit_open", {"url": other}, timeout=13), other)
    require(first != second, "Two opens returned the same tab ID")
    mcp.tool("flit_eval", {"tab": first, "script": "void 0"}, reject=True)
    mcp.tool("flit_snapshot", {"tab": first, "script": "void 0"}, reject=True)
    page = mcp.ready_snapshot(first, source, SOURCE_TITLE, DOM_TEXT, login_form=True)
    mcp.ready_snapshot(second, other, OTHER_TITLE, "두 번째 탭의 내용")
    require(site.count("/dom-text") > 0 and site.count("/target") == 0,
            "Expected the DOM update fetch and no request to the unfollowed target")
    links = page["links"]
    require(len(links) == 2 and {link.get("url"): link.get("text") for link in links}
            == {target: "상세 보기", other: "다른 페이지"} and all(type(link.get("id")) is int for link in links)
            and len({link["id"] for link in links}) == 2, f"Unexpected snapshot links: {links!r}")
    report["dynamic_dom_and_login_form_checked"] = True
    tabs = list_tabs()
    require(len(tabs) == 2 and {(t["tab"], t["url"], t["title"], t["status"]) for t in tabs}
            == {(first, source, SOURCE_TITLE, "ready"), (second, other, OTHER_TITLE, "ready")},
            f"Unexpected tab list: {tabs!r}")

    report["stage"] = "follow link and reject stale snapshot"
    follow = {"tab": first, "snapshot": page["snapshot"],
              "link": next(link["id"] for link in links if link["url"] == target)}
    navigation(mcp.tool("flit_follow_link", follow), target, first)
    arrived = mcp.ready_snapshot(first, target, TARGET_TITLE, "링크를 열었습니다")
    require(arrived["snapshot"] != page["snapshot"] and site.count("/target") > 0,
            "Link follow did not replace the source document and snapshot")
    return_link = next((link for link in arrived["links"] if link["url"] == source), None)
    require(return_link and type(return_link["id"]) is int, "Target has no valid return link")
    # The link ID is valid in the current document, so only the stale revision should reject it.
    mcp.tool("flit_follow_link", {**follow, "link": return_link["id"]}, reject=True)
    require({tab["tab"]: tab["url"] for tab in list_tabs()} == {first: target, second: other},
            "Rejected follow changed the open tabs")
    anchor = target + "#details"
    anchor_link = next(link for link in arrived["links"] if link["url"] == anchor)
    navigation(mcp.tool("flit_follow_link", {"tab": first, "snapshot": arrived["snapshot"],
               "link": anchor_link["id"]}), anchor, first)
    mcp.ready_snapshot(first, anchor, TARGET_TITLE, "링크를 열었습니다")
    require(site.count("/target") == 1, "Same-document link reloaded the page")
    report["same_document_link_stays_readable"] = True

    report["stage"] = "verify cookies and tabs are scoped to one MCP connection"
    session_url = site.origin + "/session"
    same = navigation(mcp.tool("flit_open", {"url": session_url}), session_url)
    mcp.ready_snapshot(same, session_url, "Flit MCP session", "Owned cookie present")
    mcp.tool("flit_close_tab", {"tab": same})
    handshake(peer)
    require(peer.tool("flit_list_tabs", {}) == {"tabs": []}, "Another connection sees the first connection's tabs")
    peer.tool("flit_snapshot", {"tab": first}, reject=True)
    isolated = navigation(peer.tool("flit_open", {"url": session_url}, timeout=27), session_url)
    peer.ready_snapshot(isolated, session_url, "Flit MCP session", "No session cookie")

    report["stage"] = "stop a held load in the owning connection"
    held_url = site.origin + "/held-load"
    held = navigation(mcp.tool("flit_open", {"url": held_url}), held_url)
    mcp.wait("held navigation to reach the server", time.monotonic() + 8,
             lambda: site.count("/held-load"))
    peer.tool("flit_stop_loading", {"tab": held}, reject=True)
    require(mcp.tool("flit_snapshot", {"tab": held}).get("status") == "loading",
            "The held load did not stay loading after the peer's stop attempt")
    require(mcp.tool("flit_stop_loading", {"tab": held}) == {"tab": held, "status": "stopped"},
            "The owner could not stop the held load")
    mcp.wait("browser to disconnect the held request", time.monotonic() + 8,
             lambda: site.count("disconnected /held-load"))
    stopped = mcp.tool("flit_snapshot", {"tab": held})
    require(stopped.get("status") == "stopped" and "snapshot" not in stopped and "text" not in stopped,
            f"An interrupted main document was reported as readable: {stopped!r}")
    require(mcp.tool("flit_stop_loading", {"tab": second}) == {"tab": second, "status": "not_loading"},
            "Stopping a settled tab did not report an idle no-op")
    mcp.tool("flit_close_tab", {"tab": held})
    report["owned_load_stopped"] = True

    peer.finish()
    require({tab["tab"]: tab["url"] for tab in list_tabs()} == {first: anchor, second: other},
            "Closing the other connection changed the first connection's tabs")
    report["tabs_and_cookie_isolated_between_connections"] = True

    report["stage"] = "close one tab and send stdin EOF with the other open"
    closed = mcp.tool("flit_close_tab", {"tab": first})
    require(closed.get("tab") == first and closed.get("status") == "closed", f"Unexpected close: {closed!r}")
    mcp.tool("flit_snapshot", {"tab": first}, reject=True)
    require([tab["tab"] for tab in list_tabs()] == [second], "Close changed the wrong tabs")
    mcp.finish()
    require(site.count("POST /blocked-post") == 0, "The page sent a non-read request")
    report["page_post_blocked"] = True


def run(exe, report):
    with tempfile.TemporaryDirectory(prefix="flit-mcp-fixture-", ignore_cleanup_errors=True) as root, fixture() as site:
        site.RequestHandlerClass = Handler
        root = Path(root)
        # Give even an accidental normal launch no path to the actual Flit profile.
        env = {key: os.environ[key] for key in ("SystemRoot", "SystemDrive", "WINDIR") if key in os.environ}
        for key in ("LOCALAPPDATA", "APPDATA", "USERPROFILE", "TEMP", "TMP"):
            path = root / ("temp" if key in ("TEMP", "TMP") else key)
            path.mkdir(exist_ok=True)
            env[key] = str(path)
        def connect(stack, record):
            child_in, parent_in = pipe(stack)
            parent_out, child_out = pipe(stack)
            parent_err, child_err = pipe(stack)
            proc = stack.enter_context(StdioProcess(
                [exe, "--flit-agent"], env, (child_in, child_out, child_err), record))
            return Mcp(proc, parent_in, parent_out, parent_err)

        report["stage"] = "initialize and list without Chromium children"
        try:
            with ExitStack() as stack:
                mcp = connect(stack, report)
                report["second_connection"] = {}
                peer = connect(stack, report["second_connection"])
                try:
                    exercise(mcp, peer, site, report)
                finally:
                    if mcp.stderr_tail:
                        report["stderr_tail"] = mcp.stderr_tail.decode("utf-8", errors="replace")
                    if peer.stderr_tail:
                        report["second_connection"]["stderr_tail"] = peer.stderr_tail.decode("utf-8", errors="replace")
            require(not report["cleanup_needed"] and not report["second_connection"]["cleanup_needed"],
                    "Flit needed forced cleanup after EOF")
            require(not (Path(env["LOCALAPPDATA"]) / "Flit Browser").exists(),
                    "Flit touched the normal profile location inside isolated LOCALAPPDATA")
            require(not list(Path(env["TEMP"]).glob("FlitAgent-*")), "Agent left its temporary session behind")
            report["no_visible_window_or_profile_left"] = True
        finally:
            report["http_requests"] = site.report()["requests"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged agent-capable Flit.exe next to Flit.dll")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    exe = args.exe.resolve()
    dll = exe.with_name("Flit.dll")
    if exe.name.lower() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("--exe must point to Flit.exe next to Flit.dll")

    with dll.open("rb") as file:
        report = {"result": "fail", "dll_sha256": file_digest(file, "sha256").hexdigest()}
    try:
        image = dll.read_bytes()
        # Static gate: 0.10's unrecognized flag would open the regular browser.
        if b"flit_follow_link" not in image or not (b"--flit-agent" in image or b"Flit agent mode" in image):
            report.update(result="skip", reason="Flit.dll has no agent MCP markers; no process was started")
            code = 77
        else:
            run(exe, report)
            report.update(result="pass", stage="complete")
            code = 0
    except (RuntimeError, OSError, ValueError, TypeError, KeyError, AttributeError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
        code = 1
    print(json.dumps(report, indent=2))
    return code


if __name__ == "__main__":
    sys.exit(main())
