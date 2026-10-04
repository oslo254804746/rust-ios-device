use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{error::Elapsed, Instant};

use super::response_error::ResponseErrorPolicy;
use crate::lockdown::pair_record::PairRecord;
use crate::lockdown::protocol::*;
use crate::lockdown::session::start_lockdown_session;
use crate::lockdown::{LockdownError, ServiceInfo};

/// High-level Lockdown client. Handles session management and service starting.
pub struct LockdownClient {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    session_id: Option<String>,
    state: LockdownState,
}

struct LockdownState {
    usable: bool,
    request_timeout: Option<Duration>,
    progressed: bool,
    failed: bool,
}

impl Default for LockdownState {
    fn default() -> Self {
        Self {
            usable: true,
            request_timeout: None,
            progressed: false,
            failed: false,
        }
    }
}

// The client may own buffered TLS halves. Progress here includes bytes taken
// from or accepted into those buffers, independently of new transport I/O.
struct LockdownIo<'a, T> {
    inner: &'a mut T,
    state: &'a mut LockdownState,
}

impl<T: AsyncRead + Unpin> AsyncRead for LockdownIo<'_, T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let requested = buf.remaining() > 0;
        let result = Pin::new(&mut *self.inner).poll_read(cx, buf);
        self.state.progressed |= buf.filled().len() > before;
        self.state.failed |= matches!(result, Poll::Ready(Err(_)))
            || (requested && matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() == before);
        result
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for LockdownIo<'_, T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut *self.inner).poll_write(cx, bytes);
        self.state.progressed |= matches!(result, Poll::Ready(Ok(n)) if n > 0);
        self.state.failed |= matches!(result, Poll::Ready(Err(_)))
            || (!bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))));
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let result = Pin::new(&mut *self.inner).poll_flush(cx);
        self.state.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let result = Pin::new(&mut *self.inner).poll_shutdown(cx);
        self.state.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

struct LockdownOperation<'a> {
    client: &'a mut LockdownClient,
    deadline: Option<Instant>,
    completed: bool,
}

impl LockdownOperation<'_> {
    fn finish<T>(
        mut self,
        result: Result<Result<T, LockdownError>, Elapsed>,
    ) -> Result<T, LockdownError> {
        match result {
            Ok(result) => {
                self.completed = true;
                if self.client.state.failed || (result.is_err() && self.client.state.progressed) {
                    self.client.state.usable = false;
                }
                result
            }
            Err(elapsed) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed).into()),
        }
    }
}

impl Drop for LockdownOperation<'_> {
    fn drop(&mut self) {
        if !self.completed && (self.client.state.progressed || self.client.state.failed) {
            self.client.state.usable = false;
        }
    }
}

fn await_lockdown_operation<T>(
    deadline: Option<Instant>,
    operation: impl std::future::Future<Output = Result<T, LockdownError>>,
) -> impl std::future::Future<Output = Result<Result<T, LockdownError>, Elapsed>> {
    let operation = Box::pin(operation);
    async move {
        match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, operation).await,
            None => Ok(operation.await),
        }
    }
}

impl LockdownClient {
    /// Local framing/request ownership permits another request. This does not
    /// prove peer liveness, remote cleanup or safe retry after a side effect.
    pub fn is_usable(&self) -> bool {
        self.state.usable
    }

    /// Share one absolute per-request budget across encoding, send, flush and
    /// reply decoding. None keeps legacy unbounded request timing. Synchronous
    /// encoding/decoding is cooperative; it cannot be preempted by this timer.
    pub fn set_request_timeout(&mut self, budget: Option<Duration>) {
        self.state.request_timeout = budget;
    }

