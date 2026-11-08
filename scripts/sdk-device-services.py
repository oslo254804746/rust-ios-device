#!/usr/bin/env python3
"""Actual installed Python extension or compiled C DLL service lifecycle gate.

Run with an installed wheel: python -I scripts/sdk-device-services.py.
Run C ABI with an existing DLL: python -I scripts/sdk-device-services.py --ffi-library PATH.
This gate never builds Rust, replaces the native SDK or operates a device.
"""
import argparse
import asyncio
import ctypes as C
import hashlib
import importlib.machinery
import importlib.metadata
import importlib.util
import json
import os
from pathlib import Path
import ssl
import sys
import tempfile
import threading
import time

spec = importlib.util.spec_from_file_location("sdk_service_peer", Path(__file__).with_name("sdk_service_peer.py"))
peer_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(peer_module)
Peer, UDID, BOUND, DATA = peer_module.Peer, peer_module.UDID, peer_module.BOUND, peer_module.DATA
LIVE = set()


def peer(**options):
    value = Peer(**options)
    LIVE.add(value)
    return value


def finished(value, connections=2):
    value.close(expected_connections=connections)
    LIVE.remove(value)


def capture(operation):
    try:
        return operation(), None
    except BaseException as error:
        return None, error


def category(error, expected, partial=None):
    assert error is not None and error.ios_error_class == expected, (error, expected)
    if partial is not None:
        assert getattr(error, "ios_partial_result", None) == partial, (error, getattr(error, "ios_partial_result", None))


class PyBackend:
    def __init__(self):
        import ios_rs
        self.native = ios_rs
        distribution = importlib.metadata.distribution("rust-ios-device-tunnel")
        files = {Path(distribution.locate_file(item)).resolve() for item in distribution.files or ()}
        native = [module for name, module in list(sys.modules.items())
                  if name == "ios_rs" or name.startswith("ios_rs.")]
        assert any(isinstance(getattr(getattr(module, "__spec__", None), "loader", None),
                              importlib.machinery.ExtensionFileLoader)
                   and Path(module.__file__).resolve() in files
                   and getattr(module, "DeviceSession", None) is ios_rs.DeviceSession for module in native)

    def create(self, pair, endpoint, concurrency=1):
        return self.native.DeviceSession(UDID, pair, endpoint, concurrency=concurrency)

    def token(self):
        return self.native.DeviceCancel()

    def call(self, session, kind, token=None, **options):
        return session.service(kind, cancel=token, **options)

    def query(self, session):
        return session.query("DeviceName")

    def close(self, session):
        session.close()

    def destroy(self, session, token=None):
        session.close()


class CFailure(RuntimeError):
    def __init__(self, category, partial=None):
        super().__init__("native C service returned nonzero status: " + category)
        self.ios_error_class = category
        if partial is not None:
            self.ios_partial_result = partial


class Config(C.Structure):
    _fields_ = [("udid", C.c_char_p), ("udid_len", C.c_size_t),
                ("endpoint", C.c_char_p), ("endpoint_len", C.c_size_t),
                ("pair_record", C.c_void_p), ("pair_record_len", C.c_size_t),
                ("concurrent_operations", C.c_uint32)]


class ServiceOptions(C.Structure):
    _fields_ = [("total_ms", C.c_uint64)] + [(name, C.c_size_t) for name in
        ("reply_wire_bytes", "output_bytes", "max_entries", "max_frame_bytes", "json_bytes")]


class QueryOptions(C.Structure):
    _fields_ = [("total_ms", C.c_uint64)] + [(name, C.c_size_t) for name in
        ("reply_wire_bytes", "output_utf8_bytes", "mux_frame_bytes", "lockdown_body_bytes",
         "metadata_entries", "metadata_utf8_bytes", "json_bytes")]


