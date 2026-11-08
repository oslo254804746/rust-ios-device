"""Direct Windows capture ownership; synthetic host children, no device IO."""
import ctypes
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import diagnostics
import regression

if os.name == "nt":
    import regression_windows_process as win


@unittest.skipUnless(os.name == "nt", "Windows original Job/handle capture")
class WindowsCaptureTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="capture-owner-")
        self.addCleanup(self.directory.cleanup)
        self.ready = Path(self.directory.name) / "ready"
        self.owners = []
        original = win.CapturedProcess

        def observed(*args, **kwargs):
            owner = original(*args, **kwargs)
            owner.original_handles = (owner.handle, owner.job.handle)
            self.owners.append(owner)
            return owner

        self.factory = patch.object(win, "CapturedProcess", side_effect=observed)
        self.factory.start()
        self.addCleanup(self.factory.stop)

    def assert_closed(self, result, *, incomplete=False):
        self.assertTrue(result.cleanup_complete)
        self.assertEqual(result.cleanup_scope, "windows-job")
        self.assertEqual(result.incomplete, incomplete)
        for owner in self.owners:
            self.assertIsNone(owner.handle)
            self.assertIsNone(owner.job.handle)
            self.assertTrue(owner.stdout.closed)
            self.assertTrue(owner.stderr.closed)
            get_info = win.declare("GetHandleInformation", [win.w.HANDLE, ctypes.POINTER(win.w.DWORD)], win.w.BOOL)
            for handle in owner.original_handles:
                self.assertFalse(get_info(handle, ctypes.byref(win.w.DWORD())))
                self.assertEqual(ctypes.get_last_error(), 6)  # ERROR_INVALID_HANDLE
        self.assertFalse(any(t.name == "owned-diagnostic-capture-reader" for t in threading.enumerate()))

    def tree(self, *, closed=False, exit_code=None, output="", flood=False):
        # Each descendant starts before the leader can exit, and owns a child
        # of its own. Tests retain handles while these original PIDs are alive.
        child = ("import subprocess,sys,time,os,json,pathlib\n"
                 "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'])\n"
                 "pathlib.Path(" + repr(str(self.ready)) + ").write_text(json.dumps([os.getpid(),p.pid]))\n"
                 "time.sleep(30)\n")
        return ("import subprocess,sys,os,time,pathlib\n"
                "p=subprocess.Popen([sys.executable,'-c'," + repr(child) + "]"
                + (",stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL" if closed else "")
                + ")\n"
                "until=time.monotonic()+5\n"
                "while not pathlib.Path(" + repr(str(self.ready)) + ").exists():\n"
                " if time.monotonic() > until: raise RuntimeError('descendant-not-ready')\n"
                " time.sleep(0.01)\n"
                + output + "\n"
                + ("while True: os.write(1,b'x'*4096)\n" if flood else
                   "sys.exit(" + str(exit_code) + ")\n" if exit_code is not None else "time.sleep(30)\n"))

    def test_binary_streams_original_exit_argv_environment_and_no_controller_metadata(self):
        env = dict(os.environ, CAPTURE_UNICODE="原值+with space")
        args = ["/", ".", "不存在 路径", 'literal"quote', "trailing\\"]
        script = ("import os,sys,json; "
                  "os.write(1,b'out\\x00\\xff\\r\\n');os.write(2,b'err\\x00\\xfe\\n');"
                  "os.write(1,json.dumps([sys.argv[1:],os.environ['CAPTURE_UNICODE'],os.getcwd()]).encode());"
                  "sys.exit(37)")
        result = diagnostics.capture([sys.executable, "-c", script, *args], env=env)
        self.assertTrue(result.stdout.startswith(b"out\x00\xff\r\n"))
        self.assertEqual(result.stderr, b"err\x00\xfe\n")
        self.assertEqual(json.loads(result.stdout[7:]), [args, env["CAPTURE_UNICODE"], str(diagnostics.ROOT)])
        self.assertEqual(result.returncode, 37)
        self.assertFalse(result.timed_out)
        self.assert_closed(result)

    def test_child_is_in_job_at_first_instruction_and_still_active_exit_code_is_preserved(self):
        original_launch = win.launch

        def named(job, *args, **kwargs):
            kwargs["env"] = dict(os.environ, CAPTURE_JOB_NAME=job.name)
            return original_launch(job, *args, **kwargs)

        script = ("import ctypes,sys,os; from ctypes import wintypes as w;"
                  "k=ctypes.WinDLL('kernel32');k.GetCurrentProcess.restype=w.HANDLE;"
                  "k.OpenJobObjectW.argtypes=[w.DWORD,w.BOOL,w.LPCWSTR];k.OpenJobObjectW.restype=w.HANDLE;"
                  "job=k.OpenJobObjectW(4,False,os.environ['CAPTURE_JOB_NAME']);assert job;"
                  "k.IsProcessInJob.argtypes=[w.HANDLE,w.HANDLE,ctypes.POINTER(w.BOOL)];"
                  "inside=w.BOOL();assert k.IsProcessInJob(k.GetCurrentProcess(),job,ctypes.byref(inside));"
                  "assert inside.value;print('admitted',flush=True);sys.exit(259)")
        with patch.object(win, "launch", side_effect=named):
            result = diagnostics.capture([sys.executable, "-c", script])
        self.assertEqual(result.returncode, 259)
        self.assertEqual(result.stdout, b"admitted\r\n")
        self.assert_closed(result)

    def test_closed_pipe_descendants_reaped_after_success_and_failure_without_killing_sentinel(self):
        sentinel = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(30)"],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                    creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            for code in (0, 23):
                self.ready.unlink(missing_ok=True)
                result = diagnostics.capture([sys.executable, "-c", self.tree(closed=True, exit_code=code,
                    output="os.write(1,b'nonempty-out');os.write(2,b'nonempty-err')")], timeout=6)
                self.assertEqual(result.returncode, code)
                self.assertEqual(result.stdout, b"nonempty-out")
                self.assertEqual(result.stderr, b"nonempty-err")
                self.assertFalse(result.timed_out)
                self.assert_closed(result)
                self.assertIsNone(sentinel.poll())
        finally:
            sentinel.terminate()
            sentinel.wait(timeout=3)

    def test_deadline_reaps_inherited_pipe_descendants_and_original_handles(self):
        result = diagnostics.capture([sys.executable, "-c", self.tree()], timeout=1)
        self.assertTrue(self.ready.exists())
        self.assertTrue(result.timed_out)
        self.assertEqual(result.returncode, 1)
        self.assertLess(result.duration_ms, 4500)
        self.assert_closed(result)

    def test_overflow_reaps_whole_tree_without_truncated_success(self):
        result = diagnostics.capture([sys.executable, "-c", self.tree(flood=True)], max_bytes=1024, timeout=6)
        self.assertTrue(result.overflow)
        self.assertLessEqual(len(result.stdout) + len(result.stderr), 1024)
        self.assertFalse(diagnostics.assess(result, {b"absent-secret"}, result.returncode)["passed"])
        self.assert_closed(result)

    def test_reader_failure_and_close_failure_reap_tree_and_join_original_readers(self):
        for fail_close in (False, True):
            self.ready.unlink(missing_ok=True)
            original = win.CapturedProcess

            def broken(*args, **kwargs):
                owner = original(*args, **kwargs)
                pipe = owner.stdout

                class BrokenReader:
                    @property
                    def closed(self):
                        return pipe.closed

                    def read1(inner, size):
                        until = time.monotonic() + 5
                        while not self.ready.exists() and time.monotonic() < until:
                            time.sleep(0.01)
                        if fail_close:
                            return b""
                        raise OSError("synthetic-read-fault")

                    def close(inner):
                        pipe.close()
                        if fail_close:
                            raise OSError("synthetic-close-fault")

                owner.stdout = BrokenReader()
                return owner

            with patch.object(win, "CapturedProcess", side_effect=broken):
                result = diagnostics.capture([sys.executable, "-c", self.tree()], timeout=6)
            self.assertTrue(result.incomplete)
            self.assertTrue(result.cleanup_complete)
            self.assertFalse(result.timed_out)
            self.assertLess(result.duration_ms, 4500)
            self.assert_closed(result, incomplete=True)

    def test_invalid_executable_and_failed_reader_start_close_original_resources(self):
        jobs = []
        original = win.Job

        def observed_job(*args, **kwargs):
            job = original(*args, **kwargs)
            jobs.append(job)
            return job

        with patch.object(win, "Job", side_effect=observed_job):
            with self.assertRaises(OSError):
                diagnostics.capture([str(Path(self.directory.name) / "missing.exe")])
        self.assertTrue(all(job.handle is None for job in jobs))
        with patch.object(threading.Thread, "start", side_effect=RuntimeError("synthetic-start-fault")):
            with self.assertRaisesRegex(RuntimeError, "synthetic-start-fault"):
                diagnostics.capture([sys.executable, "-c", "import time;time.sleep(30)"])
        self.assertTrue(all(owner.handle is None and owner.job.handle is None for owner in self.owners))
        self.assertTrue(all(owner.stdout.closed and owner.stderr.closed for owner in self.owners))

    def test_invalid_budgets_and_failed_job_admission_never_execute_child(self):
        for timeout in (0, -1, float("inf"), float("nan")):
            with self.assertRaises(ValueError):
                diagnostics.capture(["never-launch"], timeout=timeout)
        for budget in (0, -1, 0.5):
            with self.assertRaises(ValueError):
                diagnostics.capture(["never-launch"], max_bytes=budget)
        with patch.object(win, "update_attribute", return_value=0), patch.object(win, "create_process") as create:
            with self.assertRaises(OSError):
                diagnostics.capture([sys.executable, "-c", "raise RuntimeError('must-not-run')"])
            create.assert_not_called()

    def test_session_agent_stop_preserves_raw_secret_scan_and_reclaims_retained_descendant_handles(self):
        session = regression.Session(Path(self.directory.name))
        self.addCleanup(session.cleanup)
        try:
            secret = b"synthetic-capture-agent-private-token"
            session.variants.update(diagnostics.secret_variants(secret))
            script = self.tree(output="os.write(1,b'safe');os.write(2," + repr(secret) + ")")
            owned = session.agent([sys.executable, "-c", script], timeout=8)
            until = time.monotonic() + 5
            while not self.ready.exists() and not owned[3].done() and time.monotonic() < until:
                time.sleep(0.01)
            self.assertTrue(self.ready.exists())
            # Open once, before stop, then retain original handles across cleanup.
            open_process = win.declare("OpenProcess", [win.w.DWORD, win.w.BOOL, win.w.DWORD], win.w.HANDLE)
            handles = [win.checked(open_process(0x100000, False, pid)) for pid in json.loads(self.ready.read_text())]
            try:
                # Wait for both raw writes to enter the capture before requesting stop.
                time.sleep(0.1)
                result = session.stop_agent(owned)
                self.assertTrue(result.stop_requested)
                self.assertFalse(result.timed_out)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, b"safe")
                self.assertEqual(result.stderr, secret)
                report = session.record("host-agent", owned[0], result, 1, True)
                self.assertGreater(report["raw_hits"], 0)
                self.assertEqual(report["masked_copy_hits"], 0)
                self.assertEqual(report["status"], "FAIL")
                self.assertNotIn(secret.decode(), json.dumps(report))
                self.assert_closed(result)
                self.assertTrue(all(win.wait_handle(handle, 0) == 0 for handle in handles))
            finally:
                for handle in handles:
                    win.close_handle(handle)

        finally:
            session.cleanup()

    def test_agent_stop_after_leader_exit_reclaims_pipe_writers_without_waiting_for_deadline(self):
        session = regression.Session(Path(self.directory.name))
        self.addCleanup(session.cleanup)
        owned = session.agent([sys.executable, "-c", self.tree(exit_code=23)], timeout=8)
        until = time.monotonic() + 5
        while (not self.owners or self.owners[-1].poll() is None) and time.monotonic() < until:
            time.sleep(0.01)
        self.assertTrue(self.ready.exists())
        self.assertEqual(self.owners[-1].poll(), 23)
        result = session.stop_agent(owned)
        self.assertTrue(result.stop_requested)
        self.assertFalse(result.timed_out)
        self.assertEqual(result.returncode, 23)
        self.assertLess(result.duration_ms, 4500)
        self.assert_closed(result)


if __name__ == "__main__":
    unittest.main()
