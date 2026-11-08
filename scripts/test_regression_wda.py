"""Actual WDA CLI -> existing-trust TLS/DTX -> ready -> TCP forward -> finally.

Local synthetic peers only; peer EOF and original thread joins are independent
of the CLI exit. This does not certify WDA on a physical device.
"""
import os
from pathlib import Path
import socket
import sys
import tempfile
import threading
import time
import unittest

from diagnostics import capture
import regression
import regression_adapters as adapters
from regression_cli_fixture import Eof, UDID, exact
from regression_dtx_fixture import CliWorkflowPeer, RUNNER_ID, LAUNCH

CLI = Path(os.environ.get("IOS_REGRESSION_CLI", regression.ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios")))
WDA_PORT = 62088
REQUEST = b"GET /status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
RESPONSE = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nready"


class WdaPeer(CliWorkflowPeer):
    def __init__(self, directory, *, fault=None):
        self.port_calls = 0
        self.forwarded = []
        self.tunnel_rejections = 0
        self.ready = threading.Event()
        super().__init__(directory, fault=fault)

    def connect_result(self, port):
        if port == WDA_PORT and self.fault in ("wda-not-ready", "start"):
            self.port_calls += 1
            return 3  # Real usbmux ConnectionRefused, before a service stream.
        return super().connect_result(port)

    def start_service_reply(self, service):
        if service == "com.apple.internal.devicecompute.CoreDeviceProxy":
            self.tunnel_rejections += 1
            return {"Error": "InvalidService"}
        return super().start_service_reply(service)

    def device_service(self, port, stream):
        if port != WDA_PORT:
            return super().device_service(port, stream)
        stream.settimeout(10)
        self.port_calls += 1
        if self.port_calls == 1:
            self.ready.set()
            exact(stream, 1)  # Readiness must release its original probe socket.
            raise ValueError("fixture-unexpected-ready-probe-write")
        if self.port_calls != 2:
            raise ValueError("fixture-extra-wda-connection")
        request = exact(stream, len(REQUEST))
        if request != REQUEST:
            raise ValueError("fixture-wda-forward-content")
        self.forwarded.append(request)
        stream.sendall(RESPONSE)
        stream.shutdown(socket.SHUT_WR)
        exact(stream, 1)  # Client's real write EOF; resets must fail this worker.
        raise ValueError("fixture-extra-forward-write")


@unittest.skipUnless(sys.platform == "win32" and CLI.is_file(), "Windows isolated existing trust and a compiled native CLI required")
class ActualWdaWorkflowTests(unittest.TestCase):
    def run_case(self, *, fault=None, occupied=False, timeout=8):
        with tempfile.TemporaryDirectory(prefix="ios-native-wda-workflow-") as directory:
            peer = WdaPeer(Path(directory) / "trust", fault=fault)
            reservation = socket.socket()
            reservation.bind(("127.0.0.1", 0))
            host_port = reservation.getsockname()[1]
            if occupied:
                reservation.listen(1)
            else:
                reservation.close()
            client_errors, replies = [], []
            client = None

            def forwarded_client():
                try:
                    if not peer.ready.wait(12):
                        raise TimeoutError("fixture-ready-observation-missing")
                    deadline = time.monotonic() + 12
                    while True:
                        try:
                            stream = socket.create_connection(("127.0.0.1", host_port), timeout=1)
                            break
                        except ConnectionRefusedError:
                            if time.monotonic() >= deadline:
                                raise
                            time.sleep(0.02)
                    with stream:
                        stream.settimeout(10)
                        stream.sendall(REQUEST)
                        stream.shutdown(socket.SHUT_WR)
                        reply = exact(stream, len(RESPONSE))
                        replies.append(reply)
                        try:
                            exact(stream, 1)
                        except Eof:
                            return  # Strict local forwarded EOF.
                        raise ValueError("fixture-extra-local-forward-bytes")
                except Exception as error:
                    client_errors.append(type(error).__name__ + ":" + str(error))

            if fault is None and not occupied:
                client = threading.Thread(target=forwarded_client, name="owned-wda-forward-client")
                client.start()
            context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID), environment=peer.environment)
            try:
                result = capture(adapters.command(context, "runwda", RUNNER_ID, "--host-port", host_port,
                                                 "--device-port", WDA_PORT, "--startup-timeout-secs", timeout,
                                                 "--once"), env=peer.environment, timeout=30, max_bytes=65536)
            finally:
                if client is not None:
                    client.join(15)
                reservation.close()
                complete = peer.close()
            methods = [call for call in peer.calls if isinstance(call, str)]
            self.assertFalse(result.timed_out, (result.stderr, methods, peer.port_calls,
                                               len(peer.pids), peer.errors))
            self.assertTrue(result.cleanup_complete)
            self.assertFalse(result.overflow)
            self.assertFalse(result.incomplete)
            self.assertTrue(complete, peer.errors)
            self.assertFalse(peer.pids)
            self.assertIn(RUNNER_ID, peer.apps)
            self.assertNotIn("InstallTransferred", peer.calls)
            self.assertNotIn("Uninstall", peer.calls)
            self.assertFalse(client_errors)
            if client is not None:
                self.assertFalse(client.is_alive())
            return result, peer, replies

    def test_actual_start_ready_forward_then_original_pid_and_peers_retire(self):
        result, peer, replies = self.run_case()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(replies, [RESPONSE])
        self.assertEqual(peer.forwarded, [REQUEST])
        self.assertEqual(peer.port_calls, 2)
        self.assertEqual(peer.calls.count(LAUNCH), 1)
        self.assertEqual(peer.calls.count("killPid:"), 1)
        self.assertEqual(peer.tunnel_rejections, 1)

    def test_startup_protocol_failures_preserve_original_cleanup(self):
        for fault in ("channel", "session", "authorize"):
            with self.subTest(fault=fault):
                result, peer, replies = self.run_case(fault=fault)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertFalse(replies)
                self.assertFalse(peer.port_calls)
                self.assertEqual(peer.calls.count("killPid:"), int(LAUNCH in peer.calls))

    def test_ready_timeout_retires_confirmed_original_runner(self):
        result, peer, _ = self.run_case(fault="wda-not-ready", timeout=3)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertGreater(peer.port_calls, 0)
        self.assertFalse(peer.forwarded)
        self.assertEqual(peer.calls.count("killPid:"), 1)
        self.assertLess(result.duration_ms, 9000)

    def test_start_submission_without_listening_port_cannot_satisfy_ready(self):
        # startExecuting is one-way. Its write is not a device completion ACK;
        # the explicit negative port observation must still prevent success.
        result, peer, _ = self.run_case(fault="start", timeout=3)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertGreater(peer.port_calls, 0)
        self.assertFalse(peer.forwarded)
        self.assertEqual(peer.calls.count("killPid:"), 1)

    def test_local_bind_failure_after_ready_still_retires_original_runner(self):
        result, peer, _ = self.run_case(occupied=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(peer.port_calls, 1)
        self.assertFalse(peer.forwarded)
        self.assertEqual(peer.calls.count("killPid:"), 1)


if __name__ == "__main__":
    unittest.main()
