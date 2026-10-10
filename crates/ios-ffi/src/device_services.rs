//! Bounded read-only services on the same C session owner as finite queries.
use super::device_session::{blocking_runtime, decision, duration, output_fence, JsonBuffer};
use super::*;
use ios_core::session::{
    DeviceServiceLimits, DeviceServiceObservation, DeviceServiceRequest, ExistingTrustDevice,
    OperationCancel, OperationOptions, ServiceStreamFailure,
};
use std::io::Write;
use std::panic::AssertUnwindSafe;
use tokio::time::Instant;

thread_local! {
    static PROJECTION_FAILURE: std::cell::Cell<Option<ErrorClass>> = const { std::cell::Cell::new(None) };
}
pub(super) fn reset_projection_failure() {
    PROJECTION_FAILURE.with(|state| state.set(None));
}

/// Secondary partial-result projection failure on this caller thread. Read
/// before another int-status call; this borrowed static string must not be freed.
#[no_mangle]
pub extern "C" fn ios_last_service_projection_error_class() -> *const c_char {
    PROJECTION_FAILURE.with(|state| match state.get() {
        None => c"none".as_ptr(),
        Some(ErrorClass::Timeout) => c"timeout".as_ptr(),
        Some(ErrorClass::ResourceLimit) => c"resource_limit".as_ptr(),
        Some(ErrorClass::Protocol) => c"protocol".as_ptr(),
        Some(_) => c"unknown".as_ptr(),
    })
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IosDeviceServiceOptions {
    pub total_ms: u64,
    pub reply_wire_bytes: usize,
    pub output_bytes: usize,
    pub max_entries: usize,
    pub max_frame_bytes: usize,
    pub json_bytes: usize,
}
impl Default for IosDeviceServiceOptions {
    fn default() -> Self {
        let limits = DeviceServiceLimits::default();
        Self {
            total_ms: 15_000,
            reply_wire_bytes: limits.reply_wire_bytes,
            output_bytes: limits.output_bytes,
            max_entries: limits.max_entries,
            max_frame_bytes: limits.max_frame_bytes,
            json_bytes: 8 * 1024 * 1024,
        }
    }
}
impl IosDeviceServiceOptions {
    fn limits(self) -> DeviceServiceLimits {
        DeviceServiceLimits {
            reply_wire_bytes: self.reply_wire_bytes,
            output_bytes: self.output_bytes,
            max_entries: self.max_entries,
            max_frame_bytes: self.max_frame_bytes,
        }
    }
}

pub(super) unsafe fn from_handle<'a>(
    handle: *const IosDeviceHandle,
) -> Result<&'a ExistingTrustDevice, c_int> {
    if handle.is_null() {
        return Err(IOS_ERR_NULL);
    }
    // Lifetime is borrowed for the complete synchronized C call. This internal
    // helper never stores the reference or launches a detached task.
    (*handle).services.as_deref().ok_or(IOS_ERR_STATE)
}

fn run(
    device: &ExistingTrustDevice,
    request: DeviceServiceRequest,
    options: IosDeviceServiceOptions,
    cancel: &OperationCancel,
) -> Result<
    (
        Result<DeviceServiceObservation, ios_core::CoreError>,
        Instant,
    ),
    c_int,
> {
    let total = duration(options.total_ms)?;
    if options.json_bytes == 0 || options.json_bytes > 100 * 1024 * 1024 {
        return Err(decision(ErrorClass::InvalidInput));
    }
    let runtime = blocking_runtime()?;
    let deadline = Instant::now()
        .checked_add(total)
        .ok_or_else(|| decision(ErrorClass::InvalidInput))?;
    let prepared = device
        .prepare_service(
            request,
            options.limits(),
            OperationOptions::new(total),
            cancel.clone(),
        )
        .map_err(|error| error_status(IOS_ERR_CORE, &error))?;
    let result = runtime.block_on(prepared);
    Ok((result, deadline))
}

