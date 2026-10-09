#!/usr/bin/env python3
"""Installed-extension, owned usbmux/Lockdown peers; no hardware certification.

Run with an already installed wheel: python -I scripts/python-device-query.py.
This script never builds Rust or provides a replacement ios_rs implementation.
"""
import argparse
import ast
import asyncio
import gc
import importlib.machinery
import importlib.metadata
from pathlib import Path
import plistlib
import re
import socket
import ssl
import struct
import sys
import tempfile
import threading
import time
import weakref

import ios_rs  # Missing/uninstalled native bindings must fail, never skip.

UDID = "owned-private-python-device-query"
HOST = "owned-private-python-query-host"
BUID = "owned-private-python-query-buid"
SESSION = "owned-private-python-query-session"
BOUND = 5.0
ABSENT = object()
LIVE_PEERS = set()


def installed_extension():
    distribution = importlib.metadata.distribution("rust-ios-device-tunnel")
    files = {Path(distribution.locate_file(item)).resolve()
             for item in distribution.files or ()}
    native = [module for name, module in list(sys.modules.items())
              if name == "ios_rs" or name.startswith("ios_rs.")]
    native = [module for module in native
              if isinstance(getattr(getattr(module, "__spec__", None), "loader", None),
                            importlib.machinery.ExtensionFileLoader)
              and getattr(module, "DeviceSession", None) is ios_rs.DeviceSession]
    assert native and all(Path(module.__file__).resolve() in files for module in native), \
        "the installed distribution must supply the actual native extension"
    assert __debug__, "this validator requires assertions enabled"


def assets(path):
    text = path.read_text(encoding="utf-8")
    values = []
    for name in ("TEST_CERT_PEM", "TEST_KEY_PEM"):
        match = re.search(r"const " + name + r': &\[u8\] = b"(.*?)";', text, re.S)
        if match is None:
            raise ValueError("the existing synthetic TLS fixture constant is missing")
        values.append(ast.literal_eval('b"' + match.group(1).replace("\n", r"\n") + '"'))
    return values


def pair_bytes(cert, key):
    return plistlib.dumps({"DeviceCertificate": b"", "HostCertificate": cert,
                          "HostPrivateKey": key, "RootCertificate": b"",
                          "HostID": HOST, "SystemBUID": BUID})


def encoded(reply, stage):
    body = plistlib.dumps(reply, fmt=plistlib.FMT_XML)
    if stage < 2:
        return struct.pack("<IIII", len(body) + 16, 1, 8, stage + 1) + body
    return struct.pack(">I", len(body)) + body


class Eof(Exception):
    pass


def read_exact(connection, size):
    chunks = bytearray()
    while len(chunks) < size:
        chunk = connection.recv(size - len(chunks))
        if not chunk:
            raise Eof()
        chunks.extend(chunk)
    return bytes(chunks)


def request(connection, stage):
    if stage < 2:
        length, version, kind, tag = struct.unpack("<IIII", read_exact(connection, 16))
        if not 16 <= length <= 16384 or (version, kind, tag) != (1, 8, stage + 1):
            raise ValueError("owned mux request framing failed")
        return plistlib.loads(read_exact(connection, length - 16)), tag
    length = struct.unpack(">I", read_exact(connection, 4))[0]
    if not 0 < length <= 16384:
        raise ValueError("owned Lockdown request framing failed")
    return plistlib.loads(read_exact(connection, length)), None


class Script:
    def __init__(self, value="owned value", *, device_id=7, product_id=ABSENT,
                 tls=False, wait=False, error=None, raw=None, metadata=None,
                 stop_stage=4, header_only=None):
        properties = {"SerialNumber": UDID, "ConnectionType": "USB"}
        if product_id is not ABSENT:
            properties["ProductID"] = product_id
        reply = {"Request": "GetValue"}
        if value is not ABSENT:
            reply["Value"] = value
        if error is not None:
            reply["Error"] = error
        self.replies = [metadata if metadata is not None else
                        {"DeviceList": [{"DeviceID": device_id, "Properties": properties}]},
                        {"MessageType": "Result", "Number": 0},
                        {"Request": "QueryType", "Type": "com.apple.mobile.lockdown"},
                        {"Request": "StartSession", "SessionID": SESSION,
                         "EnableSessionSSL": tls}, reply]
        self.frames = [encoded(reply, stage) for stage, reply in enumerate(self.replies)]
        if raw is not None:
            self.frames[4] = raw
        self.tls, self.wait = tls, wait
        self.stop_stage, self.header_only = stop_stage, header_only
        self.ready = threading.Event()
        self.requests, self.tags = [], []
        self.eof, self.tail = False, b""


