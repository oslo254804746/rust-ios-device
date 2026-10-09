//! Explicit existing-trust device queries with fresh operation-owned sockets.
//!
//! A handle stores routing and already loaded credentials, never a live socket.
//! Each prepared operation captures its lease and immutable inputs rather than
//! cloning the handle. Dropping the last client handle therefore seals its owner.
//! No file loader, hostname resolver, pairing, service startup or tunnel exists
//! in this path. Ordinary plist decoding is not a CPU or allocator/RSS bound.

#[cfg(test)]
mod tests;
mod transport;

use std::{
    fmt,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

#[cfg(unix)]
use std::path::PathBuf;

use super::reply_budget::{
    OutputBudget, ReplyBudget, MAX_LOCKDOWN_BODY, MAX_MUX_FRAME, MAX_QUERY_REPLY_WIRE,
    MAX_RETAINED_UTF8,
};
use super::{failure, OperationCancel, OperationOptions, OperationValue, SessionOwner};
use crate::{CoreError, ErrorClass, PairRecord};

#[derive(Clone)]
enum Endpoint {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}

/// A numeric TCP endpoint or Unix socket path; no DNS worker is spawned.
#[derive(Clone)]
pub struct UsbmuxEndpoint(Endpoint);

impl UsbmuxEndpoint {
    pub fn tcp(address: SocketAddr) -> Self {
        Self(Endpoint::Tcp(address))
    }

    #[cfg(unix)]
    pub fn unix(path: PathBuf) -> Result<Self, CoreError> {
        use std::os::unix::ffi::OsStrExt;
        if path.as_os_str().is_empty() || path.as_os_str().as_bytes().contains(&0) {
            return Err(failure(
                ErrorClass::InvalidInput,
                "invalid usbmux Unix endpoint",
            ));
        }
        Ok(Self(Endpoint::Unix(path)))
    }

    /// Snapshot the existing usbmux environment/default address without IO.
    /// A configured hostname is rejected; legacy MuxClient still supports it.
    pub fn system() -> Result<Self, CoreError> {
        let address = crate::mux::connection::usbmuxd_socket_address();
        if let Some(address) = address.strip_prefix("tcp://") {
            let address = address.parse::<SocketAddr>().map_err(|error| {
                crate::error::context::retained_io(
                    std::io::ErrorKind::InvalidInput,
                    "parse numeric SDK usbmux TCP endpoint",
                    error,
                )
            })?;
            return Ok(Self::tcp(address));
        }
        #[cfg(unix)]
        if let Some(path) = address.strip_prefix("unix://") {
            return Self::unix(path.into());
        }
        Err(failure(
            ErrorClass::Unsupported,
            "SDK usbmux endpoint is unavailable on this platform",
        ))
    }
}

impl fmt::Debug for UsbmuxEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UsbmuxEndpoint")
            .field(
                "kind",
                &match &self.0 {
                    Endpoint::Tcp(_) => "tcp",
                    #[cfg(unix)]
                    Endpoint::Unix(_) => "unix",
                },
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfoFields {
    product_type: bool,
    product_version: bool,
    device_name: bool,
}

impl DeviceInfoFields {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn all() -> Self {
        Self {
            product_type: true,
            product_version: true,
            device_name: true,
        }
    }
    pub fn with_product_type(mut self) -> Self {
        self.product_type = true;
        self
    }
    pub fn with_product_version(mut self) -> Self {
        self.product_version = true;
        self
    }
    pub fn with_device_name(mut self) -> Self {
        self.device_name = true;
        self
    }
    pub fn includes_product_type(&self) -> bool {
        self.product_type
    }
    pub fn includes_product_version(&self) -> bool {
        self.product_version
    }
    pub fn includes_device_name(&self) -> bool {
        self.device_name
    }
    fn is_empty(self) -> bool {
        !self.product_type && !self.product_version && !self.device_name
    }
}

#[derive(Clone, Copy, Debug)]
enum Selection {
    ProductType,
    ProductVersion,
    DeviceName,
    Root(DeviceInfoFields),
}

/// Finite selectors only. Root queries retain only the explicitly selected fields.
#[derive(Clone, Copy, Debug)]
pub struct DeviceQuery {
    selection: Selection,
}

impl DeviceQuery {
    pub fn product_type() -> Self {
        Self {
            selection: Selection::ProductType,
        }
    }
    pub fn product_version() -> Self {
        Self {
            selection: Selection::ProductVersion,
        }
    }
    pub fn device_name() -> Self {
        Self {
            selection: Selection::DeviceName,
        }
    }
    pub fn root(fields: DeviceInfoFields) -> Result<Self, CoreError> {
        if fields.is_empty() {
            return Err(failure(
                ErrorClass::InvalidInput,
                "empty device information selection",
            ));
        }
        Ok(Self {
            selection: Selection::Root(fields),
        })
    }
    pub(crate) fn key(self) -> Option<&'static str> {
        match self.selection {
            Selection::ProductType => Some("ProductType"),
            Selection::ProductVersion => Some("ProductVersion"),
            Selection::DeviceName => Some("DeviceName"),
            Selection::Root(_) => None,
        }
    }
    pub(crate) fn fields(self) -> DeviceInfoFields {
        match self.selection {
            Selection::ProductType => DeviceInfoFields::new().with_product_type(),
            Selection::ProductVersion => DeviceInfoFields::new().with_product_version(),
            Selection::DeviceName => DeviceInfoFields::new().with_device_name(),
            Selection::Root(fields) => fields,
        }
    }
}

/// Checked logical limits. Replies count their prefix/header and declared body;
/// requests and TLS bytes do not count. Metadata counts every listed entry and
/// all SerialNumber + ConnectionType UTF-8 bytes after ordinary plist decoding.
/// Output counts selected strings and the returned ConnectionType, before clone.
/// These limits do not certify parser temporary allocation, CPU, or RSS.
#[derive(Clone, Copy, Debug)]
pub struct DeviceQueryLimits {
    reply_wire: usize,
    mux_frame: usize,
    lockdown_body: usize,
    metadata_entries: usize,
    metadata_utf8: usize,
    output_utf8: usize,
}

impl DeviceQueryLimits {
    /// Both budgets must be nonzero. The wire budget cannot exceed the sum of
    /// two 16MiB mux frames and three (4B + 4MiB) lockdown replies; retained
    /// UTF-8 cannot exceed 16MiB. Larger parameters are rejected before IO.
    /// Defaults retain the existing per-frame caps, 1024 metadata entries and
    /// 16MiB metadata UTF-8. Builders only accept finite documented bounds.
    pub fn new(reply_wire: usize, output_utf8: usize) -> Result<Self, CoreError> {
        if !(1..=MAX_QUERY_REPLY_WIRE).contains(&reply_wire)
            || !(1..=MAX_RETAINED_UTF8).contains(&output_utf8)
        {
            return Err(failure(
                ErrorClass::InvalidInput,
                "invalid device query budget",
            ));
        }
        Ok(Self {
            reply_wire,
            output_utf8,
            mux_frame: MAX_MUX_FRAME,
            lockdown_body: MAX_LOCKDOWN_BODY,
            metadata_entries: 1024,
            metadata_utf8: MAX_RETAINED_UTF8,
        })
    }
    pub fn with_mux_frame_bytes(mut self, bytes: usize) -> Result<Self, CoreError> {
        if !(16..=MAX_MUX_FRAME).contains(&bytes) {
            return Err(failure(
                ErrorClass::InvalidInput,
                "invalid device query mux frame limit",
            ));
        }
        self.mux_frame = bytes;
        Ok(self)
    }
    pub fn with_lockdown_body_bytes(mut self, bytes: usize) -> Result<Self, CoreError> {
        if !(1..=MAX_LOCKDOWN_BODY).contains(&bytes) {
            return Err(failure(
                ErrorClass::InvalidInput,
                "invalid device query lockdown body limit",
            ));
        }
        self.lockdown_body = bytes;
        Ok(self)
    }
    /// Accept 1..=1,000,000 entries and 1..=16MiB aggregate metadata UTF-8.
    /// Checking occurs after plist decode, before selected metadata is cloned.
    pub fn with_metadata_limits(
        mut self,
        entries: usize,
        utf8_bytes: usize,
    ) -> Result<Self, CoreError> {
        if entries == 0 || entries > 1_000_000 || !(1..=MAX_RETAINED_UTF8).contains(&utf8_bytes) {
            return Err(failure(
                ErrorClass::InvalidInput,
                "invalid device query metadata limits",
            ));
        }
        self.metadata_entries = entries;
        self.metadata_utf8 = utf8_bytes;
        Ok(self)
    }
}

/// Selected metadata preserves omitted ProductID separately from advertised zero.
pub struct DeviceMetadata {
    device_id: u32,
    connection_type: String,
    product_id: Option<u16>,
}

impl DeviceMetadata {
    pub fn device_id(&self) -> u32 {
        self.device_id
    }
    pub fn connection_type(&self) -> &str {
        &self.connection_type
    }
    pub fn product_id(&self) -> Option<u16> {
        self.product_id
    }
}

impl fmt::Debug for DeviceMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceMetadata")
            .field("product_id_present", &self.product_id.is_some())
            .finish_non_exhaustive()
    }
}

