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
import re
import subprocess
import sys
import time
import uuid

if os.name != "nt":
    raise ImportError("Windows console supervision requires Windows")

k = ctypes.WinDLL("kernel32", use_last_error=True)
SIZE = ctypes.c_size_t


class BasicLimits(ctypes.Structure):
    _fields_ = [("ProcessTime", ctypes.c_longlong), ("JobTime", ctypes.c_longlong),
                ("Flags", w.DWORD), ("MinimumWorkingSet", SIZE), ("MaximumWorkingSet", SIZE),
                ("ActiveProcessLimit", w.DWORD), ("Affinity", SIZE),
                ("Priority", w.DWORD), ("Scheduling", w.DWORD)]


class ExtendedLimits(ctypes.Structure):
    _fields_ = [("Basic", BasicLimits), ("IoCounters", ctypes.c_ulonglong * 6),
                ("ProcessMemoryLimit", SIZE), ("JobMemoryLimit", SIZE),
                ("PeakProcessMemory", SIZE), ("PeakJobMemory", SIZE)]


class Accounting(ctypes.Structure):
    _fields_ = [("Times", ctypes.c_longlong * 4), ("PageFaults", w.DWORD),
                ("Processes", w.DWORD), ("ActiveProcesses", w.DWORD), ("Terminated", w.DWORD)]


class StartupInfo(ctypes.Structure):
    _fields_ = [("cb", w.DWORD), ("reserved", w.LPWSTR), ("desktop", w.LPWSTR),
                ("title", w.LPWSTR), ("x", w.DWORD), ("y", w.DWORD),
                ("xsize", w.DWORD), ("ysize", w.DWORD), ("xchars", w.DWORD),
                ("ychars", w.DWORD), ("fill", w.DWORD), ("flags", w.DWORD),
                ("show", w.WORD), ("reserved_size", w.WORD), ("reserved_bytes", ctypes.c_void_p),
                ("stdin", w.HANDLE), ("stdout", w.HANDLE), ("stderr", w.HANDLE)]


class StartupInfoEx(ctypes.Structure):
    _fields_ = [("Startup", StartupInfo), ("Attributes", ctypes.c_void_p)]


class ProcessInfo(ctypes.Structure):
    _fields_ = [("process", w.HANDLE), ("thread", w.HANDLE), ("pid", w.DWORD), ("tid", w.DWORD)]


def declare(name, arguments, result):
    function = getattr(k, name)
    function.argtypes, function.restype = arguments, result
    return function


close_handle = declare("CloseHandle", [w.HANDLE], w.BOOL)
create_job = declare("CreateJobObjectW", [ctypes.c_void_p, w.LPCWSTR], w.HANDLE)
open_job = declare("OpenJobObjectW", [w.DWORD, w.BOOL, w.LPCWSTR], w.HANDLE)
set_job = declare("SetInformationJobObject", [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD], w.BOOL)
query_job = declare("QueryInformationJobObject", [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD, ctypes.c_void_p], w.BOOL)
terminate_job = declare("TerminateJobObject", [w.HANDLE, w.UINT], w.BOOL)
wait_handle = declare("WaitForSingleObject", [w.HANDLE, w.DWORD], w.DWORD)
exit_code = declare("GetExitCodeProcess", [w.HANDLE, ctypes.POINTER(w.DWORD)], w.BOOL)
init_attributes = declare("InitializeProcThreadAttributeList", [ctypes.c_void_p, w.DWORD, w.DWORD, ctypes.POINTER(SIZE)], w.BOOL)
update_attribute = declare("UpdateProcThreadAttribute", [ctypes.c_void_p, w.DWORD, SIZE, ctypes.c_void_p, SIZE, ctypes.c_void_p, ctypes.c_void_p], w.BOOL)
delete_attributes = declare("DeleteProcThreadAttributeList", [ctypes.c_void_p], None)
create_process = declare("CreateProcessW", [w.LPCWSTR, w.LPWSTR, ctypes.c_void_p, ctypes.c_void_p,
                          w.BOOL, w.DWORD, ctypes.c_void_p, w.LPCWSTR, ctypes.c_void_p,
                          ctypes.POINTER(ProcessInfo)], w.BOOL)