class Peer:
    """An owned listener plus bounded original worker threads and strict TCP EOF."""
    def __init__(self, scripts, tls_context, *, unix_path=None):
        self.scripts = scripts
        self.context = tls_context
        self.listener = socket.socket(socket.AF_UNIX if unix_path else socket.AF_INET,
                                      socket.SOCK_STREAM)
        if unix_path:
            self.listener.bind(str(unix_path))
            self.endpoint = "unix://" + str(unix_path)
        else:
            self.listener.bind(("127.0.0.1", 0))
            self.endpoint = "tcp://127.0.0.1:" + str(self.listener.getsockname()[1])
        self.listener.listen(4)
        self.listener.settimeout(0.05)
        self.stop = threading.Event()
        self.errors, self.workers, self.connections = [], [], []
        self.accepted = 0
        self.worker = threading.Thread(target=self.accept, name="owned-query-listener")
        self.worker.start()
        LIVE_PEERS.add(self)

    def abort(self):
        # Failure-only owned cleanup; these forced closes are never EOF evidence.
        self.stop.set()
        for connection in self.connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.worker.join(BOUND)
        for worker in self.workers:
            worker.join(BOUND)
        self.listener.close()
        LIVE_PEERS.discard(self)

    def accept(self):
        try:
            while not self.stop.is_set():
                try:
                    connection, _ = self.listener.accept()
                except socket.timeout:
                    continue
                if self.accepted >= len(self.scripts):
                    connection.close()
                    raise ValueError("unexpected extra native transport")
                script = self.scripts[self.accepted]
                self.accepted += 1
                connection.settimeout(BOUND)
                self.connections.append(connection)
                worker = threading.Thread(target=self.serve, args=(connection, script),
                                          name="owned-query-peer")
                self.workers.append(worker)
                worker.start()
        except Exception as error:
            self.errors.append(type(error).__name__)

    def serve(self, connection, script):
        current = connection
        try:
            for stage in range(5):
                if stage == 4 and script.tls:
                    current = self.context.wrap_socket(current, server_side=True,
                                                       suppress_ragged_eofs=False)
                    current.settimeout(BOUND)
                    self.connections.append(current)
                incoming, tag = request(current, stage)
                script.requests.append(incoming)
                if tag is not None:
                    script.tags.append(tag)
                if script.wait and stage == script.stop_stage:
                    script.ready.set()
                    break
                frame = script.frames[stage]
                if script.header_only == stage:
                    current.sendall(frame[:16 if stage < 2 else 4])
                    script.ready.set()
                    break
                current.sendall(frame)
                if stage == 4:
                    script.ready.set()
            # We certify the original TCP EOF, including TLS's underlying stream.
            # No server close or TLS close_notify is used to manufacture this EOF.
            if isinstance(current, ssl.SSLSocket):
                current = socket.socket(fileno=current.detach())
                current.settimeout(BOUND)
                self.connections.append(current)
            while True:
                chunk = current.recv(4096)
                if not chunk:
                    script.eof = True
                    break
                script.tail += chunk
                if len(script.tail) > 16384:
                    raise ValueError("unexpected native trailing wire")
        except Eof:
            script.eof = True  # Exact recv(b'') during an incomplete cancel path.
        except Exception as error:
            self.errors.append(type(error).__name__)
        finally:
            current.close()
            script.ready.set()

    def finish(self):
        # Call only after actual native completion/close. Join precedes assertions.
        self.stop.set()
        self.worker.join(BOUND)
        for worker in self.workers:
            worker.join(BOUND)
        alive = self.worker.is_alive() or any(worker.is_alive() for worker in self.workers)
        if alive:
            # An unsuccessful fixture still cleans up only its own sockets/threads.
            for connection in self.connections:
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            for worker in self.workers:
                worker.join(BOUND)
        self.listener.close()
        LIVE_PEERS.discard(self)
        assert not alive and not self.errors, "owned original peer/thread did not finish"
        assert all(script.eof for script in self.scripts[:self.accepted]), "strict native EOF missing"