pub(crate) struct QueryStrings {
    pub(crate) value_present: bool,
    pub(crate) product_type: Option<String>,
    pub(crate) product_version: Option<String>,
    pub(crate) device_name: Option<String>,
}

/// Owned finite information; no raw plist, socket, or checked session escapes.
pub struct DeviceQueryObservation {
    metadata: DeviceMetadata,
    fields: DeviceInfoFields,
    enable_session_ssl: bool,
    values: QueryStrings,
}

impl DeviceQueryObservation {
    pub fn metadata(&self) -> &DeviceMetadata {
        &self.metadata
    }
    pub fn fields(&self) -> DeviceInfoFields {
        self.fields
    }
    pub fn enable_session_ssl(&self) -> bool {
        self.enable_session_ssl
    }
    pub fn value_present(&self) -> bool {
        self.values.value_present
    }
    pub fn product_type(&self) -> Option<&str> {
        self.values.product_type.as_deref()
    }
    pub fn product_version(&self) -> Option<&str> {
        self.values.product_version.as_deref()
    }
    pub fn device_name(&self) -> Option<&str> {
        self.values.device_name.as_deref()
    }
}

impl fmt::Debug for DeviceQueryObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceQueryObservation")
            .field("fields", &self.fields)
            .field("enable_session_ssl", &self.enable_session_ssl)
            .field("value_present", &self.values.value_present)
            .finish_non_exhaustive()
    }
}
impl OperationValue for DeviceQueryObservation {}

