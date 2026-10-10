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

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use tokio::runtime::Runtime;

mod device_session;

static RUNTIME: LazyLock<std::io::Result<Runtime>> = LazyLock::new(Runtime::new);

/// Ordinary initialization errors are cached and do not invoke device work.
/// The caller evaluates this inside allow_threads, including lazy initialization.
fn with_runtime<T>(
    initialization: &std::io::Result<Runtime>,
    operation: impl FnOnce(&Runtime) -> T,
) -> Result<T, &std::io::Error> {
    initialization.as_ref().map(operation)
}

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

const ASYNC_DEVICE_HELPERS: &str = r#"
import asyncio

async def device_aenter(dev):
    return dev

async def device_aexit(dev, exc_type, exc_val, exc_tb):
    try:
        await dev.aclose()
    except BaseException as error:
        if exc_val is not None:
            error.__context__ = exc_val
        raise

async def device_aclose(dev):
    dev.close()

async def product_version(future):
    result = await future
    return result["value"]

async def tunnel_aclose(tunnel):
    await tunnel._close_services()
    tunnel.close()

async def noop():
    pass

def run_async(func, *args):
    # All tunnel service calls share its checked owner. The native Future
    # cancels only its lease and joins the original task.
    dev = func.__self__.get_device()
    future = getattr(dev, "async_" + func.__name__)(*args)
    async def owned():
        return await future
    return owned()
"#;

#[pymodule]
fn ios_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(list_devices, m)?)?;
    m.add_function(wrap_pyfunction!(open_device, m)?)?;
    m.add_function(wrap_pyfunction!(start_tunnel, m)?)?;
    m.add_class::<Device>()?;
    m.add_class::<Tunnel>()?;
    m.add_class::<AsyncioProxyPatch>()?;
    device_session::register(m)?;
    Ok(())
}

/// List all connected iOS devices visible to usbmuxd.
///
/// Returns a list of dicts:
///   {"udid": str, "device_id": int, "connection_type": str}
#[pyfunction]
fn list_devices(py: Python<'_>) -> PyResult<PyObject> {
    let devices = py
        .allow_threads(|| {
            with_runtime(&RUNTIME, |runtime| {
                runtime.block_on(ios_core::list_devices())
            })
        })
        .map_err(|error| operation_error(py, "initialize runtime", error))?
        .map_err(|e| operation_error(py, "list devices", &e))?;

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
    let tun_mode = parse_tunnel_mode(mode)?;
    let opts = ios_core::device::ConnectOptions {
        tun_mode,
        pair_record_path: None,
        skip_tunnel: false,
    };
    let udid = udid.to_string();
    let device = py
        .allow_threads(|| {
            with_runtime(&RUNTIME, |runtime| {
                runtime.block_on(ios_core::connect(&udid, opts))
            })
        })
        .map_err(|error| operation_error(py, "initialize runtime", error))?
        .map_err(|e| operation_error(py, "start tunnel", &e))?;

    let server_address = device.server_address().unwrap_or("").to_string();
    let rsd_port = device.rsd_port().unwrap_or(0);
    let userspace_port = device.userspace_port();

    let (mut services, service_ports, service_features) = device
        .rsd()
        .map(|rsd| {
            let ports = rsd
                .services
                .iter()
                .map(|(name, descriptor)| (name.clone(), descriptor.port))
                .collect::<BTreeMap<_, _>>();
            let features = rsd
                .services
                .iter()
                .map(|(name, descriptor)| (name.clone(), descriptor.features.clone()))
                .collect::<BTreeMap<_, _>>();
            (
                rsd.services.keys().cloned().collect::<Vec<_>>(),
                ports,
                features,
            )
        })
        .unwrap_or_default();
    services.sort();

    Ok(Tunnel {
        server_address,
        rsd_port,
        userspace_port,
        services,
        service_ports,
        service_features,
        _state: Arc::new(Mutex::new(TunnelState {
            device: Some(Arc::new(device)),
            checked: None,
            retained_patches: 0,
            close_requested: false,
        })),
    })
}

fn exception_with_metadata(
    py: Python<'_>,
    error: PyErr,
    class: ios_core::ErrorClass,
    phase: &'static str,
    effects: &'static str,
) -> PyErr {
    let attach = || -> PyResult<()> {
        let value = error.value_bound(py);
        value.setattr("ios_error_schema_version", 1)?;
        value.setattr("ios_error_class", class.as_str())?;
        value.setattr("ios_error_phase", phase)?;
        value.setattr("ios_error_effects", effects)
    };
    match attach() {
        Ok(()) => error,
        Err(metadata_error) => {
            // A metadata allocation/assignment failure remains an exception;
            // its cause contains only the original safe Python exception.
            metadata_error.set_cause(py, Some(error));
            metadata_error
        }
    }
}