def capture(function):
    try:
        return function(), None
    except BaseException as error:
        return None, error


async def captured(future):
    try:
        return await future, None
    except BaseException as error:
        return None, error


def settle(session, peer):
    try:
        session.close(cleanup_timeout=BOUND)
        session.close(cleanup_timeout=BOUND)
    finally:
        peer.finish()
    assert session.closed and session.active == 0 and session.pending_cleanup == 0


def error_contract(error, category, *, kind=RuntimeError, effects="unknown"):
    assert type(error) is kind and error.ios_error_schema_version == 1
    assert error.ios_error_class == category and error.ios_error_effects == effects
    assert error.__cause__ is None
    assert len(error.args) == 1 and type(error.args[0]) is str
    text = str(error) + repr(error)
    for secret in (UDID, HOST, BUID, SESSION, "owned-secret-error"):
        assert secret not in text, "unsafe native exception diagnostic"


def wire(script, field, device_id=7):
    assert script.tags == [1, 2] and len(script.requests) == 5
    assert script.tail == b""
    names = ["ListDevices", "Connect", "QueryType", "StartSession", "GetValue"]
    assert [request.get("MessageType" if index < 2 else "Request")
            for index, request in enumerate(script.requests)] == names
    assert script.requests[1]["DeviceID"] == device_id
    assert script.requests[1]["PortNumber"] == int.from_bytes((62078).to_bytes(2, "big"), "little")
    assert script.requests[2] == {"Label": "ios-rs", "Request": "QueryType"}
    assert script.requests[3] == {"Label": "ios-rs", "Request": "StartSession",
                                  "HostID": HOST, "SystemBUID": BUID, "ProtocolVersion": "2"}
    query = {"Label": "ios-rs", "Request": "GetValue"}
    if field != "root":
        query["Key"] = field
    assert script.requests[4] == query


def sync_values(pair, context):
    cases = [("ProductType", "iPhone42,1", False), ("ProductVersion", "26.0", True),
             ("DeviceName", "", False), ("DeviceName", ABSENT, False),
             ("root", {}, False),
             ("root", {"DeviceName": "", "ProductType": "iPhone42,1", "Ignored": 99}, True),
             ("root", ABSENT, False)]
    for field, value, tls in cases:
        script = Script(value, product_id=0, tls=tls)
        peer = Peer([script], context)
        pool = ios_rs.DeviceAdmission(1)
        session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=pool)
        result, error = capture(lambda: session.query(field))
        settle(session, peer)
        assert peer.accepted == 1 and error is None
        wire(script, field)
        assert result["metadata"] == {"device_id": 7, "connection_type": "USB", "product_id": 0}
        assert result["field"] == field and result["enable_session_ssl"] is tls
        assert result["value_present"] is (value is not ABSENT)
        expected = None if value is ABSENT else value
        if field == "root" and value is not ABSENT:
            expected = {name: value.get(name) for name in ("ProductType", "ProductVersion", "DeviceName")}
        assert result["value"] == expected
        assert (pool.available, pool.job_available, pool.observer_available) == (1, 1, 2)
    # The selected DeviceID is refreshed on every attempt on the same Session.
    scripts = [Script("first", device_id=7), Script("second", device_id=29)]
    peer = Peer(scripts, context)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint)
    first, first_error = capture(lambda: session.query("DeviceName"))
    second, second_error = capture(lambda: session.query("DeviceName"))
    settle(session, peer)
    assert first_error is second_error is None and peer.accepted == 2
    assert first["metadata"]["device_id"] == 7 and second["metadata"]["device_id"] == 29
    assert first["metadata"]["product_id"] is second["metadata"]["product_id"] is None
    assert first["value"] == "first"  # Owned result survived reaping and close.
    wire(scripts[0], "DeviceName", 7)
    wire(scripts[1], "DeviceName", 29)


