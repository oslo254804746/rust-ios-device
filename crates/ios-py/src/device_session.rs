//! Three explicit existing-trust queries, with original-task cleanup receipts.
//!
//! The Python await adapter holds only a receipt. A supervisor uniquely awaits
//! the original native task and returns its binding admission slot before
//! publishing completion. Neither task owns a client/SessionOwner clone.

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use ios_core::session::{DeviceInfoFields, DeviceQuery, DeviceQueryLimits, DeviceQueryObservation,
    ExistingTrustDevice, OperationCancel, OperationOptions, SessionOwner,
    SharedAdmission, UsbmuxEndpoint};
use ios_core::{CoreError, ErrorClass, PairRecord};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyBytesMethods, PyDict, PyModule};

const MAX_PAIR_BYTES: usize = 1024 * 1024;
const MAX_REGISTRY: usize = 4;
const QUERY_PHASE: &str = "device query";
const MAX_OBSERVERS: usize = 128;
static OBSERVERS_AVAILABLE: AtomicUsize = AtomicUsize::new(MAX_OBSERVERS);

struct Admission {
    native: SharedAdmission,
    available: AtomicUsize,
    observers: AtomicUsize,
}

impl Admission {
    fn new(limit: usize) -> Result<Arc<Self>, CoreError> {
        Ok(Arc::new(Self { native: SharedAdmission::new(limit)?,
            available: AtomicUsize::new(limit), observers: AtomicUsize::new(limit * 2) }))
    }

    fn try_job(self: &Arc<Self>) -> Option<JobPermit> {
        self.try_permit(false)
    }
    fn try_observer(self: &Arc<Self>) -> Option<JobPermit> { self.try_permit(true) }
    fn try_permit(self: &Arc<Self>, observer: bool) -> Option<JobPermit> {
        let counter = if observer { &self.observers } else { &self.available };
        counter.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |remaining| remaining.checked_sub(1)).ok()?;
        if observer && OBSERVERS_AVAILABLE.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |remaining| remaining.checked_sub(1)).is_err() {
            counter.fetch_add(1, Ordering::Release);
            return None;
        }
        Some(JobPermit { admission: Arc::clone(self), observer })
    }
}

struct JobPermit { admission: Arc<Admission>, observer: bool }
impl Drop for JobPermit {
    fn drop(&mut self) {
        let counter = if self.observer { &self.admission.observers } else { &self.admission.available };
        counter.fetch_add(1, Ordering::Release);
        if self.observer { OBSERVERS_AVAILABLE.fetch_add(1, Ordering::Release); }
    }
}

// This is shared only by default DeviceSessions in this Python extension.
// Explicit pools have their own 1..64 limits; no process-wide C/Python claim.
static DEFAULT_ADMISSION: LazyLock<Result<Arc<Admission>, CoreError>> =
    LazyLock::new(|| Admission::new(64));

#[pyclass(module = "ios_rs", name = "DeviceAdmission")]
struct DeviceAdmission { inner: Arc<Admission> }

#[pymethods]
impl DeviceAdmission {
    #[new]
    #[pyo3(signature = (limit = 64))]
    fn new(py: Python<'_>, limit: usize) -> PyResult<Self> {
        py.allow_threads(|| Admission::new(limit)).map(|inner| Self { inner })
            .map_err(|e| failure(py, &Failure::Core(e), "device admission", "not_started"))
    }

    #[getter]
    fn available(&self) -> usize { self.inner.native.available() }

    /// Slots for original native tasks whose JoinHandle has not been consumed.
    #[getter]
    fn job_available(&self) -> usize { self.inner.available.load(Ordering::Acquire) }
    /// Per-pool observation slots, additionally subject to a global 128 cap.
    #[getter]
    fn observer_available(&self) -> usize { self.inner.observers.load(Ordering::Acquire) }
}

#[pyclass(module = "ios_rs", name = "DeviceCancel")]
struct DeviceCancel { inner: OperationCancel }

#[pymethods]
impl DeviceCancel {
    #[new]
    fn new() -> Self { Self { inner: OperationCancel::new() } }
    fn cancel(&self) { self.inner.cancel(); }
    #[getter]
    fn cancelled(&self) -> bool { self.inner.is_cancelled() }
}

