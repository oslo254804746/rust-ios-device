"""Failure-path checks for the release gate and shared runner."""
import json
import io
from pathlib import Path
import re
import sys
import tempfile
import tarfile
import unittest
from unittest.mock import patch
import zipfile

import artifacts
import validate


class ValidationTests(unittest.TestCase):
    def python_artifact_fixture(self, directory):
        metadata = "Name: rust-ios-device-tunnel\nVersion: 0.0.0\nRequires-Python: >=3.9\n"
        with zipfile.ZipFile(directory / "fixture.whl", "w") as archive:
            archive.writestr("fixture.dist-info/METADATA", metadata)
            archive.writestr("fixture.dist-info/WHEEL", "Tag: cp39-abi3-linux_x86_64\n")
        with tarfile.open(directory / "fixture.tar.gz", "w:gz") as archive:
            for name in ("PKG-INFO", "Cargo.lock", "crates/ios-core/Cargo.toml",
                         "crates/ios-py/Cargo.toml"):
                data = (metadata if name == "PKG-INFO" else "fixture").encode()
                member = tarfile.TarInfo("fixture/" + name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def check_python_cache_cleanup(self, fail_label=None):
        caches, labels = [], []
        def install(label, command):
            labels.append(label)
            if label in ("wheel install", "sdist build/install"):
                cache = Path(command[command.index("--cache-dir") + 1])
                self.assertTrue(cache.parent.is_dir())
                self.assertNotEqual(str(cache), "/inherited-global-cache")
                cache.mkdir(exist_ok=True)
                (cache / "retained-rust-build").write_bytes(b"cached build")
                caches.append(cache)
            if label == fail_label:
                raise RuntimeError("synthetic artifact failure")
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.python_artifact_fixture(directory)
            with patch.object(artifacts, "version", return_value="0.0.0"), \
                    patch.object(artifacts, "run", side_effect=install), \
                    patch.dict("os.environ", {"UV_CACHE_DIR": "/inherited-global-cache"}):
                if fail_label:
                    with self.assertRaisesRegex(RuntimeError, "synthetic artifact failure"):
                        artifacts.python_artifacts(directory, sys.executable, build_sdist=True)
                else:
                    artifacts.python_artifacts(directory, sys.executable, build_sdist=True)
                    artifacts.verify_seal(directory / "fixture.whl")
                    artifacts.verify_seal(directory / "fixture.tar.gz")
        self.assertEqual(len(caches), 2)
        self.assertEqual(caches[0], caches[1])
        self.assertFalse(caches[0].parent.exists(), "validation left its build cache behind")
        return labels

    def test_python_package_discards_per_run_build_cache(self):
        self.assertIn("installed sdist API", self.check_python_cache_cleanup())

    def test_python_package_discards_build_cache_on_failure(self):
        for label in ("sdist build/install", "installed sdist API"):
            with self.subTest(label=label):
                self.check_python_cache_cleanup(fail_label=label)

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