def sync_failures(pair, context):
    cases = [(Script(12), {}, "protocol"), (Script(error="MissingValue"), {}, "not_found"),
             (Script(error="owned-secret-error"), {}, "service_failure"),
             (Script("éé"), {"max_output_utf8_bytes": 5}, "resource_limit"),
             (Script("x", header_only=0), {"max_reply_wire_bytes": 1}, "resource_limit"),
             (Script(raw=struct.pack(">I", 4) + b"bad!"), {}, "protocol")]
    for script, options, category in cases:
        peer = Peer([script], context)
        session = ios_rs.DeviceSession(UDID, pair, peer.endpoint)
        _, error = capture(lambda: session.query("DeviceName", **options))
        settle(session, peer)
        error_contract(error, category)


def invalid_inputs(pair, context):
    peer = Peer([], context)
    results = []
    for bad_pair in (b"", b"x" * (1024 * 1024 + 1)):
        _, error = capture(lambda: ios_rs.DeviceSession(UDID, bad_pair, peer.endpoint))
        results.append((error, "invalid_input"))
    for endpoint in ("tcp://localhost:1234", "tcp://127.0.0.1:0", "file://owned-private", ""):
        _, error = capture(lambda: ios_rs.DeviceSession(UDID, pair, endpoint))
        results.append((error, "invalid_input"))
    for concurrent in (0, 5):
        _, error = capture(lambda: ios_rs.DeviceSession(UDID, pair, peer.endpoint,
                                                       concurrency=concurrent))
        results.append((error, "invalid_input"))
    for limit in (0, 65):
        _, error = capture(lambda: ios_rs.DeviceAdmission(limit))
        results.append((error, "invalid_input"))
    _, malformed_pair = capture(lambda: ios_rs.DeviceSession(UDID, b"owned-private-credential", peer.endpoint))
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint)
    for field, options in [("HostID", {}),
                           ("DeviceName", {"timeout": float("inf")}),
                           ("DeviceName", {"max_reply_wire_bytes": 0}),
                           ("DeviceName", {"max_output_utf8_bytes": 0}),
                           ("DeviceName", {"max_reply_wire_bytes": 46137357}),
                           ("DeviceName", {"max_output_utf8_bytes": 16777217})]:
        _, error = capture(lambda: session.query(field, **options))
        results.append((error, "invalid_input"))
    _, zero_timeout = capture(lambda: session.query("DeviceName", timeout=0))
    token = ios_rs.DeviceCancel()
    token.cancel()
    _, cancelled = capture(lambda: session.query("DeviceName", cancel=token))
    _, no_loop = capture(lambda: session.query_async("DeviceName"))
    conversion_errors = [capture(lambda: ios_rs.DeviceSession(UDID, bytearray(pair), peer.endpoint))[1],
                         capture(lambda: session.query(123))[1]]
    settle(session, peer)
    assert peer.accepted == 0
    for error, category in results:
        error_contract(error, category, kind=ValueError, effects="not_started")
    error_contract(malformed_pair, "protocol", effects="not_started")
    error_contract(zero_timeout, "timeout", effects="not_started")
    error_contract(cancelled, "cancelled", effects="not_started")
    assert type(no_loop) is RuntimeError and "running event loop" in str(no_loop)
    assert all(type(error) is TypeError for error in conversion_errors)