enum Failure {
    Core(CoreError),
    Io(std::io::Error),
    Join(tokio::task::JoinError),
    Runtime(&'static std::io::Error),
    Local(ErrorClass, &'static str),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Core(_) => "native device operation failed",
            Self::Io(_) => "device input or cleanup failed",
            Self::Join(_) => "native device task failed",
            Self::Runtime(_) => "Python runtime initialization failed",
            Self::Local(_, message) => message,
        })
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Display::fmt(self, f) }
}
impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Core(e) => Some(e), Self::Io(e) => Some(e), Self::Join(e) => Some(e),
            Self::Runtime(e) => Some(*e),
            Self::Local(_, _) => None,
        }
    }
}

fn failure(py: Python<'_>, error: &Failure, phase: &'static str,
    effects: &'static str) -> PyErr {
    let class = match error {
        Failure::Local(class, _) => *class,
        _ => ios_core::classify_error(error),
    };
    let message = format!("{phase} failed: {}", ios_core::redaction::diagnostic_error(error));
    let exception = if class == ErrorClass::InvalidInput {
        PyValueError::new_err(message)
    } else { PyRuntimeError::new_err(message) };
    super::exception_with_metadata(py, exception, class, phase, effects)
}

fn duration(seconds: f64) -> Result<Duration, Failure> {
    Duration::try_from_secs_f64(seconds).map_err(|_| Failure::Local(
        ErrorClass::InvalidInput, "invalid finite time budget"))
}

fn deadline(seconds: f64) -> Result<Instant, Failure> {
    Instant::now().checked_add(duration(seconds)?).ok_or(Failure::Local(
        ErrorClass::InvalidInput, "unrepresentable cleanup budget"))
}

type Outcome = Arc<Result<DeviceQueryObservation, Failure>>;

struct Job {
    admission: Arc<Admission>,
    field: &'static str,
    cancel: OperationCancel,
    result: Mutex<Option<Outcome>>,
    changed: Condvar,
}

impl Job {
    fn result_lock(&self) -> std::sync::MutexGuard<'_, Option<Outcome>> {
        self.result.lock().unwrap_or_else(|error| error.into_inner())
    }
    fn done(&self) -> bool { self.result_lock().is_some() }
    fn publish(&self, outcome: Result<DeviceQueryObservation, Failure>) {
        *self.result_lock() = Some(Arc::new(outcome));
        self.changed.notify_all();
    }
    fn wait(&self, until: Option<Instant>) -> Result<Outcome, Failure> {
        let mut state = self.result_lock();
        loop {
            if let Some(result) = state.as_ref() { return Ok(Arc::clone(result)); }
            state = if let Some(until) = until {
                let remaining = until.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(Failure::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut, "device cleanup deadline exceeded")));
                }
                let (next, _) = self.changed.wait_timeout(state, remaining)
                    .unwrap_or_else(|error| error.into_inner());
                next
            } else { self.changed.wait(state).unwrap_or_else(|error| error.into_inner()) };
        }
    }
}

// Before launch, Python allocation/refusal can drop the real unpolled lease.
// The registry also sees this reservation, so a concurrent close cancels it.
struct PendingJob {
    job: Arc<Job>,
    prepared: Option<ios_core::session::PreparedDeviceQuery>,
    permit: Option<JobPermit>,
    runtime: &'static tokio::runtime::Runtime,
}

impl PendingJob {
    fn launch(mut self) -> Arc<Job> {
        let prepared = self.prepared.take().expect("pending query owns its lease");
        let permit = self.permit.take().expect("pending query owns its job slot");
        let native = self.runtime.spawn(prepared);
        let supervised = Arc::clone(&self.job);
        // Only this task consumes the original JoinHandle. No client/owner clone
        // enters either task, and cancellation never aborts or detaches native.
        drop(self.runtime.spawn(async move {
            let outcome = match native.await {
                Ok(result) => result.map_err(Failure::Core),
                Err(error) => Err(Failure::Join(error)),
            };
            drop(permit);
            supervised.publish(outcome);
        }));
        Arc::clone(&self.job)
    }
}

impl Drop for PendingJob {
    fn drop(&mut self) {
        if let Some(prepared) = self.prepared.take() {
            self.job.cancel.cancel();
            drop(prepared); // No transport was polled; actual lease is released.
            drop(self.permit.take());
            self.job.publish(Err(Failure::Local(ErrorClass::Cancelled,
                "device query initialization was cancelled")));
        }
    }
}

