"""Owned CLI watch, bounded in-memory output and original graceful stop owners.

POSIX uses its original process group. Windows uses a hidden private console
and Job Object. A killed process cannot supply a native cleanup receipt.
"""
import json
import math
import os
import signal
import subprocess
import threading
import time

from diagnostics import Capture, ROOT, group_alive


class OwnedWatch:
    def __init__(self, argv, environment, *, timeout=30, max_bytes=65536):
        if os.name not in ("posix", "nt"):
            raise ValueError("owned-graceful-watch-platform-required")
        if not math.isfinite(timeout) or not 0 < timeout <= 86400 or type(max_bytes) is not int or not 0 < max_bytes <= 16 * 1024 * 1024:
            raise ValueError("finite-watch-budgets-required")
        self.started = time.monotonic()
        self.deadline = self.started + timeout
        self.maximum = max_bytes
        self.windows = None
        if os.name == "nt":
            from regression_watch_windows import WindowsOwner
            self.windows = WindowsOwner(argv, environment, timeout, ROOT)
            self.process = self.windows.process
        else:
            self.process = subprocess.Popen(argv, cwd=ROOT, env=environment, stdout=subprocess.PIPE,
                                            stderr=subprocess.PIPE, start_new_session=True)
        self.finished = None
        self.condition = threading.Condition()
        self.buffers = [bytearray(), bytearray()]
        self.records = [[], []]
        self.pending = [bytearray(), bytearray()]
        self.failed = self.overflow = False
        self.closed = [False, False]
        self.workers = [threading.Thread(target=self.read, args=(pipe, index), name="owned-regression-watch-reader")
                        for index, pipe in enumerate((self.process.stdout, self.process.stderr))]
        for worker in self.workers:
            worker.start()

    def read(self, pipe, index):
        try:
            while True:
                chunk = pipe.read1(4096)
                if not chunk:
                    break
                with self.condition:
                    available = self.maximum - sum(map(len, self.buffers))
                    if len(chunk) > available:
                        self.overflow = True
                        self.condition.notify_all()
                        break
                    self.buffers[index].extend(chunk)
                    self.pending[index].extend(chunk)
                    while b"\n" in self.pending[index]:
                        line, _, remaining = self.pending[index].partition(b"\n")
                        self.pending[index] = bytearray(remaining)
                        if line:
                            self.records[index].append(json.loads(line))
                    self.condition.notify_all()
        except (OSError, ValueError, UnicodeError):
            with self.condition:
                self.failed = True
                self.condition.notify_all()
        finally:
            pipe.close()
            with self.condition:
                if self.pending[index]:
                    self.failed = True  # Incomplete NDJSON cannot supply a receipt.
                self.closed[index] = True
                self.condition.notify_all()

    def wait(self, predicate, seconds=10):
        until = min(self.deadline, time.monotonic() + seconds)
        with self.condition:
            while True:
                if self.failed or self.overflow:
                    raise ValueError("watch-output-failed")
                if predicate(self.records):
                    return
                if any(self.closed) or self.process.poll() is not None:
                    raise ValueError("watch-ended-before-observation")
                remaining = until - time.monotonic()
                if remaining <= 0:
                    raise ValueError("watch-observation-deadline")
                self.condition.wait(remaining)

    def quiet(self, seconds=0.3):
        """Finite negative observation; does not invent an upstream delivery ACK."""
        until = min(self.deadline, time.monotonic() + seconds)
        with self.condition:
            initial = len(self.records[0])
            while time.monotonic() < until:
                if self.failed or self.overflow or any(self.closed) or len(self.records[0]) != initial:
                    raise ValueError("duplicate-watch-event-or-output-failure")
                self.condition.wait(until - time.monotonic())

    def finish(self):
        if self.finished is not None:
            return self.finished
        if self.windows is not None:
            return self.finish_windows()
        requested = self.process.poll() is None
        if requested:
            try:
                os.killpg(self.process.pid, signal.SIGINT)
            except ProcessLookupError:
                pass
        graceful = True
        try:
            self.process.wait(timeout=max(0.1, min(10, self.deadline - time.monotonic())))
        except subprocess.TimeoutExpired:
            graceful = False
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.process.wait(timeout=0.5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.process.wait(timeout=2)
        # Never leave an owned descendant after a successful leader exits.
        if group_alive(self.process.pid) is not False:
            graceful = False
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        for worker in self.workers:
            worker.join(2)
        with self.condition:
            receipt = any(row.get("state") == "stopped" and row.get("native_reclaimed") is True
                          for row in self.records[1] if isinstance(row, dict))
            complete = (all(self.closed) and all(not worker.is_alive() for worker in self.workers)
                        and group_alive(self.process.pid) is False)
            result = Capture(bytes(self.buffers[0]), bytes(self.buffers[1]), self.process.returncode,
                             not graceful, self.overflow, self.failed or not all(self.closed),
                             round((time.monotonic() - self.started) * 1000), self.process.pid,
                             complete, "process-group", requested)
            self.finished = result, graceful and receipt, list(self.records[0])
            return self.finished

    def finish_windows(self):
        requested, graceful = self.windows.finish(max(0.1, min(12, self.deadline - time.monotonic())))
        for worker in self.workers:
            worker.join(2)
        with self.condition:
            controllers = [row for row in self.records[1] if isinstance(row, dict)
                           and row.get("operation") == "owned_watch_supervisor"]
            reclaimed = next((row for row in controllers if row.get("state") == "host_reclaimed"), None)
            native = any(row.get("state") == "stopped" and row.get("native_reclaimed") is True
                         for row in self.records[1] if isinstance(row, dict)
                         and row.get("operation") != "owned_watch_supervisor")
            graceful = (graceful and reclaimed is not None and reclaimed.get("forced") is False
                        and reclaimed.get("job_empty") is True and (not requested or reclaimed.get("interrupt_sent") is True))
            complete = self.windows.complete and all(self.closed) and all(not worker.is_alive() for worker in self.workers)
            failed = self.failed or any(worker.is_alive() for worker in self.workers)
            result = Capture(bytes(self.buffers[0]), bytes(self.buffers[1]), self.process.returncode,
                             not graceful, self.overflow, failed or not all(self.closed),
                             round((time.monotonic() - self.started) * 1000),
                             reclaimed.get("child_pid", self.process.pid) if reclaimed else self.process.pid,
                             complete, "windows-job", requested)
            self.finished = result, (graceful and native and complete and not self.failed
                                     and not self.overflow and self.process.returncode == 0), list(self.records[0])
            return self.finished
