#!/usr/bin/env python3
"""Host-only raw diagnostic checks; sensitive bytes never enter reports."""

import argparse
import base64
import hashlib
import http.client
from dataclasses import dataclass
import json
import math
import os
from pathlib import Path
import signal
import socket
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
    duration_ms: int = 0
    pid: int = 0
    cleanup_complete: bool = True
    cleanup_scope: str = "leader"
    stop_requested: bool = False


def group_alive(group):
    """Linux distinguishes terminated zombies; other Unix hosts fail conservatively.

    This observes the owned process group, not descendants which deliberately
    detach into another session. Workflow fixtures must not detach themselves.
    """
    if sys.platform.startswith("linux"):
        uncertain = False
        for entry in Path("/proc").iterdir():
            if not entry.name.isdecimal():
                continue
            try:
                fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
                if int(fields[2]) == group and fields[0] not in ("Z", "X"):
                    return True
            except FileNotFoundError:
                continue
            except (OSError, ValueError, IndexError):
                uncertain = True
        return None if uncertain else False
    try:
        os.killpg(group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return None


def capture(command, *, env=None, timeout=10, max_bytes=MAX_CAPTURE_BYTES, stop=None):
    """Bound RAM and time; terminate only this owned fixture process/group.

    Unix uses a new session; Windows atomically admits the child to its private
    Job. Both clean descendants even after leader exit or output pipe closure.
    cleanup_complete describes host ownership only, never a native/device ACK.
    """
    if not math.isfinite(timeout) or timeout <= 0 or type(max_bytes) is not int or max_bytes <= 0:
        raise ValueError("capture budgets must be positive")
    started = time.monotonic()
    windows = os.name == "nt"
    if windows:
        from regression_windows_process import CapturedProcess
        process = CapturedProcess(command, cwd=ROOT, env=env)
    else:
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
            try:
                pipe.close()
            except (OSError, ValueError):
                read_failed.set()
            finally:
                completed[index].set()

    workers = [threading.Thread(target=reader, args=(pipe, index), daemon=True,
                                name="owned-diagnostic-capture-reader")
               for index, pipe in enumerate((process.stdout, process.stderr))]
    launched = []
    deadline = started + timeout
    timed_out = False
    stop_requested = False
    cleanup_complete = False
    try:
        for worker in workers:
            worker.start()
            launched.append(worker)
        while not (process.poll() is not None and all(done.is_set() for done in completed)):
            if (stop is not None and stop.is_set() and not stop_requested
                    and (windows or process.poll() is None)):
                # Terminate only this capture's owned CLI after its service
                # contract checks. This is not a timeout or graceful-device ACK.
                process.terminate()
                stop_requested = True
            if overflow.wait(0.02) or read_failed.is_set():
                break
            if time.monotonic() >= deadline:
                timed_out = True
                break
    finally:
        if windows:
            try:
                # Include closed-pipe descendants; leader exit alone proves nothing
                # about the retained Job. There is no PID/name rediscovery here.
                if process.job.active():
                    process.terminate()
                process.wait(timeout=2)
                empty = process.job.empty(2)
                for worker in launched:
                    worker.join(timeout=1)
                cleanup_complete = empty and all(not worker.is_alive() for worker in launched)
            finally:
                process.close()
                for index, pipe in enumerate((process.stdout, process.stderr)):
                    if workers[index] not in launched:
                        pipe.close()
        else:
            cleanup_complete = _finish_capture_posix(process, launched, completed)
    with lock:
        return Capture(bytes(buffers[0]), bytes(buffers[1]), process.returncode,
                       timed_out, overflow.is_set(), read_failed.is_set()
                       or any(worker.is_alive() for worker in workers),
                       round((time.monotonic() - started) * 1000), process.pid,
                       cleanup_complete, "windows-job" if windows else "process-group" if os.name == "posix" else "leader",
                       stop_requested)


def _finish_capture_posix(process, workers, completed):
    # Even a successful leader may leave descendants which closed their
    # output pipes. Always inspect and stop this capture's owned group.
    group_remaining = os.name == "posix" and group_alive(process.pid) is not False
    if group_remaining or process.poll() is None or not all(done.is_set() for done in completed):
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
    if os.name == "posix":
        cleanup_deadline = time.monotonic() + 0.5
        while group_alive(process.pid) is True and time.monotonic() < cleanup_deadline:
            time.sleep(0.01)
        cleanup_complete = group_alive(process.pid) is False
    else:
        cleanup_complete = process.poll() is not None
    return cleanup_complete and all(not worker.is_alive() for worker in workers)


def assess(result, variants, expected_code):
    raw = result.stdout + result.stderr
    original_hits = raw_hits(raw, variants)
    copy_hits = raw_hits(masked_copy(raw, variants), variants)
    return dict(returncode=result.returncode, bytes=len(raw), raw_hits=original_hits,
                masked_copy_hits=copy_hits, timed_out=result.timed_out,
                overflow=result.overflow, incomplete=result.incomplete,
                cleanup_complete=result.cleanup_complete,
                passed=bool(raw) and result.returncode == expected_code and original_hits == 0
                and not result.timed_out and not result.overflow and not result.incomplete
                and result.cleanup_complete)


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
    cases += [("manager-explicit-access", ["-vvvv", "tunnel", "serve", "--userspace"], 1),
              ("manager-remote-plaintext", ["-vvvv", "tunnel", "serve", "--userspace",
                                             "--host", "0.0.0.0", "--local-unauthenticated"], 1)]
    for command in ("cdp", "selenium"):
        cases += [("debug-explicit-" + command, ["-vvvv", "webinspector", command], 1),
                  ("debug-remote-" + command, ["-vvvv", "webinspector", command,
                    "--host", "0.0.0.0", "--local-unauthenticated"], 1)]
        cases.append(("debug-invalid-" + command, ["-vvvv", "webinspector", command,
            "--local-unauthenticated", "--allow-origin", "https://" + value + "/path"], 1))
        cases.append(("debug-no-selector-" + command, ["-vvvv", "webinspector", command,
            "--local-unauthenticated"], 1))
    cases.append(("manager-invalid-boundary", ["-vvvv", "tunnel", "serve", "--userspace",
        "--local-unauthenticated", "--allow-origin", "https://" + value + "/path"], 1))
    cases.append(("manager-invalid-upstreams", ["-vvvv", "tunnel", "serve", "--userspace",
        "--local-unauthenticated", "--upstream-config", "synthetic-private-upstreams-" + value], 1))
    for name, args in [
        ("forward", ["forward", "1234", "62078"]),
        ("runwda", ["runwda", "com.synthetic.wda.xctrunner"]),
        ("dproxy", ["dproxy", "service", "com.synthetic.service"]),
        ("screenshot", ["screenshot", "--stream"])]:
        cases.append(("local-bind-" + name, ["-vvvv"] + args + ["--host", "0.0.0.0"], 1))
    for name, args in [("root", []), ("wifi", ["wifi", "install"]),
                       ("pair", ["pair", "supervised"]), ("prepare", ["prepare", "create-cert"]),
                       ("profiles", ["profiles", "add"]),
                       ("httpproxy", ["httpproxy", "set"]), ("mdm", ["mdm"]),
                       ("tunnel-auth-create", ["tunnel", "auth", "create"]),
                       ("tunnel-auth-check", ["tunnel", "auth", "check"])]:
        cases.append(("help-" + name, args + ["--help"], 0))
    reports = {}
    for name, args, code in cases:
        case_env = env.copy()
        if name.startswith("debug-no-selector-"):
            case_env.pop("IOS_UDID", None)
        result = capture([str(binary)] + args, env=case_env)
        report = assess(result, variants, code)
        required = {
            "invalid-port": (b"argument parsing failed", b"ValueValidation"),
            "host-validation": (b"forward failed:", b"error:"),
            "usbmux-source-error": (b"list failed:", b"usbmux error:"),
            "manager-explicit-access": (b"tunnel failed:", b"error:"),
            "manager-remote-plaintext": (b"tunnel failed:", b"error:"),
            "manager-invalid-boundary": (b"tunnel failed:", b"error:"),
            "manager-invalid-upstreams": (b"tunnel failed:", b"error:"),
        }.get(name, (b"Usage:",))
        if name.startswith("local-bind-"):
            required = ((name.removeprefix("local-bind-") + " failed:").encode(), b"error:")
        if name.startswith("debug-"):
            required = (b"webinspector failed:", b"error:")
        report["contract"] = all(marker in result.stdout + result.stderr for marker in required)
        if name.startswith("debug-no-selector-"):
            report["contract"] = report["contract"] and b"usbmux" not in result.stderr
        if name == "manager-invalid-upstreams":
            report["contract"] = report["contract"] and b"tunnel agent listening" not in result.stdout + result.stderr and b"usbmux" not in result.stderr
        report["passed"] = report["passed"] and report["contract"]
        reports[name] = report
    reports.update(doctor_cases(binary, env, variants, value))
    reports.update(credential_cases(binary, env, variants, value))
    return reports


def doctor_cases(binary, env, variants, device):
    """Exercise actual CLI report selection without contacting a real device."""
    reports = {}
    # Keep the port reserved without listening, so all device transport probes
    # fail at this owned endpoint rather than reaching the host's usbmuxd.
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        case_env = env.copy()
        case_env["USBMUXD_SOCKET_ADDRESS"] = (
            "tcp://127.0.0.1:" + str(reservation.getsockname()[1]))
        for name, args, explicit, human in [
            ("doctor-host-schema", [], False, False),
            ("doctor-device-flag", ["--udid", device], True, False),
            ("doctor-device-env", [], True, False),
            ("doctor-device-human", ["--udid", device, "--no-json"], True, True),
        ]:
            selected_env = case_env.copy()
            if not explicit:
                selected_env.pop("IOS_UDID", None)
            result = capture([str(binary), "-vvvv"] + args
                             + ["doctor", "--timeout", "1"], env=selected_env)
            report = assess(result, variants, 1)
            try:
                if human:
                    contract = (b"host and device diagnostics" in result.stdout
                                and b"trust_session" in result.stdout
                                and b"rsd_services" in result.stdout)
                else:
                    body = json.loads(result.stdout)
                    contract = body["schema_version"] == (2 if explicit else 1)
                    contract = contract and body["scope"] == (
                        "host_and_device" if explicit else "host")
                    contract = contract and isinstance(body["checks"], list)
                    if explicit:
                        checks = {check["id"]: check for check in body["device"]["checks"]}
                        contract = contract and set(checks) == {
                            "trust_session", "developer_mode", "ddi_signatures",
                            "rsd_services", "device_route"}
                        contract = contract and all(
                            checks[name]["state"] == "unreachable"
                            and checks[name]["status"] == "failed"
                            and checks[name].get("error_class") in {
                                "io", "disconnected", "timeout"}
                            and "value" not in checks[name]
                            for name in ("trust_session", "developer_mode", "ddi_signatures"))
                        contract = contract and all(
                            checks[name]["state"] == "not_probed"
                            and "value" not in checks[name]
                            for name in ("rsd_services", "device_route"))
                    else:
                        contract = contract and "device" not in body
            except (ValueError, TypeError, KeyError, AttributeError):
                contract = False
            report["contract"] = contract
            report["passed"] = report["passed"] and contract
            reports[name] = report
        result = capture([str(binary), "--error-format", "json", "--udid",
                          "../" + device, "doctor", "--timeout", "1"], env=case_env)
        report = assess(result, variants, 1)
        try:
            report["contract"] = (not result.stdout
                                  and json.loads(result.stderr)["error"]["class"] == "invalid_input")
        except (ValueError, TypeError, KeyError):
            report["contract"] = False
        report["passed"] = report["passed"] and report["contract"]
        reports["doctor-invalid-selector"] = report
    # Invalid existing-route selectors must fail before host/device/proxy IO.
    # A listening trap records actual connections; the CLI never receives an
    # answer that could disguise accidental work as a successful validation.
    with socket.socket() as trap:
        trap.bind(("127.0.0.1", 0))
        trap.listen()
        trap.setblocking(False)
        port = str(trap.getsockname()[1])
        endpoint = "[fd00:5678:1234::cafe]:58783"
        service = "synthetic-private-rsd-service-" * 160
        checked_variants = set(variants)
        checked_variants.update(secret_variants(endpoint.encode()))
        checked_variants.update(secret_variants(service.encode()))
        selected_env = env.copy()
        selected_env.pop("IOS_UDID", None)
        selected_env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + port
        for name, target, args, code, category in [
            ("doctor-rsd-needs-target", [], ["--rsd-address", endpoint], 1, "invalid_input"),
            ("doctor-rsd-zero-endpoint", ["--udid", device],
             ["--rsd-address", "[::1]:0"], 1, "invalid_input"),
            ("doctor-rsd-proxy-scope", ["--udid", device],
             ["--rsd-address", "[fd00:5678:1234::cafe%3]:58783",
              "--rsd-proxy-port", port], 1, "invalid_input"),
            ("doctor-rsd-service-limit", ["--udid", device],
             ["--rsd-address", endpoint, "--rsd-proxy-port", port,
              "--rsd-service", service], 1, "resource_limit"),
            ("doctor-rsd-zero-proxy", ["--udid", device],
             ["--rsd-address", endpoint, "--rsd-proxy-port", "0"], 2, "invalid_input"),
        ]:
            result = capture([str(binary), "-vvvv", "--error-format", "json"]
                             + target + ["doctor", "--timeout", "1"] + args,
                             env=selected_env)
            connections = 0
            while True:
                try:
                    peer, _ = trap.accept()
                except BlockingIOError:
                    break
                with peer:
                    connections += 1
            report = assess(result, checked_variants, code)
            try:
                report["contract"] = (not result.stdout and connections == 0
                                      and json.loads(result.stderr)["error"]["class"] == category)
            except (ValueError, TypeError, KeyError):
                report["contract"] = False
            report["device_proxy_connections"] = connections
            report["passed"] = report["passed"] and report["contract"]
            reports[name] = report
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
        reports.update(manager_cases(binary, env, checked_variants, device, token, Path(directory)))
        reports.update(debug_cases(binary, env, checked_variants, device, token, Path(directory)))
    return reports


def manager_cases(binary, env, variants, device, token, directory):
    """Capture the actual authenticated manager without any device permission.

    Only list/server roles are granted; its monitor skips discovery entirely.
    Own the bounded CLI process, private fixture files and loopback requests.
    Neither request headers nor raw stdout/stderr are archived or reported.
    """
    config = directory / "live-policy.json"
    variants = set(variants)
    for secret in (b"f" * 64, b"synthetic-denied-device", b"synthetic-invalid-policy", b"synthetic-invalid-body", b"synthetic-secret.evil.invalid"):
        variants.update(secret_variants(secret))
    policy = {"version": 1, "grants": [{
        "token_sha256": hashlib.sha256(token).hexdigest(),
        "expires_unix": int(time.time()) + 60,
        "permissions": ["list", "manage_server"], "devices": [device]}]}
    with os.fdopen(os.open(config, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as file:
        file.write(json.dumps(policy).encode())
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    results = []
    worker = threading.Thread(target=lambda: results.append(capture(
        [str(binary), "-vvvv", "tunnel", "serve", "--userspace", "--host", "127.0.0.1",
         "--port", str(port), "--auth-config", str(config)], env=env, timeout=12)), daemon=True)
    worker.start()
    checks = {}
    client_reports = {}

    def request(method, path, *, authorized=True, invalid=False, body=None, headers=None):
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=0.5)
        try:
            fields = dict(headers or {})
            if authorized:
                fields["Authorization"] = "Bearer " + ("f" * 64 if invalid else token.decode())
            connection.request(method, path, body=body, headers=fields)
            response = connection.getresponse()
            data = response.read(4097)
            if len(data) > 4096 or raw_hits(data, variants):
                raise ValueError("manager diagnostic reply contract failed")
            return response.status, data
        finally:
            connection.close()

    try:
        deadline = time.monotonic() + 5
        while worker.is_alive() and time.monotonic() < deadline:
            try:
                status, _ = request("GET", "/ready")
                if status == 200:
                    checks["ready"] = True
                    break
            except (OSError, http.client.HTTPException, ValueError):
                pass
            time.sleep(0.02)
        if not checks.get("ready"):
            raise ValueError("owned manager did not become ready")
        for name, method, path in [
            ("list", "GET", "/tunnels"), ("connect", "GET", "/connect?port=invalid-secret"),
            ("admin", "POST", "/upstream"), ("reload", "POST", "/auth/reload")]:
            status, _ = request(method, path, authorized=False, body=b"synthetic-invalid-body")
            checks["unauthenticated-" + name] = status == 401
        checks["invalid-token"] = request("GET", "/tunnels", invalid=True)[0] == 401
        for name, headers in [
            ("untrusted-host", {"Host": "synthetic-secret.evil.invalid:1234"}),
            ("untrusted-origin", {"Origin": "https://synthetic-secret.evil.invalid"}),
            ("null-origin", {"Origin": "null"}),
            ("forwarded-host", {"X-Forwarded-Host": "synthetic-secret.evil.invalid"})]:
            checks[name] = request("POST", "/auth/reload", headers=headers)[0] == 403
        checks["same-origin-native-header"] = request("GET", "/tunnels", headers={
            "Origin": "http://127.0.0.1:" + str(port)})[0] == 200
        checks["admin-cannot-issue-connect-ticket"] = request("POST", "/auth/ticket", body=json.dumps({
            "origin": "http://127.0.0.1:" + str(port),
            "target": {"kind": "tunnel", "udid": device, "port": 12345}
        }).encode(), headers={"Content-Type": "application/json"})[0] == 403
        status, body = request("GET", "/tunnels")
        checks["authorized-list"] = status == 200 and json.loads(body) == []
        for name, args, code in [
            ("manager-client-list", ["tunnel", "list"], 0),
            ("manager-client-denied-stop", ["--udid", device, "tunnel", "stop"], 1)]:
            captured = capture([str(binary), "-vvvv"] + args + ["--port", str(port),
                "--token-file", str(directory / "client-token")], env=env)
            checked = assess(captured, variants, code)
            if code == 0:
                try:
                    contract = json.loads(captured.stdout) == []
                except (ValueError, TypeError):
                    contract = False
            else:
                contract = b"tunnel failed:" in captured.stderr and b"error:" in captured.stderr
            checked["contract"] = contract
            checked["passed"] = checked["passed"] and contract
            client_reports[name] = checked
        for name, token_path, expected_status in [
            ("doctor-manager-auth", directory / "client-token", "ok"),
            ("doctor-manager-missing-token", directory / "missing-token", "failed")]:
            captured = capture([str(binary), "-vvvv", "doctor", "--timeout", "1",
                "--tunnel-port", str(port), "--token-file", str(token_path)], env=env)
            # usbmux is intentionally unavailable: the overall doctor fails,
            # while its manager check must independently show the right result.
            checked = assess(captured, variants, 1)
            try:
                doctor = json.loads(captured.stdout)
                manager = [entry for entry in doctor["checks"] if entry["id"] == "tunnel_manager"]
                contract = len(manager) == 1 and manager[0]["status"] == expected_status
                contract = contract and doctor.get("scope") == "host"
            except (ValueError, TypeError, KeyError, AttributeError):
                contract = False
            checked["contract"] = contract
            checked["passed"] = checked["passed"] and contract
            client_reports[name] = checked
        denied, _ = request("GET", "/connect?udid=synthetic-denied-device&port=22", headers={
            "Connection": "Upgrade", "Upgrade": "websocket", "Sec-WebSocket-Version": "13",
            "Sec-WebSocket-Key": base64.b64encode(b"owned-16byte-key!").decode()})
        checks["denied-connect"] = denied == 403
        config.write_bytes(b"synthetic-invalid-policy")
        checks["failed-reload"] = request("POST", "/auth/reload")[0] == 400
        checks["old-policy-survives"] = request("GET", "/tunnels")[0] == 200
        config.write_bytes(json.dumps(policy).encode())
        status, body = request("POST", "/auth/reload")
        checks["authorized-reload"] = status == 200 and json.loads(body).get("data") is True
        checks["legacy-shutdown-is-readonly"] = request("GET", "/shutdown")[0] in (405, 410)
        checks["alive-after-legacy-get"] = request("GET", "/health")[0] == 200
        status, body = request("POST", "/shutdown")
        checks["authorized-shutdown"] = status == 200 and json.loads(body).get("data") is True
    except (OSError, http.client.HTTPException, ValueError, TypeError, AttributeError):
        checks["fixture-contract"] = False
    worker.join(timeout=15)
    if not results or worker.is_alive():
        return {"manager-live": {"passed": False, "reason": "owned manager capture incomplete"}}
    result = results[0]
    report = assess(result, variants, 0)
    report["http_checks"] = checks
    report["contract"] = bool(checks) and all(checks.values()) and b"tunnel agent listening" in result.stderr
    report["passed"] = report["passed"] and report["contract"]
    return dict(client_reports, **{"manager-live": report})


def debug_cases(binary, env, variants, device, token, directory):
    """Capture production debug admission using metadata only, without devices.

    Denied device routes never reach their backend. Authorized paths are only
    local adapter metadata/reload, not Safari/CDP/Remote Automation evidence.
    """
    reports = {}
    variants = set(variants)
    for secret in (b"synthetic-invalid-policy", b"synthetic-invalid-body", b"f" * 64, b"synthetic-secret.evil.invalid"):
        variants.update(secret_variants(secret))
    for command, metadata in (("cdp", "/json/version"), ("selenium", "/status")):
        config = directory / (command + "-policy.json")
        policy = {"version": 1, "grants": [{
            "token_sha256": hashlib.sha256(token).hexdigest(),
            "expires_unix": int(time.time()) + 60,
            "permissions": ["debug", "manage_server"], "devices": [device]}]}
        with os.fdopen(os.open(config, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as file:
            file.write(json.dumps(policy).encode())
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        stop = threading.Event()
        results = []
        worker = threading.Thread(target=lambda: results.append(capture(
            [str(binary), "-vvvv", "--udid", device, "webinspector", command,
             "--port", str(port), "--auth-config", str(config)],
            env=env, timeout=10, stop=stop)), daemon=True)
        worker.start()
        checks = {}

        def request(method, path, authorized=True, invalid=False, body=None, fields=None):
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=0.5)
            try:
                headers = dict(fields or {})
                if authorized:
                    headers["Authorization"] = "Bearer " + ("f" * 64 if invalid else token.decode())
                connection.request(method, path, body=body, headers=headers)
                response = connection.getresponse()
                data = response.read(4097)
                if len(data) > 4096 or raw_hits(data, variants):
                    raise ValueError("debug admission reply contract failed")
                return response.status, data
            finally:
                connection.close()

        try:
            deadline = time.monotonic() + 4
            while worker.is_alive() and time.monotonic() < deadline:
                try:
                    status, body = request("GET", metadata)
                    if status == 200:
                        reply = json.loads(body)
                        checks["metadata"] = (
                            reply.get("User-Agent") == "ios-cli" if command == "cdp"
                            else reply.get("value", {}).get("ready") is True)
                        break
                except (OSError, http.client.HTTPException, ValueError):
                    pass
                time.sleep(0.02)
            if not checks.get("metadata"):
                raise ValueError("owned debug listener did not become ready")
            cases = [("GET", metadata), ("POST", "/auth/reload"), ("GET", "/not-a-route")]
            cases += ([("GET", "/json/list"), ("GET", "/devtools/page/synthetic-page")]
                      if command == "cdp" else [("POST", "/session"), ("POST", "/session/synthetic-session/url")])
            for index, (method, path) in enumerate(cases):
                status, body = request(method, path, authorized=False, body=b"synthetic-invalid-body")
                reply = json.loads(body)
                shape = (reply.get("error", {}).get("code") == -32600 if command == "cdp"
                         else reply.get("value", {}).get("error") == "invalid request")
                checks["unauthenticated-" + str(index)] = status == 401 and shape
            checks["invalid-token"] = request("GET", metadata, invalid=True)[0] == 401
            for name, fields in [
                ("untrusted-host", {"Host": "synthetic-secret.evil.invalid:1234"}),
                ("untrusted-origin", {"Origin": "https://synthetic-secret.evil.invalid"}),
                ("null-origin", {"Origin": "null"}),
                ("forwarded-host", {"X-Forwarded-Host": "synthetic-secret.evil.invalid"})]:
                checks[name] = request("POST", "/auth/reload", fields=fields)[0] == 403
            checks["same-origin-native-header"] = request("GET", metadata, fields={
                "Origin": "http://127.0.0.1:" + str(port)})[0] == 200
            if command == "cdp":
                origin = "http://127.0.0.1:" + str(port)
                status, body = request("POST", "/auth/ticket", body=json.dumps({
                    "origin": origin, "target": {"kind": "cdp_page", "page_id": "synthetic-page"}
                }).encode(), fields={"Content-Type": "application/json"})
                issued = json.loads(body)
                ticket = issued.get("ticket", "").encode()
                checks["browser-ticket-issued"] = status == 200 and len(ticket) == 64 and issued.get("expires_in") == 30
                if not checks["browser-ticket-issued"] or any(byte not in b"0123456789abcdef" for byte in ticket):
                    raise ValueError("browser ticket issuance contract failed")
                variants.update(secret_variants(ticket))
                # Ticket response is intentional credential delivery, checked
                # before adding that ticket to the diagnostic leakage detector.
                # Plain HTTP never upgrades/contacts a device, even with a ticket.
                checks["browser-ticket-http-denied"] = request("GET", issued["websocket_path"], authorized=False,
                    fields={"Origin": origin})[0] == 401
                checks["long-lived-url-denied"] = request("GET", "/devtools/page/synthetic-page?ticket=" + token.decode(),
                    authorized=False, fields={"Origin": origin})[0] == 401
            config.write_bytes(b"synthetic-invalid-policy")
            checks["failed-reload"] = request("POST", "/auth/reload")[0] == 400
            checks["old-policy-survives"] = request("GET", metadata)[0] == 200
            policy["grants"][0]["permissions"] = ["manage_server"]
            config.write_bytes(json.dumps(policy).encode())
            checks["reload"] = request("POST", "/auth/reload")[0] == 200
            checks["admin-does-not-imply-debug"] = request("GET", metadata)[0] == 403
            policy["grants"][0]["expires_unix"] = int(time.time())
            config.write_bytes(json.dumps(policy).encode())
            checks["expiry-reload"] = request("POST", "/auth/reload")[0] == 200
            checks["expired-token"] = request("GET", metadata)[0] == 401
        except (OSError, http.client.HTTPException, ValueError, TypeError, AttributeError):
            checks["fixture-contract"] = False
        finally:
            stop.set()
        worker.join(timeout=12)
        if not results or worker.is_alive():
            reports["debug-live-" + command] = {"passed": False, "reason": "owned debug capture incomplete"}
            continue
        expected = -signal.SIGTERM if os.name == "posix" else 1
        report = assess(results[0], variants, expected)
        report["http_checks"] = checks
        report["contract"] = bool(checks) and all(checks.values()) and b"Listening on http://" in results[0].stdout
        report["passed"] = report["passed"] and report["contract"]
        reports["debug-live-" + command] = report
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