fn operation_error(
    py: Python<'_>,
    phase: &'static str,
    error: &(dyn std::error::Error + 'static),
) -> PyErr {
    let exception = pyo3::exceptions::PyRuntimeError::new_err(format!(
        "{phase} failed: {}",
        ios_core::redaction::diagnostic_error(error)
    ));
    exception_with_metadata(
        py,
        exception,
        ios_core::classify_error(error),
        phase,
        "unknown",
    )
}

fn parse_tunnel_mode(mode: &str) -> PyResult<ios_core::TunMode> {
    match mode {
        "userspace" => Ok(ios_core::TunMode::Userspace),
        "kernel" => Ok(ios_core::TunMode::Kernel),
        _ => Err(Python::with_gil(|py| {
            exception_with_metadata(
                py,
                PyValueError::new_err("invalid tunnel mode; expected \"userspace\" or \"kernel\""),
                ios_core::ErrorClass::InvalidInput,
                "tunnel mode",
                "not_started",
            )
        })),
    }
}

fn apps_to_pylist(py: Python<'_>, apps: &[ios_core::apps::AppInfo]) -> PyResult<PyObject> {
    let list = PyList::empty_bound(py);
    for app in apps {
        let dict = PyDict::new_bound(py);
        dict.set_item("bundle_id", &app.bundle_id)?;
        dict.set_item("display_name", &app.display_name)?;
        dict.set_item("version", &app.version)?;
        dict.set_item("app_type", &app.app_type)?;
        dict.set_item("path", &app.path)?;
        list.append(dict)?;
    }
    Ok(list.into())
}

fn files_to_pylist(py: Python<'_>, files: &[String]) -> PyResult<PyObject> {
    let list = PyList::empty_bound(py);
    for f in files {
        list.append(f)?;
    }
    Ok(list.into())
}

/// Open a connected iOS device by UDID for app, AFC, and syslog operations.
#[pyfunction]
fn open_device(udid: &str, py: Python<'_>) -> PyResult<Device> {
    if udid.trim().is_empty() {
        return Err(exception_with_metadata(
            py,
            PyValueError::new_err("device UDID cannot be empty"),
            ios_core::ErrorClass::InvalidInput,
            "open device",
            "not_started",
        ));
    }
    let checked = py
        .allow_threads(|| device_session::DeviceSession::local(udid))
        .map_err(|e| operation_error(py, "open device", &e))?;
    Ok(Device {
        udid: udid.into(),
        device: Arc::new(Mutex::new(Some(Arc::new(checked)))),
    })
}

/// A handle to a connected iOS device for services (apps, AFC files, syslog).
#[derive(Clone)]
#[pyclass]
pub struct Device {
    #[pyo3(get)]
    pub udid: String,
    device: Arc<Mutex<Option<Arc<device_session::DeviceSession>>>>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("udid", &self.udid)
            .field("closed", &self.closed())
            .finish()
    }
}

impl Device {
    fn get_connected_device(&self, py: Python<'_>) -> PyResult<Arc<device_session::DeviceSession>> {
        let guard = self.device.lock().map_err(|_| {
            exception_with_metadata(
                py,
                pyo3::exceptions::PyRuntimeError::new_err("device lock poisoned"),
                ios_core::ErrorClass::Conflict,
                "device access",
                "not_started",
            )
        })?;
        guard
            .as_ref()
            .filter(|d| !d.closed())
            .cloned()
            .ok_or_else(|| {
                exception_with_metadata(
                    py,
                    pyo3::exceptions::PyRuntimeError::new_err("device is closed"),
                    ios_core::ErrorClass::Precondition,
                    "device access",
                    "not_started",
                )
            })
    }
}

#[pymethods]
impl Device {
    fn __repr__(&self) -> String {
        let is_closed = self
            .device
            .lock()
            .map(|g| g.as_ref().map(|d| d.closed()).unwrap_or(true))
            .unwrap_or(true);
        format!(
            "Device(udid='{}', closed={})",
            ios_core::redaction::diagnostic_id(&self.udid),
            is_closed
        )
    }

    #[getter]
    fn closed(&self) -> bool {
        self.device
            .lock()
            .map(|g| g.as_ref().map(|d| d.closed()).unwrap_or(true))
            .unwrap_or(true)
    }

