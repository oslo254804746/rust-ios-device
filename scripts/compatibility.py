#!/usr/bin/env python3
"""Representative public CLI contracts against an owned synthetic usbmux server."""

import json
from dataclasses import replace
import os
from pathlib import Path
import plistlib
import signal
import socket
import struct
import subprocess
import sys
import threading
import time

from diagnostics import ROOT, assess, capture, secret_variants
from regression import SafeArgumentParser
from backup_contracts import closed_output


def read_exact(connection, size):
    data = bytearray()
    while len(data) < size:
        chunk = connection.recv(size - len(data))
        if not chunk:
            raise ValueError("fixture-eof")
        data.extend(chunk)
    return bytes(data)


def cli_list(binary, *, human=False, empty=False, close_stdout=False,
             error_json=False, malformed_reply=False):
    """Reply only to one ListDevices request; never route to an actual device."""
    expected = [] if empty else [dict(udid="synthetic-compatibility-usb", device_id=42,
                                    connection_type="USB", product_id=1234),
                               dict(udid="synthetic-compatibility-network", device_id=43,
                                    connection_type="Network", product_id=0)]
    valid = []
    errors = []
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(5)

        def serve():
            try:
                with listener.accept()[0] as connection:
                    connection.settimeout(3)
                    length, version, kind, tag = struct.unpack("<IIII", read_exact(connection, 16))
                    if not 16 < length <= 16384 or version != 1 or kind != 8:
                        raise ValueError("fixture-request")
                    request = plistlib.loads(read_exact(connection, length - 16))
                    if request.get("MessageType") != "ListDevices":
                        raise ValueError("fixture-operation")
                    entries = []
                    for device in expected:
                        properties = dict(SerialNumber=device["udid"], ConnectionType=device["connection_type"])
                        if device["product_id"]:
                            properties["ProductID"] = device["product_id"]
                        entries.append(dict(DeviceID=device["device_id"], Properties=properties))
                    payload = (b"synthetic-private-invalid-plist" if malformed_reply else
                               plistlib.dumps(dict(DeviceList=entries)))
                    connection.sendall(struct.pack("<IIII", 16 + len(payload), 1, 8, tag) + payload)
                    valid.append(True)
            except Exception:
                errors.append(True)

        worker = threading.Thread(target=serve, daemon=True)
        worker.start()
        secret = b"synthetic-compatibility-private-environment"
        env = os.environ.copy()
        env.update(USBMUXD_SOCKET_ADDRESS="tcp://127.0.0.1:" + str(listener.getsockname()[1]),
                   IOS_UDID=secret.decode())
        command = [str(binary)] + (["--error-format=json"] if error_json else [])
        command += (["--no-json"] if human else []) + ["list"]
        if close_stdout:
            command = [sys.executable, str(Path(__file__).resolve()), "--closed-output",
                       "--binary", str(binary)] + (["--human"] if human else [])
        try:
            result = capture(command, env=env, timeout=5)
        finally:
            worker.join(timeout=5)
        if worker.is_alive() or errors or valid != [True]:
            raise ValueError("fixture-server-failed")
    variants = secret_variants(secret) | secret_variants(b"synthetic-private-invalid-plist")
    report = assess(result, variants, 1 if malformed_reply else 0)
    if malformed_reply:
        contract = machine_error_contract(result, error_class="protocol", phase="list",
                                          exit_code=1, effects="unknown")
    elif close_stdout:
        contract = (result.stdout == b'{"child_exit": 1}\n' and
                    b"io/BrokenPipe" in result.stderr and b"panicked" not in result.stderr)
    elif human:
        wanted = b"No devices connected.\n" if empty else (
            "\n".join(f"{d['udid']:<45} {d['connection_type']} (id={d['device_id']})" for d in expected) + "\n").encode()
        contract = result.stdout.replace(b"\r\n", b"\n") == wanted
    else:
        try:
            contract = json.dumps(json.loads(result.stdout), sort_keys=True) == json.dumps(expected, sort_keys=True)
        except (ValueError, TypeError):
            contract = False
    report["contract"] = contract
    report["passed"] = report["passed"] and contract and (close_stdout or malformed_reply or not result.stderr)
    return report


def machine_error_contract(result, *, error_class, phase, exit_code, effects):
    """Require one versioned final stderr error and the actual process status."""
    try:
        if result.stdout or result.returncode != exit_code or not result.stderr.endswith(b"\n"):
            return False
        if len(result.stderr.splitlines()) != 1:
            return False
        body = json.loads(result.stderr)
        error = body["error"]
        return (type(body["schema_version"]) is int and body["schema_version"] == 1 and
                error["class"] == error_class and error["phase"] == phase and
                type(error["exit_code"]) is int and error["exit_code"] == exit_code and
                error["effects"] == effects and error["confirmation_required"] is False and
                isinstance(error["diagnostic"], str) and "error:" in error["diagnostic"])
    except (ValueError, TypeError, KeyError):
        return False


