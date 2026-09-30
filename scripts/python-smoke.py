"""Installed Python public API smoke; no device or pairing side effects."""
import ios_rs

assert callable(ios_rs.list_devices)
assert callable(ios_rs.start_tunnel)
for name in ("server_address", "rsd_port", "userspace_port", "services", "service_ports",
             "service_features", "connect_info", "asyncio_proxy", "close"):
    assert hasattr(ios_rs.Tunnel, name), name
try:
    ios_rs.start_tunnel("synthetic-unreachable-device", mode="invalid")
except ValueError as error:
    assert "userspace" in str(error) and "kernel" in str(error)
else:
    raise AssertionError("invalid mode must raise ValueError before device access")
print("Installed Python API smoke passed")
