"""A bounded usbmux/Lockdown/AFC peer for the actual compiled CLI.

It serves one synthetic existing-trust device. It never connects to usbmuxd,
touches platform trust storage, or executes a hardware operation.
"""
import ast
import os
from pathlib import Path
import plistlib
import socket
import ssl
import struct
import threading
import re

UDID = "00008030-0000000000000001"
REMOTE_PATH = "/synthetic-matrix.bin"
PAYLOAD = bytes(index % 251 for index in range(8193))


class Eof(Exception):
    pass


def exact(stream, size):
    data = bytearray()
    while len(data) < size:
        chunk = stream.recv(size - len(data))
        if not chunk:
            if data:
                raise ValueError("fixture-partial-eof")
            raise Eof()
        data.extend(chunk)
    return bytes(data)


def recv_plist(stream):
    size = struct.unpack(">I", exact(stream, 4))[0]
    if not 0 < size <= 65536:
        raise ValueError("fixture-plist-bound")
    return plistlib.loads(exact(stream, size))


def send_plist(stream, value):
    data = plistlib.dumps(value)
    stream.sendall(struct.pack(">I", len(data)) + data)


class CliAfcPeer:
    services = {"com.apple.afc": 62080}

    def __init__(self, private_directory):
        root = Path(private_directory)
        trust = root / "Apple" / "Lockdown"
        trust.mkdir(parents=True)
        source = (Path(__file__).resolve().parents[1] / "crates/ios-core/src/lockdown/session.rs").read_text(encoding="utf-8")
        values = []
        for name in ("TEST_CERT_PEM", "TEST_KEY_PEM"):
            match = re.search(r"const " + name + r': &\[u8\] = b"(.*?)";', source, re.S)
            if match is None:
                raise ValueError("existing-tls-fixture-unavailable")
            values.append(ast.literal_eval('b"' + match.group(1).replace("\n", r"\n") + '"'))
        cert, key = values
        (root / "cert.pem").write_bytes(cert)
        (root / "key.pem").write_bytes(key)
        self.tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.tls.load_cert_chain(root / "cert.pem", root / "key.pem")
        self.pair = plistlib.dumps({"DeviceCertificate": b"", "HostCertificate": cert,
                                   "HostPrivateKey": key, "RootCertificate": b"",
                                   "HostID": "synthetic-matrix-host", "SystemBUID": "synthetic-matrix-buid"})
        (trust / (UDID + ".plist")).write_bytes(self.pair)
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(16)
        self.listener.settimeout(0.05)
        self.environment = dict(os.environ, ALLUSERSPROFILE=str(root),
                                USBMUXD_SOCKET_ADDRESS="tcp://127.0.0.1:" + str(self.listener.getsockname()[1]))
        self.stop = threading.Event()
        self.workers, self.connections, self.errors, self.operations = [], [], [], []
        self.eofs = 0
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self.accept, name="owned-cli-matrix-mux")
        self.thread.start()

    def accept(self):
        while not self.stop.is_set():
            try:
                stream, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            stream.settimeout(5)
            if len(self.workers) >= 128:
                self.errors.append("fixture-connection-bound")
                stream.close()
                break
            self.connections.append(stream)
            worker = threading.Thread(target=self.serve, args=(stream,), name="owned-cli-matrix-peer")
            self.workers.append(worker)
            worker.start()

    def serve(self, stream):
        try:
            while True:
                size, version, kind, tag = struct.unpack("<IIII", exact(stream, 16))
                if not 16 <= size <= 65536 or (version, kind) != (1, 8):
                    raise ValueError("fixture-mux-framing")
                request = plistlib.loads(exact(stream, size - 16))
                message = request.get("MessageType")
                if message == "ListDevices":
                    reply = {"DeviceList": [{"DeviceID": 1, "Properties": {"SerialNumber": UDID, "ConnectionType": "USB", "ProductID": 1}}]}
                elif message == "ReadPairRecord":
                    if request.get("PairRecordID") != UDID:
                        raise ValueError("fixture-pair-target")
                    reply = {"PairRecordData": self.pair}
                elif message == "ReadBUID":
                    reply = {"BUID": "synthetic-matrix-buid"}
                elif message == "Connect":
                    if request.get("DeviceID") != 1:
                        raise ValueError("fixture-connect-target")
                    reply = {"MessageType": "Result", "Number": self.connect_result(socket.ntohs(request["PortNumber"]))}
                else:
                    raise ValueError("fixture-unexpected-mux-message")
                data = plistlib.dumps(reply)
                stream.sendall(struct.pack("<IIII", len(data) + 16, 1, 8, tag) + data)
                if message == "Connect":
                    if reply["Number"] != 0:
                        continue
                    port = socket.ntohs(request["PortNumber"])
                    if port == 62078:
                        self.lockdown(stream)
                    else:
                        self.device_service(port, stream)
                    return
        except Eof:
            with self.lock:
                self.eofs += 1
        except Exception as error:
            with self.lock:
                self.errors.append(type(error).__name__ + ":" + str(error))
        finally:
            stream.close()

    def connect_result(self, port):
        return 0

    def lockdown(self, stream):
        while True:
            request = recv_plist(stream)
            operation = request.get("Request")
            if operation == "QueryType":
                reply = {"Type": "com.apple.mobile.lockdown"}
            elif operation == "StartSession":
                reply = {"SessionID": "synthetic-matrix-session", "EnableSessionSSL": True}
            elif operation == "GetValue":
                values = {"ProductVersion": "16.0", "UniqueDeviceID": UDID, "ProductType": "synthetic-matrix-device"}
                reply = {"Value": values.get(request["Key"], "") if "Key" in request else values}
            elif operation == "StartService":
                reply = self.start_service_reply(request.get("Service"))
            elif operation == "StopSession":
                reply = {}
            else:
                raise ValueError("fixture-unexpected-lockdown-operation")
            send_plist(stream, dict(reply, Request=operation))
            if operation == "StartSession":
                stream = self.tls.wrap_socket(stream, server_side=True)
                self.connections.append(stream)

    def start_service_reply(self, service):
        if service not in self.services:
            raise ValueError("fixture-service-scope")
        return {"Port": self.services[service], "EnableServiceSSL": False}

    def device_service(self, port, stream):
        if port == 62080:
            self.afc(stream)
        else:
            raise ValueError("fixture-unexpected-device-port")

    def afc(self, stream):
        offset, opened = 0, False
        while True:
            magic, entire, this, packet, opcode = struct.unpack("<QQQQQ", exact(stream, 40))
            if magic != 0x4141504C36414643 or not 40 <= this <= entire <= 131072:
                raise ValueError("fixture-afc-framing")
            body = exact(stream, entire - 40) if entire > 40 else b""
            hp = body[:this - 40]
            self.operations.append(opcode)
            reply_header, reply_data, reply_opcode = b"", b"", 2
            if opcode == 10:
                if hp != REMOTE_PATH.encode() + b"\0":
                    raise ValueError("fixture-afc-path")
                reply_header = b"st_ifmt\0S_IFREG\0st_size\0" + str(len(PAYLOAD)).encode() + b"\0"
            elif opcode == 13:
                if hp[:8] != struct.pack("<Q", 1) or hp[8:] != REMOTE_PATH.encode() + b"\0" or opened:
                    raise ValueError("fixture-afc-open")
                opened, offset = True, 0
                reply_header, reply_opcode = struct.pack("<Q", 1), 14
            elif opcode == 15:
                handle, amount = struct.unpack("<QQ", hp)
                if handle != 1 or not opened or amount > 65536:
                    raise ValueError("fixture-afc-read")
                reply_data = PAYLOAD[offset:offset + amount]
                offset += len(reply_data)
            elif opcode == 20:
                if hp != struct.pack("<Q", 1) or not opened:
                    raise ValueError("fixture-afc-close")
                opened = False
                reply_header, reply_opcode = struct.pack("<Q", 0), 1
            else:
                raise ValueError("fixture-afc-operation-scope")
            stream.sendall(struct.pack("<QQQQQ", magic, 40 + len(reply_header) + len(reply_data), 40 + len(reply_header), packet, reply_opcode) + reply_header + reply_data)

    def close(self):
        self.stop.set()
        self.thread.join(2)
        for worker in self.workers:
            worker.join(6)
        complete = not self.thread.is_alive() and all(not worker.is_alive() for worker in self.workers)
        eof = self.eofs == len(self.workers)
        self.listener.close()
        if not complete:
            # Failure-only owned cleanup. This never supplies an EOF receipt.
            for stream in self.connections:
                try:
                    stream.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            for worker in self.workers:
                worker.join(2)
        for stream in self.connections:
            stream.close()
        return complete and eof and not self.errors
