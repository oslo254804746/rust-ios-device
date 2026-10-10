"""Bounded matrix operations. Manifest observations never grant operation scopes.

The manifest contains fixture identities, not executable commands. All argv are
constructed here from the product CLI. A missing protocol observer is NOT_RUN,
never replaced with an unrelated successful query.
"""

import hashlib
import ipaddress
import json
import os
from pathlib import Path
import plistlib
import re
import stat
import tempfile
import time
import zipfile

from diagnostics import assess, capture

MANIFEST_LIMIT = 65536
FILE_LIMIT = 16 * 1024 * 1024
UNSUPPORTED = {
    "private-mac-altirk": "private-mac-and-altirk-wire-correlation-observer-unavailable",
    "cryptex-install": "cryptex-exact-install-ownership-and-uninstall-observer-unavailable",
    "display-hid-stop": "display-hid-restored-state-and-original-service-join-observer-unavailable",
    "rich-clipboard-watch": "rich-clipboard-watch-trigger-deduplication-and-restoration-observer-unavailable",
    "installcoordination-wire": "same-client-zero-budget-and-signed-wire-observer-unavailable-in-cli",
    "modern-rsd-wifi": "queued-direct-legacy-passive-mode-selection-observer-unavailable-in-cli",
}


def checked_json(result):
    if result.returncode != 0 or not result.stdout or len(result.stdout) > MANIFEST_LIMIT:
        raise ValueError("query-result-unavailable")
    return json.loads(result.stdout)


def fixture_path(root, value):
    if not isinstance(value, str) or not value or len(value) > 4096:
        raise ValueError("fixture-path-shape")
    path = root / value
    # Resolve only for comparison. Reject symlinks in every fixture component;
    # named fixtures may not escape their explicitly selected root.
    resolved = path.resolve(strict=True)
    if root not in resolved.parents or not resolved.is_file():
        raise ValueError("fixture-outside-root")
    current = path
    while current != root:
        if current.is_symlink():
            raise ValueError("fixture-symlink")
        current = current.parent
    return resolved


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(65536), b""):
            value.update(chunk)
    return value.hexdigest()


def package_id(path):
    with zipfile.ZipFile(path) as archive:
        if len(archive.infolist()) > 100000:
            raise ValueError("package-entry-limit")
        names = [name for name in archive.namelist()
                 if re.fullmatch(r"Payload/[^/]+\.app/Info\.plist", name)]
        if len(names) != 1 or archive.getinfo(names[0]).file_size > 1024 * 1024:
            raise ValueError("package-info-shape")
        value = plistlib.loads(archive.read(names[0])).get("CFBundleIdentifier")
        if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", value):
            raise ValueError("package-bundle-id")
        return value


