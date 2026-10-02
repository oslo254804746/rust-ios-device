"""Evidence-policy checks: inventory changes cannot invent maturity."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import capabilities


class CapabilityPolicyTests(unittest.TestCase):
    def setUp(self):
        self.catalog = json.loads(capabilities.CATALOG.read_text(encoding="utf-8"))

    def test_missing_public_command_fails(self):
        self.catalog["capabilities"] = [item for item in self.catalog["capabilities"]
                                        if item["id"] != "cli:list"]
        with self.assertRaisesRegex(ValueError, "inventory drift"):
            capabilities.validate(self.catalog)

    def test_host_pass_cannot_make_a_stable_capability(self):
        item = self.catalog["capabilities"][0]
        item["maturity"] = "stable"
        item["compatibility_scope"] = ["synthetic proposed scope"]
        with self.assertRaisesRegex(ValueError, "real-device evidence"):
            capabilities.validate(self.catalog)

    def test_relabeling_host_evidence_as_device_fails(self):
        item = self.catalog["capabilities"][0]
        item["observations"][0]["level"] = "real-device"
        with self.assertRaisesRegex(ValueError, "level mismatch"):
            capabilities.validate(self.catalog)

    def test_unknown_condition_cannot_be_empty_success(self):
        item = self.catalog["capabilities"][0]
        item["observations"][1] = {"level": "simulated-flow", "result": "PASS"}
        with self.assertRaisesRegex(ValueError, "needs evidence"):
            capabilities.validate(self.catalog)

    def test_duplicate_or_invented_evidence_outcomes_fail(self):
        self.catalog["evidence"].append(copy.deepcopy(self.catalog["evidence"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate evidence"):
            capabilities.validate(self.catalog)

    def test_flat_service_with_submodules_keeps_actual_entry_path(self):
        with tempfile.TemporaryDirectory(prefix="ios-inventory-entry-") as directory:
            root = Path(directory)
            cli = root / "crates/ios-cli/src"
            cli.mkdir(parents=True)
            (cli / "main.rs").write_text(
                "enum Commands {\n    List(cmd::list::ListArgs),\n}\n",
                encoding="utf-8",
            )
            services = root / "crates/ios-core/src/services"
            services.mkdir(parents=True)
            (services / "mod.rs").write_text(
                "pub mod flat;\npub mod nested;\n", encoding="utf-8",
            )
            (services / "flat.rs").write_text("mod helper;\n", encoding="utf-8")
            (services / "flat").mkdir()
            (services / "flat/helper.rs").write_text("", encoding="utf-8")
            (services / "nested").mkdir()
            (services / "nested/mod.rs").write_text("", encoding="utf-8")
            with patch.object(capabilities, "ROOT", root):
                commands, actual = capabilities.source_inventory()
            self.assertEqual(commands, {"cli:list": "crates/ios-cli/src/cmd/list.rs"})
            self.assertEqual(actual, {
                "service:flat": "crates/ios-core/src/services/flat.rs",
                "service:nested": "crates/ios-core/src/services/nested/mod.rs",
            })


if __name__ == "__main__":
    unittest.main()
