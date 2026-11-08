#!/usr/bin/env python3
"""Explicit bounded host PBKDF2 vector; ordinary validation keeps it ignored."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from diagnostics import ROOT, capture

TEST = "services::backup2::crypto::tests::pyiosbackup_keybag_and_payload_vector"
MAX_OUTPUT_BYTES = 2 * 1024 * 1024
SOURCE_FILES = ("Cargo.toml", "Cargo.lock", "crates/ios-core/Cargo.toml",
                "crates/ios-core/src/services/backup2/crypto.rs",
                "crates/ios-core/src/services/backup2/manifest.rs", "scripts/high_cost.py",
                "scripts/validate.py", "scripts/diagnostics.py", "scripts/test_high_cost.py")


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def command():
    return ["cargo", "test", "-p", "ios-core", "--lib", "--no-default-features",
            "--features", "backup2-manifest", "--release", "--locked",
            "--target-dir", str(ROOT / "target"), "--message-format=json", TEST,
            "--", "--exact", "--ignored", "--color", "never"]


def assess(result, artifact_root):
    """Require both the selected harness result and its actual Cargo artifact."""
    output = result.stdout.decode("utf-8", errors="replace")
    rows = re.findall(r"(?m)^test result: ok\. (\d+) passed; (\d+) failed; "
                      r"(\d+) ignored; (\d+) measured; (\d+) filtered out;", output)
    executed = bool(rows and len(rows) == 1 and rows[0][:4] == ("1", "0", "0", "0")
                    and re.search(r"(?m)^test " + re.escape(TEST) + r" \.\.\. ok$", output))
    artifacts = []
    malformed = False
    for line in output.splitlines():
        if not line.startswith("{"):
            continue
        try:
            message = json.loads(line)
        except (json.JSONDecodeError, RecursionError):
            malformed = True
            continue
        if not isinstance(message, dict):
            malformed = True
            continue
        if message.get("reason") == "compiler-artifact":
            target, profile = message.get("target"), message.get("profile")
            if not isinstance(target, dict) or not isinstance(profile, dict):
                malformed = True
                continue
            if target.get("name") == "ios_core" and target.get("kind") == ["lib"] \
                    and profile.get("test") is True and message.get("executable"):
                artifacts.append(message["executable"])
    binary = None
    if len(artifacts) == 1 and isinstance(artifacts[0], str):
        try:
            candidate = Path(artifacts[0]).resolve(strict=True)
            candidate.relative_to(Path(artifact_root).resolve(strict=True))
            if candidate.is_file() and candidate.stat().st_size <= 1024 * 1024 * 1024:
                binary = dict(path=str(candidate), sha256=digest(candidate))
        except (OSError, ValueError):
            pass
    clean = (result.returncode == 0 and not result.timed_out and not result.overflow
             and not result.incomplete and result.cleanup_complete
             and result.cleanup_scope == "process-group" and not result.stop_requested)
    return dict(passed=bool(clean and executed and binary and not malformed),
                selected_test_executed=executed, artifact=binary,
                test_counts=dict(passed=int(rows[0][0]), failed=int(rows[0][1]),
                                 ignored=int(rows[0][2]), measured=int(rows[0][3]),
                                 filtered=int(rows[0][4])) if len(rows) == 1 else None,
                returncode=result.returncode, timed_out=result.timed_out,
                overflow=result.overflow, incomplete=result.incomplete,
                cleanup_complete=result.cleanup_complete, cleanup_scope=result.cleanup_scope,
                duration_ms=result.duration_ms, stdout_sha256=hashlib.sha256(result.stdout).hexdigest(),
                stderr_sha256=hashlib.sha256(result.stderr).hexdigest())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=900,
                        help="seconds for Cargo compilation plus vector execution (default:900)")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or not 0 < args.timeout <= 3600:
        parser.error("--timeout must be finite and within (0,3600] seconds")
    if os.name != "posix":
        parser.error("high-cost Cargo children require process-group supervision on this host")
    evidence = dict(schema_version=1, suite="backup2-pbkdf2", test=TEST,
                    timeout_seconds=args.timeout, max_output_bytes=MAX_OUTPUT_BYTES,
                    build_profile="release", real_device_executed=False, passed=False)
    try:
        evidence["source_revision"] = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        evidence["source_sha256"] = {name: digest(ROOT / name) for name in SOURCE_FILES}
        evidence["rustc"] = subprocess.check_output(["rustc", "--version"], cwd=ROOT, text=True).strip()
        invocation = command()
        result = capture(invocation, timeout=args.timeout, max_bytes=MAX_OUTPUT_BYTES)
        evidence["command"] = invocation
        evidence.update(assess(result, ROOT / "target"))
        parent = ROOT / "target/validation/high-cost"
        parent.mkdir(parents=True, exist_ok=True)
        directory = Path(tempfile.mkdtemp(prefix="pbkdf2-", dir=parent))
        # Keep bounded, original Cargo/harness output in a newly owned ignored
        # directory. Metadata hashes can be checked against these actual bytes.
        for label, data in (("stdout", result.stdout), ("stderr", result.stderr)):
            log = directory / (label + ".log")
            log.write_bytes(data)
            evidence[label + "_log"] = str(log.relative_to(ROOT))
        # Never turn a concurrent edit into evidence for the earlier source.
        if any(digest(ROOT / name) != expected for name, expected in evidence["source_sha256"].items()):
            evidence.update(passed=False, source_changed=True)
    except (OSError, ValueError, subprocess.SubprocessError):
        evidence.update(passed=False, runner_failure=True)
    print(json.dumps(evidence, sort_keys=True))
    return 0 if evidence["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