fn partial<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a ServiceStreamFailure> {
    let mut source = Some(error);
    while let Some(error) = source {
        if let Some(partial) = error.downcast_ref::<ServiceStreamFailure>() {
            return Some(partial);
        }
        // std::io::Error::source can delegate to the wrapped error's source;
        // inspect get_ref first so the typed partial-result wrapper survives.
        source = match error.downcast_ref::<std::io::Error>() {
            Some(error) => error
                .get_ref()
                .map(|error| error as &(dyn std::error::Error + 'static))
                .or_else(|| std::error::Error::source(error)),
            None => error.source(),
        };
    }
    None
}

fn project(
    output: &mut JsonBuffer<'_>,
    observation: &DeviceServiceObservation,
) -> Result<(), c_int> {
    macro_rules! raw {
        ($bytes:expr) => {
            output
                .write_all($bytes)
                .map_err(|_| decision(output.failure.unwrap_or(ErrorClass::Unknown)))?;
        };
    }
    macro_rules! value {
        ($value:expr) => {
            serde_json::to_writer(&mut *output, $value).map_err(|error| match output.failure {
                Some(class) => decision(class),
                None => error_status(IOS_ERR_CORE, &error),
            })?;
        };
    }
    match observation {
        DeviceServiceObservation::Files(files) | DeviceServiceObservation::Logs(files) => {
            value!(files);
        }
        DeviceServiceObservation::Apps(apps) => {
            raw!(b"[");
            for (index, app) in apps.iter().enumerate() {
                if index != 0 {
                    raw!(b",");
                }
                raw!(b"{\"bundle_id\":");
                value!(&app.bundle_id);
                raw!(b",\"display_name\":");
                value!(&app.display_name);
                raw!(b",\"version\":");
                value!(&app.version);
                raw!(b",\"app_type\":");
                value!(&app.app_type);
                raw!(b",\"path\":");
                value!(&app.path);
                raw!(b"}");
            }
            raw!(b"]");
        }
        DeviceServiceObservation::Bytes(_) => return Err(decision(ErrorClass::InvalidInput)),
    }
    Ok(())
}

pub(super) unsafe fn run_json(
    device: &ExistingTrustDevice,
    request: DeviceServiceRequest,
    options: IosDeviceServiceOptions,
    cancel: &OperationCancel,
    out: *mut *mut c_char,
) -> c_int {
    let (result, deadline) = match run(device, request, options, cancel) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let mut output = JsonBuffer {
        bytes: Vec::with_capacity(options.json_bytes.min(4096)),
        limit: options.json_bytes,
        deadline,
        cancel,
        device,
        failure: None,
    };
    match result {
        Ok(observation) => {
            if let Err(code) = project(&mut output, &observation) {
                return code;
            }
            if let Err(class) = output_fence(cancel, device, deadline) {
                return decision(class);
            }
            match CString::new(output.bytes) {
                Ok(text) => {
                    *out = text.into_raw();
                    IOS_SUCCESS
                }
                Err(_) => decision(ErrorClass::Protocol),
            }
        }
        Err(error) => {
            // Partial output is evidence, never success. Serialize it with an
            // independent 100ms projection budget because the operation clock
            // or token has already terminated. Preserve the primary class.
            if let Some(partial) = partial(&error).filter(|partial| !partial.messages().is_empty())
            {
                let mut bytes = Vec::new();
                // All strings have already passed the retained-output limit.
                // A bounded Write rejects JSON expansion before append.
                struct PartialBuffer<'a> {
                    bytes: &'a mut Vec<u8>,
                    limit: usize,
                    until: Instant,
                    failure: &'a std::cell::Cell<Option<ErrorClass>>,
                }
                impl Write for PartialBuffer<'_> {
                    fn write(&mut self, part: &[u8]) -> std::io::Result<usize> {
                        if Instant::now() >= self.until {
                            self.failure.set(Some(ErrorClass::Timeout));
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "partial C projection deadline",
                            ));
                        }
                        if !self
                            .bytes
                            .len()
                            .checked_add(part.len())
                            .is_some_and(|n| n <= self.limit)
                        {
                            self.failure.set(Some(ErrorClass::ResourceLimit));
                            return Err(std::io::Error::other("partial C service output limit"));
                        }
                        self.bytes.extend_from_slice(part);
                        Ok(part.len())
                    }
                    fn flush(&mut self) -> std::io::Result<()> {
                        Ok(())
                    }
                }
                let projection_failure = std::cell::Cell::new(None);
                if let Err(error) = serde_json::to_writer(
                    PartialBuffer {
                        bytes: &mut bytes,
                        limit: options.json_bytes,
                        until: Instant::now() + std::time::Duration::from_millis(100),
                        failure: &projection_failure,
                    },
                    partial.messages(),
                ) {
                    PROJECTION_FAILURE.with(|state| {
                        state.set(Some(
                            projection_failure
                                .get()
                                .unwrap_or_else(|| ios_core::classify_error(&error)),
                        ))
                    });
                } else {
                    match CString::new(bytes) {
                        Ok(text) => {
                            *out = text.into_raw();
                        }
                        Err(_) => {
                            PROJECTION_FAILURE.with(|state| state.set(Some(ErrorClass::Protocol)))
                        }
                    };
                }
            }
            error_status(IOS_ERR_CORE, &error)
        }
    }
}

pub(super) unsafe fn run_bytes(
    device: &ExistingTrustDevice,
    path: String,
    max_bytes: usize,
    options: IosDeviceServiceOptions,
    cancel: &OperationCancel,
    out: *mut *mut u8,
    len: *mut usize,
) -> c_int {
    let (result, deadline) = match run(
        device,
        DeviceServiceRequest::ReadFile { path, max_bytes },
        options,
        cancel,
    ) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let bytes = match result {
        Ok(DeviceServiceObservation::Bytes(bytes)) => bytes,
        Ok(_) => return decision(ErrorClass::Protocol),
        Err(error) => return error_status(IOS_ERR_CORE, &error),
    };
    let mut bytes = bytes.into_boxed_slice();
    if let Err(class) = output_fence(cancel, device, deadline) {
        return decision(class);
    }
    if !bytes.is_empty() {
        *out = bytes.as_mut_ptr();
        *len = bytes.len();
        std::mem::forget(bytes);
    }
    IOS_SUCCESS
}