def sync_gil_and_admission(pair, context):
    scripts = [Script(wait=True), Script(wait=True)]
    peer = Peer(scripts, context)
    pool = ios_rs.DeviceAdmission(2)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, concurrency=1, admission=pool)
    other = ios_rs.DeviceSession(UDID, pair, peer.endpoint, concurrency=1, admission=pool)
    tokens = [ios_rs.DeviceCancel(), ios_rs.DeviceCancel()]
    outcomes = []
    workers = []
    for index, current in enumerate((session, other)):
        worker = threading.Thread(target=lambda current=current, index=index: outcomes.append(capture(
            lambda: current.query("DeviceName", cancel=tokens[index]))))
        workers.append(worker)
        worker.start()
        if not scripts[index].ready.wait(BOUND):
            for token in tokens:
                token.cancel()
            for worker in workers:
                worker.join(BOUND)
            other.close()
            settle(session, peer)
            raise AssertionError("sync query did not release the GIL to the owned peer")
        if index == 0:
            _, conflict = capture(lambda: session.query("DeviceName"))
    _, full = capture(lambda: other.query("DeviceName"))
    for token in tokens:
        token.cancel()
    for worker in workers:
        worker.join(BOUND)
    other.close()
    settle(session, peer)
    assert all(not worker.is_alive() for worker in workers)
    assert peer.accepted == 2 and len(outcomes) == 2
    error_contract(conflict, "conflict", effects="not_started")
    error_contract(full, "resource_limit", effects="not_started")
    for _, error in outcomes:
        error_contract(error, "cancelled")
    assert all(token.cancelled for token in tokens) and (pool.available, pool.job_available) == (2, 2)
    for script in scripts:
        wire(script, "DeviceName")


async def wait_ready(script):
    if not await asyncio.to_thread(script.ready.wait, BOUND):
        raise AssertionError("owned peer did not reach its bounded stage")


async def async_cancel(pair, context, mode):
    script = Script(wait=True, stop_stage=4 if mode == "outer" else 0)
    peer = Peer([script], context)
    pool = ios_rs.DeviceAdmission(1)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=pool)
    future = session.query_async("DeviceName")
    immediate_pending = None
    try:
        if mode == "outer":
            async def wrapper():
                return await future
            outer = asyncio.create_task(wrapper())
            await wait_ready(script)
            outer.cancel("owned outer cancellation")
            _, error = await captured(outer)
        else:
            if mode == "ensure":
                future = asyncio.ensure_future(future)
            future.cancel("owned immediate cancellation")
            immediate_pending = not future.done() and not future.cancelled()
            _, error = await captured(future)
    finally:
        settle(session, peer)
    assert isinstance(error, asyncio.CancelledError)
    assert error.ios_error_class == "cancelled" and error.ios_cleanup_complete
    assert error.query_receipt.done and error.query_receipt is future.receipt
    assert not hasattr(future.receipt, "_observer_finished")
    if immediate_pending is not None:
        assert immediate_pending
    assert pool.job_available == pool.available == 1 and pool.observer_available == 2
    if mode == "outer":
        wire(script, "DeviceName")


async def async_drop(pair, context):
    script = Script(wait=True)
    peer = Peer([script], context)
    pool = ios_rs.DeviceAdmission(1)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=pool)
    future = session.query_async("DeviceName")
    await wait_ready(script)
    receipt = future.receipt
    reference = weakref.ref(future)
    del future
    del session  # Last client Drop must seal; observer/receipt owns no client.
    gc.collect()
    _, error = capture(lambda: receipt.result(cleanup_timeout=BOUND))
    await asyncio.sleep(0.02)
    peer.finish()
    assert reference() is None and receipt.done
    error_contract(error, "cancelled")
    assert pool.available == pool.job_available == 1 and pool.observer_available == 2
    wire(script, "DeviceName")


async def async_error_race_and_receipt(pair, context):
    script = Script(error="MissingValue")
    next_script = Script("fresh result")
    peer = Peer([script, next_script], context)
    pool = ios_rs.DeviceAdmission(1)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=pool)
    future = session.query_async("DeviceName")
    # Deliberately keep this loop from polling the Python observer. Native runs
    # on its independent runtime; the receipt is published only after real Join.
    until = time.monotonic() + BOUND
    while not future.receipt.done and time.monotonic() < until:
        time.sleep(0.001)
    future.cancel()
    _, error = await captured(future)
    receipt = future.receipt
    # Reap the completed original job by admitting a second attempt. The first
    # user receipt must retain its original owned NotFound outcome unchanged.
    _, next_error = await captured(session.query_async("ProductType"))
    _, retained = capture(lambda: receipt.result(cleanup_timeout=0.0))
    settle(session, peer)
    error_contract(error, "not_found")
    error_contract(retained, "not_found")
    assert receipt.done and next_error is None
    wire(script, "DeviceName")
    wire(next_script, "ProductType")


