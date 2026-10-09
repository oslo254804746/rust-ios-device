#!/usr/bin/env python3
"""Query one explicit already trusted device using caller-loaded credentials."""
import argparse
import asyncio
from pathlib import Path

import ios_rs


async def query(args, pair_record_bytes):
    pool = ios_rs.DeviceAdmission(args.shared_limit)
    async with ios_rs.DeviceSession(args.udid, pair_record_bytes, args.endpoint,
                                   concurrency=1, admission=pool) as session:
        # This call admits native work immediately and returns an asyncio Future.
        future = session.query_async(args.field, timeout=args.timeout,
                                     cleanup_timeout=args.cleanup_timeout)
        try:
            result = await future
        except TimeoutError as error:
            # A cancellation cleanup timeout does not claim the native task joined.
            # Keep this owned receipt and wait for its actual original completion.
            receipt = getattr(error, "query_receipt", None)
            if receipt is not None:
                try:
                    await receipt.wait_async(cleanup_timeout=args.cleanup_timeout)
                except RuntimeError:
                    pass  # The original native cancellation/failure remains typed.
            raise
        print(result)  # Result includes caller-selected device metadata/name.


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--udid", required=True)
    parser.add_argument("--pair-record", type=Path, required=True)
    parser.add_argument("--endpoint", required=True,
                        help="explicit numeric tcp://address:port or unix://socket")
    parser.add_argument("--field", choices=["ProductType", "ProductVersion", "DeviceName", "root"],
                        default="ProductVersion")
    parser.add_argument("--shared-limit", type=int, default=4)
    parser.add_argument("--timeout", type=float, default=15.0)
    parser.add_argument("--cleanup-timeout", type=float, default=5.0)
    args = parser.parse_args()
    # File policy belongs to this caller. The binding accepts immutable bytes only.
    with args.pair_record.open("rb") as stream:
        pair_record_bytes = stream.read(1024 * 1024 + 1)
    if not 1 <= len(pair_record_bytes) <= 1024 * 1024:
        parser.error("pair record must contain 1..1048576 bytes")
    asyncio.run(query(args, pair_record_bytes))


if __name__ == "__main__":
    main()