struct Route {
    udid: String,
    endpoint: UsbmuxEndpoint,
}

/// An explicit target using credentials already loaded by the caller.
/// Construction performs no IO or authentication. Clones are client handles;
/// prepared jobs never retain one. Close only acknowledges local lease release.
pub struct ExistingTrustDevice {
    route: Arc<Route>,
    pair_record: Arc<PairRecord>,
    owner: SessionOwner,
}

impl Clone for ExistingTrustDevice {
    fn clone(&self) -> Self {
        Self {
            route: Arc::clone(&self.route),
            pair_record: Arc::clone(&self.pair_record),
            owner: self.owner.clone(),
        }
    }
}

impl fmt::Debug for ExistingTrustDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExistingTrustDevice")
            .field("closed", &self.is_closed())
            .field("active", &self.active())
            .finish_non_exhaustive()
    }
}

impl ExistingTrustDevice {
    pub fn new(
        udid: String,
        pair_record: Arc<PairRecord>,
        owner: SessionOwner,
        endpoint: UsbmuxEndpoint,
    ) -> Result<Self, CoreError> {
        if matches!(&endpoint.0, Endpoint::Tcp(address) if address.port() == 0) {
            return Err(failure(
                ErrorClass::InvalidInput,
                "SDK usbmux TCP port must be nonzero",
            ));
        }
        if udid.is_empty() || udid.as_bytes().contains(&0) {
            return Err(failure(
                ErrorClass::InvalidInput,
                "invalid explicit device identifier",
            ));
        }
        if udid.len() > 4096 {
            return Err(failure(
                ErrorClass::ResourceLimit,
                "device identifier exceeds byte limit",
            ));
        }
        Ok(Self {
            route: Arc::new(Route { udid, endpoint }),
            pair_record,
            owner,
        })
    }
    pub fn active(&self) -> usize {
        self.owner.active()
    }
    pub fn is_closed(&self) -> bool {
        self.owner.is_closed()
    }
    pub fn request_close(&self) {
        self.owner.request_close();
    }
    pub async fn close(&self, cleanup: Duration) -> Result<(), CoreError> {
        self.owner.close(cleanup).await
    }

