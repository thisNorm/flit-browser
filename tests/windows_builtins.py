"""Local Flit baseline checks; uses a fresh profile and only its owned windows.

Run with --exe PATH/Flit.exe --work PATH [--evidence NEW.json].
Exit 1 means an assertion failed; exit 0 may include explicitly limited probes.
No existing browser/profile is opened, and no download or password dialog is used.
"""

import argparse
from contextlib import closing
import ctypes as C
import json
from pathlib import Path
import re
import sqlite3
import sys
import tempfile
import time
from urllib.parse import urlsplit

from windows_smoke import Browser, OwnedProcess, WORKSPACE, Win32, fixture, require


FIND, FIND_TEXT, FIND_STATUS, FIND_CLOSE = 124, 125, 126, 129
TITLE, PHRASE = "Flit smoke: start", "Flit smoke: start"
PAGES = (
    ("history", 119, "history", ("History", "방문 기록")),
    ("bookmarks", 120, "bookmarks", ("Bookmarks", "북마크")),
    ("downloads", 121, "downloads", ("Downloads", "Download history", "다운로드", "다운로드 기록")),
    ("settings", 122, "settings", ("Settings", "설정")),
    ("extensions", 123, "extensions", ("Extensions", "확장 프로그램")),
    ("passwords", 137, "password-manager", (
        "Google Password Manager", "Password Manager", "Passwords", "비밀번호 관리자")),
)


def command(browser, cid):
    browser.process.alive()
    require(browser.win.pid(browser.hwnd) == browser.process.pid, "Main window is no longer owned")
    browser.win.message(browser.hwnd, 0x0111, cid, 0)  # WM_COMMAND, no foreground input.


def snapshot(browser):
    browser.process.alive()
    return {"url": browser.address(), "title": browser.title()}


def activation(observed, host, titles):
    url = urlsplit(observed["url"])
    title = observed["title"].partition(" — ")[2].strip().casefold()
    return (url.scheme == "chrome" and url.netloc == host
            and title in {value.casefold() for value in titles})


def check(results, name, action, browser=None):
    item = results[name] = {"status": "pass"}
    started = time.monotonic()
    try:
        action(item)
    except Exception as error:
        item.update(status="fail", error=f"{type(error).__name__}: {error}")
    if browser:
        try:
            item["observed"] = snapshot(browser)
        except Exception as error:
            item["observation_error"] = str(error)
    item["seconds"] = round(time.monotonic() - started, 3)
    print(f"{item['status'].upper()}: {name}", flush=True)
    return item


def builtin(browser, cid, host, titles, item):
    command(browser, cid)
    item["expected_url"] = f"chrome://{host}/"
    item["expected_titles"] = list(titles)

    def ready():
        item["observed"] = snapshot(browser)
        return activation(item["observed"], host, titles)

    browser.wait(f"{host} committed title and chrome URL", ready)
    item["scope"] = "Activation only: expected document title and chrome host; no WebUI DOM inspection."


def find(browser, item):
    command(browser, FIND)
    for cid, expected in ((FIND_TEXT, "edit"), (FIND_STATUS, "static")):
        hwnd = browser.win.get_item(browser.hwnd, cid)
        require(hwnd and browser.win.pid(hwnd) == browser.process.pid
                and browser.win.class_name(hwnd).casefold() == expected,
                f"Missing owned find control {cid}")
        browser.controls[cid] = hwnd
    browser.wait("visible find input", lambda: browser.win.visible(browser.controls[FIND_TEXT]))
    item["phrase"], item["expected_count"] = PHRASE, 1
    browser.set_text(FIND_TEXT, PHRASE)

    def count_is(expected):
        item["last_count"] = browser.control_text(FIND_STATUS)
        return re.fullmatch(expected, item["last_count"]) is not None

    browser.wait("fixture find count 0/1 or 1/1", lambda: count_is(r"[01]\s*/\s*1"))
    item["initial_count"] = item["last_count"]
    command(browser, 127)  # FIND_NEXT must select the known match.
    browser.wait("Find Next selects fixture match 1/1", lambda: count_is(r"1\s*/\s*1"))
    item["matching_count"] = item["last_count"]
    browser.set_text(FIND_TEXT, "flit-builtins-absent-7a804721")
    browser.wait("absent phrase count 0/0", lambda: count_is(r"0\s*/\s*0"))
    item["absent_count"] = item["last_count"]


def close_find(browser, item):
    command(browser, FIND_CLOSE)
    hwnd = browser.win.get_item(browser.hwnd, FIND_TEXT)
    require(hwnd and browser.win.pid(hwnd) == browser.process.pid, "Missing owned find input")
    browser.wait("find input hidden", lambda: not browser.win.visible(hwnd))
    item["hidden"] = True


