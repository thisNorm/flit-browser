"""Exercise fixture-only Flit Vault sign-in over Windows MCP and loopback HTTPS."""

import argparse
from base64 import b64encode
from collections import Counter
from contextlib import ExitStack, contextmanager
import ctypes as C
from ctypes import wintypes as W
from hashlib import sha256
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import ssl
import subprocess
import sys
import tempfile
from threading import Lock, Thread
from urllib.parse import parse_qs

sys.dont_write_bytecode = True
from windows_agent_mcp import Mcp, StdioProcess, handshake, pipe
from windows_smoke import api, require


HOST = "127.0.0.1"
LOGIN_TITLE = "Flit Vault login fixture"
SIGNED_IN = "Vault fixture sign-in succeeded"
PROTECTED = "Vault fixture protected content"
DENIED = "Vault fixture session missing"
OTP_TITLE = "Vault fixture additional verification"
GENERIC, SESSION, NOT_FOUND, UI_FORBIDDEN = 1, 1, 1168, 1
BYTE_PTR = C.POINTER(C.c_ubyte)


class DataBlob(C.Structure):
    _fields_ = [("size", W.DWORD), ("data", BYTE_PTR)]


class Credential(C.Structure):
    _fields_ = [("flags", W.DWORD), ("type", W.DWORD), ("target", W.LPWSTR), ("comment", W.LPWSTR),
                ("last_written", W.FILETIME), ("size", W.DWORD), ("blob", BYTE_PTR),
                ("persist", W.DWORD), ("attributes_count", W.DWORD), ("attributes", C.c_void_p),
                ("target_alias", W.LPWSTR), ("username", W.LPWSTR)]


class VaultEntry:
    def __init__(self, port):
        require(type(port) is int and 1024 <= port <= 65535, "Expected a bound loopback fixture port")
        self.target = f"FlitBrowser/AgentVault/v1/https://{HOST}:{port}"
        advapi = C.WinDLL("advapi32", use_last_error=True)
        self.read = api(advapi, "CredReadW", W.BOOL, W.LPCWSTR, W.DWORD, W.DWORD,
                        C.POINTER(C.c_void_p))
        self.free = api(advapi, "CredFree", None, C.c_void_p)
        self.write = api(advapi, "CredWriteW", W.BOOL, C.POINTER(Credential), W.DWORD)
        self.delete = api(advapi, "CredDeleteW", W.BOOL, W.LPCWSTR, W.DWORD, W.DWORD)

    def require_absent(self):
        record = C.c_void_p()
        if self.read(self.target, GENERIC, 0, C.byref(record)):
            self.free(record)  # Existence only; never dereference a pre-existing record.
            raise RuntimeError(f"Fixture target already exists: {self.target}")
        if (error := C.get_last_error()) != NOT_FOUND:
            raise C.WinError(error)

    @contextmanager
    def seed(self, username, password):
        crypt32 = C.WinDLL("crypt32", use_last_error=True)
        protect = api(crypt32, "CryptProtectData", W.BOOL, C.POINTER(DataBlob), W.LPCWSTR,
                      C.POINTER(DataBlob), C.c_void_p, C.c_void_p, W.DWORD, C.POINTER(DataBlob))
        local_free = api(C.WinDLL("kernel32", use_last_error=True), "LocalFree", C.c_void_p, C.c_void_p)
        user = username.encode("utf-8")
        payload = len(user).to_bytes(2, "little") + user + password.encode("utf-8")
        plaintext = C.create_string_buffer(payload, len(payload))
        target = C.create_unicode_buffer(self.target)  # DPAPI entropy includes the UTF-16 NUL.
        data = DataBlob(C.sizeof(plaintext), C.cast(plaintext, BYTE_PTR))
        entropy = DataBlob(C.sizeof(target), C.cast(target, BYTE_PTR))
        encrypted = DataBlob()
        owned = False
        try:
            if not protect(C.byref(data), None, C.byref(entropy), None, None, UI_FORBIDDEN,
                           C.byref(encrypted)):
                raise C.WinError(C.get_last_error())
            require(bool(encrypted.data) and 0 < encrypted.size <= 2560, "Invalid DPAPI fixture blob")
            self.require_absent()
            record = Credential(type=GENERIC, target=self.target, size=encrypted.size,
                                blob=encrypted.data, persist=SESSION)
            if not self.write(C.byref(record), 0):
                raise C.WinError(C.get_last_error())
            owned = True
            yield
        finally:
            C.memset(plaintext, 0, C.sizeof(plaintext))
            if encrypted.data:
                local_free(encrypted.data)
            if owned and not self.delete(self.target, GENERIC, 0):
                if (error := C.get_last_error()) != NOT_FOUND:
                    raise RuntimeError(f"Could not remove owned fixture credential {self.target}: {C.WinError(error)}")


