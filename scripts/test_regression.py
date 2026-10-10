"""Observe argv, leaks and cleanup failures without devices or raw archives."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

import diagnostics
import regression


def fixture_provenance():
    return dict(commit="0" * 40, working_tree_dirty=True, binary_sha256="0" * 64,
                binary_version="0.0.0", matrix_sha256="0" * 64, runner_sha256="0" * 64,
                capture_sha256="0" * 64, python_version="3.12.0", platform="linux",
                architecture="x86_64", date="2026-10-01T00:00:00+00:00")


class RegressionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ios-regression-test-")
        self.addCleanup(self.directory.cleanup)
        self.session = regression.Session(self.directory.name)
        self.addCleanup(self.session.cleanup)

    def test_child_arguments_are_observed_and_private_values_never_archived(self):
        secret = b"synthetic-test-credential-12345"
        self.session.private_file("identity", secret)
        args = ["/", ".", "synthetic-missing", "synthetic-路径", secret.decode()]
        command = [sys.executable, "-c", "import json,sys;print(json.dumps(sys.argv[1:]))"] + args
        # Passing a secret intentionally fails the original-output scan even
        # though the child received exactly the intended inputs.
        report = self.session.run("host-argv", command, contract=lambda r: json.loads(r.stdout) == args)
        self.assertTrue(report["contract"])
        self.assertEqual(report["status"], "FAIL")
        self.assertGreater(report["raw_hits"], 0)
        self.assertEqual(report["masked_copy_hits"], 0)
        evidence = self.session.finish(fixture_provenance())
        self.assertFalse(evidence["host_passed"])
        data = (self.session.evidence / "report.json").read_bytes()
        self.assertNotIn(secret, data)
        self.assertNotIn(str(self.session.private).encode(), data)
        self.assertFalse(self.session.private.exists())
        self.assertEqual(len(list(self.session.evidence.iterdir())), 1)

    def test_failure_exit_and_contract_are_separate(self):
        report = self.session.run("host-cli-rejection", [sys.executable, "-c",
            "print('synthetic-rejection');raise SystemExit(17)"], expected_code=17,
            contract=lambda r: r.stdout.replace(b"\r\n", b"\n") == b"synthetic-rejection\n")
        self.assertEqual(report["status"], "PASS")
        self.assertEqual(report["returncode"], 17)
        self.assertGreater(report["duration_ms"], 0)
        self.assertTrue(report["cleanup_complete"])

    def test_contract_exception_cannot_echo_private_payload(self):
        secret = b"synthetic-contract-exception-12345"
        self.session.private_file("identity", secret)

        def broken_contract(result):
            raise RuntimeError(secret.decode())

        report = self.session.run("host-argv", [sys.executable, "-c", "print('safe')"],
                                  contract=broken_contract)
        self.assertEqual(report["status"], "FAIL")
        self.assertNotIn(secret.decode(), json.dumps(report))

    def test_timeout_does_not_satisfy_even_matching_exit_contract(self):
        report = self.session.run("host-argv", [sys.executable, "-c",
            "import time;print('safe',flush=True);time.sleep(20)"], timeout=0.05,
            expected_code=-15, contract=lambda r: b"safe" in r.stdout)
        self.assertEqual(report["status"], "FAIL")
        self.assertTrue(report["timed_out"])
        self.assertTrue(report["cleanup_complete"])

    def test_missing_program_path_is_not_echoed(self):
        path = self.session.private / "synthetic-missing-private-program"
        report = self.session.run("host-argv", [str(path)], contract=lambda r: True)
        self.assertEqual(report["reason"], "process-capture-failed")
        self.assertNotIn(str(path), json.dumps(report))

    def test_unknown_private_file_is_preserved_and_cleanup_fails(self):
        known = self.session.private_file("identity", b"synthetic-private-identity")
        unknown = self.session.private / "unrelated-user-artifact"
        unknown.write_bytes(b"user-data")
        report = self.session.cleanup()
        self.assertFalse(report["complete"])
        self.assertFalse(known.exists())
        self.assertEqual(unknown.read_bytes(), b"user-data")
        self.assertEqual(report["private_unknown"], 1)
        self.assertEqual(report, self.session.cleanup())
        self.assertNotIn(unknown.name, json.dumps(report))

    def test_unknown_evidence_file_is_preserved_and_never_published(self):
        unknown = self.session.evidence / "synthetic-accidental-private-key"
        unknown.write_bytes(b"synthetic-key-material")
        with self.assertRaisesRegex(ValueError, "evidence-not-allowlisted"):
            self.session.finish(fixture_provenance())
        self.assertEqual(unknown.read_bytes(), b"synthetic-key-material")
        self.assertFalse((self.session.evidence / "report.json").exists())
        self.assertFalse(self.session.private.exists())

    @unittest.skipUnless(os.name == "posix", "symlink permission on Unix")
    def test_swapped_private_file_cannot_delete_external_target(self):
        known = self.session.private_file("identity", b"synthetic-owned-identity")
        victim = Path(self.directory.name) / "unrelated"
        victim.write_bytes(b"preserve-me")
        known.unlink()
        known.symlink_to(victim)
        report = self.session.cleanup()
        self.assertFalse(report["complete"])
        self.assertEqual(report["private_unsafe"], 1)
        self.assertTrue(known.is_symlink())
        self.assertEqual(victim.read_bytes(), b"preserve-me")

    def test_output_parent_inside_checkout_rejected_before_creation(self):
        with self.assertRaisesRegex(ValueError, "evidence-inside-checkout"):
            regression.Session(regression.ROOT)

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux owned group observation")
    def test_successful_parent_cannot_leave_closed_pipe_descendant_or_kill_other_session(self):
        unrelated = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(20)"],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                     start_new_session=True)
        try:
            result = diagnostics.capture([sys.executable, "-c",
                "import os,time;pid=os.fork();\n"
                "if pid == 0:\n os.close(1);os.close(2);time.sleep(20)\n"
                "else: print('safe',flush=True)"], timeout=2)
            self.assertEqual(result.returncode, 0)
            self.assertFalse(result.timed_out)
            self.assertTrue(result.cleanup_complete)
            self.assertIs(diagnostics.group_alive(result.pid), False)
            self.assertIsNone(unrelated.poll())
        finally:
            unrelated.terminate()
            unrelated.wait(timeout=3)

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux owned fixture agent")
    def test_agent_cleanup_output_uses_same_raw_detector(self):
        secret = b"synthetic-agent-cleanup-secret-12345"
        self.session.private_file("identity", secret)
        ready = self.session.private_file("agent-ready", b"pending-agent-ready")
        command = [sys.executable, "-c",
            "import pathlib,signal,sys,time;"
            "signal.signal(signal.SIGTERM,lambda *_: (print(sys.argv[2],file=sys.stderr,flush=True),sys.exit(0)));"
            "print('safe',flush=True);pathlib.Path(sys.argv[1]).write_text('ready');time.sleep(20)",
            str(ready), secret.decode()]
        owned = self.session.agent(command)
        deadline = time.monotonic() + 2
        while ready.read_bytes() != b"ready" and not owned[3].done() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(ready.read_bytes(), b"ready")
        evidence = self.session.finish(fixture_provenance())
        report = self.session.results["host-agent"]
        self.assertGreater(report["raw_hits"], 0)
        self.assertEqual(report["masked_copy_hits"], 0)
        self.assertEqual(report["status"], "FAIL")
        self.assertTrue(report["cleanup_complete"])
        self.assertFalse(evidence["host_passed"])
        self.assertNotIn(secret, (self.session.evidence / "report.json").read_bytes())

    def test_real_device_matrix_has_independent_pending_conditions(self):
        evidence = self.session.finish(fixture_provenance())
        rows = {row["id"]: row for row in evidence["cases"]}
        for row in rows.values():
            self.assertEqual(row["result"]["status"], "NOT_RUN")
        self.assertIn("authorized-unc-share", rows["windows-afc-unc"]["requires"])
        self.assertIn("protocol-evidence", rows["cryptex-install"]["requires"])
        self.assertEqual(rows["remoted-coexistence"]["platform"], "macOS")
        self.assertFalse(evidence["real_device_executed"])
        self.assertFalse(evidence["host_passed"])

    def test_arbitrary_provenance_payload_is_rejected_and_private_files_cleaned(self):
        provenance = fixture_provenance()
        provenance["unexpected"] = "synthetic-private-material"
        with self.assertRaisesRegex(ValueError, "provenance-schema"):
            self.session.finish(provenance)
        self.assertFalse(self.session.private.exists())
        self.assertFalse((self.session.evidence / "report.json").exists())

    def test_replaced_registered_regular_file_is_preserved(self):
        known = self.session.private_file("identity", b"synthetic-owned-identity")
        replacement = self.session.private / "replacement"
        replacement.write_bytes(b"unrelated-user-content")
        replacement.replace(known)
        report = self.session.cleanup()
        self.assertFalse(report["complete"])
        self.assertEqual(report["private_unsafe"], 1)
        self.assertEqual(known.read_bytes(), b"unrelated-user-content")

    def test_closed_session_cannot_execute_or_create_new_private_material(self):
        self.session.cleanup()
        with self.assertRaisesRegex(ValueError, "session-closed"):
            self.session.run("host-argv", [sys.executable, "-c", "print('safe')"], contract=lambda r: True)
        with self.assertRaisesRegex(ValueError, "session-closed"):
            self.session.private_file("new-identity", b"synthetic-private-identity")

    def test_unaccepted_case_is_rejected_before_command_side_effect(self):
        marker = Path(self.directory.name) / "must-not-exist"
        command = [sys.executable, "-c", "import pathlib,sys;pathlib.Path(sys.argv[1]).touch()", str(marker)]
        with self.assertRaisesRegex(ValueError, "case-id"):
            self.session.run("traditional-usb", command, contract=lambda r: True)
        self.assertFalse(marker.exists())

    def test_cli_parse_failure_does_not_echo_unknown_argument(self):
        secret = b"synthetic-unrecognized-private-argument-12345"
        result = diagnostics.capture([sys.executable, str(regression.ROOT / "scripts/regression.py"),
                                      "--synthetic-unknown", secret.decode()])
        report = diagnostics.assess(result, diagnostics.secret_variants(secret), 1)
        self.assertTrue(report["passed"])
        self.assertEqual(json.loads(result.stderr)["reason"], "regression-run-failed")

    def test_device_udid_validation(self):
        valid_40 = "0123456789abcdef0123456789abcdef01234567"
        valid_24 = "0123456789abcdef01234567"
        valid_dash = "00008030-00123456789abcde"
        self.assertEqual(regression.validate_udid(valid_40), valid_40)
        self.assertEqual(regression.validate_udid(valid_24), valid_24)
        self.assertEqual(regression.validate_udid(valid_dash), valid_dash)
        for invalid in ["short", "with spaces in it", "../path/traversal", "semi;colon", "0" * 45]:
            with self.assertRaises(ValueError):
                regression.validate_udid(invalid)

    def test_precondition_validation_missing_device(self):
        context = regression.ExecutionContext(binary=Path("dummy"))
        case = next(c for c in self.session.matrix["cases"] if c["id"] == "traditional-usb")
        satisfied, reason, missing = regression.check_preconditions(case, context)
        self.assertFalse(satisfied)
        self.assertIn("selected-device", missing)

    def test_precondition_validation_platform_mismatch(self):
        device = regression.DeviceTarget(udid="0123456789abcdef0123456789abcdef01234567")
        context = regression.ExecutionContext(binary=Path("dummy"), device=device)
        if sys.platform != "darwin":
            case = next(c for c in self.session.matrix["cases"] if c["id"] == "remoted-coexistence")
            satisfied, reason, missing = regression.check_preconditions(case, context)
            self.assertFalse(satisfied)
            self.assertEqual(reason, "macos-host-required")

    def test_precondition_validation_scope_gated(self):
        device = regression.DeviceTarget(udid="0123456789abcdef0123456789abcdef01234567")
        context = regression.ExecutionContext(
            binary=Path("dummy"),
            device=device,
            scopes=frozenset(("read-scope", "binary-provenance")),
            fixtures={"approved-ddi": Path("ddi.dmg")},
            evidence={"protocol-evidence": True}
        )
        case = next(c for c in self.session.matrix["cases"] if c["id"] == "cryptex-install")
        satisfied, reason, missing = regression.check_preconditions(case, context)
        self.assertFalse(satisfied)
        self.assertIn("install-scope", missing)

    def test_device_adapter_execution_records_pass_and_masks_udid(self):
        udid = "0123456789abcdef0123456789abcdef01234567"
        device = regression.DeviceTarget(udid=udid)
        command = [sys.executable, "-c", "import sys;print('device-info-mock');sys.exit(0)"]
        report = self.session.run_device_case("traditional-usb", command,
                                              contract=lambda r: b"device-info-mock" in r.stdout)
        self.assertEqual(report["status"], "PASS")
        self.assertTrue(report["contract"])
        self.assertEqual(report["id"], "traditional-usb")
        evidence = self.session.finish(fixture_provenance(), device=device)
        self.assertFalse(evidence["real_device_executed"])
        data = (self.session.evidence / "report.json").read_bytes()
        self.assertNotIn(udid.encode(), data)

    def test_device_adapter_records_not_run_when_prerequisites_missing(self):
        device = regression.DeviceTarget(udid="0123456789abcdef0123456789abcdef01234567")
        context = regression.ExecutionContext(
            binary=Path("dummy"),
            device=device,
            scopes=frozenset(("read-scope", "binary-provenance"))
        )
        case = next(c for c in self.session.matrix["cases"] if c["id"] == "cryptex-install")
        res = regression.execute_device_case(self.session, case, context)
        self.assertEqual(res["status"], "NOT_RUN")
        self.assertIn("missing_requirements", res)
        self.assertIn("install-scope", res["missing_requirements"])


if __name__ == "__main__":
    unittest.main()

