//! Read-only service jobs on the same checked SDK owner as finite queries.
use super::{
    device::{transport, Route},
    failure,
    reply_budget::{limit, OutputBudget, ReplyBudget},
    DeviceQueryLimits, ExistingTrustDevice, OperationCancel, OperationContext, OperationOptions,
    OperationValue,
};
use crate::{CoreError, ErrorClass, PairRecord};
use std::{
    fmt,
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Every request is read-only; each operation uses fresh discovery/session/service sockets.
#[derive(Debug, Clone)]
pub enum DeviceServiceRequest {
    ListApps,
    ListFiles { path: String },
    ReadFile { path: String, max_bytes: usize },
    ReadSyslog { max_messages: usize },
}

/// Logical reply/output limits; parser temporary allocations and TLS bytes are
/// separate. Individual AFC bodies are checked before allocation.
#[derive(Debug, Clone, Copy)]
pub struct DeviceServiceLimits {
    pub reply_wire_bytes: usize,
    pub output_bytes: usize,
    pub max_entries: usize,
    pub max_frame_bytes: usize,
}
impl Default for DeviceServiceLimits {
    fn default() -> Self {
        Self {
            reply_wire_bytes: 16 * 1024 * 1024,
            output_bytes: 1024 * 1024,
            max_entries: 10_000,
            max_frame_bytes: 1024 * 1024,
        }
    }
}
impl DeviceServiceLimits {
    fn validate(self) -> Result<Self, CoreError> {
        if !(1..=64 * 1024 * 1024).contains(&self.reply_wire_bytes)
            || !(1..=16 * 1024 * 1024).contains(&self.output_bytes)
            || self.max_entries > 100_000
            || !(48..=4 * 1024 * 1024).contains(&self.max_frame_bytes)
        {
            return Err(failure(
                ErrorClass::InvalidInput,
                "invalid SDK service limits",
            ));
        }
        Ok(self)
    }
}

pub enum DeviceServiceObservation {
    Apps(Vec<crate::apps::AppInfo>),
    Files(Vec<String>),
    Bytes(Vec<u8>),
    Logs(Vec<String>),
}
impl OperationValue for DeviceServiceObservation {}
impl fmt::Debug for DeviceServiceObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, count) = match self {
            Self::Apps(v) => ("apps", v.len()),
            Self::Files(v) => ("files", v.len()),
            Self::Bytes(v) => ("bytes", v.len()),
            Self::Logs(v) => ("logs", v.len()),
        };
        f.debug_struct("DeviceServiceObservation")
            .field("kind", &kind)
            .field("count", &count)
            .finish()
    }
}

/// A terminal stream failure owns completed messages and the original typed
/// cause. A caller must inspect the error even when partial messages exist.
pub struct ServiceStreamFailure {
    messages: Vec<String>,
    primary: CoreError,
}
impl ServiceStreamFailure {
    pub fn messages(&self) -> &[String] {
        &self.messages
    }
    pub fn primary(&self) -> &CoreError {
        &self.primary
    }
}
impl fmt::Display for ServiceStreamFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SDK log stream terminated before its requested count")
    }
}
impl fmt::Debug for ServiceStreamFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceStreamFailure")
            .field("completed_messages", &self.messages.len())
            .field("class", &crate::classify_error(&self.primary))
            .finish()
    }
}
impl std::error::Error for ServiceStreamFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.primary)
    }
}

