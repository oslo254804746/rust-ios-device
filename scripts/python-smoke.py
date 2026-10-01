"""Installed API and error contract; owned ListDevices peer, no device operations."""
import inspect
import os
from pathlib import Path
import plistlib
import socket
import struct
import subprocess
import sys
import threading

import ios_rs


SECRET = "synthetic-python-credential-UDID"


def error_contract(error, *, kind, error_class, phase, effects):
    assert type(error) is kind, "exception type changed"
    assert type(error.ios_error_schema_version) is int and error.ios_error_schema_version == 1
    assert error.ios_error_class == error_class
    assert error.ios_error_phase == phase
    assert error.ios_error_effects == effects
    assert error.__cause__ is None, "native source must not become an unsafe Python cause"
    assert len(error.args) == 1 and isinstance(error.args[0], str)
    assert SECRET not in str(error) and SECRET not in repr(error), "unsafe Python exception"


def error_child(case):
    if case == "invalid-mode":
        try:
            ios_rs.start_tunnel(SECRET, mode=SECRET)
        except ValueError as error:
            error_contract(error, kind=ValueError, error_class="invalid_input",
                           phase="tunnel mode", effects="not_started")
            assert error.args == ('invalid tunnel mode; expected "userspace" or "kernel"',)
        else:
            raise AssertionError("invalid mode must fail before device access")
    elif case == "list-error":
        try:
            ios_rs.list_devices()
        except RuntimeError as error:
            error_contract(error, kind=RuntimeError, error_class="protocol",
                           phase="list devices", effects="unknown")
            assert error.args[0].startswith("list devices failed: ")
        else:
            raise AssertionError("malformed owned reply must fail")
    else:
        raise AssertionError("unknown smoke case")
    print("Python error contract passed")


def run_error_child(case, listener):
    env = os.environ.copy()
    env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + str(listener.getsockname()[1])
    result = subprocess.run([sys.executable, "-I", str(Path(__file__).resolve()), case],
                            env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=8)
    assert SECRET.encode() not in result.stdout + result.stderr, "unsafe child diagnostic"
    assert result.returncode == 0, "installed Python error contract failed"
    assert result.stdout.replace(b"\r\n", b"\n") == b"Python error contract passed\n"
    assert not result.stderr, "unexpected Python diagnostic"


def read_exact(connection, size):
    data = bytearray()
    while len(data) < size:
        chunk = connection.recv(size - len(data))
        if not chunk:
            raise ValueError("owned fixture EOF")
        data.extend(chunk)
    return bytes(data)


def installed_error_contract():
    with socket.socket() as trap:
        trap.bind(("127.0.0.1", 0))
        trap.listen(1)
        run_error_child("invalid-mode", trap)
        trap.setblocking(False)
        try:
            connection, _ = trap.accept()
        except BlockingIOError:
            pass
        else:
            connection.close()
            raise AssertionError("invalid mode contacted the owned endpoint")

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
                    payload = SECRET.encode()  # Intentionally not a plist; never routes a device.
                    connection.sendall(struct.pack("<IIII", 16 + len(payload), 1, 8, tag) + payload)
                    completed.append(True)
            except Exception:
                errors.append(True)

        worker = threading.Thread(target=serve, daemon=True)
        worker.start()
        try:
            run_error_child("list-error", listener)
        finally:
            worker.join(timeout=4)
        assert not worker.is_alive() and not errors and completed == [True], "owned peer failed"


if len(sys.argv) == 2:
    error_child(sys.argv[1])
    raise SystemExit(0)

assert callable(ios_rs.list_devices)
assert callable(ios_rs.start_tunnel)
signature = inspect.signature(ios_rs.start_tunnel)
assert list(signature.parameters) == ["udid", "mode"]
assert signature.parameters["udid"].default is inspect.Parameter.empty
assert signature.parameters["mode"].default == "userspace"
assert not inspect.signature(ios_rs.list_devices).parameters
for name in ("server_address", "rsd_port", "userspace_port", "services", "service_ports",
             "service_features", "connect_info", "asyncio_proxy", "close"):
    assert hasattr(ios_rs.Tunnel, name), name
for name in ("__enter__", "__exit__", "install", "restore"):
    assert callable(getattr(ios_rs.AsyncioProxyPatch, name)), name
for cls in (ios_rs.Tunnel, ios_rs.AsyncioProxyPatch):
    try:
        cls()
    except TypeError:
        pass
    else:
        raise AssertionError("resource classes must be created by their documented factory")
assert list(inspect.signature(ios_rs.Tunnel.asyncio_proxy).parameters) == ["self"]
installed_error_contract()
print("Installed Python API smoke passed")
