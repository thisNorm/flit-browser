"""Compare owned native Flit trees: one local page, six tabs, then one again.

python -B windows_memory.py --wait-candidate
The default adds five empty tabs; --extra-tabs fixture instead loads all six.
Only the newly created evidence JSON is written; profiles use temporary dirs.
"""

import argparse
from collections import Counter
import ctypes as C
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
from windows_smoke import OwnedProcess, WORKSPACE, Win32, fixture, require, until


def utc():
    return datetime.now(timezone.utc).isoformat()


def identity(exe):
    result = {}
    for path in (exe, exe.with_suffix(".dll"), exe.parent / "libcef.dll"):
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        result[path.name] = {"path": str(path), "sha256": digest,
                             "bytes": path.stat().st_size}
    return result


class MainWindow:
    """Find only our root window; accept both legacy TabControl and ListBox."""

    def __init__(self, process):
        self.process, self.win = process, process.win
        self.hwnd = self.wait("owned main window", self.find, 35)
        self.address = self.win.get_item(self.hwnd, 101)
        self.tabs = self.win.get_item(self.hwnd, 108)
        self.tabs_class = self.win.class_name(self.tabs)
        messages = {"listbox": 0x018B, "systabcontrol32": 0x1304,
                    "syslistview32": 0x1004}
        require(self.tabs_class.casefold() in messages,
                f"Unsupported tab control: {self.tabs_class}")
        self.count_message = messages[self.tabs_class.casefold()]
        self.win.check(self.win.set_window_pos(self.hwnd, None, 0, 0, 1280, 900, 0x0016))

    def wait(self, label, check, timeout=12):
        return until(label, check, timeout, self.process.alive)

    def find(self):
        found = []

        @self.win.callback
        def visit(hwnd, _):
            if self.win.pid(hwnd) == self.process.pid and self.win.visible(hwnd):
                address, tabs = self.win.get_item(hwnd, 101), self.win.get_item(hwnd, 108)
                if address and tabs and self.win.class_name(address).casefold() == "edit":
                    found.append(hwnd)
            return True

        self.win.check(self.win.enum(visit, 0))
        return found[0] if found else None

    def text(self, hwnd):
        value = C.create_unicode_buffer(2048)
        self.win.message(hwnd, 0x000D, len(value), C.addressof(value))
        return value.value

    def count(self):
        return self.win.message(self.tabs, self.count_message)

    def command(self, command):
        self.process.alive()
        require(self.win.pid(self.hwnd) == self.process.pid, "Window is no longer owned")
        self.win.message(self.hwnd, 0x0111, command, 0)

    def navigate(self, site):
        before = site.count("/shown/start")
        value = C.create_unicode_buffer(site.origin + "/start")
        self.win.message(self.address, 0x000C, 0, C.addressof(value))
        self.command(107)
        self.wait("fixture pageshow", lambda: site.count("/shown/start") > before)
        self.wait("fixture address", lambda: self.text(self.address) == site.origin + "/start")


def sample(process, browser, collector, seconds, settle, name, expected_tabs, loaded):
    browser.wait(f"{expected_tabs} tabs", lambda: browser.count() == expected_tabs)
    time.sleep(settle)
    process.alive()
    before = process.pids()
    sampled = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-File", str(collector),
         "-RootProcessId", str(process.pid), "-SampleSeconds", str(seconds)],
        stdin=subprocess.DEVNULL, capture_output=True, timeout=45,
        creationflags=subprocess.CREATE_NO_WINDOW, check=True,
    )
    metrics = json.loads(sampled.stdout.decode("utf-8-sig"))
    after = process.pids()
    require(metrics["status"] != "error", f"Collector failed: {metrics.get('errors')}")
    endpoint = metrics["end"]
    for metric in ("privateCommitBytes", "privateWorkingSetBytes"):
        require(endpoint["totals"][metric]["complete"], f"Incomplete {metric}")
    rows = endpoint["processes"]
    measured = sorted(row["processId"] for row in rows)
    require(measured == after, "Collector endpoint does not match the owned Job membership")
    require(all(row["identityVerified"] for row in rows), "Unverified process identity")
    roles = dict(Counter(row["role"] for row in rows))
    commit = endpoint["totals"]["privateCommitBytes"]["value"]
    resident = endpoint["totals"]["privateWorkingSetBytes"]["value"]
    print(f"  {name}: {len(rows)} processes, commit {commit / 1048576:.2f} MiB, "
          f"private WS {resident / 1048576:.2f} MiB", flush=True)
    return {"state": name, "tabs": expected_tabs, "loaded_fixture_tabs": loaded,
            "extra_empty_tabs": expected_tabs - loaded, "settle_seconds": settle,
            "address": browser.text(browser.address), "title": browser.text(browser.hwnd),
            "process_count": len(rows), "roles": roles,
            "private_commit_bytes": commit, "private_working_set_bytes": resident,
            "main_window_pid": browser.win.pid(browser.hwnd),
            "single_root_ui_with_only_classified_cef_children": roles.get("root") == 1
            and not any(role in roles for role in ("unknown", "other-type")),
            "job_pids_before": before, "job_pids_after": after,
            "collector_matches_owned_job": True, "collector": metrics}


