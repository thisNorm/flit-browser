"""Verify the staged Flit bookmark button, duplicate handling, and restart persistence."""

import argparse
import json
from pathlib import Path
import sys
import tempfile

from windows_builtins import command, snapshot
from windows_smoke import Browser, OwnedProcess, WORKSPACE, Win32, fixture, require


BOOKMARKS, SELECT_FIRST_TAB, SAVE_BOOKMARK = 120, 151, 160
MANAGER = "chrome://bookmarks/"
TITLE = "Flit smoke: start"


def wait_manager(browser, *titles):
    def ready():
        observed = snapshot(browser)
        if observed["url"] == MANAGER and any(
            observed["title"].endswith(" — " + title) for title in titles
        ):
            return observed

    return browser.wait("bookmark manager: " + " or ".join(titles), ready)


def persisted(profile, url):
    def find(node):
        if node.get("type") == "url" and node.get("url") == url:
            yield node
        for child in node.get("children", ()):
            yield from find(child)

    path = profile / "Default/Bookmarks"
    require(path.is_file(), "Chromium did not create Default/Bookmarks")
    data = json.loads(path.read_text(encoding="utf-8"))
    matches = [node for root in data["roots"].values() for node in find(root)]
    require(len(matches) == 1, f"Expected one native bookmark for {url}; got {len(matches)}")
    require(matches[0]["name"] == TITLE, "Persisted bookmark title differs from the page title")
    return matches[0]


def verify(exe, profile, evidence):
    with fixture() as site:
        url = evidence["url"] = site.origin + "/start"
        args = [f"--flit-profile={profile}", f"--flit-url={url}"]
        for attempt in ("launch", "relaunch"):
            with OwnedProcess(Win32(), exe, args, evidence.setdefault(attempt, {})) as process:
                browser = Browser(process, site)
                browser.page("/start")
                browser.wait("fixture title", lambda: browser.has_page_title(TITLE))
                if attempt == "launch":
                    star = browser.win.get_item(browser.hwnd, SAVE_BOOKMARK)
                    require(star and browser.win.class_name(star).casefold() == "button",
                            "Current-page bookmark button is missing")
                    browser.controls[SAVE_BOOKMARK] = star
                    browser.click(SAVE_BOOKMARK)
                    evidence["created"] = wait_manager(browser, "Bookmarks — Page saved")
                else:
                    command(browser, BOOKMARKS)
                    evidence["reopened"] = wait_manager(browser, "Bookmarks", "북마크")

                command(browser, SELECT_FIRST_TAB)
                browser.wait("original fixture tab", lambda: browser.address() == url)
                command(browser, SAVE_BOOKMARK)
                evidence[f"{attempt}_duplicate"] = wait_manager(browser, "Bookmarks — Already saved")
                process.normal_close(browser.hwnd)

            saved = persisted(profile, url)
            if attempt == "launch":
                evidence["persisted"] = saved
            else:
                require(saved["guid"] == evidence["persisted"]["guid"],
                        "Bookmark identity changed after restart")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    exe, work = args.exe.resolve(), args.work.resolve()
    if sys.platform != "win32":
        parser.error("requires Windows")
    if not exe.is_relative_to(WORKSPACE) or exe.name.casefold() != "flit.exe" or not exe.is_file():
        parser.error("expected a staged Flit.exe inside the workspace")
    if not work.is_relative_to(WORKSPACE):
        parser.error("expected a working directory inside the workspace")
    work.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="flit-bookmarks-", dir=work))
    evidence = {"profile": str(profile), "result": "fail"}
    try:
        verify(exe, profile, evidence)
        evidence["result"] = "pass"
    except Exception as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
    out = profile.with_suffix(".json")
    out.write_text(json.dumps(evidence, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
    print(json.dumps(evidence, indent=2, ensure_ascii=True))
    print(f"Evidence: {out}")
    return int(evidence["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
