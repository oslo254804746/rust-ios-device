#!/usr/bin/env python3
"""Read-only SDK apps/AFC/log example using explicit existing-trust inputs."""
import argparse
import asyncio
from pathlib import Path
import sys

import ios_rs


async def run(args, pair):
    async with ios_rs.DeviceSession(args.udid, pair, args.endpoint, concurrency=1) as dev:
        apps = await dev.service_async("apps", timeout=args.timeout)
        print("User apps:", len(apps))
        names = await dev.service_async("files", path=args.directory, timeout=args.timeout)
        print("Directory entries:", len(names))
        if args.read_file:
            content = await dev.service_async("read_file", path=args.read_file,
                max_bytes=args.max_bytes, output_bytes=max(1, args.max_bytes), timeout=args.timeout)
            print("Complete file bytes:", len(content))
        if args.logs:
            try:
                logs = await dev.service_async("logs", max_messages=args.logs, timeout=args.timeout)
            except (RuntimeError, asyncio.CancelledError) as error:
                print("Log termination:", getattr(error, "ios_error_class", "unknown"),
                      "completed messages:", len(getattr(error, "ios_partial_result", [])), file=sys.stderr)
                raise
            print("Completed log messages:", len(logs))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("udid")
    parser.add_argument("endpoint", help="numeric tcp:// endpoint or absolute unix:// socket")
    parser.add_argument("pair_record", type=Path)
    parser.add_argument("--directory", default="/")
    parser.add_argument("--read-file")
    parser.add_argument("--max-bytes", type=int, default=65536)
    parser.add_argument("--logs", type=int, default=0)
    parser.add_argument("--timeout", type=float, default=5.0)
    args = parser.parse_args()
    # Caller-owned file IO is separate from the native service total. Read at
    # most one extra byte to refuse an oversized local credential file.
    with args.pair_record.open("rb") as source:
        pair = source.read(1024 * 1024 + 1)
    if not pair or len(pair) > 1024 * 1024:
        raise ValueError("pairing record must contain 1..1MiB bytes")
    asyncio.run(run(args, pair))


if __name__ == "__main__":
    main()
