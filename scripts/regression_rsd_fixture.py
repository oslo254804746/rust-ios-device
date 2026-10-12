"""Owned IPv6 localhost RSD/RemoteXPC peers for the actual observer CLI.

These peers decode native H2/XPC requests. No observer-shaped success JSON is
served, no device discovery/trust store is accessed, and strict EOF/join belongs
to the original sockets and workers.
"""
import socket
import struct
import threading

from regression_cli_fixture import Eof, UDID, exact

SERVICE = "com.apple.remote.installcoordination_proxy"
BUNDLE = "com.synthetic.InstallObserver"
DB_UUID = "synthetic-observer-db-uuid"
PERSISTENT = b"synthetic-observer-persistent-identifier"


class Signed(int):
    pass


def pad(value):
    return value + b"\0" * (-len(value) % 4)


def encode_value(value):
    if value is None:
        return struct.pack("<I", 0x1000)
    if type(value) is bool:
        return struct.pack("<II", 0x2000, int(value))
    if isinstance(value, Signed):
        return struct.pack("<Iq", 0x3000, value)
    if type(value) is int:
        return struct.pack("<IQ", 0x4000, value)
    if isinstance(value, bytes):
        return struct.pack("<II", 0x8000, len(value)) + pad(value)
    if isinstance(value, str):
        data = value.encode() + b"\0"
        return struct.pack("<II", 0x9000, len(data)) + pad(data)
    if isinstance(value, (list, dict)):
        body = struct.pack("<I", len(value))
        if isinstance(value, list):
            body += b"".join(encode_value(item) for item in value)
            kind = 0xE000
        else:
            body += b"".join(pad(key.encode() + b"\0") + encode_value(item) for key, item in value.items())
            kind = 0xF000
        return struct.pack("<II", kind, len(body)) + body
    raise ValueError("fixture-xpc-encode-type")


def decode_value(data, offset=0, depth=0):
    if depth > 16 or len(data) - offset < 4:
        raise ValueError("fixture-xpc-depth-or-type")
    kind = struct.unpack_from("<I", data, offset)[0]
    offset += 4
    if kind == 0x1000:
        return None, offset
    if kind in (0x2000, 0x3000, 0x4000):
        fmt = {0x2000: "<I", 0x3000: "<q", 0x4000: "<Q"}[kind]
        value = struct.unpack_from(fmt, data, offset)[0]
        return (bool(value) if kind == 0x2000 else Signed(value) if kind == 0x3000 else value), offset + struct.calcsize(fmt)
    if kind == 0xA000:
        return data[offset:offset + 16], offset + 16
    if kind in (0x8000, 0x9000):
        size = struct.unpack_from("<I", data, offset)[0]
        offset += 4
        if size > len(data) - offset:
            raise ValueError("fixture-xpc-data-size")
        value = data[offset:offset + size]
        if kind == 0x9000:
            if not value.endswith(b"\0"):
                raise ValueError("fixture-xpc-string-terminator")
            value = value[:-1].decode()
        return value, offset + ((size + 3) & ~3)
    if kind in (0xE000, 0xF000):
        size, count = struct.unpack_from("<II", data, offset)
        end = offset + 4 + size
        offset += 8
        if count > 128 or end > len(data):
            raise ValueError("fixture-xpc-collection-size")
        value = [] if kind == 0xE000 else {}
        for _ in range(count):
            if kind == 0xF000:
                terminator = data.index(0, offset, end)
                key = data[offset:terminator].decode()
                offset += ((terminator - offset + 1 + 3) & ~3)
            item, offset = decode_value(data, offset, depth + 1)
            if isinstance(value, list):
                value.append(item)
            else:
                if key in value:
                    raise ValueError("fixture-xpc-duplicate-key")
                value[key] = item
        if offset != end:
            raise ValueError("fixture-xpc-collection-trailing")
        return value, offset
    raise ValueError("fixture-xpc-unsupported-type")


def encode_xpc(flags, identifier, value=None):
    body = struct.pack("<II", 0x42133742, 5) + encode_value(value) if value is not None else b""
    return struct.pack("<IIQQ", 0x29B00B92, flags, len(body), identifier) + body


def decode_xpc(data):
    if len(data) < 24:
        raise ValueError("fixture-xpc-header")
    magic, flags, size, identifier = struct.unpack_from("<IIQQ", data)
    if magic != 0x29B00B92 or len(data) != 24 + size or size > 65536:
        raise ValueError("fixture-xpc-wrapper")
    value = None
    if size:
        if struct.unpack_from("<II", data, 24) != (0x42133742, 5):
            raise ValueError("fixture-xpc-body-header")
        value, end = decode_value(data, 32)
        if end != len(data):
            raise ValueError("fixture-xpc-body-trailing")
    return flags, identifier, value