class CBackend:
    def __init__(self, path):
        self.native = C.CDLL(str(path.resolve()))
        definitions = {
            "ios_device_session_create": ([C.POINTER(Config), C.POINTER(C.c_void_p)], C.c_int),
            "ios_device_session_close": ([C.c_void_p, C.c_uint64], C.c_int),
            "ios_device_session_destroy_checked": ([C.POINTER(C.c_void_p)], C.c_int),
            "ios_device_handle_from_session": ([C.c_void_p, C.POINTER(C.c_void_p)], C.c_int),
            "ios_device_close_checked": ([C.POINTER(C.c_void_p)], C.c_int),
            "ios_device_services_close": ([C.c_void_p, C.c_uint64], C.c_int),
            "ios_device_list_apps_json": ([C.c_void_p, C.POINTER(C.c_void_p)], C.c_int),
            "ios_device_afc_list_files_json": ([C.c_void_p, C.c_char_p, C.POINTER(C.c_void_p)], C.c_int),
            "ios_device_afc_read_file": ([C.c_void_p, C.c_char_p, C.c_size_t, C.POINTER(C.c_void_p), C.POINTER(C.c_size_t)], C.c_int),
            "ios_device_syslog_read_json": ([C.c_void_p, C.c_size_t, C.c_uint64, C.POINTER(C.c_void_p)], C.c_int),
            "ios_device_service_options_default": ([C.POINTER(ServiceOptions)], C.c_int),
            "ios_device_query_options_default": ([C.POINTER(QueryOptions)], C.c_int),
            "ios_device_session_service_json": ([C.c_void_p, C.c_uint32, C.c_char_p, C.c_size_t,
                C.c_size_t, C.POINTER(ServiceOptions), C.c_void_p, C.POINTER(C.c_void_p)], C.c_int),
            "ios_device_session_afc_read_file": ([C.c_void_p, C.c_char_p, C.c_size_t, C.c_size_t,
                C.POINTER(ServiceOptions), C.c_void_p, C.POINTER(C.c_void_p), C.POINTER(C.c_size_t)], C.c_int),
            "ios_device_session_query_json": ([C.c_void_p, C.c_uint32, C.POINTER(QueryOptions), C.c_void_p,
                C.POINTER(C.c_void_p)], C.c_int),
            "ios_operation_cancel_create": ([C.POINTER(C.c_void_p)], C.c_int),
            "ios_operation_cancel": ([C.c_void_p], C.c_int),
            "ios_operation_cancel_destroy_checked": ([C.POINTER(C.c_void_p)], C.c_int),
            "ios_free_string_checked": ([C.POINTER(C.c_void_p)], C.c_int),
            "ios_free_bytes_checked": ([C.POINTER(C.c_void_p), C.POINTER(C.c_size_t)], C.c_int),
            "ios_last_error_class": ([], C.c_char_p),
            "ios_last_service_projection_error_class": ([], C.c_char_p),
        }
        for name, (arguments, result) in definitions.items():
            function = getattr(self.native, name)
            function.argtypes, function.restype = arguments, result

    def checked(self, status, partial=None):
        # Read the typed native TLS category on the original calling OS thread,
        # before any cleanup/status function clears it.
        if status != 0:
            error = CFailure(self.native.ios_last_error_class().decode(), partial)
            error.ios_secondary_error_class = self.native.ios_last_service_projection_error_class().decode()
            raise error

    def create(self, pair, endpoint, concurrency=1):
        udid, endpoint = UDID.encode(), endpoint.encode()
        buffer = C.create_string_buffer(pair)
        config = Config(udid, len(udid), endpoint, len(endpoint), C.cast(buffer, C.c_void_p), len(pair), concurrency)
        output = C.c_void_p()
        self.checked(self.native.ios_device_session_create(C.byref(config), C.byref(output)))
        assert output.value
        return output

    class Token:
        def __init__(self, backend):
            self.backend, self.pointer = backend, C.c_void_p()
            backend.checked(backend.native.ios_operation_cancel_create(C.byref(self.pointer)))
        def cancel(self):
            self.backend.checked(self.backend.native.ios_operation_cancel(self.pointer))
        def destroy(self):
            self.backend.checked(self.backend.native.ios_operation_cancel_destroy_checked(C.byref(self.pointer)))
            assert self.pointer.value is None

    def token(self):
        return self.Token(self)

    def call(self, session, kind, token=None, **arguments):
        own_token = token is None
        token = token or self.token()
        options = ServiceOptions()
        self.checked(self.native.ios_device_service_options_default(C.byref(options)))
        options.total_ms = int(arguments.pop("timeout", 15.0) * 1000)
        path = arguments.pop("path", "/").encode()
        count = arguments.pop("max_messages", 100)
        maximum = arguments.pop("max_bytes", 1048576)
        for name, value in arguments.items():
            setattr(options, name, value)
        output, length = C.c_void_p(), C.c_size_t()
        try:
            if kind == "read_file":
                status = self.native.ios_device_session_afc_read_file(session, path, len(path), maximum,
                    C.byref(options), token.pointer, C.byref(output), C.byref(length))
                error_class = self.native.ios_last_error_class().decode()
                secondary_class = self.native.ios_last_service_projection_error_class().decode()
                value = C.string_at(output, length.value) if output.value else b""
                assert status == 0 or (output.value is None and length.value == 0)
                assert self.native.ios_free_bytes_checked(C.byref(output), C.byref(length)) == 0
                assert output.value is None and length.value == 0
            else:
                status = self.native.ios_device_session_service_json(session,
                    {"apps": 1, "files": 2, "logs": 3}[kind], path, len(path), count,
                    C.byref(options), token.pointer, C.byref(output))
                error_class = self.native.ios_last_error_class().decode()
                secondary_class = self.native.ios_last_service_projection_error_class().decode()
                value = json.loads(C.string_at(output).decode()) if output.value else None
                assert self.native.ios_free_string_checked(C.byref(output)) == 0
                assert output.value is None
            if status != 0:
                error = CFailure(error_class, value)
                error.ios_secondary_error_class = secondary_class
                raise error
            assert error_class == "none"
            return value
        finally:
            if own_token:
                token.destroy()

    def query(self, session):
        options, token, output = QueryOptions(), self.token(), C.c_void_p()
        self.checked(self.native.ios_device_query_options_default(C.byref(options)))
        try:
            self.checked(self.native.ios_device_session_query_json(session, 4, C.byref(options), token.pointer, C.byref(output)))
        finally:
            self.native.ios_free_string_checked(C.byref(output))
            token.destroy()

    def close(self, session):
        self.checked(self.native.ios_device_session_close(session, 5000))

    def legacy_handle(self, session):
        output = C.c_void_p()
        self.checked(self.native.ios_device_handle_from_session(session, C.byref(output)))
        return output

    def legacy_call(self, handle, kind, *, path="/owned", max_bytes=1048576, max_messages=100, timeout=5.0):
        output, length = C.c_void_p(), C.c_size_t()
        if kind == "read_file":
            status = self.native.ios_device_afc_read_file(handle, path.encode(), max_bytes, C.byref(output), C.byref(length))
            error_class = self.native.ios_last_error_class().decode()
            value = C.string_at(output, length.value) if output.value else b""
            assert status == 0 or (output.value is None and length.value == 0)
            assert self.native.ios_free_bytes_checked(C.byref(output), C.byref(length)) == 0
        else:
            if kind == "apps":
                status = self.native.ios_device_list_apps_json(handle, C.byref(output))
            elif kind == "files":
                status = self.native.ios_device_afc_list_files_json(handle, path.encode(), C.byref(output))
            else:
                status = self.native.ios_device_syslog_read_json(handle, max_messages, int(timeout * 1000), C.byref(output))
            error_class = self.native.ios_last_error_class().decode()
            value = json.loads(C.string_at(output).decode()) if output.value else None
            assert self.native.ios_free_string_checked(C.byref(output)) == 0
        if status:
            raise CFailure(error_class, value)
        assert error_class == "none"
        return value

    def destroy(self, session, token=None):
        self.close(session)
        self.checked(self.native.ios_device_session_destroy_checked(C.byref(session)))
        self.checked(self.native.ios_device_session_destroy_checked(C.byref(session)))
        assert session.value is None
        if token:
            token.destroy()


