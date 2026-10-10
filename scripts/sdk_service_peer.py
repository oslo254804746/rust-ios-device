"""Owned read-only usbmux/Lockdown/apps/AFC/syslog protocol fixture.

No SDK replacement, device operations or compiler are provided here. Original
peer threads and every accepted socket must retire with strict EOF.
"""
import ast
from pathlib import Path
import plistlib
import re
import socket
import ssl
import struct
import threading

UDID = "owned-sdk-service-device"
HOST = "owned-sdk-service-host"
BUID = "owned-sdk-service-buid"
PORT = 12345
BOUND = 5.0
DATA = b"owned SDK file\x00\xff\n" * 73


def assets(path):
    text = path.read_text(encoding="utf-8")
    values = []
    for name in ("TEST_CERT_PEM", "TEST_KEY_PEM"):
        match = re.search(r"const " + name + r': &\[u8\] = b"(.*?)";', text, re.S)
        assert match, name
        values.append(ast.literal_eval('b"' + match.group(1).replace("\n", r"\n") + '"'))
    return values


def pair_bytes(cert, key):
    return plistlib.dumps({"DeviceCertificate": b"", "HostCertificate": cert,
                          "HostPrivateKey": key, "RootCertificate": b"",
                          "HostID": HOST, "SystemBUID": BUID})


def exact(connection, count):
    value = bytearray()
    while len(value) < count:
        part = connection.recv(count - len(value))
        if not part:
            raise EOFError("peer request ended early")
        value.extend(part)
    return bytes(value)


def mux_read(connection):
    size, version, kind, tag = struct.unpack("<IIII", exact(connection, 16))
    assert 16 <= size <= 16384 and (version, kind) == (1, 8)
    return plistlib.loads(exact(connection, size - 16)), tag


def mux_write(connection, value, tag):
    body = plistlib.dumps(value)
    connection.sendall(struct.pack("<IIII", len(body) + 16, 1, 8, tag) + body)


def plist_read(connection):
    size = struct.unpack(">I", exact(connection, 4))[0]
    assert 0 < size < 16384
    return plistlib.loads(exact(connection, size))


def plist_write(connection, value):
    body = plistlib.dumps(value)
    connection.sendall(struct.pack(">I", len(body)) + body)


def afc_read(connection):
    magic, size, split, packet, opcode = struct.unpack("<8sQQQQ", exact(connection, 40))
    assert magic == b"CFA6LPAA" and 40 <= split <= size <= 16384
    body = exact(connection, size - 40)
    return packet, opcode, body[:split - 40], body[split - 40:]


def afc_write(connection, packet, opcode, header=b"", payload=b""):
    connection.sendall(struct.pack("<8sQQQQ", b"CFA6LPAA", 40 + len(header) + len(payload),
                                   40 + len(header), packet, opcode) + header + payload)


