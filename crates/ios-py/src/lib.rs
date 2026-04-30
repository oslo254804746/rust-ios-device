//! ios_rs - Python bindings for device listing and CoreDevice tunnel workflows.
//!
//! The binding exposes tunnel metadata plus a small asyncio adapter so Python
//! libraries can reuse the userspace proxy without reimplementing its custom
//! 20-byte preamble.
//!
//! # Usage
//! ```python
//! import ios_rs
//!
//! devices = ios_rs.list_devices()
//! tunnel = ios_rs.start_tunnel(devices[0]["udid"])
//! print(tunnel.connect_info())
//!
//! with tunnel.asyncio_proxy():
//!     ...
//!
//! tunnel.close()
//! ```

#![allow(clippy::useless_conversion)]

use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use tokio::runtime::Runtime;

static RUNTIME: Lazy<Runtime> =
    Lazy::new(|| Runtime::new().expect("failed to create tokio runtime"));

const ASYNCIO_PROXY_HELPERS: &str = r#"
import ipaddress

def install_tunnel_open_connection(asyncio_module, remote_host, proxy_port):
    stack = getattr(asyncio_module, "_ios_rs_tunnel_proxy_stack", None)
    if stack is None:
        original_open_connection = asyncio_module.open_connection

        async def open_connection(*args, **kwargs):
            original = asyncio_module._ios_rs_original_open_connection
            if kwargs.get("sock") is not None:
                return await original(*args, **kwargs)

            if args:
                host = args[0] if len(args) > 0 else kwargs.get("host")
                port = args[1] if len(args) > 1 else kwargs.get("port")
                rest = args[2:]
            else:
                host = kwargs.get("host")
                port = kwargs.get("port")
                rest = ()

            if host is not None and port is not None:
                for _token, entry_host, entry_proxy_port in reversed(
                    asyncio_module._ios_rs_tunnel_proxy_stack
                ):
                    if host == entry_host:
                        passthrough_kwargs = {
                            key: value
                            for key, value in kwargs.items()
                            if key not in {"host", "port"}
                        }
                        reader, writer = await original(
                            "127.0.0.1",
                            entry_proxy_port,
                            *rest,
                            **passthrough_kwargs,
                        )
                        writer.write(ipaddress.IPv6Address(host).packed)
                        writer.write(int(port).to_bytes(4, "little"))
                        await writer.drain()
                        return reader, writer

            return await original(*args, **kwargs)

        asyncio_module._ios_rs_original_open_connection = original_open_connection
        asyncio_module._ios_rs_tunnel_proxy_stack = []
        asyncio_module.open_connection = open_connection

    token = object()
    asyncio_module._ios_rs_tunnel_proxy_stack.append((token, remote_host, proxy_port))
    return token


def restore_tunnel_open_connection(asyncio_module, token):
    stack = getattr(asyncio_module, "_ios_rs_tunnel_proxy_stack", None)
    if stack is None:
        return

    asyncio_module._ios_rs_tunnel_proxy_stack = [
        entry for entry in stack if entry[0] is not token
    ]
    if asyncio_module._ios_rs_tunnel_proxy_stack:
        return

    original = getattr(asyncio_module, "_ios_rs_original_open_connection", None)
    if original is not None:
        asyncio_module.open_connection = original
    if hasattr(asyncio_module, "_ios_rs_original_open_connection"):
        delattr(asyncio_module, "_ios_rs_original_open_connection")
    if hasattr(asyncio_module, "_ios_rs_tunnel_proxy_stack"):
        delattr(asyncio_module, "_ios_rs_tunnel_proxy_stack")
"#;

#[pymodule]
fn ios_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(list_devices, m)?)?;
    m.add_function(wrap_pyfunction!(start_tunnel, m)?)?;
    m.add_class::<Tunnel>()?;
    m.add_class::<AsyncioProxyPatch>()?;
    Ok(())
}

/// List all connected iOS devices visible to usbmuxd.
///
/// Returns a list of dicts:
///   {"udid": str, "device_id": int, "connection_type": str}
#[pyfunction]
fn list_devices(py: Python<'_>) -> PyResult<PyObject> {
    let devices = py
        .allow_threads(|| RUNTIME.block_on(ios_core::list_devices()))
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

    let list = PyList::empty_bound(py);
    for d in devices {
        let dict = PyDict::new_bound(py);
        dict.set_item("udid", &d.udid)?;
        dict.set_item("device_id", d.device_id)?;
        dict.set_item("connection_type", &d.connection_type)?;
        list.append(dict)?;
    }
    Ok(list.into())
}