impl ExistingTrustDevice {
    pub fn prepare_service(
        &self,
        request: DeviceServiceRequest,
        limits: DeviceServiceLimits,
        options: OperationOptions,
        cancel: OperationCancel,
    ) -> Result<PreparedDeviceService, CoreError> {
        let limits = limits.validate()?;
        match &request {
            DeviceServiceRequest::ListFiles { path }
            | DeviceServiceRequest::ReadFile { path, .. } => {
                if path.is_empty() || path.len() > 4096 || path.as_bytes().contains(&0) {
                    return Err(failure(ErrorClass::InvalidInput, "invalid SDK AFC path"));
                }
            }
            DeviceServiceRequest::ReadSyslog { max_messages }
                if *max_messages > limits.max_entries =>
            {
                return Err(failure(
                    ErrorClass::InvalidInput,
                    "SDK log count exceeds entry limit",
                ))
            }
            _ => {}
        }
        if let DeviceServiceRequest::ReadFile { max_bytes, .. } = &request {
            if *max_bytes > limits.output_bytes {
                return Err(failure(
                    ErrorClass::InvalidInput,
                    "invalid SDK file byte limit",
                ));
            }
        }
        let lease = self.owner.try_begin(options, cancel)?;
        let route = Arc::clone(&self.route);
        let pair = Arc::clone(&self.pair_record);
        let logs = matches!(request, DeviceServiceRequest::ReadSyslog { .. });
        let partial = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&partial);
        let future = lease.run(move |context| {
            Box::pin(
                async move { execute(context, &route, &pair, request, limits, &captured).await },
            )
        });
        // The outer lease fence may terminate a pending read before execute is
        // polled again. Retain completed messages outside that fenced future so
        // cancellation/expiry cannot discard the stream's partial evidence.
        Ok(PreparedDeviceService {
            future: Box::pin(async move {
                match future.await {
                    Ok(DeviceServiceObservation::Logs(_)) if logs => {
                        Ok(DeviceServiceObservation::Logs(std::mem::take(
                            &mut *partial.lock().unwrap_or_else(|e| e.into_inner()),
                        )))
                    }
                    Err(primary) if logs => Err(retained(ServiceStreamFailure {
                        messages: std::mem::take(
                            &mut *partial.lock().unwrap_or_else(|e| e.into_inner()),
                        ),
                        primary,
                    })),
                    result => result,
                }
            }),
        })
    }
}
pub struct PreparedDeviceService {
    future:
        Pin<Box<dyn Future<Output = Result<DeviceServiceObservation, CoreError>> + Send + 'static>>,
}
impl Future for PreparedDeviceService {
    type Output = Result<DeviceServiceObservation, CoreError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().future.as_mut().poll(cx)
    }
}
impl fmt::Debug for PreparedDeviceService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedDeviceService")
            .finish_non_exhaustive()
    }
}

trait ServiceIo: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> ServiceIo for T {}
type Stream<'a> = Box<dyn ServiceIo + 'a>;

// Cumulative actual decrypted service reply bytes; a depleted budget refuses
// another read rather than probing beyond the caller's limit.
struct BoundedRead<S> {
    inner: S,
    remaining: usize,
}
impl<S: AsyncRead + Unpin> AsyncRead for BoundedRead<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.remaining == 0 {
            return Poll::Ready(Err(limit("SDK service cumulative reply bytes exceeded")));
        }
        let max = self.remaining.min(buf.remaining());
        let mut part = ReadBuf::new(&mut buf.initialize_unfilled()[..max]);
        let result = Pin::new(&mut self.inner).poll_read(cx, &mut part);
        let read = part.filled().len();
        self.remaining -= read;
        buf.advance(read);
        result
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for BoundedRead<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, b)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

async fn connect<'a>(
    context: &'a OperationContext,
    route: &Route,
    pair: &PairRecord,
    service: &str,
    limits: DeviceServiceLimits,
) -> Result<Stream<'a>, CoreError> {
    let query_limits = DeviceQueryLimits::new(16 * 1024 * 1024, 1024 * 1024)?;
    // Discovery, lockdown replies, and the service Connect result share one
    // separate finite control-plane budget; service bytes have their own cap.
    let budget = Arc::new(ReplyBudget::new(16 * 1024 * 1024, 1024 * 1024, 1024 * 1024));
    let mut output = OutputBudget::new(1024 * 1024);
    let (metadata, stream) =
        transport::connect_lockdown(context, route, query_limits, &budget, &mut output).await?;
    let mut session = crate::lockdown::start_existing_trust_session_with_budget(
        context,
        stream,
        pair,
        Arc::clone(&budget),
    )
    .await?;
    let info = session.start_service(service).await?;
    let stream =
        transport::connect_port(context, route, metadata.device_id(), info.port, &budget).await?;
    let stream: Stream<'a> = if info.enable_service_ssl {
        let config = crate::lockdown::session::build_rustls_config(pair)?;
        let tls = context
            .run(tokio_rustls::TlsConnector::from(config).connect(
                rustls::pki_types::ServerName::try_from("lockdown").expect("static server name"),
                stream,
            ))
            .await?;
        Box::new(tls)
    } else {
        Box::new(stream)
    };
    drop(session);
    Ok(Box::new(BoundedRead {
        inner: stream,
        remaining: limits.reply_wire_bytes,
    }))
}