def load_fixture_manifest(directory, context):
    root = Path(directory).resolve(strict=True)
    if not root.is_dir():
        raise ValueError("fixture-directory")
    manifest = root / "manifest.json"
    if manifest.is_symlink() or not manifest.is_file() or manifest.stat().st_size > MANIFEST_LIMIT:
        raise ValueError("fixture-manifest-size")
    value = json.loads(manifest.read_text(encoding="utf-8"))
    if set(value) != {"version", "target_udid", "fixtures", "observations", "evidence"} or value["version"] != 1:
        raise ValueError("fixture-manifest-schema")
    if context.device is None or value["target_udid"] != context.device.udid:
        raise ValueError("fixture-target-mismatch")
    fixtures = value["fixtures"]
    if not isinstance(fixtures, dict) or len(fixtures) > 16:
        raise ValueError("fixture-count")
    allowed = {"afc-fixture", "test-application", "test-runner", "test-bundle", "authorized-unc-share", "existing-rsd-route"}
    if not set(fixtures) <= allowed:
        raise ValueError("fixture-name")
    admitted = {}
    for name, item in fixtures.items():
        if not isinstance(item, dict):
            raise ValueError("fixture-shape")
        if name == "afc-fixture":
            if set(item) != {"remote_path", "size", "sha256"}:
                raise ValueError("afc-fixture-schema")
            if not isinstance(item["remote_path"], str) or not item["remote_path"].startswith("/") or "\x00" in item["remote_path"] or ".." in item["remote_path"].split("/"):
                raise ValueError("afc-remote-path")
            if type(item["size"]) is not int or not 0 <= item["size"] <= FILE_LIMIT or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"]):
                raise ValueError("afc-fixture-integrity")
        elif name in ("test-application", "test-runner"):
            if set(item) != {"path", "sha256", "bundle_id"}:
                raise ValueError("package-fixture-schema")
            path = fixture_path(root, item["path"])
            if not re.fullmatch(r"[0-9a-f]{64}", item["sha256"]) or digest(path) != item["sha256"] or package_id(path) != item["bundle_id"]:
                raise ValueError("package-fixture-integrity")
            item = dict(item, path=path)
        elif name == "test-bundle":
            if set(item) != {"runner_bundle_id", "xctest_config", "tests", "timeout_secs"}:
                raise ValueError("test-bundle-schema")
            if not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", item["runner_bundle_id"]) or not re.fullmatch(r"[^/\\]{1,128}\.xctest", item["xctest_config"]):
                raise ValueError("test-bundle-selection")
            if not isinstance(item["tests"], list) or not 1 <= len(item["tests"]) <= 32 or any(not re.fullmatch(r"[A-Za-z0-9_]+/[A-Za-z0-9_]+", selector) for selector in item["tests"]):
                raise ValueError("test-selector")
            if type(item["timeout_secs"]) is not int or not 1 <= item["timeout_secs"] <= 60:
                raise ValueError("test-time-budget")
        elif name == "authorized-unc-share":
            if set(item) != {"directory"} or not isinstance(item["directory"], str) or not item["directory"].startswith("\\\\") or not Path(item["directory"]).is_dir():
                raise ValueError("unc-directory")
        elif name == "existing-rsd-route":
            if not set(item) <= {"address", "proxy_port"} or "address" not in item:
                raise ValueError("rsd-route-schema")
            match = re.fullmatch(r"\[([^\]]+)\]:([0-9]{1,5})", item["address"])
            if match is None or not 1 <= int(match[2]) <= 65535:
                raise ValueError("rsd-route-address")
            address = ipaddress.IPv6Address(match[1])
            if address.is_unspecified or address.is_multicast:
                raise ValueError("rsd-route-address")
            if "proxy_port" in item and (type(item["proxy_port"]) is not int or not 1 <= item["proxy_port"] <= 65535):
                raise ValueError("rsd-route-proxy")
        admitted[name] = item
    observations = value["observations"]
    if not isinstance(observations, dict) or not set(observations) <= {"services", "modes", "wifi_trust", "rsd_identity", "identity"}:
        raise ValueError("observation-schema")
    for key in ("services", "modes"):
        values = observations.get(key, [])
        if not isinstance(values, list) or len(values) > 128 or any(not isinstance(item, str) or not 1 <= len(item) <= 256 for item in values):
            raise ValueError("observation-budget")
    for key in ("wifi_trust", "rsd_identity", "identity"):
        if key in observations and type(observations[key]) is not bool:
            raise ValueError("observation-type")
    evidence = value["evidence"]
    if not isinstance(evidence, dict) or len(evidence) > 16:
        raise ValueError("evidence-schema")
    # Evidence files are bounded and hash-bound, and remain local. They describe
    # externally collected protocol observations; they never turn missing code
    # into a passing adapter.
    for key, record in evidence.items():
        if not re.fullmatch(r"[a-z][a-z0-9-]{1,63}", key) or not isinstance(record, dict) or set(record) != {"path", "sha256"}:
            raise ValueError("evidence-record")
        path = fixture_path(root, record["path"])
        if path.stat().st_size > MANIFEST_LIMIT or not re.fullmatch(r"[0-9a-f]{64}", record["sha256"]) or digest(path) != record["sha256"]:
            raise ValueError("evidence-integrity")
    context.fixtures = admitted
    context.observed_services = frozenset(observations.get("services", []))
    context.observed_modes = frozenset(observations.get("modes", []))
    context.device.has_wifi_trust = observations.get("wifi_trust", False)
    context.device.has_rsd_identity = observations.get("rsd_identity", False)
    context.device.has_identity = observations.get("identity", False)
    context.evidence = evidence
    context.fixture_manifest_sha256 = digest(manifest)


def command(context, *arguments):
    return [str(context.binary), "--udid", context.device.udid, "--error-format", "json", *map(str, arguments)]