def history(profile, url, item):
    databases = list(profile.rglob("History"))
    require(databases, "No native History database was persisted")
    item["databases"] = [str(path) for path in databases]
    item["visits"] = []
    for path in databases:
        # All owned processes have exited. Immutable, read-only access creates no sidecars.
        with closing(sqlite3.connect(path.resolve().as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
            rows = db.execute(
                "SELECT u.title, u.visit_count, COUNT(v.id) FROM urls u "
                "JOIN visits v ON v.url=u.id WHERE u.url=? GROUP BY u.id", (url,)).fetchall()
        item["visits"].extend({"title": title, "visit_count": count, "visit_rows": visits}
                              for title, count, visits in rows)
    require(any(row["title"] == TITLE and row["visit_count"] >= 1 and row["visit_rows"] >= 1
                for row in item["visits"]), f"No persisted native visit with fixture title for {url}")
    item["url"] = url


def run(exe, work, evidence):
    require(exe.is_file(), f"Missing executable: {exe}")
    require(work.is_relative_to(WORKSPACE), f"--work must be inside {WORKSPACE}")
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-builtins-", dir=work)) / "profile"
    profile.mkdir()
    evidence["profile"] = str(profile)
    results, launch = evidence["checks"], evidence["launch"]
    with fixture() as site:
        url = site.origin + "/start"
        evidence["fixture_url"] = url
        try:
            with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}", f"--flit-url={url}"], launch) as process:
                browser = Browser(process, site)

                def local_page(item):
                    browser.page("/start", require_request=True)
                    browser.wait("fixture document title", lambda: browser.has_page_title(TITLE))

                require(check(results, "localhost_fixture", local_page, browser)["status"] == "pass",
                        "Local fixture did not load; dependent checks cannot run")
                check(results, "find_count", lambda item: find(browser, item), browser)
                check(results, "find_close", lambda item: close_find(browser, item), browser)
                for name, cid, host, titles in PAGES:
                    check(results, name, lambda item, cid=cid, host=host, titles=titles:
                          builtin(browser, cid, host, titles, item), browser)
                check(results, "normal_close", lambda item: process.normal_close(browser.hwnd))
        finally:
            evidence["fixture"] = site.report()
    if launch.get("normal_close") and launch.get("leftover_pids") == []:
        check(results, "history_profile", lambda item: history(profile, url, item))
    else:
        results["history_profile"] = {"status": "skipped", "reason": "No clean shutdown; native profile files were not inspected."}


def self_check():
    from urllib.request import ProxyHandler, build_opener

    require(activation({"url": "chrome://history/", "title": "Flit Browser — History"}, "history", ("History",)), "Title matcher failed")
    require(not activation({"url": "chrome-error://chromewebdata/", "title": "Flit Browser — History"}, "history", ("History",)), "Error URL was accepted")
    require(not activation({"url": "chrome://history/", "title": "Flit Browser — history"}, "history", ("방문 기록",)), "Unexpected title accepted")
    with fixture() as site:
        with build_opener(ProxyHandler({})).open(site.origin + "/start", timeout=3) as response:
            html = response.read().decode("utf-8")
        require(html.count(f"<h1>{PHRASE}</h1>") == 1, "Fixture phrase changed")
    with tempfile.TemporaryDirectory(prefix="flit-builtins-check-") as directory:
        profile = Path(directory)
        url = "http://127.0.0.1:12345/start"
        with closing(sqlite3.connect(profile / "History")) as db:
            db.executescript("CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT, title TEXT, visit_count INTEGER);"
                             "CREATE TABLE visits(id INTEGER PRIMARY KEY, url INTEGER);")
            db.execute("INSERT INTO urls VALUES(1, ?, ?, 1)", (url, TITLE))
            db.execute("INSERT INTO visits VALUES(1, 1)")
            db.commit()
        history(profile, url, {})
    print("PASS: title/URL matcher, fixture phrase, read-only history query (no browser launched)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path)
    parser.add_argument("--work", type=Path)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--self-check", action="store_true")
    args = parser.parse_args()
    if args.self_check:
        if args.exe or args.work or args.evidence:
            parser.error("--self-check runs on its own")
        self_check()
        return 0
    if sys.platform != "win32":
        parser.error("Browser runtime verification requires Windows")
    if not args.exe or not args.work:
        parser.error("--exe and --work are required")
    if args.evidence and (args.evidence.exists() or args.evidence.is_symlink()):
        parser.error("Choose a new evidence file")
    evidence = {"result": "fail", "checks": {}, "launch": {}, "limitations": [
        "Built-in pages: title/URL activation only; English and Korean titles accepted.",
        "No download transfer, bookmark editing, password saving/autofill or extension installation tested.",
        "A new disposable profile is retained for inspection; existing browser sessions are untouched.",
    ]}
    try:
        run(args.exe.resolve(), args.work.resolve(), evidence)
        statuses = [item["status"] for item in evidence["checks"].values()]
        evidence["result"] = "fail" if "fail" in statuses else "partial" if any(
            status in ("probe", "skipped") for status in statuses) else "pass"
    except (Exception, KeyboardInterrupt) as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        with args.evidence.open("x", encoding="utf-8") as output:
            json.dump(evidence, output, indent=2, ensure_ascii=False)
            output.write("\n")
    print(json.dumps(evidence, indent=2, ensure_ascii=True))
    return int(evidence["result"] == "fail")


if __name__ == "__main__":
    raise SystemExit(main())
