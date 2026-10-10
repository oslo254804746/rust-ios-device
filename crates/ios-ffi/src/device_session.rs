//! Finite existing-trust C queries. No transport, loader or generic key escapes.
use super::*;
use ios_core::session::{
    DeviceInfoFields, DeviceQuery, DeviceQueryLimits, DeviceQueryObservation, ExistingTrustDevice,
    OperationCancel, OperationOptions, SessionOwner, SharedAdmission, UsbmuxEndpoint,
};
use std::io::{self, Write};
use std::panic::AssertUnwindSafe;
use std::time::Duration;
use tokio::time::Instant;

#[cfg(test)]
mod tests;

const MAX_PAIR_BYTES: usize = 1024 * 1024;
const MAX_INPUT_TEXT: usize = 4096;
const MAX_DURATION_MS: u64 = 86_400_000;
const MAX_JSON_BYTES: usize = 6 * 16 * 1024 * 1024 + 1024;
const FIELDS: u32 = 7;
static ADMISSION: LazyLock<SharedAdmission> =
    LazyLock::new(|| SharedAdmission::new(64).expect("fixed finite FFI admission"));

/// Immutable borrowed buffers are copied/decoded during create only.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IosDeviceSessionConfig {
    pub udid: *const c_char,
    pub udid_len: usize,
    pub endpoint: *const c_char,
    pub endpoint_len: usize,
    pub pair_record: *const u8,
    pub pair_record_len: usize,
    pub concurrent_operations: u32,
}

/// Logical native reply, retained UTF-8 and serialized C-output limits.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IosDeviceQueryOptions {
    pub total_ms: u64,
    pub reply_wire_bytes: usize,
    pub output_utf8_bytes: usize,
    pub mux_frame_bytes: usize,
    pub lockdown_body_bytes: usize,
    pub metadata_entries: usize,
    pub metadata_utf8_bytes: usize,
    pub json_bytes: usize,
}

impl Default for IosDeviceQueryOptions {
    fn default() -> Self {
        Self {
            total_ms: 5_000,
            reply_wire_bytes: 1024 * 1024,
            output_utf8_bytes: 64 * 1024,
            mux_frame_bytes: 1024 * 1024,
            lockdown_body_bytes: 1024 * 1024,
            metadata_entries: 1024,
            metadata_utf8_bytes: 1024 * 1024,
            json_bytes: 1024 * 1024,
        }
    }
}

pub(super) struct Session {
    pub(super) device: ExistingTrustDevice,
}

/// Each allocation is a distinct C owner; retaining does not clone the device.
#[repr(C)]
pub struct IosDeviceSession {
    pub(super) state: Arc<Session>,
}

/// A distinct C owner of the actual idempotent Core cancellation token.
#[repr(C)]
pub struct IosOperationCancel {
    pub(super) cancel: OperationCancel,
}

pub(super) fn decision(class: ErrorClass) -> c_int {
    LAST_ERROR_CLASS.with(|state| state.set(Some(class)));
    IOS_ERR_CORE
}

pub(super) fn duration(milliseconds: u64) -> Result<Duration, c_int> {
    if milliseconds > MAX_DURATION_MS {
        return Err(decision(ErrorClass::InvalidInput));
    }
    let duration = Duration::from_millis(milliseconds);
    if Instant::now().checked_add(duration).is_none() {
        return Err(decision(ErrorClass::InvalidInput));
    }
    Ok(duration)
}

unsafe fn input_text<'a>(pointer: *const c_char, length: usize) -> Result<&'a str, c_int> {
    if pointer.is_null() {
        return Err(IOS_ERR_NULL);
    }
    if length == 0 {
        return Err(decision(ErrorClass::InvalidInput));
    }
    if length > MAX_INPUT_TEXT {
        return Err(decision(ErrorClass::ResourceLimit));
    }
    let bytes = std::slice::from_raw_parts(pointer.cast::<u8>(), length);
    let text = std::str::from_utf8(bytes).map_err(|_| IOS_ERR_UTF8)?;
    if bytes.contains(&0) {
        return Err(decision(ErrorClass::InvalidInput));
    }
    Ok(text)
}