def query_command(case_id, context):
    if case_id in UNSUPPORTED:
        raise ValueError("adapter-requires-operation-sequence")
    if case_id in ("traditional-usb", "traditional-wifi"):
        return command(context, "info", "lockdown"), lambda result: (
            isinstance(checked_json(result), dict)
            and checked_json(result).get("UniqueDeviceID") == context.device.udid
            and isinstance(checked_json(result).get("ProductVersion"), str))
    if case_id == "coredevice-usb":
        return command(context, "apps", "list", "--coredevice"), lambda result: isinstance(checked_json(result), list)
    if case_id == "remoted-coexistence":
        route = context.fixtures.get("existing-rsd-route")
        args = ["doctor"]
        if route:
            args += ["--rsd-address", route["address"]]
            if "proxy_port" in route:
                args += ["--rsd-proxy-port", route["proxy_port"]]
        return command(context, *args), lambda result: (
            isinstance(checked_json(result), dict)
            and next((row.get("status") for row in checked_json(result).get("checks", []) if row.get("id") == "remoted_coexistence"), None) == "ok"
            and next((row.get("status") for row in checked_json(result).get("device", {}).get("checks", []) if row.get("id") == "rsd_services"), None) == "ok")
    raise ValueError("adapter-requires-operation-sequence")


class OperationRun:
    def __init__(self, session, case_id, context):
        self.session, self.case_id, self.context = session, case_id, context
        self.steps = []
        self.deadline = time.monotonic() + 90
        self.cleanup_complete = True

    def step(self, argv, contract, expected_code=0, cleanup=False):
        # Cleanup has a separate, small ceiling even when the operation budget
        # expired. Only this run's explicit cleanup argv may use that reserve.
        remaining = 10 if cleanup else self.deadline - time.monotonic()
        if remaining <= 0:
            raise ValueError("operation-deadline")
        result = capture(argv, env=self.context.environment, timeout=min(remaining, 70), max_bytes=MANIFEST_LIMIT)
        try:
            matched = bool(contract(result))
        except Exception:
            matched = False
        checked = assess(result, self.session.variants, expected_code)
        passed = checked.pop("passed") and matched
        self.steps.append(dict(checked, contract=matched, passed=passed, expected_code=expected_code,
                               duration_ms=result.duration_ms, argv=self.session.arguments(argv), phase="cleanup" if cleanup else "operation"))
        self.cleanup_complete = self.cleanup_complete and result.cleanup_complete
        if not passed:
            raise ValueError("operation-contract-failed")
        return result

    def finish(self, error=None):
        report = dict(id=self.case_id, status="PASS" if error is None and self.cleanup_complete else "FAIL",
                      evidence_origin="localhost-protocol-fixture" if self.context.fixture_peer else "selected-device-cli",
                      contract=error is None, cleanup_complete=self.cleanup_complete,
                      steps=self.steps, reason=None if error is None else "operation-or-cleanup-failed")
        self.session.results[self.case_id] = report
        return report


class OwnedFiles:
    """A newly created directory and exact known files; unknown residue is kept."""
    def __init__(self, parent):
        self.root = Path(tempfile.mkdtemp(prefix="ios-matrix-owned-", dir=parent))
        self.identity = (self.root.stat().st_dev, self.root.stat().st_ino)
        self.files = {}
        self.directories = {self.root}

    def destination(self, relative):
        path = self.root / relative
        if self.root not in path.parents or ".." in path.parts:
            raise ValueError("owned-file-path")
        current = path.parent
        missing = []
        while current not in self.directories:
            missing.append(current)
            current = current.parent
        for directory in reversed(missing):
            directory.mkdir()
            self.directories.add(directory)
        return path

    def admit(self, path):
        value = path.lstat()
        if not stat.S_ISREG(value.st_mode) or path.is_symlink():
            raise ValueError("owned-output-not-regular")
        self.files[path] = (value.st_dev, value.st_ino)

    def clean(self):
        good = True
        for path, identity in self.files.items():
            try:
                value = path.lstat()
                if (value.st_dev, value.st_ino) != identity or not stat.S_ISREG(value.st_mode) or path.is_symlink():
                    good = False
                    continue
                path.chmod(stat.S_IWRITE | stat.S_IREAD)
                path.unlink()
            except FileNotFoundError:
                pass
            except OSError:
                good = False
        for directory in sorted(self.directories, key=lambda path: len(path.parts), reverse=True):
            try:
                if directory.is_symlink() or (directory == self.root and (directory.stat().st_dev, directory.stat().st_ino) != self.identity):
                    good = False
                    continue
                directory.rmdir()  # Never recursively delete unknown temp/user files.
            except OSError:
                good = False
        return good