class Peer:
    def __init__(self, *, kind="apps", stall=None, context=None, tls=False,
                 huge=False, finish=False, data=DATA):
        self.kind, self.stall, self.context, self.tls = kind, stall, context, tls
        self.huge, self.finish, self.data = huge, finish, data
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.endpoint = "tcp://127.0.0.1:" + str(self.listener.getsockname()[1])
        self.listener.listen(4)
        self.listener.settimeout(0.05)
        self.stop = threading.Event()
        self.ready = threading.Event()
        self.errors, self.connections, self.workers, self.eof = [], [], [], []
        self.requests = []
        self.listener_thread = threading.Thread(target=self.accept, name="owned-SDK-service-listener")
        self.listener_thread.start()

    def accept(self):
        while not self.stop.is_set():
            try:
                connection, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                if self.stop.is_set():
                    return
                raise
            connection.settimeout(BOUND)
            self.connections.append(connection)
            worker = threading.Thread(target=self.serve, args=(connection,), name="owned-SDK-service-peer")
            self.workers.append(worker)
            worker.start()

    def idle(self, connection):
        self.ready.set()
        assert connection.recv(1) == b"", "unexpected tail or missing native EOF"
        self.eof.append(True)

    def lockdown(self, connection, tag):
        if self.stall == "mux":
            return self.idle(connection)
        mux_write(connection, {"DeviceList": [{"DeviceID": 7, "Properties": {
            "SerialNumber": UDID, "ConnectionType": "USB"}}]}, tag)
        connect, tag = mux_read(connection)
        self.requests.append(connect)
        assert connect["MessageType"] == "Connect" and connect["DeviceID"] == 7
        assert connect["PortNumber"] == socket.htons(62078)
        mux_write(connection, {"MessageType": "Result", "Number": 0}, tag)
        query = plist_read(connection)
        self.requests.append(query)
        assert query["Request"] == "QueryType"
        if self.stall == "lockdown":
            return self.idle(connection)
        plist_write(connection, {"Request": "QueryType", "Type": "com.apple.mobile.lockdown"})
        start = plist_read(connection)
        self.requests.append(start)
        assert start["Request"] == "StartSession" and start["HostID"] == HOST and start["SystemBUID"] == BUID
        plist_write(connection, {"Request": "StartSession", "SessionID": "owned-SDK-session", "EnableSessionSSL": False})
        service = plist_read(connection)
        self.requests.append(service)
        expected = {"apps": "com.apple.mobile.installation_proxy", "files": "com.apple.afc",
                    "read": "com.apple.afc", "logs": "com.apple.syslog_relay"}[self.kind]
        assert service["Request"] == "StartService" and service["Service"] == expected
        plist_write(connection, {"Request": "StartService", "Port": PORT, "EnableServiceSSL": self.tls})
        # Socket destruction is checked independently of service completion.
        assert connection.recv(1) == b"", "control socket must retire"
        self.eof.append(True)

    def service(self, connection, request, tag):
        assert request["DeviceID"] == 7 and request["PortNumber"] == socket.htons(PORT)
        if self.stall == "service_connect":
            return self.idle(connection)
        mux_write(connection, {"MessageType": "Result", "Number": 0}, tag)
        if self.tls:
            connection = self.context.wrap_socket(connection, server_side=True)
        if self.kind == "apps":
            browse = plist_read(connection)
            self.requests.append(browse)
            assert browse["Command"] == "Browse" and browse["ClientOptions"]["ApplicationType"] == "User"
            if self.stall == "service":
                return self.idle(connection)
            if self.huge:
                connection.sendall(struct.pack(">I", 4 * 1024 * 1024 + 1))
            else:
                plist_write(connection, {"Status": "Complete", "CurrentList": [{
                    "CFBundleIdentifier": "org.fixture.real", "CFBundleDisplayName": "Owned app",
                    "CFBundleShortVersionString": "1.2", "ApplicationType": "User", "Path": "/owned/app"}]})
        elif self.kind == "files":
            packet, opcode, header, payload = afc_read(connection)
            self.requests.append((opcode, header, payload))
            assert opcode == 3 and header == b"/owned\0" and not payload
            if self.stall == "service":
                return self.idle(connection)
            if self.huge:
                connection.sendall(struct.pack("<8sQQQQ", b"CFA6LPAA", 40 + 4 * 1024 * 1024 + 1, 40, packet, 2))
            else:
                afc_write(connection, packet, 2, payload=b".\0..\0owned-a\0owned-b\0")
        elif self.kind == "read":
            packet, opcode, header, payload = afc_read(connection)
            self.requests.append((opcode, header, payload))
            assert opcode == 13 and header[:8] == struct.pack("<Q", 1) and header[8:] == b"/owned\0" and not payload
            if self.stall == "service":
                return self.idle(connection)
            afc_write(connection, packet, 14, struct.pack("<Q", 11))
            offset = 0
            while True:
                packet, opcode, header, payload = afc_read(connection)
                self.requests.append((opcode, header, payload))
                assert not payload
                if opcode == 20:
                    assert header == struct.pack("<Q", 11)
                    afc_write(connection, packet, 1, struct.pack("<Q", 0))
                    break
                assert opcode == 15
                handle, size = struct.unpack("<QQ", header)
                assert handle == 11 and 1 <= size <= 65536
                chunk = self.data[offset:offset + size]
                offset += len(chunk)
                afc_write(connection, packet, 2, payload=chunk)
        elif self.kind == "logs":
            if self.stall == "service":
                return self.idle(connection)
            connection.sendall(b"owned log one\0")
            if self.finish:
                connection.shutdown(socket.SHUT_WR)
            else:
                return self.idle(connection)
        self.idle(connection)

    def serve(self, connection):
        try:
            request, tag = mux_read(connection)
            self.requests.append(request)
            if request["MessageType"] == "ListDevices":
                self.lockdown(connection, tag)
            else:
                assert request["MessageType"] == "Connect"
                self.service(connection, request, tag)
        except BaseException as error:
            self.errors.append(error)
        finally:
            connection.close()

    def close(self, *, expected_connections=None):
        for worker in self.workers:
            worker.join(BOUND)
        self.stop.set()
        self.listener_thread.join(BOUND)
        self.listener.close()
        assert not self.listener_thread.is_alive() and all(not w.is_alive() for w in self.workers), "original peer threads still active"
        assert not self.errors, self.errors
        assert len(self.eof) == len(self.connections), "strict original socket EOF required"
        if expected_connections is not None:
            assert len(self.connections) == expected_connections, (len(self.connections), expected_connections)

    def abort(self):
        # Failure-only local cleanup. Forced closes never count as EOF evidence.
        self.stop.set()
        for connection in self.connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.listener_thread.join(BOUND)
        self.listener.close()
        for worker in self.workers:
            worker.join(BOUND)