fn endpoint(text: &str) -> Result<UsbmuxEndpoint, c_int> {
    if let Some(address) = text.strip_prefix("tcp://") {
        let address: std::net::SocketAddr = address
            .parse()
            .map_err(|_| decision(ErrorClass::InvalidInput))?;
        if address.port() == 0 {
            return Err(decision(ErrorClass::InvalidInput));
        }
        return Ok(UsbmuxEndpoint::tcp(address));
    }
    if let Some(path) = text.strip_prefix("unix://") {
        #[cfg(unix)]
        {
            let path = std::path::PathBuf::from(path);
            if !path.is_absolute() {
                return Err(decision(ErrorClass::InvalidInput));
            }
            return UsbmuxEndpoint::unix(path).map_err(|error| error_status(IOS_ERR_CORE, &error));
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            return Err(decision(ErrorClass::Unsupported));
        }
    }
    Err(decision(ErrorClass::InvalidInput))
}

fn selector(fields: u32) -> Result<DeviceQuery, c_int> {
    if fields == 0 || fields & !FIELDS != 0 {
        return Err(decision(ErrorClass::InvalidInput));
    }
    match fields {
        1 => Ok(DeviceQuery::product_type()),
        2 => Ok(DeviceQuery::product_version()),
        4 => Ok(DeviceQuery::device_name()),
        _ => {
            let mut selected = DeviceInfoFields::new();
            if fields & 1 != 0 {
                selected = selected.with_product_type();
            }
            if fields & 2 != 0 {
                selected = selected.with_product_version();
            }
            if fields & 4 != 0 {
                selected = selected.with_device_name();
            }
            DeviceQuery::root(selected).map_err(|error| error_status(IOS_ERR_CORE, &error))
        }
    }
}

fn limits(options: IosDeviceQueryOptions) -> Result<(DeviceQueryLimits, Duration), c_int> {
    let total = duration(options.total_ms)?;
    if options.json_bytes == 0 || options.json_bytes > MAX_JSON_BYTES {
        return Err(decision(ErrorClass::InvalidInput));
    }
    DeviceQueryLimits::new(options.reply_wire_bytes, options.output_utf8_bytes)
        .and_then(|limits| limits.with_mux_frame_bytes(options.mux_frame_bytes))
        .and_then(|limits| limits.with_lockdown_body_bytes(options.lockdown_body_bytes))
        .and_then(|limits| {
            limits.with_metadata_limits(options.metadata_entries, options.metadata_utf8_bytes)
        })
        .map(|limits| (limits, total))
        .map_err(|error| error_status(IOS_ERR_CORE, &error))
}

pub(super) fn blocking_runtime() -> Result<&'static Runtime, c_int> {
    // Calling a blocking C query from within an entered Tokio runtime cannot
    // safely block_on that thread. Refuse before preparing/admitting any IO.
    if tokio::runtime::Handle::try_current().is_ok() {
        LAST_ERROR_CLASS.with(|state| state.set(Some(ErrorClass::Precondition)));
        return Err(IOS_ERR_RUNTIME);
    }
    runtime()
}

pub(super) fn checked_services(
    device: &ios_core::ConnectedDevice,
) -> Result<ExistingTrustDevice, ios_core::CoreError> {
    let owner = SessionOwner::new(ADMISSION.clone(), 4)?;
    device.existing_trust_device(owner, UsbmuxEndpoint::system()?)
}

pub(super) fn close_services(device: &ExistingTrustDevice, cleanup_ms: u64) -> Result<(), c_int> {
    let cleanup = duration(cleanup_ms)?;
    device.request_close();
    blocking_runtime()?
        .block_on(device.close(cleanup))
        .map_err(|error| error_status(IOS_ERR_CORE, &error))
}

