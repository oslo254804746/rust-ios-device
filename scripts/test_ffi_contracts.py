"""Reject empty, incomplete or unsafe evidence from the linked C error fixture."""

from dataclasses import replace
import unittest

from diagnostics import Capture
from ffi_contracts import ENCODING_SECRET, SECRET, assess_checked_list, assess_owned_error


class FfiContractTests(unittest.TestCase):
    def result(self):
        return Capture(b"C ABI owned error passed\n", b"", 0, False, False, False)

    def test_complete_owned_fixture_can_pass(self):
        self.assertTrue(assess_owned_error(self.result(), True)["passed"])

    def test_empty_wrong_status_or_unexpected_diagnostic_cannot_pass(self):
        for result in (replace(self.result(), stdout=b""),
                       replace(self.result(), returncode=3),
                       replace(self.result(), stdout=b"C ABI smoke passed\n"),
                       replace(self.result(), stderr=b"unexpected diagnostic\n")):
            self.assertFalse(assess_owned_error(result, True)["passed"])

    def test_unsafe_incomplete_or_unconfirmed_peer_cannot_pass(self):
        self.assertFalse(assess_owned_error(self.result(), False)["passed"])
        for result in (replace(self.result(), stdout=SECRET + b"\n"),
                       replace(self.result(), timed_out=True),
                       replace(self.result(), overflow=True),
                       replace(self.result(), incomplete=True),
                       replace(self.result(), cleanup_complete=False)):
            self.assertFalse(assess_owned_error(result, True)["passed"])

    def test_checked_list_requires_its_complete_linked_output_and_peer(self):
        result = replace(self.result(), stdout=b"C ABI checked list passed\n")
        self.assertTrue(assess_checked_list(result, True)["passed"])
        self.assertFalse(assess_checked_list(result, False)["passed"])
        for failure in (self.result(), replace(result, stdout=b""),
                        replace(result, returncode=3),
                        replace(result, stderr=b"unexpected diagnostic\n")):
            self.assertFalse(assess_checked_list(failure, True)["passed"])

    def test_checked_list_rejects_leaks_including_nul_truncated_secret(self):
        result = replace(self.result(), stdout=b"C ABI checked list passed\n")
        for secret in (ENCODING_SECRET, ENCODING_SECRET.split(b"\x00", 1)[0]):
            report = assess_checked_list(replace(result, stderr=secret), True)
            self.assertGreater(report["raw_hits"], 0)
            self.assertFalse(report["passed"])

    def test_checked_list_rejects_unfinished_capture_and_cleanup(self):
        result = replace(self.result(), stdout=b"C ABI checked list passed\n")
        for failure in (replace(result, timed_out=True),
                        replace(result, overflow=True),
                        replace(result, incomplete=True),
                        replace(result, cleanup_complete=False)):
            self.assertFalse(assess_checked_list(failure, True)["passed"])


if __name__ == "__main__":
    unittest.main()
