"""Fixture admission, exact cleanup and actual compiled CLI adapter checks."""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from diagnostics import Capture, capture
import regression
import regression_adapters as adapters
from regression_cli_fixture import CliAfcPeer, UDID, REMOTE_PATH, PAYLOAD
from regression_dtx_fixture import CliWorkflowPeer, APP_ID, RUNNER_ID, PID, LAUNCH
from regression_watch_capture import OwnedWatch

CLI = Path(os.environ.get("IOS_REGRESSION_CLI", regression.ROOT / "target/debug" / ("ios.exe" if os.name == "nt" else "ios")))


class FixtureAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ios-adapter-admission-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID))

    def manifest(self, fixtures=None, target=UDID):
        value = dict(version=1, target_udid=target, fixtures=fixtures or {}, observations={}, evidence={})
        (self.root / "manifest.json").write_text(json.dumps(value))

    def test_ordinary_and_cleanup_capture_budgets_and_original_owner_evidence(self):
        session = regression.Session(self.root)
        self.addCleanup(session.cleanup)
        run = adapters.OperationRun(session, "file-workflow", self.context)
        captured = Capture(b"safe", b"", 0, False, False, False, pid=73,
                           cleanup_complete=True, cleanup_scope="windows-job")
        with patch.object(adapters, "capture", return_value=captured) as mocked:
            run.step(["owned-fixture"], lambda result: True)
            self.assertLessEqual(mocked.call_args.kwargs["timeout"], 60)
            self.assertGreater(mocked.call_args.kwargs["timeout"], 59)
            run.step(["owned-cleanup"], lambda result: True, cleanup=True)
            self.assertEqual(mocked.call_args.kwargs["timeout"], 10)
        self.assertEqual(run.steps[0]["pid"], 73)
        self.assertEqual(run.steps[0]["cleanup_scope"], "windows-job")
        self.assertFalse(run.steps[0]["stop_requested"])
        with patch.object(adapters, "capture", side_effect=OSError("synthetic-private-capture-fault")):
            with self.assertRaises(OSError):
                run.step(["owned-fixture"], lambda result: True)
        report = run.finish(OSError("synthetic-private-capture-fault"))
        self.assertFalse(report["cleanup_complete"])
        self.assertEqual(report["status"], "FAIL")
        self.assertNotIn("synthetic-private-capture-fault", json.dumps(report))

    def test_device_metadata_is_unknown_until_observed(self):
        device = self.context.device
        self.assertEqual(device.transport, "unobserved")
        self.assertEqual(device.ios_version, "unobserved")
        self.assertEqual(device.device_state, "unobserved")
        self.assertFalse(device.services)

    def test_fixture_target_does_not_grant_scope(self):
        self.manifest()
        self.context.scopes = frozenset()
        adapters.load_fixture_manifest(self.root, self.context)
        self.assertFalse(self.context.scopes)
        self.manifest(target="00008030-0000000000000002")
        with self.assertRaisesRegex(ValueError, "target-mismatch"):
            adapters.load_fixture_manifest(self.root, self.context)

    def test_package_uses_archive_identifier_and_hash(self):
        path = self.root / "wrong-name.ipa"
        with zipfile.ZipFile(path, "w") as package:
            package.writestr("Payload/Test.app/Info.plist", plistlib.dumps({"CFBundleIdentifier": "com.synthetic.real-id"}))
        fixture = dict(path=path.name, sha256=adapters.digest(path), bundle_id="com.synthetic.real-id")
        self.manifest({"test-application": fixture})
        adapters.load_fixture_manifest(self.root, self.context)
        self.assertEqual(self.context.fixtures["test-application"]["bundle_id"], "com.synthetic.real-id")
        fixture["sha256"] = "0" * 64
        self.manifest({"test-application": fixture})
        with self.assertRaisesRegex(ValueError, "integrity"):
            adapters.load_fixture_manifest(self.root, self.context)

    def test_fixture_manifest_cannot_supply_commands(self):
        self.manifest({"command": {"argv": ["untrusted"]}})
        with self.assertRaisesRegex(ValueError, "fixture-name"):
            adapters.load_fixture_manifest(self.root, self.context)

    def test_unknown_residue_is_preserved_and_fails_cleanup(self):
        owned = adapters.OwnedFiles(self.root)
        destination = owned.destination("nested/output")
        destination.write_bytes(b"owned")
        owned.admit(destination)
        unknown = destination.parent / "unrelated-marker"
        unknown.write_bytes(b"preserve")
        self.assertFalse(owned.clean())
        self.assertFalse(destination.exists())
        self.assertEqual(unknown.read_bytes(), b"preserve")

    def test_complex_cases_have_no_unrelated_query_fallback(self):
        for case_id in ("private-mac-altirk", "modern-rsd-wifi", "display-hid-stop", "installcoordination-wire", "rich-clipboard-watch", "cryptex-install"):
            with self.assertRaisesRegex(ValueError, "operation-sequence"):
                adapters.query_command(case_id, self.context)

    def ddi_fixture(self):
        directory = self.root / "Restore"
        directory.mkdir()
        names = ("Image.dmg", "Image.dmg.trustcache", "Image.dmg.cryptex_info", "Image.dmg.root_hash")
        identity = {"Info": {"Variant": "Synthetic Developer Disk Image Cryptex"}, "Cryptex1,Version": "1.0",
                    "Manifest": {key: {"Info": {"Path": name}} for key, name in zip(adapters.CRYPTEX_PAYLOADS, names)}}
        (directory / "BuildManifest.plist").write_bytes(plistlib.dumps({"ProductBuildVersion": "SyntheticBuild", "BuildIdentities": [identity]}))
        for name in names:
            (directory / name).write_bytes(b"synthetic-ddi-" + name.encode())
        files = [dict(path=name, size=(directory / name).stat().st_size, sha256=adapters.digest(directory / name))
                 for name in ("BuildManifest.plist", *names)]
        item = dict(path="Restore", files=files, identifier=adapters.CRYPTEX_ID, version="1.0", build_id="SyntheticBuild")
        return item

    def test_cryptex_all_manifest_components_are_hash_bound_and_scope_is_separate(self):
        item = self.ddi_fixture()
        self.manifest({"approved-ddi": item})
        self.context.scopes = frozenset()
        adapters.load_fixture_manifest(self.root, self.context)
        self.assertFalse(self.context.scopes)
        self.assertEqual(self.context.fixtures["approved-ddi"]["path"], (self.root / "Restore").resolve())
        (self.root / "Restore/Image.dmg.root_hash").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "integrity"):
            adapters.load_fixture_manifest(self.root, self.context)

    def test_cryptex_rejects_directory_file_and_manifest_identity_mismatch(self):
        item = self.ddi_fixture()
        item["version"] = "wrong-version"
        self.manifest({"approved-ddi": item})
        with self.assertRaisesRegex(ValueError, "manifest-identity"):
            adapters.load_fixture_manifest(self.root, self.context)
        item["version"] = "1.0"
        item["files"][0]["path"] = "../outside"
        (self.root / "outside").write_bytes(b"preserve")
        self.manifest({"approved-ddi": item})
        with self.assertRaisesRegex(ValueError, "outside-root"):
            adapters.load_fixture_manifest(self.root, self.context)

    def cryptex_sequence(self, replies):
        item = self.ddi_fixture()
        self.manifest({"approved-ddi": item})
        adapters.load_fixture_manifest(self.root, self.context)
        self.context.scopes = frozenset({"install-scope"})
        self.context.evidence = {"protocol-evidence": {"path": "synthetic", "sha256": "0" * 64}}
        session = regression.Session(self.root)
        calls = []
        pending = iter(replies)
        def response(argv, **_):
            calls.append(argv[5:])
            code, value = next(pending)
            return Capture(json.dumps(value).encode(), b"synthetic-cli-error" if code else b"", code, False, False, False)
        try:
            case = next(row for row in session.matrix["cases"] if row["id"] == "cryptex-install")
            with patch.object(adapters, "capture", response):
                report = regression.execute_device_case(session, case, self.context)
            with self.assertRaises(StopIteration):
                next(pending)
            return report, calls
        finally:
            session.cleanup()

    def test_cryptex_observer_exact_version_roundtrip_and_retains_other_images(self):
        other = {"identifier": "com.synthetic.existing", "version": "7"}
        image = {"identifier": adapters.CRYPTEX_ID, "version": "1.0"}
        report, calls = self.cryptex_sequence([(0, [other]), (0, image), (0, [other, image]),
                                               (0, [other, image]), (0, {"uninstalled": adapters.CRYPTEX_ID}), (0, [other])])
        self.assertEqual(report["status"], "PASS", report)
        self.assertEqual(calls[1], ["ddi", "cryptex", "auto-install", "--restore-dir", str((self.root / "Restore").resolve())])
        self.assertEqual(calls[4], ["ddi", "cryptex", "uninstall", adapters.CRYPTEX_ID, "--version", "1.0"])

    def test_cryptex_preexisting_version_is_never_mutated(self):
        image = {"identifier": adapters.CRYPTEX_ID, "version": "1.0"}
        report, calls = self.cryptex_sequence([(0, [image])])
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(calls, [["ddi", "cryptex", "list"]])
        self.assertTrue(report["cleanup_complete"])

    def test_cryptex_partial_failure_cleans_expected_and_preserves_other_version(self):
        image = {"identifier": adapters.CRYPTEX_ID, "version": "1.0"}
        report, calls = self.cryptex_sequence([(0, []), (1, {}), (0, [image]),
                                               (0, {"uninstalled": adapters.CRYPTEX_ID}), (0, [])])
        self.assertEqual(report["status"], "FAIL")
        self.assertTrue(report["cleanup_complete"])
        self.assertEqual(calls[3], ["ddi", "cryptex", "uninstall", adapters.CRYPTEX_ID, "--version", "1.0"])

    def test_cryptex_changed_version_is_preserved_with_failed_cleanup(self):
        different = {"identifier": adapters.CRYPTEX_ID, "version": "unowned"}
        report, calls = self.cryptex_sequence([(0, []), (1, {}), (0, [different])])
        self.assertEqual(report["status"], "FAIL")
        self.assertFalse(report["cleanup_complete"])
        self.assertFalse(any("uninstall" in call for call in calls))

    def clipboard_fixture(self):
        (self.root / "before.txt").write_bytes(b"approved-synthetic-baseline")
        (self.root / "text.txt").write_bytes(b"synthetic-matrix-text")
        (self.root / "image.png").write_bytes(b"\x89PNG\r\n\x1a\nsynthetic-test-schema-only")
        def entry(uti, name):
            return dict(uti=uti, path=name, size=(self.root / name).stat().st_size, sha256=adapters.digest(self.root / name))
        return dict(pasteboard="com.apple.UIKit.pboard.general", baseline=[entry("public.utf8-plain-text", "before.txt")],
                    contents=[entry("public.utf8-plain-text", "text.txt"), entry("public.png", "image.png")])

    def test_clipboard_requires_hash_bound_synthetic_single_item_baseline(self):
        item = self.clipboard_fixture()
        self.manifest({"synthetic-clipboard": item})
        adapters.load_fixture_manifest(self.root, self.context)
        self.assertFalse("clipboard-scope" in self.context.scopes)
        item["baseline"] = []
        self.manifest({"synthetic-clipboard": item})
        with self.assertRaisesRegex(ValueError, "single-item-baseline"):
            adapters.load_fixture_manifest(self.root, self.context)

    def test_clipboard_windows_uses_adapter_and_baseline_failure_prevents_mutation(self):
        if os.name != "nt":
            self.skipTest("Windows specific signal boundary")
        self.manifest({"synthetic-clipboard": self.clipboard_fixture()})
        adapters.load_fixture_manifest(self.root, self.context)
        self.context.scopes = frozenset({"clipboard-scope"})
        self.context.evidence = {"format-evidence": {}}
        session = regression.Session(self.root)
        try:
            case = next(row for row in session.matrix["cases"] if row["id"] == "rich-clipboard-watch")
            # Windows now enters the real workflow. A failed read-only baseline
            # must still prevent the watcher and every clipboard mutation.
            with patch.object(adapters, "capture", return_value=Capture(b"", b"", 1, False, False, False)) as observed:
                report = regression.execute_device_case(session, case, self.context)
            self.assertEqual(report["status"], "FAIL")
            observed.assert_called_once()
            self.assertEqual(observed.call_args.args[0][5:7], ["pasteboard", "get"])
            self.assertEqual(report["reason"], "operation-or-cleanup-failed")
        finally:
            session.cleanup()

    def test_clipboard_unrestorable_metadata_and_promises_are_rejected(self):
        fixture = self.clipboard_fixture()
        snapshot = {"items": [{"index": 0, "types": ["public.utf8-plain-text"], "data": [
            {"uti": "public.utf8-plain-text", "payload": {"state": "inline", "size": 2, "sha256": "0" * 64}}]}]}
        self.assertEqual(len(adapters.clipboard_descriptor(snapshot)), 1)
        snapshot["source_metadata"] = {"preserve": True}
        with self.assertRaisesRegex(ValueError, "unrestorable"):
            adapters.clipboard_descriptor(snapshot)
        del snapshot["source_metadata"]
        snapshot["items"][0]["data"][0]["payload"]["state"] = "promised"
        with self.assertRaisesRegex(ValueError, "inline-approved"):
            adapters.clipboard_descriptor(snapshot)

    @unittest.skipUnless(CLI.is_file(), "existing native CLI binary required; this test never builds")
    def test_actual_cli_accepts_adapter_argv_before_io(self):
        commands = [adapters.query_command(name, self.context)[0] for name in
                    ("traditional-usb", "traditional-wifi", "coredevice-usb", "remoted-coexistence")]
        commands += [adapters.command(self.context, *args) for args in (
            ("file", "pull", REMOTE_PATH, "synthetic-output.bin"),
            ("rsd", "services", "--all"),
            ("file", "pull", REMOTE_PATH, "synthetic-output.bin", "--force"),
            ("apps", "list", "--app-type", "all"), ("apps", "install", "synthetic.ipa", "--streaming"),
            ("apps", "show", "com.synthetic.App"), ("apps", "launch", "com.synthetic.App"),
            ("apps", "kill", "123"), ("apps", "uninstall", "com.synthetic.App"),
            ("ddi", "cryptex", "list"), ("ddi", "cryptex", "auto-install", "--restore-dir", "Restore"),
            ("ddi", "cryptex", "uninstall", adapters.CRYPTEX_ID, "--version", "1.0"),
            ("pasteboard", "get", "--pasteboard", "com.apple.UIKit.pboard.general", "--policy", "resolved"),
            ("pasteboard", "set", "--pasteboard", "com.apple.UIKit.pboard.general", "--raw", "--data", "public.png=c3ludGhldGlj"),
            ("pasteboard", "watch", "--reconnect", "--reconnect-attempts", "0", "--timeout", "45"),
            ("notify", "post", "com.apple.pasteboard.notify.changed"),
            ("runxctest", "--test-runner-bundle-id", "com.synthetic.Runner", "--xctest-config", "Tests.xctest",
             "--xctest", "--wait", "--timeout", "10", "--test", "Tests/testExample", "--junit-output", "out.xml", "--log-output", "out.log"))]
        for command in commands:
            with self.subTest(command=command[5:]):
                result = capture(command + ["--help"], timeout=3)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(result.cleanup_complete)


