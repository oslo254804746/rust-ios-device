"""A green Cargo exit cannot stand in for an executed, bounded vector."""

from dataclasses import replace
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

import high_cost
from diagnostics import Capture, capture


class HighCostTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux owned Cargo fixture")
    def test_actual_entry_dispatch_and_capture_with_synthetic_cargo(self):
        with tempfile.TemporaryDirectory(dir=high_cost.ROOT / "target") as directory:
            root = Path(directory)
            proof = self.proof(root)
            program = root / "cargo"
            program.write_text("#!" + sys.executable + "\nimport sys\n"
                               "required = " + repr([high_cost.TEST, "--exact", "--ignored", "--release", "--locked"]) + "\n"
                               "if not all(arg in sys.argv for arg in required): raise SystemExit(17)\n"
                               "sys.stdout.write(" + repr(proof.stdout.decode()) + ")\n")
            program.chmod(0o700)
            env = os.environ.copy()
            env["PATH"] = str(root) + os.pathsep + env["PATH"]
            result = capture([sys.executable, str(high_cost.ROOT / "scripts/validate.py"),
                              "high-cost", "--python", sys.executable], env=env)
            self.assertEqual(result.returncode, 0)
            self.assertTrue(result.cleanup_complete)
            report = json.loads(next(line for line in result.stdout.decode().splitlines()
                                     if line.startswith("{")))
            self.assertTrue(report["passed"])
            self.assertEqual(report["test_counts"]["passed"], 1)
            self.assertFalse(report["real_device_executed"])
            for label in ["stdout", "stderr"]:
                path = high_cost.ROOT / report[label + "_log"]
                self.assertEqual(high_cost.digest(path), report[label + "_sha256"])

    def proof(self, root, *, selected=True, summaries=1):
        binary = root / "synthetic-test-binary"
        binary.write_bytes(b"synthetic compiler artifact")
        message = dict(reason="compiler-artifact", target=dict(name="ios_core", kind=["lib"]),
                       profile=dict(test=True), executable=str(binary))
        text = json.dumps(message) + "\n"
        if selected:
            text += "test " + high_cost.TEST + " ... ok\n"
        text += "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out;\n" * summaries
        return Capture(text.encode(), b"", 0, False, False, False,
                       cleanup_scope="process-group")

    def test_proof_hashes_only_selected_existing_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = high_cost.assess(self.proof(root), root)
            self.assertTrue(report["passed"])
            self.assertEqual(report["artifact"]["sha256"], high_cost.digest(root / "synthetic-test-binary"))

    def test_green_zero_or_wrong_test_and_duplicate_summary_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for result in [Capture(b"test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out;\n", b"", 0, False, False, False, cleanup_scope="process-group"),
                           self.proof(root, selected=False), self.proof(root, summaries=2)]:
                self.assertFalse(high_cost.assess(result, root)["passed"])

    def test_partial_timeout_overflow_stop_or_unjoined_capture_never_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.proof(root)
            for fields in [dict(returncode=1), dict(timed_out=True), dict(overflow=True),
                           dict(incomplete=True), dict(stop_requested=True), dict(cleanup_complete=False),
                           dict(cleanup_scope="leader")]:
                self.assertFalse(high_cost.assess(replace(result, **fields), root)["passed"])

    def test_missing_outside_or_malformed_artifact_does_not_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.proof(root)
            self.assertFalse(high_cost.assess(result, root / "other")["passed"])
            self.assertFalse(high_cost.assess(replace(result, stdout=b"{broken-json\n" + result.stdout), root)["passed"])
            (root / "synthetic-test-binary").unlink()
            self.assertFalse(high_cost.assess(result, root)["passed"])

    def test_invalid_budget_exits_before_cargo(self):
        for budget in ["0", "-1", "nan", "inf", "3601"]:
            result = capture([sys.executable, str(Path(high_cost.__file__).resolve()), "--timeout", budget])
            self.assertEqual(result.returncode, 2)
            self.assertFalse(result.timed_out)
            self.assertFalse(result.overflow)
            self.assertTrue(result.cleanup_complete)


if __name__ == "__main__":
    unittest.main()
