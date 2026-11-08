//! XPC client: high-level wrapper around XpcConnection for service calls.

use std::net::{Ipv6Addr, SocketAddr};
use std::{future::Future, pin::Pin, sync::Arc};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::xpc::error_mapping::{h2_error, xpc_timeout};
use crate::xpc::h2_raw::H2Framer;
use crate::xpc::message::{flags, XpcMessage, XpcValue};
use crate::xpc::rsd::{initialize_xpc_connection_on_framer, XpcConnection};
use crate::xpc::XpcError;

trait XpcStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> XpcStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type DynStream = Box<dyn XpcStream>;
type ReconnectFuture = Pin<Box<dyn Future<Output = Result<XpcClient, XpcError>> + Send>>;
type Reconnector = Arc<dyn Fn() -> ReconnectFuture + Send + Sync>;

/// High-level XPC client for iOS 17+ services.
pub struct XpcClient {
    inner: XpcConnection<DynStream>,
    reconnector: Option<Reconnector>,
}

impl XpcClient {
    /// Local framing and request ownership permit another operation. This
    /// does not acknowledge remote cleanup or establish safe retry.
    pub fn is_usable(&self) -> bool {
        self.inner.is_usable()
    }

    /// Connect to an XPC service at the given IPv6 address and port.
    pub async fn connect(addr: Ipv6Addr, port: u16) -> Result<Self, XpcError> {
        let sock_addr = SocketAddr::new(addr.into(), port);
        let stream = xpc_timeout(
            crate::tunnel::TUNNEL_CONNECT_TIMEOUT,
            "dial XPC service",
            async { Ok(TcpStream::connect(sock_addr).await?) },
        )
        .await?;
        let client = xpc_timeout(
            crate::tunnel::TUNNEL_CONNECT_TIMEOUT,
            "initialize XPC service",
            Self::connect_stream(stream),
        )
        .await?;
        Ok(client.with_reconnector(move || async move {
            let stream = TcpStream::connect(sock_addr).await?;
            Self::connect_stream(stream).await
        }))
    }

