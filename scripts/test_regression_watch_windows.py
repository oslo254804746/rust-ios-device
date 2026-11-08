"""Original Windows console/job/thread tests, including actual CLI startup IO.

Synthetic JSON children verify the supervisor only. A separate actual CLI gate
uses real localhost TLS Lockdown and requires independent strict peer EOF.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest

from diagnostics import ROOT
from regression_cli_fixture import CliAfcPeer, UDID, exact
from regression_watch_capture import OwnedWatch

CLI = Path(os.environ.get("IOS_REGRESSION_CLI", ROOT / "target/debug/ios.exe"))


class WatchBudgetTests(unittest.TestCase):
    def test_invalid_budgets_fail_before_creating_any_process(self):
        for budget in (0, -1, float("inf"), float("nan"), 86401):
            with self.subTest(timeout=budget), self.assertRaises(ValueError):
                OwnedWatch(["nonexistent-invalid-budget-fixture"], {}, timeout=budget)
        for limit in (0, -1, 1.5, 16 * 1024 * 1024 + 1):
            with self.subTest(max_bytes=limit), self.assertRaises(ValueError):
                OwnedWatch(["nonexistent-invalid-budget-fixture"], {}, max_bytes=limit)


@unittest.skipUnless(os.name == "nt", "Windows original console and Job Object boundary")
class WindowsWatchOwnerTests(unittest.TestCase):
    @staticmethod
    def child(body="", stop="", *, ignore=False):
        return ("import json,os,signal,sys,time,subprocess\n"
                "def stop(signum,frame):\n"
                + (" " + stop.replace("\n", "\n ") + "\n" if stop else " pass\n")
                + " print(json.dumps({'state':'stopped','native_reclaimed':True}),file=sys.stderr,flush=True)\n"
                " sys.exit(0)\n"
                + ("signal.signal(signal.SIGINT,signal.SIG_IGN)\n" if ignore else "signal.signal(signal.SIGINT,stop)\n")
                + body + "\n"
                "print(json.dumps({'state':'connected'}),file=sys.stderr,flush=True)\n"
                "print(json.dumps({'synthetic':'one'}),flush=True)\n"
                "time.sleep(30)\n")

    def start(self, script, *, timeout=8, max_bytes=65536):
        watch = OwnedWatch([sys.executable, "-c", script], dict(os.environ), timeout=timeout, max_bytes=max_bytes)
        self.addCleanup(watch.finish)
        watch.wait(lambda rows: any(isinstance(row, dict) and row.get("state") == "connected" for row in rows[1])
                   and bool(rows[0]), min(5, timeout))
        return watch

    def test_private_ctrl_c_preserves_unrelated_process_and_original_reader_joins(self):
        unrelated = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            watch = self.start(self.child())
            result, receipt, records = watch.finish()
            self.assertTrue(receipt, result.stderr)
            self.assertEqual(result.returncode, 0)
            self.assertTrue(result.cleanup_complete)
            self.assertFalse(result.timed_out)
            self.assertFalse(result.incomplete)
            self.assertEqual(result.cleanup_scope, "windows-job")
            self.assertEqual(records, [{"synthetic": "one"}])
            self.assertTrue(all(not worker.is_alive() for worker in watch.workers))
            self.assertIs(watch.finish(), watch.finished)
            self.assertIsNone(unrelated.poll())
            owner = next(row for row in watch.records[1] if row.get("state") == "owner_ready")
            self.assertTrue(owner["private_console"])
            self.assertNotEqual(result.pid, watch.process.pid)
        finally:
            unrelated.terminate()
            unrelated.wait(timeout=3)

    def test_ignored_ctrl_c_forces_only_owned_job_and_never_native_success(self):
        watch = self.start(self.child(ignore=True), timeout=2)
        result, receipt, _ = watch.finish()
        self.assertFalse(receipt)
        self.assertTrue(result.timed_out)
        self.assertTrue(result.cleanup_complete)
        self.assertTrue(all(not worker.is_alive() for worker in watch.workers))

    def test_original_leader_receipt_cannot_hide_pipe_closed_live_descendant(self):
        with tempfile.TemporaryDirectory(prefix="owned-watch-descendant-") as directory:
            ready = str(Path(directory) / "ready")
            descendant = ("import signal,time,pathlib; signal.signal(signal.SIGINT,signal.SIG_IGN); "
                          "pathlib.Path(" + repr(ready) + ").write_bytes(b'ready'); time.sleep(30)")
            body = ("child=subprocess.Popen([sys.executable,'-c'," + repr(descendant) + "],"
                    "stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
                    "from pathlib import Path\n"
                    "until=time.monotonic()+5\n"
                    "while not Path(" + repr(ready) + ").is_file():\n"
                    " if time.monotonic() >= until: raise RuntimeError('descendant-not-ready')\n"
                    " time.sleep(0.01)")
            watch = self.start(self.child(body))
            result, receipt, _ = watch.finish()
            self.assertFalse(receipt)
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(result.cleanup_complete)
            reclaimed = next(row for row in watch.records[1] if row.get("state") == "host_reclaimed")
            self.assertTrue(reclaimed["forced"])
            self.assertTrue(reclaimed["job_empty"])

    def test_partial_final_ndjson_rejects_native_receipt(self):
        watch = self.start(self.child(stop="sys.stdout.write('partial'); sys.stdout.flush()"))
        result, receipt, _ = watch.finish()
        self.assertFalse(receipt)
        self.assertTrue(result.incomplete)
        self.assertTrue(result.cleanup_complete)

    def test_child_exit_without_native_receipt_is_not_cleanup_ack(self):
        script = "import json; print(json.dumps({'synthetic':'completed'}),flush=True)"
        watch = OwnedWatch([sys.executable, "-c", script], dict(os.environ), timeout=8)
        self.addCleanup(watch.finish)
        watch.process.wait(timeout=5)
        result, receipt, _ = watch.finish()
        self.assertFalse(receipt)
        self.assertEqual(result.returncode, 0)
        self.assertTrue(result.cleanup_complete)

    def test_invalid_executable_reaps_controller_without_native_receipt(self):
        watch = OwnedWatch([str(ROOT / "nonexistent-watch-executable.exe")], dict(os.environ), timeout=8)
        self.addCleanup(watch.finish)
        watch.process.wait(timeout=5)
        result, receipt, _ = watch.finish()
        self.assertFalse(receipt)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(result.cleanup_complete)
        self.assertTrue(any(row.get("state") == "controller_failed" for row in watch.records[1]))

    def test_overflow_reaps_original_owner_without_truncated_success(self):
        script = self.child(body="print(json.dumps({'large':'x'*4096}),flush=True)")
        watch = OwnedWatch([sys.executable, "-c", script], dict(os.environ), timeout=8, max_bytes=256)
        self.addCleanup(watch.finish)
        with self.assertRaises(ValueError):
            watch.wait(lambda _: False, 5)
        result, receipt, _ = watch.finish()
        self.assertTrue(result.overflow)
        self.assertFalse(receipt)
        self.assertTrue(result.cleanup_complete)
        self.assertTrue(all(not worker.is_alive() for worker in watch.workers))


class PendingWatchPeer(CliAfcPeer):
    def __init__(self, directory):
        self.proxy_requested = threading.Event()
        super().__init__(directory)

    def start_service_reply(self, service):
        if service == "com.apple.internal.devicecompute.CoreDeviceProxy":
            self.proxy_requested.set()
            # Hold the original TLS service-start read. Its caller cancellation
            # must release the original stream; local shutdown is not evidence.
            stream = self.connections[-1]
            exact(stream, 1)
            raise ValueError("unexpected-pending-watch-business-write")
        return super().start_service_reply(service)


@unittest.skipUnless(os.name == "nt" and CLI.is_file(), "Windows compiled CLI and isolated existing trust required")
class ActualCliWindowsWatchTests(unittest.TestCase):
    def test_actual_ctrl_c_during_tls_service_start_emits_native_stop_and_strict_peer_eof(self):
        with tempfile.TemporaryDirectory(prefix="ios-watch-console-cli-") as directory:
            peer = PendingWatchPeer(Path(directory) / "trust")
            watch = None
            try:
                watch = OwnedWatch([str(CLI.resolve()), "--udid", UDID, "--error-format", "json", "pasteboard", "watch",
                                    "--reconnect", "--connect-timeout", "20", "--cleanup-timeout", "5", "--timeout", "25"],
                                   peer.environment, timeout=30)
                self.assertTrue(peer.proxy_requested.wait(8), ("original TLS StartService not observed",
                                                              bytes(watch.buffers[1]), peer.errors))
                result, receipt, snapshots = watch.finish()
                self.assertTrue(receipt, result.stderr)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(result.cleanup_complete)
                self.assertFalse(result.timed_out)
                self.assertFalse(result.incomplete)
                self.assertFalse(snapshots)
                self.assertTrue(all(not worker.is_alive() for worker in watch.workers))
                stopped = [row for row in watch.records[1] if row.get("state") == "stopped"
                           and row.get("operation") != "owned_watch_supervisor"]
                self.assertEqual(len(stopped), 1)
                self.assertIs(stopped[0]["native_reclaimed"], True)
            finally:
                if watch is not None:
                    watch.finish()
                complete = peer.close()
            self.assertTrue(complete, peer.errors)


if __name__ == "__main__":
    unittest.main()
