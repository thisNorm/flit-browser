"""Check real page zoom and persistence with an owned Flit window and loopback page."""

import argparse
from contextlib import contextmanager
from ctypes import wintypes as W
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import tempfile
import threading

from windows_smoke import Browser, OwnedProcess, Win32, WORKSPACE, require


HTML = b"""<!doctype html><meta charset=utf-8><title>Flit zoom fixture</title>
<h1>Find this phrase</h1><script>
let last;
setInterval(() => {
  const current = [devicePixelRatio, innerWidth].join(',');
  if (last !== current) {
    last = current;
    fetch('/size?' + current, {cache:'no-store'}).catch(() => {});
  }
}, 50);
</script>"""


@contextmanager
def fixture():
    samples = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            size = self.path.partition("/size?")[2]
            if size:
                dpr, width = size.split(",")
                samples.append((float(dpr), int(width)))
                self.send_response(204)
                self.end_headers()
                return
            if self.path != "/":
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(HTML)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(HTML)

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
        thread.start()
        try:
            yield f"http://127.0.0.1:{server.server_port}/", samples
        finally:
            server.shutdown()
            thread.join(2)


def run(exe, work):
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-zoom-", dir=work)) / "profile"
    result = {}
    with fixture() as (url, samples):
        args = [f"--flit-profile={profile}", f"--flit-url={url}"]
        win = Win32()

        def wait_size(browser, start, dpr=None):
            def latest():
                if len(samples) <= start:
                    return None
                sample = samples[-1]
                return sample if dpr is None or abs(sample[0] - dpr) < .01 else None
            return browser.wait(f"renderer DPR {dpr}", latest, 7)

        def command(browser, cid):
            require(win.pid(browser.hwnd) == browser.process.pid, "Flit no longer owns the window")
            win.message(browser.hwnd, 0x0111, cid, 0)

        def change(browser, cid, dpr):
            before = len(samples)
            command(browser, cid)
            return wait_size(browser, before, dpr)

        with OwnedProcess(win, exe, args, {}) as process:
            browser = Browser(process, None)
            base = wait_size(browser, 0)
            enlarged = change(browser, 148, base[0] * 1.1)
            require(enlarged[1] < base[1], f"CSS viewport did not shrink at 110%: {base} → {enlarged}")
            process.normal_close(browser.hwnd)
            result.update(baseline=base, zoomed=enlarged)
        before = len(samples)
        with OwnedProcess(win, exe, args, {}) as process:
            browser = Browser(process, None)
            persisted = wait_size(browser, before, enlarged[0])
            smaller = change(browser, 149, base[0])
            reduced = change(browser, 149, base[0] * .9)
            require(reduced[1] > base[1], "90% zoom did not widen the CSS viewport")
            reset = change(browser, 150, base[0])
            require(abs(reset[1] - base[1]) <= 1, "Reset did not restore the CSS viewport")
            command(browser, 124)
            browser.controls[125] = win.get_item(browser.hwnd, 125)
            browser.controls[126] = win.get_item(browser.hwnd, 126)
            require(all(win.pid(browser.controls[cid]) == process.pid for cid in (125, 126)), "Missing owned find controls")
            browser.set_text(125, "Find this phrase")
            browser.wait("find reports match at default zoom", lambda: browser.control_text(126) in ("0/1", "1/1"))
            while_find = change(browser, 148, enlarged[0])
            command(browser, 129)
            change(browser, 150, base[0])
            after_find = change(browser, 148, enlarged[0])
            process.normal_close(browser.hwnd)
            result.update(persisted=persisted, smaller=smaller, reduced=reduced, reset=reset,
                          while_find=while_find, after_find=after_find)
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    exe = args.exe.resolve(strict=True)
    work = args.work.resolve()
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    print(json.dumps(run(exe, work), indent=2))