def file_workflow(run, case_id):
    fixture = run.context.fixtures["afc-fixture"]
    parent = run.context.fixtures.get("authorized-unc-share", {}).get("directory") if case_id == "windows-afc-unc" else run.session.private.parent
    owned = OwnedFiles(parent)
    error = None
    try:
        relative = "synthetic-文件.bin" if case_id == "windows-afc-unicode" else "download.bin"
        if case_id == "windows-afc-long":
            relative = "/".join(["synthetic-long-component-" + str(index) for index in range(12)]) + "/download.bin"
        path = owned.destination(relative)
        argv_path = str(path)
        if case_id == "windows-afc-verbatim":
            argv_path = "\\\\?\\" + str(path.resolve())
        before = b"synthetic-original-local-target"
        existing = case_id in ("windows-afc-existing", "windows-afc-force", "windows-afc-readonly", "windows-afc-recovery")
        if existing:
            with path.open("xb") as output:
                output.write(before)
            owned.admit(path)
        if case_id == "windows-afc-readonly":
            path.chmod(stat.S_IREAD)

        def valid_output(_):
            owned.admit(path)
            return path.stat().st_size == fixture["size"] and digest(path) == fixture["sha256"] and set(path.parent.iterdir()) == {path}

        def retained_output(result):
            error = json.loads(result.stderr.decode("utf-8").splitlines()[-1]).get("error", {})
            expected = "permission_denied" if case_id == "windows-afc-readonly" else "precondition"
            return path.read_bytes() == before and set(path.parent.iterdir()) == {path} and error.get("phase") == "file" and error.get("class") == expected

        force = case_id in ("windows-afc-force", "windows-afc-readonly")
        argv = command(run.context, "file", "pull", fixture["remote_path"], argv_path, *( ["--force"] if force else []))
        if case_id in ("windows-afc-existing", "windows-afc-readonly", "windows-afc-recovery"):
            run.step(argv, retained_output, expected_code=1)
            if case_id == "windows-afc-recovery":
                run.step(command(run.context, "file", "pull", fixture["remote_path"], argv_path, "--force"), valid_output)
        else:
            run.step(argv, valid_output)
        if case_id == "windows-afc-readonly":
            directory = owned.root / "readonly-directory"
            marker = owned.destination("readonly-directory/marker")
            marker.write_bytes(before)
            owned.admit(marker)
            def retained_directory(result):
                error = json.loads(result.stderr.decode("utf-8").splitlines()[-1]).get("error", {})
                # The current file replacement guard reports a local non-file
                # refusal as unknown; native read-only-file denial is separately
                # required to retain permission_denied above.
                return marker.read_bytes() == before and set(directory.iterdir()) == {marker} and error.get("phase") == "file" and error.get("class") == "unknown" and b"Downloading " in result.stderr and set(owned.root.iterdir()) == {path, directory}
            run.step(command(run.context, "file", "pull", fixture["remote_path"], directory, "--force"), retained_directory, expected_code=1)
        run.steps[-1]["integrity"] = "original-preserved" if case_id in ("windows-afc-existing", "windows-afc-readonly") else "full-size-and-sha256"
    except Exception as caught:
        error = caught
        # A failed CLI may have created its destination. Admit only a regular
        # exact destination in our exclusively owned directory, never its temp
        # residue or any other path.
        if 'path' in locals() and path.exists() and not path.is_symlink():
            owned.admit(path)
    finally:
        run.cleanup_complete = owned.clean() and run.cleanup_complete
    return run.finish(error)


