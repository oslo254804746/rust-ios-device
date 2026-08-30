//! Companion proxy service client.
//!
//! The companion proxy is a plist-framed lockdown service used for paired
//! accessories such as Apple Watch.  The tunnel/RSD service uses the same
//! protocol under [`RSD_SERVICE_NAME`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_core::Stream;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;

/// Lockdown service name for the classic companion proxy service.
pub const SERVICE_NAME: &str = "com.apple.companion_proxy";
/// RSD service name for the companion proxy shim.
pub const RSD_SERVICE_NAME: &str = "com.apple.companion_proxy.shim.remote";

const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;

type EventFuture = Pin<Box<dyn Future<Output = Result<plist::Dictionary, CompanionError>> + Send>>;

service_error!(CompanionError, between {
    /// The operation did not complete before its configured deadline.
    #[error("companion operation timed out after {0:?}")]
    Timeout(Duration),
});

/// A client for the companion proxy plist protocol.
pub struct CompanionProxyClient<S> {
    stream: Arc<Mutex<S>>,
    timeout: Duration,
}

impl<S: AsyncRead + AsyncWrite + Unpin> CompanionProxyClient<S> {
    /// Construct a client with the standard ten-second request timeout.
    pub fn new(stream: S) -> Self {
        Self::with_timeout(stream, DEFAULT_OPERATION_TIMEOUT)
    }

    /// Construct a client with an explicit timeout for request/response
    /// operations.  Device-listening reads are idle by design; use
    /// [`CompanionDeviceStream::next_event_with_timeout`] when an idle limit
    /// is required for a listener.
    pub fn with_timeout(stream: S, timeout: Duration) -> Self {
        Self {
            stream: Arc::new(Mutex::new(stream)),
            timeout,
        }
    }

    /// Return the configured request timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// List the paired companion devices currently known to the device.
    pub async fn list(&mut self) -> Result<Vec<plist::Value>, CompanionError> {
        let request = command("GetDeviceRegistry");
        let response = self.round_trip(request).await?;
        if let Some(error) = device_error(&response, "GetDeviceRegistry") {
            return Err(error);
        }

        Ok(response
            .get("PairedDevicesArray")
            .and_then(plist::Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Query one value from a paired companion registry.
    pub async fn get_value(
        &mut self,
        udid: &str,
        key: &str,
    ) -> Result<plist::Value, CompanionError> {
        let request = plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("GetValueFromRegistry".into()),
            ),
            (
                "GetValueGizmoUDIDKey".to_string(),
                plist::Value::String(udid.to_string()),
            ),
            (
                "GetValueKeyKey".to_string(),
                plist::Value::String(key.to_string()),
            ),
        ]);
        let response = self.round_trip(request).await?;
        if let Some(value) = response.get("RetrievedValueDictionary") {
            return Ok(value.clone());
        }
        if let Some(error) = device_error(&response, "GetValueFromRegistry") {
            return Err(error);
        }
        Err(CompanionError::Protocol(
            "GetValueFromRegistry response is missing RetrievedValueDictionary".into(),
        ))
    }

    /// Start the companion device event stream.
    ///
    /// This consumes the client because event messages and request/response
    /// messages share one wire connection.  Each dictionary is yielded as-is,
    /// including future/unknown event shapes and duplicate events.
    pub async fn listen_for_devices(self) -> Result<CompanionDeviceStream<S>, CompanionError>
    where
        S: Send + 'static,
    {
        self.send_only(command("StartListeningForDevices")).await?;
        Ok(CompanionDeviceStream {
            stream: self.stream,
            timeout: self.timeout,
            pending: None,
            finished: false,
        })
    }

    /// Start forwarding a service port on the companion device.
    ///
    /// The returned value is the `CompanionProxyServicePort` assigned by the
    /// device, matching the pmd3/libimobiledevice API.  Call
    /// [`Self::start_forwarding_service_port_handle`] when the forwarding
    /// lifetime should be tied to an RAII handle.
    pub async fn start_forwarding_service_port(
        &mut self,
        remote_port: u16,
        service_name: Option<&str>,
        options: Option<plist::Dictionary>,
    ) -> Result<u16, CompanionError> {
        let (service_port, _, _) = self
            .start_forwarding_service_port_inner(remote_port, service_name, options)
            .await?;
        Ok(service_port)
    }