@unittest.skipUnless(os.name == "posix", "POSIX original process-group and SIGINT boundary")
class WatchCaptureTests(unittest.TestCase):
    def test_original_watch_sigint_receipt_and_reader_joins_preserve_unrelated_process(self):
        unrelated = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(20)"],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        watch = None
        script = ("import json,signal,sys,time\n"
                  "def stop(*_):\n"
                  " print(json.dumps({'state':'stopped','native_reclaimed':True}),file=sys.stderr,flush=True)\n"
                  " sys.exit(0)\n"
                  "signal.signal(signal.SIGINT,stop)\n"
                  "print(json.dumps({'state':'connected'}),file=sys.stderr,flush=True)\n"
                  "print(json.dumps({'synthetic':'one'}),flush=True)\n"
                  "time.sleep(20)\n")
        try:
            watch = OwnedWatch([sys.executable, "-c", script], dict(os.environ), timeout=4)
            watch.wait(lambda records: any(row.get("state") == "connected" for row in records[1]) and bool(records[0]), 2)
            result, receipt, records = watch.finish()
            watch = None
            self.assertTrue(receipt)
            self.assertEqual(result.returncode, 0)
            self.assertTrue(result.cleanup_complete)
            self.assertFalse(result.incomplete)
            self.assertFalse(result.timed_out)
            self.assertEqual(records, [{"synthetic": "one"}])
            self.assertIsNone(unrelated.poll())
        finally:
            if watch:
                watch.finish()
            unrelated.terminate()
            unrelated.wait(timeout=3)