    /// Seal new admission, cancel active jobs, and join their original tasks.
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let device = self
            .device
            .lock()
            .map_err(|_| {
                operation_error(
                    py,
                    "device close",
                    &std::io::Error::other("device lock poisoned"),
                )
            })?
            .as_ref()
            .cloned();
        if let Some(device) = device {
            device.close(py, 5.0)?;
        }
        Ok(())
    }
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let device = self
            .device
            .lock()
            .map_err(|_| {
                operation_error(
                    py,
                    "device close",
                    &std::io::Error::other("device lock poisoned"),
                )
            })?
            .as_ref()
            .cloned();
        if let Some(device) = device {
            return device.aclose(py, 5.0).map(|value| value.into_bound(py));
        }
        let helpers = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        helpers.getattr("device_aclose")?.call1((self.clone(),))
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[pyo3(signature = (_exc_type=None, _exc=None, _tb=None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc: Option<&Bound<'_, PyAny>>,
        _tb: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        self.close(py)
    }

    fn __aenter__<'py>(slf: &Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let helpers = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        helpers.getattr("device_aenter")?.call1((slf,))
    }

    #[pyo3(signature = (exc_type=None, exc=None, tb=None))]
    fn __aexit__<'py>(
        slf: &Bound<'py, Self>,
        py: Python<'py>,
        exc_type: Option<&Bound<'_, PyAny>>,
        exc: Option<&Bound<'_, PyAny>>,
        tb: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let helpers = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        helpers
            .getattr("device_aexit")?
            .call1((slf, exc_type, exc, tb))
    }

    fn product_version(&self, py: Python<'_>) -> PyResult<String> {
        let device = self.get_connected_device(py)?;
        let result = device.query(py, "ProductVersion", 15.0, 1048576, 4096, None)?;
        result.bind(py).get_item("value")?.extract()
    }
    fn list_apps(&self, py: Python<'_>) -> PyResult<PyObject> {
        self.get_connected_device(py)?.run_service(
            py,
            ios_core::session::DeviceServiceRequest::ListApps,
            15.0,
            Default::default(),
        )
    }
    #[pyo3(signature=(path="/"))]
    fn list_files(&self, path: &str, py: Python<'_>) -> PyResult<PyObject> {
        self.get_connected_device(py)?.run_service(
            py,
            ios_core::session::DeviceServiceRequest::ListFiles { path: path.into() },
            15.0,
            Default::default(),
        )
    }
    #[pyo3(signature=(path,max_bytes=1048576))]
    fn read_file(&self, path: &str, max_bytes: usize, py: Python<'_>) -> PyResult<PyObject> {
        let limits = ios_core::session::DeviceServiceLimits {
            output_bytes: max_bytes.max(1),
            ..Default::default()
        };
        self.get_connected_device(py)?.run_service(
            py,
            ios_core::session::DeviceServiceRequest::ReadFile {
                path: path.into(),
                max_bytes,
            },
            15.0,
            limits,
        )
    }
    #[pyo3(signature=(max_messages=100,timeout_ms=5000))]
    fn read_syslog(
        &self,
        max_messages: usize,
        timeout_ms: u64,
        py: Python<'_>,
    ) -> PyResult<PyObject> {
        self.get_connected_device(py)?.run_service(
            py,
            ios_core::session::DeviceServiceRequest::ReadSyslog { max_messages },
            timeout_ms as f64 / 1000.0,
            Default::default(),
        )
    }
    fn async_product_version<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let future = self.get_connected_device(py)?.query_async(
            py,
            "ProductVersion",
            15.0,
            1048576,
            4096,
            None,
            5.0,
        )?;
        let helper = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        helper.getattr("product_version")?.call1((future,))
    }
    fn async_list_apps<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.get_connected_device(py)?
            .async_service(
                py,
                ios_core::session::DeviceServiceRequest::ListApps,
                15.0,
                Default::default(),
            )
            .map(|v| v.into_bound(py))
    }
    #[pyo3(signature=(path="/"))]
    fn async_list_files<'py>(&self, path: &str, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.get_connected_device(py)?
            .async_service(
                py,
                ios_core::session::DeviceServiceRequest::ListFiles { path: path.into() },
                15.0,
                Default::default(),
            )
            .map(|v| v.into_bound(py))
    }
    #[pyo3(signature=(path,max_bytes=1048576))]
    fn async_read_file<'py>(
        &self,
        path: &str,
        max_bytes: usize,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let limits = ios_core::session::DeviceServiceLimits {
            output_bytes: max_bytes.max(1),
            ..Default::default()
        };
        self.get_connected_device(py)?
            .async_service(
                py,
                ios_core::session::DeviceServiceRequest::ReadFile {
                    path: path.into(),
                    max_bytes,
                },
                15.0,
                limits,
            )
            .map(|v| v.into_bound(py))
    }
    #[pyo3(signature=(max_messages=100,timeout_ms=5000))]
    fn async_read_syslog<'py>(
        &self,
        max_messages: usize,
        timeout_ms: u64,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.get_connected_device(py)?
            .async_service(
                py,
                ios_core::session::DeviceServiceRequest::ReadSyslog { max_messages },
                timeout_ms as f64 / 1000.0,
                Default::default(),
            )
            .map(|v| v.into_bound(py))
    }
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
///   service_ports (dict[str, int]): RSD service names mapped to device ports
///   service_features (dict[str, list[str]]): Every RSD service's advertised
///     capability identifiers; an empty list means metadata was not advertised
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
    #[pyo3(get)]
    pub service_ports: BTreeMap<String, u16>,
    #[pyo3(get)]
    pub service_features: BTreeMap<String, Vec<String>>,
    _state: Arc<Mutex<TunnelState>>,
}

