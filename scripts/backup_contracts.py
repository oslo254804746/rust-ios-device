#!/usr/bin/env python3
"""Actual backup CLI output contracts against an owned, synthetic local index."""

import json
import os
from pathlib import Path
import plistlib
import socket
import sqlite3
import stat
import subprocess
import sys
import tempfile

from diagnostics import ROOT, assess, capture, secret_variants
from regression import SafeArgumentParser

SOURCE = "synthetic-private-backup-source"


def identity(path):
    metadata = path.lstat()
    return metadata.st_dev, metadata.st_ino


def cleanup(root, root_identity, directory_identity, known):
    """Delete only the unchanged owned directories and their known regular files."""
    directory = root / SOURCE
    try:
        for path, expected in ((root, root_identity), (directory, directory_identity)):
            metadata = path.lstat()
            if not stat.S_ISDIR(metadata.st_mode) or identity(path) != expected:
                return False
        if {path.name for path in root.iterdir()} != {SOURCE}:
            return False
        actual = {path.name: path for path in directory.iterdir()}
        if set(actual) != set(known):
            return False
        for name, path in actual.items():
            metadata = path.lstat()
            if not stat.S_ISREG(metadata.st_mode) or identity(path) != known[name]:
                return False
        for path in actual.values():
            path.unlink()
        directory.rmdir()
        root.rmdir()
        return True
    except OSError:
        return False


def closed_output(command):
    """The child inherits this capture's owned group and bounded stderr pipe."""
    reader, writer = os.pipe()
    os.close(reader)
    try:
        result = subprocess.run(command, stdout=writer, timeout=5)
        print(json.dumps({"child_exit": result.returncode}))
        return result.returncode == 1
    except Exception:
        print(json.dumps({"child_exit": None}))
        return False
    finally:
        os.close(writer)


def main():
    parser = SafeArgumentParser(prog="backup-contracts")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/ios")
    parser.add_argument("--closed-output", action="store_true", help="internal owned fixture probe")
    parser.add_argument("--fixture-root", type=Path, help="internal owned fixture root")
    args = parser.parse_args()
    if args.closed_output:
        if args.fixture_root is None:
            print(json.dumps({"child_exit": None}))
            return 1
        command = [str(args.binary.resolve()), "backup", "list-local",
                   str(args.fixture_root), "--source", SOURCE]
        return 0 if closed_output(command) else 1
    if not sys.platform.startswith("linux"):
        print(json.dumps({"passed": False, "status": "NOT_RUN", "condition": "Linux owned process group"}))
        return 1

    root = None
    root_identity = directory_identity = None
    known = {}
    reports = {}
    trap = None
    device_connections = None
    passed = False
    clean = False
    try:
        root = Path(tempfile.mkdtemp(prefix="ios-backup-contract-"))
        root_identity = identity(root)
        source = SOURCE
        directory = root / source
        directory.mkdir(mode=0o700)
        directory_identity = identity(directory)
        for name in ("Info.plist", "Status.plist", "Manifest.plist"):
            path = directory / name
            with path.open("xb") as output:
                metadata = {"IsEncrypted": False}
                if name == "Manifest.plist":
                    metadata["Lockdown"] = {"ProductVersion": "17.0"}
                output.write(plistlib.dumps(metadata))
            path.chmod(0o600)
            known[name] = identity(path)
        metadata = plistlib.dumps({
            "$archiver": "NSKeyedArchiver", "$version": 100000,
            "$top": {"root": plistlib.UID(1)},
            "$objects": ["$null", {"$class": plistlib.UID(2),
                                   "Mode": plistlib.UID(3), "Size": plistlib.UID(4)},
                         {"$classname": "MBFile", "$classes": ["MBFile", "NSObject"]},
                         0o100644, 42],
        }, fmt=plistlib.FMT_BINARY)
        with sqlite3.connect(directory / "Manifest.db") as connection:
            connection.execute("CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT, file BLOB)")
            connection.execute("INSERT INTO Files VALUES (?,?,?,?)",
                               ("1" * 40, "HomeDomain", "Library/Test.txt", metadata))
        (directory / "Manifest.db").chmod(0o600)
        known["Manifest.db"] = identity(directory / "Manifest.db")
        secret = "synthetic-private-backup-environment"
        env = os.environ.copy()
        env["IOS_BACKUP_CONTRACT_SECRET"] = secret
        # Own the only possible usbmux endpoint and observe any unintended dial.
        trap = socket.socket()
        trap.bind(("127.0.0.1", 0))
        trap.listen()
        env["USBMUXD_SOCKET_ADDRESS"] = "tcp://127.0.0.1:" + str(trap.getsockname()[1])
        variants = secret_variants(secret.encode()) | secret_variants(source.encode())
        variants |= secret_variants(str(root).encode())
        binary = str(args.binary.resolve())
        command = [binary, "backup", "list-local", str(root), "--source", source]
        expected = [{"file_id": "1" * 40, "domain": "HomeDomain", "relative_path": "Library/Test.txt",
                     "mode": 0o100644, "size": 42, "link_target": None}]
        human = capture([binary, "--no-json", *command[1:]], env=env)
        reports["human"] = assess(human, variants, 0)
        reports["human"]["contract"] = human.stdout == b"HomeDomain\tLibrary/Test.txt\t42 bytes\tmode=0x81a4\n" and not human.stderr
        machine = capture(command, env=env)
        reports["json"] = assess(machine, variants, 0)
        reports["json"]["contract"] = machine.stdout == (json.dumps(expected, indent=2) + "\n").encode() and not machine.stderr
        probe = capture([sys.executable, str(Path(__file__).resolve()), "--closed-output",
                         "--binary", binary, "--fixture-root", str(root)], env=env)
        reports["closed-output"] = assess(probe, variants, 0)
        reports["closed-output"]["contract"] = probe.stdout == b'{"child_exit": 1}\n' and b"io/BrokenPipe" in probe.stderr and b"panicked" not in probe.stderr
        trap.setblocking(False)
        device_connections = 0
        while True:
            try:
                connection, _ = trap.accept()
                connection.close()
                device_connections += 1
            except BlockingIOError:
                break
        passed = device_connections == 0 and all(item["passed"] and item["contract"] for item in reports.values())
    except Exception:
        passed = False
    finally:
        if trap is not None:
            trap.close()
        if root is not None and root_identity is not None and directory_identity is not None:
            clean = cleanup(root, root_identity, directory_identity, known)
    print(json.dumps({"passed": passed and clean, "cleanup_complete": clean,
                      "real_device_executed": False, "device_connections": device_connections,
                      "cases": reports}, sort_keys=True))
    return 0 if passed and clean else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        print('{"passed":false,"reason":"backup-contract-check-failed"}', file=sys.stderr)
        sys.exit(1)
