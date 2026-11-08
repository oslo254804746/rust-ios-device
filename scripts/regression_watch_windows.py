"""Private hidden console and original Job Object owners for Windows watches.

The child joins the job atomically at CreateProcess, before it runs. Ctrl-C is
sent only inside this controller's new console after checking its membership.
Job termination is failure cleanup and can never supply a native receipt.
"""
import argparse
import ctypes
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from regression_windows_process import (
    Job, checked, close_handle, console_ignore, console_processes, console_signal,
    exit_code, launch, peek_pipe, standard_handle, wait_handle,
)


def private_console(job, parent_pid):
    pids = (w.DWORD * 4096)()
    count = checked(console_processes(pids, len(pids)))
    if count > len(pids):
        raise ValueError("private-watch-console-process-limit")
    current = set(pids[:count])
    if parent_pid in current or os.getpid() not in current or not current <= job.pids() | {os.getpid()}:
        raise ValueError("watch-console-is-not-private")


def emit(state, **facts):
    print(json.dumps(dict(operation="owned_watch_supervisor", state=state, **facts)), file=sys.stderr, flush=True)


def supervise(job_name, argv, timeout, parent_pid):
    job, process = Job(job_name), None
    forced, sent = False, False
    try:
        private_console(job, parent_pid)
        checked(console_ignore(None, False))  # Child must inherit enabled Ctrl-C.
        process, pid = launch(job, argv)
        checked(console_ignore(None, True))  # Ignore only in this already-running controller.
        private_console(job, parent_pid)
        emit("owner_ready", child_pid=pid, private_console=True)
        until = time.monotonic() + timeout
        control = bytearray()
        while True:
            waited = wait_handle(process, 20)
            if waited == 0:
                break
            if waited != 258:
                checked(False)  # Preserve WaitForSingleObject's native Win32 cause.
            available = w.DWORD()
            ok = peek_pipe(standard_handle(-10 & 0xFFFFFFFF), None, 0, None, ctypes.byref(available), None)
            if not ok:
                if ctypes.get_last_error() != 109:
                    checked(ok)
                control = bytearray(b"interrupt\n")  # Parent abandonment also requests cleanup.
            elif available.value:
                if available.value > 64 - len(control):
                    raise ValueError("watch-control-limit")
                control.extend(os.read(sys.stdin.fileno(), available.value))
                if len(control) > 64:
                    raise ValueError("watch-control-limit")
            if not sent and b"\n" in control:
                if bytes(control) != b"interrupt\n":
                    raise ValueError("invalid-watch-control")
                private_console(job, parent_pid)
                checked(console_signal(0, 0))  # This NEW console contains only the controller and job.
                sent = True
                until = min(until, time.monotonic() + 10)
                emit("interrupt_sent", child_pid=pid)
            if time.monotonic() >= until:
                forced = True
                job.terminate()
                if wait_handle(process, 2000) != 0:
                    raise TimeoutError("watch-original-process-reap")
                break
        code = w.DWORD()
        checked(exit_code(process, ctypes.byref(code)))
        if job.active():
            forced = True  # Leader success cannot hide a surviving original descendant.
            job.terminate()
        complete = job.empty(2)
        emit("host_reclaimed", child_pid=pid, child_exit_code=code.value, forced=forced,
             interrupt_sent=sent, job_empty=complete)
        return 124 if forced or not complete else int(code.value)
    except (OSError, ValueError, TimeoutError) as error:
        emit("controller_failed", error_type=type(error).__name__, error_code=getattr(error, "winerror", None))
        job.terminate()
        if process is not None:
            wait_handle(process, 2000)
        job.empty(2)
        return 125
    finally:
        if process is not None:
            checked(close_handle(process))
        job.close()


class WindowsOwner:
    def __init__(self, argv, environment, timeout, cwd):
        self.job = Job()
        self.complete = False
        self.forced = False
        self.process = None
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
        try:
            self.process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                                             "--job", self.job.name, "--timeout", str(timeout),
                                             "--parent-pid", str(os.getpid()), "--", *argv],
                                            cwd=cwd, env=environment, stdin=subprocess.PIPE,
                                            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                            creationflags=subprocess.CREATE_NEW_CONSOLE, startupinfo=startup)
        except BaseException:
            self.job.close()
            raise

    def finish(self, seconds):
        requested = self.process.poll() is None
        if requested:
            try:
                self.process.stdin.write(b"interrupt\n")
                self.process.stdin.flush()
            except (OSError, ValueError):
                self.forced = True
        self.process.stdin.close()
        try:
            self.process.wait(timeout=seconds)
        except subprocess.TimeoutExpired:
            self.forced = True
            self.job.terminate()
            self.process.terminate()  # This original controller only, never a discovered process.
            self.process.wait(timeout=2)
        if self.job.active():
            self.forced = True
            self.job.terminate()
        self.complete = self.job.empty(2)
        self.job.close()
        return requested, not self.forced


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--job", required=True)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("--parent-pid", type=int, required=True)
    parser.add_argument("argv", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    argv = args.argv[1:] if args.argv[:1] == ["--"] else args.argv
    if not argv or not 0 < args.timeout <= 86400 or args.parent_pid <= 0:
        parser.error("invalid finite watch supervision")
    sys.exit(supervise(args.job, argv, args.timeout, args.parent_pid))
