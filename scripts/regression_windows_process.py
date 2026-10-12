"""Original Windows Job/handle ownership shared by capture and watch.

JOB_LIST assigns a child before it can execute; no PID lookup or name-based kill.
"""
import ctypes
from ctypes import wintypes as w
from contextlib import ExitStack
import os
import re
import subprocess
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

    def terminate(self, code=124):
        checked(terminate_job(self.handle, code))

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


def launch(job, argv, *, output_handles=None, env=None, cwd=None, creationflags=0):
    import msvcrt
    # Explicit handles and atomic JOB_LIST admission prevent inherited command
    # pipes and execution-before-job races. Windows 10+ supports JOB_LIST.
    info = StartupInfoEx()
    info.Startup.cb = ctypes.sizeof(info)
    info.Startup.flags = 0x101  # STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW
    info.Startup.show = 0  # SW_HIDE
    info.Startup.stdout, info.Startup.stderr = output_handles or (
        standard_handle(-11 & 0xFFFFFFFF), standard_handle(-12 & 0xFFFFFFFF))
    environment = None
    if env is not None:
        entries = []
        for key, value in sorted(env.items(), key=lambda item: item[0].upper()):
            if not key or "=" in key[1:] or "\0" in key or "\0" in value:
                raise ValueError("invalid-capture-environment")
            entries.append(key + "=" + value)
        environment = ctypes.create_unicode_buffer("\0".join(entries) + "\0\0")
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
            # A null application name preserves CreateProcess/Popen PATH lookup.
            checked(create_process(None, command, None, None, True, 0x80400 | creationflags,
                                   environment, os.fspath(cwd) if cwd is not None else None,
                                   ctypes.byref(info), ctypes.byref(process)))
    finally:
        delete_attributes(attributes)
    checked(close_handle(process.thread))
    return process.process, process.pid


class CapturedProcess:
    """Direct child with separate byte pipes; no controller or metadata output.

    The original process and job handles remain owned until all readers join.
    terminate() is forced host cleanup, never a native/device acknowledgement.
    """
    def __init__(self, argv, *, env=None, cwd=None):
        import msvcrt
        self.args = argv
        self.returncode = None
        self.handle = None
        self.job = Job()
        self.stdout = self.stderr = None
        try:
            with ExitStack() as writers:
                outputs = []
                for name in ("stdout", "stderr"):
                    read_fd, write_fd = os.pipe()
                    writers.callback(os.close, write_fd)
                    try:
                        pipe = os.fdopen(read_fd, "rb")
                    except BaseException:
                        os.close(read_fd)
                        raise
                    setattr(self, name, pipe)
                    outputs.append(msvcrt.get_osfhandle(write_fd))
                self.handle, self.pid = launch(self.job, argv, output_handles=outputs,
                                               env=env, cwd=cwd,
                                               creationflags=subprocess.CREATE_NO_WINDOW)
        except BaseException:
            self.job.close()  # KILL_ON_JOB_CLOSE also covers a partially completed launch.
            if self.handle is not None:
                wait_handle(self.handle, 2000)
                close_handle(self.handle)
                self.handle = None
            for pipe in (self.stdout, self.stderr):
                if pipe is not None:
                    pipe.close()
            raise

    def poll(self):
        if self.returncode is None:
            waited = wait_handle(self.handle, 0)
            if waited == 0:
                code = w.DWORD()
                checked(exit_code(self.handle, ctypes.byref(code)))
                self.returncode = int(code.value)
            elif waited != 258:
                checked(False)
        return self.returncode

    def wait(self, timeout):
        if self.poll() is None:
            waited = wait_handle(self.handle, max(0, int(timeout * 1000)))
            if waited == 258:
                raise subprocess.TimeoutExpired(self.args, timeout)
            if waited != 0:
                checked(False)
        return self.poll()

    def terminate(self):
        # Match Popen.terminate's Windows exit code, but cover the original tree.
        self.job.terminate(1)

    kill = terminate

    def close(self):
        try:
            self.job.close()
        finally:
            if self.handle is not None:
                handle, self.handle = self.handle, None
                checked(close_handle(handle))