    /// Reserve immediately without a queue; the total starts before this future
    /// is polled. Every attempt dials a fresh socket and reselects the exact UDID.
    /// Cancellation/failure never reuses or retries its transport. A response
    /// missing Value yields value_present=false; explicit remote MissingValue
    /// remains a typed NotFound error. Selected fields must be strings, and
    /// their empty strings remain present. Root retains no unselected fields.
    pub fn prepare_query(
        &self,
        query: DeviceQuery,
        limits: DeviceQueryLimits,
        options: OperationOptions,
        cancel: OperationCancel,
    ) -> Result<PreparedDeviceQuery, CoreError> {
        let lease = self.owner.try_begin(options, cancel)?;
        let route = Arc::clone(&self.route);
        let pair_record = Arc::clone(&self.pair_record);
        let budget = Arc::new(ReplyBudget::new(
            limits.reply_wire,
            limits.mux_frame,
            limits.lockdown_body,
        ));
        #[cfg(test)]
        let observed_budget = Arc::clone(&budget);
        let future = lease.run(move |context| {
            Box::pin(async move {
                let mut output = OutputBudget::new(limits.output_utf8);
                let (metadata, stream) =
                    transport::connect_lockdown(context, &route, limits, &budget, &mut output)
                        .await?;
                let mut session = crate::lockdown::start_existing_trust_session_with_budget(
                    context,
                    stream,
                    pair_record.as_ref(),
                    budget,
                )
                .await?;
                let enable_session_ssl = session.uses_tls();
                let values = session.get_value(query, &mut output).await?;
                Ok(DeviceQueryObservation {
                    metadata,
                    fields: query.fields(),
                    enable_session_ssl,
                    values,
                })
            })
        });
        Ok(PreparedDeviceQuery {
            future: Box::pin(future),
            #[cfg(test)]
            observed_budget,
        })
    }
}

/// A job owns admission and input snapshots, not a hidden client handle.
pub struct PreparedDeviceQuery {
    #[cfg(test)]
    observed_budget: Arc<ReplyBudget>,
    future:
        Pin<Box<dyn Future<Output = Result<DeviceQueryObservation, CoreError>> + Send + 'static>>,
}
impl Future for PreparedDeviceQuery {
    type Output = Result<DeviceQueryObservation, CoreError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().future.as_mut().poll(cx)
    }
}
impl fmt::Debug for PreparedDeviceQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedDeviceQuery")
            .finish_non_exhaustive()
    }
}

/// A nonzero usbmux Result number, without guessed meanings for device codes.
pub struct MuxResponseError {
    code: u64,
}
impl MuxResponseError {
    pub fn code(&self) -> u64 {
        self.code
    }
    pub fn class(&self) -> ErrorClass {
        ErrorClass::ServiceFailure
    }
}
impl fmt::Display for MuxResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SDK usbmux request rejected")
    }
}
impl fmt::Debug for MuxResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MuxResponseError").finish_non_exhaustive()
    }
}
impl std::error::Error for MuxResponseError {}