    /// Start forwarding a service port and return an RAII handle.
    ///
    /// Dropping the handle schedules a bounded best-effort stop when a Tokio
    /// runtime is available.  [`ForwardingHandle::stop`] is the reliable,
    /// awaited form and is idempotent after the first successful stop.
    pub async fn start_forwarding_service_port_handle(
        &mut self,
        remote_port: u16,
        service_name: Option<&str>,
        options: Option<plist::Dictionary>,
    ) -> Result<ForwardingHandle<S>, CompanionError>
    where
        S: Send + 'static,
    {
        let (service_port, response, effective_remote_port) = self
            .start_forwarding_service_port_inner(remote_port, service_name, options)
            .await?;
        Ok(ForwardingHandle {
            stream: Arc::clone(&self.stream),
            timeout: self.timeout,
            remote_port: effective_remote_port,
            service_port,
            response,
            stop_response: None,
            stopped: false,
        })
    }

    /// Stop forwarding the given companion-device port and return the raw
    /// response dictionary.  The protocol identifies a forwarding by its
    /// remote port; it does not define a reusable host-side connection ID.
    pub async fn stop_forwarding_service_port(
        &mut self,
        remote_port: u16,
    ) -> Result<plist::Dictionary, CompanionError> {
        validate_port(remote_port, "GizmoRemotePortNumber")?;
        let response = self
            .round_trip(plist::Dictionary::from_iter([
                (
                    "Command".to_string(),
                    plist::Value::String("StopForwardingServicePort".into()),
                ),
                (
                    "GizmoRemotePortNumber".to_string(),
                    plist::Value::Integer(remote_port.into()),
                ),
            ]))
            .await?;
        if let Some(error) = device_error(&response, "StopForwardingServicePort") {
            return Err(error);
        }
        Ok(response)
    }

    async fn start_forwarding_service_port_inner(
        &self,
        remote_port: u16,
        service_name: Option<&str>,
        options: Option<plist::Dictionary>,
    ) -> Result<(u16, plist::Dictionary, u16), CompanionError> {
        validate_port(remote_port, "GizmoRemotePortNumber")?;
        if let Some(service_name) = service_name {
            validate_service_name(service_name)?;
        }

        let mut request = plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("StartForwardingServicePort".into()),
            ),
            (
                "GizmoRemotePortNumber".to_string(),
                plist::Value::Integer(remote_port.into()),
            ),
            (
                "IsServiceLowPriority".to_string(),
                plist::Value::Boolean(false),
            ),
            ("PreferWifi".to_string(), plist::Value::Boolean(false)),
        ]);
        if let Some(service_name) = service_name {
            request.insert(
                "ForwardedServiceName".into(),
                plist::Value::String(service_name.to_owned()),
            );
        }
        // pmd3 merges caller options last.  Validate the resulting port and
        // service name so an override cannot silently wrap or emit malformed
        // protocol values.
        if let Some(options) = options {
            request.extend(options);
        }
        let effective_remote_port = request
            .get("GizmoRemotePortNumber")
            .ok_or_else(|| {
                CompanionError::Protocol(
                    "StartForwardingServicePort request is missing GizmoRemotePortNumber".into(),
                )
            })
            .and_then(|value| parse_port(value, "GizmoRemotePortNumber"))?;
        if let Some(value) = request.get("ForwardedServiceName") {
            let name = value.as_string().ok_or_else(|| {
                CompanionError::Protocol(
                    "ForwardedServiceName must be a string when supplied".into(),
                )
            })?;
            validate_service_name(name)?;
        }

        let response = self.round_trip(request).await?;
        if let Some(error) = device_error(&response, "StartForwardingServicePort") {
            return Err(error);
        }
        let service_port = response
            .get("CompanionProxyServicePort")
            .ok_or_else(|| {
                CompanionError::Protocol(
                    "StartForwardingServicePort response is missing CompanionProxyServicePort"
                        .into(),
                )
            })
            .and_then(|value| parse_port(value, "CompanionProxyServicePort"))?;
        Ok((service_port, response, effective_remote_port))
    }

    async fn send_only(&self, request: plist::Dictionary) -> Result<(), CompanionError> {
        let stream = Arc::clone(&self.stream);
        let operation = async move {
            let mut stream = stream.lock().await;
            super::plist_frame::write_xml_plist_frame(
                &mut *stream,
                &plist::Value::Dictionary(request),
                MAX_PLIST_SIZE,
            )
            .await
            .map_err(map_frame_error)
        };
        tokio::time::timeout(self.timeout, operation)
            .await
            .map_err(|_| CompanionError::Timeout(self.timeout))?
    }

    async fn round_trip(
        &self,
        request: plist::Dictionary,
    ) -> Result<plist::Dictionary, CompanionError> {
        let stream = Arc::clone(&self.stream);
        let operation = async move {
            let mut stream = stream.lock().await;
            super::plist_frame::write_xml_plist_frame(
                &mut *stream,
                &plist::Value::Dictionary(request),
                MAX_PLIST_SIZE,
            )
            .await
            .map_err(map_frame_error)?;
            super::plist_frame::read_plist_frame(&mut *stream, MAX_PLIST_SIZE)
                .await
                .map_err(map_frame_error)
        };
        tokio::time::timeout(self.timeout, operation)
            .await
            .map_err(|_| CompanionError::Timeout(self.timeout))?
    }
}