/// Start a CDTunnel to the given device.
///
/// Args:
///   udid: device UDID (from list_devices)
///   mode: "userspace" (default, no root) or "kernel" (requires root/admin)
///
/// Returns a `Tunnel` object. The tunnel stays alive while you hold the object.
/// Call `tunnel.close()` or let it go out of scope to tear it down.
#[pyfunction]
#[pyo3(signature = (udid, mode = "userspace"))]
fn start_tunnel(py: Python<'_>, udid: &str, mode: &str) -> PyResult<Tunnel> {
    let tun_mode = match mode {
        "kernel" => ios_core::tunnel::TunMode::Kernel,
        _ => ios_core::tunnel::TunMode::Userspace,
    };
    let opts = ios_core::device::ConnectOptions {
        tun_mode,
        pair_record_path: None,
        skip_tunnel: false,
    };
    let udid = udid.to_string();
    let device = py
        .allow_threads(|| RUNTIME.block_on(ios_core::connect(&udid, opts)))
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

    let server_address = device.server_address().unwrap_or("").to_string();
    let rsd_port = device.rsd_port().unwrap_or(0);
    let userspace_port = device.userspace_port();

    let services: Vec<String> = device
        .rsd()
        .map(|r| r.services.keys().cloned().collect())
        .unwrap_or_default();

    Ok(Tunnel {
        server_address,
        rsd_port,
        userspace_port,
        services,
        _state: Arc::new(Mutex::new(TunnelState {
            device: Some(device),
            retained_patches: 0,
            close_requested: false,
        })),
    })
}

/// A live iOS CDTunnel.
///
/// Keep this object alive for the duration of your session.
/// The tunnel is torn down when you call `close()` or when the object is GC'd.
///
/// Attributes:
///   server_address (str):  Device's tunnel IPv6 address
///   rsd_port (int):        RSD service discovery port
///   userspace_port (int|None): Local TCP port for the go-ios-compatible proxy
///   services (list[str]):  Discovered RSD service names
#[pyclass]
pub struct Tunnel {
    #[pyo3(get)]
    pub server_address: String,
    #[pyo3(get)]
    pub rsd_port: u16,
    #[pyo3(get)]
    pub userspace_port: Option<u16>,
    #[pyo3(get)]
    pub services: Vec<String>,
    _state: Arc<Mutex<TunnelState>>,
}

struct TunnelState {
    device: Option<ios_core::ConnectedDevice>,
    retained_patches: usize,
    close_requested: bool,
}

struct TunnelKeepalive {
    state: Arc<Mutex<TunnelState>>,
    released: bool,
}

impl TunnelKeepalive {
    fn new(state: &Arc<Mutex<TunnelState>>) -> Self {
        if let Ok(mut guard) = state.lock() {
            guard.retained_patches += 1;
        }
        Self {
            state: Arc::clone(state),
            released: false,
        }
    }

    fn release(&mut self) {
        if self.released {
            return;
        }

        if let Ok(mut guard) = self.state.lock() {
            guard.retained_patches = guard.retained_patches.saturating_sub(1);
            if guard.retained_patches == 0 && guard.close_requested {
                guard.device.take();
            }
        }
        self.released = true;
    }
}

impl Drop for TunnelKeepalive {
    fn drop(&mut self) {
        self.release();
    }
}

#[pymethods]
impl Tunnel {
    fn __repr__(&self) -> String {
        format!(
            "Tunnel(server='{}', rsd_port={}, userspace_port={:?}, services={})",
            self.server_address,
            self.rsd_port,
            self.userspace_port,
            self.services.len(),
        )
    }

    /// Close the tunnel and release device resources.
    fn close(&self) {
        if let Ok(mut guard) = self._state.lock() {
            guard.close_requested = true;
            if guard.retained_patches == 0 {
                guard.device.take();
            }
        }
    }

    /// Connect instructions for common tools.
    fn connect_info(&self, py: Python<'_>) -> PyResult<PyObject> {
        let dict = PyDict::new_bound(py);
        dict.set_item("server_address", &self.server_address)?;
        dict.set_item("rsd_port", self.rsd_port)?;
        if let Some(p) = self.userspace_port {
            dict.set_item("userspace_port", p)?;
            dict.set_item(
                "proxy_hint",
                format!("TCP 127.0.0.1:{p} -> send 16B IPv6 + 4B LE port"),
            )?;
        }
        Ok(dict.into())
    }

