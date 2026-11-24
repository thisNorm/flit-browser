"""Check Flit's Windows display language in an isolated, staged browser.

Run: python tests/windows_settings_language.py --exe PATH/Flit.exe --work SCRATCH_PARENT
Requires an interactive Windows desktop. Prints sanitized JSON and exits 1 on failure.
"""

import argparse
from hashlib import file_digest
import json
from pathlib import Path
import re
import sys
from tempfile import TemporaryDirectory

sys.dont_write_bytecode = True
from windows_devtools import tools_windows
from windows_extensions import uia
from windows_multi_session import clean_scratch
from windows_smoke import GO, Browser, OwnedProcess, Win32, require
from windows_windows import fixture, seed_profile


LANGUAGES_URL = "chrome://settings/languages"
WEB_LANGUAGES = "fr,de"
CAPTIONS = {
    105: {"en-US": "New tab", "ko": "새 탭"},
    117: {"en-US": "Flit menu", "ko": "Flit 메뉴"},
    190: {"en-US": "Downloads", "ko": "다운로드"},
}
SETTINGS_TEXT = {
    "en-US": ("Settings", "Languages", "Preferred languages"),
    "ko": ("설정", "언어", "기본 언어"),
}


def seed_language(profile, origin, locale):
    seed_profile(profile, origin)  # Explicit profile marker and a loopback proxy that never forwards.
    for name, intl in (
        ("Local State", {"app_locale": locale}),
        ("Default/Preferences", {"selected_languages": WEB_LANGUAGES, "accept_languages": WEB_LANGUAGES}),
    ):
        path = profile / name
        data = json.loads(path.read_text(encoding="utf-8"))
        data.setdefault("intl", {}).update(intl)
        path.write_text(json.dumps(data), encoding="utf-8")


def native_controls(browser, locale):
    win, pid = browser.win, browser.process.pid
    require(win.pid(browser.hwnd) == pid, "Flit window is no longer owned")
    evidence = {}
    for cid, names in CAPTIONS.items():
        hwnd = win.get_item(browser.hwnd, cid)
        expected = names[locale]
        item = evidence[str(cid)] = {"expected": expected, "status": "missing_owned_visible_button"}
        if hwnd and win.pid(hwnd) == pid and win.visible(hwnd) and win.class_name(hwnd).casefold() == "button":
            browser.controls[cid] = hwnd
            actual = browser.control_text(cid)
            item["observed"] = actual if actual in names.values() else None
            item["status"] = ("pass" if actual == expected else
                              "wrong_locale" if item["observed"] is not None else "unexpected_caption")
    return evidence


def settings_language(browser, locale, evidence):
    require(not tools_windows(browser), "An owned Chromium window was already open")
    tabs = browser.tab_count(), browser.selected()
    browser.edit_address(LANGUAGES_URL)
    browser.click(GO)

    def observe():
        # Settings pages now open in the Flit tab itself, with the Flit skin.
        require(not tools_windows(browser), "Settings opened a separate Chromium window")
        if not browser.address().startswith(LANGUAGES_URL):
            return None
        hwnd, caption = browser.hwnd, browser.title().partition(" — ")[2]
        evidence["in_tab"] = True
        parts = re.split(r"\s+[-–—]\s+", caption.strip(), maxsplit=2)
        for language, (settings, section, _) in SETTINGS_TEXT.items():
            if parts[0] == settings:
                evidence["settings_title"] = settings
            if parts[0] == section or parts[:2] == [settings, section]:
                return language, "title", " - ".join(parts[:2] if parts[0] == settings else parts[:1])
        # A generic Settings title alone cannot distinguish the languages route from Settings home.
        visible = {row["name"] for row in uia(browser, hwnd) if row.get("rect")}
        for language, (_, _, heading) in SETTINGS_TEXT.items():
            if heading in visible:
                return language, "visible_uia", heading
        return None

    try:
        observed, source, marker = browser.wait("language settings title or visible heading", observe, 15)
    except RuntimeError:
        browser.process.alive()
        evidence["status"] = "language_page_unconfirmed"
        return False
    evidence.update(observed_locale=observed, source=source, marker=marker,
                    main_tab_unchanged=tabs[0] == browser.tab_count())
    matched = observed == locale and evidence["main_tab_unchanged"]
    evidence["status"] = "pass" if matched else "fail"
    return matched