def synchronous(backend, pair, context):
    cases = []
    for kind, expected, options in (
        ("apps", "org.fixture.real", {}),
        ("files", ["owned-a", "owned-b"], {"path": "/owned"}),
        ("read_file", DATA, {"path": "/owned", "max_bytes": len(DATA)}),
        ("read_file", b"", {"path": "/owned", "max_bytes": 0}),
    ):
        actual_kind = "read" if kind == "read_file" else kind
        current = peer(kind=actual_kind, data=b"" if expected == b"" else DATA)
        session = backend.create(pair, current.endpoint)
        value = backend.call(session, kind, **options)
        backend.destroy(session)
        finished(current)
        if kind == "apps":
            assert value[0]["bundle_id"] == expected and value[0]["display_name"] == "Owned app"
        else:
            assert value == expected
        if kind == "read_file":
            assert hashlib.sha256(value).digest() == hashlib.sha256(expected).digest()
        cases.append("success:" + kind + (":empty" if expected == b"" else ""))

    current = peer(kind="apps", context=context, tls=True)
    session = backend.create(pair, current.endpoint)
    assert backend.call(session, "apps")[0]["bundle_id"] == "org.fixture.real"
    backend.destroy(session)
    finished(current)
    cases.append("service-TLS")

    for kind, options in (("apps", {"max_entries": 0}), ("files", {"max_entries": 1}),
                          ("apps", {"output_bytes": 1}), ("files", {"output_bytes": 1}),
                          ("read_file", {"max_bytes": len(DATA) - 1})):
        current = peer(kind="read" if kind == "read_file" else kind)
        session = backend.create(pair, current.endpoint)
        _, error = capture(lambda: backend.call(session, kind, path="/owned", **options))
        category(error, "resource_limit")
        backend.destroy(session)
        finished(current)
        cases.append("budget:" + kind + ":" + next(iter(options)))

    for kind in ("apps", "files"):
        current = peer(kind=kind, huge=True)
        session = backend.create(pair, current.endpoint)
        _, error = capture(lambda: backend.call(session, kind, path="/owned"))
        category(error, "protocol" if kind == "apps" else "resource_limit")
        backend.destroy(session)
        finished(current)
        cases.append("declared-frame-refusal:" + kind)

    for stage in ("mux", "lockdown", "service_connect", "service"):
        current = peer(kind="apps", stall=stage)
        session = backend.create(pair, current.endpoint)
        started = time.monotonic()
        _, error = capture(lambda: backend.call(session, "apps", timeout=0.3))
        elapsed = time.monotonic() - started
        category(error, "timeout")
        assert 0.25 <= elapsed < 2.0, elapsed
        backend.destroy(session)
        finished(current, 2 if stage in ("service_connect", "service") else 1)
        cases.append("whole-total:" + stage)

    for stage in ("mux", "lockdown", "service_connect", "service"):
        current = peer(kind="apps", stall=stage)
        session, token, outcomes = backend.create(pair, current.endpoint), backend.token(), []
        worker = threading.Thread(target=lambda: outcomes.append(capture(lambda: backend.call(session, "apps", token=token))), name="owned-native-SDK-call")
        worker.start()
        assert current.ready.wait(BOUND)
        _, conflict = capture(lambda: backend.query(session))
        category(conflict, "conflict")
        started = time.monotonic()
        token.cancel()
        worker.join(BOUND)
        assert not worker.is_alive() and time.monotonic() - started < 2.0
        category(outcomes[0][1], "cancelled")
        backend.destroy(session, token)
        finished(current, 2 if stage in ("service_connect", "service") else 1)
        cases.append("cancel-and-query-conflict:" + stage)

    current = peer(kind="apps", stall="service")
    session, outcomes = backend.create(pair, current.endpoint), []
    worker = threading.Thread(target=lambda: outcomes.append(capture(lambda: backend.call(session, "apps"))), name="owned-native-SDK-close-call")
    worker.start()
    assert current.ready.wait(BOUND)
    backend.close(session)
    worker.join(BOUND)
    assert not worker.is_alive()
    category(outcomes[0][1], "cancelled")
    _, error = capture(lambda: backend.call(session, "logs", max_messages=0))
    category(error, "precondition")
    backend.destroy(session)
    finished(current)
    cases.append("close-active-join-and-refuse")

    for finish in (False, True):
        current = peer(kind="logs", finish=finish)
        session = backend.create(pair, current.endpoint)
        _, error = capture(lambda: backend.call(session, "logs", max_messages=2, timeout=0.3))
        # A02 deliberately leaves generic UnexpectedEof as io; it does not
        # infer a device disconnect from an error also used for truncated files.
        category(error, "io" if finish else "timeout", ["owned log one"])
        backend.destroy(session)
        finished(current)
        cases.append("partial-syslog:" + ("EOF" if finish else "timeout"))

    if isinstance(backend, CBackend):
        current = peer(kind="logs", finish=False)
        session = backend.create(pair, current.endpoint)
        _, error = capture(lambda: backend.call(session, "logs", timeout=0.3,
                                                max_messages=2, json_bytes=1))
        category(error, "timeout")
        assert not hasattr(error, "ios_partial_result")
        assert error.ios_secondary_error_class == "resource_limit", error.ios_secondary_error_class
        backend.destroy(session)
        finished(current)
        cases.append("partial-projection-keeps-primary-and-secondary")

    # Cancel-before-start and zero count keep the real listener untouched.
    current = peer()
    session, token = backend.create(pair, current.endpoint), backend.token()
    assert backend.call(session, "logs", max_messages=0) == []
    token.cancel()
    _, error = capture(lambda: backend.call(session, "apps", token=token))
    category(error, "cancelled")
    backend.destroy(session, token)
    finished(current, 0)
    cases.append("zero-count-and-cancel-before-IO")
    return cases


