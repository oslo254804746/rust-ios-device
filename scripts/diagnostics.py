#!/usr/bin/env python3
"""Host-only raw diagnostic checks; sensitive bytes never enter reports."""

import argparse
import base64
from dataclasses import dataclass
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import tempfile
import time
import urllib.parse
import uuid

ROOT = Path(__file__).resolve().parents[1]
MAX_CAPTURE_BYTES = 256 * 1024


def secret_variants(secret):
    if len(secret) < 8:
        raise ValueError("fixture secret must be at least eight bytes")
    text = secret.decode("utf-8")
    return set((secret, secret.hex().encode(), secret.hex().upper().encode(),
                base64.b64encode(secret), base64.urlsafe_b64encode(secret),
                urllib.parse.quote_from_bytes(secret, safe="").encode(),
                urllib.parse.quote_plus(text, safe="").encode(),
                json.dumps(text, ensure_ascii=True)[1:-1].encode(),
                json.dumps(text, ensure_ascii=False)[1:-1].encode()))


def raw_hits(data, variants):
    """Count matching representations without returning matched secret text."""
    return sum(value in data for value in variants)


def masked_copy(data, variants):
    for value in sorted(variants, key=len, reverse=True):
        data = data.replace(value, b"[REDACTED]")
    return data


@dataclass
class Capture:
    stdout: bytes
    stderr: bytes
    returncode: int
    timed_out: bool
    overflow: bool
    incomplete: bool


def capture(command, *, env=None, timeout=10, max_bytes=MAX_CAPTURE_BYTES):
    """Bound RAM and time; terminate only this owned fixture process/group.

    Unix uses a new session to clean up inherited pipe writers. On Windows the
    host cases launch only ios (which does not launch child processes); general
    Windows process-tree supervision belongs to the later workflow harness.
    """
    if timeout <= 0 or max_bytes <= 0:
        raise ValueError("capture budgets must be positive")
    process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=os.name == "posix")
    buffers = [bytearray(), bytearray()]
    lock = threading.Lock()
    overflow = threading.Event()
    read_failed = threading.Event()
    completed = [threading.Event(), threading.Event()]
    used = [0]

    def reader(pipe, index):
        try:
            while True:
                chunk = pipe.read1(4096)
                if not chunk:
                    break
                with lock:
                    available = max_bytes - used[0]
                    buffers[index].extend(chunk[:available])
                    used[0] += min(len(chunk), available)
                    if len(chunk) > available:
                        overflow.set()
        except (OSError, ValueError):
            read_failed.set()
        finally:
            pipe.close()
            completed[index].set()

    workers = [threading.Thread(target=reader, args=(pipe, index), daemon=True)
               for index, pipe in enumerate((process.stdout, process.stderr))]
    for worker in workers:
        worker.start()
    deadline = time.monotonic() + timeout
    timed_out = False
    try:
        while not (process.poll() is not None and all(done.is_set() for done in completed)):
            if overflow.wait(0.02):
                break
            if time.monotonic() >= deadline:
                timed_out = True
                break
    finally:
        if process.poll() is None or not all(done.is_set() for done in completed):
            if os.name == "posix":
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            elif process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=0.5)
            except subprocess.TimeoutExpired:
                pass
            # The leader can exit while an owned descendant still holds pipes.
            if os.name == "posix":
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            elif process.poll() is None:
                process.kill()
        process.wait(timeout=2)
        for worker in workers:
            worker.join(timeout=1)
    with lock:
        return Capture(bytes(buffers[0]), bytes(buffers[1]), process.returncode,
                       timed_out, overflow.is_set(), read_failed.is_set()
                       or any(worker.is_alive() for worker in workers))


def assess(result, variants, expected_code):
    raw = result.stdout + result.stderr
    original_hits = raw_hits(raw, variants)
    copy_hits = raw_hits(masked_copy(raw, variants), variants)
    return dict(returncode=result.returncode, bytes=len(raw), raw_hits=original_hits,
                masked_copy_hits=copy_hits, timed_out=result.timed_out,
                overflow=result.overflow, incomplete=result.incomplete,
                passed=bool(raw) and result.returncode == expected_code and original_hits == 0
                and not result.timed_out and not result.overflow and not result.incomplete)