/// # Safety
/// Output/config and each specified input buffer must be valid/non-overlapping.
#[no_mangle]
pub unsafe extern "C" fn ios_device_session_create(
    config: *const IosDeviceSessionConfig,
    out: *mut *mut IosDeviceSession,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        if !out.is_null() {
            *out = std::ptr::null_mut();
        }
        if out.is_null() || config.is_null() {
            return IOS_ERR_NULL;
        }
        let config = *config;
        if !(1..=4).contains(&config.concurrent_operations) {
            return decision(ErrorClass::InvalidInput);
        }
        if config.pair_record.is_null() {
            return IOS_ERR_NULL;
        }
        if config.pair_record_len == 0 {
            return decision(ErrorClass::InvalidInput);
        }
        if config.pair_record_len > MAX_PAIR_BYTES {
            return decision(ErrorClass::ResourceLimit);
        }
        let udid = match input_text(config.udid, config.udid_len) {
            Ok(value) => value,
            Err(code) => return code,
        };
        let route = match input_text(config.endpoint, config.endpoint_len).and_then(endpoint) {
            Ok(value) => value,
            Err(code) => return code,
        };
        let pair = match ios_core::PairRecord::from_bytes(std::slice::from_raw_parts(
            config.pair_record,
            config.pair_record_len,
        )) {
            Ok(pair) => pair,
            Err(error) => return error_status(IOS_ERR_CORE, &error),
        };
        let owner =
            match SessionOwner::new(ADMISSION.clone(), config.concurrent_operations as usize) {
                Ok(owner) => owner,
                Err(error) => return error_status(IOS_ERR_CORE, &error),
            };
        let device = match ExistingTrustDevice::new(udid.to_owned(), Arc::new(pair), owner, route) {
            Ok(device) => device,
            Err(error) => return error_status(IOS_ERR_CORE, &error),
        };
        *out = Box::into_raw(Box::new(IosDeviceSession {
            state: Arc::new(Session { device }),
        }));
        IOS_SUCCESS
    }))
}

/// # Safety
/// A live borrowed handle and distinct writable output are required.
#[no_mangle]
pub unsafe extern "C" fn ios_device_session_retain(
    session: *const IosDeviceSession,
    out: *mut *mut IosDeviceSession,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        if !out.is_null() {
            *out = std::ptr::null_mut();
        }
        if out.is_null() || session.is_null() {
            return IOS_ERR_NULL;
        }
        *out = Box::into_raw(Box::new(IosDeviceSession {
            state: Arc::clone(&(*session).state),
        }));
        IOS_SUCCESS
    }))
}

/// Seal all aliases and cancel; await actual lease release without consuming.
/// # Safety
/// Handle must remain live for the entire call; query/close may run concurrently.
#[no_mangle]
pub unsafe extern "C" fn ios_device_session_close(
    session: *const IosDeviceSession,
    cleanup_ms: u64,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        if session.is_null() {
            return IOS_ERR_NULL;
        }
        let cleanup = match duration(cleanup_ms) {
            Ok(value) => value,
            Err(code) => return code,
        };
        let session_ref = &*session;
        let device = &session_ref.state.device;
        device.request_close();
        let runtime = match blocking_runtime() {
            Ok(runtime) => runtime,
            Err(code) => return code,
        };
        match runtime.block_on(device.close(cleanup)) {
            Ok(()) => IOS_SUCCESS,
            Err(error) => error_status(IOS_ERR_CORE, &error),
        }
    }))
}

/// Consume one C allocation. Other retained aliases stay open. Last alias Drop
/// broadcasts close; destroy does not join another caller's OS thread.
/// # Safety
/// Owner storage holds NULL or an exclusively owned live allocation. No call may
/// be in progress or start on this allocation, and no retained raw alias may be used.
#[no_mangle]
pub unsafe extern "C" fn ios_device_session_destroy_checked(
    owner: *mut *mut IosDeviceSession,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        consume_pointer(owner, |pointer| {
            drop(Box::from_raw(pointer));
            IOS_SUCCESS
        })
    }))
}

/// # Safety
/// Writable options must not overlap any live handle/input allocation.
#[no_mangle]
pub unsafe extern "C" fn ios_device_query_options_default(
    out: *mut IosDeviceQueryOptions,
) -> c_int {
    ffi_status(|| {
        if out.is_null() {
            return IOS_ERR_NULL;
        }
        *out = IosDeviceQueryOptions::default();
        IOS_SUCCESS
    })
}

