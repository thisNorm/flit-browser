"""Verify background link deferral and foreground link return in an owned Flit window."""

import argparse
from hashlib import file_digest
from http.server import BaseHTTPRequestHandler
import json
from pathlib import Path
import sys
import tempfile
import time

sys.dont_write_bytecode = True
from windows_downloads import click_attachment, seed_profile
from windows_multi_session import clean_scratch
from windows_session import loaded_panels, select, send
from windows_smoke import Browser, OwnedProcess, WORKSPACE, Win32, fixture, page_counts, require


WM_COMMAND, DEFER_BACKGROUND = 0x0111, 168
WM_LBUTTONDOWN, WM_LBUTTONUP, MK_LBUTTON, MK_CONTROL = 0x0201, 0x0202, 0x0001, 0x0008
WM_MBUTTONDOWN, WM_MBUTTONUP, MK_MBUTTON = 0x0207, 0x0208, 0x0010
SOURCE, TARGET, TRUSTED_CLICK = "/source", "/target", "/trusted-middle-click"
TRUSTED_CTRL_CLICK = "/trusted-ctrl-click"
FOREGROUND_SOURCE, FOREGROUND_TARGET = "/foreground-source", "/foreground-target"
FOREGROUND_CLICK = "/trusted-foreground-click"
MOUSE_EVENTS = ("mousedown", "mouseup", "click", "auxclick")
TARGET_TITLE = "Flit background link target"
PAGES = {
    SOURCE: f'''<title>Flit background link source</title>
<style>a {{ position:fixed; inset:0; display:grid; place-items:center }}</style>
<a href="{TARGET}">Open background link</a>
<script>
const link = document.querySelector("a");
for (const type of {json.dumps(MOUSE_EVENTS)}) {{
    addEventListener(type, event => {{
        const onLink = link.contains(event.target);
        const details = new URLSearchParams({{
            button: event.button, buttons: event.buttons, trusted: event.isTrusted,
            ctrl: event.ctrlKey, shift: event.shiftKey, link: onLink,
            x: event.clientX, y: event.clientY, time: event.timeStamp,
        }});
        fetch("/input/" + type + "?" + details, {{keepalive: true}});
        if (event.isTrusted && onLink) {{
            if (type === "auxclick" && event.button === 1) fetch("{TRUSTED_CLICK}", {{keepalive: true}});
            if (type === "click" && event.button === 0 && event.ctrlKey)
                fetch("{TRUSTED_CTRL_CLICK}", {{keepalive: true}});
        }}
    }}, true);
}}
</script>''',
    TARGET: f"<title>{TARGET_TITLE}</title><h1>{TARGET_TITLE}</h1>",
    FOREGROUND_SOURCE: f'''<title>Flit foreground link source</title>
<style>a {{ position:fixed; inset:0; display:grid; place-items:center }}</style>
<a href="{FOREGROUND_TARGET}" target="_blank" rel="noopener">Open foreground link</a>
<script>document.querySelector("a").onclick = event => {{
    if (event.isTrusted && navigator.userActivation.isActive) fetch("{FOREGROUND_CLICK}");
}};</script>''',
    FOREGROUND_TARGET: "<title>Flit foreground link target</title><h1>Flit foreground link target</h1>",
}


class LinkHandler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_CONNECT(self):
        self.send_error(403)

    def do_GET(self):
        site = self.server
        # The fresh Chromium profile uses this server as a non-forwarding proxy.
        if not self.path.startswith(site.origin + "/"):
            self.send_error(403)
            return
        route = self.path[len(site.origin):]
        path = route.partition("?")[0]
        if path in PAGES:
            body = f'''<!doctype html><meta charset="utf-8"><link rel="icon" href="data:,">
{PAGES[path]}
<script>addEventListener("pageshow", () => requestAnimationFrame(() =>
    requestAnimationFrame(() => fetch("/shown" + location.pathname))));</script>'''.encode("ascii")
        elif (path.startswith("/shown") and path[len("/shown"):] in PAGES) or path in (
            TRUSTED_CLICK, TRUSTED_CTRL_CLICK, FOREGROUND_CLICK, *("/input/" + name for name in MOUSE_EVENTS)
        ):
            body = b""
        else:
            self.send_error(404)
            return
        with site.lock:
            site.hits[route] += 1
        self.send_response(200 if body else 204)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)


def post_link_click(browser, panel, attempts, *, ctrl=False):
    win, process = browser.win, browser.process

    def widgets(parent):
        for child in win.visible_children(parent):
            if win.class_name(child) == "Chrome_RenderWidgetHostHWND":
                yield child
            else:
                yield from widgets(child)

    matches = list(widgets(panel))
    require(len(matches) == 1, f"Expected one visible Chromium render widget, got {len(matches)}")
    widget = matches[0]
    with win.window_coordinates(widget):
        _, _, width, height = win.rect(widget, client=True)
        require(width > 0 and height > 0, "Flit render widget has no client area")
        x, y = width // 2, height // 2
        point = x | (y << 16)
        attempts.append({"gesture": "ctrl-left" if ctrl else "middle", "hwnd": widget,
                         "client_size": (width, height), "point": (x, y)})
        # Chromium reads MK_CONTROL from the mouse messages; no global keyboard state is changed.
        messages = ((WM_LBUTTONDOWN, MK_LBUTTON | MK_CONTROL), (WM_LBUTTONUP, MK_CONTROL)) if ctrl else (
            (WM_MBUTTONDOWN, MK_MBUTTON), (WM_MBUTTONUP, 0),
        )
        for message, buttons in messages:
            require(win.pid(widget) in process.pids(), "Flit render widget is no longer owned")
            win.check(win.post(widget, message, buttons, point))