def cli_argument_error(binary, *, machine=False):
    secret = b"synthetic-private-invalid-cli-port"
    with socket.socket() as trap:
        trap.bind(("127.0.0.1", 0))
        trap.listen(1)
        env = os.environ.copy()
        env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + str(trap.getsockname()[1])
        command = [str(binary)] + (["--error-format=json"] if machine else [])
        command += ["forward", secret.decode(), "62078"]
        result = capture(command, env=env, timeout=5)
        trap.setblocking(False)
        try:
            connection, _ = trap.accept()
        except BlockingIOError:
            contacted_device_endpoint = False
        else:
            connection.close()
            contacted_device_endpoint = True
    report = assess(result, secret_variants(secret), 2)
    if machine:
        contract = machine_error_contract(result, error_class="invalid_input",
                                          phase="argument_parsing", exit_code=2, effects="not_started")
    else:
        contract = (not result.stdout and b"argument parsing failed (ValueValidation);" in result.stderr and
                    b"Use 'ios --help'" in result.stderr)
    report["contract"] = contract and not contacted_device_endpoint
    report["passed"] = report["passed"] and report["contract"]
    report["device_connections"] = int(contacted_device_endpoint)
    return report


def cli_help_closed_output(binary, flag, *, machine=False):
    result = capture([sys.executable, str(Path(__file__).resolve()), "--closed-output",
                      "--binary", str(binary), "--help-kind", flag] +
                     (["--machine-errors"] if machine else []), timeout=5)
    report = assess(result, secret_variants(b"synthetic-output-private-environment"), 0)
    if machine:
        report["contract"] = (result.stdout == b'{"child_exit": 1}\n' and
                              machine_error_contract(replace(result, stdout=b"", returncode=1),
                                                     error_class="disconnected", phase="help/version output",
                                                     exit_code=1, effects="unknown"))
    else:
        report["contract"] = (result.stdout == b'{"child_exit": 1}\n' and
                              b"io/BrokenPipe" in result.stderr and b"panicked" not in result.stderr)
    report["passed"] = report["passed"] and report["contract"]
    return report


def webinspector_arguments(subcommand):
    if subcommand == "eval":
        return ["webinspector", "eval", "6*7", "--page-id", "7"]
    if subcommand in {"opened-tabs", "launch", "js-shell"}:
        return ["webinspector", subcommand]
    raise ValueError("fixture-webinspector-command")


def cli_notification_deadline_error(binary, subcommand, timeout, *, explicit=True, machine=True, family="notify"):
    """Reject finite timeout input before selection/setup; trap any device contact."""
    private_device = b"synthetic-notification-private-device"
    private_name = b"synthetic-notification-private-name"
    with socket.socket() as trap:
        trap.bind(("127.0.0.1", 0))
        trap.listen()
        env = os.environ.copy()
        env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + str(trap.getsockname()[1])
        command = [str(binary)] + (["--error-format=json"] if machine else [])
        if explicit:
            env["IOS_UDID"] = private_device.decode()
            command += ["--udid", private_device.decode()]
        else:
            env.pop("IOS_UDID", None)
        command += webinspector_arguments(subcommand) if family == "webinspector" else [family, subcommand]
        if family == "companion" and subcommand in {"forward", "stop"}:
            command += ["42"]
        elif family == "notify" and subcommand != "springboard":
            command += [private_name.decode()]
        command += ["--timeout=" + str(timeout)] if family == "webinspector" else ["--timeout", str(timeout)]
        result = capture(command, env=env, timeout=5)
        trap.setblocking(False)
        device_connections = 0
        while True:
            try:
                connection, _ = trap.accept()
            except BlockingIOError:
                break
            connection.close()
            device_connections += 1
    report = assess(result, secret_variants(private_device) | secret_variants(private_name), 1)
    error_class = "timeout" if family == "webinspector" and timeout == 0 else "invalid_input"
    if machine:
        contract = machine_error_contract(result, error_class=error_class, phase=family,
                                          exit_code=1, effects="unknown")
    else:
        contract = (not result.stdout and result.stderr.startswith((family + " failed:").encode()) and
                    b"io/InvalidInput" in result.stderr and b"panicked" not in result.stderr)
    report["device_connections"] = device_connections
    report["contract"] = contract and device_connections == 0
    report["passed"] = report["passed"] and report["contract"]
    return report