async def async_close_limits_and_refusal(pair, context):
    script = Script(wait=True)
    peer = Peer([script], context)
    pool = ios_rs.DeviceAdmission(1)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=pool)
    future = session.query_async("DeviceName")
    await wait_ready(script)
    close = session.aclose()
    _, close_error = await captured(close)
    _, query_error = await captured(future)
    _, repeat_error = await captured(session.aclose())
    settle(session, peer)
    assert close_error is repeat_error is None
    assert isinstance(query_error, RuntimeError) and query_error.ios_error_class == "cancelled"
    assert pool.observer_available == 2
    wire(script, "DeviceName")

    # A task factory refusal happens before native is polled. All real lease,
    # job and observer slots return; the owned trap observes zero source IO.
    peer = Peer([], context)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=pool)
    loop = asyncio.get_running_loop()
    original_factory = loop.get_task_factory()
    def refuse(loop, coroutine):
        raise MemoryError("owned observer allocation refusal")
    loop.set_task_factory(refuse)
    try:
        _, error = capture(lambda: session.query_async("DeviceName"))
    finally:
        loop.set_task_factory(original_factory)
    settle(session, peer)
    assert type(error) is MemoryError and peer.accepted == 0
    assert pool.available == pool.job_available == 1 and pool.observer_available == 2

    peer = Peer([], context)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=pool)
    first, second = session.aclose(), session.aclose()
    _, exhausted = capture(lambda: session.aclose())
    outcomes = [await captured(first), await captured(second)]
    settle(session, peer)
    error_contract(exhausted, "resource_limit", effects="not_started")
    assert all(error is None for _, error in outcomes) and peer.accepted == 0
    assert pool.observer_available == 2

    peer = Peer([], context)
    pools = [ios_rs.DeviceAdmission(1) for _ in range(65)]
    sessions = [ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=item)
                for item in pools]
    observers = [sessions[index // 2].aclose() for index in range(128)]
    _, global_exhausted = capture(lambda: sessions[-1].aclose())
    outcomes = [await captured(observer) for observer in observers]
    for item in sessions:
        item.close()
    peer.finish()
    error_contract(global_exhausted, "resource_limit", effects="not_started")
    assert all(error is None for _, error in outcomes) and peer.accepted == 0
    assert all(item.observer_available == 2 for item in pools)


async def async_registry_and_timeout(pair, context):
    scripts = [Script(wait=True) for _ in range(4)]
    peer = Peer(scripts, context)
    pool = ios_rs.DeviceAdmission(8)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=pool, concurrency=4)
    futures = [session.query_async("DeviceName", cleanup_timeout=0.0) for _ in scripts]
    for script in scripts:
        await wait_ready(script)
    _, full = capture(lambda: session.query_async("DeviceName"))
    _, zero_close = capture(lambda: session.close(cleanup_timeout=0.0))
    for future in futures:
        future.cancel()
    outcomes = [await captured(future) for future in futures]
    retained = [capture(lambda receipt=future.receipt: receipt.result(cleanup_timeout=BOUND))
                for future in futures]
    _, repeat = await captured(session.aclose(cleanup_timeout=BOUND))
    settle(session, peer)
    error_contract(full, "conflict", effects="not_started")
    if zero_close is not None:
        error_contract(zero_close, "timeout")
    assert repeat is None and peer.accepted == 4
    for (_, error), (_, original), future in zip(outcomes, retained, futures):
        # Native cancellation may beat the zero cleanup deadline. Either outcome
        # must report actual completion honestly; deterministic timeout ordering
        # is separately checked by the adapter host test.
        if isinstance(error, TimeoutError):
            assert not error.ios_cleanup_complete and error.query_receipt is future.receipt
        else:
            assert isinstance(error, asyncio.CancelledError) and error.ios_cleanup_complete
        error_contract(original, "cancelled")
        assert future.receipt.done
    assert pool.available == pool.job_available == 8 and pool.observer_available == 16
    for script in scripts:
        wire(script, "DeviceName")


async def async_receipt_timeout(pair, context):
    script = Script(wait=True)
    peer = Peer([script], context)
    pool = ios_rs.DeviceAdmission(1)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint, admission=pool)
    future = session.query_async("DeviceName")
    await wait_ready(script)
    # The real original native task is blocked on this owned peer's GetValue
    # reply. Receipt waiting has its own total and cannot reset on cancellation.
    waiter = future.receipt.wait_async(cleanup_timeout=0.0)
    _, timeout = await captured(waiter)
    _, original = await captured(future)
    _, retained = capture(lambda: timeout.query_receipt.result(cleanup_timeout=BOUND))
    _, repeat = await captured(session.aclose(cleanup_timeout=BOUND))
    settle(session, peer)
    assert type(timeout) is TimeoutError and timeout.ios_error_class == "timeout"
    assert timeout.ios_error_phase == "device query receipt" and not timeout.ios_cleanup_complete
    assert timeout.query_receipt.done and timeout.query_receipt is waiter.receipt
    assert timeout.query_receipt is not future.receipt  # Owned receipts, same original Job.
    error_contract(original, "cancelled")
    error_contract(retained, "cancelled")
    assert repeat is None and pool.available == pool.job_available == 1
    assert pool.observer_available == 2
    wire(script, "DeviceName")


