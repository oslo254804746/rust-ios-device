"""Raw leaks cannot pass by masking a copy; capture budgets fail closed."""
import os
import re
import sys
import unittest

import diagnostics


class DiagnosticCaptureTests(unittest.TestCase):
    def test_owned_rust_error_debug_does_not_regress_to_payload_derivation(self):
        violations = []
        for source in (diagnostics.ROOT / "crates/ios-core/src").rglob("*.rs"):
            for derive in re.findall(r"#\[derive\(([^)]*)\)\]", source.read_text(encoding="utf-8")):
                if "thiserror::Error" in derive and "Debug" in derive:
                    violations.append(str(source.relative_to(diagnostics.ROOT)))
        self.assertEqual(violations, [], "owned errors must use a safe Debug view")

    def test_empty_success_cannot_satisfy_diagnostic_contract(self):
        result = diagnostics.Capture(b"", b"", 0, False, False, False)
        self.assertFalse(diagnostics.assess(result, {b"absent-secret"}, 0)["passed"])

    def test_each_secret_representation_is_detected_in_original(self):
        secret = 'synthetic-秘密-token/+/"\\\n'.encode()
        variants = diagnostics.secret_variants(secret)
        for variant in variants:
            with self.subTest(representation_length=len(variant)):
                result = diagnostics.Capture(b"prefix" + variant, b"", 0, False, False, False)
                report = diagnostics.assess(result, variants, 0)
                self.assertGreater(report["raw_hits"], 0)
                self.assertEqual(report["masked_copy_hits"], 0)
                self.assertFalse(report["passed"])
                self.assertNotIn(secret.decode(), str(report))

    def test_stdout_and_stderr_are_checked_without_writing_raw_files(self):
        secret = b"synthetic-private-credential"
        result = diagnostics.capture([sys.executable, "-c",
            "import sys;sys.stdout.write('safe');sys.stderr.write('synthetic-private-credential')"])
        self.assertEqual(result.stdout, b"safe")
        self.assertEqual(result.stderr, secret)
        self.assertFalse(diagnostics.assess(result, diagnostics.secret_variants(secret), 0)["passed"])

    def test_output_budget_stops_flood_and_cannot_pass_truncated_capture(self):
        result = diagnostics.capture([sys.executable, "-c",
            "import os;\nwhile True: os.write(1,b'x'*4096)"], max_bytes=1024, timeout=2)
        self.assertTrue(result.overflow)
        self.assertLessEqual(len(result.stdout) + len(result.stderr), 1024)
        self.assertFalse(diagnostics.assess(result, {b"absent-secret"}, result.returncode)["passed"])

    def test_timeout_stops_owned_process_and_is_not_success(self):
        result = diagnostics.capture([sys.executable, "-c", "import time;time.sleep(20)"], timeout=0.1)
        self.assertTrue(result.timed_out)
        self.assertFalse(result.incomplete)
        self.assertFalse(diagnostics.assess(result, {b"absent-secret"}, result.returncode)["passed"])

    @unittest.skipUnless(os.name == "posix", "Unix owned-session cleanup")
    def test_exited_parent_does_not_leave_inherited_pipe_writers_running(self):
        result = diagnostics.capture([sys.executable, "-c",
            "import os,time;pid=os.fork();\nif pid == 0: time.sleep(20)"], timeout=0.1)
        self.assertTrue(result.timed_out)
        self.assertFalse(result.incomplete)


if __name__ == "__main__":
    unittest.main()