/// An owned receipt survives session Drop or a cancellation-cleanup timeout.
#[pyclass(module = "ios_rs", name = "DeviceQueryReceipt")]
struct DeviceQueryReceipt { job: Arc<Job> }

// This guard belongs exclusively to the observer coroutine. It is not
// registered on the module or exposed through the user's Future/receipt.
#[pyclass(module = "ios_rs", name = "_DeviceObserverGuard")]
struct ObserverGuard { permit: Mutex<Option<JobPermit>> }

#[pymethods]
impl ObserverGuard {
    fn _observer_finished(&self) {
        self.permit.lock().unwrap_or_else(|error| error.into_inner()).take();
    }
}

impl Drop for DeviceQueryReceipt {
    fn drop(&mut self) { self.job.cancel.cancel(); }
}

#[pymethods]
impl DeviceQueryReceipt {
    #[getter]
    fn done(&self) -> bool { self.job.done() }
    fn cancel(&self) { self.job.cancel.cancel(); }

    /// Wait for the original native JoinHandle, releasing the GIL throughout.
    #[pyo3(signature = (cleanup_timeout = 5.0))]
    fn result(&self, py: Python<'_>, cleanup_timeout: f64) -> PyResult<PyObject> {
        let until = deadline(cleanup_timeout)
            .map_err(|e| failure(py, &e, "device query receipt", "not_started"))?;
        let outcome = py.allow_threads(|| self.job.wait(Some(until)))
            .map_err(|e| failure(py, &e, "device query receipt", "unknown"))?;
        observation(py, self.job.field, outcome.as_ref())
    }

    #[pyo3(signature = (cleanup_timeout = 5.0))]
    fn wait_async(&self, py: Python<'_>, cleanup_timeout: f64) -> PyResult<PyObject> {
        deadline(cleanup_timeout)
            .map_err(|e| failure(py, &e, "device query receipt", "not_started"))?;
        let helper = helpers(py)?;
        helper.getattr("require_loop")?.call0()?;
        let observer = observer_guard(py, &self.job.admission)?;
        let receipt = Py::new(py, Self { job: Arc::clone(&self.job) })?;
        helper.getattr("start_receipt")?.call1((receipt, cleanup_timeout, observer)).map(Into::into)
    }
}

#[pyclass(module = "ios_rs", name = "_DeviceCloseReceipt")]
struct CloseReceipt { jobs: Vec<Arc<Job>> }

impl CloseReceipt {
    fn wait_until(&self, until: Instant) -> Result<(), Failure> {
        for job in &self.jobs { let _ = job.wait(Some(until))?; }
        Ok(())
    }
}

#[pymethods]
impl CloseReceipt {
    #[getter]
    fn done(&self) -> bool { self.jobs.iter().all(|job| job.done()) }

    #[pyo3(signature = (cleanup_timeout = 5.0))]
    fn wait(&self, py: Python<'_>, cleanup_timeout: f64) -> PyResult<()> {
        let until = deadline(cleanup_timeout)
            .map_err(|e| failure(py, &e, "device close receipt", "not_started"))?;
        py.allow_threads(|| self.wait_until(until))
            .map_err(|e| failure(py, &e, "device close", "unknown"))
    }
}

#[pyclass(module = "ios_rs", name = "DeviceSession")]
struct DeviceSession {
    device: ExistingTrustDevice,
    admission: Arc<Admission>,
    jobs: Mutex<Vec<Arc<Job>>>,
}

impl DeviceSession {
    fn jobs(&self) -> std::sync::MutexGuard<'_, Vec<Arc<Job>>> {
        self.jobs.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn prepare(&self, field: &str, timeout: f64, max_reply_wire_bytes: usize,
        max_output_utf8_bytes: usize, cancel: OperationCancel) -> Result<PendingJob, Failure> {
        let (query, field) = selector(field)?;
        let total = duration(timeout)?;
        let limits = DeviceQueryLimits::new(max_reply_wire_bytes, max_output_utf8_bytes)
            .map_err(Failure::Core)?;
        let runtime = super::RUNTIME.as_ref().map_err(Failure::Runtime)?;
        let mut jobs = self.jobs();
        jobs.retain(|job| !job.done());
        if self.device.is_closed() {
            return Err(Failure::Local(ErrorClass::Precondition, "device session is closed"));
        }
        if cancel.is_cancelled() {
            return Err(Failure::Io(std::io::Error::other(ios_core::OperationCancelled::new())));
        }
        if jobs.len() == MAX_REGISTRY {
            return Err(Failure::Local(ErrorClass::Conflict, "device receipt registry is full"));
        }
        let permit = self.admission.try_job().ok_or(Failure::Local(
            ErrorClass::ResourceLimit, "shared native-task admission is full"))?;
        let prepared = self.device.prepare_query(query, limits, OperationOptions::new(total),
            cancel.clone()).map_err(Failure::Core)?;
        let job = Arc::new(Job { admission: Arc::clone(&self.admission), field, cancel,
            result: Mutex::new(None), changed: Condvar::new() });
        jobs.push(Arc::clone(&job));
        Ok(PendingJob { job, prepared: Some(prepared), permit: Some(permit), runtime })
    }