def cli_notification_empty_default(binary, *, family="notify", subcommand="opened-tabs"):
    """Serve one empty discovery reply to a valid default-selection command."""
    valid, errors = [], []
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(5)

        def serve():
            try:
                with listener.accept()[0] as connection:
                    connection.settimeout(3)
                    length, version, kind, tag = struct.unpack("<IIII", read_exact(connection, 16))
                    if not 16 < length <= 16384 or version != 1 or kind != 8:
                        raise ValueError("fixture-request")
                    request = plistlib.loads(read_exact(connection, length - 16))
                    if request.get("MessageType") != "ListDevices":
                        raise ValueError("fixture-operation")
                    payload = plistlib.dumps(dict(DeviceList=[]))
                    connection.sendall(struct.pack("<IIII", 16 + len(payload), 1, 8, tag) + payload)
                    valid.append(True)
            except Exception:
                errors.append(True)

        worker = threading.Thread(target=serve, daemon=True)
        worker.start()
        env = os.environ.copy()
        env.pop("IOS_UDID", None)
        env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + str(listener.getsockname()[1])
        private_name = b"synthetic-notification-private-name"
        try:
            command = (webinspector_arguments(subcommand) if family == "webinspector" else
                       ["notify", "wait", private_name.decode()] if family == "notify" else
                       ["companion", "listen"])
            result = capture([str(binary), "--error-format=json", *command, "--timeout", "1"],
                             env=env, timeout=5)
        finally:
            worker.join(timeout=5)
        if worker.is_alive() or errors or valid != [True]:
            raise ValueError("fixture-server-failed")
        listener.setblocking(False)
        unexpected_connections = 0
        while True:
            try:
                connection, _ = listener.accept()
            except BlockingIOError:
                break
            connection.close()
            unexpected_connections += 1
    report = assess(result, secret_variants(private_name), 1)
    report["contract"] = machine_error_contract(result, error_class="unknown", phase=family,
                                                 exit_code=1, effects="unknown")
    report["contract"] = report["contract"] and unexpected_connections == 0
    report["passed"] = report["passed"] and report["contract"]
    report["list_requests"] = len(valid)
    report["unexpected_connections"] = unexpected_connections
    return report


def cli_webinspector_stalled_default(binary, subcommand):
    """Own one unanswered ListDevices socket; require total expiry and release."""
    valid, errors = [], []
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(3)

        def serve():
            try:
                with listener.accept()[0] as connection:
                    connection.settimeout(3)
                    length, version, kind, _tag = struct.unpack("<IIII", read_exact(connection, 16))
                    if not 16 < length <= 16384 or version != 1 or kind != 8:
                        raise ValueError("fixture-request")
                    request = plistlib.loads(read_exact(connection, length - 16))
                    if request.get("MessageType") != "ListDevices":
                        raise ValueError("fixture-operation")
                    valid.append(connection.recv(1) == b"")
            except Exception:
                errors.append(True)

        worker = threading.Thread(target=serve, daemon=True)
        worker.start()
        env = os.environ.copy()
        env.pop("IOS_UDID", None)
        env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + str(listener.getsockname()[1])
        started = time.monotonic()
        try:
            result = capture([str(binary), "--error-format=json", *webinspector_arguments(subcommand),
                              "--timeout", "0.2"], env=env, timeout=3)
        finally:
            worker.join(timeout=3)
        elapsed = time.monotonic() - started
        if worker.is_alive() or errors or valid != [True]:
            raise ValueError("fixture-server-failed")
        listener.setblocking(False)
        unexpected_connections = 0
        while True:
            try:
                connection, _ = listener.accept()
            except BlockingIOError:
                break
            connection.close()
            unexpected_connections += 1
    report = assess(result, secret_variants(b"synthetic-private-cancel-device"), 1)
    report["contract"] = (machine_error_contract(result, error_class="timeout", phase="webinspector",
                                                exit_code=1, effects="unknown") and
                          elapsed < 2 and unexpected_connections == 0)
    report["list_requests"] = len(valid)
    report["peer_eof"] = valid == [True]
    report["unexpected_connections"] = unexpected_connections
    report["passed"] = report["passed"] and report["contract"]
    return report