def application_workflow(run):
    fixture = run.context.fixtures["test-application"]
    if digest(fixture["path"]) != fixture["sha256"] or package_id(fixture["path"]) != fixture["bundle_id"]:
        return run.finish(ValueError("package-changed-after-admission"))
    bundle_id = fixture["bundle_id"]
    owned = False
    pid = None
    error = None
    try:
        before = checked_json(run.step(command(run.context, "apps", "list", "--app-type", "all"), lambda result: isinstance(checked_json(result), list)))
        if any(row.get("bundle_id") == bundle_id or row.get("CFBundleIdentifier") == bundle_id for row in before):
            raise ValueError("application-already-exists-no-owned-install")
        owned = True  # A partial installation must also be removed.
        run.step(command(run.context, "apps", "install", fixture["path"], "--streaming"), lambda result: checked_json(result).get("operation") == "install")
        run.step(command(run.context, "apps", "show", bundle_id), lambda result: isinstance(checked_json(result), dict) and checked_json(result).get("bundle_id") == bundle_id)
        result = run.step(command(run.context, "apps", "launch", bundle_id), lambda result: type(checked_json(result).get("pid")) is int and checked_json(result)["pid"] > 0)
        pid = checked_json(result)["pid"]
        run.step(command(run.context, "apps", "kill", pid), lambda result: checked_json(result).get("operation") == "kill" and checked_json(result).get("pid") == pid)
        pid = None
    except Exception as caught:
        error = caught
    finally:
        if pid is not None:
            try:
                run.step(command(run.context, "apps", "kill", pid), lambda result: result.returncode == 0, cleanup=True)
            except Exception as caught:
                run.cleanup_complete = False
                error = error or caught
        if owned:
            try:
                run.step(command(run.context, "apps", "uninstall", bundle_id), lambda result: checked_json(result).get("operation") == "uninstall" and checked_json(result).get("bundle_id") == bundle_id, cleanup=True)
            except Exception as caught:
                run.cleanup_complete = False
                error = error or caught
            try:
                run.step(command(run.context, "apps", "list", "--app-type", "all"), lambda result: isinstance(checked_json(result), list) and not any(row.get("bundle_id") == bundle_id or row.get("CFBundleIdentifier") == bundle_id for row in checked_json(result)), cleanup=True)
            except Exception as caught:
                run.cleanup_complete = False
                error = error or caught
    return run.finish(error)


def xctest_workflow(run):
    from xml.etree import ElementTree
    fixture = run.context.fixtures["test-bundle"]
    owned = OwnedFiles(run.session.private.parent)
    error = None
    try:
        output = owned.destination("results.xml")
        logs = owned.destination("syslog.txt")
        arguments = ["runxctest", "--test-runner-bundle-id", fixture["runner_bundle_id"],
                     "--xctest-config", fixture["xctest_config"], "--xctest", "--wait",
                     "--timeout", fixture["timeout_secs"], "--junit-output", output, "--log-output", logs]
        for selector in fixture["tests"]:
            arguments += ["--test", selector]
        def valid_result(result):
            owned.admit(output)
            owned.admit(logs)
            tree = ElementTree.parse(output)
            cases = tree.findall(".//testcase")
            selectors = {row.get("classname", "") + "/" + row.get("name", "") for row in cases}
            return set(fixture["tests"]) <= selectors and not tree.findall(".//failure") and not tree.findall(".//error") and logs.stat().st_size <= FILE_LIMIT and b"log capture finished" in logs.read_bytes()[-256:]
        run.step(command(run.context, *arguments), valid_result)
    except Exception as caught:
        error = caught
    finally:
        for path in (output, logs):
            if path.exists() and not path.is_symlink():
                owned.admit(path)
        run.cleanup_complete = owned.clean() and run.cleanup_complete
    return run.finish(error)


def execute(session, case, context):
    case_id = case["id"]
    if case_id in UNSUPPORTED:
        return session.record_not_run(case_id, UNSUPPORTED[case_id], ["production-protocol-observer"])
    if case_id == "remoted-coexistence" and "existing-rsd-route" not in context.fixtures:
        return session.record_not_run(case_id, "existing-selected-rsd-route-required", ["existing-rsd-route"])
    run = OperationRun(session, case_id, context)
    if case_id == "file-workflow" or case_id.startswith("windows-afc-"):
        return file_workflow(run, case_id)
    if case_id == "application-workflow":
        return application_workflow(run)
    if case_id == "xctest-workflow":
        return xctest_workflow(run)
    try:
        argv, contract = query_command(case_id, context)
        run.step(argv, contract)
        return run.finish()
    except Exception as error:
        return run.finish(error)