def cli_cases(binary):
    secret = ("synthetic-credential-UDID-" + uuid.uuid4().hex).encode()
    value = secret.decode()
    variants = secret_variants(secret)
    env = os.environ.copy()
    env.update(IOS_UDID=value, P12_PASSWORD=value, IOS_WIFI_PASSWORD=value,
               USBMUXD_SOCKET_ADDRESS="tcp://127.0.0.1:" + value)
    cases = [("invalid-port", ["forward", value, "62078"], 2),
             ("host-validation", ["forward", "0", "62078"], 1),
             # Invalid numeric socket address fails before any device connection.
             ("usbmux-source-error", ["-vvvv", "list"], 1)]
    for name, args in [("root", []), ("wifi", ["wifi", "install"]),
                       ("pair", ["pair", "supervised"]), ("prepare", ["prepare", "create-cert"]),
                       ("profiles", ["profiles", "add"]),
                       ("httpproxy", ["httpproxy", "set"]), ("mdm", ["mdm"]),
                       ("tunnel-auth-create", ["tunnel", "auth", "create"]),
                       ("tunnel-auth-check", ["tunnel", "auth", "check"])]:
        cases.append(("help-" + name, args + ["--help"], 0))
    reports = {}
    for name, args, code in cases:
        result = capture([str(binary)] + args, env=env)
        report = assess(result, variants, code)
        required = {
            "invalid-port": (b"argument parsing failed", b"ValueValidation"),
            "host-validation": (b"forward failed:", b"error:"),
            "usbmux-source-error": (b"list failed:", b"usbmux error:"),
        }.get(name, (b"Usage:",))
        report["contract"] = all(marker in result.stdout + result.stderr for marker in required)
        report["passed"] = report["passed"] and report["contract"]
        reports[name] = report
    reports.update(credential_cases(binary, env, variants, value))
    return reports


def credential_cases(binary, env, variants, device):
    """Create only this fixture's private files and detect the generated token.

    TemporaryDirectory owns cleanup; no production policy or device is used.
    Generated token bytes are read with a fixed bound, checked in RAM, and never
    printed. Creation is a two-file operation, so dependent cases fail closed.
    """
    reports = {}
    env = env.copy()
    env.pop("IOS_UDID", None)
    with tempfile.TemporaryDirectory(prefix="ios-auth-diagnostic-") as directory:
        config = str(Path(directory) / "policy.json")
        token_path = Path(directory) / "client-token"
        create_args = ["-vvvv", "tunnel", "auth", "create", "--config", config,
                       "--token-file", str(token_path), "--permission", "connect",
                       "--device", device, "--port", "58783"]
        created = capture([str(binary)] + create_args, env=env)
        try:
            with token_path.open("rb") as file:
                token = file.read(67)
            if len(token) != 64 or any(byte not in b"0123456789abcdef" for byte in token):
                raise ValueError("invalid generated fixture credential")
            checked_variants = variants | secret_variants(token)
            creation_contract = json.loads(created.stdout).get("operation") == "credential_create"
            creation_contract = creation_contract and json.loads(created.stdout).get("data") is True
        except (OSError, ValueError, TypeError, AttributeError):
            checked_variants = variants
            creation_contract = False
        report = assess(created, checked_variants, 0)
        report["contract"] = creation_contract
        report["passed"] = report["passed"] and creation_contract
        reports["auth-create"] = report
        if not report["passed"]:
            for name in ("auth-check", "auth-denied-port"):
                reports[name] = {"passed": False, "reason": "credential fixture creation failed"}
            return reports
        base = ["-vvvv", "tunnel", "auth", "check", "--config", config,
                "--token-file", str(token_path), "--permission", "connect", "--device", device]
        for name, port, code in [("auth-check", "58783", 0), ("auth-denied-port", "22", 1)]:
            result = capture([str(binary)] + base + ["--port", port], env=env)
            report = assess(result, checked_variants, code)
            if code == 0:
                try:
                    reply = json.loads(result.stdout)
                    contract = reply.get("operation") == "credential_check" and reply.get("data") is True
                except (ValueError, TypeError, AttributeError):
                    contract = False
            else:
                contract = all(marker in result.stderr for marker in (b"tunnel failed:", b"error:"))
            report["contract"] = contract
            report["passed"] = report["passed"] and contract
            reports[name] = report
    return reports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug" /
                        ("ios.exe" if os.name == "nt" else "ios"))
    parser.add_argument("--build-cli", action="store_true")
    args = parser.parse_args()
    if args.build_cli:
        subprocess.run(["cargo", "build", "-p", "ios-cli", "--locked"], cwd=ROOT, check=True)
    reports = cli_cases(args.binary.resolve())
    # Never print subprocess argv, env, raw output, or secret variants.
    print(json.dumps({"kind": "host-diagnostic-capture", "cases": reports}, indent=2))
    return 0 if all(case["passed"] for case in reports.values()) else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, subprocess.SubprocessError, ValueError):
        print("[FAIL] diagnostic capture setup failed", file=sys.stderr)
        sys.exit(1)