def interrupted_launch(binary, *, notification_setup=False, companion_setup=False, webinspector_setup=None):
    """Interrupt only an owned CLI blocked on its sole ListDevices endpoint."""
    process = None
    try:
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(3)
            env = os.environ.copy()
            env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + str(listener.getsockname()[1])
            if webinspector_setup is not None:
                env.pop("IOS_UDID", None)
                command = [str(binary), "--error-format=json", *webinspector_arguments(webinspector_setup),
                           "--timeout", "5"]
            elif companion_setup:
                env.pop("IOS_UDID", None)
                command = [str(binary), "--error-format=json", "companion", "listen", "--timeout", "5"]
            elif notification_setup:
                env.pop("IOS_UDID", None)
                command = [str(binary), "--error-format=json", "notify", "wait",
                           "synthetic-notification-private-name", "--timeout", "5"]
            else:
                command = [str(binary), "--error-format=json", "-u",
                           "synthetic-private-cancel-device", "webinspector", "launch"]
            process = subprocess.Popen(command, env=env)
            with listener.accept()[0] as connection:
                connection.settimeout(3)
                length, version, kind, _tag = struct.unpack("<IIII", read_exact(connection, 16))
                if not 16 < length <= 16384 or version != 1 or kind != 8:
                    raise ValueError("fixture-request")
                request = plistlib.loads(read_exact(connection, length - 16))
                if request.get("MessageType") != "ListDevices":
                    raise ValueError("fixture-operation")
                # No reply, pairing record, device service or private protocol
                # is supplied. The production launch cannot advance to a device.
                process.send_signal(signal.SIGINT)
                code = process.wait(timeout=3)
                eof = connection.recv(1) == b""
            print(json.dumps({"child_exit": code, "list_request": True, "peer_eof": eof,
                              "signal": "SIGINT"}, sort_keys=True))
            return code == 1 and eof
    except Exception:
        print('{"child_exit":null,"fixture_failed":true}')
        return False
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=0.5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=0.5)


def cli_launch_cancellation(binary, *, notification_setup=False, companion_setup=False, webinspector_setup=None):
    flag = ("--interrupt-companion-setup" if companion_setup else
            "--interrupt-notification-setup" if notification_setup else "--interrupt-launch")
    probe = (["--interrupt-webinspector-setup", webinspector_setup] if webinspector_setup is not None else [flag])
    result = capture([sys.executable, str(Path(__file__).resolve()), *probe,
                      "--binary", str(binary)], timeout=6)
    report = assess(result, secret_variants(b"synthetic-private-cancel-device") |
                    secret_variants(b"synthetic-notification-private-name"), 0)
    try:
        trace = json.loads(result.stdout)
    except (ValueError, TypeError):
        trace = None
    report["contract"] = (trace == {"child_exit": 1, "list_request": True, "peer_eof": True,
                                    "signal": "SIGINT"} and
                          machine_error_contract(replace(result, stdout=b"", returncode=1),
                                                 error_class="cancelled", phase="companion" if companion_setup else "notify" if notification_setup else "webinspector",
                                                 exit_code=1, effects="unknown"))
    report["passed"] = report["passed"] and report["contract"]
    return report


