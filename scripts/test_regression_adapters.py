"""Fixture admission, exact cleanup and actual compiled CLI adapter checks."""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest
import zipfile

from diagnostics import capture
import regression
import regression_adapters as adapters
from regression_cli_fixture import CliAfcPeer, UDID, REMOTE_PATH, PAYLOAD

CLI = Path(os.environ.get("IOS_REGRESSION_CLI", regression.ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios")))


class FixtureAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ios-adapter-admission-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID))

    def manifest(self, fixtures=None, target=UDID):
        value = dict(version=1, target_udid=target, fixtures=fixtures or {}, observations={}, evidence={})
        (self.root / "manifest.json").write_text(json.dumps(value))

    def test_device_metadata_is_unknown_until_observed(self):
        device = self.context.device
        self.assertEqual(device.transport, "unobserved")
        self.assertEqual(device.ios_version, "unobserved")
        self.assertEqual(device.device_state, "unobserved")
        self.assertFalse(device.services)

    def test_fixture_target_does_not_grant_scope(self):
        self.manifest()
        self.context.scopes = frozenset()
        adapters.load_fixture_manifest(self.root, self.context)
        self.assertFalse(self.context.scopes)
        self.manifest(target="00008030-0000000000000002")
        with self.assertRaisesRegex(ValueError, "target-mismatch"):
            adapters.load_fixture_manifest(self.root, self.context)

    def test_package_uses_archive_identifier_and_hash(self):
        path = self.root / "wrong-name.ipa"
        with zipfile.ZipFile(path, "w") as package:
            package.writestr("Payload/Test.app/Info.plist", plistlib.dumps({"CFBundleIdentifier": "com.synthetic.real-id"}))
        fixture = dict(path=path.name, sha256=adapters.digest(path), bundle_id="com.synthetic.real-id")
        self.manifest({"test-application": fixture})
        adapters.load_fixture_manifest(self.root, self.context)
        self.assertEqual(self.context.fixtures["test-application"]["bundle_id"], "com.synthetic.real-id")
        fixture["sha256"] = "0" * 64
        self.manifest({"test-application": fixture})
        with self.assertRaisesRegex(ValueError, "integrity"):
            adapters.load_fixture_manifest(self.root, self.context)

    def test_fixture_manifest_cannot_supply_commands(self):
        self.manifest({"command": {"argv": ["untrusted"]}})
        with self.assertRaisesRegex(ValueError, "fixture-name"):
            adapters.load_fixture_manifest(self.root, self.context)

    def test_unknown_residue_is_preserved_and_fails_cleanup(self):
        owned = adapters.OwnedFiles(self.root)
        destination = owned.destination("nested/output")
        destination.write_bytes(b"owned")
        owned.admit(destination)
        unknown = destination.parent / "unrelated-marker"
        unknown.write_bytes(b"preserve")
        self.assertFalse(owned.clean())
        self.assertFalse(destination.exists())
        self.assertEqual(unknown.read_bytes(), b"preserve")

    def test_complex_cases_have_no_unrelated_query_fallback(self):
        for case_id in adapters.UNSUPPORTED:
            with self.assertRaisesRegex(ValueError, "operation-sequence"):
                adapters.query_command(case_id, self.context)

    @unittest.skipUnless(CLI.is_file(), "existing native CLI binary required; this test never builds")
    def test_actual_cli_accepts_adapter_argv_before_io(self):
        commands = [adapters.query_command(name, self.context)[0] for name in
                    ("traditional-usb", "traditional-wifi", "coredevice-usb", "remoted-coexistence")]
        commands += [adapters.command(self.context, *args) for args in (
            ("file", "pull", REMOTE_PATH, "synthetic-output.bin"),
            ("rsd", "services", "--all"),
            ("file", "pull", REMOTE_PATH, "synthetic-output.bin", "--force"),
            ("apps", "list", "--app-type", "all"), ("apps", "install", "synthetic.ipa", "--streaming"),
            ("apps", "show", "com.synthetic.App"), ("apps", "launch", "com.synthetic.App"),
            ("apps", "kill", "123"), ("apps", "uninstall", "com.synthetic.App"),
            ("runxctest", "--test-runner-bundle-id", "com.synthetic.Runner", "--xctest-config", "Tests.xctest",
             "--xctest", "--wait", "--timeout", "10", "--test", "Tests/testExample", "--junit-output", "out.xml", "--log-output", "out.log"))]
        for command in commands:
            with self.subTest(command=command[5:]):
                result = capture(command + ["--help"], timeout=3)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(result.cleanup_complete)


@unittest.skipUnless(sys.platform == "win32" and CLI.is_file(), "Windows existing-trust path isolation and an existing native CLI required")
class ActualCliAfcTests(unittest.TestCase):
    def test_device_file_adapters_use_actual_cli_and_strict_original_peer_eof(self):
        with tempfile.TemporaryDirectory(prefix="ios-actual-cli-afc-") as directory:
            root = Path(directory)
            peer = CliAfcPeer(root / "trust")
            try:
                context = regression.ExecutionContext(
                    binary=CLI, device=regression.DeviceTarget(UDID, transport="USB"),
                    fixtures={"afc-fixture": dict(remote_path=REMOTE_PATH, size=len(PAYLOAD), sha256=hashlib.sha256(PAYLOAD).hexdigest())},
                    environment=peer.environment, fixture_peer=True)
                session = regression.Session(root)
                try:
                    ids = ("file-workflow", "windows-afc-new", "windows-afc-existing", "windows-afc-force",
                           "windows-afc-unicode", "windows-afc-long", "windows-afc-verbatim", "windows-afc-readonly", "windows-afc-recovery")
                    for case_id in ids:
                        case = next(case for case in session.matrix["cases"] if case["id"] == case_id)
                        report = regression.execute_device_case(session, case, context)
                        self.assertEqual(report["status"], "PASS", report)
                        self.assertEqual(report["evidence_origin"], "localhost-protocol-fixture")
                        self.assertTrue(report["cleanup_complete"])
                    self.assertFalse(list(root.glob("ios-matrix-owned-*")))
                finally:
                    session.cleanup()
            finally:
                complete = peer.close()
            self.assertTrue(complete, peer.errors)
            self.assertTrue({10, 13, 15, 20} <= set(peer.operations))
            self.assertFalse({8, 16} & set(peer.operations))  # Read scope never writes/deletes remotely.


if __name__ == "__main__":
    unittest.main()