/// A stream of raw companion-proxy device event dictionaries.
pub struct CompanionDeviceStream<S>
where
    S: AsyncRead + Unpin + Send + 'static,
{
    stream: Arc<Mutex<S>>,
    timeout: Duration,
    pending: Option<EventFuture>,
    finished: bool,
}

impl<S> Unpin for CompanionDeviceStream<S> where S: AsyncRead + Unpin + Send + 'static {}

impl<S> Stream for CompanionDeviceStream<S>
where
    S: AsyncRead + Unpin + Send + 'static,
{
    type Item = Result<plist::Dictionary, CompanionError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        if this.pending.is_none() {
            let stream = Arc::clone(&this.stream);
            this.pending = Some(Box::pin(async move {
                let mut stream = stream.lock().await;
                super::plist_frame::read_plist_frame(&mut *stream, MAX_PLIST_SIZE)
                    .await
                    .map_err(map_frame_error)
            }));
        }

        let Some(pending) = this.pending.as_mut() else {
            // `pending` is initialized above. Keep this defensive branch so
            // a future refactor cannot turn an invalid state into a panic.
            return Poll::Pending;
        };
        match pending.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                this.pending = None;
                if result.is_err() {
                    this.finished = true;
                }
                Poll::Ready(Some(result))
            }
        }
    }
}

impl<S> CompanionDeviceStream<S>
where
    S: AsyncRead + Unpin + Send + 'static,
{
    /// Await the next raw event, returning `None` after EOF or the first
    /// terminal read error.
    pub async fn next_event(&mut self) -> Option<Result<plist::Dictionary, CompanionError>> {
        futures_util::StreamExt::next(self).await
    }

    /// Await one event with an idle timeout.  EOF is returned as `Ok(None)`;
    /// malformed frames and transport errors are returned as `Err`.
    pub async fn next_event_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<plist::Dictionary>, CompanionError> {
        match tokio::time::timeout(timeout, self.next_event()).await {
            Ok(item) => item.transpose(),
            Err(_) => {
                // A cancelled read_exact may already have consumed part of a
                // frame. Do not let callers resume on a desynchronized
                // plist stream after an idle timeout.
                self.finished = true;
                Err(CompanionError::Timeout(timeout))
            }
        }
    }

    /// The timeout inherited from the client for diagnostics and callers that
    /// want to choose a related idle deadline.
    pub fn request_timeout(&self) -> Duration {
        self.timeout
    }
}

/// RAII owner for one active companion-proxy forwarding request.
pub struct ForwardingHandle<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    stream: Arc<Mutex<S>>,
    timeout: Duration,
    remote_port: u16,
    service_port: u16,
    response: plist::Dictionary,
    stop_response: Option<plist::Dictionary>,
    stopped: bool,
}

