"""Actual CLI debug statistics with a policy denying all device discovery/IO."""
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import socket
import tempfile
import threading
import time
import unittest

from diagnostics import ROOT, assess, capture, secret_variants

CLI = Path(os.environ.get("IOS_REGRESSION_CLI", ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios")))


@unittest.skipUnless(CLI.is_file(), "compiled CLI required")
class ActualManagerPayloadTests(unittest.TestCase):
    def test_readonly_policy_emits_bounded_identity_free_stats_and_manager_shutdown(self):
        token = os.urandom(32).hex().encode()
        device = "synthetic-payload-selected-device"
        variants = secret_variants(token) | secret_variants(device.encode())
        results, failures = [], []
        stop = threading.Event()
        with tempfile.TemporaryDirectory(prefix="ios-payload-cli-") as directory, socket.socket() as trap:
            config = Path(directory) / "policy.json"
            with os.fdopen(os.open(config, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as file:
                file.write(json.dumps({"version": 1, "grants": [{
                    "token_sha256": hashlib.sha256(token).hexdigest(),
                    "expires_unix": int(time.time()) + 30,
                    "permissions": ["list", "manage_server"], "devices": [device],
                }]}).encode())
            trap.bind(("127.0.0.1", 0))
            trap.listen()
            trap.setblocking(False)
            environment = dict(os.environ, USBMUXD_SOCKET_ADDRESS="tcp://127.0.0.1:" + str(trap.getsockname()[1]))
            with socket.socket() as reservation:
                reservation.bind(("127.0.0.1", 0))
                port = reservation.getsockname()[1]

            def owned_capture():
                try:
                    results.append(capture([str(CLI.resolve()), "-vvvv", "tunnel", "serve", "--userspace",
                                            "--host", "127.0.0.1", "--port", str(port),
                                            "--auth-config", str(config)], env=environment, timeout=10, stop=stop))
                except Exception as error:
                    failures.append(type(error).__name__)

            def request(method, path):
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=0.25)
                try:
                    connection.request(method, path, headers={"Authorization": "Bearer " + token.decode()})
                    response = connection.getresponse()
                    return response.status, response.read(4097)
                finally:
                    connection.close()

            worker = threading.Thread(target=owned_capture, name="actual-manager-payload-capture")
            worker.start()
            try:
                until = time.monotonic() + 5
                ready = False
                while time.monotonic() < until and worker.is_alive():
                    try:
                        status, body = request("GET", "/ready")
                        ready = status == 200
                        if ready:
                            break
                    except (OSError, http.client.HTTPException, ValueError):
                        pass
                    time.sleep(0.02)
                self.assertTrue(ready, "original manager readiness not observed")
                status, body = request("GET", "/tunnels")
                self.assertEqual(status, 200)
                self.assertEqual(json.loads(body), [])
                status, body = request("POST", "/shutdown")
                self.assertEqual(status, 200)
                self.assertIs(json.loads(body).get("data"), True)
                worker.join(3)
                self.assertFalse(worker.is_alive(), "original capture/manager did not join")
            finally:
                if worker.is_alive():
                    stop.set()
                worker.join(12)
            self.assertFalse(failures)
            self.assertEqual(len(results), 1)
            result = results[0]
            self.assertTrue(assess(result, variants, 0)["passed"])
            self.assertFalse(result.stop_requested)
            stats = [line for line in result.stderr.splitlines() if b"sampled manager retained error payloads" in line]
            self.assertTrue(stats, "production debug payload statistics absent")
            for line in stats:
                plain = re.sub(rb"\x1b\[[0-9;]*m", b"", line)
                for key in (b"known_payload_capacity_bytes", b"opaque_nodes", b"visited_nodes", b"active_scans",
                            b"unjoined_native_owners"):
                    self.assertRegex(plain, key + rb"=0\b")
                self.assertIn(b"fully_measured_payloads=true", plain)
                self.assertIn(b"truncated=false", plain)
            connections = 0
            while True:
                try:
                    peer, _ = trap.accept()
                    peer.close()
                    connections += 1
                except BlockingIOError:
                    break
            self.assertEqual(connections, 0, "list/server-only policy must skip original Source discovery")


if __name__ == "__main__":
    unittest.main()
