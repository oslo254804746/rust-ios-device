"""Synthetic installation and XCTest services reached by the actual CLI.

Every socket belongs to CliAfcPeer's original thread/join/EOF owner. The peer
decodes the transmitted IPA and DTX arguments, rather than returning CLI-shaped
JSON. It supplies no hardware acceptance or Apple signature evidence.
"""
import binascii
import json
import plistlib
import struct

from regression_cli_fixture import CliAfcPeer, exact, recv_plist, send_plist

APP_ID = "com.synthetic.MatrixApp"
RUNNER_ID = "com.synthetic.MatrixRunner"
PID = 40123
PROCESS = "com.apple.instruments.server.services.processcontrol"
DAEMON = "dtxproxy:XCTestManager_IDEInterface:XCTestManager_DaemonConnectionInterface"
DRIVER = "dtxproxy:XCTestDriverInterface:XCTestManager_IDEInterface"
LAUNCH = "launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:"


def archive(value):
    return plistlib.dumps({"$archiver": "NSKeyedArchiver", "$version": 100000,
                          "$top": {"root": plistlib.UID(1)}, "$objects": ["$null", value]},
                         fmt=plistlib.FMT_BINARY)


def unarchive(data):
    archive_data = plistlib.loads(data)
    objects = archive_data["$objects"]
    if len(objects) > 4096:
        raise ValueError("fixture-archive-object-bound")

    def decode(value, depth=0):
        if depth > 32:
            raise ValueError("fixture-archive-depth")
        if isinstance(value, plistlib.UID):
            if value.data >= len(objects):
                raise ValueError("fixture-archive-reference")
            return decode(objects[value.data], depth + 1)
        if isinstance(value, list):
            return [decode(item, depth + 1) for item in value]
        if isinstance(value, dict):
            if "NS.keys" in value and "NS.objects" in value:
                return dict(zip(decode(value["NS.keys"], depth + 1), decode(value["NS.objects"], depth + 1)))
            if "NS.objects" in value:
                return decode(value["NS.objects"], depth + 1)
            return {key: decode(item, depth + 1) for key, item in value.items() if key != "$class"}
        return value
    return decode(archive_data["$top"]["root"])


def read_dtx(stream):
    header = exact(stream, 32)
    if header[:4] != bytes.fromhex("795b3d1f"):
        raise ValueError("fixture-dtx-magic")
    header_size, fragment, count, size, identifier, conversation, channel, reply = struct.unpack("<IHHIIIiI", header[4:])
    if header_size != 32 or fragment != 0 or count != 1 or not 16 <= size <= 1024 * 1024:
        raise ValueError("fixture-dtx-framing")
    body = exact(stream, size)
    kind, aux_size, total_size, flags = struct.unpack("<IIII", body[:16])
    if flags != 0 or total_size != len(body) - 16 or aux_size > total_size:
        raise ValueError("fixture-dtx-payload")
    args = []
    if aux_size:
        aux = body[16:16 + aux_size]
        if len(aux) < 16 or struct.unpack("<IIII", aux[:16]) != (496, 0, len(aux) - 16, 0):
            raise ValueError("fixture-dtx-aux-header")
        offset = 16
        while offset < len(aux):
            null, code = struct.unpack("<II", aux[offset:offset + 8])
            offset += 8
            if null != 10:
                raise ValueError("fixture-dtx-argument-key")
            if code == 2:
                size = struct.unpack("<I", aux[offset:offset + 4])[0]
                offset += 4
                if size > len(aux) - offset:
                    raise ValueError("fixture-dtx-argument-size")
                args.append(unarchive(aux[offset:offset + size]))
                offset += size
            elif code in (3, 6, 9):
                fmt = {3: "<i", 6: "<q", 9: "<d"}[code]
                size = struct.calcsize(fmt)
                args.append(struct.unpack(fmt, aux[offset:offset + size])[0])
                offset += size
            else:
                raise ValueError("fixture-dtx-argument-type")
        if offset != len(aux):
            raise ValueError("fixture-dtx-argument-trailing")
    payload = body[16 + aux_size:]
    return dict(identifier=identifier, conversation=conversation, channel=channel,
                reply=bool(reply), kind=kind, value=unarchive(payload) if payload else None, args=args)