def run_version(label, exe, site, collector, args, result):
    result.update(label=label, executable=str(exe), started_at_utc=utc(),
                  binaries_before=identity(exe), samples=[], lifecycle={})
    before_requests = site.report()["requests"]
    with tempfile.TemporaryDirectory(prefix=f"flit-memory-{label}-") as directory:
        profile = Path(directory) / "profile"
        profile.mkdir()
        result["profile"] = str(profile)
        launch = [f"--flit-profile={profile}", f"--flit-url={site.origin}/start"]
        result["arguments"] = launch
        shown = site.count("/shown/start")
        with OwnedProcess(Win32(), exe, launch, result["lifecycle"]) as process:
            browser = MainWindow(process)
            result.update(window_class=browser.win.class_name(browser.hwnd),
                          tab_control_class=browser.tabs_class,
                          window_rect=browser.win.rect(browser.hwnd),
                          dpi=browser.win.dpi(browser.hwnd))
            browser.wait("initial fixture pageshow", lambda: site.count("/shown/start") > shown)
            browser.wait("initial fixture address", lambda: browser.text(browser.address)
                         == site.origin + "/start")
            result["samples"].append(sample(process, browser, collector, args.sample_seconds,
                                            args.settle_seconds, "one_loaded_page", 1, 1))
            for count in range(2, 7):
                browser.command(105)
                browser.wait(f"new tab {count}", lambda: browser.count() == count)
                if args.extra_tabs == "fixture":
                    browser.navigate(site)
            loaded = 6 if args.extra_tabs == "fixture" else 1
            state = "six_loaded_pages" if loaded == 6 else "one_loaded_plus_five_empty_tabs"
            result["samples"].append(sample(process, browser, collector, args.sample_seconds,
                                            args.settle_seconds, state, 6, loaded))
            result["close_commands"] = []
            try:
                for count in range(5, 0, -1):
                    result["close_commands"].append({"command": 106, "expected_remaining": count})
                    browser.command(106)
                    browser.wait(f"closed extras: {count} remain", lambda: browser.count() == count)
                browser.wait("original fixture still open", lambda: browser.text(browser.address)
                             == site.origin + "/start")
                result["samples"].append(sample(process, browser, collector, args.sample_seconds,
                                                args.settle_seconds, "one_page_after_closing_extras", 1, 1))
                process.normal_close(browser.hwnd)
            except Exception as error:
                result["close_extras"] = {"status": "unavailable", "error": f"{type(error).__name__}: {error}"}
        result["profile_processes_exited_before_removal"] = not result["lifecycle"]["leftover_pids"]
    result["profile_removed"] = not profile.exists()
    after_requests = site.report()["requests"]
    result["fixture_requests"] = {path: count - before_requests.get(path, 0)
                                  for path, count in after_requests.items()
                                  if count != before_requests.get(path, 0)}
    result["binaries_after"] = identity(exe)
    result["binaries_unchanged_during_run"] = result["binaries_before"] == result["binaries_after"]
    require(result["binaries_unchanged_during_run"], "Binaries changed during measurement")
    result.update(result="partial" if "close_extras" in result else "pass", finished_at_utc=utc())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, default=WORKSPACE / "work/flit-setup/payload/Flit.exe")
    parser.add_argument("--candidate", type=Path, default=WORKSPACE / "work/flit-0.2.0/Flit-Browser/Flit.exe")
    parser.add_argument("--collector", type=Path, default=WORKSPACE /
                        "outputs/flit-browser-phase0/experiments/Measure-Browser.ps1")
    parser.add_argument("--evidence", type=Path, default=WORKSPACE / "work/flit-0.2.0/memory-comparison.json")
    parser.add_argument("--sample-seconds", type=float, default=1)
    parser.add_argument("--settle-seconds", type=float, default=2)
    parser.add_argument("--extra-tabs", choices=("blank", "fixture"), default="blank")
    parser.add_argument("--wait-candidate", action="store_true", help="Wait for 'ready' on stdin after baseline")
    parser.add_argument("--resume-candidate", action="store_true",
                        help="Keep this harness's failed baseline evidence and run only candidate")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Windows required")
    if args.evidence.exists() and not args.resume_candidate:
        parser.error(f"Refusing to overwrite existing evidence: {args.evidence}")
    if not (0 < args.sample_seconds <= 3 and 0 <= args.settle_seconds <= 10):
        parser.error("Use a sample in (0,3] seconds and settle in [0,10] seconds")
    evidence = {"schema_version": 1, "result": "running", "started_at_utc": utc(),
                "collector": str(args.collector.resolve()), "workload": args.extra_tabs,
                "sequence": "1 loaded local page -> 6 tabs -> close 5 via WM_COMMAND 106",
                "sample_seconds": args.sample_seconds, "settle_seconds": args.settle_seconds,
                "limitations": ["One sequential diagnostic per version; order is baseline then candidate.",
                                "Private commit is not resident RAM; private working set is measured separately.",
                                "The default six-tab state has one loaded fixture and five empty tabs, not six sites.",
                                "Two seconds is a settle delay, not proof of steady state."], "versions": []}

    def save():
        args.evidence.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")

    if args.resume_candidate:
        evidence = json.loads(args.evidence.read_text(encoding="utf-8"))
        require(evidence["result"] == "fail" and len(evidence["versions"]) == 1,
                "Resume requires one failed baseline from this harness")
        baseline = evidence["versions"][0]
        require(baseline["label"] == "baseline_v0.1" and len(baseline["samples"]) == 2
                and baseline["lifecycle"].get("leftover_pids") == [], "Baseline is not safely resumable")
        baseline.update(result="partial", close_extras={"status": "unavailable", "error": evidence.pop("error")},
                        profile_removed=not Path(baseline["profile"]).exists())
        evidence["result"] = "running"
        evidence["limitations"].append("Candidate resumed after baseline close-phase failure; same fixture HTML on a new loopback port.")
    else:
        # Reserve only a new evidence file; later updates belong to this run.
        with args.evidence.open("x", encoding="utf-8") as stream:
            json.dump(evidence, stream, indent=2)
    evidence["limitations"].append("Other UI checks were allowed concurrently; machine-wide isolation is not enforced.")
    try:
        with fixture() as site:
            evidence["fixture_url"] = site.origin + "/start"
            versions = [("candidate_v0.2", args.candidate)] if args.resume_candidate else [
                ("baseline_v0.1", args.baseline), ("candidate_v0.2", args.candidate)]
            for label, exe in versions:
                if label.startswith("candidate") and args.wait_candidate:
                    evidence["result"] = "waiting_for_candidate_ready"
                    save()
                    print("WAITING: baseline finished and cleaned up. Send ready to run candidate.", flush=True)
                    require(input().strip().casefold() == "ready", "Candidate was not authorized ready")
                item = {}
                evidence["versions"].append(item)
                print(f"RUN: {label}", flush=True)
                run_version(label, exe.resolve(), site, args.collector.resolve(), args, item)
                save()
            evidence["fixture"] = site.report()
        baseline, candidate = evidence["versions"]
        evidence["candidate_minus_baseline"] = [
            {"state": a["state"], "private_commit_bytes": b["private_commit_bytes"] - a["private_commit_bytes"],
             "private_working_set_bytes": b["private_working_set_bytes"] - a["private_working_set_bytes"],
             "process_count": b["process_count"] - a["process_count"]}
            for a, b in zip(baseline["samples"], candidate["samples"]) if a["state"] == b["state"]]
        evidence["result"] = "pass" if all(v["result"] == "pass" for v in evidence["versions"]) else "partial"
    except (Exception, KeyboardInterrupt) as error:
        evidence.update(result="fail", error=f"{type(error).__name__}: {error}")
        print(evidence["error"], file=sys.stderr, flush=True)
    finally:
        evidence["updated_at_utc"] = utc()
        save()
    print(f"{evidence['result'].upper()}: {args.evidence}", flush=True)
    return int(evidence["result"] != "pass")


if __name__ == "__main__":
    raise SystemExit(main())