    fn seal(&self) -> CloseReceipt {
        let jobs = self.jobs();
        self.device.request_close();
        for job in jobs.iter() { job.cancel.cancel(); }
        CloseReceipt { jobs: jobs.clone() }
    }
}

impl Drop for DeviceSession {
    fn drop(&mut self) {
        let jobs = self.jobs();
        self.device.request_close();
        for job in jobs.iter() { job.cancel.cancel(); }
    }
}

#[pymethods]
impl DeviceSession {
    /// Credentials are immutable bytes supplied by the caller, never a filename.
    #[new]
    #[pyo3(signature = (udid, pair_record_bytes, endpoint, *, concurrency = 4, admission = None))]
    fn new(py: Python<'_>, udid: &str, pair_record_bytes: &Bound<'_, PyBytes>,
        endpoint: &str, concurrency: usize, admission: Option<PyRef<'_, DeviceAdmission>>)
        -> PyResult<Self> {
        let bytes = pair_record_bytes.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_PAIR_BYTES {
            return Err(failure(py, &Failure::Local(ErrorClass::InvalidInput,
                "pair record byte length must be 1..1048576"), "device session input", "not_started"));
        }
        if udid.is_empty() || udid.len() > 4096 || udid.as_bytes().contains(&0)
            || endpoint.is_empty() || endpoint.len() > 4096 || endpoint.as_bytes().contains(&0) {
            return Err(failure(py, &Failure::Local(ErrorClass::InvalidInput,
                "invalid explicit device route"), "device session input", "not_started"));
        }
        let pool = admission.map(|pool| Arc::clone(&pool.inner));
        py.allow_threads(|| {
            let pool = match pool {
                Some(pool) => pool,
                None => DEFAULT_ADMISSION.as_ref().map(Arc::clone)
                    .map_err(|error| Failure::Local(error.class(), "default admission unavailable"))?,
            };
            let endpoint = endpoint_parse(endpoint)?;
            let owner = SessionOwner::new(pool.native.clone(), concurrency).map_err(Failure::Core)?;
            let pair = PairRecord::from_bytes(bytes).map_err(Failure::Io)?;
            let device = ExistingTrustDevice::new(udid.to_owned(), Arc::new(pair), owner, endpoint)
                .map_err(Failure::Core)?;
            Ok(Self { device, admission: pool, jobs: Mutex::new(Vec::new()) })
        }).map_err(|e| failure(py, &e, "device session input", "not_started"))
    }

    #[getter]
    fn closed(&self) -> bool { self.device.is_closed() }
    #[getter]
    fn active(&self) -> usize { self.device.active() }
    #[getter]
    fn pending_cleanup(&self) -> usize { self.jobs().iter().filter(|job| !job.done()).count() }

    #[pyo3(signature = (field, *, timeout = 15.0, max_reply_wire_bytes = 1048576,
        max_output_utf8_bytes = 4096, cancel = None))]
    fn query(&self, py: Python<'_>, field: &str, timeout: f64,
        max_reply_wire_bytes: usize, max_output_utf8_bytes: usize,
        cancel: Option<PyRef<'_, DeviceCancel>>) -> PyResult<PyObject> {
        let cancel = cancel.map(|token| token.inner.clone()).unwrap_or_default();
        let pending = py.allow_threads(|| self.prepare(field, timeout, max_reply_wire_bytes,
            max_output_utf8_bytes, cancel))
            .map_err(|e| failure(py, &e, QUERY_PHASE, "not_started"))?;
        let job = py.allow_threads(|| pending.launch());
        let outcome = py.allow_threads(|| job.wait(None))
            .map_err(|e| failure(py, &e, QUERY_PHASE, "unknown"))?;
        observation(py, job.field, outcome.as_ref())
    }

    /// Reserve/start immediately; awaiting drives observation, never transport.
    #[pyo3(signature = (field, *, timeout = 15.0, max_reply_wire_bytes = 1048576,
        max_output_utf8_bytes = 4096, cancel = None, cleanup_timeout = 5.0))]
    fn query_async(&self, py: Python<'_>, field: &str, timeout: f64,
        max_reply_wire_bytes: usize, max_output_utf8_bytes: usize,
        cancel: Option<PyRef<'_, DeviceCancel>>, cleanup_timeout: f64) -> PyResult<PyObject> {
        deadline(cleanup_timeout)
            .map_err(|e| failure(py, &e, QUERY_PHASE, "not_started"))?;
        // Allocate helper code before admitting the native task.
        let helper = helpers(py)?;
        helper.getattr("require_loop")?.call0()?;
        let observer = observer_guard(py, &self.admission)?;
        let cancel = cancel.map(|token| token.inner.clone()).unwrap_or_default();
        let pending = py.allow_threads(|| self.prepare(field, timeout, max_reply_wire_bytes,
            max_output_utf8_bytes, cancel))
            .map_err(|e| failure(py, &e, QUERY_PHASE, "not_started"))?;
        let receipt = Py::new(py, DeviceQueryReceipt { job: Arc::clone(&pending.job) })?;
        let future = helper.getattr("start_query")?.call1((receipt, cleanup_timeout, observer))?;
        // Allocate the Future and bounded observer before polling native IO.
        py.allow_threads(|| pending.launch());
        Ok(future.into())
    }

    #[pyo3(signature = (cleanup_timeout = 5.0))]
    fn close(&self, py: Python<'_>, cleanup_timeout: f64) -> PyResult<()> {
        let until = deadline(cleanup_timeout)
            .map_err(|e| failure(py, &e, "device close", "not_started"))?;
        let receipt = py.allow_threads(|| self.seal());
        py.allow_threads(|| receipt.wait_until(until))
            .map_err(|e| failure(py, &e, "device close", "unknown"))
    }

    #[pyo3(signature = (cleanup_timeout = 5.0))]
    fn aclose(&self, py: Python<'_>, cleanup_timeout: f64) -> PyResult<PyObject> {
        deadline(cleanup_timeout)
            .map_err(|e| failure(py, &e, "device close", "not_started"))?;
        let helper = helpers(py)?;
        helper.getattr("require_loop")?.call0()?;
        let receipt = py.allow_threads(|| self.seal());
        let observer = observer_guard(py, &self.admission)?;
        let receipt = Py::new(py, receipt)?;
        helper.getattr("start_close")?.call1((receipt, cleanup_timeout, false, py.None(), observer)).map(Into::into)
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        if slf.device.is_closed() {
            return Err(failure(slf.py(), &Failure::Local(ErrorClass::Precondition,
                "device session is closed"), "device context", "not_started"));
        }
        Ok(slf)
    }
    fn __exit__(&self, py: Python<'_>, _kind: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>, _traceback: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Err(error) = self.close(py, 5.0) {
            if !value.is_none() {
                // Keep the caller's original exception object, without rendering it.
                if let Err(metadata) = error.value_bound(py).setattr("__context__", value) {
                    metadata.set_cause(py, Some(error));
                    return Err(metadata);
                }
            }
            return Err(error);
        }
        Ok(false)
    }
    fn __aenter__(slf: PyRef<'_, Self>) -> PyResult<PyObject> {
        let py = slf.py();
        if slf.device.is_closed() {
            return Err(failure(py, &Failure::Local(ErrorClass::Precondition,
                "device session is closed"), "device context", "not_started"));
        }
        helpers(py)?.getattr("enter_session")?.call1((slf.into_py(py),)).map(Into::into)
    }
    fn __aexit__(&self, py: Python<'_>, _kind: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>, _traceback: &Bound<'_, PyAny>) -> PyResult<PyObject> {
        let helper = helpers(py)?;
        helper.getattr("require_loop")?.call0()?;
        let receipt = py.allow_threads(|| self.seal());
        let observer = observer_guard(py, &self.admission)?;
        let receipt = Py::new(py, receipt)?;
        helper.getattr("start_close")?.call1((receipt, 5.0, true, value, observer)).map(Into::into)
    }
}