def send_dtx(stream, identifier, conversation=0, channel=1, kind=2, value=None, args=(), reply=False):
    payload = archive(value) if value is not None else b""
    entries = b""
    for arg in args:
        encoded = archive(arg)
        entries += struct.pack("<III", 10, 2, len(encoded)) + encoded
    aux = struct.pack("<IIII", 496, 0, len(entries), 0) + entries if entries else b""
    body = struct.pack("<IIII", kind, len(aux), len(aux) + len(payload), 0) + aux + payload
    stream.sendall(bytes.fromhex("795b3d1f") + struct.pack("<IHHIIIiI", 32, 0, 1, len(body), identifier,
                                                         conversation, channel, int(reply)) + body)


def respond(stream, request, value=None):
    send_dtx(stream, request["identifier"], request["conversation"] + 1, request["channel"], 3, value)


class CliWorkflowPeer(CliAfcPeer):
    services = dict(CliAfcPeer.services, **{
        "com.apple.mobile.installation_proxy": 62081,
        "com.apple.streaming_zip_conduit": 62082,
        "com.apple.instruments.remoteserver.DVTSecureSocketProxy": 62083,
        "com.apple.testmanagerd.lockdown.secure": 62084,
        "com.apple.syslog_relay": 62085,
        "com.apple.crashreportmover": 62086,
        "com.apple.crashreportcopymobile": 62087,
    })

    def __init__(self, directory, *, preexisting=False, install_error=False, test_error=False,
                 fault=None, runner_preexisting=True, first_test_fails=False, crash_fault=None,
                 terminal_scenario=None, rerun_terminal_scenario=None):
        self.crash_fault, self.crash_copies, self.crash_movers = crash_fault, 0, 0
        self.crash_events = []
        self.apps = {RUNNER_ID: self.app_record(RUNNER_ID)} if runner_preexisting else {}
        if preexisting:
            self.apps[APP_ID] = self.app_record(APP_ID)
        self.install_error, self.test_error = install_error, test_error
        self.fault, self.first_test_fails, self.test_attempts = fault, first_test_fails, 0
        self.terminal_scenario, self.rerun_terminal_scenario = terminal_scenario, rerun_terminal_scenario
        self.calls, self.transferred, self.pids, self.configurations = [], {}, set(), []
        super().__init__(directory)

    @staticmethod
    def app_record(bundle_id):
        return {"CFBundleIdentifier": bundle_id, "CFBundleDisplayName": "Synthetic Matrix",
                "CFBundleExecutable": "SyntheticRunner", "CFBundleVersion": "1", "ApplicationType": "User",
                "Path": "/private/synthetic/" + bundle_id + ".app", "Container": "/private/synthetic/container"}

    def device_service(self, port, stream):
        # A 5 s CLI total includes setup before these service reads and an
        # independent cleanup phase afterwards. Keep the fixture observer alive
        # through that window; only original client EOF can satisfy its joins.
        # The CLI still has its 5 s deadline, <9 s duration assertion and 12 s
        # owned process supervisor. This socket timeout never supplies success.
        stream.settimeout(10)
        if port == 62081:
            return self.installation(stream)
        if port == 62082:
            return self.zipconduit(stream)
        if port == 62083:
            return self.instruments(stream)
        if port == 62084:
            return self.testmanager(stream)
        if port == 62085:
            if self.fault == "log-eof":
                stream.shutdown(1)
                exact(stream, 1)
                raise ValueError("fixture-unexpected-log-write")
            stream.sendall(b"Oct 11 12:00:00 synthetic Synthetic[1] <Notice>: matrix-log\0")
            exact(stream, 1)  # Original owner must close; no locally manufactured EOF.
            raise ValueError("fixture-unexpected-syslog-write")
        if port == 62086:
            self.crash_movers += 1
            self.crash_events.append(("mover", self.crash_movers))
            stream.sendall(b"pong" if self.crash_fault == "mover-before" and self.crash_movers == 1 else b"ping")
            exact(stream, 1)
            raise ValueError("fixture-unexpected-crash-mover-write")
        if port == 62087:
            self.crash_copies += 1
            return self.crash_afc(stream, self.crash_copies)
        return super().device_service(port, stream)

    @staticmethod
    def crash_report(bundle_id, *, padding=0):
        body = {"procName": "SyntheticRunner", "pid": PID, "bundleInfo": {}, "threads": [], "usedImages": []}
        if bundle_id is not None:
            body["bundleInfo"]["CFBundleIdentifier"] = bundle_id
        if padding:
            body["diagnosticPadding"] = "p" * padding
        return (json.dumps({"bug_type": "309"}) + "\n" + json.dumps(body)).encode()

    def crash_afc(self, stream, phase):
        reports = {"existing.ips": self.crash_report(RUNNER_ID)}
        if phase > 1:
            reports.update({"matched.ips": self.crash_report(APP_ID),
                            "unrelated.ips": self.crash_report("com.synthetic.Other"),
                            "unknown.ips": self.crash_report(None)})
            # Same path recreated with another report is not a new path.
            reports["existing.ips"] = self.crash_report(APP_ID)
            if self.crash_fault == "count":
                reports.update({f"candidate-{index}.ips": self.crash_report(APP_ID) for index in range(65)})
            if self.crash_fault == "partial-read":
                reports["matched.ips"] = self.crash_report(APP_ID, padding=80000)
            if self.crash_fault in ("growth", "growth-close"):
                reports["matched.ips"] = self.crash_report(APP_ID, padding=4 * 1024 * 1024)
        opened, offset, handle = None, 0, 17
        while True:
            magic, entire, this, packet, opcode = struct.unpack("<QQQQQ", exact(stream, 40))
            if magic != 0x4141504C36414643 or not 40 <= this <= entire <= 131072:
                raise ValueError("fixture-crash-afc-framing")
            body = exact(stream, entire - 40)
            hp = body[:this - 40]
            rh, rd, ro = b"", b"", 2
            self.crash_events.append(("afc", phase, opcode))
            if opcode == 3:
                if hp != b".\0":
                    raise ValueError("fixture-crash-directory-scope")
                if self.crash_fault == ("list-before" if phase == 1 else "list-after"):
                    rh, ro = struct.pack("<Q", 5), 1
                else:
                    names = list(reports)
                    if phase > 1 and self.crash_fault == "unsafe":
                        names.append("../escape.ips")
                    rd = ("\0".join([".", ".."] + names) + "\0").encode()
            elif opcode == 10:
                path = hp.removesuffix(b"\0").decode()
                if path not in reports:
                    raise ValueError("fixture-crash-stat-scope")
                size = len(reports[path])
                if phase > 1 and path == "matched.ips":
                    if self.crash_fault == "oversize":
                        size = 4 * 1024 * 1024 + 1
                    if self.crash_fault in ("growth", "growth-close"):
                        size = 1
                rd = b"st_ifmt\0S_IFREG\0st_size\0" + str(size).encode() + b"\0st_mtime\0" + b"1\0"
            elif opcode == 13:
                mode = struct.unpack("<Q", hp[:8])[0]
                path = hp[8:].removesuffix(b"\0").decode()
                if mode != 1 or path not in reports or opened is not None or path == "existing.ips":
                    raise ValueError("fixture-crash-open-scope")
                opened, offset = path, 0
                self.crash_events.append(("open", phase, path))
                rh, ro = struct.pack("<Q", handle), 14
            elif opcode == 15:
                fd, amount = struct.unpack("<QQ", hp)
                if fd != handle or opened is None or not 1 <= amount <= 65536:
                    raise ValueError("fixture-crash-read-scope")
                if opened == "matched.ips" and (self.crash_fault == "read" or self.crash_fault == "partial-read" and offset > 0):
                    rh, ro = struct.pack("<Q", 5), 1
                else:
                    rd = reports[opened][offset:offset + amount]
                    offset += len(rd)
            elif opcode == 20:
                if hp != struct.pack("<Q", handle) or opened is None:
                    raise ValueError("fixture-crash-close-scope")
                self.crash_events.append(("close", phase, opened))
                opened = None
                rh, ro = struct.pack("<Q", 5 if self.crash_fault in ("close", "growth-close") else 0), 1
            else:
                raise ValueError("fixture-crash-readonly-operation")
            stream.sendall(struct.pack("<QQQQQ", magic, 40 + len(rh) + len(rd), 40 + len(rh), packet, ro) + rh + rd)

    def installation(self, stream):
        while True:
            request = recv_plist(stream)
            operation = request.get("Command")
            self.calls.append(operation)
            if operation == "Browse":
                send_plist(stream, {"CurrentList": list(self.apps.values()), "Status": "Complete"})
            elif operation == "Lookup":
                selected = request.get("ClientOptions", {}).get("BundleIDs", [])
                execution_lookup = "ReturnAttributes" in request.get("ClientOptions", {})
                missing = (RUNNER_ID if self.fault == "lookup-runner" else APP_ID if self.fault == "lookup-target" else None) if execution_lookup else None
                send_plist(stream, {"LookupResult": {key: self.apps[key] for key in selected if key in self.apps and key != missing}, "Status": "Complete"})
            elif operation == "Uninstall":
                bundle_id = request.get("ApplicationIdentifier")
                if bundle_id not in (APP_ID, RUNNER_ID):
                    raise ValueError("fixture-uninstall-nonowned-bundle")
                if self.fault == "uninstall" and bundle_id == RUNNER_ID:
                    send_plist(stream, {"Error": "SyntheticUninstallRejected"})
                else:
                    self.apps.pop(bundle_id, None)
                    send_plist(stream, {"Status": "Complete"})
            else:
                raise ValueError("fixture-installation-operation")

    def zipconduit(self, stream):
        init = recv_plist(stream)
        if init.get("InstallTransferredDirectory") != 1 or init.get("MediaSubdir", "").split("/")[-1] not in ("wrong-name.ipa", "wrong-runner-name.ipa"):
            raise ValueError("fixture-zip-init")
        entries, total = {}, 0
        while True:
            signature = exact(stream, 4)
            if signature == b"PK\x01\x02":
                break
            if signature != b"PK\x03\x04" or len(entries) >= 64:
                raise ValueError("fixture-zip-entry")
            header = exact(stream, 26)
            version, flags, method, clock, date, crc, compressed, expanded, name_size, extra_size = struct.unpack("<HHHHHIIIHH", header)
            if method != 0 or flags != 0 or compressed != expanded or expanded > 1024 * 1024 or name_size > 4096 or extra_size > 4096:
                raise ValueError("fixture-zip-header")
            name = exact(stream, name_size).decode()
            exact(stream, extra_size) if extra_size else None
            value = exact(stream, compressed) if compressed else b""
            total += compressed
            if total > 2 * 1024 * 1024 or name in entries or binascii.crc32(value) != crc:
                raise ValueError("fixture-zip-integrity")
            entries[name] = value
        info = plistlib.loads(entries["Payload/Synthetic.app/Info.plist"])
        bundle_id = info.get("CFBundleIdentifier")
        if bundle_id not in (APP_ID, RUNNER_ID):
            raise ValueError("fixture-zip-bundle-id")
        self.transferred = entries
        self.apps[bundle_id] = self.app_record(bundle_id)  # Failure can follow partial device effects.
        self.calls.append("InstallTransferred")
        if self.install_error or (self.fault == "install-runner" and bundle_id == RUNNER_ID):
            send_plist(stream, {"InstallProgressDict": {"Error": "SyntheticInstallRejected"}})
        else:
            send_plist(stream, {"Status": "DataComplete"})
        exact(stream, 1)
        raise ValueError("fixture-unexpected-zip-write")

    def instruments(self, stream):
        while True:
            request = read_dtx(stream)
            selector, args = request["value"], request["args"]
            self.calls.append(selector)
            if selector == "_requestChannelWithCode:identifier:" and args == [1, PROCESS]:
                respond(stream, request)
            elif selector == LAUNCH:
                if len(args) != 5 or args[1] not in (APP_ID, RUNNER_ID) or args[1] not in self.apps:
                    raise ValueError("fixture-launch-bundle")
                self.pids.add(PID)
                respond(stream, request, PID)
            elif selector == "killPid:" and args == [PID]:
                self.pids.discard(PID)
            elif selector == "requestDisableMemoryLimitsForPid:" and args == [PID]:
                respond(stream, request, True)
            else:
                raise ValueError("fixture-process-operation")

    def testmanager(self, stream):
        session = False
        attempt = 0
        while True:
            request = read_dtx(stream)
            selector, args = request["value"], request["args"]
            self.calls.append(selector)
            if request["kind"] == 3:
                if isinstance(request["value"], dict):
                    self.configurations.append(request["value"])
                continue
            if request["kind"] == 0:
                continue
            failed_stage = {"channel": "_requestChannelWithCode:identifier:",
                            "session": "_IDE_initiateSessionWithIdentifier:capabilities:",
                            "authorize": "_IDE_authorizeTestSessionWithProcessID:"}.get(self.fault)
            if selector == failed_stage:
                send_dtx(stream, request["identifier"], request["conversation"] + 1,
                         request["channel"], 4, "synthetic-test-stage-rejected")
                continue
            if selector == "_requestChannelWithCode:identifier:" and args == [1, DAEMON]:
                respond(stream, request)
            elif selector == "_IDE_initiateControlSessionWithCapabilities:" and len(args) == 1:
                respond(stream, request)
            elif selector == "_IDE_authorizeTestSessionWithProcessID:" and args == [PID]:
                respond(stream, request, True)
            elif selector == "_IDE_initiateSessionWithIdentifier:capabilities:" and len(args) == 2:
                session = True
                self.test_attempts += 1
                attempt = self.test_attempts
                if self.fault == "deadline-session":
                    exact(stream, 1)  # CLI total deadline must close the original peer.
                    raise ValueError("fixture-unexpected-stalled-session-write")
                if self.fault == "rerun-session" and attempt > 1:
                    send_dtx(stream, request["identifier"], request["conversation"] + 1,
                             request["channel"], 4, "synthetic-rerun-startup-rejected")
                    continue
                respond(stream, request)
                send_dtx(stream, 501, value="_requestChannelWithCode:identifier:", args=(1, DRIVER), reply=True)
                send_dtx(stream, 502, value="_XCT_testBundleReadyWithProtocolVersion:minimumVersion:", args=(36, 29), reply=True)
                send_dtx(stream, 503, value="_XCT_testRunnerReadyWithCapabilities:", args=({},), reply=True)
            elif selector == "_IDE_startExecutingTestPlanWithProtocolVersion:" and session and args == [36]:
                if self.fault == "start":
                    stream.sendall(b"bad!")
                    stream.shutdown(1)
                    exact(stream, 1)
                    raise ValueError("fixture-unexpected-start-write")
                failure = self.test_error or (self.first_test_fails and attempt == 1)
                scenario = self.rerun_terminal_scenario if attempt > 1 else self.terminal_scenario
                failure = failure or scenario in ("failed", "failed-zero-counter", "expected-failure")
                events = [("_XCT_didBeginExecutingTestPlan", ()),
                          ("_XCT_testCaseDidStartForTestClass:method:", ("MatrixTests", "testExample"))]
                if failure:
                    events.append(("_XCT_testCaseDidFailForTestClass:method:withMessage:file:line:",
                                   ("MatrixTests", "testExample", "synthetic assertion", "Synthetic.swift", 4)))
                status = {"stalled": "stalled", "unknown": "synthetic-new-status", "skipped": "skipped",
                          "expected-failure": "expected failure"}.get(scenario, "failed" if failure else "passed")
                if scenario != "unfinished":
                    events.append(("_XCT_testCaseDidFinishForTestClass:method:withStatus:duration:",
                                   ("MatrixTests", "testExample", status, 0.01)))
                if scenario in ("failed-zero-counter", "uncaught"):
                    events.append(("_XCT_testSuiteWithIdentifier:didFinishAt:runCount:skipCount:failureCount:expectedFailureCount:uncaughtExceptionCount:testDuration:totalDuration:",
                                   ("MatrixTests", "synthetic-finish", 1, 0, 0, 0, int(scenario == "uncaught"), 0.01, 0.01)))
                if scenario == "unexpected-zero-counter":
                    events.append(("_XCT_testSuite:didFinishAt:runCount:withFailures:unexpected:testDuration:totalDuration:",
                                   ("MatrixTests", "synthetic-finish", 1, 0, 1, 0.01, 0.01)))
                if scenario == "empty":
                    events = events[:1]
                if scenario == "finish-only":
                    events = []
                events.append(("_XCT_didFinishExecutingTestPlan", ()))
                for index, (method, values) in enumerate(events, 600):
                    send_dtx(stream, index, channel=-1, value=method, args=values, reply=True)
                    if index == 601 and (self.fault in ("result", "deadline-result") or (self.fault == "rerun-result" and attempt > 1)):
                        if self.fault != "deadline-result":
                            stream.shutdown(1)
                        # XCTest acknowledges both observed events before it
                        # observes EOF; retain them instead of mistaking the
                        # ACK bytes for an unexpected operation.
                        while True:
                            ack = read_dtx(stream)
                            if ack["kind"] != 0:
                                raise ValueError("fixture-unexpected-result-write")
            else:
                raise ValueError("fixture-testmanager-operation")
