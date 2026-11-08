"""Owned malformed and checked ListDevices contracts for linked C calls."""

import os
import plistlib
import socket
import struct
import threading

from diagnostics import assess, capture, secret_variants


SECRET = b"synthetic-ffi-private-malformed-plist"
ENCODING_SECRET = b"synthetic-ffi-private\x00unencodable"


def assess_owned_error(result, peer_complete):
    report = assess(result, secret_variants(SECRET), 0)
    report["contract"] = (peer_complete and not result.stderr and
                          result.stdout.replace(b"\r\n", b"\n") == b"C ABI owned error passed\n")
    report["passed"] = report["passed"] and report["contract"]
    return report


def assess_checked_list(result, peer_complete):
    # A C diagnostic could truncate at the NUL; scan that private prefix too.
    variants = secret_variants(ENCODING_SECRET) | secret_variants(ENCODING_SECRET.split(b"\x00", 1)[0])
    report = assess(result, variants, 0)
    report["contract"] = (peer_complete and not result.stderr and
                          result.stdout.replace(b"\r\n", b"\n") == b"C ABI checked list passed\n")
    report["passed"] = report["passed"] and report["contract"]
    return report


def read_exact(connection, size):
    data = bytearray()
    while len(data) < size:
        chunk = connection.recv(size - len(data))
        if not chunk:
            raise ValueError("owned peer EOF")
        data.extend(chunk)
    return bytes(data)


def malformed_list_error(binary, *, env=None):
    completed, errors = [], []
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(3)

        def serve():
            try:
                with listener.accept()[0] as connection:
                    connection.settimeout(3)
                    length, version, kind, tag = struct.unpack("<IIII", read_exact(connection, 16))
                    if not 16 < length <= 16384 or version != 1 or kind != 8:
                        raise ValueError("owned request header")
                    request = plistlib.loads(read_exact(connection, length - 16))
                    if request.get("MessageType") != "ListDevices":
                        raise ValueError("owned request operation")
                    connection.sendall(struct.pack("<IIII", 16 + len(SECRET), 1, 8, tag) + SECRET)
                    completed.append(True)
            except Exception:
                errors.append(True)

        worker = threading.Thread(target=serve, daemon=True)
        worker.start()
        child_env = (os.environ if env is None else env).copy()
        child_env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + str(listener.getsockname()[1])
        try:
            result = capture([str(binary), "--owned-list-error"], env=child_env, timeout=5)
        finally:
            worker.join(timeout=4)
        peer_complete = not worker.is_alive() and not errors and completed == [True]
    return assess_owned_error(result, peer_complete)


def checked_list_projection(binary, *, env=None):
    """Exercise both actual C entries over five bounded owned ListDevices replies."""
    completed, errors = [], []
    valid = [dict(DeviceID=42, Properties=dict(SerialNumber="owned-valid-before", ConnectionType="USB")),
             dict(DeviceID=45, Properties=dict(SerialNumber="owned-valid-after", ConnectionType="Network"))]
    bad_udid = dict(DeviceID=43, Properties=dict(SerialNumber=ENCODING_SECRET.decode(), ConnectionType="USB"))
    bad_type = dict(DeviceID=44, Properties=dict(SerialNumber="owned-invalid-type", ConnectionType=ENCODING_SECRET.decode()))
    mixed = [valid[0], bad_udid, bad_type, valid[1]]
    replies = [mixed, mixed, [valid[0], bad_type, valid[1]], valid, []]
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(3)

        def serve():
            try:
                for entries in replies:
                    with listener.accept()[0] as connection:
                        connection.settimeout(3)
                        length, version, kind, tag = struct.unpack("<IIII", read_exact(connection, 16))
                        if not 16 < length <= 16384 or version != 1 or kind != 8:
                            raise ValueError("owned request header")
                        request = plistlib.loads(read_exact(connection, length - 16))
                        if request.get("MessageType") != "ListDevices":
                            raise ValueError("owned request operation")
                        payload = plistlib.dumps(dict(DeviceList=entries), fmt=plistlib.FMT_BINARY)
                        connection.sendall(struct.pack("<IIII", 16 + len(payload), 1, 8, tag) + payload)
                        completed.append(True)
            except Exception:
                errors.append(True)

        worker = threading.Thread(target=serve, daemon=True)
        worker.start()
        child_env = (os.environ if env is None else env).copy()
        child_env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + str(listener.getsockname()[1])
        try:
            result = capture([str(binary), "--owned-checked-list"], env=child_env, timeout=8)
        finally:
            worker.join(timeout=4)
        peer_complete = not worker.is_alive() and not errors and completed == [True] * len(replies)
    return assess_checked_list(result, peer_complete)