def c_legacy(backend, pair):
    cases = []
    for kind in ("apps", "files", "read_file"):
        current = peer(kind="read" if kind == "read_file" else kind)
        session = backend.create(pair, current.endpoint)
        handle = backend.legacy_handle(session)
        value = backend.legacy_call(handle, kind, max_bytes=len(DATA))
        if kind == "apps":
            assert value[0]["bundle_id"] == "org.fixture.real"
        elif kind == "files":
            assert value == ["owned-a", "owned-b"]
        else:
            assert value == DATA
        backend.checked(backend.native.ios_device_close_checked(C.byref(handle)))
        backend.destroy(session)
        finished(current)
        cases.append("legacy-actual-binding:" + kind)

    current = peer(kind="logs")
    session = backend.create(pair, current.endpoint)
    handle = backend.legacy_handle(session)
    _, error = capture(lambda: backend.legacy_call(handle, "logs", max_messages=2, timeout=0.3))
    category(error, "timeout", ["owned log one"])
    backend.checked(backend.native.ios_device_close_checked(C.byref(handle)))
    backend.destroy(session)
    finished(current)
    cases.append("legacy-actual-log-partial-timeout")

    current = peer(kind="apps", stall="service")
    session = backend.create(pair, current.endpoint)
    handle, outcomes = backend.legacy_handle(session), []
    worker = threading.Thread(target=lambda: outcomes.append(capture(lambda: backend.legacy_call(handle, "apps"))), name="owned-legacy-C-service")
    worker.start()
    assert current.ready.wait(BOUND)
    _, conflict = capture(lambda: backend.query(session))
    category(conflict, "conflict")
    backend.checked(backend.native.ios_device_services_close(handle, 5000))
    worker.join(BOUND)
    assert not worker.is_alive()
    category(outcomes[0][1], "cancelled")
    _, error = capture(lambda: backend.legacy_call(handle, "logs", max_messages=0))
    category(error, "precondition")
    backend.checked(backend.native.ios_device_close_checked(C.byref(handle)))
    backend.destroy(session)
    finished(current)
    cases.append("legacy-shared-owner-close-cancel-and-query-conflict")
    return cases


