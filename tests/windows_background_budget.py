"""Check default background deferral and the shared speculative budget in owned Flit windows."""

import argparse
from hashlib import file_digest
import json
from pathlib import Path
import re
import sys
import tempfile
import time

sys.dont_write_bytecode = True
from windows_background_links import DEFER_BACKGROUND, LinkHandler, post_link_click
from windows_downloads import seed_profile
from windows_multi_session import clean_scratch
from windows_session import loaded_panels, select
from windows_smoke import OwnedProcess, WORKSPACE, Win32, fixture, page_counts, require
from windows_windows import NEW_WINDOW, Window


QUIET_SECONDS = 1.0
ROUTE = re.compile(r"(/shown)?/(source|target|middle|ctrl)/([a-z-]+)")
SOURCE_HTML = '''<style>a {{ position:fixed; inset:0; display:grid; place-items:center }}</style>
<a href="/target/{label}">Open background link</a>
<script>
const link = document.querySelector("a");
link.onauxclick = event => {{
    if (event.isTrusted && event.button === 1) fetch("/middle/{label}", {{keepalive: true}});
}};
link.onclick = event => {{
    if (event.isTrusted && event.button === 0 && event.ctrlKey) fetch("/ctrl/{label}", {{keepalive: true}});
}};
</script>'''


class BudgetHandler(LinkHandler):
    def do_GET(self):
        site = self.server
        if not self.path.startswith(site.origin + "/"):
            self.send_error(403)  # The disposable proxy never forwards.
            return
        path = self.path[len(site.origin):]
        match = ROUTE.fullmatch(path)
        if not match:
            self.send_error(404)
            return
        shown, kind, label = match.groups()
        body = b""
        if not shown and kind in ("source", "target"):
            content = SOURCE_HTML.format(label=label) if kind == "source" else f"<h1>{label}</h1>"
            # Hidden CEF views can run the document without receiving animation frames.
            body = (f'<!doctype html><meta charset="utf-8"><link rel="icon" href="data:,">'
                    f'<title>{kind}: {label}</title>{content}'
                    '<script>fetch("/shown" + location.pathname);</script>').encode("ascii")
        with site.lock:
            site.hits[path] += 1
        self.send_response(200 if body else 204)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)


def open_link(window, label, checks, *, warm):
    site = window.site
    source, target = f"/source/{label}", f"/target/{label}"
    source_url = site.origin + source
    window.navigate(source)
    panel = window.wait(f"{label}: visible source panel", window.page_panel)
    row, selected, panels = window.tab_count(), window.selected(), loaded_panels(window)
    require(0 <= selected < row and panels > 0, f"{label}: missing selected source tab")
    require(page_counts(site, target) == (0, 0), f"{label}: target requested before clicking")
    check = checks[label] = {"hwnd": window.hwnd, "expected_warm": warm, "click_attempts": []}

    def trusted_clicks():
        return site.count(f"/middle/{label}") + site.count(f"/ctrl/{label}")

    def progressed():
        return (trusted_clicks() or window.tab_count() != row or window.address() != source_url
                or site.count(target))

    post_link_click(window, panel, check["click_attempts"])
    retry_at = time.monotonic() + 2
    window.wait(f"{label}: middle click or retry deadline", lambda: progressed() or time.monotonic() >= retry_at, 3)
    if not progressed():
        post_link_click(window, panel, check["click_attempts"], ctrl=True)
    window.wait(f"{label}: new tab after trusted link click",
                lambda: window.tab_count() > row and trusted_clicks())

    expected_panels = panels + int(warm)
    deadline = time.monotonic() + QUIET_SECONDS

    def observe():
        state = check["before_selection"] = {
            "tabs": window.tab_count(), "selected": window.selected(), "loaded_panels": loaded_panels(window),
            "target_counts": page_counts(site, target), "trusted_clicks": trusted_clicks(),
        }
        require(state["tabs"] == row + 1 and state["selected"] == selected and state["trusted_clicks"] == 1
                and window.address() == source_url and window.page_panel() == panel,
                f"{label}: expected exactly one new background tab and the same selected source: {state}")
        if warm:
            return state["loaded_panels"] == expected_panels and all(state["target_counts"])
        require(state["loaded_panels"] == expected_panels and state["target_counts"] == (0, 0),
                f"{label}: background link loaded before selection: {state}")
        return time.monotonic() >= deadline

    expectation = "warm an attached view" if warm else "remain unloaded"
    window.wait(f"{label}: background link to {expectation}", observe)
    return row


