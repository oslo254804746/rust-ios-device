"""Actual compiled CLI + original IPv6 RemoteXPC peer, never a Rust build."""
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

from diagnostics import capture
import regression
import regression_adapters as adapters
from regression_cli_fixture import UDID
from regression_rsd_fixture import NativeInstallPeer, BUNDLE, DB_UUID, PERSISTENT

CLI = Path(os.environ.get("IOS_REGRESSION_CLI", regression.ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios")))


@unittest.skipUnless(CLI.is_file(), "existing native CLI required; this gate never builds")
class ActualInstallObserverTests(unittest.TestCase):
    def context(self, root, peer):
        record = {"version": 1, "bundle_id": BUNDLE, "db_sequence": 276,
                  "db_uuid_sha256": hashlib.sha256(DB_UUID.encode()).hexdigest(),
                  "persistent_identifier_size": len(PERSISTENT),
                  "persistent_identifier_sha256": hashlib.sha256(PERSISTENT).hexdigest(), "wire_type": "int64"}
        path = root / "record.json"
        path.write_text(json.dumps(record))
        manifest = {"version": 1, "target_udid": UDID,
                    "fixtures": {"existing-rsd-route": {"address": peer.address},
                                 "install-record-observation": {"path": path.name, "sha256": adapters.digest(path)}},
                    "observations": {}, "evidence": {}}
        (root / "manifest.json").write_text(json.dumps(manifest))
        context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID),
                                              scopes=frozenset({"protocol-observation-scope", "synthetic-operation"}), fixture_peer=True)
        adapters.load_fixture_manifest(root, context)
        return context

    def test_matrix_signed_wire_zero_budget_same_original_client_and_strict_eof(self):
        with tempfile.TemporaryDirectory(prefix="ios-install-observer-") as directory:
            root = Path(directory)
            peer = NativeInstallPeer()
            session = regression.Session(root)
            try:
                context = self.context(root, peer)
                case = next(row for row in session.matrix["cases"] if row["id"] == "installcoordination-wire")
                report = regression.execute_device_case(session, case, context)
                self.assertEqual(report["status"], "PASS", report)
                self.assertEqual(report["evidence_origin"], "localhost-protocol-fixture")
                self.assertEqual(report["steps"][0]["observed_wire_type"], "int64")
                self.assertTrue(report["steps"][0]["same_service_connection"])
            finally:
                session.cleanup()
                complete = peer.close()
            self.assertTrue(complete, peer.errors)
            self.assertEqual(len(peer.requests), 1)
            self.assertEqual(len(peer.workers), 2)
            self.assertEqual(peer.eofs, 2)
            self.assertTrue(any(isinstance(body, dict) and body.get("MessageType") == "Handshake"
                                for _, _, body in peer.rsd_messages))

    def test_actual_unsigned_wire_tag_is_preserved_without_satisfying_signed_matrix_row(self):
        peer = NativeInstallPeer(wire_type="uint64")
        context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID))
        try:
            result = capture(adapters.command(context, "apps", "install-record", BUNDLE,
                                               "--observe-zero-budget-recovery", "--rsd-address", peer.address,
                                               "--timeout-secs", "10"), timeout=16)
            self.assertEqual(result.returncode, 0, result.stderr)
            value = json.loads(result.stdout)
            self.assertEqual(value["db_sequence_wire_type"], "uint64")
            self.assertTrue(value["same_service_connection"])
            self.assertTrue(value["cleanup_receipt"]["strict_eof"])
            self.assertEqual(value["cleanup_receipt"]["opened_sockets"], 1)
            self.assertEqual(value["cleanup_receipt"]["closed_sockets"], 1)
            self.assertNotIn(DB_UUID.encode(), result.stdout + result.stderr)
            self.assertNotIn(PERSISTENT, result.stdout + result.stderr)
        finally:
            complete = peer.close()
        self.assertTrue(complete, peer.errors)
        self.assertEqual(len(peer.requests), 1)

    def test_wrong_current_target_and_missing_canonical_service_reject_before_service_io(self):
        for arguments in ({"observed_udid": "00008030-0000000000000002"}, {"missing_service": True}):
            with self.subTest(arguments=arguments):
                peer = NativeInstallPeer(**arguments)
                context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID))
                try:
                    result = capture(adapters.command(context, "apps", "install-record", BUNDLE,
                                                       "--observe-zero-budget-recovery", "--rsd-address", peer.address,
                                                       "--timeout-secs", "10"), timeout=16)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertTrue(result.cleanup_complete)
                    self.assertFalse(result.timed_out)
                finally:
                    complete = peer.close()
                self.assertTrue(complete, peer.errors)
                self.assertFalse(peer.requests)
                self.assertEqual(len(peer.workers), 1)

    def test_negative_signed_reply_preserves_failure_and_original_socket_cleanup(self):
        peer = NativeInstallPeer(sequence=-1)
        context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID))
        try:
            result = capture(adapters.command(context, "apps", "install-record", BUNDLE,
                                               "--observe-zero-budget-recovery", "--rsd-address", peer.address,
                                               "--timeout-secs", "10"), timeout=16)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertTrue(result.cleanup_complete)
            self.assertFalse(result.timed_out)
            self.assertNotIn(DB_UUID.encode(), result.stderr)
        finally:
            complete = peer.close()
        self.assertTrue(complete, peer.errors)
        self.assertEqual(len(peer.requests), 1)


if __name__ == "__main__":
    unittest.main()