    fn begin_operation(
        &mut self,
        budget: Option<Duration>,
    ) -> Result<LockdownOperation<'_>, LockdownError> {
        if !self.is_usable() {
            return Err(LockdownError::Protocol(
                "lockdown connection is unusable; reconnect before another request".into(),
            ));
        }
        let deadline = match budget {
            Some(budget) if budget.is_zero() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "lockdown request budget is zero",
                )
                .into());
            }
            Some(budget) => Some(Instant::now().checked_add(budget).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "lockdown request deadline is too large",
                )
            })?),
            None => None,
        };
        self.state.progressed = false;
        self.state.failed = false;
        Ok(LockdownOperation {
            client: self,
            deadline,
            completed: false,
        })
    }

    async fn request<T: Serialize, R: DeserializeOwned>(
        &mut self,
        request: &T,
    ) -> Result<R, LockdownError> {
        let operation = self.begin_operation(self.state.request_timeout)?;
        let result = await_lockdown_operation(operation.deadline, async {
            let client = &mut *operation.client;
            send_lockdown(
                &mut LockdownIo {
                    inner: &mut client.writer,
                    state: &mut client.state,
                },
                request,
            )
            .await?;
            recv_lockdown(&mut LockdownIo {
                inner: &mut client.reader,
                state: &mut client.state,
            })
            .await
        })
        .await;
        operation.finish(result)
    }

    /// Create a LockdownClient from an already-connected usbmux stream, performing TLS handshake.
    pub async fn connect_with_stream<S>(
        stream: S,
        pair_record: &PairRecord,
    ) -> Result<Self, LockdownError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (session_id, reader, writer) = start_lockdown_session(stream, pair_record).await?;
        Ok(Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            session_id: Some(session_id),
            state: LockdownState::default(),
        })
    }

    /// Get a value from lockdown.
    pub async fn get_value(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<plist::Value, LockdownError> {
        self.get_value_with_policy(domain, key, ResponseErrorPolicy::Legacy)
            .await
    }

    /// Get a value while retaining device Error fields as LockdownResponseError.
    /// Requests and successful values retain the legacy policy. A present
    /// non-string Error field is rejected as malformed received protocol data.
    pub async fn get_value_preserving_errors(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<plist::Value, LockdownError> {
        self.get_value_with_policy(domain, key, ResponseErrorPolicy::Preserve)
            .await
    }

    async fn get_value_with_policy(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
        policy: ResponseErrorPolicy,
    ) -> Result<plist::Value, LockdownError> {
        let resp: plist::Value = self
            .request(&GetValueRequest {
                label: "ios-rs",
                request: "GetValue",
                domain,
                key,
            })
            .await?;
        match policy {
            ResponseErrorPolicy::Legacy => extract_get_value(resp, domain, key),
            ResponseErrorPolicy::Preserve => {
                extract_get_value_with_policy(resp, domain, key, policy)
            }
        }
    }

    /// Set a lockdown value.
    pub async fn set_value<T>(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
        value: T,
    ) -> Result<(), LockdownError>
    where
        T: Serialize,
    {
        let resp: ValueOperationResponse = self
            .request(&SetValueRequest {
                label: "ios-rs",
                request: "SetValue",
                domain,
                key,
                value,
            })
            .await?;
        if let Some(err) = resp.error {
            return Err(LockdownError::Protocol(format!(
                "SetValue failed for domain={domain:?} key={key:?}: {err}"
            )));
        }
        Ok(())
    }

    /// Remove a lockdown value.
    pub async fn remove_value(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<(), LockdownError> {
        let resp: ValueOperationResponse = self
            .request(&RemoveValueRequest {
                label: "ios-rs",
                request: "RemoveValue",
                domain,
                key,
            })
            .await?;
        if let Some(err) = resp.error {
            return Err(LockdownError::Protocol(format!(
                "RemoveValue failed for domain={domain:?} key={key:?}: {err}"
            )));
        }
        Ok(())
    }

    /// Apply a legacy activation record through lockdown.
    ///
    /// iOS versions predating the session-based mobileactivationd flow expect
    /// the server's `activation-record` value under an `Activate` request.
    /// This remains separate from `HandleActivationInfoWithSessionRequest` so
    /// callers cannot accidentally send one protocol's envelope to the other.
    pub async fn activate(&mut self, activation_record: plist::Value) -> Result<(), LockdownError> {
        self.simple_request("Activate", Some(("ActivationRecord", activation_record)))
            .await
    }

    /// Deactivate through the legacy lockdown protocol.
    pub async fn deactivate(&mut self) -> Result<(), LockdownError> {
        self.simple_request("Deactivate", None).await
    }

    async fn simple_request(
        &mut self,
        request_name: &str,
        field: Option<(&str, plist::Value)>,
    ) -> Result<(), LockdownError> {
        let mut request = plist::Dictionary::from_iter([
            ("Label".to_string(), plist::Value::String("ios-rs".into())),
            (
                "Request".to_string(),
                plist::Value::String(request_name.to_owned()),
            ),
        ]);
        if let Some((key, value)) = field {
            request.insert(key.to_owned(), value);
        }
        let response: plist::Value = self.request(&plist::Value::Dictionary(request)).await?;
        let response = response.as_dictionary().ok_or_else(|| {
            LockdownError::Protocol(format!("{request_name} returned a non-dictionary response"))
        })?;
        match response.get("Request").and_then(plist::Value::as_string) {
            Some(request) if request == request_name => {}
            _ => {
                return Err(LockdownError::Protocol(format!(
                    "{request_name} returned an unexpected response request"
                )))
            }
        }
        if let Some(error) = response.get("Error") {
            return Err(LockdownError::Protocol(format!(
                "{request_name} failed: {}",
                redact_request_error(error)
            )));
        }
        if let Some(status) = response.get("Status").and_then(plist::Value::as_string) {
            if matches!(
                status.to_ascii_lowercase().as_str(),
                "error" | "failed" | "failure" | "rejected"
            ) {
                return Err(LockdownError::Protocol(format!(
                    "{request_name} failed with Status={status}"
                )));
            }
        }
        if let Some(chain) = response.get("ErrorChain") {
            let count = chain.as_array().map_or(1, std::vec::Vec::len);
            if count > 0 {
                return Err(LockdownError::Protocol(format!(
                    "{request_name} failed with ErrorChain entries={count}"
                )));
            }
        }
        // Older lockdown versions report failure under `Result` rather than
        // `Error`; accepting that response would falsely acknowledge Activate
        // or Deactivate and then persist a misleading local state marker.
        if response
            .get("Result")
            .and_then(plist::Value::as_string)
            .is_some_and(|result| result == "Failure")
        {
            return Err(LockdownError::Protocol(format!(
                "{request_name} failed with Result=Failure"
            )));
        }
        Ok(())
    }

    /// Start a service and return its port information.
    pub async fn start_service(&mut self, service: &str) -> Result<ServiceInfo, LockdownError> {
        let resp: StartServiceResponse = self
            .request(&StartServiceRequest {
                label: "ios-rs",
                request: "StartService",
                service: service.to_string(),
            })
            .await?;
        if let Some(err) = resp.error {
            return Err(LockdownError::Protocol(format!(
                "StartService '{service}' failed: {err}"
            )));
        }
        let port = resp.port.ok_or_else(|| {
            LockdownError::Protocol(format!("StartService '{service}': missing Port field"))
        })?;
        Ok(ServiceInfo {
            port,
            enable_service_ssl: resp.enable_service_ssl.unwrap_or(false),
        })
    }

    /// Stop the current session with a separate five-second write/flush budget,
    /// clamped by a shorter configured request timeout. Success marks the client
    /// closed and repeated stops return Ok without I/O; it is not a remote ACK.
    /// Cancellation/failure after accepted bytes makes the client unusable.
    pub async fn stop_session(&mut self) -> Result<(), LockdownError> {
        if self.session_id.is_none() {
            return Ok(());
        }
        let budget = self
            .state
            .request_timeout
            .map_or(Duration::from_secs(5), |budget| {
                budget.min(Duration::from_secs(5))
            });
        let operation = self.begin_operation(Some(budget))?;
        let result = await_lockdown_operation(operation.deadline, async {
            let client = &mut *operation.client;
            send_lockdown(
                &mut LockdownIo {
                    inner: &mut client.writer,
                    state: &mut client.state,
                },
                &StopSessionRequest {
                    label: "ios-rs",
                    request: "StopSession",
                    session_id: client.session_id.clone().expect("checked active session"),
                },
            )
            .await?;
            client.session_id = None;
            client.state.usable = false;
            Ok(())
        })
        .await;
        operation.finish(result)
    }

    /// Get the device product version string.
    pub async fn product_version(&mut self) -> Result<semver::Version, LockdownError> {
        let val = self.get_value(None, Some("ProductVersion")).await?;
        let s = val
            .as_string()
            .ok_or_else(|| LockdownError::Protocol("ProductVersion is not a string".into()))?;
        // iOS may return "15.5" (two-part); semver requires three parts
        let normalized = match s.matches('.').count() {
            0 => format!("{s}.0.0"),
            1 => format!("{s}.0"),
            _ => s.to_string(),
        };
        semver::Version::parse(&normalized).map_err(|error| {
            super::error_context::native(
                std::io::ErrorKind::InvalidData,
                "parse lockdown product version",
                error,
            )
        })
    }
}

fn redact_request_error(value: &plist::Value) -> String {
    match value {
        plist::Value::String(text) => format!("<redacted error: {} chars>", text.chars().count()),
        plist::Value::Data(data) => format!("<redacted data: {} bytes>", data.len()),
        plist::Value::Integer(_) | plist::Value::Real(_) | plist::Value::Boolean(_) => {
            format!("{value:?}")
        }
        _ => "<redacted structured error>".into(),
    }
}

fn extract_get_value(
    response: plist::Value,
    domain: Option<&str>,
    key: Option<&str>,
) -> Result<plist::Value, LockdownError> {
    extract_get_value_with_policy(response, domain, key, ResponseErrorPolicy::Legacy)
}

fn extract_get_value_with_policy(
    response: plist::Value,
    domain: Option<&str>,
    key: Option<&str>,
    policy: ResponseErrorPolicy,
) -> Result<plist::Value, LockdownError> {
    if let plist::Value::Dictionary(mut values) = response {
        if let Some(error) = values.remove("Error") {
            match error {
                plist::Value::String(error) => {
                    return Err(policy.remote_error("GetValue", error, |error| {
                        format!("GetValue failed for domain={domain:?} key={key:?}: {error}")
                    }))
                }
                _ if matches!(policy, ResponseErrorPolicy::Preserve) => {
                    return Err(LockdownError::Protocol(
                        "GetValue Error field is not a string".into(),
                    ))
                }
                _ => {}
            }
        }

        if let Some(value) = values.remove("Value") {
            return Ok(value);
        }

        return Err(LockdownError::Protocol(format!(
            "GetValue missing Value for domain={domain:?} key={key:?}: {:?}",
            plist::Value::Dictionary(values)
        )));
    }

    Err(LockdownError::Protocol(format!(
        "GetValue returned non-dictionary response for domain={domain:?} key={key:?}: {response:?}"
    )))
}

#[cfg(test)]
mod tests {
    use crate::test_util::MockStream;
    use std::error::Error;

    use super::*;

    fn lifecycle_lockdown_client(capacity: usize) -> (LockdownClient, tokio::io::DuplexStream) {
        let (stream, peer) = tokio::io::duplex(capacity);
        let (reader, writer) = tokio::io::split(stream);
        (
            LockdownClient {
                reader: Box::new(reader),
                writer: Box::new(writer),
                session_id: Some("owned-session".into()),
                state: LockdownState::default(),
            },
            peer,
        )
    }

    fn lifecycle_lockdown_assert_elapsed(error: LockdownError) {
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        let LockdownError::Io(io) = error else {
            panic!("native expiry lost")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
        assert!(io.get_ref().unwrap().is::<Elapsed>());
    }

    async fn lifecycle_lockdown_assert_rejected(client: &mut LockdownClient) {
        assert!(!client.is_usable());
        for result in [
            client.get_value(None, None).await.map(|_| ()),
            client
                .get_value_preserving_errors(None, None)
                .await
                .map(|_| ()),
            client.set_value(None, None, false).await,
            client.remove_value(None, None).await,
            client.start_service("owned-service").await.map(|_| ()),
            client
                .activate(plist::Value::Dictionary(plist::Dictionary::new()))
                .await,
            client.deactivate().await,
            client.product_version().await.map(|_| ()),
            client.stop_session().await,
        ] {
            assert_eq!(
                crate::classify_error(&result.unwrap_err()),
                crate::ErrorClass::Protocol
            );
        }
    }

    #[tokio::test]
    async fn lifecycle_lockdown_zero_overflow_unpolled_and_local_encode_error_preserve_session() {
        use tokio::io::AsyncReadExt;
        struct Refused;
        impl Serialize for Refused {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom(
                    "owned-private serialization refused",
                ))
            }
        }
        let (mut client, mut peer) = lifecycle_lockdown_client(4096);
        client.set_request_timeout(Some(Duration::ZERO));
        for result in [
            client.get_value(None, None).await.map(|_| ()),
            client
                .get_value_preserving_errors(None, None)
                .await
                .map(|_| ()),
            client.set_value(None, None, Refused).await,
            client.remove_value(None, None).await,
            client.start_service("owned-service").await.map(|_| ()),
            client.activate(plist::Value::Boolean(false)).await,
            client.deactivate().await,
            client.stop_session().await,
        ] {
            let LockdownError::Io(error) = result.unwrap_err() else {
                panic!("zero budget changed family")
            };
            assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
            assert!(error.source().is_none());
        }
        client.set_request_timeout(Some(Duration::MAX));
        let LockdownError::Io(error) = client.get_value(None, None).await.unwrap_err() else {
            panic!("overflow changed family")
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        client.set_request_timeout(None);
        drop(client.get_value(None, None));
        drop(client.stop_session());
        let error = client.set_value(None, None, Refused).await.unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::InvalidInput
        );
        assert!(crate::test_native::has_source::<plist::Error>(&error));
        assert!(client.is_usable());
        assert_eq!(client.session_id.as_deref(), Some("owned-session"));
        assert!(
            tokio::time::timeout(Duration::from_millis(5), peer.read(&mut [0]))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn lifecycle_lockdown_sent_cancel_expiry_and_fragment_reject_late_reply() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for mode in 0..3 {
            tokio::time::timeout(Duration::from_secs(1), async {
                let (mut client, mut peer) = lifecycle_lockdown_client(4096);
                if mode == 1 {
                    client.set_request_timeout(Some(Duration::from_millis(5)));
                }
                let mut request =
                    Box::pin(client.get_value_preserving_errors(None, Some("OwnedKey")));
                assert!(futures_util::poll!(&mut request).is_pending());
                let wire: plist::Value = recv_lockdown(&mut peer).await.unwrap();
                assert_eq!(
                    wire.as_dictionary().unwrap()["Request"].as_string(),
                    Some("GetValue")
                );
                let mut reply = Vec::new();
                send_lockdown(&mut reply, &plist::Dictionary::from_iter([("Value", true)]))
                    .await
                    .unwrap();
                if mode == 2 {
                    peer.write_all(&reply[..6]).await.unwrap();
                    assert!(futures_util::poll!(&mut request).is_pending());
                }
                if mode == 1 {
                    lifecycle_lockdown_assert_elapsed(request.await.unwrap_err());
                } else {
                    drop(request);
                }
                peer.write_all(&reply[if mode == 2 { 6 } else { 0 }..])
                    .await
                    .unwrap();
                lifecycle_lockdown_assert_rejected(&mut client).await;
                assert_eq!(client.session_id.as_deref(), Some("owned-session"));
                assert!(
                    tokio::time::timeout(Duration::from_millis(5), peer.read(&mut [0]))
                        .await
                        .is_err()
                );
                drop(client);
                assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn lifecycle_lockdown_shared_budget_includes_blocked_send_and_receive() {
        use tokio::io::AsyncReadExt;
        tokio::time::timeout(Duration::from_secs(1), async {
            let (mut client, mut peer) = lifecycle_lockdown_client(512);
            client.set_request_timeout(Some(Duration::from_millis(200)));
            let (error, ()) = tokio::join!(
                async {
                    client
                        .set_value(None, Some("OwnedKey"), "x".repeat(4096))
                        .await
                        .unwrap_err()
                },
                async {
                    tokio::time::sleep(Duration::from_millis(125)).await;
                    let request: plist::Value = recv_lockdown(&mut peer).await.unwrap();
                    assert_eq!(
                        request.as_dictionary().unwrap()["Request"].as_string(),
                        Some("SetValue")
                    );
                    tokio::time::sleep(Duration::from_millis(125)).await;
                    let reply = plist::Dictionary::from_iter([("Request", "SetValue")]);
                    let mut bytes = Vec::new();
                    send_lockdown(&mut bytes, &reply).await.unwrap();
                    // The complete reply fits even after the client expires.
                    // A separately restarted 200-ms receive budget would
                    // accept it; the shared deadline must expire first.
                    use tokio::io::AsyncWriteExt;
                    assert!(bytes.len() <= 512);
                    peer.write_all(&bytes).await.unwrap();
                }
            );
            lifecycle_lockdown_assert_elapsed(error);
            assert!(!client.is_usable());
            drop(client);
            assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lifecycle_lockdown_empty_send_wait_preserves_reuse_but_accepted_prefix_invalidates() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        tokio::time::timeout(Duration::from_secs(1), async {
            let (mut client, mut peer) = lifecycle_lockdown_client(64);
            // Fill this direction before the guarded request. Its empty write
            // wait consumes no request bytes and must remain reusable.
            client.writer.write_all(&[0xA5; 64]).await.unwrap();
            client.set_request_timeout(Some(Duration::from_millis(5)));
            lifecycle_lockdown_assert_elapsed(client.get_value(None, None).await.unwrap_err());
            assert!(client.is_usable());
            let mut filled = [0; 64];
            peer.read_exact(&mut filled).await.unwrap();
            assert_eq!(filled, [0xA5; 64]);
            // An actual accepted request prefix now changes ownership.
            lifecycle_lockdown_assert_elapsed(client.get_value(None, None).await.unwrap_err());
            assert!(!client.is_usable());
            drop(client);
            let mut prefix = Vec::new();
            peer.read_to_end(&mut prefix).await.unwrap();
            assert_eq!(prefix.len(), 64);
            assert!(u32::from_be_bytes(prefix[..4].try_into().unwrap()) > 60);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lifecycle_lockdown_complete_business_reply_reuses_but_native_eof_and_decode_error_do_not(
    ) {
        use tokio::io::AsyncWriteExt;
        tokio::time::timeout(Duration::from_secs(1), async {
            let (mut client, mut peer) = lifecycle_lockdown_client(4096);
            let ((), ()) = tokio::join!(
                async {
                    let error = client
                        .get_value_preserving_errors(None, None)
                        .await
                        .unwrap_err();
                    assert_eq!(crate::classify_error(&error), crate::ErrorClass::NotFound);
                    assert!(client.is_usable());
                    let error = client.start_service("owned-service").await.unwrap_err();
                    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
                    assert!(client.is_usable());
                    assert_eq!(
                        client.get_value(None, None).await.unwrap(),
                        plist::Value::Boolean(true)
                    );
                    assert!(client.is_usable());
                },
                async {
                    for reply in [
                        plist::Value::Dictionary(plist::Dictionary::from_iter([(
                            "Error",
                            "MissingValue",
                        )])),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([(
                            "Error",
                            "InvalidService",
                        )])),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([("Value", true)])),
                    ] {
                        let _: plist::Value = recv_lockdown(&mut peer).await.unwrap();
                        send_lockdown(&mut peer, &reply).await.unwrap();
                    }
                }
            );
        })
        .await
        .unwrap();
        for mode in 0..3 {
            tokio::time::timeout(Duration::from_secs(1), async {
                let (mut client, mut peer) = lifecycle_lockdown_client(4096);
                let (error, ()) = tokio::join!(
                    async { client.get_value(None, None).await.unwrap_err() },
                    async {
                        let _: plist::Value = recv_lockdown(&mut peer).await.unwrap();
                        if mode == 1 {
                            peer.write_all(&encode_frame(b"<plist><dict>"))
                                .await
                                .unwrap();
                        } else if mode == 2 {
                            peer.write_all(&(4 * 1024 * 1024 + 1u32).to_be_bytes())
                                .await
                                .unwrap();
                        }
                        drop(peer);
                    }
                );
                if mode == 0 {
                    let LockdownError::Io(io) = error else {
                        panic!("EOF native source lost")
                    };
                    assert_eq!(io.kind(), std::io::ErrorKind::UnexpectedEof);
                } else {
                    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
                    if mode == 1 {
                        assert!(crate::test_native::has_source::<plist::Error>(&error));
                    }
                }
                lifecycle_lockdown_assert_rejected(&mut client).await;
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn lifecycle_lockdown_buffered_reader_consumption_cancellation_invalidates() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        tokio::time::timeout(Duration::from_secs(1), async {
            let (host, mut peer) = tokio::io::duplex(4096);
            let (reader, writer) = tokio::io::split(host);
            let mut reader = tokio::io::BufReader::new(reader);
            let mut reply = Vec::new();
            send_lockdown(&mut reply, &plist::Dictionary::from_iter([("Value", true)]))
                .await
                .unwrap();
            peer.write_all(&[b"z".as_slice(), &reply[..4]].concat())
                .await
                .unwrap();
            reader.read_exact(&mut [0]).await.unwrap();
            assert_eq!(reader.buffer(), &reply[..4]);
            let mut client = LockdownClient {
                reader: Box::new(reader),
                writer: Box::new(writer),
                session_id: Some("owned-session".into()),
                state: LockdownState::default(),
            };
            let mut request = Box::pin(client.get_value(None, None));
            assert!(futures_util::poll!(&mut request).is_pending());
            let _: plist::Value = recv_lockdown(&mut peer).await.unwrap();
            drop(request);
            assert!(!client.is_usable());
            peer.write_all(&reply[4..]).await.unwrap();
            lifecycle_lockdown_assert_rejected(&mut client).await;
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lifecycle_lockdown_stop_success_closes_once_and_failure_keeps_session_unknown() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        tokio::time::timeout(Duration::from_secs(1), async {
            let (mut client, mut peer) = lifecycle_lockdown_client(4096);
            client.stop_session().await.unwrap();
            assert!(!client.is_usable());
            assert!(client.session_id.is_none());
            let request: plist::Value = recv_lockdown(&mut peer).await.unwrap();
            let request = request.as_dictionary().unwrap();
            assert_eq!(request["Request"].as_string(), Some("StopSession"));
            assert_eq!(request["SessionID"].as_string(), Some("owned-session"));
            client.stop_session().await.unwrap();
            assert!(client.get_value(None, None).await.is_err());
            assert!(
                tokio::time::timeout(Duration::from_millis(5), peer.read(&mut [0]))
                    .await
                    .is_err()
            );
            drop(client);
            assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
        })
        .await
        .unwrap();
        for mode in 0..4 {
            tokio::time::timeout(Duration::from_secs(1), async {
                let (mut client, peer) = lifecycle_lockdown_client(64);
                if mode == 0 {
                    drop(peer);
                    let error = client.stop_session().await.unwrap_err();
                    let LockdownError::Io(io) = error else {
                        panic!("write cause lost")
                    };
                    assert_eq!(io.kind(), std::io::ErrorKind::BrokenPipe);
                } else if mode == 3 {
                    let mut stop = Box::pin(client.stop_session());
                    assert!(futures_util::poll!(&mut stop).is_pending());
                    drop(stop);
                } else {
                    client.set_request_timeout(Some(Duration::from_millis(5)));
                    if mode == 2 {
                        // No new request byte is accepted; expiry can be retried.
                        client.writer.write_all(&[0xA5; 64]).await.unwrap();
                    }
                    lifecycle_lockdown_assert_elapsed(client.stop_session().await.unwrap_err());
                }
                assert_eq!(client.session_id.as_deref(), Some("owned-session"));
                assert_eq!(client.is_usable(), mode == 2);
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn lifecycle_lockdown_default_cleanup_budget_expires_independently() {
        tokio::time::timeout(Duration::from_secs(6), async {
            let (mut client, _peer) = lifecycle_lockdown_client(64);
            assert!(client.state.request_timeout.is_none());
            lifecycle_lockdown_assert_elapsed(client.stop_session().await.unwrap_err());
            assert!(!client.is_usable());
            assert_eq!(client.session_id.as_deref(), Some("owned-session"));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn response_codes_get_value_preserves_wire_code_and_legacy_policy() {
        use crate::{classify_error, ErrorClass, LockdownResponseError};
        for code in [
            "MissingValue",
            "InvalidService",
            "owned-private-MissingValue",
            "",
        ] {
            let mut requests = Vec::new();
            for preserve in [false, true] {
                let (host, mut peer) = tokio::io::duplex(4096);
                let (reader, writer) = tokio::io::split(host);
                let mut client = LockdownClient {
                    reader: Box::new(reader),
                    writer: Box::new(writer),
                    session_id: None,
                    state: LockdownState::default(),
                };
                let caller = async {
                    let error = if preserve {
                        client
                            .get_value_preserving_errors(Some("owned-domain"), Some("owned-key"))
                            .await
                    } else {
                        client
                            .get_value(Some("owned-domain"), Some("owned-key"))
                            .await
                    }
                    .unwrap_err();
                    drop(client);
                    error
                };
                let server = async {
                    let request = recv_frame(&mut peer).await.unwrap();
                    let reply = plist::Dictionary::from_iter([
                        ("Error".to_string(), plist::Value::String(code.into())),
                        ("Value".to_string(), plist::Value::Boolean(true)),
                    ]);
                    send_lockdown(&mut peer, &reply).await.unwrap();
                    request
                };
                let (error, request) =
                    tokio::time::timeout(std::time::Duration::from_secs(1), async {
                        tokio::join!(caller, server)
                    })
                    .await
                    .unwrap();
                requests.push(request);
                if preserve {
                    let observed = LockdownResponseError::in_error_chain(&error).unwrap();
                    assert_eq!(observed.operation(), "GetValue");
                    assert_eq!(observed.code(), code);
                    assert_eq!(
                        classify_error(&error),
                        if code == "MissingValue" {
                            ErrorClass::NotFound
                        } else {
                            ErrorClass::ServiceFailure
                        }
                    );
                    assert!(observed.source().is_none());
                    assert!(!format!("{error:?} {error}").contains("owned-private"));
                } else {
                    assert!(LockdownResponseError::in_error_chain(&error).is_none());
                    assert_eq!(classify_error(&error), ErrorClass::Protocol);
                    let LockdownError::Protocol(text) = error else {
                        panic!("legacy variant changed")
                    };
                    assert_eq!(text, format!("GetValue failed for domain=Some(\"owned-domain\") key=Some(\"owned-key\"): {code}"));
                }
            }
            assert_eq!(requests[0], requests[1]);
            let request: plist::Value = plist::from_bytes(&requests[0]).unwrap();
            let request = request.as_dictionary().unwrap();
            assert_eq!(request["Request"].as_string(), Some("GetValue"));
            assert_eq!(request["Label"].as_string(), Some("ios-rs"));
            assert_eq!(request["Domain"].as_string(), Some("owned-domain"));
            assert_eq!(request["Key"].as_string(), Some("owned-key"));
        }
    }

    #[tokio::test]
    async fn response_codes_get_value_retains_success_shapes_and_native_io() {
        use crate::{classify_error, ErrorClass, LockdownResponseError};
        for reply in [
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Value".to_owned(),
                plist::Value::Boolean(false),
            )])),
            plist::Value::Dictionary(plist::Dictionary::new()),
            plist::Value::Array(vec![]),
            // Legacy accepts the Value; preserving refuses this malformed Error.
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("Error".to_owned(), plist::Value::Integer(7.into())),
                ("Value".to_owned(), plist::Value::Boolean(true)),
            ])),
        ] {
            let mut results = Vec::new();
            for preserve in [false, true] {
                let mut client = LockdownClient {
                    reader: Box::new(MockStream::with_response(reply.clone())),
                    writer: Box::new(MockStream::eof()),
                    session_id: None,
                    state: LockdownState::default(),
                };
                let result = if preserve {
                    client.get_value_preserving_errors(None, None).await
                } else {
                    client.get_value(None, None).await
                };
                match result {
                    Ok(value) => results.push(Ok(value)),
                    Err(error) => {
                        assert_eq!(classify_error(&error), ErrorClass::Protocol);
                        assert!(LockdownResponseError::in_error_chain(&error).is_none());
                        results.push(Err(error.to_string()));
                    }
                }
            }
            if reply
                .as_dictionary()
                .is_some_and(|reply| reply.contains_key("Error"))
            {
                assert_eq!(results[0], Ok(plist::Value::Boolean(true)));
                assert_eq!(
                    results[1],
                    Err("protocol error: GetValue Error field is not a string".to_owned())
                );
            } else {
                assert_eq!(results[0], results[1]);
            }
        }
        let (host, mut peer) = tokio::io::duplex(4096);
        let (reader, writer) = tokio::io::split(host);
        let mut client = LockdownClient {
            reader: Box::new(reader),
            writer: Box::new(writer),
            session_id: None,
            state: LockdownState::default(),
        };
        let caller = async {
            client
                .get_value_preserving_errors(None, None)
                .await
                .unwrap_err()
        };
        let server = async {
            let _: plist::Value = recv_lockdown(&mut peer).await.unwrap();
            drop(peer);
        };
        let (error, ()) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(caller, server)
        })
        .await
        .unwrap();
        assert_eq!(classify_error(&error), ErrorClass::Io);
        let LockdownError::Io(io) = &error else {
            panic!("native IO changed")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(LockdownResponseError::in_error_chain(&error).is_none());
    }

    #[tokio::test]
    async fn actual_product_version_parse_retains_semver_source_and_normalization() {
        use crate::{classify_error, ErrorClass};
        for (version, expected) in [
            ("17", Some("17.0.0")),
            ("17.5", Some("17.5.0")),
            ("17.5.1", Some("17.5.1")),
            ("owned-private-invalid", None),
        ] {
            let response =
                plist::Value::Dictionary(plist::Dictionary::from_iter([("Value", version)]));
            let mut client = LockdownClient {
                reader: Box::new(MockStream::with_response(response)),
                writer: Box::new(MockStream::eof()),
                session_id: None,
                state: LockdownState::default(),
            };
            let result = client.product_version().await;
            if let Some(expected) = expected {
                assert_eq!(result.unwrap().to_string(), expected);
            } else {
                let error = result.unwrap_err();
                let LockdownError::Io(io) = &error else {
                    panic!("semver source flattened");
                };
                assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);
                assert_eq!(classify_error(&error), ErrorClass::Protocol);
                assert!(io.source().unwrap().is::<semver::Error>());
                assert!(!format!("{error:?} {:?}", io.get_ref()).contains("owned-private"));
            }
        }
    }

    #[test]
    fn missing_get_value_payload_reports_context() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Success".into()),
        )]));

        let err = extract_get_value(
            response,
            Some("com.apple.mobile.wireless_lockdown"),
            Some("EnableWifiConnections"),
        )
        .expect_err("missing value should error");

        let rendered = err.to_string();
        assert!(rendered.contains("EnableWifiConnections"));
        assert!(rendered.contains("com.apple.mobile.wireless_lockdown"));
        assert!(rendered.contains("Status"));
    }

    #[tokio::test]
    async fn legacy_activation_rejects_result_failure() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Request".to_string(),
                plist::Value::String("Activate".into()),
            ),
            ("Result".to_string(), plist::Value::String("Failure".into())),
        ]));
        let reader = MockStream::with_response(response);
        let writer = MockStream::eof();
        let mut client = LockdownClient {
            reader: Box::new(reader),
            writer: Box::new(writer),
            session_id: None,
            state: LockdownState::default(),
        };

        let error = client
            .activate(plist::Value::Dictionary(plist::Dictionary::new()))
            .await
            .expect_err("Result=Failure must not be treated as success");
        assert!(error.to_string().contains("Result=Failure"));
    }

    #[tokio::test]
    async fn legacy_activation_rejects_unrelated_response_request() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Request".to_string(),
            plist::Value::String("GetValue".into()),
        )]));
        let reader = MockStream::with_response(response);
        let writer = MockStream::eof();
        let mut client = LockdownClient {
            reader: Box::new(reader),
            writer: Box::new(writer),
            session_id: None,
            state: LockdownState::default(),
        };

        let error = client
            .deactivate()
            .await
            .expect_err("a response for another request must be rejected");
        assert!(error.to_string().contains("unexpected response request"));
    }
}
