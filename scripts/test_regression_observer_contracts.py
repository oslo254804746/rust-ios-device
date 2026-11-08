"""Observer admission/negative receipt tests, not native protocol acceptance."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from diagnostics import Capture
import regression
import regression_adapters as adapters
from regression_cli_fixture import UDID


class ObserverContractTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ios-observer-contracts-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.context = regression.ExecutionContext(
            binary=Path("synthetic-cli-unexecuted"), device=regression.DeviceTarget(UDID),
            scopes=frozenset({"display-input-scope", "stop-all-media-scope", "protocol-observation-scope", "synthetic-operation"}),
            observed_services=frozenset({"com.apple.coredevice.displayservice", "com.apple.coredevice.hid"}), fixture_peer=True)
        self.session = regression.Session(self.root)
        self.addCleanup(self.session.cleanup)
        self.status = "9" * 64

    def admit(self, name, record, extra=None):
        path = self.root / (name + ".json")
        path.write_text(json.dumps(record))
        fixtures = {name: {"path": path.name, "sha256": adapters.digest(path)}}
        fixtures.update(extra or {})
        manifest = {"version": 1, "target_udid": UDID, "fixtures": fixtures,
                    "observations": {"services": ["com.apple.coredevice.displayservice", "com.apple.coredevice.hid"]}, "evidence": {}}
        (self.root / "manifest.json").write_text(json.dumps(manifest))
        adapters.load_fixture_manifest(self.root, self.context)

    def display_receipt(self):
        return {"schema_version": 1, "operation": "display_hid_stop_observation", "selected_udid_matched": True,
                "hid_reports_flushed": 2, "local_contact_closed": True, "hid_device_ack": "unavailable-in-protocol",
                "media_stop_reply_received": True, "original_media_tasks_joined": True,
                "status_before_sha256": self.status, "status_after_sha256": self.status,
                "media_status_restored": True, "stop_scope": "explicit-stop-all",
                "original_native_joins_complete": True, "cleanup_complete": True}

    def execute(self, case_id, value):
        case = next(row for row in self.session.matrix["cases"] if row["id"] == case_id)
        result = Capture(json.dumps(value).encode(), b"", 0, False, False, False)
        with patch.object(adapters, "capture", return_value=result) as capture:
            report = regression.execute_device_case(self.session, case, self.context)
        return report, capture

    def test_display_stop_requires_separate_stop_all_scope_before_any_mutation(self):
        self.admit("display-hid-baseline", {"version": 1, "status_sha256": self.status, "scenario": "center-contact-cancel"})
        self.context.scopes = self.context.scopes - {"stop-all-media-scope"}
        report, capture = self.execute("display-hid-stop", self.display_receipt())
        self.assertEqual(report["status"], "NOT_RUN")
        self.assertIn("stop-all-media-scope", report["missing_requirements"])
        capture.assert_not_called()

    def test_display_changed_fixture_is_rejected_before_cli(self):
        self.admit("display-hid-baseline", {"version": 1, "status_sha256": self.status, "scenario": "center-contact-cancel"})
        self.context.fixtures["display-hid-baseline"]["fixture_path"].write_text("{}")
        report, capture = self.execute("display-hid-stop", self.display_receipt())
        self.assertEqual(report["status"], "FAIL")
        capture.assert_not_called()

    def test_display_failed_restoration_and_unjoined_owner_never_pass_even_exit_zero(self):
        self.admit("display-hid-baseline", {"version": 1, "status_sha256": self.status, "scenario": "center-contact-cancel"})
        for field, replacement in (("media_status_restored", False), ("status_after_sha256", "0" * 64),
                                   ("original_native_joins_complete", False), ("original_media_tasks_joined", False),
                                   ("media_stop_reply_received", False), ("local_contact_closed", False),
                                   ("cleanup_complete", False), ("hid_reports_flushed", 1), ("hid_device_ack", True)):
            with self.subTest(field=field):
                receipt = self.display_receipt()
                receipt[field] = replacement
                report, capture = self.execute("display-hid-stop", receipt)
                self.assertEqual(report["status"], "FAIL", report)
                capture.assert_called_once()

    def test_display_exact_one_way_receipt_builds_real_observer_argv(self):
        self.admit("display-hid-baseline", {"version": 1, "status_sha256": self.status, "scenario": "center-contact-cancel"})
        report, capture = self.execute("display-hid-stop", self.display_receipt())
        self.assertEqual(report["status"], "PASS")  # Contract unit, not a native observation.
        arguments = capture.call_args.args[0]
        self.assertEqual(arguments[1:5], ["--udid", UDID, "--error-format", "json"])
        self.assertEqual(arguments[5:], ["hid", "--confirm", "--timeout", "30", "observe-stop",
                                        "--expected-status-sha256", self.status, "--confirm-stop-all"])

    def install_record(self):
        return {"version": 1, "bundle_id": "com.synthetic.Observer", "db_sequence": 9,
                "db_uuid_sha256": hashlib.sha256(b"synthetic-db").hexdigest(),
                "persistent_identifier_size": 3, "persistent_identifier_sha256": hashlib.sha256(b"pid").hexdigest(),
                "wire_type": "int64"}

    def install_receipt(self):
        expected = self.install_record()
        return {"schema_version": 1, "scope": "existing-rsd-sockets", "selected_udid_matched": True,
                "canonical_service_observed": True,
                "zero_budget": {"error_class": "timeout", "no_business_io": True, "client_usable": True, "elapsed_us": 1},
                "same_service_connection": True, "default_budget_unchanged": True,
                "client_usable_after_finite_query": True, "db_sequence_wire_type": "int64", "finite_query_elapsed_us": 100,
                "record": {key: expected[key] for key in ("db_sequence", "db_uuid_sha256", "persistent_identifier_size", "persistent_identifier_sha256")},
                "cleanup_complete": True, "cleanup_scope": "original-sockets-shutdown-and-peer-eof",
                "cleanup_receipt": {"opened_sockets": 1, "closed_sockets": 1, "drained_bytes": 0, "strict_eof": True}}

    def admit_install(self):
        self.admit("install-record-observation", self.install_record(), {"existing-rsd-route": {"address": "[::1]:12345"}})

    def test_unsigned_fixture_cannot_claim_signed_matrix_acceptance(self):
        value = self.install_record()
        value["wire_type"] = "uint64"
        with self.assertRaisesRegex(ValueError, "signed-observation-required"):
            self.admit("install-record-observation", value)

    def test_install_elapsed_alone_does_not_supply_no_business_io_or_same_client_receipt(self):
        self.admit_install()
        for field in ("no_business_io", "client_usable"):
            receipt = self.install_receipt()
            receipt["zero_budget"][field] = False
            report, _ = self.execute("installcoordination-wire", receipt)
            self.assertEqual(report["status"], "FAIL")
        for field, value in (("same_service_connection", False), ("db_sequence_wire_type", "uint64"),
                             ("default_budget_unchanged", False), ("canonical_service_observed", False)):
            receipt = self.install_receipt()
            receipt[field] = value
            report, _ = self.execute("installcoordination-wire", receipt)
            self.assertEqual(report["status"], "FAIL")

    def test_install_real_close_receipt_is_required_beyond_cleanup_boolean(self):
        self.admit_install()
        for change in ({"strict_eof": False}, {"closed_sockets": 0}, {"opened_sockets": 2}):
            receipt = self.install_receipt()
            receipt["cleanup_receipt"].update(change)
            report, _ = self.execute("installcoordination-wire", receipt)
            self.assertEqual(report["status"], "FAIL")

    def test_rsd_actual_bootstrap_current_catalogue_and_local_release_are_distinct(self):
        self.context.observed_modes = frozenset({"direct", "queued", "legacy", "passive"})
        value = {"schema_version": 1, "selected_udid": UDID, "observed_udid": UDID, "handshake_mode": "queued",
                 "attempted_modes": ["direct", "queued"], "transport_scope": "existing-rsd-socket",
                 "services": [{"name": "com.synthetic.Service", "port": 1234, "features": None}],
                 "directory_socket_released": True, "directory_release_evidence": "local-raii", "cleanup_complete": True}
        def valid(value):
            return adapters.rsd_observer_contract(Capture(json.dumps(value).encode(), b"", 0, False, False, False), self.context)
        self.assertTrue(valid(value))
        for field, change in (("observed_udid", "00008030-0000000000000002"), ("handshake_mode", "userspace"),
                              ("attempted_modes", ["queued"]), ("services", None), ("directory_socket_released", False),
                              ("directory_release_evidence", "external-tunnel-joined"), ("cleanup_complete", False)):
            with self.subTest(field=field):
                invalid = dict(value, **{field: change})
                self.assertFalse(valid(invalid))

    def test_private_mac_and_saved_altirk_both_require_independent_complete_receipts(self):
        private = {"mechanism": "private-mac-lockdown", "advertisement_type": "mobdev2", "advertisement_index": 0,
                   "selected_udid_match": True, "saved_mac_matches": False, "record_accepted": True,
                   "unique_device_id_verified": True}
        remote = {"mechanism": "remote-pairing-altirk", "advertisement_type": "remotepairing", "advertisement_index": 0,
                  "selected_udid_match": True, "credential_record_validated": True, "advertisement_auth_matched": True,
                  "credential_validation_scope": "structure-and-key-material"}
        def valid(records):
            value = {"schema_version": 1, "selected_udid": UDID, "transport_scope": "existing-trust-identity-observation", "observations": records}
            return adapters.paired_identity_contract(Capture(json.dumps(value).encode(), b"", 0, False, False, False), self.context)
        self.assertTrue(valid([private, remote]))
        for records in ([], [private], [remote], [dict(private, record_accepted=False), remote],
                        [dict(private, saved_mac_matches=True), remote], [dict(private, unique_device_id_verified=False), remote],
                        [private, dict(remote, advertisement_auth_matched=False)],
                        [private, dict(remote, credential_validation_scope="apple-signature-verified")],
                        [private, dict(remote, record_accepted=True)]):
            with self.subTest(records=records):
                self.assertFalse(valid(records))


if __name__ == "__main__":
    unittest.main()
