"""Exercise the real Flit.exe through its Win32 controls and a loopback site."""

import argparse
from collections import Counter
from contextlib import contextmanager
import ctypes as C
from ctypes import wintypes as W
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import secrets
import struct
import subprocess
import sys
import tempfile
import threading
import time


WORKSPACE = Path(__file__).resolve().parents[1]
ADDRESS, BACK, FORWARD, RELOAD, NEW_TAB, GO = 101, 102, 103, 104, 105, 107
TABS, SIDEBAR_TOGGLE, TAB_FILTER = 108, 109, 110
TOOLBAR = (ADDRESS, BACK, FORWARD, RELOAD, GO, SIDEBAR_TOGGLE)
SIDEBAR = (TABS, TAB_FILTER)
CLASSES = {ADDRESS: "Edit", TAB_FILTER: "Edit", TABS: "ListBox",
           **dict.fromkeys((BACK, FORWARD, RELOAD, NEW_TAB, GO, SIDEBAR_TOGGLE), "Button")}
PAGES = {"/start", "/edit", "/first", "/second", "/tab", "/prime", "/restart"}
TAB_TITLE = "Flit smoke: Orchid"
SHOWN = {"/shown" + path for path in PAGES}
EXECUTED = {"/executed/prime", "/executed/restart"}
ASSET = b'''fetch("/executed/" + document.body.dataset.phase, {cache: "no-store"});'''


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