class Fixture(ThreadingHTTPServer):
    allow_reuse_address = False

    def __init__(self, tls):
        super().__init__((HOST, 0), Handler)
        self.tls = tls
        self.origin = f"https://{HOST}:{self.server_port}"
        self.fields = {name: secrets.token_hex(16) for name in ("username", "password", "csrf")}
        self.session = secrets.token_hex(16)
        self.cookie = f"flit_vault_fixture={self.session}"
        self.lock = Lock()
        self.hits = Counter()

    def process_request_thread(self, request, address):
        request.settimeout(5)
        try:
            request = self.tls.wrap_socket(request, server_side=True)
        except OSError:
            self.close_request(request)
            return
        super().process_request_thread(request, address)

    def record(self, event):
        with self.lock:
            self.hits[event] += 1

    def counts(self):
        with self.lock:
            return self.hits.copy()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def respond(self, status, body="", headers=()):
        payload = body.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", "no-store")
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        if payload:
            self.wfile.write(payload)

    def do_GET(self):
        site = self.server
        if self.path in ("/login", "/other-action", "/other-redirect", "/retry-redirect", "/otp-login", "/no-cookie-login"):
            action = site.foreign + "/login" if self.path == "/other-action" else self.path
            self.respond(200, f'''<!doctype html><title>{LOGIN_TITLE}</title><h1>{LOGIN_TITLE}</h1>
<form method="post" action="{action}">
<label>Username <input name="username" autocomplete="username"></label>
<label>Password <input type="password" name="password" autocomplete="current-password"></label>
<input type="hidden" name="csrf" value="{site.fields['csrf']}">
<button type="submit">Sign in</button></form>''')
            return
        if self.path == "/otp":
            site.record("GET /otp")
            self.respond(200, f'<!doctype html><title>{OTP_TITLE}</title><h1>{OTP_TITLE}</h1>'
                              '<input autocomplete="one-time-code" name="otp">')
            return
        if self.path not in ("/signed-in", "/protected", "/without-session"):
            self.respond(404)
            return
        has_cookie = site.cookie in (part.strip() for part in self.headers.get("Cookie", "").split(";"))
        site.record(f"GET {self.path} {'with' if has_cookie else 'without'} cookie")
        if not has_cookie:
            self.respond(403, f"<!doctype html><title>{DENIED}</title><h1>{DENIED}</h1>")
        elif self.path == "/signed-in":
            self.respond(200, f'<!doctype html><title>{SIGNED_IN}</title><h1>{SIGNED_IN}</h1>'
                              '<a href="/protected">Protected fixture</a>')
        else:
            self.respond(200, f"<!doctype html><title>{PROTECTED}</title><h1>{PROTECTED}</h1>")

    def do_POST(self):
        site = self.server
        site.record("POST")
        try:
            size = int(self.headers.get("Content-Length", "0"))
            fields = parse_qs(self.rfile.read(size).decode("ascii"), strict_parsing=True,
                              keep_blank_values=True, max_num_fields=3) if 0 < size <= 4096 else {}
        except (ValueError, OSError):
            fields = {}
        if (self.path not in ("/login", "/other-redirect", "/retry-redirect", "/otp-login", "/no-cookie-login")
                or self.headers.get_content_type() != "application/x-www-form-urlencoded"
                or fields != {name: [value] for name, value in site.fields.items()}):
            self.respond(400)
            return
        if self.path == "/other-redirect":
            self.respond(303, headers=(("Location", site.foreign + "/signed-in"),))
            return
        if self.path == "/retry-redirect":
            self.respond(307, headers=(("Location", site.foreign + "/login"),))
            return
        if self.path == "/otp-login":
            self.respond(303, headers=(("Location", "/otp"),))
            return
        if self.path == "/no-cookie-login":
            self.respond(303, headers=(("Location", "/without-session"),))
            return
        site.record("accepted login POST")
        self.respond(303, headers=(("Location", "/signed-in"),
                                   ("Set-Cookie", site.cookie + "; Path=/; Secure; HttpOnly; SameSite=Lax")))


