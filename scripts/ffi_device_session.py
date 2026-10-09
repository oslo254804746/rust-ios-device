#!/usr/bin/env python3
"""Dedicated finite C query gate over actual native sockets, never a Rust builder.

Root freezes sources/builds serially and supplies matching library build evidence.
This runner installs that exact library, compiles C only, and runs one C process
at a time. Each successful case joins its original C pthread and real peer worker
after strict raw TCP EOF before releasing the C behavior assertions.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import selectors
import shutil
import socket
import ssl
import struct
import subprocess
import sys
import threading
import time

UDID = "owned-ffi-session-device"
HOST_ID = "owned-ffi-host"
BUID = "owned-ffi-buid"
MAX_CAPTURE = 256 * 1024
CASE_SECONDS = 15
SOCKET_SECONDS = 5
VALUES = {"ProductType": "iPhoneFixture", "ProductVersion": "1.0", "DeviceName": "owned C fixture"}
KEYS = {1: "ProductType", 2: "ProductVersion", 4: "DeviceName"}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_fingerprint(repo: Path) -> dict:
    """Hash current on-disk build inputs, including untracked delivery files."""
    paths = {repo / "Cargo.toml", repo / "Cargo.lock"}
    for name in ("ios-core", "ios-ffi"):
        crate = repo / "crates" / name
        paths.update(crate.rglob("*.rs"))
        paths.update(crate.rglob("Cargo.toml"))
    paths.update((repo / "crates/ios-ffi/include/ios_rs.h", repo / "crates/ios-ffi/cbindgen.toml"))
    entries = {str(path.relative_to(repo)): sha256(path) for path in sorted(paths) if path.is_file()}
    encoded = json.dumps(entries, sort_keys=True, separators=(",", ":")).encode()
    return {"algorithm": "sha256(sorted relative-path/file-sha256 JSON)",
            "sha256": hashlib.sha256(encoded).hexdigest(), "files": entries}


def synthetic_pair(repo: Path, install: Path) -> tuple[bytes, ssl.SSLContext]:
    source = (repo / "crates/ios-core/src/lockdown/checked_session/test_assets.rs").read_text()
    pem = {}
    for name in ("TEST_CERT_PEM", "TEST_KEY_PEM"):
        match = re.search(r"const " + name + r': &\[u8\] = b"(.*?)";', source, re.S)
        if match is None:
            raise ValueError("synthetic TLS asset declaration missing")
        pem[name] = match.group(1).encode("ascii")
    cert, key = install / "synthetic-cert.pem", install / "synthetic-key.pem"
    for path, content in ((cert, pem["TEST_CERT_PEM"]), (key, pem["TEST_KEY_PEM"])):
        with path.open("xb") as sink:
            os.chmod(path, 0o600)
            sink.write(content)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.maximum_version = ssl.TLSVersion.TLSv1_2
    context.options |= ssl.OP_NO_TICKET
    if hasattr(context, "num_tickets"):
        context.num_tickets = 0
    context.load_cert_chain(cert, key)
    pair = plistlib.dumps({"DeviceCertificate": b"", "HostCertificate": pem["TEST_CERT_PEM"],
                          "HostPrivateKey": pem["TEST_KEY_PEM"], "RootCertificate": b"",
                          "HostID": HOST_ID, "SystemBUID": BUID}, fmt=plistlib.FMT_XML)
    return pair, context


def receive_exact(stream: socket.socket, size: int) -> bytes:
    if size < 0 or size > 1024 * 1024:
        raise ValueError("fixture frame size outside finite bound")
    result = bytearray()
    while len(result) < size:
        chunk = stream.recv(size - len(result))
        if not chunk:
            raise EOFError("fixture incomplete request")
        result.extend(chunk)
    return bytes(result)


def read_mux(stream: socket.socket, tag: int, message: str) -> dict:
    length, version, kind, actual_tag = struct.unpack("<IIII", receive_exact(stream, 16))
    if length < 16 or version != 1 or kind != 8 or actual_tag != tag:
        raise ValueError("unexpected native mux header")
    value = plistlib.loads(receive_exact(stream, length - 16))
    if not isinstance(value, dict) or value.get("MessageType") != message:
        raise ValueError("unexpected native mux request")
    return value


def write_mux(stream: socket.socket, tag: int, value: dict) -> None:
    body = plistlib.dumps(value, fmt=plistlib.FMT_XML)
    stream.sendall(struct.pack("<IIII", 16 + len(body), 1, 8, tag) + body)


def read_lockdown(stream: socket.socket, request: str) -> dict:
    length, = struct.unpack(">I", receive_exact(stream, 4))
    value = plistlib.loads(receive_exact(stream, length))
    if not isinstance(value, dict) or value.get("Request") != request:
        raise ValueError("unexpected native lockdown request")
    return value


def write_lockdown(stream: socket.socket, value: dict, binary: bool = False) -> None:
    body = plistlib.dumps(value, fmt=plistlib.FMT_BINARY if binary else plistlib.FMT_XML)
    stream.sendall(struct.pack(">I", len(body)) + body)


def cases() -> list[dict]:
    result = []
    for name in ("plain", "tls", "fresh", "root-missing", "root-empty", "single-empty", "single-type", "single-version", "filtered", "binary-nul"):
        fields = {"single-type": 1, "single-version": 2, "single-empty": 4, "filtered": 3}.get(name, 7)
        result.append({"name": name, "status": 0, "class": "none", "fields": fields})
    errors = (("permission", "permission_denied"), ("missing-error", "not_found"),
              ("business-error", "service_failure"), ("wrong-type", "protocol"),
              ("partial-eof", "io"), ("wire-limit", "resource_limit"),
              ("utf8-limit", "resource_limit"), ("json-limit", "resource_limit"),
              ("timeout", "timeout"), ("cancel", "cancelled"),
              ("close", "cancelled"), ("admission", "cancelled"))
    result.extend({"name": name, "status": 3, "class": category, "fields": 7} for name, category in errors)
    result.append({"name": "contracts", "status": 0, "class": "none", "fields": 7})
    return result


class NativePeer:
    """One original bounded peer thread, not a fake Rust result/source."""
    def __init__(self, case: dict, tls: ssl.SSLContext):
        self.case, self.tls = case, tls
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(4)
        self.listener.settimeout(SOCKET_SECONDS)
        self.endpoint = "tcp://127.0.0.1:" + str(self.listener.getsockname()[1])
        self.ready = threading.Event()
        self.stop = threading.Event()
        self.lock = threading.Lock()
        self.active: socket.socket | None = None
        self.receipts: list[dict] = []
        self.error: dict | None = None
        self.phase = "accept"
        self.thread = None if case["name"] == "contracts" else threading.Thread(target=self.run, name="original-native-peer")

    def start(self) -> None:
        if self.thread is not None:
            self.thread.start()

    def strict_eof(self, stream: socket.socket, receipt: dict) -> None:
        self.phase = "strict-raw-tcp-eof"
        # SSL recv()==b'' can mean only close_notify. Detach and use original raw
        # TCP recv()==b''; no timeout/reset conversion into a release receipt.
        tls_stream = isinstance(stream, ssl.SSLSocket)
        if tls_stream:
            raw_fd = stream.detach()
            stream = socket.socket(fileno=raw_fd)
            stream.settimeout(SOCKET_SECONDS)
            with self.lock:
                self.active = stream
        tail = 0
        try:
            while True:
                chunk = stream.recv(4096)
                if chunk == b"":
                    receipt["strict_raw_tcp_eof"] = True
                    receipt["tcp_tail_bytes_after_last_reply"] = tail
                    return
                tail += len(chunk)
                if not tls_stream or tail > 65536:
                    raise ValueError("unexpected post-query native bytes")
        finally:
            stream.close()
            with self.lock:
                self.active = None

    def exchange(self, stream: socket.socket, ordinal: int) -> dict:
        name = self.case["name"]
        device_id = 7 if ordinal == 0 else 99
        receipt = {"ordinal": ordinal, "device_id": device_id, "steps": [],
                   "strict_raw_tcp_eof": False, "tls_version": None}
        self.receipts.append(receipt)
        self.phase = "list-devices"
        read_mux(stream, 1, "ListDevices")
        receipt["steps"].append({"request": "ListDevices", "tag": 1})
        if name == "wire-limit":
            # Only the charged header is sent. No unread body can create a TCP
            # reset when the real budget guard drops its socket.
            stream.sendall(struct.pack("<IIII", 1024, 1, 8, 1))
            self.strict_eof(stream, receipt)
            return receipt
        properties = {"SerialNumber": UDID, "DeviceID": device_id, "ConnectionType": "USB"}
        if name == "tls":
            properties["ProductID"] = 0
        write_mux(stream, 1, {"DeviceList": [{"DeviceID": device_id, "Properties": properties}]})
        self.phase = "connect"
        connect = read_mux(stream, 2, "Connect")
        port = int.from_bytes((62078).to_bytes(2, "big"), "little")
        if connect.get("DeviceID") != device_id or connect.get("PortNumber") != port:
            raise ValueError("fresh native DeviceID or lockdown port mismatch")
        receipt["steps"].append({"request": "Connect", "tag": 2, "device_id": device_id, "port_wire": port})
        write_mux(stream, 2, {"MessageType": "Result", "Number": 0})
        self.phase = "query-type"
        read_lockdown(stream, "QueryType")
        receipt["steps"].append({"request": "QueryType"})
        write_lockdown(stream, {"Request": "QueryType", "Type": "com.apple.mobile.lockdown"})
        self.phase = "start-session"
        start = read_lockdown(stream, "StartSession")
        if start.get("HostID") != HOST_ID or start.get("SystemBUID") != BUID or start.get("ProtocolVersion") != "2":
            raise ValueError("existing-trust snapshot mismatch")
        receipt["steps"].append({"request": "StartSession", "owned_pair_snapshot_matches": True})
        if name == "permission":
            write_lockdown(stream, {"Request": "StartSession", "Error": "InvalidHostID",
                                    "SessionID": "must-not-win", "EnableSessionSSL": False})
            self.strict_eof(stream, receipt)
            return receipt
        enable_tls = name in ("tls", "cancel", "close")
        write_lockdown(stream, {"Request": "StartSession", "SessionID": "fixture-session", "EnableSessionSSL": enable_tls})
        if enable_tls:
            self.phase = "native-tls-handshake"
            stream = self.tls.wrap_socket(stream, server_side=True)
            stream.settimeout(SOCKET_SECONDS)
            with self.lock:
                self.active = stream
            receipt["tls_version"] = stream.version()
        self.phase = "get-value"
        get = read_lockdown(stream, "GetValue")
        expected_key = KEYS.get(self.case["fields"])
        if get.get("Key") != expected_key or "Domain" in get:
            raise ValueError("finite native query selector mismatch")
        receipt["steps"].append({"request": "GetValue", "key": expected_key})
        if name in ("cancel", "close", "admission", "timeout"):
            self.ready.set()
            self.strict_eof(stream, receipt)
            return receipt
        if name == "partial-eof":
            stream.sendall(struct.pack(">I", 128) + b"<")
            stream.shutdown(socket.SHUT_WR)
            self.strict_eof(stream, receipt)
            return receipt
        reply = {"Request": "GetValue"}
        if name in ("missing-error", "business-error"):
            reply["Error"] = "MissingValue" if name == "missing-error" else "GetProhibited"
            reply["Value"] = dict(VALUES)
        elif name != "root-missing":
            if expected_key is not None:
                reply["Value"] = "" if name == "single-empty" else VALUES[expected_key]
            else:
                value = {} if name == "root-empty" else dict(VALUES)
                if name == "tls":
                    value["DeviceName"] = ""
                if name == "filtered":
                    value["DeviceName"] = {"unselected": "discard this opaque value"}
                    value["UnselectedPrivateField"] = "not-projected"
                if name == "wrong-type":
                    value["ProductVersion"] = 123
                if name == "binary-nul":
                    value["DeviceName"] = "owned\0设备"
                reply["Value"] = value
        write_lockdown(stream, reply, binary=name == "binary-nul")
        self.strict_eof(stream, receipt)
        return receipt

    def run(self) -> None:
        try:
            count = 2 if self.case["name"] == "fresh" else 1
            for ordinal in range(count):
                self.phase = "accept"
                stream, _ = self.listener.accept()
                stream.settimeout(SOCKET_SECONDS)
                stream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                with self.lock:
                    self.active = stream
                try:
                    self.exchange(stream, ordinal)
                finally:
                    with self.lock:
                        if self.active is not None:
                            self.active.close()
                            self.active = None
        except Exception as error:
            # Bounded phase/type evidence: never raw plist/key/identity/error text.
            self.error = {"phase": self.phase, "exception_type": type(error).__name__}

    def joined_receipt(self) -> dict:
        if self.thread is not None:
            self.thread.join(SOCKET_SECONDS + 1)
            if self.thread.is_alive():
                raise TimeoutError("original native peer thread did not join")
            if self.error is not None or not all(receipt["strict_raw_tcp_eof"] for receipt in self.receipts):
                raise ValueError("original native peer lacks strict TCP EOF receipt")
        # The first active lease must prevent a second Source open in admission;
        # checked invalid inputs/pre-cancel/closed handles must open no Source.
        self.listener.setblocking(False)
        try:
            unexpected, _ = self.listener.accept()
        except BlockingIOError:
            unexpected = None
        if unexpected is not None:
            unexpected.close()
            raise ValueError("unexpected extra native Source connection")
        return {"original_peer_thread_joined": self.thread is not None,
                "native_operations": self.receipts,
                "no_extra_source_connection": True,
                "no_native_owner_opened": self.thread is None}

    def cleanup(self) -> bool:
        self.stop.set()
        with self.lock:
            if self.active is not None:
                try:
                    self.active.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                self.active.close()
                self.active = None
        self.listener.close()
        if self.thread is not None:
            self.thread.join(SOCKET_SECONDS + 1)
        return self.thread is None or not self.thread.is_alive()


def validate_result(case: dict, payload: dict, peer: dict) -> None:
    if payload.get("status") != "passed":
        raise ValueError("C assertion gate did not pass")
    if case["name"] == "contracts":
        if payload.get("scenario") != "contracts-no-source" or not peer["no_native_owner_opened"]:
            raise ValueError("checked-input no-source gate mismatch")
        return
    if not payload.get("original_c_thread_joined") or not payload.get("peer_join_receipt_before_assertions"):
        raise ValueError("original C pthread join receipt missing")
    if payload.get("query_status") != case["status"] or payload.get("query_class") != case["class"]:
        raise ValueError("native C status/category mismatch")
    results = payload.get("results")
    count = 2 if case["name"] == "fresh" else 1
    if not isinstance(results, list) or len(results) != count:
        raise ValueError("native query result count mismatch")
    if case["status"] != 0:
        if results != [None]:
            raise ValueError("error returned partial owned JSON")
        return
    for ordinal, result in enumerate(results):
        name = case["name"]
        selected = {key: value for bit, key in KEYS.items() if case["fields"] & bit for value in [VALUES[key]]}
        if name in ("root-missing", "root-empty"):
            selected = {key: None for key in selected}
        if name in ("tls", "single-empty"):
            selected["DeviceName"] = ""
        if name == "binary-nul":
            selected["DeviceName"] = "owned\0设备"
        expected = {"value_present": name != "root-missing", "requested_fields": case["fields"],
                    "values": selected, "device_id": 7 if ordinal == 0 else 99,
                    "connection_type": "USB", "product_id": 0 if name == "tls" else None,
                    "enable_session_ssl": name == "tls"}
        if result != expected:
            raise ValueError("owned selected C JSON projection mismatch")


def run_case(binary: Path, install: Path, case: dict, pair: bytes, tls: ssl.SSLContext, evidence: Path) -> dict:
    peer = NativePeer(case, tls)
    evidence.mkdir()
    record = {"case": case["name"], "expected_status": case["status"], "expected_class": case["class"],
              "status": "failed", "peer": None, "c_returncode": None, "failure": None}
    environment = dict(os.environ)
    environment["LD_LIBRARY_PATH"] = str(install)
    environment["DYLD_LIBRARY_PATH"] = str(install)
    # Never inherit a library injection that could override the installed binary.
    environment.pop("LD_PRELOAD", None)
    environment.pop("DYLD_INSERT_LIBRARIES", None)
    if platform.system() == "Darwin":
        environment["DYLD_PRINT_LIBRARIES"] = "1"
    process = None
    output, errors, pending = bytearray(), bytearray(), bytearray()
    selector = selectors.DefaultSelector()
    released_assertions = False
    acted = False
    start = time.monotonic()
    try:
        peer.start()
        process = subprocess.Popen([str(binary), peer.endpoint, case["name"], str(case["status"]), case["class"], str(case["fields"])],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment)
        process.stdin.write(struct.pack(">I", len(pair)) + pair)
        process.stdin.flush()
        for stream, kind in ((process.stdout, "stdout"), (process.stderr, "stderr")):
            os.set_blocking(stream.fileno(), False)
            selector.register(stream, selectors.EVENT_READ, kind)
        while selector.get_map():
            if time.monotonic() - start > CASE_SECONDS:
                raise TimeoutError("bounded linked C scenario deadline")
            if case["name"] in ("cancel", "close", "admission") and peer.ready.is_set() and not acted:
                process.stdin.write(b"act\n")
                process.stdin.flush()
                acted = True
            for key, _ in selector.select(0.05):
                chunk = os.read(key.fileobj.fileno(), 8192)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                target = output if key.data == "stdout" else errors
                if len(target) + len(chunk) > MAX_CAPTURE:
                    raise ValueError("bounded C evidence capture limit")
                target.extend(chunk)
                if key.data != "stdout":
                    continue
                pending.extend(chunk)
                while b"\n" in pending:
                    line, _, remainder = pending.partition(b"\n")
                    pending[:] = remainder
                    if line in (b"C_QUERY_THREAD_JOINED", b"C_CONTRACTS_JOINED"):
                        if released_assertions:
                            raise ValueError("duplicate C join marker")
                        if platform.system() == "Linux":
                            mapped = []
                            for entry in Path(f"/proc/{process.pid}/maps").read_text().splitlines():
                                fields = entry.split(maxsplit=5)
                                if len(fields) == 6 and Path(fields[5]).name == "libios_ffi.so":
                                    mapped.append(fields[5])
                            if set(mapped) != {str(install / "libios_ffi.so")}:
                                raise ValueError("actual C process library mapping differs from private install")
                            record["loaded_library_path"] = mapped[0]
                            record["loaded_library_sha256"] = sha256(Path(mapped[0]))
                        record["peer"] = peer.joined_receipt()
                        process.stdin.write(b"peer_joined\n")
                        process.stdin.flush()
                        released_assertions = True
        record["c_returncode"] = process.wait(timeout=1)
        if record["c_returncode"] != 0 or not released_assertions:
            raise ValueError("linked C assertion process failed")
        if platform.system() == "Darwin":
            mapped = str(install / "libios_ffi.dylib")
            if mapped.encode() not in errors:
                raise ValueError("actual dyld library load receipt missing")
            record["loaded_library_path"] = mapped
            record["loaded_library_sha256"] = sha256(Path(mapped))
        lines = [line for line in output.decode("utf-8").splitlines() if line.startswith("{")]
        if len(lines) != 1:
            raise ValueError("linked C result envelope mismatch")
        payload = json.loads(lines[0])
        # Also cover the C closed-handle probe and final checked cleanup, which
        # run after the first EOF/join receipt was delivered to the caller.
        record["peer"] = peer.joined_receipt()
        validate_result(case, payload, record["peer"])
        record["c_result"] = payload
        record["status"] = "passed"
    except Exception as error:
        record["failure"] = {"exception_type": type(error).__name__, "message": str(error)}
    finally:
        if process is not None:
            if process.poll() is None:
                process.kill()  # Owned finite C subprocess only; never another session.
            process.wait(timeout=2)
            record["c_returncode"] = process.returncode
            # Retain unread final failure output after reaping this owned process.
            # Every capture has the same finite bound, including assertion aborts.
            for stream, target in ((process.stdout, output), (process.stderr, errors)):
                while True:
                    try:
                        chunk = os.read(stream.fileno(), 8192)
                    except BlockingIOError:
                        break
                    if not chunk:
                        break
                    room = MAX_CAPTURE - len(target)
                    target.extend(chunk[:room])
                    if len(chunk) > room:
                        record["capture_truncated"] = True
                        record["status"] = "failed"
            for stream in (process.stdin, process.stdout, process.stderr):
                stream.close()
        selector.close()
        record["original_peer_thread_joined_after_cleanup"] = peer.cleanup()
        record["peer_failure"] = peer.error
        record["native_receipts"] = peer.receipts
        if not record["original_peer_thread_joined_after_cleanup"]:
            record["status"] = "failed"
            record["failure"] = {"exception_type": "CleanupFailure", "message": "original peer thread still alive"}
        record["elapsed_seconds"] = time.monotonic() - start
        (evidence / "c.stdout").write_bytes(output)
        (evidence / "c.stderr").write_bytes(errors)
        (evidence / "result.json").write_text(json.dumps(record, sort_keys=True, indent=2) + "\n")
    return record


def c_command(cc: str, header: Path, source: Path, binary: Path | None, library: Path | None) -> list[str]:
    command = [cc, "-std=c11", "-Wall", "-Wextra", "-Werror", "-pthread", "-I", str(header.parent)]
    if binary is None:
        return command + ["-fsyntax-only", str(source)]
    return command + [str(source), str(library), "-Wl,-rpath," + str(library.parent), "-o", str(binary)]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--cc", default="cc")
    parser.add_argument("--static", action="store_true", help="C syntax only, no library or Rust compiler")
    parser.add_argument("--fingerprint", action="store_true", help="print current build-input manifest, without building")
    parser.add_argument("--library", type=Path, help="fresh root-built libios_ffi.so/.dylib")
    parser.add_argument("--build-evidence", type=Path, help="root build evidence JSON; see crate README")
    parser.add_argument("--output", type=Path, help="new private evidence directory, never overwritten")
    args = parser.parse_args()
    repo = args.repo.resolve()
    header = repo / "crates/ios-ffi/include/ios_rs.h"
    example = repo / "crates/ios-ffi/examples/device_info.c"
    source = repo / "crates/ios-ffi/examples/device_session_smoke.c"
    if args.fingerprint:
        print(json.dumps(source_fingerprint(repo), sort_keys=True, indent=2))
        return 0
    if args.static:
        for path in (example, source):
            subprocess.run(c_command(args.cc, header, path, None, None), check=True)
        print("C syntax passed; Rust/native/linked gates NOT_RUN")
        return 0
    if not all((args.library, args.build_evidence, args.output)):
        parser.error("linked gate requires --library, --build-evidence and --output")
    if os.name != "posix" or platform.system() not in ("Linux", "Darwin"):
        parser.error("POSIX Linux/macOS linked pthread fixture only; other platform gate NOT_RUN")
    library = args.library.resolve()
    if library.name not in ("libios_ffi.so", "libios_ffi.dylib"):
        parser.error("provide the matching native dynamic library basename")
    evidence = json.loads(args.build_evidence.read_text())
    fingerprint = source_fingerprint(repo)
    library_hash = sha256(library)
    if evidence.get("build_result") != "passed" or evidence.get("source_fingerprint") != fingerprint["sha256"] or evidence.get("library_sha256") != library_hash:
        parser.error("fresh root build evidence does not match current frozen sources/library")
    for name in ("rust_build_command", "rust_toolchain", "build_mode"):
        if not isinstance(evidence.get(name), str) or not evidence[name]:
            parser.error("root build evidence missing " + name)
    build_log = evidence.get("build_log", {})
    if not build_log.get("path") or sha256(Path(build_log["path"])) != build_log.get("sha256"):
        parser.error("root build log evidence missing or hash mismatched")
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    install = output / "installed"
    install.mkdir(mode=0o700)
    installed_library = install / library.name
    installed_header = install / header.name
    shutil.copy2(library, installed_library)
    shutil.copy2(header, installed_header)
    if sha256(installed_library) != library_hash:
        raise ValueError("installed dynamic library hash mismatch")
    report = {"schema": "ios-ffi-finite-device-query-v1", "status": "failed", "source": fingerprint,
              "root_build_evidence_sha256": sha256(args.build_evidence), "root_build_evidence": evidence,
              "library_sha256": library_hash, "header_sha256": sha256(header),
              "c_source_sha256": sha256(source), "driver_sha256": sha256(Path(__file__)),
              "host": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(),
                       "python": platform.python_version(), "openssl": ssl.OPENSSL_VERSION},
              "cases": [], "physical_device": "NOT_RUN", "other_platforms": "NOT_RUN",
              "global_runtime_exhaustion": "NOT_RUN", "main_A03": "NOT_COMPLETE"}
    try:
        binary = install / "device-session-smoke"
        command = c_command(args.cc, installed_header, source, binary, installed_library)
        report["c_link_command"] = command
        compile_result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
        (output / "c-link.stdout").write_bytes(compile_result.stdout[:MAX_CAPTURE])
        (output / "c-link.stderr").write_bytes(compile_result.stderr[:MAX_CAPTURE])
        if compile_result.returncode != 0 or len(compile_result.stdout) > MAX_CAPTURE or len(compile_result.stderr) > MAX_CAPTURE:
            raise ValueError("strict installed C dynamic link failed")
        report["c_binary_sha256"] = sha256(binary)
        pair, tls = synthetic_pair(repo, install)
        for case in cases():  # Deliberately one native C process at a time.
            result = run_case(binary, install, case, pair, tls, output / case["name"])
            report["cases"].append(result)
            if result["status"] != "passed":
                break  # Keep bounded full failure evidence, no stale result reuse.
        report["source_unchanged"] = source_fingerprint(repo) == fingerprint
        report["gate_source_unchanged"] = (sha256(header) == report["header_sha256"] and
                                          sha256(source) == report["c_source_sha256"] and
                                          sha256(Path(__file__)) == report["driver_sha256"])
        report["library_unchanged"] = sha256(library) == library_hash and sha256(installed_library) == library_hash
        if len(report["cases"]) == len(cases()) and all(case["status"] == "passed" for case in report["cases"]) and report["source_unchanged"] and report["gate_source_unchanged"] and report["library_unchanged"]:
            report["status"] = "passed"
    except Exception as error:
        report["failure"] = {"exception_type": type(error).__name__, "message": str(error)}
    finally:
        for path in (install / "synthetic-cert.pem", install / "synthetic-key.pem"):
            path.unlink(missing_ok=True)
        (output / "report.json").write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    print(json.dumps({"status": report["status"], "evidence": str(output / "report.json"),
                      "completed_host_cases": len(report["cases"]), "main_A03": "NOT_COMPLETE"}))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
