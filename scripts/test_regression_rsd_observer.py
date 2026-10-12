"""Actual compiled read-only RSD observer using the original localhost H2 peer."""
import json
import os
from pathlib import Path
import unittest

from diagnostics import capture
import regression
import regression_adapters as adapters
from regression_cli_fixture import UDID
from regression_rsd_fixture import NativeInstallPeer, SERVICE

CLI = Path(os.environ.get("IOS_REGRESSION_CLI", regression.ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios")))


@unittest.skipUnless(CLI.is_file(), "existing native CLI required; never builds")
class ActualRsdObserverTests(unittest.TestCase):
    def test_direct_current_directory_does_not_claim_external_tunnel_or_service_cleanup(self):
        peer = NativeInstallPeer()
        context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID), observed_modes=frozenset({"direct"}))
        try:
            result = capture(adapters.command(context, "rsd", "observe", "--address", peer.address, "--timeout", "10"), timeout=14)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(adapters.rsd_observer_contract(result, context))
            value = json.loads(result.stdout)
            self.assertEqual(value["handshake_mode"], "direct")
            self.assertEqual(value["attempted_modes"], ["direct"])
            self.assertEqual(value["services"][0]["name"], SERVICE)
            self.assertEqual(value["directory_release_evidence"], "local-raii")
        finally:
            complete = peer.close()
        self.assertTrue(complete, peer.errors)
        self.assertEqual(len(peer.workers), 1)
        self.assertEqual(peer.eofs, 1)
        self.assertFalse(peer.requests)

    def test_wrong_current_identity_is_rejected_without_a_service_connection(self):
        peer = NativeInstallPeer(observed_udid="00008030-0000000000000002")
        context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID), observed_modes=frozenset({"direct"}))
        try:
            result = capture(adapters.command(context, "rsd", "observe", "--address", peer.address, "--timeout", "10"), timeout=14)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertTrue(result.cleanup_complete)
            self.assertFalse(result.timed_out)
        finally:
            complete = peer.close()
        self.assertTrue(complete, peer.errors)
        self.assertFalse(peer.requests)