def select_target(window, label, row, checks):
    target = f"/target/{label}"
    previous = window.page_panel()
    select(window, row)
    window.page(target, require_request=True)
    window.wait(f"{label}: target has a visible panel and title", lambda: (
        window.has_page_title(f"target: {label}") and (panel := window.page_panel()) and panel != previous
    ))
    require(window.tab_count() == row + 1, f"{label}: selecting the target changed the tab count")
    checks[label]["after_selection"] = page_counts(window.site, target)


def exercise_budget(first, phase, report):
    checks = report["links"]
    first_label = f"{phase}-first"
    row = open_link(first, first_label, checks, warm=True)
    panels = loaded_panels(first)
    peer = first.open_window(NEW_WINDOW, f"{phase}-peer", report["windows"])
    peer.blank()
    open_link(peer, f"{phase}-second", checks, warm=True)
    open_link(peer, f"{phase}-over-budget", checks, warm=False)
    require(loaded_panels(first) == panels, f"{phase}: first window lost a loaded view at the budget")

    # Only this selection releases a speculative slot; the next open is in the other window.
    select_target(first, first_label, row, checks)
    open_link(peer, f"{phase}-reused-slot", checks, warm=True)
    peer.close_window()


def run(exe, profile, report):
    report.update(profile=str(profile), launches={}, links={}, windows={}, saved_preferences={})
    with fixture() as site:
        site.RequestHandlerClass = BudgetHandler
        seed_profile(profile, profile.parent / "downloads", site.origin)
        session = profile / "session.json"
        require(not session.exists(), "Default check must start without a saved session")
        # Skip restoring old tabs on the second launch while still reading the profile's preferences.
        args = [f"--flit-profile={profile}", "--flit-url=about:blank"]
        win = Win32()
        try:
            for phase in ("menu", "restart"):
                with OwnedProcess(win, exe, args, report["launches"].setdefault(phase, {})) as process:
                    first = Window(process, site)
                    first.blank()
                    if phase == "menu":
                        row = open_link(first, "default", report["links"], warm=False)
                        select_target(first, "default", row, report["links"])
                        first.command(DEFER_BACKGROUND)
                    exercise_budget(first, phase, report)
                    process.normal_close(first.hwnd)
                state = json.loads(session.read_text(encoding="utf-8"))
                report["saved_preferences"][phase] = state.get("defer_background")
                require(state.get("defer_background") is False,
                        f"{phase}: clean shutdown did not save explicit defer_background=false")
        finally:
            report["http_requests"] = site.report()["requests"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path, help="staged Flit.exe next to Flit.dll")
    parser.add_argument("--work", required=True, type=Path, help="workspace directory for a disposable profile")
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    dll = exe.with_name("Flit.dll")
    if sys.platform != "win32" or exe.name.casefold() != "flit.exe" or not exe.is_file() or not dll.is_file():
        parser.error("requires Windows and a staged Flit.exe next to Flit.dll")
    if not work.is_relative_to(WORKSPACE):
        parser.error(f"--work must be inside {WORKSPACE}")

    report = {"result": "fail"}
    try:
        with dll.open("rb") as stream:
            report["dll_sha256"] = file_digest(stream, "sha256").hexdigest()
        work.mkdir(parents=True, exist_ok=True)
        scratch = tempfile.TemporaryDirectory(prefix="flit-background-budget-", dir=work)
        try:
            run(exe, Path(scratch.name) / "profile", report)
        finally:
            clean_scratch(scratch, report)
        require("scratch_cleanup_error" not in report, "Could not remove disposable profile")
        report["result"] = "pass"
    except (OSError, RuntimeError, ValueError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