impl<S> ForwardingHandle<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    /// The companion-device port used to identify this forwarding.
    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }

    /// The device-side proxy port returned as `CompanionProxyServicePort`.
    pub fn service_port(&self) -> u16 {
        self.service_port
    }

    /// The default timeout used by [`Self::stop`].
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The raw start response, retained for forward-compatible fields.
    pub fn response(&self) -> &plist::Dictionary {
        &self.response
    }

    /// Return a protocol-provided connection identifier if a device includes
    /// one, without assuming a field that pmd3 does not currently require.
    pub fn connection_id(&self) -> Option<&plist::Value> {
        self.response
            .get("CompanionProxyConnectionID")
            .or_else(|| self.response.get("ConnectionID"))
    }

    /// Whether a successful stop has already been completed.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// Stop this forwarding.  Repeated calls after success are no-ops and
    /// return the original stop response; a failed stop remains retryable.
    pub async fn stop(&mut self) -> Result<plist::Dictionary, CompanionError> {
        self.stop_with_timeout(self.timeout).await
    }

    /// Stop this forwarding with a caller-provided remaining deadline.
    ///
    /// This is useful to callers that share one absolute deadline across
    /// connection, start, wait, and cleanup. Repeated calls after success
    /// remain no-ops regardless of the supplied timeout.
    pub async fn stop_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<plist::Dictionary, CompanionError> {
        if self.stopped {
            return Ok(self.stop_response.clone().unwrap_or_default());
        }
        let response = tokio::time::timeout(
            timeout,
            stop_shared(Arc::clone(&self.stream), self.remote_port),
        )
        .await
        .map_err(|_| CompanionError::Timeout(timeout))??;
        self.stopped = true;
        self.stop_response = Some(response.clone());
        Ok(response)
    }
}

impl<S> Drop for ForwardingHandle<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    fn drop(&mut self) {
        if self.stopped {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let stream = Arc::clone(&self.stream);
        let timeout = self.timeout;
        let remote_port = self.remote_port;
        runtime.spawn(async move {
            let _ = tokio::time::timeout(timeout, stop_shared(stream, remote_port)).await;
        });
    }
}

async fn stop_shared<S>(
    stream: Arc<Mutex<S>>,
    remote_port: u16,
) -> Result<plist::Dictionary, CompanionError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    validate_port(remote_port, "GizmoRemotePortNumber")?;
    let mut stream = stream.lock().await;
    super::plist_frame::write_xml_plist_frame(
        &mut *stream,
        &plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("StopForwardingServicePort".into()),
            ),
            (
                "GizmoRemotePortNumber".to_string(),
                plist::Value::Integer(remote_port.into()),
            ),
        ])),
        MAX_PLIST_SIZE,
    )
    .await
    .map_err(map_frame_error)?;
    let response: plist::Dictionary =
        super::plist_frame::read_plist_frame(&mut *stream, MAX_PLIST_SIZE)
            .await
            .map_err(map_frame_error)?;
    if let Some(error) = device_error(&response, "StopForwardingServicePort") {
        return Err(error);
    }
    Ok(response)
}

fn command(name: &str) -> plist::Dictionary {
    plist::Dictionary::from_iter([(
        "Command".to_string(),
        plist::Value::String(name.to_string()),
    )])
}

fn validate_port(port: u16, field: &str) -> Result<(), CompanionError> {
    if port == 0 {
        return Err(CompanionError::Protocol(format!(
            "{field} must be between 1 and {}",
            u16::MAX
        )));
    }
    Ok(())
}

fn parse_port(value: &plist::Value, field: &str) -> Result<u16, CompanionError> {
    let number = value
        .as_unsigned_integer()
        .or_else(|| {
            value
                .as_signed_integer()
                .and_then(|number| number.try_into().ok())
        })
        .ok_or_else(|| {
            CompanionError::Protocol(format!("{field} must be a non-negative integer"))
        })?;
    let port = u16::try_from(number).map_err(|_| {
        CompanionError::Protocol(format!("{field} {number} exceeds the u16 port range"))
    })?;
    validate_port(port, field)?;
    Ok(port)
}

fn validate_service_name(name: &str) -> Result<(), CompanionError> {
    if name.is_empty() {
        return Err(CompanionError::Protocol(
            "ForwardedServiceName must not be empty".into(),
        ));
    }
    if name.as_bytes().contains(&0) {
        return Err(CompanionError::Protocol(
            "ForwardedServiceName must not contain NUL".into(),
        ));
    }
    const MAX_SERVICE_NAME_BYTES: usize = 1024;
    if name.len() > MAX_SERVICE_NAME_BYTES {
        return Err(CompanionError::Protocol(format!(
            "ForwardedServiceName exceeds {MAX_SERVICE_NAME_BYTES} bytes"
        )));
    }
    Ok(())
}

fn device_error(response: &plist::Dictionary, operation: &str) -> Option<CompanionError> {
    response.get("Error").map(|value| {
        let detail = value.as_string().unwrap_or("device returned an error");
        CompanionError::Protocol(format!("{operation} failed: {detail}"))
    })
}

