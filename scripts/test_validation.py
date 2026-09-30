"""Failure-path checks for the release gate and shared runner."""
import json
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest.mock import patch

import artifacts
import validate


class ValidationTests(unittest.TestCase):
    def test_every_publisher_waits_for_required_gates(self):
        workflow = (validate.ROOT / ".github/workflows/ci.yml").read_text()
        jobs = {}
        # Restrict the maintained workflow to literal needs lists. actionlint
        # validates the full YAML and expression syntax separately.
        for match in re.finditer(r"(?m)^  ([a-z][a-z-]+):\n(.*?)(?=^  [a-z][a-z-]+:|\Z)",
                                 workflow, re.S):
            jobs[match[1]] = match[2]
        def ancestors(name):
            needs = re.search(r"(?m)^    needs: \[([^\]]+)\]$", jobs[name])
            direct = {n.strip() for n in needs[1].split(",")} if needs else set()
            return direct | set().union(*(ancestors(n) for n in direct))
        required = {"validate-tag", "check", "feature-check", "msrv", "build-and-test",
                    "python-smoke", "native-artifacts", "python-wheels", "python-sdist"}
        for publisher in ("release", "publish-pypi", "publish-crates"):
            self.assertTrue(required <= ancestors(publisher), publisher)
            self.assertNotIn("always()", jobs[publisher])
        for gate in required:
            self.assertNotIn("continue-on-error", jobs[gate])

    def test_command_failure_is_not_success(self):
        with self.assertRaisesRegex(RuntimeError, "exit 17"):
            validate.run("synthetic failure", [sys.executable, "-c", "raise SystemExit(17)"])

    def test_missing_tool_is_not_skipped(self):
        with self.assertRaises(OSError):
            validate.run("missing prerequisite", ["ios-synthetic-missing-program"])

    def test_corrupt_checksum_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            file = Path(temporary) / "fixture.tar.gz"
            file.write_bytes(b"synthetic archive")
            artifacts.seal(file)
            artifacts.verify_seal(file)
            file.write_bytes(b"changed artifact")
            with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
                artifacts.verify_seal(file)

    def test_self_consistent_missing_library_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            manifest = directory / "native.json"
            manifest.write_text(json.dumps({"version": "0.0.0", "target": "test-linux",
                "artifacts": [{"file": "ios-0.0.0-test-linux.tar.gz", "members": ["ios"]}]}))
            with patch.object(artifacts, "version", return_value="0.0.0"):
                with self.assertRaisesRegex(RuntimeError, "count mismatch"):
                    artifacts.verify_native(directory, manifest)


if __name__ == "__main__":
    unittest.main()
