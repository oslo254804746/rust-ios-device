#!/usr/bin/env python3
"""Device services example: apps listing, AFC file operations, and syslog streaming."""
import argparse
import asyncio
import json

import ios_rs


def run_sync(udid: str) -> None:
    print(f"\n--- Synchronous device operations for {udid} ---")
    with ios_rs.open_device(udid) as dev:
        print(f"Opened: {dev}")

        # 1. Product version
        try:
            version = dev.product_version()
            print(f"Product version: {version}")
        except Exception as e:
            print(f"Product version error: {e}")

        # 2. Installed applications
        try:
            apps = dev.list_apps()
            print(f"Installed user applications ({len(apps)} found):")
            for app in apps[:5]:
                print(f"  - {app.get('bundle_id')} ({app.get('display_name')}) v{app.get('version')}")
            if len(apps) > 5:
                print(f"  ... and {len(apps) - 5} more")
        except Exception as e:
            print(f"List apps error: {e}")

        # 3. AFC directory listing
        try:
            files = dev.list_files("/")
            print(f"AFC root files ({len(files)} found): {files[:10]}")
        except Exception as e:
            print(f"List files error: {e}")

        # 4. AFC read file
        try:
            data = dev.read_file("/iTunesArtwork", max_bytes=65536)
            print(f"Read AFC /iTunesArtwork: {len(data)} bytes")
        except Exception as e:
            print(f"Read file error (expected if not present): {e}")

        # 5. Syslog streaming
        try:
            messages = dev.read_syslog(max_messages=5, timeout_ms=3000)
            print(f"Syslog entries ({len(messages)} read):")
            for msg in messages:
                print(f"  [log] {msg[:100]}")
        except Exception as e:
            print(f"Syslog error: {e}")

    print(f"Closed device: closed={dev.closed}")


async def run_async(udid: str) -> None:
    print(f"\n--- Asynchronous device operations for {udid} ---")
    async with ios_rs.open_device(udid) as dev:
        print(f"Opened (async): {dev}")

        # Async product version
        try:
            version = await dev.async_product_version()
            print(f"Product version: {version}")
        except Exception as e:
            print(f"Product version error: {e}")

        # Async apps
        try:
            apps = await dev.async_list_apps()
            print(f"Installed applications (async): {len(apps)} apps")
        except Exception as e:
            print(f"Async list apps error: {e}")

        # Async files
        try:
            files = await dev.async_list_files("/")
            print(f"AFC files (async): {len(files)} entries")
        except Exception as e:
            print(f"Async list files error: {e}")

        # Async syslog
        try:
            logs = await dev.async_read_syslog(max_messages=3, timeout_ms=2000)
            print(f"Syslog (async): {len(logs)} messages")
        except Exception as e:
            print(f"Async syslog error: {e}")

    print(f"Closed device (async): closed={dev.closed}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--udid", help="Device UDID (defaults to first connected device)")
    parser.add_argument("--async-only", action="store_true", help="Run only async operations")
    args = parser.parse_args()

    udid = args.udid
    if not udid:
        devices = ios_rs.list_devices()
        if not devices:
            print("No iOS devices connected via usbmuxd.")
            return
        udid = devices[0]["udid"]
        print(f"Using discovered device: {udid}")

    if not args.async_only:
        run_sync(udid)

    asyncio.run(run_async(udid))


if __name__ == "__main__":
    main()