@contextmanager
def fixture(tls):
    with Fixture(tls) as site:
        thread = Thread(target=site.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
        thread.start()
        try:
            yield site
        finally:
            site.shutdown()
            thread.join()


class VaultMcp(Mcp):
    def __init__(self, process, stdin, stdout, stderr, site):
        super().__init__(process, stdin, stdout, stderr)
        self.markers = tuple(value.encode("ascii") for value in (*site.fields.values(), site.session))

    def pump(self):
        super().pump()
        require(not any(marker in self.output or marker in self.stderr_tail for marker in self.markers),
                "Flit exposed a fixture username, password, CSRF token, or session cookie")


def open_page(mcp, url, title, *, login_form=False):
    opened = mcp.tool("flit_open", {"url": url}, timeout=27)
    tab = opened.get("tab")
    require(type(tab) is int and opened.get("url") == url, "flit_open did not return the requested tab")
    mcp.ready_snapshot(tab, url, title, title, login_form=login_form)
    return tab


def sign_in(mcp, tab, status, reason):
    result = mcp.tool("flit_sign_in", {"tab": tab}, timeout=16)
    require((result.get("tab"), result.get("status"), result.get("reason")) == (tab, status, reason),
            f"Expected flit_sign_in {status}/{reason}, got {result!r}")


def exercise(mcp, site, entry, credential_stack, report):
    handshake(mcp)
    login, signed_in, protected = (site.origin + path for path in ("/login", "/signed-in", "/protected"))
    report["stage"] = "missing credential and unauthenticated page"
    tab = open_page(mcp, login, LOGIN_TITLE, login_form=True)
    sign_in(mcp, tab, "action_required", "not_saved")
    mcp.ready_snapshot(tab, login, LOGIN_TITLE, LOGIN_TITLE, login_form=True)
    denied_tab = open_page(mcp, protected, DENIED)
    mcp.tool("flit_close_tab", {"tab": denied_tab})
    require(site.counts() == {"GET /protected without cookie": 1}, "Unexpected request before seeding")

    credential_stack.enter_context(entry.seed(site.fields["username"], site.fields["password"]))
    report["stage"] = "reject other origins and stop for a missing session or OTP"
    for path, reason in (("/other-action", "different_origin"),
                         ("/other-redirect", "login_not_verified"),
                         ("/retry-redirect", "login_not_verified"),
                         ("/no-cookie-login", "verification_page_failed"),
                         ("/otp-login", "additional_verification")):
        other = open_page(mcp, site.origin + path, LOGIN_TITLE, login_form=True)
        sign_in(mcp, other, "action_required", reason)
        mcp.tool("flit_close_tab", {"tab": other})
    require(site.counts()["POST"] == 4, "Unsafe form submitted or a POST was retried")
    report["cross_origin_and_mfa_stopped"] = True
    report["stage"] = "submit saved synthetic credential and verify continuation"
    sign_in(mcp, tab, "submitted", "destination_verified")
    page = mcp.ready_snapshot(tab, signed_in, SIGNED_IN, SIGNED_IN)
    link = next((link for link in page["links"] if link.get("url") == protected), None)
    require(link is not None and type(link.get("id")) is int, "Signed-in page lacks the protected link")
    report["stage"] = "verify protected page and exactly one POST"
    mcp.tool("flit_follow_link", {"tab": tab, "snapshot": page["snapshot"], "link": link["id"]})
    mcp.ready_snapshot(tab, protected, PROTECTED, PROTECTED)
    mcp.finish()
    require(site.counts() == {"GET /protected without cookie": 1, "POST": 5, "accepted login POST": 1,
                              "GET /without-session without cookie": 1, "GET /otp": 1,
                              "GET /signed-in with cookie": 1, "GET /protected with cookie": 1},
            "Expected one authenticated continuation and one authenticated protected GET")


def run(exe, tls, report):
    with fixture(tls) as site, fixture(tls) as foreign, tempfile.TemporaryDirectory(
            prefix="flit-vault-fixture-", ignore_cleanup_errors=True) as temp:
        site.foreign = foreign.origin
        report["origin"] = site.origin
        entry = VaultEntry(site.server_port)
        entry.require_absent()
        root = Path(temp)
        env = {key: os.environ[key] for key in ("SystemRoot", "SystemDrive", "WINDIR") if key in os.environ}
        for key in ("LOCALAPPDATA", "APPDATA", "USERPROFILE", "TEMP", "TMP"):
            path = root / ("temp" if key in ("TEMP", "TMP") else key)
            path.mkdir(exist_ok=True)
            env[key] = str(path)
        try:
            with ExitStack() as credential_stack, ExitStack() as process_stack:
                child_in, parent_in = pipe(process_stack)
                parent_out, child_out = pipe(process_stack)
                parent_err, child_err = pipe(process_stack)
                process = process_stack.enter_context(StdioProcess(
                    [exe, "--flit-agent"], env, (child_in, child_out, child_err), report))
                exercise(VaultMcp(process, parent_in, parent_out, parent_err, site),
                         site, entry, credential_stack, report)
            entry.require_absent()
            require(not foreign.counts(), "A blocked login redirected to another origin")
            report["blocked_origin_requests"] = 0
            report["synthetic_vault_entry_removed"] = True
            require(not report["cleanup_needed"], "Flit needed forced cleanup after EOF")
            require(not (Path(env["LOCALAPPDATA"]) / "Flit Browser").exists(), "Flit created a normal profile")
            require(not list(Path(env["TEMP"]).glob("FlitAgent-*")), "Agent left its temporary session behind")
        finally:
            report["https_requests"] = dict(site.counts())


def fixture_build(dll, cert, openssl):
    def run_openssl(*args, input=None):
        return subprocess.run([openssl, *args], input=input, capture_output=True, check=True, timeout=10).stdout

    public_key = run_openssl("x509", "-in", str(cert), "-pubkey", "-noout")
    spki = b64encode(sha256(run_openssl("pkey", "-pubin", "-outform", "DER", input=public_key)).digest())
    image = dll.read_bytes()
    require(all(marker in image for marker in (b"Flit agent mode", b"flit_sign_in",
                                               b"ignore-certificate-errors-spki-list", spki)),
            "Flit.dll must be an agent build compiled with vault-fixture and FLIT_FIXTURE_SPKI for --cert")
    return sha256(image).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="fixture-only Flit.exe beside Flit.dll")
    parser.add_argument("--cert", required=True, type=Path, help="PEM certificate pinned in the fixture build")
    parser.add_argument("--key", required=True, type=Path, help="matching PEM key for the loopback fixture")
    parser.add_argument("--openssl", type=Path, default=Path(os.environ.get("ProgramFiles", "C:/Program Files"))
                        / "Git/usr/bin/openssl.exe", help="OpenSSL executable (defaults to Git for Windows)")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    exe, cert, key, openssl = (path.resolve() for path in (args.exe, args.cert, args.key, args.openssl))
    dll = exe.with_name("Flit.dll")
    if exe.name.lower() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("--exe must point to Flit.exe next to Flit.dll")
    if not all(path.is_file() for path in (cert, key, openssl)):
        parser.error("--cert, --key, and --openssl must point to files")
    report = {"result": "fail", "stage": "verify fixture build and TLS inputs"}
    try:
        report["dll_sha256"] = fixture_build(dll, cert, openssl)
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(cert, key)
        run(exe, tls, report)
        report.update(result="pass", stage="complete")
    except (RuntimeError, OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    sys.exit(main())