fn selector(field: &str) -> Result<(DeviceQuery, &'static str), Failure> {
    match field {
        "ProductType" => Ok((DeviceQuery::product_type(), "ProductType")),
        "ProductVersion" => Ok((DeviceQuery::product_version(), "ProductVersion")),
        "DeviceName" => Ok((DeviceQuery::device_name(), "DeviceName")),
        "root" => DeviceQuery::root(DeviceInfoFields::all())
            .map(|query| (query, "root")).map_err(Failure::Core),
        _ => Err(Failure::Local(ErrorClass::InvalidInput, "unsupported finite device field")),
    }
}

fn observer_permit(py: Python<'_>, admission: &Arc<Admission>) -> PyResult<JobPermit> {
    admission.try_observer().ok_or_else(|| failure(py, &Failure::Local(
        ErrorClass::ResourceLimit, "shared asyncio-observer admission is full"),
        "device observer", "not_started"))
}

fn observer_guard(py: Python<'_>, admission: &Arc<Admission>) -> PyResult<Py<ObserverGuard>> {
    let permit = observer_permit(py, admission)?;
    Py::new(py, ObserverGuard { permit: Mutex::new(Some(permit)) })
}

fn endpoint_parse(endpoint: &str) -> Result<UsbmuxEndpoint, Failure> {
    if let Some(address) = endpoint.strip_prefix("tcp://") {
        let address = address.parse::<SocketAddr>().map_err(|e| Failure::Io(
            std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;
        if address.port() == 0 {
            return Err(Failure::Local(ErrorClass::InvalidInput, "usbmux TCP port must be nonzero"));
        }
        return Ok(UsbmuxEndpoint::tcp(address));
    }
    #[cfg(unix)]
    if let Some(path) = endpoint.strip_prefix("unix://") {
        return UsbmuxEndpoint::unix(path.into()).map_err(Failure::Core);
    }
    Err(Failure::Local(ErrorClass::InvalidInput, "expected explicit numeric tcp:// or unix:// endpoint"))
}

fn observation(py: Python<'_>, field: &str,
    outcome: &Result<DeviceQueryObservation, Failure>) -> PyResult<PyObject> {
    let output = outcome.as_ref().map_err(|e| failure(py, e, QUERY_PHASE, "unknown"))?;
    let dict = PyDict::new_bound(py);
    let metadata = PyDict::new_bound(py);
    metadata.set_item("device_id", output.metadata().device_id())?;
    metadata.set_item("connection_type", output.metadata().connection_type())?;
    metadata.set_item("product_id", output.metadata().product_id())?;
    dict.set_item("metadata", metadata)?;
    dict.set_item("field", field)?;
    if field == "root" && output.value_present() {
        let values = PyDict::new_bound(py);
        values.set_item("ProductType", output.product_type())?;
        values.set_item("ProductVersion", output.product_version())?;
        values.set_item("DeviceName", output.device_name())?;
        dict.set_item("value", values)?;
    } else {
        dict.set_item("value", match field {
            "ProductType" => output.product_type(), "ProductVersion" => output.product_version(),
            "DeviceName" => output.device_name(), _ => None,
        })?;
    }
    dict.set_item("value_present", output.value_present())?;
    dict.set_item("enable_session_ssl", output.enable_session_ssl())?;
    Ok(dict.into())
}

fn helpers(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    PyModule::from_code_bound(py, ASYNC_HELPERS, "ios_rs_device_async.py", "_ios_rs_device_async")
}

pub(super) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<DeviceAdmission>()?;
    module.add_class::<DeviceCancel>()?;
    module.add_class::<DeviceSession>()?;
    module.add_class::<DeviceQueryReceipt>()?;
    module.add_class::<CloseReceipt>()?;
    Ok(())
}

const ASYNC_HELPERS: &str = r#"
import asyncio
import weakref

def require_loop():
    return asyncio.get_running_loop()

def metadata(error, category, phase, complete, receipt):
    try:
        error.ios_error_schema_version = 1
        error.ios_error_class = category
        error.ios_error_phase = phase
        error.ios_error_effects = "unknown"
        error.ios_cleanup_complete = complete
        error.query_receipt = receipt
        error.ios_query_receipt = receipt
    except BaseException as metadata_error:
        raise metadata_error from error
    return error

class NativeFuture(asyncio.Future):
    def __init__(self, receipt, state):
        super().__init__()
        self.receipt = receipt
        self.state = state

    def cancel(self, msg=None):
        if self.done():
            return False
        if self.state["query"]:
            self.receipt.cancel()
        if not self.state["cancel_requested"]:
            self.state["cancel_requested"] = True
            self.state["cancel_message"] = msg
            if self.state["query"]:
                candidate = self.get_loop().time() + self.state["cleanup"]
                previous = self.state["until"]
                self.state["until"] = candidate if previous is None else min(previous, candidate)
        # Task.cancel propagates here. Keep pending until the ORIGINAL native
        # task has joined, or deliver a distinct cleanup-timeout receipt.
        return True

    def _annotate_cancel(self, error):
        phase = "device query" if self.state["query"] else "device close"
        metadata(error, "cancelled", phase, self.receipt.done, self.receipt)
        error.__cause__ = self.state.get("native_cancel_cause")
        return error

    def result(self):
        try:
            return super().result()
        except asyncio.CancelledError as error:
            self._annotate_cancel(error)
            raise

    def __await__(self):
        try:
            return (yield from super().__await__())
        except asyncio.CancelledError as error:
            self._annotate_cancel(error)
            raise

    __iter__ = __await__

    def __del__(self):
        if not self.done() and self.state["query"]:
            self.receipt.cancel()
        super().__del__()

async def observe(target, receipt, state, observer_guard):
    loop = asyncio.get_running_loop()
    try:
        while True:
            future = target()
            if future is None:
                # The Rust supervisor still owns the original JoinHandle.
                if state["query"]:
                    receipt.cancel()
                return
            if receipt.done:
                try:
                    output = receipt.result(cleanup_timeout=0.0) if state["query"] else receipt.wait(cleanup_timeout=0.0)
                except BaseException as error:
                    if state["cancel_requested"] and getattr(error, "ios_error_class", None) == "cancelled":
                        state["native_cancel_cause"] = error
                        asyncio.Future.cancel(future, state["cancel_message"])
                    else:
                        preserve_context(error, state)
                        future.set_exception(error)
                    return
                if state["cancel_requested"]:
                    asyncio.Future.cancel(future, state["cancel_message"])
                else:
                    future.set_result(False if state["exit"] else output)
                return
            if state["until"] is not None and loop.time() >= state["until"]:
                phase = ("device query cancellation cleanup" if state["cancel_requested"]
                         else "device query receipt") if state["query"] else "device close"
                error = metadata(TimeoutError(phase + " timed out"),
                    "timeout", phase, False, receipt)
                preserve_context(error, state)
                future.set_exception(error)
                return
            del future  # Observer does not retain a dropped user awaitable.
            await asyncio.sleep(0.005)
    except BaseException as error:
        future = target()
        if future is not None and not future.done():
            future.set_exception(error)
    finally:
        if state["query"] and not receipt.done:
            receipt.cancel()
        observer_guard._observer_finished()

def preserve_context(error, state):
    original = state["original_error"]
    if original is not None:
        error.__context__ = original

def start(receipt, cleanup_timeout, query, exit_context=False, original_error=None,
          bounded_wait=False, observer_guard=None):
    loop = asyncio.get_running_loop()
    state = {"query": query, "exit": exit_context, "cleanup": cleanup_timeout,
        "original_error": original_error,
        "cancel_requested": False, "cancel_message": None,
        "until": None if query and not bounded_wait else loop.time() + cleanup_timeout}
    # Adapter unit fixtures supply their own guard; public native entry points
    # always pass an internal Rust guard, separate from the user receipt.
    if observer_guard is None:
        observer_guard = receipt
    observer = None
    try:
        future = NativeFuture(receipt, state)
        observer = observe(weakref.ref(future), receipt, state, observer_guard)
        loop.create_task(observer)
    except BaseException:
        if observer is not None:
            observer.close()
        if query:
            receipt.cancel()
        observer_guard._observer_finished()
        raise
    return future

def start_query(receipt, cleanup_timeout, observer_guard=None):
    return start(receipt, cleanup_timeout, True, observer_guard=observer_guard)

def start_receipt(receipt, cleanup_timeout, observer_guard=None):
    return start(receipt, cleanup_timeout, True, bounded_wait=True, observer_guard=observer_guard)

def start_close(receipt, cleanup_timeout, exit_context, original_error=None, observer_guard=None):
    return start(receipt, cleanup_timeout, False, exit_context, original_error,
                 observer_guard=observer_guard)

async def enter_session(session):
    return session

"#;

#[cfg(test)]
mod tests;