    /// Create a context manager that patches asyncio.open_connection so
    /// asyncio-based libraries can use this userspace tunnel transparently.
    fn asyncio_proxy(&self) -> PyResult<AsyncioProxyPatch> {
        let proxy_port = self.userspace_port.ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(
                "userspace proxy is unavailable for this tunnel; start_tunnel(..., mode='userspace') is required",
            )
        })?;

        Ok(AsyncioProxyPatch {
            remote_host: self.server_address.clone(),
            proxy_port,
            patch_tokens: Vec::new(),
            state: Arc::clone(&self._state),
            keepalive: None,
        })
    }
}

#[pyclass]
pub struct AsyncioProxyPatch {
    remote_host: String,
    proxy_port: u16,
    patch_tokens: Vec<Py<PyAny>>,
    state: Arc<Mutex<TunnelState>>,
    keepalive: Option<TunnelKeepalive>,
}

#[pymethods]
impl AsyncioProxyPatch {
    fn __repr__(&self) -> String {
        format!(
            "AsyncioProxyPatch(remote_host='{}', proxy_port={})",
            self.remote_host, self.proxy_port
        )
    }

    fn __enter__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<PyObject> {
        slf.install(py)?;
        Ok(slf.into_py(py))
    }

    #[pyo3(signature = (_exc_type=None, _exc=None, _tb=None))]
    fn __exit__(
        &mut self,
        py: Python<'_>,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc: Option<&Bound<'_, PyAny>>,
        _tb: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        self.restore(py)
    }

    /// Install the asyncio.open_connection monkeypatch.
    fn install(&mut self, py: Python<'_>) -> PyResult<()> {
        let asyncio = py.import_bound("asyncio")?;
        let helper_module = PyModule::from_code_bound(
            py,
            ASYNCIO_PROXY_HELPERS,
            "ios_rs_asyncio_proxy.py",
            "ios_rs_asyncio_proxy",
        )?;
        let token = helper_module
            .getattr("install_tunnel_open_connection")?
            .call1((asyncio.clone(), self.remote_host.clone(), self.proxy_port))?
            .unbind();
        if self.patch_tokens.is_empty() {
            self.keepalive = Some(TunnelKeepalive::new(&self.state));
        }
        self.patch_tokens.push(token);
        Ok(())
    }

    /// Restore asyncio.open_connection to its original implementation.
    fn restore(&mut self, py: Python<'_>) -> PyResult<()> {
        if let Some(token) = self.patch_tokens.pop() {
            let asyncio = py.import_bound("asyncio")?;
            let helper_module = PyModule::from_code_bound(
                py,
                ASYNCIO_PROXY_HELPERS,
                "ios_rs_asyncio_proxy.py",
                "ios_rs_asyncio_proxy",
            )?;
            helper_module
                .getattr("restore_tunnel_open_connection")?
                .call1((asyncio, token.bind(py)))?;
            if self.patch_tokens.is_empty() {
                if let Some(mut keepalive) = self.keepalive.take() {
                    keepalive.release();
                }
            }
        }
        Ok(())
    }
}

impl AsyncioProxyPatch {
    fn restore_all(&mut self, py: Python<'_>) -> PyResult<()> {
        while !self.patch_tokens.is_empty() {
            self.restore(py)?;
        }
        Ok(())
    }
}

impl Drop for AsyncioProxyPatch {
    fn drop(&mut self) {
        if self.patch_tokens.is_empty() {
            return;
        }

        if unsafe { pyo3::ffi::Py_IsInitialized() } == 0 {
            return;
        }

        Python::with_gil(|py| {
            let _ = self.restore_all(py);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asyncio_proxy_patch_restores_original_after_out_of_order_nested_restore() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let asyncio = py
                .import_bound("asyncio")
                .expect("asyncio module should import");
            let original = asyncio
                .getattr("open_connection")
                .expect("asyncio.open_connection should exist")
                .unbind();

            let mut outer = AsyncioProxyPatch {
                remote_host: "fd00::1".into(),
                proxy_port: 49160,
                patch_tokens: Vec::new(),
                state: Arc::new(Mutex::new(TunnelState {
                    device: None,
                    retained_patches: 0,
                    close_requested: false,
                })),
                keepalive: None,
            };
            let mut inner = AsyncioProxyPatch {
                remote_host: "fd00::2".into(),
                proxy_port: 49161,
                patch_tokens: Vec::new(),
                state: Arc::new(Mutex::new(TunnelState {
                    device: None,
                    retained_patches: 0,
                    close_requested: false,
                })),
                keepalive: None,
            };

            outer.install(py).expect("outer patch should install");
            inner.install(py).expect("inner patch should install");
            outer
                .restore(py)
                .expect("restoring outer patch should succeed");
            let still_patched_after_outer_restore = !asyncio
                .getattr("open_connection")
                .expect("patched open_connection should exist")
                .is(original.bind(py));

            inner
                .restore(py)
                .expect("restoring inner patch should succeed");
            let restored_to_original = asyncio
                .getattr("open_connection")
                .expect("restored open_connection should exist")
                .is(original.bind(py));

            assert!(
                still_patched_after_outer_restore,
                "restoring an outer patch must not drop a newer nested patch"
            );
            assert!(
                restored_to_original,
                "restoring the final patch should put asyncio.open_connection back"
            );
        });
    }

