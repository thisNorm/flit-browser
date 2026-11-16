"""Check public search documents in an owned, disposable Windows Flit profile."""

import argparse
from base64 import b64encode
from hashlib import file_digest
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import time
from types import SimpleNamespace
from urllib.parse import parse_qs, urlsplit

sys.dont_write_bytecode = True
from windows_extensions import uia
from windows_multi_session import clean_scratch
from windows_session import select, send
from windows_smoke import ADDRESS, BACK, Browser, NEW_TAB, OwnedProcess, WORKSPACE, Win32, require


ENGINES = {
    "duckduckgo": (176, ("duckduckgo.com",), "q"),
    "google": (177, ("google.com",), "q"),
    "naver": (178, ("naver.com",), "query"),
    "bing": (179, ("bing.com",), "q"),
}
TERM = "러스트 프로그래밍 언어 공식 문서"
REFERENCE = "https://rust-lang.org/learn/"
BOOK_LABEL = "The Rust Programming Language"
RESULT_TERMS = ("rust", "러스트", "공식 문서", "programming language")
BLOCK_TERMS = ("captcha", "unusual traffic", "verify you are human", "not a robot",
               "접근이 제한", "자동 입력 방지", "사람인지", "ERR_", "are you a robot",
               "robot check", "access denied", "proxy error", "failed to load")


def same_domain(host, bases):
    return any(host == base or host.endswith("." + base) for base in bases)


def command(browser, id):
    send(browser, 0x0111, id, post=True)


def search_url(url, engine):
    _, domains, parameter = ENGINES[engine]
    parts = urlsplit(url)
    return (parts.scheme == "https" and same_domain(parts.hostname or "", domains)
            and parse_qs(parts.query).get(parameter) == [TERM])


def search_results(row, engine):
    return search_url(row.get("url", ""), engine) and bool(row.get("result_links"))


def observe(browser, seconds, until_result):
    stop = time.monotonic() + seconds
    report = {}
    while time.monotonic() < stop:
        browser.process.alive()
        url, title = browser.address(), browser.title()
        report.update(url=url, title=title)
        if url and browser.page_panel():
            try:
                rows = [row for row in uia(browser, browser.hwnd) if isinstance(row, dict)]
            except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
                rows = []
                report["accessibility_error"] = str(error)[:180]
            names = [(row.get("name") or "") for row in rows]
            links = [row["name"] for row in rows if row.get("type") == "ControlType.Hyperlink"
                     and row.get("name") and any(term in row["name"].lower() for term in RESULT_TERMS)]
            report["result_links"] = links[:4]
            report["relevant_text"] = [n[:180] for n in names if any(t in n.lower() for t in RESULT_TERMS)][:8]
            visible = "\n".join(names+[title]).lower()
            report["block_markers"] = [mark for mark in BLOCK_TERMS if mark.lower() in visible]
            if until_result(report) or report["block_markers"]:
                return report
        time.sleep(.35)
    return report


def follow_official_result(browser):
    panel = browser.page_panel()
    require(panel and browser.win.pid(panel) == browser.process.pid
            and search_url(browser.address(), "duckduckgo"), "Expected owned DuckDuckGo page")
    count = browser.tab_count()
    invoke = rf"""
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$pidExpected = {browser.process.pid}
$root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr][long]{panel})
if (!$root -or $root.Current.ProcessId -ne $pidExpected) {{ throw 'Not the owned page' }}
$type = [Windows.Automation.ControlType]::Hyperlink
$condition = [Windows.Automation.PropertyCondition]::new(
    [Windows.Automation.AutomationElement]::ControlTypeProperty, $type)
$links = $root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)
$book = $links | Where-Object {{
    $_.Current.ProcessId -eq $pidExpected -and !$_.Current.IsOffscreen -and
    $_.Current.IsEnabled -and $_.Current.Name -ceq '{BOOK_LABEL}'
}} | Select-Object -First 1
if (!$book -or $root.Current.ProcessId -ne $pidExpected -or $book.Current.ProcessId -ne $pidExpected) {{
    throw 'Expected an owned, visible Rust Book search result'
}}
$book.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
"""
    result = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand",
         b64encode(invoke.encode("utf-16le")).decode("ascii")],
        stdin=subprocess.DEVNULL, capture_output=True, text=True,
        timeout=10, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    require(result.returncode == 0, f"Result invocation failed: {result.stderr.strip()}")

    def book_loaded():
        url = urlsplit(browser.address())
        return (url.scheme == "https" and url.hostname == "doc.rust-lang.org"
                and url.path.startswith("/book/") and BOOK_LABEL in browser.title())

    browser.wait("official Rust Book rendered", book_loaded, 15)
    destination = browser.address()
    title = browser.title()
    current_count = browser.tab_count()
    require(current_count in (count, count + 1), "Unexpected number of tabs from result link")
    if current_count == count + 1:
        browser.close_row(browser.selected())
        method = "close result tab"
    else:
        browser.click(BACK)
        method = "Back button"
    browser.wait("return to DuckDuckGo results", lambda: search_url(browser.address(), "duckduckgo")
                 and browser.tab_count() == count and "DuckDuckGo" in browser.title(), 10)
    restored = observe(browser, 10, lambda row: search_results(row, "duckduckgo"))
    require(search_results(restored, "duckduckgo"), "Search results did not render after returning")
    return dict(label=BOOK_LABEL, url=destination, title=title, returned_by=method,
                returned_to=restored["url"], returned_result_links=restored["result_links"])