def open_background_link(browser, panel, attempts):
    site = browser.site

    def progressed():
        return (site.count(TRUSTED_CLICK) or browser.tab_count() != 1
                or browser.address() != site.origin + SOURCE or site.count(TARGET))

    post_link_click(browser, panel, attempts)
    retry_at = time.monotonic() + 2
    browser.wait("middle click or retry deadline", lambda: progressed() or time.monotonic() >= retry_at, 3)
    if not progressed():
        post_link_click(browser, panel, attempts, ctrl=True)

    # The link's native action is the tab row; DOM reports are diagnostic and suppress unsafe retries.
    browser.wait("new tab after posted link gesture (see http_requests for mouse events)",
                 lambda: browser.tab_count() > 1)


def check_foreground_return(browser, report):
    site = browser.site
    select(browser, 0)  # Put a loaded, unrelated tab to the right of this source.
    browser.navigate(FOREGROUND_SOURCE)
    source = browser.wait("visible foreground link source", browser.page_panel)
    require(browser.tab_count() == 2 and browser.selected() == 0, "Source must not be the last tab")
    before = site.count(FOREGROUND_SOURCE)

    click_attachment(browser, 0)  # The only link covers the fixture viewport.
    browser.page(FOREGROUND_TARGET, require_request=True)
    require(site.count(FOREGROUND_CLICK) == 1 and browser.tab_count() == 3
            and browser.selected() == 2, "Expected a trusted foreground link to open a new tab")
    browser.close_row(browser.selected())
    browser.wait("foreground link tab closed", lambda: browser.tab_count() == 2)
    check = report["foreground_return"] = {
        "selected_row": browser.selected(), "address": browser.address(),
        "same_panel": browser.page_panel() == source,
        "source_requests_before": before, "source_requests_after": site.count(FOREGROUND_SOURCE),
    }
    require(check["selected_row"] == 0 and check["address"] == site.origin + FOREGROUND_SOURCE
            and check["same_panel"] and check["source_requests_after"] == before,
            f"Closing the link did not return to the still-loaded source: {check}")


def run(exe, scratch, report):
    profile = scratch / "profile"
    report.update(profile=str(profile), launch={})
    with fixture() as site:
        site.RequestHandlerClass = LinkHandler
        seed_profile(profile, scratch / "downloads", site.origin)
        # Start as an existing profile that explicitly allows speculative loading.
        (profile / "session.json").write_text(
            json.dumps({"entries": [], "selected": 0, "defer_background": False}), encoding="utf-8"
        )
        args = [f"--flit-profile={profile}", f"--flit-url={site.origin}{SOURCE}"]
        try:
            with OwnedProcess(Win32(), exe, args, report["launch"]) as process:
                browser = Browser(process, site)
                browser.page(SOURCE, require_request=True)
                source_panel = browser.wait("source page panel", browser.page_panel)
                report["initial"] = {"tabs": browser.tab_count(), "loaded_panels": loaded_panels(browser)}
                require(report["initial"] == {"tabs": 1, "loaded_panels": 1} and browser.selected() == 0,
                        f"Expected one loaded source tab, got {report['initial']}")
                require(page_counts(site, TARGET) == (0, 0), "Target was requested before clicking")

                send(browser, WM_COMMAND, DEFER_BACKGROUND)
                report["click_attempts"] = []
                open_background_link(browser, source_panel, report["click_attempts"])

                # Give asynchronous CEF navigation a chance to expose an eager load.
                deadline = time.monotonic() + 1

                def deferred():
                    state = report["before_selection"] = {
                        "tabs": browser.tab_count(), "selected": browser.selected(),
                        "loaded_panels": loaded_panels(browser), "target_counts": page_counts(site, TARGET),
                    }
                    require(state["tabs"] == 2 and state["selected"] == 0 and browser.page_panel() == source_panel,
                            f"Opening the background link did not leave the source tab selected: {state}")
                    require(state["loaded_panels"] == 1 and state["target_counts"] == (0, 0),
                            f"Background link loaded before selection: {state}")
                    return time.monotonic() >= deadline

                browser.wait("background link to stay deferred for one second", deferred, 3)
                select(browser, 1)
                browser.page(TARGET, require_request=True)
                browser.wait("visible target page with its rendered title", lambda: (
                    browser.has_page_title(TARGET_TITLE)
                    and (panel := browser.page_panel()) and panel != source_panel
                ))
                require(browser.tab_count() == 2, "Selecting the background link changed the tab count")
                report["after_selection"] = page_counts(site, TARGET)
                check_foreground_return(browser, report)
                process.normal_close(browser.hwnd)
        finally:
            report["http_requests"] = site.report()["requests"]

    session = json.loads((profile / "session.json").read_text(encoding="utf-8"))
    report["defer_background"] = session.get("defer_background")
    require(report["defer_background"] is True, "Menu command 168 did not persist defer_background=true")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe next to Flit.dll")
    parser.add_argument("--work", required=True, type=Path, help="parent directory for a fresh test profile")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")
    exe = args.exe.resolve()
    if exe.name.lower() != "flit.exe" or not exe.is_file() or not exe.with_name("Flit.dll").is_file():
        parser.error("--exe must point to Flit.exe next to Flit.dll")
    work = args.work.resolve()
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    work.mkdir(parents=True, exist_ok=True)
    scratch = tempfile.TemporaryDirectory(prefix="flit-link-tabs-", dir=work)
    with exe.with_name("Flit.dll").open("rb") as dll:
        report = {"result": "fail", "dll_sha256": file_digest(dll, "sha256").hexdigest()}
    try:
        run(exe, Path(scratch.name), report)
        report["result"] = "pass"
    except (RuntimeError, OSError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        clean_scratch(scratch, report)
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
