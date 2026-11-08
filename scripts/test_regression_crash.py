"""Crash collection through the actual CLI + original TLSmux/DTX/AFC peers.

This is a localhost protocol fixture, never a physical XCTest/device PASS.
"""
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
from regression_cli_fixture import UDID
from regression_dtx_fixture import CliWorkflowPeer, APP_ID, RUNNER_ID, LAUNCH

CLI = Path(os.environ.get("IOS_REGRESSION_CLI", regression.ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios")))


@unittest.skipUnless(sys.platform == "win32" and CLI.is_file(), "Windows isolated existing trust and a compiled native CLI required")
class ActualCrashWorkflowTests(unittest.TestCase):
    def run_case(self, *, crash_fault=None, fault=None, enabled=True, rerun=False, junit_directory=False,
                 entry="runxctest"):
        with tempfile.TemporaryDirectory(prefix="ios-native-crash-workflow-") as directory:
            root = Path(directory)
            peer = CliWorkflowPeer(root / "trust", runner_preexisting=False, crash_fault=crash_fault,
                                   fault=fault, first_test_fails=rerun)
            packages = []
            for name, bundle in (("wrong-name.ipa", APP_ID), ("wrong-runner-name.ipa", RUNNER_ID)):
                path = root / name
                with zipfile.ZipFile(path, "w") as archive:
                    archive.writestr("Payload/Synthetic.app/Info.plist", plistlib.dumps({"CFBundleIdentifier": bundle,
                                                                                       "CFBundleExecutable": "SyntheticRunner"}))
                packages.append(path)
            crash_root = root / "crashes"
            crash_root.mkdir()
            marker = crash_root / "preserved.txt"
            marker.write_bytes(b"previous invocation artifacts")
            xml = root / "results.xml"
            if junit_directory:
                xml.mkdir()
                (xml / "preserved.txt").write_bytes(b"preserve report directory")
            context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID), environment=peer.environment)
            arguments = ["runxctest", "--bundle-id", APP_ID, "--test-runner-bundle-id", RUNNER_ID,
                         "--xctest-config", "MatrixTests.xctest", "--xctest", "--wait", "--timeout", "8",
                         "--test", "MatrixTests/testExample"]
            if entry == "runtest":
                xctestrun = root / "MatrixTests.xctestrun"
                xctestrun.write_bytes(plistlib.dumps({"__xctestrun_metadata__": {"FormatVersion": 2},
                    "TestConfigurations": [{"Name": "Matrix", "TestTargets": [{
                        "TestHostBundleIdentifier": RUNNER_ID, "TestBundlePath": "MatrixTests.xctest",
                        "IsUITestBundle": True, "OnlyTestIdentifiers": ["MatrixTests/testExample"],
                        "UITargetAppEnvironmentVariables": {"UITargetAppBundleIdentifier": APP_ID},
                    }]}]}))
                arguments = ["runtest", xctestrun, "--wait", "--startup-timeout-secs", "4",
                             "--result-timeout-secs", "4"]
            arguments += ["--install-app", packages[0], "--install-runner", packages[1],
                         "--uninstall-on-finish", "--junit-output", xml]
            if enabled:
                arguments += ["--crash-output", crash_root]
            if rerun:
                arguments += ["--rerun-failed", "1"]
            try:
                result = capture(adapters.command(context, *arguments), env=peer.environment, timeout=30, max_bytes=65536)
                self.assertFalse(result.timed_out, result.stderr)
                self.assertTrue(result.cleanup_complete)
                self.assertFalse(result.overflow)
                self.assertFalse(result.incomplete)
                outputs = list(crash_root.glob("workflow-crash-*"))
                self.assertEqual(len(outputs), int(enabled))
                summary = None
                payloads = {}
                if outputs:
                    self.assertTrue((outputs[0] / "summary.json").is_file(), result.stderr)
                    summary = json.loads((outputs[0] / "summary.json").read_bytes())
                    for item in summary["artifacts"]:
                        path = outputs[0] / item["file"]
                        self.assertEqual(path.parent, outputs[0])
                        self.assertTrue(path.is_file())
                        data = path.read_bytes()
                        self.assertEqual(item["bytes"], len(data))
                        self.assertEqual(item["sha256"], hashlib.sha256(data).hexdigest())
                        payloads[item["file"]] = data
                    self.assertEqual(set(path.name for path in outputs[0].iterdir()), {"summary.json"} | set(payloads))
                self.assertEqual(marker.read_bytes(), b"previous invocation artifacts")
                self.assertFalse(list(root.rglob("*.tmp")))
                if junit_directory:
                    self.assertEqual((xml / "preserved.txt").read_bytes(), b"preserve report directory")
                history = json.loads(Path(str(xml) + ".attempts.json").read_bytes())
            finally:
                complete = peer.close()
            self.assertTrue(complete, peer.errors)
            self.assertFalse(peer.pids)
            if crash_fault not in ("mover-before", "list-before"):
                self.assertEqual(peer.calls.count("Uninstall"), 2)
                self.assertNotIn(APP_ID, peer.apps)
                self.assertNotIn(RUNNER_ID, peer.apps)
            else:
                self.assertNotIn(LAUNCH, peer.calls)
                self.assertNotIn("InstallTransferred", peer.calls)
            return result, peer, summary, payloads, history

    def test_new_report_requires_embedded_exact_bundle_and_real_eof_hash(self):
        result, peer, summary, payloads, _ = self.run_case()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(summary["status"], "complete")
        self.assertEqual(summary["baseline_entries"], 1)
        self.assertEqual(summary["new_candidates"], 3)
        self.assertEqual(summary["unmatched"], 2)
        self.assertEqual(len(payloads), 1)
        self.assertEqual(summary["artifacts"][0]["bundle_id"], APP_ID)
        opened = [event[2] for event in peer.crash_events if event[0] == "open"]
        self.assertEqual(set(opened), {"matched.ips", "unrelated.ips", "unknown.ips"})
        self.assertNotIn("existing.ips", opened)
        self.assertEqual([event[2] for event in peer.crash_events if event[0] == "close"], opened)

    def test_unrequested_crash_collection_has_no_service_or_file_io(self):
        result, peer, summary, _, _ = self.run_case(enabled=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIsNone(summary)
        self.assertFalse(peer.crash_events)

    def test_baseline_failure_stops_before_package_side_effects(self):
        for fault in ("mover-before", "list-before"):
            with self.subTest(fault=fault):
                result, _, summary, payloads, history = self.run_case(crash_fault=fault)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(summary["status"], "baseline_incomplete")
                self.assertFalse(payloads)
                self.assertIsNotNone(history["terminal_error_class"])

    def test_final_crash_failure_and_bounds_preserve_original_workflow_cleanup(self):
        for fault in ("list-after", "read", "partial-read", "close", "oversize", "growth", "growth-close", "count", "unsafe"):
            with self.subTest(fault=fault):
                result, _, summary, _, history = self.run_case(crash_fault=fault)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(summary["status"], "incomplete")
                self.assertIsNotNone(summary["error_class"])
                if fault in ("oversize", "growth", "growth-close", "count"):
                    self.assertEqual(summary["error_class"], "resource_limit")
                self.assertIsNotNone(history["terminal_error_class"])

    def test_failed_test_start_and_report_write_still_collect_crashes(self):
        for parameters in (dict(fault="session"), dict(junit_directory=True)):
            with self.subTest(parameters=parameters):
                result, _, summary, payloads, history = self.run_case(**parameters)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(summary["status"], "complete")
                self.assertEqual(len(payloads), 1)
                self.assertIsNotNone(history["terminal_error_class"])

    def test_rerun_collects_once_and_keeps_both_attempts(self):
        result, peer, summary, payloads, history = self.run_case(rerun=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(peer.crash_copies, 2)
        self.assertEqual(peer.crash_movers, 2)
        self.assertEqual(len(payloads), 1)
        self.assertEqual(summary["status"], "complete")
        self.assertEqual([attempt["state"] for attempt in history["attempts"]], ["failed_tests", "completed"])

    def test_final_receipt_agrees_with_exit_and_sidecar_after_actual_crash_failure(self):
        for entry in ("runxctest", "runtest"):
            with self.subTest(entry=entry):
                result, peer, summary, _, history = self.run_case(entry=entry, crash_fault="list-after")
                self.assertEqual(result.returncode, 1, result.stderr)
                body = json.loads(result.stdout)
                self.assertEqual(body["status"], "finished")
                self.assertEqual(body["outcome"], "failed")
                self.assertEqual(body["test_outcome"], "passed")
                self.assertEqual(body["attempts"], history)
                self.assertEqual(body["terminal_error_class"], history["terminal_error_class"])
                self.assertIsNotNone(body["terminal_error_class"])
                self.assertEqual(history["attempts"][0]["state"], "completed")
                self.assertEqual(summary["status"], "incomplete")
                self.assertEqual(peer.calls.count("killPid:"), 1)


if __name__ == "__main__":
    unittest.main()