fn map_frame_error(error: super::plist_frame::PlistFrameError) -> CompanionError {
    match error {
        super::plist_frame::PlistFrameError::Io(error) => CompanionError::Io(error),
        super::plist_frame::PlistFrameError::Plist(error) => CompanionError::Plist(error),
        super::plist_frame::PlistFrameError::Protocol(error) => CompanionError::Protocol(error),
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::pin::Pin;
    use std::sync::{Arc as StdArc, Mutex as StdMutex};
    use std::task::{Context, Poll};

    use crate::test_util::MockStream;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::*;

    fn dict_frame(value: plist::Value) -> Vec<u8> {
        MockStream::plist_frame(value)
    }

    fn request_dict(written: &[u8]) -> plist::Dictionary {
        let len = u32::from_be_bytes(written[..4].try_into().unwrap()) as usize;
        plist::from_bytes(&written[4..4 + len]).unwrap()
    }

    fn request_dicts(written: &[u8]) -> Vec<plist::Dictionary> {
        let mut offset = 0;
        let mut requests = Vec::new();
        while offset + 4 <= written.len() {
            let len = u32::from_be_bytes(written[offset..offset + 4].try_into().unwrap()) as usize;
            let end = offset + 4 + len;
            if end > written.len() {
                break;
            }
            requests.push(plist::from_bytes(&written[offset + 4..end]).unwrap());
            offset = end;
        }
        requests
    }

    #[tokio::test]
    async fn list_sends_get_device_registry_command() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "PairedDevicesArray".to_string(),
            plist::Value::Array(vec![plist::Value::String("watch".into())]),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = CompanionProxyClient::new(&mut stream);

        let devices = client.list().await.unwrap();
        assert_eq!(devices.len(), 1);

        let dict = request_dict(&stream.written);
        assert_eq!(dict["Command"].as_string(), Some("GetDeviceRegistry"));
    }

    #[tokio::test]
    async fn get_value_sends_registry_lookup_request() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "RetrievedValueDictionary".to_string(),
            plist::Value::String("AppleWatch".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = CompanionProxyClient::new(&mut stream);

        let value = client.get_value("watch-udid", "name").await.unwrap();
        assert_eq!(value.as_string(), Some("AppleWatch"));

        let dict = request_dict(&stream.written);
        assert_eq!(dict["Command"].as_string(), Some("GetValueFromRegistry"));
        assert_eq!(dict["GetValueGizmoUDIDKey"].as_string(), Some("watch-udid"));
        assert_eq!(dict["GetValueKeyKey"].as_string(), Some("name"));
    }

    #[tokio::test]
    async fn listen_sends_start_and_preserves_unknown_duplicate_events() {
        let events = vec![
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Event".to_string(),
                plist::Value::String("added".into()),
            )])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "FutureEvent".to_string(),
                plist::Value::Integer(7.into()),
            )])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Event".to_string(),
                plist::Value::String("added".into()),
            )])),
        ];
        let expected = events.clone();
        let stream = MockStream::with_responses(events);
        let client = CompanionProxyClient::new(stream);
        let mut listener = client.listen_for_devices().await.unwrap();
        let written = {
            let stream = listener.stream.lock().await;
            stream.written.clone()
        };
        let command = request_dict(&written);
        assert_eq!(
            command["Command"].as_string(),
            Some("StartListeningForDevices")
        );

        let mut received = Vec::new();
        for _ in 0..expected.len() {
            received.push(listener.next_event().await.unwrap().unwrap());
        }
        assert_eq!(
            received,
            expected
                .into_iter()
                .map(|value| value.into_dictionary().unwrap())
                .collect::<Vec<_>>()
        );
    }

    struct ChunkedStream {
        inner: MockStream,
        max_chunk: usize,
    }

    impl AsyncRead for ChunkedStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let amount = self.max_chunk.max(1).min(buf.remaining()).min(8);
            let mut chunk = [0u8; 8];
            let mut chunk_buf = ReadBuf::new(&mut chunk[..amount]);
            match Pin::new(&mut self.inner).poll_read(cx, &mut chunk_buf) {
                Poll::Ready(Ok(())) => {
                    let filled = chunk_buf.filled().len();
                    buf.put_slice(&chunk[..filled]);
                    Poll::Ready(Ok(()))
                }
                other => other,
            }
        }
    }

    impl AsyncWrite for ChunkedStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.inner).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }

    #[tokio::test]
    async fn listener_reassembles_fragmented_length_and_payload_frames() {
        let events = vec![
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Event".to_string(),
                plist::Value::String("fragmented".into()),
            )])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Event".to_string(),
                plist::Value::String("second".into()),
            )])),
        ];
        let expected = events.clone();
        let stream = ChunkedStream {
            inner: MockStream::with_responses(events),
            max_chunk: 1,
        };
        let client = CompanionProxyClient::new(stream);
        let mut listener = client.listen_for_devices().await.unwrap();
        for expected in expected {
            let event = listener.next_event().await.unwrap().unwrap();
            assert_eq!(event, expected.into_dictionary().unwrap());
        }
    }

    #[tokio::test]
    async fn forwarding_request_has_exact_defaults_and_port_is_checked() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "CompanionProxyServicePort".to_string(),
            plist::Value::Integer(65535u16.into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = CompanionProxyClient::new(&mut stream);

        assert_eq!(
            client
                .start_forwarding_service_port(65535, Some("com.example.watch"), None)
                .await
                .unwrap(),
            65535
        );
        let request = request_dict(&stream.written);
        assert_eq!(
            request["Command"].as_string(),
            Some("StartForwardingServicePort")
        );
        assert_eq!(
            request["GizmoRemotePortNumber"].as_unsigned_integer(),
            Some(65535)
        );
        assert_eq!(request["IsServiceLowPriority"].as_boolean(), Some(false));
        assert_eq!(request["PreferWifi"].as_boolean(), Some(false));
        assert_eq!(
            request["ForwardedServiceName"].as_string(),
            Some("com.example.watch")
        );
    }

    #[tokio::test]
    async fn forwarding_options_override_defaults_but_invalid_port_is_rejected() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "CompanionProxyServicePort".to_string(),
            plist::Value::Integer(1234.into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = CompanionProxyClient::new(&mut stream);
        let options =
            plist::Dictionary::from_iter([("PreferWifi".to_string(), plist::Value::Boolean(true))]);
        client
            .start_forwarding_service_port(1, None, Some(options))
            .await
            .unwrap();
        let request = request_dict(&stream.written);
        assert_eq!(request["PreferWifi"].as_boolean(), Some(true));

        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::new()));
        let mut client = CompanionProxyClient::new(&mut stream);
        let options = plist::Dictionary::from_iter([(
            "GizmoRemotePortNumber".to_string(),
            plist::Value::Integer(u64::MAX.into()),
        )]);
        let error = client
            .start_forwarding_service_port(1, None, Some(options))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("u16 port range"));
        assert!(stream.written.is_empty());
    }

    #[tokio::test]
    async fn stop_is_idempotent_and_drop_uses_a_bounded_best_effort_request() {
        let responses = vec![
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "CompanionProxyServicePort".to_string(),
                plist::Value::Integer(4321.into()),
            )])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Command".to_string(),
                plist::Value::String("CommandSuccess".into()),
            )])),
        ];
        let stream = MockStream::with_responses(responses);
        let mut client = CompanionProxyClient::with_timeout(stream, Duration::from_millis(50));
        let mut handle = client
            .start_forwarding_service_port_handle(1234, None, None)
            .await
            .unwrap();
        let first = handle
            .stop_with_timeout(Duration::from_millis(10))
            .await
            .unwrap();
        let second = handle.stop().await.unwrap();
        assert_eq!(first, second);
        assert!(handle.is_stopped());
    }

    #[tokio::test]
    async fn missing_or_bad_responses_and_error_are_diagnostic() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Error".to_string(),
            plist::Value::String("UnsupportedWatchKey".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = CompanionProxyClient::new(&mut stream);
        let error = client.get_value("u", "k").await.unwrap_err();
        assert!(error.to_string().contains("UnsupportedWatchKey"));

        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::new()));
        let mut client = CompanionProxyClient::new(&mut stream);
        let error = client
            .start_forwarding_service_port(1, None, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("CompanionProxyServicePort"));
    }

    #[test]
    fn invalid_ports_and_service_names_are_rejected_without_wire_io() {
        assert!(validate_port(0, "port").is_err());
        assert!(validate_port(1, "port").is_ok());
        assert!(validate_port(u16::MAX, "port").is_ok());
        assert_eq!(
            parse_port(&plist::Value::Integer(u16::MAX.into()), "port").unwrap(),
            u16::MAX
        );
        assert!(parse_port(&plist::Value::Integer((u16::MAX as u64 + 1).into()), "port").is_err());
        assert!(parse_port(&plist::Value::Integer(u64::MAX.into()), "port").is_err());
        assert!(parse_port(&plist::Value::Integer((-1i64).into()), "port").is_err());
        assert!(validate_service_name("").is_err());
        assert!(validate_service_name("a\0b").is_err());
        assert!(validate_service_name(&"x".repeat(1025)).is_err());
        assert!(validate_service_name("看護時計").is_ok());
    }

    struct PendingStream;

    impl AsyncRead for PendingStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for PendingStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn request_timeout_bounds_stalled_transport() {
        let mut client =
            CompanionProxyClient::with_timeout(PendingStream, Duration::from_millis(5));
        let error = client.list().await.unwrap_err();
        assert!(matches!(error, CompanionError::Timeout(_)));
    }

    #[tokio::test]
    async fn stop_error_does_not_mark_handle_stopped() {
        let responses = vec![
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "CompanionProxyServicePort".to_string(),
                plist::Value::Integer(4321.into()),
            )])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Error".to_string(),
                plist::Value::String("no such port".into()),
            )])),
        ];
        let stream = MockStream::with_responses(responses);
        let mut client = CompanionProxyClient::new(stream);
        let mut handle = client
            .start_forwarding_service_port_handle(1234, None, None)
            .await
            .unwrap();
        assert!(handle.stop().await.is_err());
        assert!(!handle.is_stopped());
    }

    #[derive(Clone)]
    struct SharedMockStream(StdArc<StdMutex<MockStream>>);

    impl AsyncRead for SharedMockStream {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let mut stream = self.0.lock().unwrap();
            Pin::new(&mut *stream).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for SharedMockStream {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            let mut stream = self.0.lock().unwrap();
            Pin::new(&mut *stream).poll_write(cx, buf)
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            let mut stream = self.0.lock().unwrap();
            Pin::new(&mut *stream).poll_flush(cx)
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            let mut stream = self.0.lock().unwrap();
            Pin::new(&mut *stream).poll_shutdown(cx)
        }
    }

    #[tokio::test]
    async fn dropping_forwarding_handle_schedules_stop_without_blocking() {
        let responses = vec![
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "CompanionProxyServicePort".to_string(),
                plist::Value::Integer(4321.into()),
            )])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Command".to_string(),
                plist::Value::String("ComandSuccess".into()),
            )])),
        ];
        let inner = StdArc::new(StdMutex::new(MockStream::with_responses(responses)));
        let stream = SharedMockStream(StdArc::clone(&inner));
        let mut client = CompanionProxyClient::with_timeout(stream, Duration::from_millis(50));
        let handle = client
            .start_forwarding_service_port_handle(1234, None, None)
            .await
            .unwrap();
        drop(handle);
        drop(client);

        for _ in 0..20 {
            tokio::task::yield_now().await;
            if inner.lock().unwrap().written.len() > 4 {
                break;
            }
        }
        let requests = request_dicts(&inner.lock().unwrap().written);
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1]["Command"].as_string(),
            Some("StopForwardingServicePort")
        );
    }

    #[tokio::test]
    async fn listener_idle_timeout_is_bounded() {
        let event = plist::Dictionary::from_iter([(
            "Event".to_string(),
            plist::Value::String("ok".into()),
        )]);
        let stream = MockStream::with_raw_frames(vec![dict_frame(plist::Value::Dictionary(event))]);
        let client = CompanionProxyClient::new(stream);
        let mut listener = client.listen_for_devices().await.unwrap();
        assert!(listener
            .next_event_with_timeout(Duration::from_millis(50))
            .await
            .is_ok());

        let mut pending = CompanionDeviceStream {
            stream: Arc::new(Mutex::new(PendingStream)),
            timeout: Duration::from_millis(5),
            pending: None,
            finished: false,
        };
        assert!(matches!(
            pending
                .next_event_with_timeout(Duration::from_millis(5))
                .await,
            Err(CompanionError::Timeout(_))
        ));
        assert!(pending.next_event().await.is_none());
    }
}