standard_handle = declare("GetStdHandle", [w.DWORD], w.HANDLE)
inherit_handle = declare("SetHandleInformation", [w.HANDLE, w.DWORD, w.DWORD], w.BOOL)
console_ignore = declare("SetConsoleCtrlHandler", [ctypes.c_void_p, w.BOOL], w.BOOL)
console_signal = declare("GenerateConsoleCtrlEvent", [w.DWORD, w.DWORD], w.BOOL)
console_processes = declare("GetConsoleProcessList", [ctypes.POINTER(w.DWORD), w.DWORD], w.DWORD)
peek_pipe = declare("PeekNamedPipe", [w.HANDLE, ctypes.c_void_p, w.DWORD, ctypes.c_void_p,
                                    ctypes.POINTER(w.DWORD), ctypes.c_void_p], w.BOOL)


def checked(value):
    if not value:
        raise ctypes.WinError(ctypes.get_last_error())
    return value


class Job:
    def __init__(self, name=None):
        self.name = name or "ios-watch-" + uuid.uuid4().hex
        if not re.fullmatch(r"ios-watch-[0-9a-f]{32}", self.name):
            raise ValueError("invalid-private-watch-job")
        self.handle = None
        if name is not None:
            self.handle = checked(open_job(0x001F003F, False, name))
        else:
            ctypes.set_last_error(0)
            self.handle = checked(create_job(None, self.name))
            try:
                if ctypes.get_last_error() == 183:
                    raise ValueError("private-watch-job-already-exists")
                limits = ExtendedLimits()
                limits.Basic.Flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE; no breakaway.
                checked(set_job(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)))
            except BaseException:
                self.close()
                raise

    def active(self):
        info = Accounting()
        checked(query_job(self.handle, 1, ctypes.byref(info), ctypes.sizeof(info), None))
        return info.ActiveProcesses

    def pids(self):
        class ProcessList(ctypes.Structure):
            _fields_ = [("assigned", w.DWORD), ("count", w.DWORD), ("pids", SIZE * 4096)]
        info = ProcessList()
        checked(query_job(self.handle, 3, ctypes.byref(info), ctypes.sizeof(info), None))
        if info.count > 4096 or info.assigned > 4096:
            raise ValueError("private-watch-process-limit")
        return set(info.pids[:info.count])

    def terminate(self):
        checked(terminate_job(self.handle, 124))

    def empty(self, seconds):
        until = time.monotonic() + seconds
        while self.active():
            if time.monotonic() >= until:
                return False
            time.sleep(0.02)
        return True

    def close(self):
        if self.handle is not None:
            handle, self.handle = self.handle, None
            checked(close_handle(handle))


def private_console(job, parent_pid):
    pids = (w.DWORD * 4096)()
    count = checked(console_processes(pids, len(pids)))
    if count > len(pids):
        raise ValueError("private-watch-console-process-limit")
    current = set(pids[:count])
    if parent_pid in current or os.getpid() not in current or not current <= job.pids() | {os.getpid()}:
        raise ValueError("watch-console-is-not-private")


def launch(job, argv):
    import msvcrt
    # Explicit handles and atomic JOB_LIST admission prevent inherited command
    # pipes and execution-before-job races. Windows 10+ supports JOB_LIST.
    info = StartupInfoEx()
    info.Startup.cb = ctypes.sizeof(info)
    info.Startup.flags = 0x101  # STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW
    info.Startup.show = 0  # SW_HIDE
    info.Startup.stdout, info.Startup.stderr = standard_handle(-11 & 0xFFFFFFFF), standard_handle(-12 & 0xFFFFFFFF)
    jobs = (w.HANDLE * 1)(job.handle)
    size = SIZE()
    init_attributes(None, 2, 0, ctypes.byref(size))
    if not 0 < size.value <= 65536:
        raise ValueError("watch-startup-attribute-limit")
    attributes = ctypes.create_string_buffer(size.value)
    checked(init_attributes(attributes, 2, 0, ctypes.byref(size)))
    process = ProcessInfo()
    try:
        with open(os.devnull, "rb") as null:
            info.Startup.stdin = msvcrt.get_osfhandle(null.fileno())
            handles = (w.HANDLE * 3)(info.Startup.stdin, info.Startup.stdout, info.Startup.stderr)
            for handle in handles:
                checked(inherit_handle(handle, 1, 1))
            checked(update_attribute(attributes, 0, 0x2000D, jobs, ctypes.sizeof(jobs), None, None))
            checked(update_attribute(attributes, 0, 0x20002, handles, ctypes.sizeof(handles), None, None))
            info.Attributes = ctypes.cast(attributes, ctypes.c_void_p)
            command = ctypes.create_unicode_buffer(subprocess.list2cmdline(argv))
            checked(create_process(argv[0], command, None, None, True, 0x80000,
                                   None, None, ctypes.byref(info), ctypes.byref(process)))
    finally:
        delete_attributes(attributes)
    checked(close_handle(process.thread))
    return process.process, process.pid


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