@unittest.skipUnless(sys.platform == "win32" and CLI.is_file(), "Windows existing-trust path isolation and an existing native CLI required")
class ActualCliAfcTests(unittest.TestCase):
    def test_device_file_adapters_use_actual_cli_and_strict_original_peer_eof(self):
        with tempfile.TemporaryDirectory(prefix="ios-actual-cli-afc-") as directory:
            root = Path(directory)
            peer = CliAfcPeer(root / "trust")
            try:
                context = regression.ExecutionContext(
                    binary=CLI, device=regression.DeviceTarget(UDID, transport="USB"),
                    fixtures={"afc-fixture": dict(remote_path=REMOTE_PATH, size=len(PAYLOAD), sha256=hashlib.sha256(PAYLOAD).hexdigest())},
                    environment=peer.environment, fixture_peer=True)
                session = regression.Session(root)
                try:
                    ids = ("file-workflow", "windows-afc-new", "windows-afc-existing", "windows-afc-force",
                           "windows-afc-unicode", "windows-afc-long", "windows-afc-verbatim", "windows-afc-readonly", "windows-afc-recovery")
                    for case_id in ids:
                        case = next(case for case in session.matrix["cases"] if case["id"] == case_id)
                        report = regression.execute_device_case(session, case, context)
                        self.assertEqual(report["status"], "PASS", report)
                        self.assertEqual(report["evidence_origin"], "localhost-protocol-fixture")
                        self.assertTrue(report["cleanup_complete"])
                        self.assertTrue(report["artifacts"])
                        for artifact in report["artifacts"]:
                            self.assertEqual(artifact["size"], artifact["expected_size"])
                            self.assertEqual(artifact["sha256"], artifact["expected_sha256"])
                            self.assertTrue(artifact["no_unexpected_output"])
                            if artifact["kind"] == "downloaded-file":
                                self.assertEqual(artifact["sha256"], hashlib.sha256(PAYLOAD).hexdigest())
                            if case_id == "windows-afc-long":
                                self.assertGreater(artifact["target_utf16_units"], 260)
                        for step in report["steps"]:
                            self.assertGreater(step["pid"], 0)
                            self.assertEqual(step["cleanup_scope"], "windows-job")
                    self.assertFalse(list(root.glob("ios-matrix-owned-*")))
                finally:
                    session.cleanup()
            finally:
                complete = peer.close()
            self.assertTrue(complete, peer.errors)
            self.assertTrue({10, 13, 15, 20} <= set(peer.operations))
            self.assertFalse({8, 16} & set(peer.operations))  # Read scope never writes/deletes remotely.