    #[test]
    fn asyncio_proxy_patch_supports_nested_reuse_of_same_object() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let asyncio = py
                .import_bound("asyncio")
                .expect("asyncio module should import");
            let original = asyncio
                .getattr("open_connection")
                .expect("asyncio.open_connection should exist")
                .unbind();

            let mut patch = AsyncioProxyPatch {
                remote_host: "fd00::3".into(),
                proxy_port: 49162,
                patch_tokens: Vec::new(),
                state: Arc::new(Mutex::new(TunnelState {
                    device: None,
                    retained_patches: 0,
                    close_requested: false,
                })),
                keepalive: None,
            };

            patch.install(py).expect("outer install should succeed");
            patch.install(py).expect("inner install should succeed");
            patch.restore(py).expect("inner restore should succeed");

            let still_patched_after_inner_restore = !asyncio
                .getattr("open_connection")
                .expect("patched open_connection should exist")
                .is(original.bind(py));

            patch.restore(py).expect("outer restore should succeed");
            let restored_to_original = asyncio
                .getattr("open_connection")
                .expect("restored open_connection should exist")
                .is(original.bind(py));

            assert!(
                still_patched_after_inner_restore,
                "restoring the inner install must not tear down an outer install from the same object"
            );
            assert!(
                restored_to_original,
                "restoring the final install should put asyncio.open_connection back"
            );
        });
    }

    #[test]
    fn asyncio_proxy_patch_drop_restores_original_open_connection() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let asyncio = py
                .import_bound("asyncio")
                .expect("asyncio module should import");
            let original = asyncio
                .getattr("open_connection")
                .expect("asyncio.open_connection should exist")
                .unbind();

            {
                let mut patch = AsyncioProxyPatch {
                    remote_host: "fd00::4".into(),
                    proxy_port: 49163,
                    patch_tokens: Vec::new(),
                    state: Arc::new(Mutex::new(TunnelState {
                        device: None,
                        retained_patches: 0,
                        close_requested: false,
                    })),
                    keepalive: None,
                };
                patch.install(py).expect("install should succeed");
                assert!(
                    !asyncio
                        .getattr("open_connection")
                        .expect("patched open_connection should exist")
                        .is(original.bind(py)),
                    "patch install should replace asyncio.open_connection"
                );
            }

            assert!(
                asyncio
                    .getattr("open_connection")
                    .expect("restored open_connection should exist")
                    .is(original.bind(py)),
                "dropping the patch object should restore asyncio.open_connection"
            );
        });
    }

    #[test]
    fn asyncio_proxy_patch_only_retains_tunnel_while_installed() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let state = Arc::new(Mutex::new(TunnelState {
                device: None,
                retained_patches: 0,
                close_requested: false,
            }));
            let mut patch = AsyncioProxyPatch {
                remote_host: "fd00::5".into(),
                proxy_port: 49164,
                patch_tokens: Vec::new(),
                state: Arc::clone(&state),
                keepalive: None,
            };

            assert_eq!(state.lock().unwrap().retained_patches, 0);

            patch.install(py).expect("install should succeed");
            assert_eq!(
                state.lock().unwrap().retained_patches,
                1,
                "the first active install should retain the parent tunnel"
            );

            patch.install(py).expect("nested install should succeed");
            assert_eq!(
                state.lock().unwrap().retained_patches,
                1,
                "nested installs on the same patch should not stack extra tunnel retains"
            );

            patch.restore(py).expect("nested restore should succeed");
            assert_eq!(
                state.lock().unwrap().retained_patches,
                1,
                "restoring an inner install must keep the outer retain active"
            );

            patch.restore(py).expect("final restore should succeed");
            assert_eq!(
                state.lock().unwrap().retained_patches,
                0,
                "once the final install is restored the tunnel retain should be released"
            );
        });
    }
}
