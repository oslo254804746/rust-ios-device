"""Owned POSIX CLI watch with bounded in-memory observations and graceful SIGINT.

Windows console-signal supervision is deliberately unavailable. A killed
process can prove host reclamation, but cannot supply a native cleanup receipt.
"""
import json
import os
import signal
import subprocess
import threading
import time

from diagnostics import Capture, ROOT, group_alive


class OwnedWatch:
    def __init__(self, argv, environment, *, timeout=30, max_bytes=65536):
        if os.name != "posix":
            raise ValueError("posix-graceful-watch-required")
        self.started = time.monotonic()
        self.deadline = self.started + timeout
        self.maximum = max_bytes
        self.process = subprocess.Popen(argv, cwd=ROOT, env=environment, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, start_new_session=True)
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
            return result, graceful and receipt, list(self.records[0])
