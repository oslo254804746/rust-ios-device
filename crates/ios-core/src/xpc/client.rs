//! XPC client: high-level wrapper around XpcConnection for service calls.

use std::net::{Ipv6Addr, SocketAddr};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::xpc::h2_raw::H2Framer;
use crate::xpc::message::{flags, XpcMessage, XpcValue};
use crate::xpc::rsd::{initialize_xpc_connection_on_framer, XpcConnection};
use crate::xpc::XpcError;

trait XpcStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> XpcStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type DynStream = Box<dyn XpcStream>;

/// High-level XPC client for iOS 17+ services.
pub struct XpcClient {
    inner: XpcConnection<DynStream>,
}

impl XpcClient {
    /// Connect to an XPC service at the given IPv6 address and port.
    pub async fn connect(addr: Ipv6Addr, port: u16) -> Result<Self, XpcError> {
        let sock_addr = SocketAddr::new(addr.into(), port);
        let stream = TcpStream::connect(sock_addr).await?;
        Self::connect_stream(stream).await
    }

    /// Connect to an XPC service over an already-established stream.
    pub async fn connect_stream<S>(stream: S) -> Result<Self, XpcError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let stream: DynStream = Box::new(stream);
        let mut framer = H2Framer::connect(stream)
            .await
            .map_err(|e| XpcError::Tls(format!("H2: {e}")))?;
        initialize_xpc_connection_on_framer(&mut framer).await?;
        Ok(Self {
            inner: XpcConnection::new(framer),
        })
    }

    /// Send an XPC dictionary and receive the response.
    pub async fn call(&mut self, body: XpcValue) -> Result<XpcMessage, XpcError> {
        self.inner
            .send_with_flags(body, flags::WANTING_REPLY)
            .await?;
        self.inner.recv().await
    }

    /// Send an XPC dictionary and receive the response from stream 1.
    pub async fn call_recv_client_server(
        &mut self,
        body: XpcValue,
    ) -> Result<XpcMessage, XpcError> {
        self.inner
            .send_with_flags(body, flags::WANTING_REPLY)
            .await?;
        self.inner.recv_client_server().await
    }

    /// Send without waiting for a response.
    pub async fn send(&mut self, body: XpcValue) -> Result<(), XpcError> {
        self.inner.send(body).await
    }

    /// Receive the next XPC message.
    pub async fn recv(&mut self) -> Result<XpcMessage, XpcError> {
        self.inner.recv().await
    }

    /// Receive the next XPC message from stream 1.
    pub async fn recv_client_server(&mut self) -> Result<XpcMessage, XpcError> {
        self.inner.recv_client_server().await
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use indexmap::IndexMap;
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
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
            body: Some(XpcValue::Dictionary(IndexMap::new()))
                .filter(|_| flags == flags::ALWAYS_SET),
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

    fn decode_message_payload(bytes: &[u8]) -> (u32, u64) {
        let msg = crate::xpc::message::decode_message(Bytes::copy_from_slice(bytes)).unwrap();
        (msg.flags, msg.msg_id)
    }
}