    /// Connect to an XPC service over an already-established stream.
    pub async fn connect_stream<S>(stream: S) -> Result<Self, XpcError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let stream: DynStream = Box::new(stream);
        let mut framer = H2Framer::connect(stream)
            .await
            .map_err(|e| h2_error("H2: ", e))?;
        initialize_xpc_connection_on_framer(&mut framer).await?;
        Ok(Self {
            inner: XpcConnection::new(framer),
            reconnector: None,
        })
    }

    /// Connect to an XPC service over an already-established stream with an overall budget.
    ///
    /// The budget is shared across H2 framing handshake and XPC initialization.
    /// A zero budget rejects immediately before network IO.
    pub async fn connect_stream_with_timeout<S>(
        stream: S,
        budget: std::time::Duration,
    ) -> Result<Self, XpcError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        if budget.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "XPC stream connect budget is zero",
            )
            .into());
        }
        // One outer clock owns both phases. Resetting a per-frame budget after
        // H2 connect would let bootstrap consume a second full budget.
        xpc_timeout(
            budget,
            "initialize XPC service",
            Self::connect_stream(stream),
        )
        .await
    }

    /// Supply a factory for a fresh, initialized connection to the same service.
    /// Custom stream transports should use this when a service requires one
    /// request per connection. The factory must preserve the tunnel route.
    pub fn with_reconnector<F, Fut>(mut self, connect: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Self, XpcError>> + Send + 'static,
    {
        self.reconnector = Some(Arc::new(move || Box::pin(connect())));
        self
    }

    /// Open a new connection without reusing this connection's reply channel.
    /// Direct TCP and ConnectedDevice clients retain their original route;
    /// custom streams require [`Self::with_reconnector`].
    pub fn reconnect(&self) -> impl Future<Output = Result<Self, XpcError>> + Send + 'static {
        // Copy only the factory so the future does not borrow an active,
        // potentially non-Sync byte stream across an await.
        let connect = self.reconnector.clone();
        async move {
            let connect = connect.ok_or_else(|| {
                XpcError::Tls("this XPC transport has no reconnection factory".into())
            })?;
            let mut client = xpc_timeout(
                crate::tunnel::TUNNEL_CONNECT_TIMEOUT,
                "reconnect XPC service",
                connect(),
            )
            .await?;
            client.reconnector = Some(connect);
            Ok(client)
        }
    }

    /// Send an XPC dictionary and receive the response.
    pub async fn call(&mut self, body: XpcValue) -> Result<XpcMessage, XpcError> {
        self.inner
            .call_on_stream(body, crate::xpc::h2_raw::STREAM_SERVER_CLIENT, None)
            .await
    }

    /// `sent` becomes true after the complete request is flushed locally, even
    /// if the caller subsequently drops while waiting for its reply. This is
    /// evidence of submission, not acknowledgement or remote completion.
    pub async fn call_observed(
        &mut self,
        body: XpcValue,
        sent: &mut bool,
    ) -> Result<XpcMessage, XpcError> {
        self.inner
            .call_on_stream_observed(body, crate::xpc::h2_raw::STREAM_SERVER_CLIENT, None, sent)
            .await
    }

    /// Observe full local submission while sharing one send/reply total clock.
    pub async fn call_observed_with_timeout(
        &mut self,
        body: XpcValue,
        budget: std::time::Duration,
        sent: &mut bool,
    ) -> Result<XpcMessage, XpcError> {
        self.inner
            .call_on_stream_observed(
                body,
                crate::xpc::h2_raw::STREAM_SERVER_CLIENT,
                Some(budget),
                sent,
            )
            .await
    }

    /// Share one absolute budget across send, flow control and correlated
    /// reply reception. Zero rejects before I/O or identifier changes.
    pub async fn call_with_timeout(
        &mut self,
        body: XpcValue,
        budget: std::time::Duration,
    ) -> Result<XpcMessage, XpcError> {
        self.inner
            .call_on_stream(body, crate::xpc::h2_raw::STREAM_SERVER_CLIENT, Some(budget))
            .await
    }

    /// Send an XPC dictionary and receive the response from whichever control stream replies.
    pub async fn call_any_stream(&mut self, body: XpcValue) -> Result<XpcMessage, XpcError> {
        self.inner.call_any_stream(body, None).await
    }

    /// Send an XPC dictionary and receive the response from whichever control stream replies under a shared budget.
    pub async fn call_any_stream_with_timeout(
        &mut self,
        body: XpcValue,
        budget: std::time::Duration,
    ) -> Result<XpcMessage, XpcError> {
        self.inner.call_any_stream(body, Some(budget)).await
    }

    /// Observe full local submission while receiving the response from whichever control stream replies.
    pub async fn call_any_stream_observed(
        &mut self,
        body: XpcValue,
        sent: &mut bool,
    ) -> Result<XpcMessage, XpcError> {
        self.inner.call_any_stream_observed(body, None, sent).await
    }

    /// Observe full local submission under a shared budget while receiving the response from whichever control stream replies.
    pub async fn call_any_stream_observed_with_timeout(
        &mut self,
        body: XpcValue,
        budget: std::time::Duration,
        sent: &mut bool,
    ) -> Result<XpcMessage, XpcError> {
        self.inner
            .call_any_stream_observed(body, Some(budget), sent)
            .await
    }

    /// Send an XPC dictionary and receive the response from stream 1.
    pub async fn call_recv_client_server(
        &mut self,
        body: XpcValue,
    ) -> Result<XpcMessage, XpcError> {
        self.inner
            .call_on_stream(body, crate::xpc::h2_raw::STREAM_CLIENT_SERVER, None)
            .await
    }

    /// A shared send/reply budget for a correlated response on stream 1.
    pub async fn call_recv_client_server_with_timeout(
        &mut self,
        body: XpcValue,
        budget: std::time::Duration,
    ) -> Result<XpcMessage, XpcError> {
        self.inner
            .call_on_stream(body, crate::xpc::h2_raw::STREAM_CLIENT_SERVER, Some(budget))
            .await
    }

    /// Send an XPC request and yield each subsequent message from the
    /// server-client stream.
    ///
    /// CoreDevice streaming features acknowledge a single request and then
    /// push an arbitrary number of side-channel status messages.  Unlike
    /// [`Self::call`], the messages are therefore intentionally not matched by
    /// the request message id.  Dropping the returned stream cancels the
    /// receive loop without leaving a spawned task or an unbounded queue.
    pub fn stream_invoke(
        &mut self,
        body: XpcValue,
    ) -> impl futures_core::Stream<Item = Result<XpcMessage, XpcError>> + '_ {
        self.stream_invoke_with_flags(body, flags::WANTING_REPLY)
    }

    /// Send an XPC request and yield each subsequent message under an operation budget.
    pub fn stream_invoke_with_timeout(
        &mut self,
        body: XpcValue,
        budget: std::time::Duration,
    ) -> impl futures_core::Stream<Item = Result<XpcMessage, XpcError>> + '_ {
        self.stream_invoke_mapped_with_timeout(body, flags::WANTING_REPLY, budget, |message| {
            Ok(Some((message, false)))
        })
    }

    /// Send an XPC request and yield messages from the server-client stream
    /// without imposing a reply-correlation policy.
    ///
    /// Most CoreDevice services ask the daemon for a reply with
    /// [`flags::WANTING_REPLY`], which is what [`Self::stream_invoke`] does.
    /// A few RemoteXPC services (notably InstallCoordinationProxy) send a
    /// request without that bit and then return the next fresh message on the
    /// connection.  Keeping the flag choice in this shared primitive avoids
    /// making those services misuse [`Self::call`], which waits for a matching
    /// message id and can therefore miss a valid fresh response.
    pub fn stream_invoke_with_flags(
        &mut self,
        body: XpcValue,
        extra_flags: u32,
    ) -> impl futures_core::Stream<Item = Result<XpcMessage, XpcError>> + '_ {
        self.stream_invoke_mapped(body, extra_flags, |message| Ok(Some((message, false))))
    }

    /// A service-owned parser can recognize a complete terminal message.
    /// Public raw streams have no terminal grammar and retain cancellation
    /// ownership until dropped; completion is never inferred from a yield.
    pub(crate) fn stream_invoke_mapped<'a, T: 'a, F>(
        &'a mut self,
        body: XpcValue,
        extra_flags: u32,
        mut parse: F,
    ) -> impl futures_core::Stream<Item = Result<T, XpcError>> + 'a
    where
        F: FnMut(XpcMessage) -> Result<Option<(T, bool)>, XpcError> + 'a,
    {
        async_stream::try_stream! {
            let mut operation = self.inner.begin_operation(None)?;
            operation
                .send_with_flags(body, extra_flags)
                .await?;
            loop {
                let Some((value, finished)) = parse(operation.recv().await?)? else { continue };
                if finished {
                    operation.complete();
                }
                yield value;
                if finished { return; }
            }
        }
    }

    /// Streaming invoke with a shared deadline across send and reception.
    pub(crate) fn stream_invoke_mapped_with_timeout<'a, T: 'a, F>(
        &'a mut self,
        body: XpcValue,
        extra_flags: u32,
        budget: std::time::Duration,
        mut parse: F,
    ) -> impl futures_core::Stream<Item = Result<T, XpcError>> + 'a
    where
        F: FnMut(XpcMessage) -> Result<Option<(T, bool)>, XpcError> + 'a,
    {
        async_stream::try_stream! {
            let mut operation = self.inner.begin_operation(Some(budget))?;
            operation
                .send_with_flags(body, extra_flags)
                .await?;
            loop {
                let Some((value, finished)) = parse(operation.recv().await?)? else { continue };
                if finished {
                    operation.complete();
                }
                yield value;
                if finished { return; }
            }
        }
    }

    /// Finite uncorrelated services skip bodyless/empty dictionary ACKs. The
    /// service owns its total timeout and parses its business response after
    /// this guarded send/reply boundary has completed.
    #[cfg(feature = "pasteboard")]
    pub(crate) async fn call_nonempty(&mut self, body: XpcValue) -> Result<XpcValue, XpcError> {
        self.inner.call_nonempty(body).await
    }

    /// Send without waiting for a response.
    pub async fn send(&mut self, body: XpcValue) -> Result<(), XpcError> {
        self.inner.send(body).await
    }

    #[cfg(feature = "cryptex")]
    pub(crate) async fn send_request(&mut self, body: XpcValue) -> Result<(), XpcError> {
        self.inner
            .send_with_flags(body, flags::WANTING_REPLY)
            .await
            .map(|_| ())
    }

    /// Send a bounded announced file without retaining its contents in memory.
    #[cfg(feature = "cryptex")]
    pub(crate) async fn send_file_transfer<R: AsyncRead + Unpin>(
        &mut self,
        transfer_id: u64,
        size: u64,
        reader: &mut R,
    ) -> Result<(), XpcError> {
        self.inner
            .send_file_transfer(transfer_id, size, reader)
            .await
    }

    /// Receive the next XPC message.
    pub async fn recv(&mut self) -> Result<XpcMessage, XpcError> {
        self.inner.recv().await
    }

    /// Wait for one message using an absolute idle-read budget. An empty
    /// timeout remains usable; interruption after bytes/header consumption
    /// requires a fresh transport. This is not a lifetime limit for a stream.
    pub async fn recv_with_timeout(
        &mut self,
        budget: std::time::Duration,
    ) -> Result<XpcMessage, XpcError> {
        self.inner
            .recv_on_stream_with_budget(crate::xpc::h2_raw::STREAM_SERVER_CLIENT, Some(budget))
            .await
    }

    /// Receive the next XPC message from stream 1.
    pub async fn recv_client_server(&mut self) -> Result<XpcMessage, XpcError> {
        self.inner.recv_client_server().await
    }

    /// Wait for one message on stream 1 with an absolute idle-read budget.
    pub async fn recv_client_server_with_timeout(
        &mut self,
        budget: std::time::Duration,
    ) -> Result<XpcMessage, XpcError> {
        self.inner
            .recv_on_stream_with_budget(crate::xpc::h2_raw::STREAM_CLIENT_SERVER, Some(budget))
            .await
    }

    /// Receive the next fresh XPC message from any stream.
    ///
    /// Services whose daemons answer with fresh uncorrelated messages
    /// (InstallCoordinationProxy) must not assume the response stream.
    /// Message reassembly stays per-stream, so fragmented or multi-message
    /// frames from one stream are never spliced with another stream's data.
    pub async fn recv_any(&mut self) -> Result<XpcMessage, XpcError> {
        self.inner.recv_any_stream().await
    }

    /// Wait for one fresh message from any stream with a shared selection and
    /// body-read budget, preserving per-stream reassembly.
    pub async fn recv_any_with_timeout(
        &mut self,
        budget: std::time::Duration,
    ) -> Result<XpcMessage, XpcError> {
        self.inner.recv_any_stream_with_budget(Some(budget)).await
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures_util::StreamExt;
    use indexmap::IndexMap;
    use tokio::io::{duplex, AsyncRead, AsyncReadExt, AsyncWriteExt};
    use tokio::time::{timeout, Duration};

    use super::*;
    use crate::xpc::message::{encode_message, flags, XpcMessage, XpcValue};

    const FRAME_DATA: u8 = 0x00;
    const FRAME_HEADERS: u8 = 0x01;
    const FRAME_SETTINGS: u8 = 0x04;
    const FLAG_END_HEADERS: u8 = 0x04;
    const FLAG_SETTINGS_ACK: u8 = 0x01;
    const STREAM_INIT: u32 = 0;
    const STREAM_CLIENT_SERVER: u32 = 1;
    const STREAM_SERVER_CLIENT: u32 = 3;

    async fn lifecycle_xpc_client() -> (XpcClient, tokio::io::DuplexStream) {
        let (stream, mut peer) = duplex(4096);
        let (client, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(XpcClient::connect_stream(stream), async {
                let mut preface = [0; 58];
                peer.read_exact(&mut preface).await.unwrap();
                assert_eq!(&preface[..24], crate::xpc::h2_raw::H2_PREFACE);
                peer.write_all(&settings_frame()).await.unwrap();
                assert_eq!(lifecycle_xpc_frame(&mut peer).await.0, FRAME_SETTINGS);
                for (index, stream_id) in [1, 1, 3].into_iter().enumerate() {
                    if index < 2 {
                        let frame = lifecycle_xpc_frame(&mut peer).await;
                        assert_eq!(
                            (frame.0, frame.1),
                            (FRAME_HEADERS, if index == 0 { 1 } else { 3 })
                        );
                    }
                    let frame = lifecycle_xpc_frame(&mut peer).await;
                    assert_eq!((frame.0, frame.1), (FRAME_DATA, stream_id));
                    peer.write_all(&data_frame(stream_id, &lifecycle_xpc_message(0)))
                        .await
                        .unwrap();
                }
            })
        })
        .await
        .unwrap();
        (client.unwrap(), peer)
    }

    #[tokio::test]
    async fn observed_submission_survives_caller_drop_before_reply() {
        let (mut client, mut peer) = lifecycle_xpc_client().await;
        let mut sent = false;
        {
            let request = client.call_observed(XpcValue::Bool(true), &mut sent);
            tokio::pin!(request);
            tokio::select! {
                result = &mut request => panic!("unconfirmed call completed: {result:?}"),
                frame = lifecycle_xpc_frame(&mut peer) => { assert_eq!(frame.0, FRAME_DATA); }
            }
        }
        assert!(
            sent,
            "the full local write was observed before receiving a PID"
        );
        assert!(!client.is_usable());
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }

    async fn lifecycle_xpc_frame(peer: &mut tokio::io::DuplexStream) -> (u8, u32, Vec<u8>) {
        let mut header = [0; 9];
        peer.read_exact(&mut header).await.unwrap();
        let size = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
        assert!(size <= 16_384);
        let mut body = vec![0; size];
        peer.read_exact(&mut body).await.unwrap();
        (
            header[3],
            u32::from_be_bytes(header[5..9].try_into().unwrap()),
            body,
        )
    }

    fn lifecycle_xpc_message(id: u64) -> Bytes {
        encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: id,
            body: None,
        })
        .unwrap()
    }

    fn lifecycle_xpc_elapsed(error: &XpcError) {
        let XpcError::Io(io) = error else {
            panic!("timeout must be IO")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
        assert!(io.get_ref().unwrap().is::<tokio::time::error::Elapsed>());
        assert_eq!(crate::classify_error(error), crate::ErrorClass::Timeout);
    }

    async fn lifecycle_xpc_rejected(client: &mut XpcClient) {
        assert!(!client.is_usable());
        assert!(client.send(XpcValue::Bool(true)).await.is_err());
        assert!(client.call(XpcValue::Bool(true)).await.is_err());
        assert!(client
            .call_recv_client_server(XpcValue::Bool(true))
            .await
            .is_err());
        assert!(client.recv().await.is_err());
        assert!(client.recv_client_server().await.is_err());
        assert!(client.recv_any().await.is_err());
        assert!(client
            .call_with_timeout(XpcValue::Bool(true), Duration::ZERO)
            .await
            .is_err());
        assert!(client.recv_with_timeout(Duration::ZERO).await.is_err());
        assert!(Box::pin(client.stream_invoke(XpcValue::Bool(true)))
            .next()
            .await
            .unwrap()
            .is_err());
    }

    #[tokio::test]
    async fn lifecycle_xpc_public_zero_large_unpolled_and_complete_calls_keep_state() {
        let (mut client, mut peer) = lifecycle_xpc_client().await;
        for error in [
            client
                .call_with_timeout(XpcValue::Bool(true), Duration::ZERO)
                .await
                .unwrap_err(),
            client
                .call_recv_client_server_with_timeout(XpcValue::Bool(true), Duration::ZERO)
                .await
                .unwrap_err(),
            client.recv_with_timeout(Duration::ZERO).await.unwrap_err(),
            client
                .recv_client_server_with_timeout(Duration::ZERO)
                .await
                .unwrap_err(),
            client
                .recv_any_with_timeout(Duration::ZERO)
                .await
                .unwrap_err(),
        ] {
            let XpcError::Io(io) = error else {
                panic!("zero budget")
            };
            assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
            assert!(io.get_ref().unwrap().source().is_none());
        }
        let XpcError::Io(io) = client
            .call_with_timeout(XpcValue::Bool(true), Duration::MAX)
            .await
            .unwrap_err()
        else {
            panic!("unrepresentable deadline")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::InvalidInput);
        drop(client.call(XpcValue::Bool(true)));
        drop(client.stream_invoke(XpcValue::Bool(true)));
        assert!(client.is_usable());
        let (reply, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(
                client.call_with_timeout(XpcValue::Bool(true), Duration::from_secs(1)),
                async {
                    let (_, stream, bytes) = lifecycle_xpc_frame(&mut peer).await;
                    assert_eq!(stream, 1);
                    let request = crate::xpc::message::decode_message(bytes.into()).unwrap();
                    assert_eq!(request.msg_id, 1);
                    assert_eq!(request.flags & flags::WANTING_REPLY, flags::WANTING_REPLY);
                    peer.write_all(&data_frame(3, &lifecycle_xpc_message(9)))
                        .await
                        .unwrap();
                    peer.write_all(&data_frame(3, &lifecycle_xpc_message(1)))
                        .await
                        .unwrap();
                }
            )
        })
        .await
        .unwrap();
        assert_eq!(reply.unwrap().msg_id, 1);
        assert!(client.is_usable());
        assert_eq!(
            client
                .recv_with_timeout(Duration::from_secs(1))
                .await
                .unwrap()
                .msg_id,
            9
        );
        let (reply, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(
                client.call_recv_client_server_with_timeout(
                    XpcValue::Bool(false),
                    Duration::from_secs(1)
                ),
                async {
                    let (_, _, bytes) = lifecycle_xpc_frame(&mut peer).await;
                    assert_eq!(
                        crate::xpc::message::decode_message(bytes.into())
                            .unwrap()
                            .msg_id,
                        3
                    );
                    peer.write_all(&data_frame(1, &lifecycle_xpc_message(3)))
                        .await
                        .unwrap();
                }
            )
        })
        .await
        .unwrap();
        assert_eq!(reply.unwrap().msg_id, 3);
        assert!(client.is_usable());
        peer.write_all(&data_frame(5, &lifecycle_xpc_message(92)))
            .await
            .unwrap();
        assert_eq!(
            client
                .recv_any_with_timeout(Duration::from_secs(1))
                .await
                .unwrap()
                .msg_id,
            92
        );
    }

    #[tokio::test]
    async fn lifecycle_xpc_sent_call_cancellation_and_expiry_reject_late_reply() {
        for caller_cancel in [false, true] {
            let (mut client, mut peer) = lifecycle_xpc_client().await;
            let (result, ()) = timeout(Duration::from_secs(1), async {
                tokio::join!(
                    async {
                        if caller_cancel {
                            assert!(timeout(
                                Duration::from_millis(10),
                                client.call(XpcValue::Bool(true))
                            )
                            .await
                            .is_err());
                            None
                        } else {
                            Some(
                                client
                                    .call_with_timeout(
                                        XpcValue::Bool(true),
                                        Duration::from_millis(10),
                                    )
                                    .await
                                    .unwrap_err(),
                            )
                        }
                    },
                    async {
                        let (_, _, request) = lifecycle_xpc_frame(&mut peer).await;
                        assert_eq!(
                            crate::xpc::message::decode_message(request.into())
                                .unwrap()
                                .msg_id,
                            1
                        );
                    }
                )
            })
            .await
            .unwrap();
            if let Some(error) = result {
                lifecycle_xpc_elapsed(&error);
            }
            peer.write_all(&data_frame(3, &lifecycle_xpc_message(1)))
                .await
                .unwrap();
            lifecycle_xpc_rejected(&mut client).await;
            let mut byte = [0];
            assert!(timeout(Duration::from_millis(5), peer.read(&mut byte))
                .await
                .is_err());
            drop(client);
            assert_eq!(peer.read(&mut byte).await.unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn lifecycle_xpc_call_deadline_includes_blocked_send_and_reply_wait() {
        let (mut client, mut peer) = lifecycle_xpc_client().await;
        let (result, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(
                client.call_with_timeout(
                    XpcValue::Data(vec![42; 8192].into()),
                    Duration::from_millis(200)
                ),
                async {
                    // The 4096-byte transport cannot finish this DATA until read.
                    tokio::time::sleep(Duration::from_millis(125)).await;
                    let (_, _, request) = lifecycle_xpc_frame(&mut peer).await;
                    let request = crate::xpc::message::decode_message(request.into()).unwrap();
                    assert_eq!(request.msg_id, 1);
                    tokio::time::sleep(Duration::from_millis(125)).await;
                    peer.write_all(&data_frame(3, &lifecycle_xpc_message(1)))
                        .await
                        .unwrap();
                }
            )
        })
        .await
        .unwrap();
        lifecycle_xpc_elapsed(&result.unwrap_err());
        lifecycle_xpc_rejected(&mut client).await;
    }

    #[tokio::test]
    async fn lifecycle_xpc_blocked_send_and_empty_idle_have_distinct_reuse() {
        let (mut client, mut peer) = lifecycle_xpc_client().await;
        lifecycle_xpc_elapsed(
            &client
                .recv_with_timeout(Duration::from_millis(5))
                .await
                .unwrap_err(),
        );
        lifecycle_xpc_elapsed(
            &client
                .recv_any_with_timeout(Duration::from_millis(5))
                .await
                .unwrap_err(),
        );
        assert!(client.is_usable());
        peer.write_all(&data_frame(1, &lifecycle_xpc_message(7)))
            .await
            .unwrap();
        assert_eq!(
            client
                .recv_client_server_with_timeout(Duration::from_secs(1))
                .await
                .unwrap()
                .msg_id,
            7
        );
        let error = client
            .call_with_timeout(
                XpcValue::Data(vec![42; 8192].into()),
                Duration::from_millis(5),
            )
            .await
            .unwrap_err();
        lifecycle_xpc_elapsed(&error);
        lifecycle_xpc_rejected(&mut client).await;
        drop(client);
        let mut prefix = Vec::new();
        peer.read_to_end(&mut prefix).await.unwrap();
        assert_eq!(prefix.len(), 4096);
        assert_eq!(prefix[3], FRAME_DATA);
    }

    #[tokio::test]
    async fn lifecycle_xpc_stream_drop_between_messages_invalidates_request_ownership() {
        let (mut client, mut peer) = lifecycle_xpc_client().await;
        {
            let mut stream = Box::pin(client.stream_invoke_with_flags(XpcValue::Bool(true), 0));
            let (message, ()) = timeout(Duration::from_secs(1), async {
                tokio::join!(stream.next(), async {
                    let (_, _, request) = lifecycle_xpc_frame(&mut peer).await;
                    let request = crate::xpc::message::decode_message(request.into()).unwrap();
                    assert_eq!(request.flags & flags::WANTING_REPLY, 0);
                    peer.write_all(&data_frame(3, &lifecycle_xpc_message(91)))
                        .await
                        .unwrap();
                })
            })
            .await
            .unwrap();
            assert_eq!(message.unwrap().unwrap().msg_id, 91);
        }
        lifecycle_xpc_rejected(&mut client).await;
    }

    #[cfg(feature = "cryptex")]
    #[tokio::test]
    async fn lifecycle_xpc_announced_file_cancellation_and_host_error_invalidate_owner() {
        for cancel in [false, true] {
            let (mut client, mut peer) = lifecycle_xpc_client().await;
            if cancel {
                let (mut reader, _source) = duplex(64);
                let (result, ()) = timeout(Duration::from_secs(1), async {
                    tokio::join!(
                        timeout(
                            Duration::from_millis(10),
                            client.send_file_transfer(1, 8, &mut reader)
                        ),
                        async {
                            assert_eq!(lifecycle_xpc_frame(&mut peer).await.0, FRAME_HEADERS);
                            let (_, stream, preamble) = lifecycle_xpc_frame(&mut peer).await;
                            assert_eq!(stream, 5);
                            assert_eq!(
                                crate::xpc::message::decode_message(preamble.into())
                                    .unwrap()
                                    .flags,
                                flags::ALWAYS_SET | flags::FILE_TX_STREAM_REQUEST
                            );
                        }
                    )
                })
                .await
                .unwrap();
                assert!(result.is_err());
            } else {
                struct HostFailure;
                impl tokio::io::AsyncRead for HostFailure {
                    fn poll_read(
                        self: Pin<&mut Self>,
                        _: &mut std::task::Context<'_>,
                        _: &mut tokio::io::ReadBuf<'_>,
                    ) -> std::task::Poll<std::io::Result<()>> {
                        std::task::Poll::Ready(Err(std::fs::File::open("").unwrap_err()))
                    }
                }
                let error = client
                    .send_file_transfer(1, 8, &mut HostFailure)
                    .await
                    .unwrap_err();
                let XpcError::Io(io) = error else {
                    panic!("original host error")
                };
                let expected = std::fs::File::open("").unwrap_err();
                assert_eq!(io.kind(), expected.kind());
                assert_eq!(io.raw_os_error(), expected.raw_os_error());
                assert_eq!(lifecycle_xpc_frame(&mut peer).await.0, FRAME_HEADERS);
                assert_eq!(lifecycle_xpc_frame(&mut peer).await.1, 5);
            }
            lifecycle_xpc_rejected(&mut client).await;
            drop(client);
            let mut remaining = Vec::new();
            peer.read_to_end(&mut remaining).await.unwrap();
            assert!(remaining.is_empty(), "Drop must not send a detached FIN");
        }
    }

    #[cfg(feature = "apps")]
    #[tokio::test]
    async fn lifecycle_xpc_coredevice_explicit_finish_preserves_connection_reuse() {
        let (mut client, mut peer) = lifecycle_xpc_client().await;
        {
            let mut stream = Box::pin(crate::services::coredevice::stream_invoke(
                &mut client,
                "owned-device",
                "owned-feature",
                XpcValue::Dictionary(IndexMap::new()),
            ));
            let (result, ()) = timeout(Duration::from_secs(1), async {
                tokio::join!(stream.next(), async {
                    lifecycle_xpc_frame(&mut peer).await;
                    peer.write_all(&data_frame(3, &lifecycle_xpc_message(0)))
                        .await
                        .unwrap();
                    let reply = encode_message(&XpcMessage {
                        flags: flags::ALWAYS_SET,
                        msg_id: 99,
                        body: Some(XpcValue::Dictionary(IndexMap::from([(
                            "CoreDevice.XPCMessageKey.sideChannelStatus".into(),
                            XpcValue::Dictionary(IndexMap::from([(
                                "finishStreaming".into(),
                                XpcValue::Bool(true),
                            )])),
                        )]))),
                    })
                    .unwrap();
                    peer.write_all(&data_frame(3, &reply)).await.unwrap();
                })
            })
            .await
            .unwrap();
            assert!(result.is_none());
        }
        assert!(client.is_usable());
        client.send(XpcValue::Bool(true)).await.unwrap();
        assert_eq!(lifecycle_xpc_frame(&mut peer).await.1, 1);
    }

    #[cfg(feature = "pasteboard")]
    #[tokio::test]
    async fn lifecycle_xpc_finite_pasteboard_replies_and_business_errors_preserve_reuse() {
        let (client, mut peer) = lifecycle_xpc_client().await;
        let mut pasteboard =
            crate::pasteboard::PasteboardClient::with_timeout(client, Duration::from_secs(1));
        let ((), ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(
                async {
                    assert!(matches!(
                        pasteboard.get().await,
                        Err(crate::pasteboard::PasteboardError::Service { .. })
                    ));
                    assert!(pasteboard.get().await.is_ok());
                },
                async {
                    for id in [1, 3] {
                        let (_, _, bytes) = lifecycle_xpc_frame(&mut peer).await;
                        let request = crate::decode_xpc_message(bytes.into()).unwrap();
                        assert_eq!(request.msg_id, id);
                        assert_eq!(
                            request.body.unwrap().as_dict().unwrap()["command"].as_str(),
                            Some("PULL")
                        );
                        peer.write_all(&data_frame(3, &lifecycle_xpc_message(0)))
                            .await
                            .unwrap();
                        let body = if id == 1 {
                            XpcValue::Dictionary(IndexMap::from([(
                                "error".into(),
                                XpcValue::Dictionary(IndexMap::from([
                                    (
                                        "domain".into(),
                                        XpcValue::String("NSCocoaErrorDomain".into()),
                                    ),
                                    ("code".into(), XpcValue::Int64(4864)),
                                ])),
                            )]))
                        } else {
                            XpcValue::Dictionary(IndexMap::from([(
                                "command".into(),
                                XpcValue::String("PULL_REPLY".into()),
                            )]))
                        };
                        let reply = encode_message(&XpcMessage {
                            flags: flags::ALWAYS_SET,
                            msg_id: 90 + id,
                            body: Some(body),
                        })
                        .unwrap();
                        peer.write_all(&data_frame(3, &reply)).await.unwrap();
                    }
                }
            )
        })
        .await
        .unwrap();
    }

    #[cfg(feature = "cryptex")]
    #[tokio::test]
    async fn lifecycle_xpc_successful_file_preserves_fin_and_response_half() {
        let (mut client, mut peer) = lifecycle_xpc_client().await;
        let mut reader = &b"content"[..];
        let (result, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(client.send_file_transfer(1, 7, &mut reader), async {
                assert_eq!(lifecycle_xpc_frame(&mut peer).await.0, FRAME_HEADERS);
                assert_eq!(lifecycle_xpc_frame(&mut peer).await.1, 5);
                assert_eq!(lifecycle_xpc_frame(&mut peer).await.2, b"content");
                let mut fin = [0; 9];
                peer.read_exact(&mut fin).await.unwrap();
                assert_eq!(fin, [0, 0, 0, FRAME_DATA, 1, 0, 0, 0, 5]);
                peer.write_all(&data_frame(3, &lifecycle_xpc_message(7)))
                    .await
                    .unwrap();
            })
        })
        .await
        .unwrap();
        result.unwrap();
        assert!(client.is_usable());
        assert_eq!(client.recv().await.unwrap().msg_id, 7);
    }

    fn build_frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
        let len = payload.len();
        let mut out = Vec::with_capacity(9 + len);
        out.push(((len >> 16) & 0xFF) as u8);
        out.push(((len >> 8) & 0xFF) as u8);
        out.push((len & 0xFF) as u8);
        out.push(frame_type);
        out.push(flags);
        out.extend_from_slice(&(stream_id & 0x7FFF_FFFF).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn settings_frame() -> Vec<u8> {
        build_frame(FRAME_SETTINGS, 0, STREAM_INIT, &[])
    }

    fn settings_ack_frame() -> Vec<u8> {
        build_frame(FRAME_SETTINGS, FLAG_SETTINGS_ACK, STREAM_INIT, &[])
    }

    fn headers_frame(stream_id: u32) -> Vec<u8> {
        build_frame(FRAME_HEADERS, FLAG_END_HEADERS, stream_id, &[])
    }

    fn data_frame(stream_id: u32, payload: &[u8]) -> Vec<u8> {
        build_frame(FRAME_DATA, 0, stream_id, payload)
    }

    fn empty_message(flags: u32) -> Bytes {
        encode_message(&XpcMessage {
            flags,
            msg_id: 0,
            body: (flags == flags::ALWAYS_SET).then_some(XpcValue::Dictionary(IndexMap::new())),
        })
        .expect("message should encode")
    }

    #[tokio::test]
    async fn connect_stream_bootstraps_remote_xpc_before_returning() {
        let (client, mut server) = duplex(4096);

        let msg1 = empty_message(flags::ALWAYS_SET);
        let msg2 = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .expect("message should encode");
        let msg3 = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .expect("message should encode");

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            let mut cs_headers = [0u8; 9];
            server.read_exact(&mut cs_headers).await.unwrap();
            assert_eq!(cs_headers, headers_frame(STREAM_CLIENT_SERVER).as_slice());

            let mut cs_msg1_header = [0u8; 9];
            server.read_exact(&mut cs_msg1_header).await.unwrap();
            assert_eq!(cs_msg1_header[3], FRAME_DATA);
            let cs_msg1_len = ((cs_msg1_header[0] as usize) << 16)
                | ((cs_msg1_header[1] as usize) << 8)
                | (cs_msg1_header[2] as usize);
            let mut cs_msg1 = vec![0u8; cs_msg1_len];
            server.read_exact(&mut cs_msg1).await.unwrap();
            assert_eq!(cs_msg1, msg1);

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &msg2))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_headers = [0u8; 9];
            server.read_exact(&mut sc_headers).await.unwrap();
            assert_eq!(sc_headers, headers_frame(STREAM_SERVER_CLIENT).as_slice());

            let mut cs_msg3_header = [0u8; 9];
            server.read_exact(&mut cs_msg3_header).await.unwrap();
            assert_eq!(cs_msg3_header[3], FRAME_DATA);
            let cs_msg3_len = ((cs_msg3_header[0] as usize) << 16)
                | ((cs_msg3_header[1] as usize) << 8)
                | (cs_msg3_header[2] as usize);
            let mut cs_msg3 = vec![0u8; cs_msg3_len];
            server.read_exact(&mut cs_msg3).await.unwrap();
            assert_eq!(
                decode_message_payload(&cs_msg3),
                (flags::ALWAYS_SET | 0x200, 0)
            );

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &msg3))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_msg2_header = [0u8; 9];
            server.read_exact(&mut sc_msg2_header).await.unwrap();
            assert_eq!(sc_msg2_header[3], FRAME_DATA);
            let sc_msg2_len = ((sc_msg2_header[0] as usize) << 16)
                | ((sc_msg2_header[1] as usize) << 8)
                | (sc_msg2_header[2] as usize);
            let mut sc_msg2 = vec![0u8; sc_msg2_len];
            server.read_exact(&mut sc_msg2).await.unwrap();
            assert_eq!(
                decode_message_payload(&sc_msg2),
                (flags::INIT_HANDSHAKE | flags::ALWAYS_SET, 0)
            );

            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &msg2))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        timeout(Duration::from_secs(1), XpcClient::connect_stream(client))
            .await
            .expect("connect timed out")
            .expect("connect should succeed");

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn call_sets_wanting_reply_on_outgoing_request() {
        let (client, mut server) = duplex(4096);

        let empty = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .expect("message should encode");
        let reply = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET | flags::REPLY | flags::DATA,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::new())),
        })
        .expect("message should encode");

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            let mut cs_headers = [0u8; 9];
            server.read_exact(&mut cs_headers).await.unwrap();
            assert_eq!(cs_headers, headers_frame(STREAM_CLIENT_SERVER).as_slice());

            let mut cs_msg1_header = [0u8; 9];
            server.read_exact(&mut cs_msg1_header).await.unwrap();
            let cs_msg1_len = ((cs_msg1_header[0] as usize) << 16)
                | ((cs_msg1_header[1] as usize) << 8)
                | (cs_msg1_header[2] as usize);
            let mut cs_msg1 = vec![0u8; cs_msg1_len];
            server.read_exact(&mut cs_msg1).await.unwrap();
            assert_eq!(
                cs_msg1.as_slice(),
                empty_message(flags::ALWAYS_SET).as_ref()
            );

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_headers = [0u8; 9];
            server.read_exact(&mut sc_headers).await.unwrap();
            assert_eq!(sc_headers, headers_frame(STREAM_SERVER_CLIENT).as_slice());

            let mut cs_msg3_header = [0u8; 9];
            server.read_exact(&mut cs_msg3_header).await.unwrap();
            let cs_msg3_len = ((cs_msg3_header[0] as usize) << 16)
                | ((cs_msg3_header[1] as usize) << 8)
                | (cs_msg3_header[2] as usize);
            let mut cs_msg3 = vec![0u8; cs_msg3_len];
            server.read_exact(&mut cs_msg3).await.unwrap();
            assert_eq!(
                decode_message_payload(&cs_msg3),
                (flags::ALWAYS_SET | 0x200, 0)
            );

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_msg2_header = [0u8; 9];
            server.read_exact(&mut sc_msg2_header).await.unwrap();
            let sc_msg2_len = ((sc_msg2_header[0] as usize) << 16)
                | ((sc_msg2_header[1] as usize) << 8)
                | (sc_msg2_header[2] as usize);
            let mut sc_msg2 = vec![0u8; sc_msg2_len];
            server.read_exact(&mut sc_msg2).await.unwrap();
            assert_eq!(
                decode_message_payload(&sc_msg2),
                (flags::INIT_HANDSHAKE | flags::ALWAYS_SET, 0)
            );

            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &empty))
                .await
                .unwrap();

            let mut request_header = [0u8; 9];
            server.read_exact(&mut request_header).await.unwrap();
            assert_eq!(request_header[3], FRAME_DATA);
            let request_len = ((request_header[0] as usize) << 16)
                | ((request_header[1] as usize) << 8)
                | (request_header[2] as usize);
            let mut request = vec![0u8; request_len];
            server.read_exact(&mut request).await.unwrap();
            assert_eq!(
                decode_message_payload(&request),
                (flags::ALWAYS_SET | flags::DATA | flags::WANTING_REPLY, 1)
            );

            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &reply))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let mut client = timeout(Duration::from_secs(1), XpcClient::connect_stream(client))
            .await
            .expect("connect timed out")
            .expect("connect should succeed");

        let response = timeout(
            Duration::from_secs(1),
            client.call(XpcValue::Dictionary(IndexMap::new())),
        )
        .await
        .expect("call timed out")
        .expect("call should succeed");

        assert_eq!(
            response.flags,
            flags::ALWAYS_SET | flags::REPLY | flags::DATA
        );
        assert_eq!(response.msg_id, 1);

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn stream_invoke_with_flags_yields_each_pushed_message() {
        let (client, mut server) = duplex(16 * 1024);
        let empty = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .expect("message should encode");
        let first = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET | flags::REPLY | flags::DATA,
            msg_id: 71,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "batch".into(),
                XpcValue::Int64(1),
            )]))),
        })
        .expect("message should encode");
        let second = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET | flags::REPLY | flags::DATA,
            msg_id: 72,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "batch".into(),
                XpcValue::Int64(2),
            )]))),
        })
        .expect("message should encode");

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let (frame_type, _, stream_id, _) = read_h2_frame(&mut server).await;
            assert_eq!((frame_type, stream_id), (FRAME_SETTINGS, STREAM_INIT));
            let (frame_type, _, stream_id, _) = read_h2_frame(&mut server).await;
            assert_eq!((frame_type, stream_id), (FRAME_WINDOW_UPDATE, STREAM_INIT));

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let (frame_type, frame_flags, stream_id, _) = read_h2_frame(&mut server).await;
            assert_eq!(
                (frame_type, frame_flags, stream_id),
                (FRAME_SETTINGS, FLAG_SETTINGS_ACK, STREAM_INIT)
            );

            let (frame_type, _, stream_id, _) = read_h2_frame(&mut server).await;
            assert_eq!(
                (frame_type, stream_id),
                (FRAME_HEADERS, STREAM_CLIENT_SERVER)
            );
            let (frame_type, _, stream_id, payload) = read_h2_frame(&mut server).await;
            assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_CLIENT_SERVER));
            assert_eq!(decode_message_payload(&payload), (flags::ALWAYS_SET, 0));
            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();

            let (frame_type, _, stream_id, _) = read_h2_frame(&mut server).await;
            assert_eq!(
                (frame_type, stream_id),
                (FRAME_HEADERS, STREAM_SERVER_CLIENT)
            );
            let (frame_type, _, stream_id, payload) = read_h2_frame(&mut server).await;
            assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_CLIENT_SERVER));
            assert_eq!(
                decode_message_payload(&payload),
                (flags::ALWAYS_SET | 0x200, 0)
            );
            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();

            let (frame_type, _, stream_id, payload) = read_h2_frame(&mut server).await;
            assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_SERVER_CLIENT));
            assert_eq!(
                decode_message_payload(&payload),
                (flags::INIT_HANDSHAKE | flags::ALWAYS_SET, 0)
            );
            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &empty))
                .await
                .unwrap();

            let (frame_type, _, stream_id, payload) = read_h2_frame(&mut server).await;
            assert_eq!((frame_type, stream_id), (FRAME_DATA, STREAM_CLIENT_SERVER));
            assert_eq!(
                decode_message_payload(&payload),
                (flags::ALWAYS_SET | flags::DATA, 1)
            );
            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &first))
                .await
                .unwrap();
            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &second))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let mut client = timeout(Duration::from_secs(1), XpcClient::connect_stream(client))
            .await
            .expect("connect timed out")
            .expect("connect should succeed");
        let mut stream = Box::pin(client.stream_invoke_with_flags(
            XpcValue::Dictionary(IndexMap::from([("request".into(), XpcValue::Bool(true))])),
            0,
        ));

        let first = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("first stream response timed out")
            .expect("stream should yield first response")
            .expect("first response should decode");
        assert_eq!(first.msg_id, 71);
        assert_eq!(
            first
                .body
                .as_ref()
                .and_then(XpcValue::as_dict)
                .and_then(|dict| dict.get("batch")),
            Some(&XpcValue::Int64(1))
        );

        let second = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("second stream response timed out")
            .expect("stream should yield second response")
            .expect("second response should decode");
        assert_eq!(second.msg_id, 72);
        assert_eq!(
            second
                .body
                .as_ref()
                .and_then(XpcValue::as_dict)
                .and_then(|dict| dict.get("batch")),
            Some(&XpcValue::Int64(2))
        );

        let eof = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("EOF response timed out")
            .expect("stream should report transport EOF")
            .expect_err("stream EOF must not look like a successful response");
        assert!(!eof.to_string().is_empty());

        drop(stream);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn call_recv_client_server_reads_reply_from_stream_1() {
        let (client, mut server) = duplex(4096);

        let empty = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .expect("message should encode");
        let reply = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET | flags::REPLY | flags::DATA,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "FileList".to_string(),
                XpcValue::Array(vec![XpcValue::String("Documents".into())]),
            )]))),
        })
        .expect("message should encode");

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            let mut cs_headers = [0u8; 9];
            server.read_exact(&mut cs_headers).await.unwrap();
            assert_eq!(cs_headers, headers_frame(STREAM_CLIENT_SERVER).as_slice());

            let mut cs_msg1_header = [0u8; 9];
            server.read_exact(&mut cs_msg1_header).await.unwrap();
            let cs_msg1_len = ((cs_msg1_header[0] as usize) << 16)
                | ((cs_msg1_header[1] as usize) << 8)
                | (cs_msg1_header[2] as usize);
            let mut cs_msg1 = vec![0u8; cs_msg1_len];
            server.read_exact(&mut cs_msg1).await.unwrap();
            assert_eq!(
                cs_msg1.as_slice(),
                empty_message(flags::ALWAYS_SET).as_ref()
            );

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_headers = [0u8; 9];
            server.read_exact(&mut sc_headers).await.unwrap();
            assert_eq!(sc_headers, headers_frame(STREAM_SERVER_CLIENT).as_slice());

            let mut cs_msg3_header = [0u8; 9];
            server.read_exact(&mut cs_msg3_header).await.unwrap();
            let cs_msg3_len = ((cs_msg3_header[0] as usize) << 16)
                | ((cs_msg3_header[1] as usize) << 8)
                | (cs_msg3_header[2] as usize);
            let mut cs_msg3 = vec![0u8; cs_msg3_len];
            server.read_exact(&mut cs_msg3).await.unwrap();
            assert_eq!(
                decode_message_payload(&cs_msg3),
                (flags::ALWAYS_SET | 0x200, 0)
            );

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_msg2_header = [0u8; 9];
            server.read_exact(&mut sc_msg2_header).await.unwrap();
            let sc_msg2_len = ((sc_msg2_header[0] as usize) << 16)
                | ((sc_msg2_header[1] as usize) << 8)
                | (sc_msg2_header[2] as usize);
            let mut sc_msg2 = vec![0u8; sc_msg2_len];
            server.read_exact(&mut sc_msg2).await.unwrap();
            assert_eq!(
                decode_message_payload(&sc_msg2),
                (flags::INIT_HANDSHAKE | flags::ALWAYS_SET, 0)
            );

            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut request_header = [0u8; 9];
            server.read_exact(&mut request_header).await.unwrap();
            assert_eq!(request_header[3], FRAME_DATA);
            let request_len = ((request_header[0] as usize) << 16)
                | ((request_header[1] as usize) << 8)
                | (request_header[2] as usize);
            let mut request = vec![0u8; request_len];
            server.read_exact(&mut request).await.unwrap();
            assert_eq!(
                decode_message_payload(&request),
                (flags::ALWAYS_SET | flags::DATA | flags::WANTING_REPLY, 1)
            );

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &reply))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let mut client = timeout(Duration::from_secs(1), XpcClient::connect_stream(client))
            .await
            .expect("connect timed out")
            .expect("connect should succeed");

        let response = timeout(
            Duration::from_secs(1),
            client.call_recv_client_server(XpcValue::Dictionary(IndexMap::new())),
        )
        .await
        .expect("call timed out")
        .expect("call should succeed");

        assert_eq!(response.msg_id, 1);
        let body = response.body.and_then(|value| match value {
            XpcValue::Dictionary(dict) => Some(dict),
            _ => None,
        });
        assert!(body.unwrap().contains_key("FileList"));

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn call_waits_for_matching_reply_id_and_buffers_early_replies() {
        let (client, mut server) = duplex(4096);

        let empty = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .expect("message should encode");
        let early_reply = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET | flags::REPLY | flags::DATA,
            msg_id: 2,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "kind".to_string(),
                XpcValue::String("early".into()),
            )]))),
        })
        .expect("message should encode");
        let matching_reply = encode_message(&XpcMessage {
            flags: flags::ALWAYS_SET | flags::REPLY | flags::DATA,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "kind".to_string(),
                XpcValue::String("matching".into()),
            )]))),
        })
        .expect("message should encode");

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();
            assert_eq!(window_update[3], 0x08);

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            let mut cs_headers = [0u8; 9];
            server.read_exact(&mut cs_headers).await.unwrap();
            assert_eq!(cs_headers, headers_frame(STREAM_CLIENT_SERVER).as_slice());

            let mut cs_msg1_header = [0u8; 9];
            server.read_exact(&mut cs_msg1_header).await.unwrap();
            let cs_msg1_len = ((cs_msg1_header[0] as usize) << 16)
                | ((cs_msg1_header[1] as usize) << 8)
                | (cs_msg1_header[2] as usize);
            let mut cs_msg1 = vec![0u8; cs_msg1_len];
            server.read_exact(&mut cs_msg1).await.unwrap();
            assert_eq!(
                cs_msg1.as_slice(),
                empty_message(flags::ALWAYS_SET).as_ref()
            );

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_headers = [0u8; 9];
            server.read_exact(&mut sc_headers).await.unwrap();
            assert_eq!(sc_headers, headers_frame(STREAM_SERVER_CLIENT).as_slice());

            let mut cs_msg3_header = [0u8; 9];
            server.read_exact(&mut cs_msg3_header).await.unwrap();
            let cs_msg3_len = ((cs_msg3_header[0] as usize) << 16)
                | ((cs_msg3_header[1] as usize) << 8)
                | (cs_msg3_header[2] as usize);
            let mut cs_msg3 = vec![0u8; cs_msg3_len];
            server.read_exact(&mut cs_msg3).await.unwrap();
            assert_eq!(
                decode_message_payload(&cs_msg3),
                (flags::ALWAYS_SET | 0x200, 0)
            );

            server
                .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut sc_msg2_header = [0u8; 9];
            server.read_exact(&mut sc_msg2_header).await.unwrap();
            let sc_msg2_len = ((sc_msg2_header[0] as usize) << 16)
                | ((sc_msg2_header[1] as usize) << 8)
                | (sc_msg2_header[2] as usize);
            let mut sc_msg2 = vec![0u8; sc_msg2_len];
            server.read_exact(&mut sc_msg2).await.unwrap();
            assert_eq!(
                decode_message_payload(&sc_msg2),
                (flags::INIT_HANDSHAKE | flags::ALWAYS_SET, 0)
            );

            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &empty))
                .await
                .unwrap();
            server.flush().await.unwrap();

            let mut request_header = [0u8; 9];
            server.read_exact(&mut request_header).await.unwrap();
            assert_eq!(request_header[3], FRAME_DATA);
            let request_len = ((request_header[0] as usize) << 16)
                | ((request_header[1] as usize) << 8)
                | (request_header[2] as usize);
            let mut request = vec![0u8; request_len];
            server.read_exact(&mut request).await.unwrap();
            assert_eq!(
                decode_message_payload(&request),
                (flags::ALWAYS_SET | flags::DATA | flags::WANTING_REPLY, 1)
            );

            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &early_reply))
                .await
                .unwrap();
            server
                .write_all(&data_frame(STREAM_SERVER_CLIENT, &matching_reply))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let mut client = timeout(Duration::from_secs(1), XpcClient::connect_stream(client))
            .await
            .expect("connect timed out")
            .expect("connect should succeed");

        let response = timeout(
            Duration::from_secs(1),
            client.call(XpcValue::Dictionary(IndexMap::new())),
        )
        .await
        .expect("call timed out")
        .expect("call should succeed");

        assert_eq!(response.msg_id, 1);
        let response_kind = response
            .body
            .as_ref()
            .and_then(XpcValue::as_dict)
            .and_then(|dict| dict.get("kind"))
            .and_then(XpcValue::as_str);
        assert_eq!(response_kind, Some("matching"));

        let buffered = client.recv().await.expect("early reply should be buffered");
        assert_eq!(buffered.msg_id, 2);
        let buffered_kind = buffered
            .body
            .as_ref()
            .and_then(XpcValue::as_dict)
            .and_then(|dict| dict.get("kind"))
            .and_then(XpcValue::as_str);
        assert_eq!(buffered_kind, Some("early"));

        server_task.await.unwrap();
    }

    const FRAME_WINDOW_UPDATE: u8 = 0x08;

    async fn read_h2_frame<S: AsyncRead + Unpin>(server: &mut S) -> (u8, u8, u32, Vec<u8>) {
        let mut header = [0u8; 9];
        server.read_exact(&mut header).await.unwrap();
        let len = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | (header[2] as usize);
        let mut payload = vec![0u8; len];
        server.read_exact(&mut payload).await.unwrap();
        (
            header[3],
            header[4],
            u32::from_be_bytes(header[5..9].try_into().unwrap()),
            payload,
        )
    }

    fn decode_message_payload(bytes: &[u8]) -> (u32, u64) {
        let msg = crate::xpc::message::decode_message(Bytes::copy_from_slice(bytes)).unwrap();
        (msg.flags, msg.msg_id)
    }

    #[tokio::test(start_paused = true)]
    async fn review_regression_xpc_connect_has_one_clock_across_h2_and_bootstrap() {
        let (stream, mut peer) = duplex(4096);
        let start = tokio::time::Instant::now();
        let (result, ()) = tokio::join!(
            XpcClient::connect_stream_with_timeout(stream, Duration::from_millis(100)),
            async {
                let mut preface = [0; 58];
                peer.read_exact(&mut preface).await.unwrap();
                tokio::time::sleep(Duration::from_millis(60)).await;
                peer.write_all(&settings_frame()).await.unwrap();
                assert_eq!(lifecycle_xpc_frame(&mut peer).await.0, FRAME_SETTINGS);
                // H2 has completed, but the peer withholds the XPC response.
                assert_eq!(lifecycle_xpc_frame(&mut peer).await.0, FRAME_HEADERS);
                let mut rest = Vec::new();
                peer.read_to_end(&mut rest).await.unwrap();
            }
        );
        let error = result.err().expect("bootstrap must time out");
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        let XpcError::Io(io) = &error else {
            panic!("expected native timeout IO")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
        let mut cause = io
            .get_ref()
            .map(|value| value as &(dyn std::error::Error + 'static));
        let mut found_elapsed = false;
        while let Some(source) = cause {
            found_elapsed |= source.is::<tokio::time::error::Elapsed>();
            cause = source.source();
        }
        assert!(
            found_elapsed,
            "fixed phase context must retain native Elapsed"
        );
        assert_eq!(start.elapsed(), Duration::from_millis(100));
    }

    #[tokio::test]
    async fn test_connect_stream_with_timeout_zero_budget_fails_without_io() {
        let (client, _server) = tokio::io::duplex(64);
        let res = XpcClient::connect_stream_with_timeout(client, Duration::ZERO).await;
        match res {
            Err(XpcError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::TimedOut),
            other => panic!("expected TimedOut io error, got: {:?}", other.is_err()),
        }
    }

    #[tokio::test]
    async fn test_connect_stream_with_timeout_expires_during_handshake() {
        let (client, mut server) = tokio::io::duplex(1024);
        let server_task = tokio::spawn(async move {
            // Read client preface
            let mut preface = [0u8; 24];
            let _ = server.read_exact(&mut preface).await;
            // Stall without replying
            tokio::time::sleep(Duration::from_millis(150)).await;
        });

        let res = XpcClient::connect_stream_with_timeout(client, Duration::from_millis(40)).await;
        match res {
            Err(XpcError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::TimedOut),
            other => panic!("expected TimedOut io error, got: {:?}", other.is_err()),
        }

        let _ = server_task.await;
    }
}
