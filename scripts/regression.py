#!/usr/bin/env python3
"""Host and real-device regression evidence with bounded capture, precondition checks and owned cleanup (Python 3.9+)."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from datetime import datetime, timezone
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import threading
import time
from typing import Optional

from diagnostics import ROOT, assess, capture, group_alive, raw_hits, secret_variants

MATRIX = ROOT / "docs/regression-matrix.json"
PUBLIC_ARGUMENTS = frozenset(("--version", "-vvvv", "forward", "62078", "-c", "/", ".",
                              "info", "list", "apps", "file", "ls", "pull", "tunnel",
                              "pasteboard", "watch", "doctor", "--udid", "--force"))
HOST_IDS = frozenset(("host-argv", "host-cli-version", "host-cli-rejection",
                     "host-agent", "host-artifacts"))
PROVENANCE_FIELDS = frozenset(("commit", "working_tree_dirty", "binary_sha256", "binary_version",
                              "matrix_sha256", "runner_sha256", "capture_sha256", "windows_owner_sha256", "python_version",
                              "platform", "architecture", "date"))
ARCHITECTURES = frozenset(("x86_64", "aarch64", "arm64", "i686", "AMD64", "unrecorded"))

UDID_PATTERN = re.compile(r"^[0-9a-fA-F]{24,40}$|^[0-9a-fA-F]{8}-[0-9a-fA-F]{16}$")


def validate_udid(udid: str) -> str:
    if not isinstance(udid, str) or not UDID_PATTERN.match(udid):
        raise ValueError("invalid-device-udid")
    return udid


@dataclass
class DeviceTarget:
    udid: str
    transport: str = "unobserved"
    name: str = "target-device"
    ios_version: str = "unobserved"
    device_state: str = "unobserved"
    services: frozenset = frozenset()
    has_wifi_trust: bool = False
    has_rsd_identity: bool = False
    has_identity: bool = False
    is_coredevice: bool = False

    def __post_init__(self):
        validate_udid(self.udid)


@dataclass
class ExecutionContext:
    binary: Path
    device: Optional[DeviceTarget] = None
    scopes: frozenset = frozenset(("read-scope", "binary-provenance"))
    fixtures: dict = field(default_factory=dict)
    observed_services: frozenset = frozenset()
    observed_modes: frozenset = frozenset()
    evidence: dict = field(default_factory=dict)
    environment: Optional[dict] = None
    fixture_peer: bool = False
    fixture_manifest_sha256: Optional[str] = None


def detect_device(binary: Path) -> Optional[DeviceTarget]:
    if not binary or not Path(binary).is_file():
        return None
    try:
        res = capture([str(binary), "list"], timeout=5)
        if res.returncode == 0 and res.stdout:
            entries = json.loads(res.stdout)
            # Selection is deliberately ambiguous when more than one device is
            # present. List proves transport/discovery, never trust or services.
            if isinstance(entries, list) and len(entries) == 1:
                entry = entries[0]
                transport = {"USB": "USB", "Network": "Wi-Fi"}.get(entry.get("connection_type"), "unobserved")
                return DeviceTarget(udid=entry["udid"], transport=transport, device_state="discovered")
    except Exception:
        pass
    return None


def check_provenance(value):
    if set(value) != PROVENANCE_FIELDS or type(value["working_tree_dirty"]) is not bool:
        raise ValueError("provenance-schema")
    for field, length in (("commit", 40), ("binary_sha256", 64), ("matrix_sha256", 64),
                          ("runner_sha256", 64), ("capture_sha256", 64), ("windows_owner_sha256", 64)):
        if not re.fullmatch(r"[0-9a-f]{" + str(length) + "}", value[field]):
            raise ValueError("provenance-hash")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", value["binary_version"]):
        raise ValueError("provenance-version")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", value["python_version"]):
        raise ValueError("provenance-python")
    if value["platform"] not in ("linux", "darwin", "win32") or value["architecture"] not in ARCHITECTURES:
        raise ValueError("provenance-host")
    if len(value["date"]) > 40 or datetime.fromisoformat(value["date"]).tzinfo is None:
        raise ValueError("provenance-date")


def load_matrix():
    data = json.loads(MATRIX.read_text(encoding="utf-8"))
    if set(data) != {"version", "selection", "cases"} or data["version"] != 1:
        raise ValueError("matrix-schema")
    seen = set()
    for case in data["cases"]:
        if set(case) != {"id", "level", "platform", "transport", "scope", "requires", "reason"}:
            raise ValueError("matrix-schema")
        if not re.fullmatch(r"[a-z][a-z0-9-]{1,63}", case["id"]) or case["id"] in seen:
            raise ValueError("matrix-id")
        if case["level"] not in ("host-protocol", "simulated-flow", "real-device"):
            raise ValueError("matrix-level")
        if not isinstance(case["requires"], list) or any(not isinstance(p, str) for p in case["requires"]):
            raise ValueError("matrix-preconditions")
        seen.add(case["id"])
    if not HOST_IDS <= seen or len(seen) > 128:
        raise ValueError("matrix-coverage")
    return data


def outside_checkout(path):
    resolved = path.resolve()
    if resolved == ROOT or ROOT in resolved.parents:
        raise ValueError("evidence-inside-checkout")
    return resolved


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


class SafeArgumentParser(argparse.ArgumentParser):
    def error(self, message):
        # argparse's default error includes the untrusted offending argument.
        raise ValueError("argument-parsing-failed")


class Session:
    """Only allowlisted metadata is written. Raw bytes and private paths stay in RAM.

    Commands and contract callbacks are trusted Python callers, not user-supplied
    shell/manifest text. Real-device operation adapters are validated against
    exact fixtures, devices and granted operation scopes before execution.
    """

    def __init__(self, parent=None):
        self.matrix = load_matrix()
        parent = outside_checkout(Path(parent or tempfile.gettempdir()))
        if not parent.is_dir():
            raise ValueError("output-parent-unavailable")
        self.evidence = Path(tempfile.mkdtemp(prefix="ios-regression-evidence-", dir=parent))
        try:
            self.private = Path(tempfile.mkdtemp(prefix="ios-regression-private-", dir=parent))
        except BaseException:
            self.evidence.rmdir()
            raise
        self.private_identity = self.identity(self.private)
        self.evidence_identity = self.identity(self.evidence)
        self.files = {}
        self.variants = set()
        self.results = {}
        self.agents = []
        self.closed = False
        self.key = os.urandom(32).hex().encode()
        try:
            self.private_file("argument-key", self.key)
        except BaseException:
            self.cleanup()
            raise

    @staticmethod
    def identity(path):
        value = path.lstat()
        return value.st_dev, value.st_ino

    def ensure_open(self):
        if self.closed:
            raise ValueError("session-closed")

    def private_file(self, name, data):
        self.ensure_open()
        if not re.fullmatch(r"[a-z][a-z0-9-]{1,63}", name) or len(data) > 65536:
            raise ValueError("private-file-shape")
        variants = secret_variants(data)
        path = self.private / name
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(path, flags, 0o600)
        value = os.fstat(descriptor)
        self.files[name] = (value.st_dev, value.st_ino)
        self.variants.update(variants)
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
        return path

    def arguments(self, command):
        if not 1 <= len(command) <= 128 or any(len(str(arg)) > 16384 for arg in command):
            raise ValueError("argument-budget")
        return [dict(index=index, value=str(arg) if index and str(arg) in PUBLIC_ARGUMENTS else "[PRIVATE]",
                     tag=hmac.new(self.key, str(arg).encode(), hashlib.sha256).hexdigest()[:16],
                     utf8_bytes=len(str(arg).encode()),
                     utf16_units=len(str(arg).encode("utf-16-le")) // 2)
                for index, arg in enumerate(command)]

    def record(self, case_id, command, result, expected_code, contract):
        self.ensure_open()
        if case_id not in HOST_IDS or case_id in self.results:
            raise ValueError("case-id")
        checked = assess(result, self.variants, expected_code)
        passed = checked.pop("passed") and contract
        report = dict(checked, id=case_id, status="PASS" if passed else "FAIL",
                      contract=bool(contract), expected_code=expected_code,
                      duration_ms=result.duration_ms, pid=result.pid,
                      cleanup_scope=result.cleanup_scope, stop_requested=result.stop_requested,
                      argv=self.arguments(command), argv_observation="direct-subprocess-input")
        self.results[case_id] = report
        return report

    def run(self, case_id, command, *, expected_code=0, contract=None, env=None, timeout=10):
        self.ensure_open()
        if case_id not in HOST_IDS or case_id in self.results:
            raise ValueError("case-id")
        if contract is None or not 0 < timeout <= 90:
            raise ValueError("case-contract-budget")
        self.arguments(command)
        started = time.monotonic()
        try:
            result = capture(command, env=env, timeout=timeout)
            try:
                matched = bool(contract(result))
            except Exception:
                matched = False
            return self.record(case_id, command, result, expected_code, matched)
        except OSError:
            # Exception strings can contain paths, argv or remote payloads.
            report = dict(id=case_id, status="FAIL", reason="process-capture-failed",
                          returncode=None, expected_code=expected_code, contract=False,
                          duration_ms=round((time.monotonic() - started) * 1000),
                          cleanup_complete=False, cleanup_scope="unconfirmed", pid=None,
                          argv=self.arguments(command))
            self.results[case_id] = report
            return report

    def run_device_case(self, case_id, command, *, expected_code=0, contract=None, env=None, timeout=15):
        self.ensure_open()
        matrix_ids = set(c["id"] for c in self.matrix["cases"])
        if case_id not in matrix_ids or case_id in HOST_IDS or case_id in self.results:
            raise ValueError("case-id")
        if contract is None or not 0 < timeout <= 90:
            raise ValueError("case-contract-budget")
        self.arguments(command)
        started = time.monotonic()
        try:
            result = capture(command, env=env, timeout=timeout)
            try:
                matched = bool(contract(result))
            except Exception:
                matched = False
            checked = assess(result, self.variants, expected_code)
            passed = checked.pop("passed") and matched
            report = dict(checked, id=case_id, status="PASS" if passed else "FAIL",
                          contract=bool(matched), expected_code=expected_code,
                          evidence_origin="unclassified-subprocess",
                          duration_ms=result.duration_ms, pid=result.pid,
                          cleanup_scope=result.cleanup_scope, stop_requested=result.stop_requested,
                          argv=self.arguments(command), argv_observation="direct-subprocess-input")
            self.results[case_id] = report
            return report
        except OSError:
            report = dict(id=case_id, status="FAIL", reason="process-capture-failed",
                          returncode=None, expected_code=expected_code, contract=False,
                          duration_ms=round((time.monotonic() - started) * 1000),
                          cleanup_complete=False, cleanup_scope="unconfirmed", pid=None,
                          argv=self.arguments(command))
            self.results[case_id] = report
            return report

    def record_not_run(self, case_id, reason, missing_requirements=None):
        self.ensure_open()
        matrix_ids = set(c["id"] for c in self.matrix["cases"])
        if case_id not in matrix_ids or case_id in self.results:
            raise ValueError("case-id")
        report = dict(id=case_id, status="NOT_RUN", reason=reason,
                      missing_requirements=list(missing_requirements or []))
        self.results[case_id] = report
        return report

    def agent(self, command, *, timeout=10):
        self.ensure_open()
        if os.name != "nt" and not sys.platform.startswith("linux"):
            raise ValueError("process-tree-platform-unverified")
        if not 0 < timeout <= 10:
            raise ValueError("agent-budget")
        self.arguments(command)
        stop = threading.Event()
        executor = ThreadPoolExecutor(max_workers=1)
        future = executor.submit(capture, command, timeout=timeout, stop=stop)
        owned = (command, stop, executor, future)
        self.agents.append(owned)
        return owned

    def stop_agent(self, owned):
        command, stop, executor, future = owned
        stop.set()
        try:
            result = future.result(timeout=15)
        finally:
            executor.shutdown(wait=True)
            self.agents.remove(owned)
        return result

    def provenance(self, binary):
        commit = capture(["git", "rev-parse", "HEAD"], timeout=5, max_bytes=4096)
        dirty = capture(["git", "status", "--porcelain=v1", "--untracked-files=normal"],
                        timeout=5, max_bytes=65536)
        value = commit.stdout.strip().decode("ascii", errors="replace")
        if commit.returncode or dirty.returncode or not re.fullmatch(r"[0-9a-f]{40}", value) or any(
                r.timed_out or r.overflow or r.incomplete or not r.cleanup_complete for r in (commit, dirty)):
            raise ValueError("provenance-unavailable")
        architecture = os.uname().machine if hasattr(os, "uname") else "unrecorded"
        return dict(commit=value, working_tree_dirty=bool(dirty.stdout),
                    binary_sha256=sha256_file(binary),
                    binary_version="unverified",
                    matrix_sha256=sha256_file(MATRIX),
                    runner_sha256=sha256_file(Path(__file__)),
                    capture_sha256=sha256_file(ROOT / "scripts/diagnostics.py"),
                    windows_owner_sha256=sha256_file(ROOT / "scripts/regression_windows_process.py"),
                    python_version=".".join(map(str, sys.version_info[:3])),
                    platform=sys.platform, architecture=architecture if architecture in ARCHITECTURES else "unrecorded",
                    date=datetime.now(timezone.utc).isoformat())

    def cleanup(self):
        if self.closed:
            return self.cleanup_report
        agent_failures = 0
        for owned in list(self.agents):
            try:
                result = self.stop_agent(owned)
                if not result.cleanup_complete or result.incomplete:
                    agent_failures += 1
                if "host-agent" not in self.results:
                    self.record("host-agent", owned[0], result, 1 if os.name == "nt" else -15, False)
            except Exception:
                agent_failures += 1
        unknown = 0
        removed = 0
        unsafe = 0
        # Do not follow swapped roots or links; do not recursively delete unknown
        # files, user content, existing identities or evidence from other runs.
        try:
            root_ok = not self.private.is_symlink() and self.private.resolve() == self.private and self.identity(self.private) == self.private_identity
        except OSError:
            root_ok = False
        if root_ok:
            for path in self.private.iterdir():
                if path.name not in self.files:
                    unknown += 1
                elif path.is_symlink() or not stat.S_ISREG(path.lstat().st_mode) or self.identity(path) != self.files[path.name]:
                    unsafe += 1
                else:
                    path.unlink()
                    removed += 1
            if not unknown and not unsafe:
                self.private.rmdir()
        else:
            unsafe += 1
        try:
            evidence_ok = not self.evidence.is_symlink() and self.evidence.resolve() == self.evidence and self.identity(self.evidence) == self.evidence_identity
        except OSError:
            evidence_ok = False
        evidence_unknown = len(list(self.evidence.iterdir())) if evidence_ok else 1
        self.cleanup_report = dict(private_files_removed=removed, private_unknown=unknown,
                                   private_unsafe=unsafe, evidence_unknown=evidence_unknown,
                                   agent_failures=agent_failures,
                                   complete=not (unknown or unsafe or evidence_unknown or agent_failures))
        self.closed = True
        self.key = b""
        return self.cleanup_report

    def finish(self, provenance, device=None, context=None):
        device_tag = (hmac.new(self.key, device.udid.encode(), hashlib.sha256).hexdigest()[:16]
                      if device and self.key else None)
        cleanup = self.cleanup()
        check_provenance(provenance)
        rows = []
        for case in self.matrix["cases"]:
            result = self.results.get(case["id"], dict(id=case["id"], status="NOT_RUN", reason=case["reason"]))
            if case["id"] in HOST_IDS or device is None:
                dev_val = None
                ios_val = None
                dev_states = "not-observed"
                serv_avail = "not-probed"
            else:
                dev_val = device_tag
                ios_val = device.ios_version
                dev_states = device.device_state
                serv_avail = "probed" if device.services else "not-probed"
            rows.append(dict(case, result=result, device=dev_val, ios=ios_val,
                             device_states=dev_states, service_availability=serv_avail))

        applicable_host_ids = set(
            row["id"] for row in rows
            if row["id"] in HOST_IDS and (row["platform"] == "current" or row["platform"] == sys.platform
                                          or (row["platform"] == "linux" and sys.platform.startswith("linux")))
        )
        host_passed = cleanup["complete"] and all(
            row["result"]["status"] == "PASS" for row in rows if row["id"] in applicable_host_ids
        ) and bool(applicable_host_ids)

        real_device_executed = any(
            row["result"]["status"] in ("PASS", "FAIL") and row["id"] not in HOST_IDS
            and row["result"].get("evidence_origin") == "selected-device-cli"
            for row in rows
        )
        report = dict(version=1, provenance=provenance, cases=rows, cleanup=cleanup,
                      adapter_source_sha256=sha256_file(ROOT / "scripts/regression_adapters.py"),
                      adapter_support_source_sha256={name: sha256_file(ROOT / "scripts" / name) for name in
                                                     ("regression_watch_capture.py", "regression_watch_windows.py", "regression_windows_process.py",
                                                      "regression_cli_fixture.py", "regression_dtx_fixture.py", "regression_rsd_fixture.py")},
                      fixture_manifest_sha256=context.fixture_manifest_sha256 if context else None,
                      host_passed=host_passed,
                      real_device_executed=real_device_executed)
        encoded = (json.dumps(report, ensure_ascii=True, sort_keys=True, indent=2) + "\n").encode()
        if raw_hits(encoded, self.variants):
            raise ValueError("evidence-secret-detected")
        if cleanup["evidence_unknown"]:
            raise ValueError("evidence-not-allowlisted")
        destination = self.evidence / "report.json"
        descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(encoded)
        return report


def check_preconditions(case: dict, context: ExecutionContext):
    """Validate platform, device, fixtures, and operation scopes for a case."""
    # 1. Platform check
    if case["platform"] == "Windows" and sys.platform != "win32":
        return False, "windows-host-required", ["windows-host"]
    if case["platform"] == "macOS" and sys.platform != "darwin":
        return False, "macos-host-required", ["macos-host"]
    if case["platform"] == "linux" and not sys.platform.startswith("linux"):
        return False, "linux-host-required", ["owned-process-group"]

    # 2. Requirements check
    missing = []
    if case["level"] == "real-device" and context.device is None:
        missing.append("selected-device")
    if context.device and case["transport"] in ("USB", "Wi-Fi") and context.device.transport != case["transport"]:
        missing.append("selected-transport-observed")
    for req in case.get("requires", []):
        if req == "selected-device":
            if context.device is None:
                missing.append(req)
        elif req == "read-scope":
            if "read-scope" not in context.scopes:
                missing.append(req)
        elif req == "binary-provenance":
            if "binary-provenance" not in context.scopes:
                missing.append(req)
        elif req == "windows-host":
            if sys.platform != "win32":
                missing.append(req)
        elif req == "macos-host":
            if sys.platform != "darwin":
                missing.append(req)
        elif req == "service-observed":
            avail = context.observed_services or (context.device.services if context.device else set())
            if not avail:
                missing.append(req)
        elif req == "existing-wifi-trust":
            if not (context.device and context.device.has_wifi_trust and context.device.transport == "Wi-Fi"):
                missing.append(req)
        elif req == "existing-rsd-identity":
            if not (context.device and context.device.has_rsd_identity):
                missing.append(req)
        elif req == "mode-observed":
            if not context.observed_modes:
                missing.append(req)
        elif req == "coexistence-scope":
            if "coexistence-scope" not in context.scopes:
                missing.append(req)
        elif req == "existing-identity":
            if not (context.device and context.device.has_identity):
                missing.append(req)
        elif req == "identity-observation-scope":
            if "identity-observation-scope" not in context.scopes:
                missing.append(req)
        elif req == "approved-ddi":
            if "approved-ddi" not in context.fixtures:
                missing.append(req)
        elif req == "install-scope":
            if "install-scope" not in context.scopes:
                missing.append(req)
        elif req == "protocol-evidence":
            if "protocol-evidence" not in context.evidence:
                missing.append(req)
        elif req == "observed-services":
            avail = context.observed_services or (context.device.services if context.device else set())
            if not avail:
                missing.append(req)
        elif req == "display-input-scope":
            if "display-input-scope" not in context.scopes:
                missing.append(req)
        elif req == "synthetic-clipboard":
            if "synthetic-clipboard" not in context.fixtures:
                missing.append(req)
        elif req == "clipboard-scope":
            if "clipboard-scope" not in context.scopes:
                missing.append(req)
        elif req == "format-evidence":
            if "format-evidence" not in context.evidence:
                missing.append(req)
        elif req == "test-application":
            if "test-application" not in context.fixtures:
                missing.append(req)
        elif req == "application-operation-scope":
            if "application-operation-scope" not in context.scopes:
                missing.append(req)
        elif req == "afc-fixture":
            if "afc-fixture" not in context.fixtures:
                missing.append(req)
        elif req == "test-bundle":
            if "test-bundle" not in context.fixtures:
                missing.append(req)
        elif req == "test-operation-scope":
            if "test-operation-scope" not in context.scopes:
                missing.append(req)
        elif req == "authorized-unc-share":
            if "authorized-unc-share" not in context.fixtures:
                missing.append(req)
        elif req == "protocol-observation-scope":
            if "protocol-observation-scope" not in context.scopes:
                missing.append(req)
        elif req == "synthetic-operation":
            if "synthetic-operation" not in context.scopes:
                missing.append(req)
        elif req == "built-cli":
            if not (context.binary and context.binary.is_file()):
                missing.append(req)
        elif req == "owned-process-group":
            if not sys.platform.startswith("linux"):
                missing.append(req)
        elif req == "owned-process-tree":
            if os.name != "nt" and not sys.platform.startswith("linux"):
                missing.append(req)
        else:
            missing.append(req)

    if missing:
        return False, case.get("reason", "prerequisite-missing"), missing
    return True, "preconditions-satisfied", []


def make_device_command(case_id: str, context: ExecutionContext):
    """Compatibility entry point for single-query adapters; no fallback query."""
    from regression_adapters import query_command
    return query_command(case_id, context)


def execute_device_case(session: Session, case: dict, context: ExecutionContext):
    case_id = case["id"]
    satisfied, reason, missing = check_preconditions(case, context)
    if not satisfied:
        session.record_not_run(case_id, reason=reason, missing_requirements=missing)
        return session.results[case_id]
    from regression_adapters import execute
    return execute(session, case, context)


def run_device_matrix(session: Session, context: ExecutionContext, case_filter=None):
    results = {}
    if context.device is not None and "read-scope" in context.scopes:
        try:
            observed = capture([str(context.binary), "list"], env=context.environment, timeout=5, max_bytes=65536)
            entries = json.loads(observed.stdout) if observed.returncode == 0 and observed.cleanup_complete and not observed.timed_out else []
            matches = [row for row in entries if isinstance(row, dict) and row.get("udid") == context.device.udid]
            if len(matches) == 1:
                context.device.transport = {"USB": "USB", "Network": "Wi-Fi"}.get(matches[0].get("connection_type"), "unobserved")
                context.device.device_state = "discovered"
        except Exception:
            pass
    for case in session.matrix["cases"]:
        if case["id"] in HOST_IDS:
            continue
        if case_filter and case["id"] not in case_filter:
            continue
        results[case["id"]] = execute_device_case(session, case, context)
    return results


def run_host_cases(session: Session, binary: Path, provenance: dict):
    secret = ("synthetic-regression-" + os.urandom(16).hex()).encode()
    identity = session.private_file("synthetic-identity", secret)
    # The child observes its own argv, without a shell or MSYS conversion.
    expected = ["/", ".", "synthetic-path-does-not-exist", "synthetic-路径", str(identity)]
    command = [sys.executable, "-c", "import json,sys;print(json.dumps(sys.argv[1:]))"] + expected
    session.run("host-argv", command, contract=lambda r: json.loads(r.stdout) == expected)
    session.results["host-argv"]["argv_observation"] = "child-sys.argv-compared-in-ram"

    def version_contract(result):
        if re.fullmatch(rb"ios [0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?\r?\n", result.stdout) and not result.stderr:
            provenance["binary_version"] = result.stdout.strip().decode("ascii")[4:]
            return True
        return False

    session.run("host-cli-version", [str(binary), "--version"], contract=version_contract)
    env = os.environ.copy()
    env.update(IOS_UDID=secret.decode(), USBMUXD_SOCKET_ADDRESS="tcp://127.0.0.1:" + secret.decode())
    session.run("host-cli-rejection", [str(binary), "-vvvv", "forward", secret.decode(), "62078"],
                env=env, expected_code=2,
                contract=lambda r: b"ValueValidation" in r.stderr and b"argument parsing failed" in r.stderr
                and b"usbmux" not in r.stderr)
    if os.name == "nt" or sys.platform.startswith("linux"):
        ready = session.private_file("agent-ready", b"pending-agent-ready")
        command = [sys.executable, "-c",
                   "import pathlib,sys,time;print('fixture-ready',flush=True);"
                   "pathlib.Path(sys.argv[1]).write_text('ready');time.sleep(30)", str(ready)]
        owned = session.agent(command)
        deadline = time.monotonic() + 3
        while ready.read_bytes() != b"ready" and not owned[3].done() and time.monotonic() < deadline:
            time.sleep(0.01)
        result = session.stop_agent(owned)
        session.record("host-agent", command, result, 1 if os.name == "nt" else -15,
                       ready.read_bytes() == b"ready" and b"fixture-ready" in result.stdout and result.stop_requested
                       and result.cleanup_complete
                       and (result.cleanup_scope == "windows-job" if os.name == "nt" else group_alive(result.pid) is False))


def host_run(binary, parent=None, device=None, context=None, case_filter=None):
    session = Session(parent)
    try:
        provenance = session.provenance(binary)
        run_host_cases(session, binary, provenance)
        if context is None:
            context = ExecutionContext(binary=binary, device=device)
        run_device_matrix(session, context, case_filter=case_filter)
        # Local cleanup is a filesystem API, not an uncaptured shell command.
        cleanup = session.cleanup()
        session.results["host-artifacts"] = dict(id="host-artifacts", status="PASS" if cleanup["complete"] else "FAIL",
                                                 contract=cleanup["complete"], private_outside_evidence=True)
        return session.finish(provenance, device=context.device, context=context), session.evidence
    finally:
        session.cleanup()


def main():
    parser = SafeArgumentParser(prog="regression.py", description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios"))
    parser.add_argument("--output-parent", type=Path, help="existing safe directory outside the checkout; creates new private run directories")
    parser.add_argument("--device", type=str, default=None, help="UDID of target device or 'auto'")
    parser.add_argument("--cases", type=str, default=None, help="comma-separated list of case IDs to run")
    parser.add_argument("--scope", type=str, default="read-scope,binary-provenance", help="comma-separated list of granted operation scopes")
    parser.add_argument("--fixture-dir", type=Path, default=None, help="path to directory containing test fixtures")
    parser.add_argument("--build-cli", action="store_true", help="build the ios CLI binary before running")
    args = parser.parse_args()

    if args.build_cli and not args.binary.is_file():
        subprocess.run(["cargo", "build", "-p", "ios-cli", "--locked"], cwd=ROOT, check=True)

    device = None
    if args.device == "auto":
        device = detect_device(args.binary)
    elif args.device:
        device = DeviceTarget(udid=validate_udid(args.device))

    scopes = frozenset(s.strip() for s in (args.scope or "").split(",") if s.strip())
    case_filter = set(c.strip() for c in args.cases.split(",")) if args.cases else None

    context = ExecutionContext(
        binary=args.binary,
        device=device,
        scopes=scopes,
    )
    if args.fixture_dir is not None:
        from regression_adapters import load_fixture_manifest
        load_fixture_manifest(args.fixture_dir, context)

    report, evidence = host_run(args.binary, args.output_parent, device=device, context=context, case_filter=case_filter)
    # CLI paths are user input and may contain secrets; stdout exposes only the
    # generated public run basename. Caller chooses/knows the output parent.
    print(json.dumps(dict(host_passed=report["host_passed"], evidence_directory=evidence.name,
                          real_device_executed=report["real_device_executed"], statuses={status: sum(
                              row["result"]["status"] == status for row in report["cases"])
                              for status in ("PASS", "FAIL", "NOT_RUN")})))
    return 0 if report["host_passed"] else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (Exception, KeyboardInterrupt):
        print('{"host_passed":false,"reason":"regression-run-failed"}', file=sys.stderr)
        sys.exit(1)