class Fixture(ThreadingHTTPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.origin = f"http://127.0.0.1:{self.server_port}"
        self.cookie = "flit_smoke=" + secrets.token_hex(12)
        self.lock = threading.Lock()
        self.hits = Counter()
        self.cookie_hits = Counter()
        self.remaining = 200
        self.release_edit = threading.Event()

    def get_request(self):
        conn, addr = super().get_request()
        conn.settimeout(3)
        return conn, addr

    def count(self, path, cookie=False):
        with self.lock:
            return (self.cookie_hits if cookie else self.hits)[path]

    def report(self):
        with self.lock:
            return {"requests": dict(self.hits), "synthetic_cookie_requests": dict(self.cookie_hits)}


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        site = self.server
        path = self.path
        with site.lock:
            site.remaining -= 1
            allowed = site.remaining >= 0
        if not allowed:
            self.send_error(429)
            return

        cache = "no-store"
        content_type = "text/plain"
        status = 200
        body = b""
        if path in PAGES:
            phase = path[1:]
            title = TAB_TITLE if path == "/tab" else f"Flit smoke: {phase}"
            script = '<script src="/cache.js"></script>' if path in {"/prime", "/restart"} else ""
            if path == "/edit":
                script = '<script>fetch("/edit-gate",{cache:"no-store"}).then(() => {document.title="Flit edit race complete";});</script>'
            body = f'''<!doctype html><meta charset="utf-8"><title>{title}</title>
<body data-phase="{phase}" style="background:#173145;color:#fff;font:32px sans-serif">
<h1>Flit smoke: {phase}</h1><div style="height:80px;background:#35b8a6"></div>
<script>addEventListener("pageshow", () => fetch("/shown" + location.pathname,
{{cache: "no-store"}}));</script>{script}'''.encode("ascii")
            content_type = "text/html; charset=utf-8"
        elif path == "/cache.js":
            body = ASSET
            cache = "public, max-age=3600, immutable"
            content_type = "application/javascript"
        elif path == "/edit-gate":
            status = 204 if site.release_edit.wait(7) else 408
        elif path in SHOWN or path in EXECUTED or path == "/favicon.ico":
            status = 204
        else:
            self.send_error(404)
            return

        with site.lock:
            site.hits[path] += 1
            cookies = self.headers.get("Cookie", "").split(";")
            if site.cookie in (cookie.strip() for cookie in cookies):
                site.cookie_hits[path] += 1
        self.send_response(status)
        self.send_header("Cache-Control", cache)
        if path == "/prime":
            self.send_header("Set-Cookie", site.cookie + "; Max-Age=86400; Path=/; HttpOnly; SameSite=Lax")
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def log_message(self, *_):
        pass


@contextmanager
def fixture():
    with Fixture() as site:
        thread = threading.Thread(target=site.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.shutdown()
            thread.join(2)


def until(label, check, timeout=12, alive=None):
    end = time.monotonic() + timeout
    while True:
        if alive:
            alive()
        result = check()
        if result:
            return result
        if time.monotonic() >= end:
            raise RuntimeError(f"Timed out waiting for {label} ({timeout}s)")
        time.sleep(0.05)


class BasicLimit(C.Structure):
    _fields_ = [
        ("time_limits", C.c_longlong * 2), ("flags", W.DWORD),
        ("min_working_set", C.c_size_t), ("max_working_set", C.c_size_t),
        ("process_limit", W.DWORD), ("affinity", C.c_size_t),
        ("priority", W.DWORD), ("scheduling", W.DWORD),
    ]


class JobLimit(C.Structure):
    _fields_ = [("basic", BasicLimit), ("io", C.c_ulonglong * 6), ("memory", C.c_size_t * 4)]


class JobPids(C.Structure):
    _fields_ = [("assigned", W.DWORD), ("listed", W.DWORD), ("pids", C.c_size_t * 128)]


def api(dll, name, result, *args):
    fn = getattr(dll, name)
    fn.restype, fn.argtypes = result, args
    return fn


class Win32:
    def __init__(self):
        import _winapi

        self.native = _winapi
        kernel = C.WinDLL("kernel32", use_last_error=True)
        user = C.WinDLL("user32", use_last_error=True)
        advapi = C.WinDLL("advapi32", use_last_error=True)
        handle, ptr, dword, boolean = W.HANDLE, C.c_void_p, W.DWORD, W.BOOL
        self.create_job = api(kernel, "CreateJobObjectW", handle, ptr, W.LPCWSTR)
        self.set_job = api(kernel, "SetInformationJobObject", boolean, handle, C.c_int, ptr, dword)
        self.query_job = api(kernel, "QueryInformationJobObject", boolean, handle, C.c_int, ptr, dword, ptr)
        self.assign_job = api(kernel, "AssignProcessToJobObject", boolean, handle, handle)
        self.in_job = api(kernel, "IsProcessInJob", boolean, handle, handle, C.POINTER(boolean))
        self.kill_job = api(kernel, "TerminateJobObject", boolean, handle, W.UINT)
        self.resume = api(kernel, "ResumeThread", dword, handle)
        self.callback = C.WINFUNCTYPE(boolean, W.HWND, W.LPARAM)
        self.enum = api(user, "EnumWindows", boolean, self.callback, W.LPARAM)
        self.visible = api(user, "IsWindowVisible", boolean, W.HWND)
        self.enabled = api(user, "IsWindowEnabled", boolean, W.HWND)
        self.get_pid = api(user, "GetWindowThreadProcessId", dword, W.HWND, C.POINTER(dword))
        self.get_class = api(user, "GetClassNameW", C.c_int, W.HWND, W.LPWSTR, C.c_int)
        self.get_item = api(user, "GetDlgItem", W.HWND, W.HWND, C.c_int)
        self.get_style = api(user, "GetWindowLongW", W.LONG, W.HWND, C.c_int)
        self.find_child = api(user, "FindWindowExW", W.HWND, W.HWND, W.HWND, W.LPCWSTR, W.LPCWSTR)
        self.get_client_rect = api(user, "GetClientRect", boolean, W.HWND, C.POINTER(W.RECT))
        self.get_window_rect = api(user, "GetWindowRect", boolean, W.HWND, C.POINTER(W.RECT))
        self.client_to_screen = api(user, "ClientToScreen", boolean, W.HWND, C.POINTER(W.POINT))
        self.dpi = api(user, "GetDpiForWindow", W.UINT, W.HWND)
        self.window_dpi_context = api(user, "GetWindowDpiAwarenessContext", ptr, W.HWND)
        self.thread_dpi_context = api(user, "SetThreadDpiAwarenessContext", ptr, ptr)
        self.set_window_pos = api(user, "SetWindowPos", boolean, W.HWND, W.HWND, C.c_int, C.c_int,
                                  C.c_int, C.c_int, W.UINT)
        self.send = api(user, "SendMessageTimeoutW", W.LPARAM, W.HWND, W.UINT, W.WPARAM, W.LPARAM,
                        W.UINT, W.UINT, C.POINTER(C.c_size_t))
        self.post = api(user, "PostMessageW", boolean, W.HWND, W.UINT, W.WPARAM, W.LPARAM)
        self.map_key = api(user, "MapVirtualKeyW", W.UINT, W.UINT, W.UINT)
        self.open_process = api(kernel, "OpenProcess", handle, dword, boolean, dword)
        self.open_token = api(advapi, "OpenProcessToken", boolean, handle, dword, C.POINTER(handle))
        self.token_info = api(advapi, "GetTokenInformation", boolean, handle, C.c_int, ptr, dword,
                              C.POINTER(dword))
        self.restricted = api(advapi, "IsTokenRestricted", boolean, handle)
        self.sid_count = api(advapi, "GetSidSubAuthorityCount", C.POINTER(C.c_ubyte), ptr)
        self.sid_part = api(advapi, "GetSidSubAuthority", C.POINTER(dword), ptr, dword)

    @staticmethod
    def check(result):
        if not result:
            raise C.WinError(C.get_last_error())
        return result

    def pid(self, hwnd):
        pid = W.DWORD()
        self.get_pid(hwnd, C.byref(pid))
        return pid.value

    def class_name(self, hwnd):
        name = C.create_unicode_buffer(128)
        self.get_class(hwnd, name, len(name))
        return name.value

    def rect(self, hwnd, *, client=False):
        rect = W.RECT()
        self.check((self.get_client_rect if client else self.get_window_rect)(hwnd, C.byref(rect)))
        return rect.left, rect.top, rect.right, rect.bottom

    def visible_children(self, parent, class_name=None):
        child = None
        while child := self.find_child(parent, child, class_name, None):
            if self.visible(child):
                yield child

    @contextmanager
    def window_coordinates(self, hwnd):
        previous = self.check(self.thread_dpi_context(self.check(self.window_dpi_context(hwnd))))
        try:
            yield
        finally:
            self.check(self.thread_dpi_context(previous))

    def message(self, hwnd, msg, wparam=0, lparam=0):
        value = C.c_size_t()
        C.set_last_error(0)
        result = self.send(hwnd, msg, wparam, lparam, 0x23, 5000, C.byref(value))
        if not result:
            raise RuntimeError(f"Win32 message 0x{msg:04x} failed (error {C.get_last_error()})")
        return C.c_ssize_t(value.value).value


class OwnedProcess:
    def __init__(self, win, exe, args, report):
        self.win = win
        self.report = report
        self.process = None
        self.job = win.check(win.create_job(None, None))
        try:
            limit = JobLimit()
            limit.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            win.check(win.set_job(self.job, 9, C.byref(limit), C.sizeof(limit)))
            self.process, thread, self.pid, _ = win.native.CreateProcess(
                str(exe), subprocess.list2cmdline([str(exe), *args]), None, None,
                False, 0x08000004, None, str(exe.parent), subprocess.STARTUPINFO(),
            )
            report["pid"] = self.pid
            try:
                # Assign before resuming, so no Chromium child can start outside the job.
                if not win.assign_job(self.job, self.process):
                    error = C.WinError(C.get_last_error())
                    win.native.TerminateProcess(self.process, 1)
                    win.native.WaitForSingleObject(self.process, 5000)
                    raise error
                if win.resume(thread) == 0xFFFFFFFF:
                    raise C.WinError(C.get_last_error())
            finally:
                win.native.CloseHandle(thread)
        except BaseException:
            win.native.CloseHandle(self.job)
            if self.process:
                win.native.CloseHandle(self.process)
            raise

    def __enter__(self):
        return self

    def __exit__(self, *_):
        try:
            remaining = self.pids()
            self.report["cleanup_needed"] = bool(remaining)
            if remaining:
                self.report["pids_before_cleanup"] = remaining
                self.win.check(self.win.kill_job(self.job, 1))
                until("owned process cleanup", lambda: not self.pids(), 6)
        finally:
            try:
                self.report["leftover_pids"] = self.pids()
            finally:
                self.win.native.CloseHandle(self.job)
                self.win.native.CloseHandle(self.process)

    def pids(self):
        pids = JobPids()
        self.win.check(self.win.query_job(self.job, 3, C.byref(pids), C.sizeof(pids), None))
        require(pids.assigned == pids.listed, "Owned process list was truncated")
        return sorted(pids.pids[:pids.listed])

    def exited(self):
        return self.win.native.WaitForSingleObject(self.process, 0) == 0

    def alive(self):
        if self.exited():
            code = self.win.native.GetExitCodeProcess(self.process)
            raise RuntimeError(f"Flit exited early (0x{code:08x})")

    def normal_close(self, hwnd):
        self.alive()
        require(self.win.pid(hwnd) == self.pid, "Flit main window disappeared before WM_CLOSE")
        self.win.check(self.win.post(hwnd, 0x0010, 0, 0))
        self.report["wm_close_posted"] = True
        until("normal exit after WM_CLOSE", self.exited, 20)
        code = self.win.native.GetExitCodeProcess(self.process)
        self.report["exit_code"] = code
        until("empty process job after WM_CLOSE", lambda: not self.pids(), 8)
        require(code == 0, f"Flit returned 0x{code:08x} after WM_CLOSE")
        self.report["normal_close"] = True


class Browser:
    def __init__(self, process, site):
        self.process, self.site, self.win = process, site, process.win
        self.hwnd = self.wait("visible FlitBrowserWindow", self.find_window, 35)
        self.controls = self.wait("native Win32 toolbar, tab ListBox and filter Edit", self.find_controls)
        require(not self.win.get_item(self.hwnd, 106), "Unexpected Close Tab child control 106")
        styles = 0x0051  # LBS_HASSTRINGS | LBS_OWNERDRAWFIXED | LBS_NOTIFY
        require(self.win.get_style(self.controls[TABS], -16) & styles == styles,
                "Tab ListBox is missing HASSTRINGS, OWNERDRAWFIXED or NOTIFY")

    def wait(self, label, check, timeout=12):
        return until(label, check, timeout, self.process.alive)

    def find_window(self):
        found = []

        @self.win.callback
        def visit(hwnd, _):
            if (self.win.pid(hwnd) == self.process.pid and self.win.visible(hwnd)
                    and self.win.class_name(hwnd) == "FlitBrowserWindow"):
                found.append(hwnd)
            return True

        self.win.check(self.win.enum(visit, 0))
        return found[0] if found else None

    def find_controls(self):
        controls = {cid: self.win.get_item(self.hwnd, cid) for cid in CLASSES}
        if all(hwnd and self.win.pid(hwnd) == self.process.pid
               and self.win.class_name(hwnd).casefold() == CLASSES[cid].casefold()
               for cid, hwnd in controls.items()):
            return controls
        return None

    def message(self, cid, msg, wparam=0, lparam=0):
        hwnd = self.controls[cid]
        require(self.win.pid(hwnd) == self.process.pid, f"Flit control {cid} disappeared")
        return self.win.message(hwnd, msg, wparam, lparam)

    def control_text(self, cid):
        buf = C.create_unicode_buffer(1024)
        self.message(cid, 0x000D, len(buf), C.addressof(buf))  # WM_GETTEXT
        return buf.value

    def address(self):
        return self.control_text(ADDRESS)

    def title(self):
        buf = C.create_unicode_buffer(1024)
        self.win.message(self.hwnd, 0x000D, len(buf), C.addressof(buf))
        return buf.value

    def has_page_title(self, title):
        return title in self.title().split(" — ")

    def set_text(self, cid, text):
        value = C.create_unicode_buffer(text)
        require(self.message(cid, 0x000C, 0, C.addressof(value)), f"Could not set text in control {cid}")

    def edit_address(self, text):
        self.set_text(ADDRESS, text)

    def click(self, cid):
        self.wait(f"enabled Flit button {cid}", lambda: self.win.enabled(self.controls[cid]))
        # BM_CLICK is not reliable when the automation-owned window is not foreground.
        # This is the same BN_CLICKED notification a native button sends to its parent.
        button = self.controls[cid]
        require(self.win.pid(button) == self.process.pid, f"Flit button {cid} disappeared")
        self.win.message(self.hwnd, 0x0111, cid, button)  # WM_COMMAND

    def tab_count(self):
        return self.message(TABS, 0x018B)  # LB_GETCOUNT

    def selected(self):
        return self.message(TABS, 0x0188)  # LB_GETCURSEL

    def press(self, cid, key):
        lparam = 1 | (self.win.map_key(key, 0) << 16)
        if key in (0x26, 0x28):  # Extended arrow keys.
            lparam |= 1 << 24
        self.message(cid, 0x0100, key, lparam)  # WM_KEYDOWN
        if key in (0x0D, 0x1B):
            self.message(cid, 0x0102, key, lparam)  # WM_CHAR from a translated Enter/Escape
        self.message(cid, 0x0101, key, lparam | (3 << 30))  # WM_KEYUP

    def switch(self, index, path=None):
        current = self.selected()
        if current != index:
            require(current in (0, 1), f"Unexpected current tab index: {current}")
            self.press(TABS, 0x26 if index < current else 0x28)  # VK_UP / VK_DOWN
        self.wait(f"selected tab {index}", lambda: self.selected() == index)
        if path:
            self.wait(f"tab {index} address {path}", lambda: self.address() == self.site.origin + path)

    def close_row(self, index):
        hwnd = self.controls[TABS]
        require(self.win.pid(hwnd) == self.process.pid and self.win.visible(hwnd), "Tab ListBox is not visible and owned")
        with self.win.window_coordinates(hwnd):
            _, _, width, height = self.win.rect(hwnd, client=True)
            row = W.RECT()
            require(self.message(TABS, 0x0198, index, C.addressof(row)) != -1,  # LB_GETITEMRECT
                    f"Could not read tab row {index}")
            x = row.right - (14 * self.win.check(self.win.dpi(hwnd)) + 48) // 96
            y = row.top + (row.bottom - row.top) // 2
            require(row.left <= x < row.right and row.top <= y < row.bottom
                    and 0 <= x < width and 0 <= y < height, f"Tab close point is outside row {index}")
            point = x | (y << 16)
            require(self.message(TABS, 0x01A9, 0, point) == index, f"Close point did not hit tab row {index}")
            # Mouse-message coordinates use the sender's DPI context too.
            self.win.check(self.win.post(hwnd, 0x0201, 0x0001, point))  # WM_LBUTTONDOWN, MK_LBUTTON
            self.win.check(self.win.post(hwnd, 0x0202, 0, point))       # WM_LBUTTONUP

    def page_panel(self):
        win = self.win
        visible = [child for child in win.visible_children(self.hwnd, "Static")
                   if next(win.visible_children(child), None)]
        return visible[0] if len(visible) == 1 else None

    def in_client(self, hwnd):
        origin = W.POINT()
        self.win.check(self.win.client_to_screen(self.hwnd, C.byref(origin)))
        left, top, right, bottom = self.win.rect(hwnd)
        return left - origin.x, top - origin.y, right - origin.x, bottom - origin.y

    def page(self, path, after=None, require_request=False, baseline=(0, 0)):
        requests, before = page_counts(self.site, path) if after else baseline
        if after:
            after()
        self.wait(f"page {path} to render and update the address", lambda: (
            self.site.count("/shown" + path) > before
            and self.address() == self.site.origin + path
            and (not require_request or self.site.count(path) > requests)
        ))

    def navigate(self, path):
        def submit():
            self.edit_address(self.site.origin + path)
            self.click(GO)

        self.page(path, submit, require_request=True)


def sidebar_layout(browser):
    win, hwnd = browser.win, browser.hwnd
    panel = browser.wait("visible native page panel", browser.page_panel)
    address = browser.address()
    with win.window_coordinates(hwnd):
        dpi = win.check(win.dpi(hwnd))
        inset = 8 * dpi // 96
        tolerance = (dpi + 47) // 48  # Two logical pixels for native rounding.
        animation = W.BOOL()
        get_setting = api(C.WinDLL("user32", use_last_error=True), "SystemParametersInfoW",
                          W.BOOL, W.UINT, W.UINT, C.c_void_p, W.UINT)
        animated = bool(get_setting(0x1042, 0, C.byref(animation), 0) and animation.value)

        def snapshot(expanded):
            mode = "expanded" if expanded else "collapsed"
            client = win.rect(hwnd, client=True)
            require(browser.page_panel() == panel and browser.address() == address,
                    f"{mode}: active native page changed or was hidden")

            def checked_rect(target, label):
                require(win.visible(target), f"{mode}: {label} is hidden")
                bounds = browser.in_client(target)
                l, t, r, b = bounds
                require(0 <= l < r <= client[2] and 0 <= t < b <= client[3],
                        f"{mode}: {label} {bounds} falls outside the client {client}")
                return bounds

            page = checked_rect(panel, "native page panel")
            require(abs(page[2] - (client[2] - inset)) <= tolerance
                    and abs(page[3] - (client[3] - inset)) <= tolerance,
                    f"{mode}: page panel {page} does not meet the {inset}px frame inset in {client}")
            hosts = [browser.in_client(child) for child in win.visible_children(panel)]
            require(any(all(abs(a - b) <= tolerance for a, b in zip(host, page)) for host in hosts),
                    f"{mode}: no visible browser child fills page {page}: {hosts}")
            native = {cid: checked_rect(browser.controls[cid], f"control {cid}")
                      for cid in (*TOOLBAR, NEW_TAB)}
            toolbar = sorted((bounds for cid, bounds in native.items()
                              if cid != NEW_TAB or not expanded))
            require(all(bounds[3] <= page[1] for bounds in toolbar)
                    and all(a[2] <= b[0] for a, b in zip(toolbar, toolbar[1:])),
                    f"{mode}: toolbar overlaps the page or itself: {toolbar}, page {page}")
            for cid in SIDEBAR:
                require(bool(win.visible(browser.controls[cid])) == expanded,
                        f"{mode}: unexpected sidebar control {cid} visibility")
            if not expanded:
                return {"client": client, "page": page}
            tabs = checked_rect(browser.controls[TABS], "tab ListBox")
            edit = checked_rect(browser.controls[TAB_FILTER], "filter Edit")
            row_height = browser.message(TABS, 0x01A1)  # LB_GETITEMHEIGHT
            add = native[NEW_TAB]
            require(add[2] <= page[0] or add[3] <= page[1], f"New Tab button {add} overlaps page {page}")
            require(max(tabs[2], edit[2]) <= page[0] and min(tabs[1], edit[1]) >= page[1],
                    f"Tab list {tabs} and filter {edit} are not beside page {page}")
            require(row_height > 0 and tabs[3] - tabs[1] >= 2 * row_height
                    and (edit[3] <= tabs[1] or tabs[3] <= edit[1]),
                    f"Sidebar does not have room for the filter and two vertical tab rows: {edit}, {tabs}")
            return {"client": client, "page": page, "tabs": tabs, "filter": edit}

        def toggle_cycle(observe_motion=False):
            opened = snapshot(True)
            open_x = opened["page"][0]

            def intermediate(label):
                def sample():
                    x = browser.in_client(panel)[0]
                    return x if inset + tolerance < x < open_x - tolerance else None
                return browser.wait(label, sample, timeout=2)

            def wait_open():
                browser.wait("sidebar returns to its final position", lambda: (
                    all(win.visible(browser.controls[c]) for c in SIDEBAR)
                    and browser.in_client(panel) == opened["page"]
                ))

            positions = {}
            browser.click(SIDEBAR_TOGGLE)
            if observe_motion and animated:
                positions["closing_x"] = intermediate("an intermediate closing position")
            browser.wait("sidebar collapsed into its final position", lambda: (
                not any(win.visible(browser.controls[c]) for c in SIDEBAR)
                and abs(browser.in_client(panel)[0] - inset) <= tolerance
            ))
            closed = snapshot(False)
            require(opened["page"][0] - closed["page"][0] >= opened["tabs"][2] - opened["tabs"][0] - tolerance,
                    f"Collapsing the sidebar did not reclaim its width: {opened['page']} -> {closed['page']}")
            browser.click(SIDEBAR_TOGGLE)
            if observe_motion and animated:
                positions["opening_x"] = intermediate("an intermediate opening position")
            wait_open()
            require(snapshot(True) == opened, "Reopening the sidebar did not restore its layout")
            if observe_motion and animated:
                browser.click(SIDEBAR_TOGGLE)
                positions["before_reversal_x"] = intermediate("sidebar in motion before reversal")
                browser.click(SIDEBAR_TOGGLE)
                wait_open()
                require(snapshot(True) == opened, "Reversing the sidebar did not restore its layout")
            result = {"expanded": opened, "collapsed": closed}
            if observe_motion:
                result["motion"] = {"windows_animations_enabled": animated, **positions}
            return result

        result = {"default": toggle_cycle(observe_motion=True)}
        left, top, right, bottom = win.rect(hwnd)
        width, height = right - left, bottom - top
        narrow = max(width * 2 // 3, 690 * dpi // 96)
        if narrow < width:
            flags = 0x0016  # SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
            win.check(win.set_window_pos(hwnd, None, 0, 0, narrow, height, flags))
            require(win.rect(hwnd, client=True)[2] < result["default"]["expanded"]["client"][2],
                    "Could not narrow the Flit window for the second layout check")
            result["narrow"] = toggle_cycle()
            win.check(win.set_window_pos(hwnd, None, 0, 0, width, height, flags))
        else:
            result["narrow"] = {"status": "skipped", "reason": "default window is already narrow at this DPI"}
        return result


def filter_tabs(browser, first_panel, second_panel):
    def active(path, panel):
        require(browser.address() == browser.site.origin + path and browser.page_panel() == panel,
                f"Filter changed or hid the active page {path}")

    def filter_to(query, count):
        browser.set_text(TAB_FILTER, query)
        browser.wait(f"{count} visible tabs for filter {query!r}", lambda: (
            browser.control_text(TAB_FILTER) == query and browser.tab_count() == count
        ))

    def clear_with(key, index, path, panel):
        browser.press(TAB_FILTER, key)
        browser.wait(f"filter cleared with tab {path} active", lambda: (
            browser.control_text(TAB_FILTER) == "" and browser.tab_count() == 2
            and browser.selected() == index and browser.address() == browser.site.origin + path
            and browser.page_panel() == panel
        ))

    filter_to("/tab", 1)  # URL only; the fixture title is Orchid.
    require(browser.selected() == 0, "Filtered ListBox did not remap the active tab to visible row zero")
    active("/tab", second_panel)
    clear_with(0x1B, 1, "/tab", second_panel)  # Escape leaves the active page alone.

    filter_to("Flit smoke:", 2)
    active("/tab", second_panel)
    clear_with(0x0D, 0, "/first", first_panel)  # Enter picks the first of two matches.

    filter_to("Orchid", 1)  # Title only; the active /first page is not a match.
    active("/first", first_panel)
    clear_with(0x0D, 1, "/tab", second_panel)


def renderer_tokens(process):
    win = process.win
    pids = process.pids()
    if not pids:
        return {"status": "unavailable", "reason": "no owned processes to inspect"}
    where = " OR ".join(f"ProcessId={pid}" for pid in pids)
    query = (f"Get-CimInstance Win32_Process -Filter '{where}' -ErrorAction Stop | "
             r"Where-Object { $_.CommandLine -match '(^|\s)--type=renderer(\s|$)' } | "
             "ForEach-Object { $_.ProcessId }")
    try:
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", query],
            stdin=subprocess.DEVNULL, capture_output=True, timeout=8,
            creationflags=subprocess.CREATE_NO_WINDOW, check=True, text=True,
        )
    except (OSError, subprocess.SubprocessError) as error:
        return {"status": "unavailable", "reason": f"renderer lookup failed: {type(error).__name__}"}

    def read(pid):
        handle = win.check(win.open_process(0x1000, False, pid))  # QUERY_LIMITED_INFORMATION
        token = W.HANDLE()
        try:
            member = W.BOOL()
            win.check(win.in_job(handle, process.job, C.byref(member)))
            require(member.value, "PID no longer belongs to this run")
            win.check(win.open_token(handle, 0x0008, C.byref(token)))  # TOKEN_QUERY
            size = W.DWORD()
            win.token_info(token, 25, None, 0, C.byref(size))  # TokenIntegrityLevel
            require(0 < size.value <= 4096, "Windows did not return a token label size")
            label = C.create_string_buffer(size.value)
            win.check(win.token_info(token, 25, label, len(label), C.byref(size)))
            sid = C.cast(label, C.POINTER(C.c_void_p))[0]
            last = win.sid_count(sid)[0] - 1
            container = W.DWORD()
            win.check(win.token_info(token, 29, C.byref(container), C.sizeof(container), C.byref(size)))
            return {"integrity_rid": win.sid_part(sid, last)[0],
                    "restricted": bool(win.restricted(token)), "app_container": bool(container.value)}
        finally:
            if token.value:
                win.native.CloseHandle(token.value)
            win.native.CloseHandle(handle)

    renderer_pids = {int(line) for line in result.stdout.splitlines() if line.strip().isascii()
                     and line.strip().isdecimal()}
    observations = []
    for pid in sorted(renderer_pids.intersection(pids)):
        try:
            observations.append({"pid": pid, **read(pid)})
        except (OSError, RuntimeError) as error:
            observations.append({"pid": pid, "unavailable": str(error)})
    if any("integrity_rid" in item for item in observations):
        return {"status": "observed", "renderers": observations}
    return {"status": "unavailable", "reason": "no owned renderer token could be read", "renderers": observations}


def capture_window(browser, capture_file):
    """Capture only the owned HWND using PrintWindow; never copy the desktop DC."""
    from queue import Empty, Queue

    result = Queue(1)

    def read():
        from PIL import Image

        win, hwnd = browser.win, browser.hwnd
        require(win.pid(hwnd) == browser.process.pid, "capture window is no longer owned")
        user = C.WinDLL("user32", use_last_error=True)
        gdi = C.WinDLL("gdi32", use_last_error=True)
        _, _, width, height = win.rect(hwnd, client=True)
        panel = browser.page_panel()
        require(panel, "No visible native page panel to capture")
        page = browser.in_client(panel)
        left, top, right, bottom = page
        require(width > 0 and height > 0 and width * height <= 16_000_000, "invalid capture dimensions")
        require(0 <= left < right <= width and 0 <= top < bottom <= height,
                f"Native page {page} falls outside the capture {(width, height)}")
        handle, ptr = W.HANDLE, C.c_void_p
        create_dc = api(gdi, "CreateCompatibleDC", handle, handle)
        delete_dc = api(gdi, "DeleteDC", W.BOOL, handle)
        create_bitmap = api(gdi, "CreateDIBSection", handle, handle, ptr, W.UINT, C.POINTER(ptr), handle, W.DWORD)
        delete_bitmap = api(gdi, "DeleteObject", W.BOOL, handle)
        select = api(gdi, "SelectObject", handle, handle, handle)
        print_window = api(user, "PrintWindow", W.BOOL, W.HWND, handle, W.UINT)
        dc = win.check(create_dc(None))
        bitmap = previous = None
        try:
            # Top-down 32-bit BI_RGB DIB. PrintWindow renders the target into this private bitmap.
            header = struct.pack("<IiiHHIIiiII", 40, width, -height, 1, 32, 0, 0, 0, 0, 0, 0)
            pixels = ptr()
            bitmap = win.check(create_bitmap(dc, C.cast(C.c_char_p(header), ptr), 0, C.byref(pixels), None, 0))
            previous = win.check(select(dc, bitmap))
            win.check(print_window(hwnd, dc, 3))  # PW_CLIENTONLY | PW_RENDERFULLCONTENT
            frame = Image.frombytes("RGB", (width, height), C.string_at(pixels, width * height * 4), "raw", "BGRX")
            sample = frame.crop(page).resize((128, 128), getattr(Image, "Resampling", Image).NEAREST)
            colors = {color: count for count, color in sample.getcolors(128 * 128)}
            matches = [colors.get(color, 0) for color in ((23, 49, 69), (53, 184, 166))]
            report = {"size": [width, height], "page_rect": page, "fixture_color_samples": matches}
            if min(matches) < 8:
                return {**report, "status": "unavailable", "reason": "PrintWindow did not show both fixture colors"}, None
            return {**report, "status": "observed"}, frame if capture_file else None
        finally:
            if previous:
                select(dc, previous)
            if bitmap:
                delete_bitmap(bitmap)
            delete_dc(dc)

    def worker():
        try:
            with browser.win.window_coordinates(browser.hwnd):
                captured = read()
            result.put(captured)
        except (ImportError, OSError, RuntimeError, ValueError) as error:
            result.put(({"status": "unavailable", "reason": str(error)}, None))

    threading.Thread(target=worker, daemon=True).start()
    try:
        report, frame = result.get(timeout=5)
    except Empty:
        return {"status": "unavailable", "reason": "owned-window PrintWindow timed out"}
    if frame is not None:
        try:
            capture_file.parent.mkdir(parents=True, exist_ok=True)
            with capture_file.open("xb") as output:
                frame.save(output, format="PNG")
        except OSError as error:
            report["file_error"] = str(error)
        else:
            report["file"] = str(capture_file.absolute())
    return report


def page_counts(site, path):
    return site.count(path), site.count("/shown" + path)


def verify_backdrop(browser):
    dwm = C.WinDLL("dwmapi")
    get = api(dwm, "DwmGetWindowAttribute", C.c_long, W.HWND, W.DWORD, C.c_void_p, W.DWORD)

    def mode():
        material = C.c_int()
        result = get(browser.hwnd, 38, C.byref(material), C.sizeof(material))
        return material.value if result >= 0 else None

    initial = mode()
    if initial != 4:
        return {"status": "not observed", "system_backdrop": initial}
    browser.win.message(browser.hwnd, 0x0111, 139)
    browser.wait("DWM system backdrop switches to Acrylic", lambda: mode() == 3)
    browser.win.message(browser.hwnd, 0x0111, 139)
    browser.wait("DWM system backdrop switches back to Mica Alt", lambda: mode() == 4)
    return {"status": "configured and read back", "mica_alt": 4, "acrylic": 3,
            "scope": "The DWM attribute is observable; the final desktop compositor blend is not measured."}


def smoke(exe, work, capture, capture_file, collector, evidence):
    require(exe.is_file(), f"Flit executable does not exist: {exe}")
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    win = Win32()
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-win32-", dir=work)) / "profile"
    profile.mkdir()
    evidence["profile"] = str(profile)
    steps = evidence["steps"]
    with fixture() as site:
        args = [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"]
        evidence["origin"] = site.origin
        launches = evidence["launches"]
        try:
            first = {}
            launches.append(first)
            with OwnedProcess(win, exe, args, first) as process:
                browser = Browser(process, site)
                browser.page("/start", require_request=True)
                browser.wait("one selected tab at startup", lambda: browser.tab_count() == 1 and browser.selected() == 0)
                steps.append("startup fixture and native controls")
                evidence["dwm_backdrop"] = verify_backdrop(browser)
                evidence["sidebar_layout"] = sidebar_layout(browser)
                steps.append("sidebar and native page fit with the sidebar expanded and collapsed")

                browser.navigate("/edit")
                draft = site.origin + "/first"
                browser.edit_address(draft)
                require(browser.address() == draft, "Could not set a pending address")
                site.release_edit.set()
                browser.wait("async page title while address is edited", lambda: browser.has_page_title("Flit edit race complete"))
                require(browser.address() == draft, "The previous page's update overwrote an edited address")
                browser.page("/first", lambda: browser.click(GO), require_request=True)
                steps.append("page updates preserve an unsubmitted address")
                browser.navigate("/second")
                browser.page("/first", lambda: browser.click(BACK))
                browser.page("/second", lambda: browser.click(FORWARD))
                browser.page("/second", lambda: browser.click(RELOAD), require_request=True)
                steps.append("address and Go, Back, Forward, Reload")

                first_panel = browser.wait("first tab panel", browser.page_panel)
                browser.click(NEW_TAB)
                browser.wait("two tabs", lambda: browser.tab_count() == 2)
                browser.switch(1)
                browser.navigate("/tab")
                browser.wait("Orchid fixture title", lambda: browser.has_page_title(TAB_TITLE))
                second_panel = browser.wait("second tab panel", browser.page_panel)
                require(second_panel != first_panel, "New tab did not show a different native page panel")
                browser.switch(0, "/second")
                browser.page("/first", lambda: browser.click(BACK))
                browser.switch(1, "/tab")
                steps.append("two tabs, vertical keyboard switching, and active history")

                filter_tabs(browser, first_panel, second_panel)
                steps.append("filter by URL and title, visible index mapping, Enter and Escape")

                browser.close_row(0)  # Close the inactive /first row; /tab must remain open.
                browser.wait("second tab still active after closing the first row", lambda: (
                    browser.tab_count() == 1 and browser.selected() == 0
                    and browser.address() == site.origin + "/tab" and browser.page_panel() == second_panel
                ))
                steps.append("native row close button closes the targeted inactive tab")

                browser.navigate("/prime")
                browser.wait("prime script and synthetic cookie", lambda: site.count("/executed/prime", cookie=True))
                cache_hits = site.count("/cache.js")
                require(cache_hits == 1, f"Expected one script download before restart; got {cache_hits}")
                evidence["cache_requests_before_restart"] = cache_hits
                evidence["renderer_token"] = renderer_tokens(process)
                if collector:
                    sampled = subprocess.run(
                        ["powershell.exe", "-NoProfile", "-NonInteractive", "-File", str(collector),
                         "-RootProcessId", str(process.pid), "-SampleSeconds", "1"],
                        stdin=subprocess.DEVNULL, capture_output=True, timeout=20,
                        creationflags=subprocess.CREATE_NO_WINDOW, check=True,
                    )
                    metrics = json.loads(sampled.stdout.decode("utf-8-sig"))
                    require(metrics["status"] != "error" and metrics["end"]["totals"]["privateCommitBytes"]["complete"],
                            "Memory collector did not return complete private commit for the Flit process tree")
                    evidence["memory_sample"] = metrics
                if capture:
                    evidence["window_capture"] = capture_window(browser, capture_file)
                process.normal_close(browser.hwnd)
                steps.append("prime cookie and HTTP cache; close first process tree by WM_CLOSE")

            second = {}
            launches.append(second)
            baseline = page_counts(site, "/start")
            with OwnedProcess(win, exe, args, second) as process:
                browser = Browser(process, site)
                browser.page("/start", require_request=True, baseline=baseline)
                browser.navigate("/restart")
                browser.wait("restarted page to execute the cacheable script", lambda: site.count("/executed/restart"))
                require(site.count("/restart", cookie=True) > 0, "Restart navigation did not send the persistent cookie")
                require(site.count("/executed/restart", cookie=True) > 0, "Restart script did not send the persistent cookie")
                after = site.count("/cache.js")
                evidence["cache_requests_after_restart"] = after
                require(after == cache_hits, f"Cache script was downloaded again after restart ({cache_hits} -> {after})")
                steps.append("same profile persisted cookie and reused the disk HTTP cache")
                browser.click(NEW_TAB)
                browser.wait("two live tabs before window shutdown", lambda: browser.tab_count() == 2)
                process.normal_close(browser.hwnd)
                steps.append("close second process tree with two open tabs by WM_CLOSE")
        finally:
            evidence["fixture"] = site.report()


def self_check():
    from http.client import HTTPConnection

    with fixture() as site:
        def get(path, headers=None):
            connection = HTTPConnection("127.0.0.1", site.server_port, timeout=3)
            try:
                connection.request("GET", path, headers=headers or {})
                response = connection.getresponse()
                return response.status, response.headers, response.read()
            finally:
                connection.close()

        status, headers, body = get("/prime")
        require(status == 200 and b'"pageshow"' in body and b'"/cache.js"' in body, "fixture HTML is incomplete")
        cookie = headers["Set-Cookie"]
        require(cookie.startswith(site.cookie + ";") and "Max-Age=" in cookie, "fixture cookie is not persistent")
        status, headers, body = get("/cache.js")
        require(status == 200 and "max-age=3600" in headers["Cache-Control"] and body == ASSET, "fixture cache headers are invalid")
        require(get("/executed/restart", {"Cookie": "flit_smoke=wrong"})[0] == 204, "fixture execution endpoint failed")
        require(get("/executed/restart", {"Cookie": site.cookie})[0] == 204, "fixture did not accept its own cookie")
        require(site.count("/executed/restart") == 2 and site.count("/executed/restart", cookie=True) == 1,
                "fixture request or cookie accounting is wrong")
        _, _, tab = get("/tab")
        require(f"<title>{TAB_TITLE}</title>".encode("ascii") in tab and "/tab" not in TAB_TITLE,
                "filter fixture must have distinct title and URL search terms")
        with site.lock:
            site.remaining = 0
        require(get("/start")[0] == 429, "fixture request limit failed")

    report = {}
    child = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); time.sleep(30)"
    with OwnedProcess(Win32(), Path(sys.executable), ["-B", "-c", child], report) as process:
        until("self-check job contains root and child", lambda: len(process.pids()) == 2, 5, process.alive)
        require(process.pid in process.pids(), "self-check root missing from the job")
    require(report["cleanup_needed"] and not report["leftover_pids"], "self-check job cleanup failed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, help="Flit.exe from the official bootstrap")
    parser.add_argument("--work", type=Path, help="workspace directory for a new isolated profile")
    parser.add_argument("--evidence", type=Path, help="optional new JSON file for run evidence")
    parser.add_argument("--capture", action="store_true", help="check fixture pixels in the owned Flit window")
    parser.add_argument("--capture-file", type=Path, help="new PNG file for the owned window; requires --capture and fixture colors")
    parser.add_argument("--collector", type=Path, help="Phase 0 read-only Windows memory collector script")
    parser.add_argument("--self-check", action="store_true", help="verify the fixture and process ownership without Flit.exe")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    if args.capture_file and not args.capture:
        parser.error("--capture-file requires --capture")
    if args.self_check:
        if args.exe or args.work or args.evidence or args.capture or args.collector:
            parser.error("--self-check runs on its own")
        self_check()
        print("PASS: loopback fixture, request and cookie accounting, and owned process-tree cleanup")
        return 0
    if not (args.exe and args.work):
        parser.error("--exe and --work are required for a browser run")
    if args.evidence and args.evidence.exists():
        parser.error(f"evidence file already exists: {args.evidence}")
    if args.capture_file:
        if args.capture_file.exists() or args.capture_file.is_symlink():
            parser.error(f"capture file already exists: {args.capture_file}")
        if args.evidence and args.capture_file.resolve() == args.evidence.resolve():
            parser.error("--capture-file and --evidence must name different files")

    evidence = {"result": "fail", "steps": [], "launches": [],
                "renderer_token": {"status": "unavailable", "reason": "inspection was not reached"},
                "window_capture": {"status": "unavailable", "reason": "capture was not reached" if args.capture
                                   else "--capture was not requested"}}
    try:
        collector = args.collector.resolve(strict=True) if args.collector else None
        smoke(args.exe.resolve(), args.work.resolve(), args.capture, args.capture_file, collector, evidence)
        evidence["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        evidence["error"] = f"{type(error).__name__}: {error}"

    if "error" in evidence:
        print("FAIL: " + evidence["error"], file=sys.stderr)
    else:
        print("PASS: Flit Win32 E2E")
    for step in evidence["steps"]:
        print(f"  {step}")
    if "fixture" in evidence:
        print(f"  HTTP requests: {evidence['fixture']['requests']}")
    for name in ("renderer_token", "window_capture"):
        item = evidence[name]
        print(f"  {name}: {item['status']} ({item.get('reason') or item.get('renderers') or item.get('fixture_color_samples')})")
    if "file" in evidence["window_capture"]:
        print(f"  Window PNG: {evidence['window_capture']['file']}")
    elif "file_error" in evidence["window_capture"]:
        print(f"  Window PNG unavailable: {evidence['window_capture']['file_error']}")
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        with args.evidence.open("x", encoding="utf-8") as output:
            json.dump(evidence, output, indent=2)
            output.write("\n")
        print(f"  Evidence: {args.evidence.resolve()}")
    return int(evidence["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