unsafe fn path(pointer: *const c_char, length: usize) -> Result<String, c_int> {
    if pointer.is_null() {
        return Err(IOS_ERR_NULL);
    }
    if length == 0 || length > 4096 {
        return Err(decision(ErrorClass::InvalidInput));
    }
    let bytes = std::slice::from_raw_parts(pointer.cast::<u8>(), length);
    if bytes.contains(&0) {
        return Err(decision(ErrorClass::InvalidInput));
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| IOS_ERR_UTF8)
}

/// # Safety
/// Options must be writable and distinct from live library allocations.
#[no_mangle]
pub unsafe extern "C" fn ios_device_service_options_default(
    out: *mut IosDeviceServiceOptions,
) -> c_int {
    ffi_status(|| {
        if out.is_null() {
            return IOS_ERR_NULL;
        }
        *out = IosDeviceServiceOptions::default();
        IOS_SUCCESS
    })
}

/// Create a services-only legacy handle sharing the configured checked owner.
/// The legacy generic lockdown queries require ios_device_open; configured
/// callers use ios_device_session_query_json for finite queries.
/// # Safety
/// Borrowed session remains live throughout; distinct output is writable and
/// cleared before allocation. Release follows legacy handle synchronization.
#[no_mangle]
pub unsafe extern "C" fn ios_device_handle_from_session(
    session: *const IosDeviceSession,
    out: *mut *mut IosDeviceHandle,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        if !out.is_null() {
            *out = std::ptr::null_mut();
        }
        if session.is_null() || out.is_null() {
            return IOS_ERR_NULL;
        }
        let state = &(*session).state;
        *out = Box::into_raw(Box::new(IosDeviceHandle {
            _device: Mutex::new(None),
            services: Some(Arc::new(state.device.clone())),
        }));
        IOS_SUCCESS
    }))
}

/// Apps=1, AFC directory=2, syslog=3. Path is length-delimited for selector 2.
/// On stream error, out may own partial messages; status remains nonzero.
/// # Safety
/// Borrowed session/cancel/options/path stay live until this synchronous call
/// returns. Output is distinct writable pointer storage. Destroy requires all
/// callers to have joined; session_close/cancel may run concurrently.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn ios_device_session_service_json(
    session: *const IosDeviceSession,
    selector: u32,
    path_pointer: *const c_char,
    path_len: usize,
    max_messages: usize,
    options: *const IosDeviceServiceOptions,
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
        let request = match selector {
            1 => DeviceServiceRequest::ListApps,
            2 => match path(path_pointer, path_len) {
                Ok(path) => DeviceServiceRequest::ListFiles { path },
                Err(code) => return code,
            },
            3 => DeviceServiceRequest::ReadSyslog { max_messages },
            _ => return decision(ErrorClass::InvalidInput),
        };
        let state = &(*session).state;
        run_json(&state.device, request, *options, &(*cancel).cancel, out)
    }))
}

/// # Safety
/// Ownership/synchronization follow ios_device_session_service_json. Both byte
/// outputs are writable/distinct and must be freed using the returned length.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn ios_device_session_afc_read_file(
    session: *const IosDeviceSession,
    path_pointer: *const c_char,
    path_len: usize,
    max_bytes: usize,
    options: *const IosDeviceServiceOptions,
    cancel: *const IosOperationCancel,
    out: *mut *mut u8,
    len: *mut usize,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        if !out.is_null() {
            *out = std::ptr::null_mut();
        }
        if !len.is_null() {
            *len = 0;
        }
        if session.is_null()
            || options.is_null()
            || cancel.is_null()
            || out.is_null()
            || len.is_null()
        {
            return IOS_ERR_NULL;
        }
        let path = match path(path_pointer, path_len) {
            Ok(path) => path,
            Err(code) => return code,
        };
        let state = &(*session).state;
        run_bytes(
            &state.device,
            path,
            max_bytes,
            *options,
            &(*cancel).cancel,
            out,
            len,
        )
    }))
}

/// Seal legacy-handle service admission, cancel active leases and await actual
/// local release without consuming the C allocation.
/// # Safety
/// Handle remains live; other service calls may run concurrently. Destruction
/// still requires all caller OS threads to have stopped/joined.
#[no_mangle]
pub unsafe extern "C" fn ios_device_services_close(
    handle: *const IosDeviceHandle,
    cleanup_ms: u64,
) -> c_int {
    ffi_status(AssertUnwindSafe(|| {
        let device = match from_handle(handle) {
            Ok(device) => device,
            Err(code) => return code,
        };
        match device_session::close_services(device, cleanup_ms) {
            Ok(()) => IOS_SUCCESS,
            Err(code) => code,
        }
    }))
}