struct TunnelState {
    device: Option<Arc<ios_core::ConnectedDevice>>,
    checked: Option<Arc<device_session::DeviceSession>>,
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
            ios_core::redaction::diagnostic_id(&self.server_address),
            self.rsd_port,
            self.userspace_port,
            self.services.len(),
        )
    }

    /// Close the tunnel and release device resources.
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let checked = {
            let mut guard = self._state.lock().map_err(|_| {
                operation_error(
                    py,
                    "tunnel close",
                    &std::io::Error::other("tunnel lock poisoned"),
                )
            })?;
            guard.close_requested = true;
            let checked = guard.checked.clone();
            if guard.retained_patches == 0 {
                guard.device.take();
            }
            checked
        };
        if let Some(checked) = checked {
            checked.close(py, 5.0)?;
        }
        Ok(())
    }

    fn _close_services(&self, py: Python<'_>) -> PyResult<PyObject> {
        let checked = {
            let mut guard = self._state.lock().map_err(|_| {
                operation_error(
                    py,
                    "tunnel close",
                    &std::io::Error::other("tunnel lock poisoned"),
                )
            })?;
            guard.close_requested = true;
            guard.checked.clone()
        };
        if let Some(checked) = checked {
            return checked.aclose(py, 5.0);
        }
        let helper = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        Ok(helper.getattr("noop")?.call0()?.into())
    }

    /// Connect instructions for common tools.
    fn connect_info(&self, py: Python<'_>) -> PyResult<PyObject> {
        let dict = PyDict::new_bound(py);
        dict.set_item("server_address", &self.server_address)?;
        dict.set_item("rsd_port", self.rsd_port)?;
        dict.set_item("service_ports", self.service_ports.clone())?;
        dict.set_item("service_features", self.service_features.clone())?;
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
    fn asyncio_proxy(&self, py: Python<'_>) -> PyResult<AsyncioProxyPatch> {
        let proxy_port = self.userspace_port.ok_or_else(|| {
            exception_with_metadata(
                py,
                pyo3::exceptions::PyRuntimeError::new_err(
                    "userspace proxy is unavailable for this tunnel; start_tunnel(..., mode='userspace') is required",
                ),
                ios_core::ErrorClass::Precondition,
                "asyncio proxy",
                "not_started",
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

    /// Return a Device handle sharing this tunnel's connection.
    fn get_device(&self, py: Python<'_>) -> PyResult<Device> {
        let mut guard = self._state.lock().map_err(|_| {
            exception_with_metadata(
                py,
                pyo3::exceptions::PyRuntimeError::new_err("tunnel lock poisoned"),
                ios_core::ErrorClass::Conflict,
                "tunnel get_device",
                "not_started",
            )
        })?;
        let device_arc = guard
            .device
            .as_ref()
            .filter(|_| !guard.close_requested)
            .cloned()
            .ok_or_else(|| {
                exception_with_metadata(
                    py,
                    pyo3::exceptions::PyRuntimeError::new_err("tunnel device is closed"),
                    ios_core::ErrorClass::Precondition,
                    "tunnel get_device",
                    "not_started",
                )
            })?;
        let checked = match &guard.checked {
            Some(checked) => Arc::clone(checked),
            None => {
                let checked = Arc::new(
                    device_session::DeviceSession::from_connected(&device_arc)
                        .map_err(|e| operation_error(py, "tunnel get_device", &e))?,
                );
                guard.checked = Some(Arc::clone(&checked));
                checked
            }
        };
        Ok(Device {
            udid: self.server_address.clone(),
            device: Arc::new(Mutex::new(Some(checked))),
        })
    }

    /// List user-installed applications through the tunnel's underlying device connection.
    fn list_apps(&self, py: Python<'_>) -> PyResult<PyObject> {
        self.get_device(py)?.list_apps(py)
    }

    /// List directory contents via AFC through the tunnel's underlying device connection.
    #[pyo3(signature = (path = "/"))]
    fn list_files(&self, path: &str, py: Python<'_>) -> PyResult<PyObject> {
        self.get_device(py)?.list_files(path, py)
    }

    /// Read file content via AFC through the tunnel's underlying device connection.
    #[pyo3(signature = (path, max_bytes = 1024 * 1024))]
    fn read_file(&self, path: &str, max_bytes: usize, py: Python<'_>) -> PyResult<PyObject> {
        self.get_device(py)?.read_file(path, max_bytes, py)
    }

    /// Read syslog messages through the tunnel's underlying device connection.
    #[pyo3(signature = (max_messages = 100, timeout_ms = 5000))]
    fn read_syslog(
        &self,
        max_messages: usize,
        timeout_ms: u64,
        py: Python<'_>,
    ) -> PyResult<PyObject> {
        self.get_device(py)?
            .read_syslog(max_messages, timeout_ms, py)
    }

    /// Async adapter for close.
    fn aclose<'py>(slf: &Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let helpers = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        helpers.getattr("tunnel_aclose")?.call1((slf,))
    }

    /// Async adapter for list_apps.
    fn async_list_apps<'py>(
        slf: &Bound<'py, Self>,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let helpers = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        let method = slf.getattr("list_apps")?;
        helpers.getattr("run_async")?.call1((method,))
    }

    /// Async adapter for list_files.
    #[pyo3(signature = (path = "/"))]
    fn async_list_files<'py>(
        slf: &Bound<'py, Self>,
        path: &str,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let helpers = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        let method = slf.getattr("list_files")?;
        helpers.getattr("run_async")?.call1((method, path))
    }

    /// Async adapter for read_file.
    #[pyo3(signature = (path, max_bytes = 1024 * 1024))]
    fn async_read_file<'py>(
        slf: &Bound<'py, Self>,
        path: &str,
        max_bytes: usize,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let helpers = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        let method = slf.getattr("read_file")?;
        helpers
            .getattr("run_async")?
            .call1((method, path, max_bytes))
    }

    /// Async adapter for read_syslog.
    #[pyo3(signature = (max_messages = 100, timeout_ms = 5000))]
    fn async_read_syslog<'py>(
        slf: &Bound<'py, Self>,
        max_messages: usize,
        timeout_ms: u64,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let helpers = PyModule::from_code_bound(
            py,
            ASYNC_DEVICE_HELPERS,
            "ios_rs_async_device.py",
            "ios_rs_async_device",
        )?;
        let method = slf.getattr("read_syslog")?;
        helpers
            .getattr("run_async")?
            .call1((method, max_messages, timeout_ms))
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
            ios_core::redaction::diagnostic_id(&self.remote_host),
            self.proxy_port
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

    fn metadata(py: Python<'_>, error: &PyErr) -> (u8, String, String, String) {
        let value = error.value_bound(py);
        (
            value
                .getattr("ios_error_schema_version")
                .unwrap()
                .extract()
                .unwrap(),
            value.getattr("ios_error_class").unwrap().extract().unwrap(),
            value.getattr("ios_error_phase").unwrap().extract().unwrap(),
            value
                .getattr("ios_error_effects")
                .unwrap()
                .extract()
                .unwrap(),
        )
    }

    #[test]
    fn cached_runtime_io_failure_skips_operation_and_preserves_safe_python_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        pyo3::prepare_freethreaded_python();
        let starts = AtomicUsize::new(0);
        let operations = AtomicUsize::new(0);
        let secret = "synthetic-private-runtime-initialization-source";
        let initialization: LazyLock<std::io::Result<Runtime>, _> = LazyLock::new(|| {
            starts.fetch_add(1, Ordering::SeqCst);
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, secret))
        });
        for _ in 0..2 {
            let source = with_runtime(&initialization, |_| {
                operations.fetch_add(1, Ordering::SeqCst);
                42
            })
            .unwrap_err();
            assert_eq!(source.kind(), std::io::ErrorKind::TimedOut);
            assert_eq!(source.to_string(), secret);
            Python::with_gil(|py| {
                let error = operation_error(py, "initialize runtime", source);
                assert!(error.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py));
                assert_eq!(
                    metadata(py, &error),
                    (
                        1,
                        "timeout".into(),
                        "initialize runtime".into(),
                        "unknown".into()
                    )
                );
                let (message,) = error
                    .value_bound(py)
                    .getattr("args")
                    .unwrap()
                    .extract::<(String,)>()
                    .unwrap();
                assert!(message.starts_with("initialize runtime failed: io/TimedOut error:"));
                assert!(!format!("{message} {error:?}").contains(secret));
                assert!(error.cause(py).is_none());
            });
        }
        assert_eq!(
            starts.load(Ordering::SeqCst),
            1,
            "ordinary failure stays cached without poisoning"
        );
        assert_eq!(operations.load(Ordering::SeqCst), 0);

        let ready = tokio::runtime::Builder::new_current_thread().build();
        let native = with_runtime(&ready, |runtime| {
            operations.fetch_add(1, Ordering::SeqCst);
            runtime.block_on(std::future::ready(Err::<(), _>(ios_core::CoreError::Io(
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, secret),
            ))))
        })
        .unwrap()
        .unwrap_err();
        assert_eq!(operations.load(Ordering::SeqCst), 1);
        Python::with_gil(|py| {
            let error = operation_error(py, "list devices", &native);
            assert_eq!(
                metadata(py, &error),
                (
                    1,
                    "permission_denied".into(),
                    "list devices".into(),
                    "unknown".into()
                )
            );
            assert!(!format!("{error:?}").contains(secret));
        });
    }

    #[test]
    fn python_core_error_metadata_keeps_safe_args_and_original_rust_source() {
        pyo3::prepare_freethreaded_python();
        let secret = "synthetic-private-python-source";
        let raw =
            ios_core::CoreError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, secret));
        Python::with_gil(|py| {
            let error = operation_error(py, "list devices", &raw);
            assert!(error.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py));
            assert_eq!(
                metadata(py, &error),
                (1, "timeout".into(), "list devices".into(), "unknown".into())
            );
            let expected = format!(
                "list devices failed: {}",
                ios_core::redaction::diagnostic_error(&raw)
            );
            assert_eq!(
                error
                    .value_bound(py)
                    .getattr("args")
                    .unwrap()
                    .extract::<(String,)>()
                    .unwrap(),
                (expected,)
            );
            assert!(
                !format!("{} {error:?}", error.value_bound(py).str().unwrap()).contains(secret)
            );
            assert!(error.cause(py).is_none());
        });
        let source = std::error::Error::source(&raw)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(source.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(source.to_string(), secret);
    }

    #[test]
    fn python_input_and_proxy_precondition_metadata_keep_builtin_errors() {
        pyo3::prepare_freethreaded_python();
        let secret = "synthetic-private-python-input";
        let tunnel = Tunnel {
            server_address: secret.into(),
            rsd_port: 1234,
            userspace_port: None,
            services: vec![],
            service_ports: BTreeMap::new(),
            service_features: BTreeMap::new(),
            _state: Arc::new(Mutex::new(TunnelState {
                device: None,
                checked: None,
                retained_patches: 0,
                close_requested: false,
            })),
        };
        Python::with_gil(|py| {
            let error = parse_tunnel_mode(secret).unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py));
            assert_eq!(
                metadata(py, &error),
                (
                    1,
                    "invalid_input".into(),
                    "tunnel mode".into(),
                    "not_started".into()
                )
            );
            assert_eq!(
                error
                    .value_bound(py)
                    .getattr("args")
                    .unwrap()
                    .extract::<(String,)>()
                    .unwrap(),
                ("invalid tunnel mode; expected \"userspace\" or \"kernel\"".into(),)
            );
            let error = tunnel.asyncio_proxy(py).err().unwrap();
            assert!(error.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py));
            assert_eq!(
                metadata(py, &error),
                (
                    1,
                    "precondition".into(),
                    "asyncio proxy".into(),
                    "not_started".into()
                )
            );
            assert!(!format!("{error} {error:?}").contains(secret));
        });
        assert_eq!(tunnel._state.lock().unwrap().retained_patches, 0);
    }

    #[test]
    fn python_error_metadata_distinguishes_observed_cancel_and_opaque_text() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let cases: [(Box<dyn std::error::Error>, &str); 3] = [
                (
                    Box::new(ios_core::CoreError::Other(
                        "permission denied timeout cancelled".into(),
                    )),
                    "unknown",
                ),
                (Box::new(ios_core::OperationCancelled::new()), "cancelled"),
                (
                    Box::new(std::io::Error::from(std::io::ErrorKind::Interrupted)),
                    "io",
                ),
            ];
            for (raw, class) in cases {
                let error = operation_error(py, "start tunnel", raw.as_ref());
                assert!(error.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py));
                assert_eq!(
                    metadata(py, &error),
                    (1, class.into(), "start tunnel".into(), "unknown".into())
                );
            }
        });
    }

    #[test]
    fn metadata_assignment_failure_returns_an_exception_with_safe_original_cause() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let module = PyModule::from_code_bound(py, "class RejectMetadata(RuntimeError):\n    def __setattr__(self, name, value):\n        raise MemoryError('metadata allocation failed')\nerror = RejectMetadata('safe original failure')\n", "metadata_fixture.py", "metadata_fixture").unwrap();
            let original = PyErr::from_value_bound(module.getattr("error").unwrap());
            let error = exception_with_metadata(
                py,
                original,
                ios_core::ErrorClass::Unknown,
                "list devices",
                "unknown",
            );
            assert!(error.is_instance_of::<pyo3::exceptions::PyMemoryError>(py));
            let cause = error.cause(py).unwrap();
            assert_eq!(
                cause.value_bound(py).str().unwrap().to_string(),
                "safe original failure"
            );
        });
    }

    #[test]
    fn python_exception_hides_protocol_data_and_invalid_argument_values() {
        pyo3::prepare_freethreaded_python();
        let secret = "synthetic-private-key-UDID-token-device-data";
        let raw = ios_core::CoreError::Protocol(secret.into());
        assert!(raw.to_string().contains(secret));
        Python::with_gil(|py| {
            let error = operation_error(py, "start tunnel", &raw);
            assert!(error.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py));
            let rendered = error.value_bound(py).str().unwrap().to_string();
            assert!(rendered.contains("start tunnel failed: protocol error:"));
            assert!(!format!("{rendered} {error:?}").contains(secret));
            let error = parse_tunnel_mode(secret).err().unwrap();
            assert!(error.is_instance_of::<PyValueError>(py));
            assert!(!format!("{error} {error:?}").contains(secret));
        });
    }

    #[test]
    fn python_repr_hides_address_but_connect_info_preserves_explicit_selector() {
        pyo3::prepare_freethreaded_python();
        let secret = "synthetic-private-tunnel-address";
        let tunnel = Tunnel {
            server_address: secret.into(),
            rsd_port: 1234,
            userspace_port: Some(49160),
            services: vec![],
            service_ports: BTreeMap::new(),
            service_features: BTreeMap::new(),
            _state: Arc::new(Mutex::new(TunnelState {
                device: None,
                checked: None,
                retained_patches: 0,
                close_requested: false,
            })),
        };
        assert!(!tunnel.__repr__().contains(secret));
        assert!(tunnel.__repr__().contains("id:"));
        Python::with_gil(|py| {
            let patch = tunnel.asyncio_proxy(py).unwrap();
            assert!(!patch.__repr__().contains(secret));
            let info = tunnel.connect_info(py).unwrap();
            assert_eq!(
                info.bind(py)
                    .get_item("server_address")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                secret
            );
        });
    }

    #[test]
    fn tunnel_mode_rejects_unknown_values_with_value_error() {
        pyo3::prepare_freethreaded_python();
        let error = parse_tunnel_mode("kernal").expect_err("unknown modes must be rejected");
        Python::with_gil(|py| {
            assert!(error.is_instance_of::<PyValueError>(py));
            assert!(error.to_string().contains("userspace"));
            assert!(error.to_string().contains("kernel"));
        });
    }

    #[test]
    fn tunnel_mode_accepts_documented_values() {
        assert_eq!(
            parse_tunnel_mode("userspace").unwrap(),
            ios_core::TunMode::Userspace
        );
        assert_eq!(
            parse_tunnel_mode("kernel").unwrap(),
            ios_core::TunMode::Kernel
        );
    }

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
                    checked: None,
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
                    checked: None,
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
                    checked: None,
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
                        checked: None,
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
                checked: None,
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

    #[test]
    fn open_device_rejects_empty_udid() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = open_device("", py).expect_err("empty udid should fail");
            let (version, class, phase, effects) = metadata(py, &err);
            assert_eq!(version, 1);
            assert_eq!(class, "invalid_input");
            assert_eq!(phase, "open device");
            assert_eq!(effects, "not_started");

            let err_spaces = open_device("   ", py).expect_err("whitespace udid should fail");
            let (_, class_spaces, _, _) = metadata(py, &err_spaces);
            assert_eq!(class_spaces, "invalid_input");
        });
    }

    #[test]
    fn closed_device_rejects_operations_with_precondition() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let device = Device {
                udid: "00008030-001234567890".into(),
                device: Arc::new(Mutex::new(None)),
            };
            assert!(device.closed());
            assert_eq!(
                device.__repr__(),
                format!(
                    "Device(udid='{}', closed=true)",
                    ios_core::redaction::diagnostic_id(&device.udid)
                )
            );

            let err = device.list_apps(py).expect_err("closed device should fail");
            let (v, c, phase, effects) = metadata(py, &err);
            assert_eq!(v, 1);
            assert_eq!(c, "precondition");
            assert_eq!(phase, "device access");
            assert_eq!(effects, "not_started");

            let err = device
                .list_files("/", py)
                .expect_err("closed device should fail");
            let (_, c, _, _) = metadata(py, &err);
            assert_eq!(c, "precondition");

            let err = device
                .read_file("/test", 100, py)
                .expect_err("closed device should fail");
            let (_, c, _, _) = metadata(py, &err);
            assert_eq!(c, "precondition");

            let err = device
                .read_syslog(10, 1000, py)
                .expect_err("closed device should fail");
            let (_, c, _, _) = metadata(py, &err);
            assert_eq!(c, "precondition");

            let err = device
                .product_version(py)
                .expect_err("closed device should fail");
            let (_, c, _, _) = metadata(py, &err);
            assert_eq!(c, "precondition");
        });
    }

    #[test]
    fn device_sync_and_async_context_manager_close() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            // Sync context manager
            let device = Device {
                udid: "00008030-001234567891".into(),
                device: Arc::new(Mutex::new(None)),
            };
            {
                let py_dev = Bound::new(py, device).expect("create Py");
                let _ = py_dev.call_method0("__enter__").expect("call __enter__");
                let _ = py_dev
                    .call_method1("__exit__", (py.None(), py.None(), py.None()))
                    .expect("call __exit__");
                assert!(py_dev.borrow().closed());
            }

            // Async context manager via python script
            let device2 = Device {
                udid: "00008030-001234567892".into(),
                device: Arc::new(Mutex::new(None)),
            };
            let py_dev2 = Bound::new(py, device2).expect("create Py");
            let locals = PyDict::new_bound(py);
            locals.set_item("dev", &py_dev2).unwrap();
            let code = r#"