def main():
    parser = SafeArgumentParser(prog="compatibility.py", description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios"))
    parser.add_argument("--closed-output", action="store_true", help="internal owned output probe")
    parser.add_argument("--human", action="store_true", help="internal owned human output probe")
    parser.add_argument("--machine-errors", action="store_true", help="internal owned JSON failure probe")
    parser.add_argument("--interrupt-launch", action="store_true", help="internal owned SIGINT probe")
    parser.add_argument("--interrupt-notification-setup", action="store_true", help="internal owned notification SIGINT probe")
    parser.add_argument("--interrupt-companion-setup", action="store_true", help="internal owned Companion SIGINT probe")
    parser.add_argument("--interrupt-webinspector-setup", choices=["opened-tabs", "eval", "launch", "js-shell"],
                        help="internal owned WebInspector default-selection SIGINT probe")
    parser.add_argument("--help-kind", choices=["help", "version"], help="internal owned help/version probe")
    args = parser.parse_args()
    if args.interrupt_launch or args.interrupt_notification_setup or args.interrupt_companion_setup or args.interrupt_webinspector_setup:
        if not sys.platform.startswith("linux"):
            print('{"status":"NOT_RUN","reason":"Linux owned process group required"}')
            return 1
        return 0 if interrupted_launch(args.binary, notification_setup=args.interrupt_notification_setup,
                                       companion_setup=args.interrupt_companion_setup,
                                       webinspector_setup=args.interrupt_webinspector_setup) else 1
    if args.closed_output:
        if not sys.platform.startswith("linux"):
            print('{"status":"NOT_RUN","reason":"Linux owned process group required"}')
            return 0
        command = [str(args.binary)] + (["--error-format=json"] if args.machine_errors else [])
        command += ["--" + args.help_kind] if args.help_kind else (["--no-json"] if args.human else []) + ["list"]
        return 0 if closed_output(command) else 1
    reports = {f"list-{'human' if human else 'json'}-{'empty' if empty else 'two-devices'}":
               cli_list(args.binary, human=human, empty=empty) for human in (False, True) for empty in (False, True)}
    reports["list-with-json-errors-success"] = cli_list(args.binary, error_json=True)
    reports["list-with-json-errors-malformed-reply"] = cli_list(args.binary, error_json=True, malformed_reply=True)
    reports["argument-error-human"] = cli_argument_error(args.binary)
    reports["argument-error-json"] = cli_argument_error(args.binary, machine=True)
    reports["notification-empty-default"] = cli_notification_empty_default(args.binary)
    for subcommand in ["roundtrip", "observe", "wait", "springboard", "stream"]:
        for kind, timeout in [("zero", 0), ("overflow", (1 << 64) - 1)]:
            for explicit in [False, True]:
                key = f"notification-{subcommand}-{kind}-{'explicit' if explicit else 'default'}-json"
                reports[key] = cli_notification_deadline_error(args.binary, subcommand, timeout, explicit=explicit)
    for kind, timeout in [("zero", 0), ("overflow", (1 << 64) - 1)]:
        reports[f"notification-wait-{kind}-human"] = cli_notification_deadline_error(args.binary, "wait", timeout, machine=False)
    reports["companion-empty-default"] = cli_notification_empty_default(args.binary, family="companion")
    for subcommand in ["listen", "forward", "stop"]:
        for kind, timeout in [("zero", 0), ("overflow", (1 << 64) - 1)]:
            for explicit in [False, True]:
                key = f"companion-{subcommand}-{kind}-{'explicit' if explicit else 'default'}-json"
                reports[key] = cli_notification_deadline_error(args.binary, subcommand, timeout, explicit=explicit, family="companion")
    for kind, timeout in [("zero", 0), ("overflow", (1 << 64) - 1)]:
        reports[f"companion-listen-{kind}-human"] = cli_notification_deadline_error(args.binary, "listen", timeout, machine=False, family="companion")
    for subcommand in ["opened-tabs", "eval", "launch", "js-shell"]:
        reports[f"webinspector-{subcommand}-empty-default"] = cli_notification_empty_default(
            args.binary, family="webinspector", subcommand=subcommand)
        reports[f"webinspector-{subcommand}-stalled-default"] = cli_webinspector_stalled_default(args.binary, subcommand)
        for kind, timeout in [("zero", 0), ("clock-overflow", 1e19), ("duration-overflow", sys.float_info.max)]:
            for explicit in [False, True]:
                key = f"webinspector-{subcommand}-{kind}-{'explicit' if explicit else 'default'}-json"
                reports[key] = cli_notification_deadline_error(args.binary, subcommand, timeout,
                                                               explicit=explicit, family="webinspector")
        reports[f"webinspector-{subcommand}-overflow-human"] = cli_notification_deadline_error(
            args.binary, subcommand, 1e19, machine=False, family="webinspector")
    if sys.platform.startswith("linux"):
        reports["launch-cancel-json-error"] = cli_launch_cancellation(args.binary)
        reports["notification-setup-cancel-json-error"] = cli_launch_cancellation(args.binary, notification_setup=True)
        reports["companion-setup-cancel-json-error"] = cli_launch_cancellation(args.binary, companion_setup=True)
        for subcommand in ["opened-tabs", "eval", "launch", "js-shell"]:
            reports[f"webinspector-{subcommand}-default-cancel-json-error"] = cli_launch_cancellation(
                args.binary, webinspector_setup=subcommand)
        for human in (False, True):
            reports[f"list-{'human' if human else 'json'}-closed-output"] = cli_list(args.binary, human=human, close_stdout=True)
        for flag in ["help", "version"]:
            reports[flag + "-closed-output"] = cli_help_closed_output(args.binary, flag)
            reports[flag + "-closed-output-json-error"] = cli_help_closed_output(args.binary, flag, machine=True)
    else:
        reports["closed-output"] = {"status": "NOT_RUN", "reason": "Linux owned process group required"}
    print(json.dumps(reports, sort_keys=True))
    return 0 if all(report.get("status") == "NOT_RUN" or report["passed"] for report in reports.values()) else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        print('{"passed":false,"reason":"compatibility-check-failed"}', file=sys.stderr)
        sys.exit(1)