def launch(exe, profile, site, locale):
    evidence = {"locale": locale, "result": "fail"}
    job = {}
    stage = "launch"
    try:
        with OwnedProcess(Win32(), exe, [f"--flit-profile={profile}"], job) as process:
            browser = Browser(process, site)
            stage = "native_controls"
            native = evidence["native_controls"] = native_controls(browser, locale)
            stage = "settings_webui"
            webui = evidence["settings_webui"] = {"requested_url": LANGUAGES_URL}
            webui_ok = settings_language(browser, locale, webui)
            stage = "close"
            process.normal_close(browser.hwnd)
        stage = "preferred_web_languages"
        intl = json.loads((profile / "Default" / "Preferences").read_text(encoding="utf-8")).get("intl", {})
        selected_ok = intl.get("selected_languages") == WEB_LANGUAGES
        evidence["preferred_web_languages"] = {
            "seeded": WEB_LANGUAGES,
            "selected_unchanged": selected_ok,
            "accept_unchanged": intl.get("accept_languages") == WEB_LANGUAGES,
        }
        if all(item["status"] == "pass" for item in native.values()) and webui_ok and selected_ok:
            evidence["result"] = "pass"
    except Exception as error:
        evidence.update(stage=stage, error=type(error).__name__)
    finally:
        # Keep process lifecycle evidence without logging IDs, paths, or UI trees.
        evidence["job"] = {key: job[key] for key in ("normal_close", "exit_code", "cleanup_needed") if key in job}
        if "leftover_pids" in job:
            evidence["job"]["remaining_process_count"] = len(job["leftover_pids"])
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="staged Flit.exe beside Flit.dll")
    parser.add_argument("--work", type=Path, required=True, help="scratch parent outside any live browser profile")
    parser.add_argument("--english-control", action="store_true", help="also run en-US in a separate temporary profile")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("requires Windows")

    report = {"result": "fail", "runs": {}}
    stage = "validate_bundle"
    try:
        exe = args.exe.resolve()
        dll = exe.with_name("Flit.dll")
        require(exe.name.casefold() == "flit.exe" and exe.is_file() and dll.is_file(),
                "Expected staged Flit.exe beside Flit.dll")
        with dll.open("rb") as stream:
            report["dll_sha256"] = file_digest(stream, "sha256").hexdigest()
        stage = "create_scratch"
        work = args.work.resolve()
        work.mkdir(parents=True, exist_ok=True)
        scratch = TemporaryDirectory(prefix="flit-settings-language-", dir=work)
        try:
            stage = "run_fixture"
            root, runs = Path(scratch.name), report["runs"]
            with fixture() as site:
                korean = root / "ko"
                seed_language(korean, site.origin, "ko")
                runs["ko"] = launch(exe, korean, site, "ko")
                if runs["ko"]["result"] == "pass":
                    runs["ko_relaunch"] = launch(exe, korean, site, "ko")  # No reseed.
                if args.english_control:
                    english = root / "en-US"
                    seed_language(english, site.origin, "en-US")
                    runs["en-US"] = launch(exe, english, site, "en-US")
        finally:
            cleanup = {}
            clean_scratch(scratch, cleanup)
            report["scratch_removed"] = "scratch_cleanup_error" not in cleanup
        if report["scratch_removed"] and all(item["result"] == "pass" for item in runs.values()):
            report["result"] = "pass"
    except (Exception, KeyboardInterrupt) as error:
        report.update(stage=stage, error=type(error).__name__)
    print(json.dumps(report, indent=2))
    return int(report["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
