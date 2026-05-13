#!/usr/bin/env python3
"""Use ios_rs userspace tunnels as pymobiledevice3's RemoteXPC transport.

This example demonstrates interoperability between rust-ios-device's Python
binding and pymobiledevice3:

1. ios_rs starts a CoreDevice userspace tunnel.
2. tunnel.asyncio_proxy() routes asyncio.open_connection() calls for the
   tunnel IPv6 address through the local userspace proxy.
3. pymobiledevice3's RemoteServiceDiscoveryService and CoreDevice service
   classes run unchanged over that patched asyncio transport.

The default run is read-only and only reports RSD peer information plus service
presence. The optional --probe-coredevice checks whether selected CoreDevice
services can be opened. It does not run WDA/XCTest, restore/reset, or a full
sysdiagnose capture.
"""

from __future__ import annotations

import argparse
import asyncio
import json
from collections.abc import Awaitable, Callable
from typing import Any

import ios_rs

try:
    from pymobiledevice3.remote.core_device.app_service import AppServiceService
    from pymobiledevice3.remote.core_device.device_info import DeviceInfoService
    from pymobiledevice3.remote.core_device.diagnostics_service import (
        DiagnosticsServiceService,
    )
    from pymobiledevice3.remote.remote_service_discovery import (
        RemoteServiceDiscoveryService,
    )
except ImportError as exc:  # pragma: no cover - import guard for example users
    raise SystemExit(
        "pymobiledevice3 is required for this example. Install it with:\n"
        "  uv pip install pymobiledevice3\n"
        "or run inside an environment where pymobiledevice3 is already installed."
    ) from exc


COREDEVICE_SERVICES = [
    "com.apple.coredevice.deviceinfo",
    "com.apple.coredevice.appservice",
    "com.apple.coredevice.diagnosticsservice",
    "com.apple.sysdiagnose.remote",
    "com.apple.sysdiagnose.remote.trusted",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Bridge ios_rs userspace tunnels into pymobiledevice3 RemoteXPC "
            "clients."
        )
    )
    parser.add_argument(
        "--udid",
        help="Device UDID. Defaults to the first device returned by ios_rs.list_devices().",
    )
    parser.add_argument(
        "--probe-coredevice",
        action="store_true",
        help=(
            "Try opening selected pymobiledevice3 CoreDevice service classes. "
            "Diagnostics is connect-only and does not capture sysdiagnose."
        ),
    )
    return parser.parse_args()


def choose_udid(explicit_udid: str | None) -> str:
    if explicit_udid:
        return explicit_udid

    devices = ios_rs.list_devices()
    if not devices:
        raise SystemExit("No iOS devices are visible to ios_rs.list_devices().")
    return devices[0]["udid"]


def summarize_peer_info(peer_info: dict[str, Any]) -> dict[str, Any]:
    properties = peer_info.get("Properties", {})
    services = peer_info.get("Services", {})
    return {
        "udid": properties.get("UniqueDeviceID"),
        "product_type": properties.get("ProductType"),
        "os_version": properties.get("OSVersion"),
        "service_count": len(services),
        "service_presence": {
            service_name: service_name in services
            for service_name in COREDEVICE_SERVICES
        },
    }


async def call_probe(
    label: str,
    factory: Callable[[], Any],
    action: Callable[[Any], Awaitable[Any]],
) -> dict[str, Any]:
    client = factory()
    try:
        await client.connect()
        result = await action(client)
    except Exception as exc:  # noqa: BLE001 - example should report library errors
        return {
            "label": label,
            "ok": False,
            "error_type": type(exc).__name__,
            "error": str(exc),
        }
    finally:
        service = getattr(client, "service", None)
        if service is not None:
            await client.close()

    return {"label": label, "ok": True, "result": result}


async def probe_coredevice_services(
    rsd: RemoteServiceDiscoveryService,
) -> list[dict[str, Any]]:
    return [
        await call_probe(
            "DeviceInfoService.get_lockstate",
            lambda: DeviceInfoService(rsd),
            lambda client: client.get_lockstate(),
        ),
        await call_probe(
            "DeviceInfoService.query_mobilegestalt",
            lambda: DeviceInfoService(rsd),
            lambda client: client.query_mobilegestalt(["ProductVersion", "ProductType"]),
        ),
        await call_probe(
            "AppServiceService.list_apps",
            lambda: AppServiceService(rsd),
            lambda client: client.list_apps(),
        ),
        await call_probe(
            "DiagnosticsServiceService.connect_only",
            lambda: DiagnosticsServiceService(rsd),
            lambda _client: asyncio.sleep(0, result="connected"),
        ),
    ]


async def run_with_pymobiledevice3(tunnel: Any, probe_coredevice: bool) -> dict[str, Any]:
    rsd = RemoteServiceDiscoveryService(
        (tunnel.server_address, tunnel.rsd_port),
        name="ios_rs-userspace",
    )
    await rsd.connect()
    try:
        result: dict[str, Any] = {
            "tunnel": tunnel.connect_info(),
            "rsd": summarize_peer_info(rsd.peer_info or {}),
        }
        if probe_coredevice:
            result["probes"] = await probe_coredevice_services(rsd)
        return result
    finally:
        await rsd.close()


def main() -> None:
    args = parse_args()
    udid = choose_udid(args.udid)
    tunnel = ios_rs.start_tunnel(udid, mode="userspace")
    try:
        with tunnel.asyncio_proxy():
            result = asyncio.run(
                run_with_pymobiledevice3(tunnel, args.probe_coredevice)
            )
        print(json.dumps(result, indent=2, sort_keys=True, default=str))
    finally:
        tunnel.close()


if __name__ == "__main__":
    main()
