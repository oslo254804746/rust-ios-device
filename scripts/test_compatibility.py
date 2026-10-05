"""Closed-output acceptance must reject success, panic and missing binaries."""

from pathlib import Path
import json
import sys
import tempfile
import unittest

import compatibility
from diagnostics import Capture


class MachineErrorContracts(unittest.TestCase):
    def result(self, *, returncode=2, stdout=b"", prefix=b"", suffix=b"\n", **fields):
        error = dict(error_class="invalid_input", phase="argument_parsing", exit_code=2,
                     effects="not_started", confirmation_required=False, diagnostic="argument error:synthetic")
        error["class"] = error.pop("error_class")
        error.update(fields)
        return Capture(stdout=stdout, stderr=prefix + json.dumps({"schema_version": 1, "error": error}).encode() + suffix,
                       returncode=returncode, timed_out=False, overflow=False, incomplete=False)

    def accepted(self, result):
        return compatibility.machine_error_contract(result, error_class="invalid_input",
                                                    phase="argument_parsing", exit_code=2, effects="not_started")

    def test_expected_envelope_is_accepted(self):
        self.assertTrue(self.accepted(self.result()))

    def test_success_or_wrong_embedded_status_is_rejected(self):
        self.assertFalse(self.accepted(self.result(returncode=0)))
        self.assertFalse(self.accepted(self.result(exit_code=1)))
        self.assertFalse(self.accepted(self.result(exit_code=True)))

    def test_mixed_output_or_unterminated_error_is_rejected(self):
        self.assertFalse(self.accepted(self.result(stdout=b"progress\n")))
        self.assertFalse(self.accepted(self.result(prefix=b"diagnostic text\n")))
        self.assertFalse(self.accepted(self.result(suffix=b"")))

    def test_wrong_class_or_untyped_confirmation_is_rejected(self):
        self.assertFalse(self.accepted(self.result(**{"class": "timeout"})))
        self.assertFalse(self.accepted(self.result(confirmation_required=0)))
        self.assertFalse(self.accepted(self.result(diagnostic=None)))

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux owned process group")
    def test_capture_rejects_private_values_even_in_a_valid_json_envelope(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "owned-machine-error-fixture"
            body = ("import json, sys\n"
                    "print(json.dumps({'schema_version':1, 'error':{'class':'invalid_input', "
                    "'phase':'argument_parsing', 'exit_code':2, 'effects':'not_started', "
                    "'confirmation_required':False, 'diagnostic':'error:' + sys.argv[-2]}}), file=sys.stderr)\n"
                    "sys.exit(2)\n")
            binary.write_text("#!" + sys.executable + "\n" + body)
            binary.chmod(0o700)
            report = compatibility.cli_argument_error(binary, machine=True)
            self.assertTrue(report["contract"])
            self.assertFalse(report["passed"])
            self.assertGreater(report["raw_hits"], 0)
            self.assertEqual(report["device_connections"], 0)
            self.assertTrue(report["cleanup_complete"])


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux owned process group")
class ClosedOutputContracts(unittest.TestCase):
    def probe(self, body):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "owned-synthetic-cli"
            # Only this owned fixture is executable; no device/service paths.
            binary.write_text("#!" + sys.executable + "\n" + body)
            binary.chmod(0o700)
            return compatibility.cli_help_closed_output(binary, "help")

    def test_silent_success_is_not_a_closed_output_pass(self):
        report = self.probe("import sys\nsys.exit(0)\n")
        self.assertFalse(report["passed"])
        self.assertFalse(report["contract"])

    def test_panic_diagnostic_fails_even_with_expected_status_and_io_kind(self):
        report = self.probe("import sys\nsys.stderr.write('io/BrokenPipe panicked\\n')\nsys.exit(1)\n")
        self.assertFalse(report["passed"])
        self.assertFalse(report["contract"])

    def test_missing_binary_fails_without_a_private_path_in_the_report(self):
        with tempfile.TemporaryDirectory() as directory:
            private = Path(directory) / "synthetic-private-missing-cli"
            report = compatibility.cli_help_closed_output(private, "version")
            self.assertFalse(report["passed"])
            self.assertNotIn(str(private), str(report))
            self.assertTrue(report["cleanup_complete"])


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux owned process group")
class NotificationAdmissionContracts(unittest.TestCase):
    family = "notify"
    def probe(self, prefix="", diagnostic="'error:synthetic'"):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "owned-notification-admission-fixture"
            body = ("import json, os, socket, sys\n" + prefix +
                    "print(json.dumps({'schema_version':1, 'error':{'class':'invalid_input', "
                    "'phase':" + repr(self.family) + ", 'exit_code':1, 'effects':'unknown', "
                    "'confirmation_required':False, 'diagnostic':" + diagnostic + "}}), file=sys.stderr)\n"
                    "sys.exit(1)\n")
            binary.write_text("#!" + sys.executable + "\n" + body)
            binary.chmod(0o700)
            return compatibility.cli_notification_deadline_error(binary, "listen" if self.family == "companion" else "wait", 0, family=self.family)

    def test_expected_error_without_contact_is_accepted(self):
        report = self.probe()
        self.assertTrue(report["passed"])
        self.assertEqual(report["device_connections"], 0)
        self.assertTrue(report["cleanup_complete"])

    def test_device_contact_fails_even_with_a_valid_error(self):
        report = self.probe("address = os.environ['USBMUXD_SOCKET_ADDRESS'].removeprefix('tcp://')\n"
                            "host, port = address.rsplit(':', 1)\n"
                            "with socket.create_connection((host, int(port)), timeout=1): pass\n")
        self.assertFalse(report["passed"])
        self.assertFalse(report["contract"])
        self.assertEqual(report["device_connections"], 1)
        self.assertTrue(report["cleanup_complete"])

    def test_private_diagnostic_fails_even_when_the_error_contract_matches(self):
        report = self.probe(diagnostic="'error:' + os.environ['IOS_UDID']")
        self.assertTrue(report["contract"])
        self.assertFalse(report["passed"])
        self.assertGreater(report["raw_hits"], 0)
        self.assertEqual(report["device_connections"], 0)
        self.assertTrue(report["cleanup_complete"])


class CompanionAdmissionContracts(NotificationAdmissionContracts):
    family = "companion"


if __name__ == "__main__":
    unittest.main()