fn retained(error: impl std::error::Error + Send + Sync + 'static) -> CoreError {
    io::Error::other(error).into()
}
async fn execute(
    context: &OperationContext,
    route: &Route,
    pair: &PairRecord,
    request: DeviceServiceRequest,
    limits: DeviceServiceLimits,
    partial: &Mutex<Vec<String>>,
) -> Result<DeviceServiceObservation, CoreError> {
    let service = match &request {
        DeviceServiceRequest::ListApps => crate::apps::INSTALLATION_PROXY_SERVICE,
        DeviceServiceRequest::ReadSyslog { .. } => crate::syslog::SERVICE_NAME,
        _ => crate::afc::AFC_SERVICE,
    };
    if matches!(
        request,
        DeviceServiceRequest::ReadSyslog { max_messages: 0 }
    ) {
        return Ok(DeviceServiceObservation::Logs(Vec::new()));
    }
    let stream = connect(context, route, pair, service, limits).await?;
    let mut output = OutputBudget::new(limits.output_bytes);
    match request {
        DeviceServiceRequest::ListApps => {
            let remaining = context
                .deadline
                .saturating_duration_since(tokio::time::Instant::now());
            let browse = crate::apps::BrowseLimits::new(remaining)
                .with_max_entries(limits.max_entries)
                .with_max_wire_bytes(limits.reply_wire_bytes);
            let mut proxy = crate::apps::InstallationProxy::new(stream);
            let mut apps = proxy
                .list_user_apps_with_limits(browse)
                .await
                .map_err(retained)?;
            for app in &mut apps {
                output.admit(
                    app.bundle_id.len()
                        + app.display_name.len()
                        + app.version.len()
                        + app.app_type.len()
                        + app.path.len(),
                )?;
                app.extra.clear();
            }
            Ok(DeviceServiceObservation::Apps(apps))
        }
        DeviceServiceRequest::ListFiles { path } => {
            let mut client =
                crate::afc::AfcClient::new(stream).with_max_frame_body(limits.max_frame_bytes);
            let files = client.list_dir(&path).await.map_err(retained)?;
            if files.len() > limits.max_entries {
                return Err(limit("SDK AFC directory entries exceeded").into());
            }
            for name in &files {
                output.admit(name.len())?;
            }
            Ok(DeviceServiceObservation::Files(files))
        }
        DeviceServiceRequest::ReadFile { path, max_bytes } => {
            let mut client =
                crate::afc::AfcClient::new(stream).with_max_frame_body(limits.max_frame_bytes);
            let bytes = client
                .read_file_with_limit(&path, max_bytes)
                .await
                .map_err(retained)?;
            output.admit(bytes.len())?;
            Ok(DeviceServiceObservation::Bytes(bytes.to_vec()))
        }
        DeviceServiceRequest::ReadSyslog { max_messages } => {
            let mut log = crate::syslog::SyslogStream::new(stream);
            for _ in 0..max_messages {
                let message = log.next_message().await?;
                output.admit(message.len())?;
                partial
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(message);
            }
            // Publish the vector only after the outer operation fence accepts
            // completion. A late successful read still leaves partial evidence.
            Ok(DeviceServiceObservation::Logs(Vec::new()))
        }
    }
}
