#!/usr/bin/env python3
"""Representative public CLI contracts against an owned synthetic usbmux server."""

import json
import os
from pathlib import Path
import plistlib
import socket
import struct
import sys
import threading

from diagnostics import ROOT, assess, capture, secret_variants
from regression import SafeArgumentParser


def read_exact(connection, size):
    data = bytearray()
    while len(data) < size:
        chunk = connection.recv(size - len(data))
        if not chunk:
            raise ValueError("fixture-eof")
        data.extend(chunk)
    return bytes(data)


def cli_list(binary, *, human=False, empty=False):
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
                    payload = plistlib.dumps(dict(DeviceList=entries))
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
        command = [str(binary)] + (["--no-json"] if human else []) + ["list"]
        try:
            result = capture(command, env=env, timeout=5)
        finally:
            worker.join(timeout=5)
        if worker.is_alive() or errors or valid != [True]:
            raise ValueError("fixture-server-failed")
    report = assess(result, secret_variants(secret), 0)
    if human:
        wanted = b"No devices connected.\n" if empty else (
            "\n".join(f"{d['udid']:<45} {d['connection_type']} (id={d['device_id']})" for d in expected) + "\n").encode()
        contract = result.stdout.replace(b"\r\n", b"\n") == wanted
    else:
        try:
            contract = json.dumps(json.loads(result.stdout), sort_keys=True) == json.dumps(expected, sort_keys=True)
        except (ValueError, TypeError):
            contract = False
    report["contract"] = contract
    report["passed"] = report["passed"] and contract and not result.stderr
    return report


def main():
    parser = SafeArgumentParser(prog="compatibility.py", description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios"))
    args = parser.parse_args()
    reports = {f"list-{'human' if human else 'json'}-{'empty' if empty else 'two-devices'}":
               cli_list(args.binary, human=human, empty=empty) for human in (False, True) for empty in (False, True)}
    print(json.dumps(reports, sort_keys=True))
    return 0 if all(report["passed"] for report in reports.values()) else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        print('{"passed":false,"reason":"compatibility-check-failed"}', file=sys.stderr)
        sys.exit(1)