async def python_async(backend, pair):
    cases = []
    for stage in ("mux", "lockdown", "service_connect", "service"):
        current = peer(kind="apps", stall=stage)
        pool = backend.native.DeviceAdmission(1)
        session = backend.native.DeviceSession(UDID, pair, current.endpoint, concurrency=1, admission=pool)
        future = session.service_async("apps")
        until = asyncio.get_running_loop().time() + BOUND
        while not current.ready.is_set():
            assert asyncio.get_running_loop().time() < until
            await asyncio.sleep(0.005)
        assert future.cancel()
        try:
            await future
            raise AssertionError("native service cancellation must fail")
        except asyncio.CancelledError as error:
            category(error, "cancelled")
            assert error.ios_error_phase == "device service" and error.ios_cleanup_complete
            assert error.query_receipt.done
        await session.aclose()
        await asyncio.sleep(0.01)
        assert pool.available == pool.job_available == 1 and pool.observer_available == 2
        finished(current, 2 if stage in ("service_connect", "service") else 1)
        cases.append("async-native-cancel-and-join:" + stage)

    current = peer(kind="logs")
    session = backend.create(pair, current.endpoint)
    future = session.service_async("logs", max_messages=2)
    until = asyncio.get_running_loop().time() + BOUND
    while not current.ready.is_set():
        assert asyncio.get_running_loop().time() < until
        await asyncio.sleep(0.005)
    # Ensure the completed message has been read before cancellation.
    await asyncio.sleep(0.02)
    future.cancel()
    try:
        await future
    except asyncio.CancelledError as error:
        category(error, "cancelled", ["owned log one"])
        assert error.query_receipt.done
    else:
        raise AssertionError("cancelled log service succeeded")
    await session.aclose()
    finished(current)
    cases.append("async-partial-log-cancel")

    if os.name == "nt":
        # Public convenience Device uses an isolated local trust file. Never
        # touch the user's actual platform pairing store.
        with tempfile.TemporaryDirectory(prefix="ios-owned-SDK-trust-") as directory:
            trust = Path(directory) / "Apple" / "Lockdown"
            trust.mkdir(parents=True)
            (trust / (UDID + ".plist")).write_bytes(pair)
            current = peer(kind="logs", stall="service")
            old_profile, old_mux = os.environ.get("ALLUSERSPROFILE"), os.environ.get("USBMUXD_SOCKET_ADDRESS")
            os.environ["ALLUSERSPROFILE"] = directory
            os.environ["USBMUXD_SOCKET_ADDRESS"] = current.endpoint
            try:
                device = backend.native.open_device(UDID)
                assert not device.closed
                future = device.async_read_syslog(2, 15000)
                until = asyncio.get_running_loop().time() + BOUND
                while not current.ready.is_set():
                    assert asyncio.get_running_loop().time() < until
                    await asyncio.sleep(0.005)
                await device.aclose()
                assert device.closed
                _, error = await asyncio_result(future)
                category(error, "cancelled")
                await device.aclose()
                device.close()
                finished(current)
                cases.append("installed-public-Device-aclose-active-and-repeat")
            finally:
                for name, original in (("ALLUSERSPROFILE", old_profile), ("USBMUXD_SOCKET_ADDRESS", old_mux)):
                    if original is None:
                        os.environ.pop(name, None)
                    else:
                        os.environ[name] = original
    else:
        cases.append("NOT_RUN:public-Device-local-trust-platform-fixture-Windows-only")
    return cases