async def async_context(pair, context):
    script = Script("context value")
    peer = Peer([script], context)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint)
    original = ValueError("caller owned body error")
    try:
        async with session as entered:
            result = await entered.query_async("DeviceName")
            raise original
    except ValueError as error:
        observed = error
    finally:
        settle(session, peer)
    assert observed is original and result["value"] == "context value"
    wire(script, "DeviceName")


def unix_query(pair, context, directory):
    if not hasattr(socket, "AF_UNIX") or sys.platform == "win32":
        return
    script = Script("unix value")
    path = directory / "query.sock"
    peer = Peer([script], context, unix_path=path)
    session = ios_rs.DeviceSession(UDID, pair, peer.endpoint)
    result, error = capture(lambda: session.query("DeviceName"))
    settle(session, peer)
    path.unlink()
    assert error is None and result["value"] == "unix value"
    wire(script, "DeviceName")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tls-assets", type=Path, default=Path(__file__).resolve().parents[1]
                        / "crates/ios-core/src/lockdown/checked_session/test_assets.rs")
    args = parser.parse_args()
    installed_extension()
    cert, key = assets(args.tls_assets)
    pair = pair_bytes(cert, key)
    with tempfile.TemporaryDirectory(prefix="ios-python-owned-query-") as temporary:
        directory = Path(temporary)
        cert_path, key_path = directory / "cert.pem", directory / "key.pem"
        cert_path.write_bytes(cert)
        key_path.write_bytes(key)
        key_path.chmod(0o600)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(str(cert_path), str(key_path))
        sync_values(pair, context)
        sync_failures(pair, context)
        invalid_inputs(pair, context)
        sync_gil_and_admission(pair, context)
        unix_query(pair, context, directory)
        async def run():
            for mode in ("immediate", "ensure", "outer"):
                await async_cancel(pair, context, mode)
            await async_drop(pair, context)
            await async_error_race_and_receipt(pair, context)
            await async_close_limits_and_refusal(pair, context)
            await async_registry_and_timeout(pair, context)
            await async_receipt_timeout(pair, context)
            await async_context(pair, context)
        asyncio.run(run())
    print("Installed Python finite device query native fixtures passed")


if __name__ == "__main__":
    try:
        main()
    finally:
        for peer in list(LIVE_PEERS):
            peer.abort()