@unittest.skipUnless(sys.platform == "win32" and CLI.is_file(), "Windows existing-trust path isolation and an existing native CLI required")
class ActualCliWorkflowTests(unittest.TestCase):
    def capture_closed_stdout(self, command, environment):
        import threading
        import time
        from regression_windows_process import CapturedProcess
        started = time.monotonic()
        process = CapturedProcess(command, cwd=regression.ROOT, env=environment)
        # Close the original reader before the command publishes its final receipt.
        process.stdout.close()
        error_bytes, reader_errors = bytearray(), []
        overflow, forced, timed_out = False, False, False

        def reader():
            nonlocal overflow
            try:
                while chunk := process.stderr.read1(4096):
                    remaining = 65536 - len(error_bytes)
                    error_bytes.extend(chunk[:remaining])
                    overflow = overflow or len(chunk) > remaining
            except (OSError, ValueError):
                reader_errors.append(True)
            finally:
                process.stderr.close()

        worker = threading.Thread(target=reader, name="owned-closed-output-stderr")
        worker.start()
        try:
            process.wait(timeout=12)
        except subprocess.TimeoutExpired:
            timed_out = True
        finally:
            if process.poll() is None or process.job.active():
                forced = True
                process.terminate()
            process.wait(timeout=2)
            empty = process.job.empty(2)
            worker.join(timeout=1)
            complete = empty and not worker.is_alive() and not forced
            process.close()
        return Capture(b"", bytes(error_bytes), process.returncode, timed_out, overflow,
                       bool(reader_errors) or worker.is_alive(), round((time.monotonic() - started) * 1000),
                       process.pid, complete, "windows-job")

    def run_native_xctest(self, *, fault=None, first_test_fails=False, rerun=False, report_directory=False,
                          preexisting_runner=False, entry="runxctest", terminal_scenario=None,
                          rerun_terminal_scenario=None, sidecar_directory=False, human_output=False,
                          closed_output=False):
        from xml.etree import ElementTree
        with tempfile.TemporaryDirectory(prefix="ios-native-xctest-stages-") as directory:
            root = Path(directory)
            peer = CliWorkflowPeer(root / "trust", fault=fault, runner_preexisting=preexisting_runner,
                                   first_test_fails=first_test_fails, terminal_scenario=terminal_scenario,
                                   rerun_terminal_scenario=rerun_terminal_scenario)
            packages = []
            for name, bundle_id in (("wrong-name.ipa", APP_ID), ("wrong-runner-name.ipa", RUNNER_ID)):
                package = root / name
                with zipfile.ZipFile(package, "w") as archive:
                    archive.writestr("Payload/Synthetic.app/Info.plist", plistlib.dumps({"CFBundleIdentifier": bundle_id,
                                                                                       "CFBundleExecutable": "SyntheticRunner"}))
                packages.append(package)
            xml, log = root / "results.xml", root / "syslog.txt"
            if report_directory:
                xml.mkdir()
                (xml / "preserved-marker").write_bytes(b"preserve-directory")
            history_path = Path(str(xml) + ".attempts.json")
            if sidecar_directory:
                history_path.mkdir()
                (history_path / "preserved-marker").write_bytes(b"preserve-sidecar-directory")
            context = regression.ExecutionContext(binary=CLI, device=regression.DeviceTarget(UDID), environment=peer.environment)
            arguments = ["runxctest", "--bundle-id", APP_ID, "--test-runner-bundle-id", RUNNER_ID,
                         "--xctest-config", "MatrixTests.xctest", "--xctest", "--wait", "--timeout", "5",
                         "--test", "MatrixTests/testExample"]
            if entry == "runtest":
                xctestrun = root / "MatrixTests.xctestrun"
                xctestrun.write_bytes(plistlib.dumps({"__xctestrun_metadata__": {"FormatVersion": 2},
                    "TestConfigurations": [{"Name": "Matrix", "TestTargets": [{
                        "TestHostBundleIdentifier": RUNNER_ID, "TestBundlePath": "MatrixTests.xctest",
                        "IsUITestBundle": True, "OnlyTestIdentifiers": ["MatrixTests/testExample"],
                        "UITargetAppEnvironmentVariables": {"UITargetAppBundleIdentifier": APP_ID},
                    }]}]}))
                arguments = ["runtest", xctestrun, "--wait", "--startup-timeout-secs", "2",
                             "--result-timeout-secs", "3"]
            arguments += ["--install-app", packages[0], "--install-runner", packages[1],
                         "--uninstall-on-finish", "--junit-output", xml, "--log-output", log]
            if rerun:
                arguments += ["--rerun-failed", "1"]
            try:
                command = adapters.command(context, *arguments)
                if human_output:
                    command.insert(1, "--no-json")
                result = (self.capture_closed_stdout(command, peer.environment) if closed_output else
                          capture(command, env=peer.environment, timeout=12, max_bytes=65536))
                xml_value = ElementTree.parse(xml).getroot() if xml.is_file() else None
                log_value = log.read_bytes() if log.is_file() else b""
                history = json.loads(history_path.read_bytes()) if history_path.is_file() else None
                if report_directory:
                    self.assertEqual((xml / "preserved-marker").read_bytes(), b"preserve-directory")
                    self.assertEqual(set(xml.iterdir()), {xml / "preserved-marker"})
                if sidecar_directory:
                    self.assertEqual((history_path / "preserved-marker").read_bytes(), b"preserve-sidecar-directory")
                    self.assertEqual(set(history_path.iterdir()), {history_path / "preserved-marker"})
                self.assertFalse(result.timed_out, result.stderr)
                self.assertTrue(result.cleanup_complete)
                self.assertFalse(result.overflow)
                self.assertFalse(result.incomplete)
            finally:
                complete = peer.close()
            self.assertTrue(complete, peer.errors)
            self.assertFalse(peer.pids)
            self.assertNotIn(APP_ID, peer.apps)
            self.assertEqual(RUNNER_ID in peer.apps, preexisting_runner or fault == "uninstall")
            self.assertIn(b"log capture finished", log_value)
            self.assertFalse(list(root.glob(".*.tmp")))
            return result, peer, xml_value, log_value, history

    def run_case(self, case_id, *, preexisting=False, install_error=False, test_error=False):
        with tempfile.TemporaryDirectory(prefix="ios-actual-cli-workflow-") as directory:
            root = Path(directory)
            peer = CliWorkflowPeer(root / "trust", preexisting=preexisting,
                                   install_error=install_error, test_error=test_error)
            package = root / "wrong-name.ipa"
            info = plistlib.dumps({"CFBundleIdentifier": APP_ID, "CFBundleExecutable": "Synthetic"})
            with zipfile.ZipFile(package, "w") as archive:
                archive.writestr("Payload/Synthetic.app/Info.plist", info)
                archive.writestr("Payload/Synthetic.app/synthetic.txt", b"synthetic-matrix-payload")
            context = regression.ExecutionContext(
                binary=CLI, device=regression.DeviceTarget(UDID, transport="USB"),
                scopes=frozenset({"application-operation-scope", "test-operation-scope"}),
                fixtures={"test-application": dict(path=package, sha256=adapters.digest(package), bundle_id=APP_ID),
                          "test-bundle": dict(runner_bundle_id=RUNNER_ID, xctest_config="MatrixTests.xctest",
                                              tests=["MatrixTests/testExample"], timeout_secs=10)},
                environment=peer.environment, fixture_peer=True)
            session = regression.Session(root)
            try:
                case = next(case for case in session.matrix["cases"] if case["id"] == case_id)
                report = regression.execute_device_case(session, case, context)
                self.assertTrue(report["cleanup_complete"], report)
                self.assertEqual(report["evidence_origin"], "localhost-protocol-fixture")
                self.assertFalse(list(root.glob("ios-matrix-owned-*")))
            finally:
                session.cleanup()
                complete = peer.close()
            self.assertTrue(complete, peer.errors)
            return peer, report, info

    def test_application_install_launch_stop_and_owned_uninstall_actual_cli(self):
        peer, report, info = self.run_case("application-workflow")
        self.assertEqual(report["status"], "PASS", report)
        self.assertEqual(peer.transferred["Payload/Synthetic.app/Info.plist"], info)
        self.assertEqual(peer.transferred["Payload/Synthetic.app/synthetic.txt"], b"synthetic-matrix-payload")
        self.assertNotIn(APP_ID, peer.apps)
        self.assertIn(RUNNER_ID, peer.apps)
        self.assertFalse(peer.pids)
        self.assertEqual(peer.calls.count("InstallTransferred"), 1)
        self.assertEqual(peer.calls.count(LAUNCH), 1)
        self.assertEqual(peer.calls.count("killPid:"), 1)
        self.assertEqual(peer.calls.count("Uninstall"), 1)

    def test_preexisting_application_is_preserved_without_install_or_uninstall(self):
        peer, report, _ = self.run_case("application-workflow", preexisting=True)
        self.assertEqual(report["status"], "FAIL", report)
        self.assertIn(APP_ID, peer.apps)
        self.assertEqual(peer.calls, ["Browse"])
        self.assertFalse(peer.transferred)

    def test_partial_install_failure_still_uninstalls_only_owned_package(self):
        peer, report, _ = self.run_case("application-workflow", install_error=True)
        self.assertEqual(report["status"], "FAIL", report)
        self.assertNotIn(APP_ID, peer.apps)
        self.assertIn(RUNNER_ID, peer.apps)
        self.assertEqual(peer.calls.count("Uninstall"), 1)
        self.assertNotIn(LAUNCH, peer.calls)

    def test_xctest_actual_cli_native_dtx_selection_results_logs_and_join(self):
        peer, report, _ = self.run_case("xctest-workflow")
        self.assertEqual(report["status"], "PASS", report)
        self.assertEqual(peer.calls.count("_IDE_authorizeTestSessionWithProcessID:"), 1)
        self.assertEqual(peer.calls.count("_IDE_startExecutingTestPlanWithProtocolVersion:"), 1)
        self.assertEqual(len(peer.configurations), 1)
        self.assertIn("MatrixTests/testExample", str(peer.configurations[0]))
        self.assertFalse(peer.pids)
        self.assertNotIn("Uninstall", peer.calls)

    def test_xctest_actual_failure_is_nonzero_and_cleanup_still_finishes(self):
        peer, report, _ = self.run_case("xctest-workflow", test_error=True)
        self.assertEqual(report["status"], "FAIL", report)
        self.assertFalse(peer.pids)
        self.assertEqual(report["steps"][0]["returncode"], 1)

    def test_native_xctest_install_runner_failure_cleans_both_real_package_identifiers(self):
        result, peer, xml, _, history = self.run_native_xctest(fault="install-runner")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(peer.calls.count("InstallTransferred"), 2)
        self.assertEqual(peer.calls.count("Uninstall"), 2)
        self.assertNotIn(LAUNCH, peer.calls)
        self.assertIsNotNone(xml)
        self.assertIsNotNone(history["terminal_error_class"])

    def test_native_xctest_each_protocol_stage_failure_runs_original_finally(self):
        for stage in ("lookup-runner", "lookup-target", "channel", "session", "authorize", "start", "result"):
            with self.subTest(stage=stage):
                result, peer, xml, _, history = self.run_native_xctest(fault=stage)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(peer.calls.count("InstallTransferred"), 2)
                self.assertEqual(peer.calls.count("Uninstall"), 2)
                self.assertIsNotNone(xml)
                self.assertIsNotNone(history["terminal_error_class"])

    def test_native_xctest_log_eof_and_junit_failure_keep_finally_and_original_directory(self):
        for parameters in (dict(fault="log-eof"), dict(report_directory=True)):
            with self.subTest(parameters=parameters):
                result, peer, _, _, history = self.run_native_xctest(**parameters)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(peer.calls.count("Uninstall"), 2)
                self.assertIsNotNone(history["terminal_error_class"])

    def test_native_xctest_rerun_retains_first_failure_and_does_not_reinstall(self):
        result, peer, xml, _, history = self.run_native_xctest(first_test_fails=True, rerun=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        body = json.loads(result.stdout)
        self.assertEqual(peer.calls.count("InstallTransferred"), 2)
        self.assertEqual(peer.test_attempts, 2)
        summary = body["summary"]
        case = summary["suites"][0]["cases"][0]
        self.assertEqual(case["status"], "passed")
        self.assertEqual(body["attempts"], history)
        self.assertEqual([attempt["state"] for attempt in history["attempts"]], ["failed_tests", "completed"])
        self.assertEqual(history["attempts"][0]["summary"]["suites"][0]["cases"][0]["failure"]["message"], "synthetic assertion")
        self.assertEqual(history["attempts"][1]["summary"]["failed_tests"], 0)
        self.assertFalse(xml.findall(".//failure"))

    def test_native_xctest_rerun_startup_and_result_failures_retain_first_failure(self):
        for stage in ("rerun-session", "rerun-result"):
            with self.subTest(stage=stage):
                result, peer, xml, _, history = self.run_native_xctest(fault=stage, first_test_fails=True, rerun=True)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual([attempt["state"] for attempt in history["attempts"]], ["failed_tests", "interrupted"])
                self.assertEqual(history["attempts"][0]["summary"]["failed_tests"], 1)
                self.assertIsNotNone(history["terminal_error_class"])
                self.assertTrue(xml.findall(".//failure") or xml.findall(".//error"))

    def test_native_xctest_preexisting_runner_is_never_uninstalled(self):
        result, peer, _, _, _ = self.run_native_xctest(preexisting_runner=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(peer.calls.count("Uninstall"), 1)

    def test_native_xctest_total_deadline_wakes_stalled_startup_and_result_peers(self):
        for stage in ("deadline-session", "deadline-result"):
            with self.subTest(stage=stage):
                result, peer, xml, _, history = self.run_native_xctest(fault=stage)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertLess(result.duration_ms, 9000)
                self.assertEqual(peer.calls.count("Uninstall"), 2)
                self.assertIsNotNone(history["terminal_error_class"])
                self.assertIsNotNone(xml)

    def assert_final_receipt(self, result, history, *, outcome, test_outcome):
        # A single JSON value proves that an earlier successful receipt was not emitted.
        body = json.loads(result.stdout)
        self.assertEqual(body["status"], "finished")
        self.assertEqual(body["outcome"], outcome)
        self.assertEqual(body["test_outcome"], test_outcome)
        self.assertEqual(body["attempts"], history)
        self.assertEqual(body["terminal_error_class"], history["terminal_error_class"])
        self.assertEqual(body["terminal_error_class"] is None, outcome == "passed")
        return body

    def test_terminal_supported_statuses_and_empty_plan_preserve_actual_cli_outcomes(self):
        for entry in ("runxctest", "runtest"):
            for scenario in (None, "failed", "skipped", "expected-failure", "empty"):
                with self.subTest(entry=entry, scenario=scenario):
                    result, peer, xml, _, history = self.run_native_xctest(entry=entry, terminal_scenario=scenario)
                    failed = scenario == "failed"
                    self.assertEqual(result.returncode, int(failed), result.stderr)
                    body = self.assert_final_receipt(result, history, outcome="failed" if failed else "passed",
                                                     test_outcome="failed_tests" if failed else "passed")
                    self.assertEqual(body["summary"]["failed_tests"], int(failed))
                    self.assertEqual(body["summary"]["total_tests"], int(scenario != "empty"))
                    self.assertEqual(body["summary"]["skipped_tests"], int(scenario in ("skipped", "expected-failure")))
                    self.assertEqual(peer.test_attempts, 1)
                    self.assertEqual(peer.calls.count("killPid:"), 1)
                    self.assertFalse(xml.findall(".//error") if not failed else [])
                    if scenario == "expected-failure":
                        self.assertEqual(body["summary"]["suites"][0]["cases"][0]["failure"]["message"], "synthetic assertion")

    def test_terminal_invalid_evidence_is_nonzero_and_keeps_raw_history_without_rerun(self):
        for entry in ("runxctest", "runtest"):
            for scenario in ("unfinished", "stalled", "unknown", "uncaught", "failed-zero-counter", "unexpected-zero-counter", "finish-only"):
                with self.subTest(entry=entry, scenario=scenario):
                    result, peer, xml, _, history = self.run_native_xctest(entry=entry, terminal_scenario=scenario,
                                                                         rerun=scenario not in ("failed-zero-counter", "unexpected-zero-counter"))
                    self.assertEqual(result.returncode, 1, result.stderr)
                    supported_failure = scenario in ("failed-zero-counter", "unexpected-zero-counter")
                    body = self.assert_final_receipt(result, history, outcome="failed",
                                                     test_outcome="failed_tests" if supported_failure else "incomplete")
                    attempt = history["attempts"][0]
                    self.assertEqual(attempt["state"], "failed_tests" if supported_failure else "incomplete")
                    self.assertEqual(attempt["summary"], body["summary"])
                    self.assertEqual(body["summary"]["finished"], True)
                    self.assertEqual(peer.test_attempts, 1)
                    self.assertEqual(peer.calls.count("InstallTransferred"), 2)
                    self.assertEqual(peer.calls.count("Uninstall"), 2)
                    self.assertEqual(peer.calls.count("killPid:"), 1)
                    self.assertTrue(xml.findall(".//failure") if scenario == "failed-zero-counter" else xml.findall(".//error"))
                    if supported_failure:
                        self.assertEqual(body["summary"]["failed_tests"], 1)
                        self.assertEqual(body["summary"]["suites"][0]["failures"], 0)
                    if scenario == "unexpected-zero-counter":
                        self.assertEqual(body["summary"]["suites"][0]["unexpected_failures"], 1)
                    if scenario == "uncaught":
                        self.assertEqual(body["summary"]["suites"][0]["uncaught_exceptions"], 1)
                    if scenario == "finish-only":
                        self.assertEqual(body["summary"]["began"], False)
                    if scenario in ("unfinished", "stalled", "unknown"):
                        expected_status = {"unfinished": None, "stalled": "stalled",
                                           "unknown": {"other": "synthetic-new-status"}}[scenario]
                        self.assertEqual(body["summary"]["suites"][0]["cases"][0]["status"], expected_status)

    def test_rerun_invalid_terminal_evidence_preserves_first_failure_and_both_attempts(self):
        for entry in ("runxctest", "runtest"):
            for scenario in ("unfinished", "stalled", "unknown", "uncaught", "finish-only"):
                with self.subTest(entry=entry, scenario=scenario):
                    result, peer, xml, _, history = self.run_native_xctest(entry=entry, first_test_fails=True,
                        rerun=True, rerun_terminal_scenario=scenario)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    body = self.assert_final_receipt(result, history, outcome="failed", test_outcome="failed_tests")
                    self.assertEqual([attempt["state"] for attempt in history["attempts"]], ["failed_tests", "incomplete"])
                    first = history["attempts"][0]["summary"]
                    self.assertEqual(first, body["summary"])
                    self.assertEqual(first["suites"][0]["cases"][0]["failure"]["message"], "synthetic assertion")
                    self.assertEqual(history["attempts"][1]["summary"]["finished"], True)
                    self.assertEqual(peer.test_attempts, 2)
                    self.assertEqual(peer.calls.count("InstallTransferred"), 2)
                    self.assertEqual(peer.calls.count("Uninstall"), 2)
                    self.assertEqual(peer.calls.count("killPid:"), 2)
                    self.assertTrue(xml.findall(".//failure"))

    def test_final_output_waits_for_log_uninstall_report_and_sidecar_outcomes(self):
        for entry in ("runxctest", "runtest"):
            for parameters in (dict(fault="log-eof"), dict(fault="uninstall"),
                               dict(report_directory=True), dict(sidecar_directory=True)):
                with self.subTest(entry=entry, parameters=parameters):
                    result, peer, _, _, history = self.run_native_xctest(entry=entry, **parameters)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    if history is not None:
                        body = self.assert_final_receipt(result, history, outcome="failed", test_outcome="passed")
                        self.assertEqual(history["attempts"][0]["state"], "completed")
                    else:
                        body = json.loads(result.stdout)
                        self.assertEqual(body["outcome"], "failed")
                        self.assertEqual(body["test_outcome"], "passed")
                        self.assertIsNotNone(body["terminal_error_class"])
                        self.assertEqual(body["terminal_error_class"], body["attempts"]["terminal_error_class"])
                    self.assertEqual(body["summary"]["finished"], True)
                    self.assertEqual(peer.calls.count("killPid:"), 1)
                    self.assertEqual(peer.calls.count("Uninstall"), 2)

    def test_human_finished_output_reports_final_failure_after_cleanup(self):
        result, _, _, _, history = self.run_native_xctest(fault="log-eof", human_output=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(b"XCTest failed", result.stdout)
        self.assertNotIn(b"XCTest finished", result.stdout)
        self.assertIsNotNone(history["terminal_error_class"])

    def test_closed_stdout_failure_reclassifies_actual_sidecar_after_cleanup_without_replay(self):
        for entry in ("runxctest", "runtest"):
            with self.subTest(entry=entry):
                result, peer, _, _, history = self.run_native_xctest(entry=entry, closed_output=True)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(result.stdout, b"")
                self.assertIsNotNone(history["terminal_error_class"])
                self.assertEqual(history["attempts"][0]["state"], "completed")
                self.assertEqual(history["attempts"][0]["summary"]["failed_tests"], 0)
                self.assertEqual(peer.test_attempts, 1)
                self.assertEqual(peer.calls.count("InstallTransferred"), 2)
                self.assertEqual(peer.calls.count("Uninstall"), 2)
                self.assertEqual(peer.calls.count("killPid:"), 1)

    def test_runtest_passing_rerun_retains_first_failure_and_single_package_installation(self):
        result, peer, _, _, history = self.run_native_xctest(entry="runtest", first_test_fails=True, rerun=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        body = self.assert_final_receipt(result, history, outcome="passed", test_outcome="passed")
        self.assertEqual(body["summary"]["failed_tests"], 0)
        self.assertEqual([attempt["state"] for attempt in history["attempts"]], ["failed_tests", "completed"])
        self.assertEqual(history["attempts"][0]["summary"]["suites"][0]["cases"][0]["failure"]["message"], "synthetic assertion")
        self.assertEqual(peer.calls.count("InstallTransferred"), 2)
        self.assertEqual(peer.calls.count("killPid:"), 2)


if __name__ == "__main__":
    unittest.main()