async def asyncio_result(future):
    try:
        return await future, None
    except BaseException as error:
        return None, error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ffi-library", type=Path)
    args = parser.parse_args()
    assert __debug__, "assertions must be enabled"
    backend = CBackend(args.ffi_library) if args.ffi_library else PyBackend()
    cert, key = peer_module.assets(Path(__file__).resolve().parents[1] / "crates/ios-core/src/lockdown/checked_session/test_assets.rs")
    pair = peer_module.pair_bytes(cert, key)
    with tempfile.TemporaryDirectory(prefix="ios-owned-SDK-services-") as temporary:
        directory = Path(temporary)
        certificate, private_key = directory / "cert.pem", directory / "key.pem"
        certificate.write_bytes(cert)
        private_key.write_bytes(key)
        private_key.chmod(0o600)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(str(certificate), str(private_key))
        cases = synchronous(backend, pair, context)
        if isinstance(backend, PyBackend):
            cases.extend(asyncio.run(python_async(backend, pair)))
        else:
            cases.extend(c_legacy(backend, pair))
    print(json.dumps({"backend": "C-ABI" if args.ffi_library else "installed-Python", "cases": cases,
                      "physical_devices": "NOT_RUN", "other_platforms": "NOT_RUN"}))


if __name__ == "__main__":
    try:
        main()
    finally:
        for current in list(LIVE):
            current.abort()
