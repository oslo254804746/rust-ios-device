"""Installed Python public API smoke; no device or pairing side effects."""
import ios_rs
import inspect

assert callable(ios_rs.list_devices)
assert callable(ios_rs.start_tunnel)
signature = inspect.signature(ios_rs.start_tunnel)
assert list(signature.parameters) == ["udid", "mode"]
assert signature.parameters["udid"].default is inspect.Parameter.empty
assert signature.parameters["mode"].default == "userspace"
assert not inspect.signature(ios_rs.list_devices).parameters
for name in ("server_address", "rsd_port", "userspace_port", "services", "service_ports",
             "service_features", "connect_info", "asyncio_proxy", "close"):
    assert hasattr(ios_rs.Tunnel, name), name
for name in ("__enter__", "__exit__", "install", "restore"):
    assert callable(getattr(ios_rs.AsyncioProxyPatch, name)), name
for cls in (ios_rs.Tunnel, ios_rs.AsyncioProxyPatch):
    try:
        cls()
    except TypeError:
        pass
    else:
        raise AssertionError("resource classes must be created by their documented factory")
try:
    ios_rs.start_tunnel("synthetic-python-credential-UDID", mode="synthetic-python-credential-UDID")
except ValueError as error:
    assert "userspace" in str(error) and "kernel" in str(error)
    assert "synthetic-python-credential-UDID" not in str(error)
    assert "synthetic-python-credential-UDID" not in repr(error)
else:
    raise AssertionError("invalid mode must raise ValueError before device access")
print("Installed Python API smoke passed")
