"""Run old and new Flit in the same disposable app directory and CEF profile."""

import argparse
from hashlib import file_digest
import json
import os
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

sys.dont_write_bytecode = True
from windows_bookmark_transfer import native_bookmarks, one_bookmark
from windows_bookmarks import SAVE_BOOKMARK, wait_manager
from windows_session import select
from windows_smoke import OwnedProcess, WORKSPACE, Win32, require
from windows_windows import Window, fixture, seed_profile, visit


BINARIES = ("Flit.exe", "Flit.dll", "libcef.dll")


def stage(bundle, target):
    require(not any((bundle / name).exists() for name in ("User Data", ".flit-browser-profile")),
            "The input must be an application bundle, not a browser profile")
    target.mkdir()
    for path in bundle.rglob("*"):
        if path.name == "debug.log":
            continue
        dest = target / path.relative_to(bundle)
        if path.is_symlink() or path.is_junction():
            raise ValueError(f"Unexpected link in staged bundle: {path}")
        if path.is_dir():
            dest.mkdir(exist_ok=True)
        else:
            dest.parent.mkdir(parents=True, exist_ok=True)
            os.link(path, dest)
    digests = {}
    for name in BINARIES:
        with (target / name).open("rb") as binary:
            digests[name] = file_digest(binary, "sha256").hexdigest()
    return digests


def exercise(baseline, candidate, scratch, evidence):
    app, replacement, profile = (scratch / name for name in ("app", "replacement", "profile"))
    evidence["baseline_sha256"] = stage(baseline.parent, app)
    evidence["candidate_sha256"] = stage(candidate.parent, replacement)
    require(evidence["baseline_sha256"]["Flit.dll"] != evidence["candidate_sha256"]["Flit.dll"],
            "The baseline and candidate Flit builds are identical")

    exe = app / "Flit.exe"
    args = [f"--flit-profile={profile}"]
    with fixture() as site:
        seed_profile(profile, site.origin)
        win = Win32()
        address = site.origin + site.paths["normal-peer"]
        with OwnedProcess(win, exe, args, evidence.setdefault("before", {})) as process:
            window = Window(process, site)
            visit(window, "normal-seed", "normal", {})
            visit(window, "normal-peer", "normal", {})
            saved_bookmark = bool(win.get_item(window.hwnd, SAVE_BOOKMARK))
            if saved_bookmark:
                window.command(SAVE_BOOKMARK)
                wait_manager(window, "Bookmarks — Page saved")
                select(window, 0)
                window.wait("old selected page before the upgrade", lambda: window.address() == address)
            process.normal_close(window.hwnd)

        def bookmark():
            return one_bookmark(native_bookmarks(profile), address, site.title("normal-peer"))

        original_bookmark = bookmark() if saved_bookmark else None
        with site.lock:
            site.requests.pop(site.paths["normal-peer"], None)
            site.requests.pop(site.paths["normal-peer"] + "/ready", None)

        # Only rename the two app directories created above, after the old job exits.
        archived = scratch / "original"
        require(all(path.resolve().parent == scratch for path in (app, replacement, archived)),
                "Replacement must stay inside the temporary test directory")
        app.rename(archived)
        replacement.rename(app)

        had_session = (profile / "session.json").is_file()
        evidence["tab_restore_supported_by_baseline"] = had_session
        with OwnedProcess(win, exe, args, evidence.setdefault("after", {})) as process:
            window = Window(process, site)
            if had_session:
                visit(window, "normal-peer", "normal", {}, start=True)
                evidence["selected_tab_restored"] = True
            visit(window, "normal-restart", "normal", {})
            evidence["persisted_login_cookie"] = True
            evidence["persisted_site_storage"] = True
            process.normal_close(window.hwnd)
        if original_bookmark:
            require(bookmark() == original_bookmark, "The bookmark changed after replacing the app")
            evidence["bookmark_preserved"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--work", required=True, type=Path)
    args = parser.parse_args()
    baseline, candidate, work = (path.resolve() for path in (args.baseline, args.candidate, args.work))
    if sys.platform != "win32" or not work.is_relative_to(WORKSPACE / "work"):
        parser.error("requires Windows and a test directory under workspace work/")
    for exe in (baseline, candidate):
        if (not exe.is_relative_to(WORKSPACE / "work") or exe.name.casefold() != "flit.exe"
                or any(not (exe.parent / name).is_file() for name in BINARIES)):
            parser.error("only complete staged Flit bundles under workspace work/ may be used")

    evidence = {"result": "failed", "installer_executed": False, "uses_real_user_profile": False}
    try:
        work.mkdir(parents=True, exist_ok=True)
        with TemporaryDirectory(prefix="flit-profile-upgrade-", dir=work, ignore_cleanup_errors=True) as scratch:
            exercise(baseline, candidate, Path(scratch), evidence)
        evidence["result"] = "pass"
    except Exception as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(evidence, indent=2))
    return int(evidence["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