import asyncio

async def test_acm(d):
    async with d as ctx:
        assert ctx is d
        assert ctx.closed

asyncio.run(test_acm(dev))
"#;
            py.run_bound(code, None, Some(&locals))
                .expect("run async context manager");
            assert!(py_dev2.borrow().closed());
        });
    }

    #[test]
    fn apps_to_pylist_and_files_to_pylist_format() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let app_info = ios_core::apps::AppInfo {
                bundle_id: "com.example.test".into(),
                display_name: "TestApp".into(),
                version: "1.0.0".into(),
                app_type: "User".into(),
                path: "/private/var/containers/Bundle/Application/uuid/TestApp.app".into(),
                extra: std::collections::HashMap::new(),
            };
            let py_list = apps_to_pylist(py, &[app_info]).expect("apps to pylist");
            let list_bound = py_list.bind(py).downcast::<PyList>().expect("is list");
            assert_eq!(list_bound.len(), 1);
            let item = list_bound
                .get_item(0)
                .unwrap()
                .downcast::<PyDict>()
                .unwrap()
                .clone();
            assert_eq!(
                item.get_item("bundle_id")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "com.example.test"
            );
            assert_eq!(
                item.get_item("display_name")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "TestApp"
            );
            assert_eq!(
                item.get_item("version")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "1.0.0"
            );
            assert_eq!(
                item.get_item("app_type")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "User"
            );

            let files = vec!["Photos".to_string(), "DCIM".to_string()];
            let py_files = files_to_pylist(py, &files).expect("files to pylist");
            let files_bound = py_files.bind(py).downcast::<PyList>().expect("is list");
            assert_eq!(files_bound.len(), 2);
            assert_eq!(
                files_bound
                    .get_item(0)
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "Photos"
            );
            assert_eq!(
                files_bound
                    .get_item(1)
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "DCIM"
            );
        });
    }
}
