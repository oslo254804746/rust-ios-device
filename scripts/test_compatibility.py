"""Closed-output acceptance must reject success, panic and missing binaries."""

from pathlib import Path
import sys
import tempfile
import unittest

import compatibility


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


if __name__ == "__main__":
    unittest.main()
