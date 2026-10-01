"""Backup fixture failures must preserve unrelated data and never become PASS."""

import json
from pathlib import Path
import sys
import tempfile
import unittest

import backup_contracts
from diagnostics import capture


class BackupContractTests(unittest.TestCase):
    def fixture(self, parent):
        root = Path(parent) / "owned-root"
        root.mkdir(mode=0o700)
        directory = root / backup_contracts.SOURCE
        directory.mkdir(mode=0o700)
        file = directory / "owned-file"
        file.write_bytes(b"synthetic owned file")
        return root, backup_contracts.identity(root), backup_contracts.identity(directory), {
            file.name: backup_contracts.identity(file)}

    def test_unknown_root_entry_preserves_everything_and_fails_cleanup(self):
        with tempfile.TemporaryDirectory() as parent:
            root, root_id, directory_id, known = self.fixture(parent)
            unrelated = root / "unrelated-user-data"
            unrelated.write_bytes(b"user data")
            self.assertFalse(backup_contracts.cleanup(root, root_id, directory_id, known))
            self.assertEqual(unrelated.read_bytes(), b"user data")
            self.assertTrue((root / backup_contracts.SOURCE / "owned-file").is_file())

    def test_replaced_file_preserves_both_original_and_replacement(self):
        with tempfile.TemporaryDirectory() as parent:
            root, root_id, directory_id, known = self.fixture(parent)
            file = root / backup_contracts.SOURCE / "owned-file"
            original = Path(parent) / "original"
            file.rename(original)
            file.write_bytes(b"replacement user data")
            self.assertFalse(backup_contracts.cleanup(root, root_id, directory_id, known))
            self.assertEqual(file.read_bytes(), b"replacement user data")
            self.assertEqual(original.read_bytes(), b"synthetic owned file")

    def test_replaced_directory_is_never_removed(self):
        with tempfile.TemporaryDirectory() as parent:
            root, root_id, directory_id, known = self.fixture(parent)
            directory = root / backup_contracts.SOURCE
            original = Path(parent) / "original-directory"
            directory.rename(original)
            directory.mkdir()
            self.assertFalse(backup_contracts.cleanup(root, root_id, directory_id, known))
            self.assertTrue(directory.is_dir())
            self.assertTrue((original / "owned-file").is_file())

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux owned process group")
    def test_missing_binary_is_failure_with_private_path_omitted_and_fixture_cleaned(self):
        with tempfile.TemporaryDirectory() as parent:
            missing = str(Path(parent) / "synthetic-private-missing-binary")
            result = capture([sys.executable, str(Path(backup_contracts.__file__).resolve()),
                              "--binary", missing])
            self.assertEqual(result.returncode, 1)
            self.assertNotIn(missing.encode(), result.stdout + result.stderr)
            report = json.loads(result.stdout)
            self.assertFalse(report["passed"])
            self.assertTrue(report["cleanup_complete"])
            self.assertFalse(report["real_device_executed"])
            self.assertFalse(report["cases"])


if __name__ == "__main__":
    unittest.main()