def send_frame(stream, kind, stream_id, payload=b"", flags=0):
    stream.sendall(len(payload).to_bytes(3, "big") + bytes((kind, flags)) + struct.pack(">I", stream_id) + payload)


def read_frame(stream):
    header = exact(stream, 9)
    size = int.from_bytes(header[:3], "big")
    if size > 65536:
        raise ValueError("fixture-h2-frame-size")
    return header[3], header[4], struct.unpack(">I", header[5:])[0] & 0x7FFFFFFF, exact(stream, size) if size else b""


class NativeInstallPeer:
    def __init__(self, *, wire_type="int64", sequence=276, observed_udid=UDID, missing_service=False):
        self.wire_type, self.sequence, self.observed_udid, self.missing_service = wire_type, sequence, observed_udid, missing_service
        self.stop = threading.Event()
        self.errors, self.workers, self.sockets, self.requests, self.rsd_messages = [], [], [], [], []
        self.eofs = 0
        self.lock = threading.Lock()
        self.listeners = {}
        self.acceptors = []
        for name in ("rsd", "service"):
            listener = socket.socket(socket.AF_INET6)
            listener.bind(("::1", 0))
            listener.listen(8)
            listener.settimeout(0.05)
            self.listeners[name] = listener
        self.address = "[::1]:" + str(self.listeners["rsd"].getsockname()[1])
        for name, listener in self.listeners.items():
            thread = threading.Thread(target=self.accept, args=(name, listener), name="owned-native-observer-accept")
            self.acceptors.append(thread)
            thread.start()

    def accept(self, name, listener):
        while not self.stop.is_set():
            try:
                stream, _ = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            stream.settimeout(15)
            self.sockets.append(stream)
            if len(self.workers) >= 8:
                self.errors.append("fixture-observer-connection-limit")
                stream.close()
                break
            worker = threading.Thread(target=self.serve, args=(name, stream), name="owned-native-observer-wire")
            self.workers.append(worker)
            worker.start()

    def serve(self, name, stream):
        try:
            if exact(stream, 24) != b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n":
                raise ValueError("fixture-h2-preface")
            send_frame(stream, 4, 0)
            phase = 0
            while True:
                kind, flags, channel, payload = read_frame(stream)
                if kind != 0:
                    if kind not in (1, 4, 8):
                        raise ValueError("fixture-h2-control-kind")
                    continue
                message_flags, identifier, body = decode_xpc(payload)
                if name == "rsd":
                    self.rsd_messages.append((channel, message_flags, body))
                    if isinstance(body, dict) and body.get("MessageType") == "Handshake":
                        services = {} if self.missing_service else {SERVICE: {"Port": str(self.listeners["service"].getsockname()[1]),
                                                                              "Properties": {"Features": []}}}
                        directory = {"MessageType": "Handshake", "Properties": {"UniqueDeviceID": self.observed_udid}, "Services": services}
                        send_frame(stream, 0, 1, encode_xpc(0x101, 71, directory))
                    continue
                if phase < 3:
                    expected = ((1, 1, {}), (1, 0x201, None), (3, 0x400001, None))[phase]
                    if (channel, message_flags, body) != expected:
                        raise ValueError("fixture-service-bootstrap-order")
                    send_frame(stream, 0, channel, encode_xpc(1, 0))
                    phase += 1
                    continue
                if channel != 1 or message_flags != 0x101 or body != {"RequestVersion": 1, "ProtocolVersion": 1,
                                                                      "RequestType": 4, "BundleID": BUNDLE}:
                    raise ValueError("fixture-install-query-shape-or-flags")
                self.requests.append((name, identifier, body))
                if len(self.requests) > 1:
                    raise ValueError("fixture-zero-budget-put-business-request-on-wire")
                value = Signed(self.sequence) if self.wire_type == "int64" else self.sequence
                reply = {"Success": True, "DBUUID": DB_UUID, "DBSequence": value, "PersistentIdentifier": PERSISTENT}
                send_frame(stream, 0, 1, encode_xpc(0x101, 99, reply))
        except Eof:
            with self.lock:
                self.eofs += 1
        except Exception as error:
            with self.lock:
                self.errors.append(type(error).__name__ + ":" + str(error))
        finally:
            stream.close()

    def close(self):
        self.stop.set()
        for thread in self.acceptors:
            thread.join(2)
        for worker in self.workers:
            worker.join(16)
        complete = all(not worker.is_alive() for worker in self.workers + self.acceptors)
        strict = self.eofs == len(self.workers)
        for listener in self.listeners.values():
            listener.close()
        if not complete:
            for stream in self.sockets:
                try:
                    stream.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            for worker in self.workers:
                worker.join(2)
        for stream in self.sockets:
            stream.close()
        return complete and strict and not self.errors
