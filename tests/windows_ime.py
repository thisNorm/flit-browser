"""Check that the native address field defers Enter and Escape during IME composition.

These are targeted Win32 messages, not a Microsoft Korean IME or physical keyboard test.
"""

import argparse
import json
from pathlib import Path
import tempfile

from windows_smoke import ADDRESS, Browser, OwnedProcess, WORKSPACE, Win32, fixture, require


WM_IME_STARTCOMPOSITION, WM_IME_ENDCOMPOSITION = 0x010D, 0x010E
WM_KEYDOWN, WM_CHAR = 0x0100, 0x0102
VK_RETURN, VK_ESCAPE = 0x0D, 0x1B


def run(exe, work, report):
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-ime-", dir=work)) / "profile"
    report["launch"] = launch = {}
    with fixture() as site, OwnedProcess(
        Win32(), exe, [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"], launch,
    ) as process:
        browser = Browser(process, site)
        browser.page("/start", require_request=True)
        before = site.count("/second")
        draft = site.origin + "/second"
        browser.edit_address(draft)
        browser.message(ADDRESS, WM_IME_STARTCOMPOSITION)
        try:
            browser.press(ADDRESS, VK_ESCAPE)
            require(browser.address() == draft, "Escape during composition replaced the address draft")
            browser.message(ADDRESS, WM_KEYDOWN, VK_RETURN, 0x001C0001)
        finally:
            browser.message(ADDRESS, WM_IME_ENDCOMPOSITION)
        # TranslateMessage can deliver this character after the IME has ended.
        browser.message(ADDRESS, WM_CHAR, VK_RETURN, 0x001C0001)
        browser.win.message(browser.hwnd, 0)
        require(browser.has_page_title("Flit smoke: start") and site.count("/second") == before,
                "The composition's Enter submitted the address")
        report["composition_draft_preserved"] = True
        browser.page("/second", lambda: browser.press(ADDRESS, VK_RETURN), require_request=True)
        require(site.count("/second") == before + 1, "Enter after composition did not navigate exactly once")
        report["entered_after_composition"] = True
        process.normal_close(browser.hwnd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--work", required=True, type=Path)
    args = parser.parse_args()
    report = {"result": "fail"}
    try:
        exe, work = args.exe.resolve(strict=True), args.work.resolve()
        require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
        run(exe, work, report)
        report["result"] = "pass"
    except (RuntimeError, OSError) as error:
        report["error"] = str(error)
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
