"""Reject empty, incomplete or unsafe evidence from the linked C error fixture."""

from dataclasses import replace
import unittest

from diagnostics import Capture
from ffi_contracts import SECRET, assess_owned_error


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


if __name__ == "__main__":
    unittest.main()