pub(super) struct JsonBuffer<'a> {
    pub(super) bytes: Vec<u8>,
    pub(super) limit: usize,
    pub(super) deadline: Instant,
    pub(super) cancel: &'a OperationCancel,
    pub(super) device: &'a ExistingTrustDevice,
    pub(super) failure: Option<ErrorClass>,
}

pub(super) fn output_fence(
    cancel: &OperationCancel,
    device: &ExistingTrustDevice,
    deadline: Instant,
) -> Result<(), ErrorClass> {
    if cancel.is_cancelled() || device.is_closed() {
        Err(ErrorClass::Cancelled)
    } else if Instant::now() >= deadline {
        Err(ErrorClass::Timeout)
    } else {
        Ok(())
    }
}

impl JsonBuffer<'_> {
    fn check(&mut self) -> io::Result<()> {
        if let Err(class) = output_fence(self.cancel, self.device, self.deadline) {
            self.failure = Some(class);
            return Err(io::Error::other("finite C query output fence"));
        }
        Ok(())
    }
}

impl Write for JsonBuffer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.check()?;
        if !self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_some_and(|length| length <= self.limit)
        {
            self.failure = Some(ErrorClass::ResourceLimit);
            return Err(io::Error::other("finite C JSON byte budget exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.check()
    }
}

fn project_json(
    observation: &DeviceQueryObservation,
    fields: u32,
    output: &mut JsonBuffer<'_>,
) -> Result<(), c_int> {
    let mut values = BTreeMap::new();
    if fields & 1 != 0 {
        values.insert("ProductType", observation.product_type());
    }
    if fields & 2 != 0 {
        values.insert("ProductVersion", observation.product_version());
    }
    if fields & 4 != 0 {
        values.insert("DeviceName", observation.device_name());
    }
    // Values borrow owned strings. No raw plist, unselected value or credential
    // is cloned into the C projection. Serialization is bounded before append.
    // Serialize borrowed standard values using the existing serde_json dependency
    // rather than adding a derive dependency or building a cloned JSON Value.
    macro_rules! raw {
        ($bytes:expr) => {
            output
                .write_all($bytes)
                .map_err(|_| decision(output.failure.unwrap_or(ErrorClass::Unknown)))?;
        };
    }
    macro_rules! json {
        ($value:expr) => {
            serde_json::to_writer(&mut *output, $value).map_err(|error| match output.failure {
                Some(class) => decision(class),
                None => error_status(IOS_ERR_CORE, &error),
            })?;
        };
    }
    raw!(b"{\"value_present\":");
    json!(&observation.value_present());
    raw!(b",\"requested_fields\":");
    json!(&fields);
    raw!(b",\"values\":");
    json!(&values);
    raw!(b",\"device_id\":");
    json!(&observation.metadata().device_id());
    raw!(b",\"connection_type\":");
    json!(&observation.metadata().connection_type());
    raw!(b",\"product_id\":");
    json!(&observation.metadata().product_id());
    raw!(b",\"enable_session_ssl\":");
    json!(&observation.enable_session_ssl());
    raw!(b"}");
    output
        .check()
        .map_err(|_| decision(output.failure.unwrap_or(ErrorClass::Unknown)))
}

/// # Safety
/// All borrowed handles/options stay live during this call; writable output must
/// not overlap them. Destroy/free must follow caller synchronization/OS-thread joins.
#[no_mangle]
pub unsafe extern "C" fn ios_device_session_query_json(
    session: *const IosDeviceSession,
    fields: u32,
    options: *const IosDeviceQueryOptions,
    cancel: *const IosOperationCancel,
    out: *mut *mut c_char,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        if !out.is_null() {
            *out = std::ptr::null_mut();
        }
        if session.is_null() || options.is_null() || cancel.is_null() || out.is_null() {
            return IOS_ERR_NULL;
        }
        let query = match selector(fields) {
            Ok(query) => query,
            Err(code) => return code,
        };
        let options = *options;
        let (limits, total) = match limits(options) {
            Ok(value) => value,
            Err(code) => return code,
        };
        let runtime = match blocking_runtime() {
            Ok(runtime) => runtime,
            Err(code) => return code,
        };
        let cancel = (*cancel).cancel.clone();
        let session_ref = &*session;
        let device = &session_ref.state.device;
        let deadline = match Instant::now().checked_add(total) {
            Some(deadline) => deadline,
            None => return decision(ErrorClass::InvalidInput),
        };
        let prepared =
            match device.prepare_query(query, limits, OperationOptions::new(total), cancel.clone())
            {
                Ok(prepared) => prepared,
                Err(error) => return error_status(IOS_ERR_CORE, &error),
            };
        if let Err(class) = output_fence(&cancel, device, deadline) {
            drop(prepared); // Actual immediate lease retirement before any poll/IO.
            return decision(class);
        }
        // The job contains its original lease/immutable input snapshots. Neither
        // the C Arc, ExistingTrustDevice nor SessionOwner is cloned into it.
        enum QueryError {
            Fence(ErrorClass),
            Native(ios_core::CoreError),
        }
        let observation = match runtime.block_on(async {
            // Cover time between the synchronous check and block_on's first poll.
            // PreparedDeviceQuery exposes no Core absolute deadline getter, so
            // this stricter parent clock remains separate, never called shared.
            if let Err(class) = output_fence(&cancel, device, deadline) {
                drop(prepared);
                return Err(QueryError::Fence(class));
            }
            prepared.await.map_err(QueryError::Native)
        }) {
            Ok(observation) => observation,
            Err(QueryError::Fence(class)) => return decision(class),
            Err(QueryError::Native(error)) => return error_status(IOS_ERR_CORE, &error),
        };
        let mut output = JsonBuffer {
            bytes: Vec::with_capacity(options.json_bytes.min(4096)),
            limit: options.json_bytes,
            deadline,
            cancel: &cancel,
            device,
            failure: None,
        };
        if let Err(code) = project_json(&observation, fields, &mut output) {
            return code;
        }
        let text = match CString::new(output.bytes) {
            Ok(text) => text,
            Err(_) => return decision(ErrorClass::Protocol),
        };
        // CString allocation is synchronous. Recheck before publishing its owner.
        if let Err(class) = output_fence(&cancel, device, deadline) {
            return decision(class);
        }
        *out = text.into_raw();
        IOS_SUCCESS
    }))
}