def run(args, profile, report):
    initial = [f"--flit-profile={profile}", "--flit-url=about:blank"]
    with OwnedProcess(Win32(), args.exe, initial, report.setdefault("launch", {})) as process:
        browser = Browser(process, SimpleNamespace(origin=""))
        checks = report["search"] = {}
        for engine in args.engine:
            report["stage"] = f"{engine}: open search tab"
            command(browser, ENGINES[engine][0])
            count = browser.tab_count()
            command(browser, NEW_TAB)
            browser.wait("new empty tab", lambda: browser.tab_count() == count + 1)
            command(browser, 141)  # The action bound to Ctrl+K and Ctrl+E.
            browser.wait("search mode address", lambda: browser.address() == "? ")
            report["stage"] = f"{engine}: submit query"
            browser.edit_address("? " + TERM)
            browser.press(ADDRESS, 0x0D)
            report["stage"] = f"{engine}: verify rendered results"
            item = checks[engine] = observe(browser, 16, lambda row: search_results(row, engine))
            item["query_preserved"] = search_url(item.get("url", ""), engine)
            if search_results(item, engine):
                item["outcome"] = "visible_results"
            elif item.get("block_markers"):
                item["outcome"] = "provider_or_network_block"
            else:
                item["outcome"] = "not_confirmed"
            if args.follow_result and engine == "duckduckgo":
                report["stage"] = "DuckDuckGo: open an actual result and return"
                try:
                    report["result_navigation"] = follow_official_result(browser)
                except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
                    report["result_navigation_error"] = str(error)

        if args.new_tab:
            # A committed page with a stable public title is used as the source.
            # The exact same new-tab route is used for address and search input.
            report["stage"] = "new tab: load public source"
            browser.edit_address(REFERENCE)
            browser.press(ADDRESS, 0x0D)
            browser.wait("public Rust learn page", lambda: browser.page_panel() and
                         same_domain(urlsplit(browser.address()).hostname or "", ("rust-lang.org",))
                         and "Learn Rust" in browser.title(), 15)
            source = browser.address()
            source_panel = browser.page_panel()
            source_row = browser.selected()
            command(browser, NEW_TAB)
            browser.wait("unrelated blank tab after the source",
                         lambda: browser.tab_count() == source_row + 2
                         and browser.selected() == source_row + 1)
            select(browser, source_row)
            browser.wait("public source selected again", lambda: browser.address() == source)
            old_count = browser.tab_count()
            command(browser, 178)
            command(browser, 141)
            browser.wait("search mode for new tab", lambda: browser.address() == "? ")
            browser.edit_address("? " + TERM)
            key = 0x201C0001  # Alt context, Enter scan code, initial keypress.
            browser.message(ADDRESS, 0x0104, 0x0D, key)  # WM_SYSKEYDOWN
            browser.message(ADDRESS, 0x0106, 0x0D, key)  # WM_SYSCHAR
            browser.wait("new tab selected", lambda: browser.tab_count() == old_count + 1)
            item = report["new_tab"] = observe(browser, 16, lambda row: search_results(row, "naver"))
            require(search_results(item, "naver"), "Naver results missing in new tab")
            item["source_row"] = source_row
            item["unrelated_row"] = source_row + 1
            browser.close_row(browser.selected())
            browser.wait("new results tab closed", lambda: browser.tab_count() == old_count, 8)
            item["row_after_close"] = browser.selected()
            item["address_after_close"] = browser.address()
            item["same_page_panel"] = browser.page_panel() == source_panel
            require(item["row_after_close"] == source_row and item["address_after_close"] == source
                    and item["same_page_panel"] and "Learn Rust" in browser.title(),
                    f"Closing the results selected row {item['row_after_close']} instead of source row {source_row}")
            item["original_after_close"] = item["address_after_close"]
        process.normal_close(browser.hwnd)
    require(all(item["outcome"] == "visible_results" for item in report["search"].values()),
            "Visible results were not confirmed for every selected search engine")
    require(not args.follow_result or "result_navigation" in report,
            report.get("result_navigation_error", "Actual result navigation was not confirmed"))
    report["stage"] = "complete"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, default=WORKSPACE / "work" / "windows-public-search")
    parser.add_argument("--engine", nargs="+", choices=list(ENGINES), default=list(ENGINES))
    parser.add_argument("--follow-result", action="store_true", help="open the official Rust Book result from DuckDuckGo")
    parser.add_argument("--new-tab", action="store_true")
    args = parser.parse_args()
    args.exe = args.exe.resolve(strict=True)
    if args.follow_result and "duckduckgo" not in args.engine:
        parser.error("--follow-result requires DuckDuckGo")
    args.work.mkdir(parents=True, exist_ok=True)
    scratch = TemporaryDirectory(prefix="flit-live-search-", dir=args.work.resolve())
    with args.exe.with_suffix(".dll").open("rb") as dll:
        report = {"dll_sha256": file_digest(dll, "sha256").hexdigest(), "result": "fail", "query": TERM}
    try:
        run(args, Path(scratch.name) / "profile", report)
        report["result"] = "observed"
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        clean_scratch(scratch, report)
    print(json.dumps(report, indent=2, ensure_ascii=True))
    return int(report["result"] != "observed")


if __name__ == "__main__":
    raise SystemExit(main())