/// # Safety
/// Required writable output may not overlap any library allocation.
#[no_mangle]
pub unsafe extern "C" fn ios_operation_cancel_create(out: *mut *mut IosOperationCancel) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        if out.is_null() {
            return IOS_ERR_NULL;
        }
        *out = std::ptr::null_mut();
        *out = Box::into_raw(Box::new(IosOperationCancel {
            cancel: OperationCancel::new(),
        }));
        IOS_SUCCESS
    }))
}

/// # Safety
/// Live borrowed token and distinct writable output are required.
#[no_mangle]
pub unsafe extern "C" fn ios_operation_cancel_retain(
    cancel: *const IosOperationCancel,
    out: *mut *mut IosOperationCancel,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        if !out.is_null() {
            *out = std::ptr::null_mut();
        }
        if cancel.is_null() || out.is_null() {
            return IOS_ERR_NULL;
        }
        *out = Box::into_raw(Box::new(IosOperationCancel {
            cancel: (*cancel).cancel.clone(),
        }));
        IOS_SUCCESS
    }))
}

/// # Safety
/// Token remains live throughout the call. Concurrent queries use retained tokens.
#[no_mangle]
pub unsafe extern "C" fn ios_operation_cancel(cancel: *const IosOperationCancel) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        if cancel.is_null() {
            return IOS_ERR_NULL;
        }
        (*cancel).cancel.cancel();
        IOS_SUCCESS
    }))
}

/// Does not cancel: a prepared job owns its token snapshot until real Drop.
/// # Safety
/// Exclusive owner storage; all calls on this allocation have stopped/joined.
#[no_mangle]
pub unsafe extern "C" fn ios_operation_cancel_destroy_checked(
    owner: *mut *mut IosOperationCancel,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        consume_pointer(owner, |pointer| {
            drop(Box::from_raw(pointer));
            IOS_SUCCESS
        })
    }))
}
