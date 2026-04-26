use crate::xpc::h2_raw::H2Framer;
use crate::xpc::{XpcMessage, XpcValue};
use bytes::BytesMut;
use indexmap::IndexMap;
use tokio::io::{AsyncRead, AsyncWrite};

pub const SERVICE_NAME: &str = "com.apple.RestoreRemoteServices.restoreserviced";

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct RestoreServiceClient<S> {
    framer: H2Framer<S>,
    next_msg_id: u64,
    pending_control_data: BytesMut,
}

impl<S: AsyncRead + AsyncWrite + Unpin> RestoreServiceClient<S> {
    pub async fn connect(stream: S) -> Result<Self, RestoreError> {
        let mut framer = H2Framer::connect(stream)
            .await
            .map_err(|err| RestoreError::Protocol(format!("H2 error: {err}")))?;
        bootstrap_remote_xpc(&mut framer).await?;
        Ok(Self {
            framer,
            next_msg_id: 1,
            pending_control_data: BytesMut::new(),
        })
    }

    pub async fn enter_recovery(&mut self) -> Result<IndexMap<String, XpcValue>, RestoreError> {
        self.validate_command("recovery").await
    }

    pub async fn delay_recovery_image(
        &mut self,
    ) -> Result<IndexMap<String, XpcValue>, RestoreError> {
        self.validate_command("delayrecoveryimage").await
    }

    pub async fn reboot(&mut self) -> Result<IndexMap<String, XpcValue>, RestoreError> {
        self.validate_command("reboot").await
    }

    pub async fn get_preflight_info(&mut self) -> Result<IndexMap<String, XpcValue>, RestoreError> {
        self.send_command("getpreflightinfo", None).await
    }

    pub async fn get_nonces(&mut self) -> Result<IndexMap<String, XpcValue>, RestoreError> {
        self.send_command("getnonces", None).await
    }

    pub async fn get_app_parameters(&mut self) -> Result<IndexMap<String, XpcValue>, RestoreError> {
        self.validate_command("getappparameters").await
    }

    pub async fn restore_lang(
        &mut self,
        language: impl Into<String>,
    ) -> Result<IndexMap<String, XpcValue>, RestoreError> {
        self.send_command("restorelang", Some(XpcValue::String(language.into())))
            .await
    }

    async fn validate_command(
        &mut self,
        command: &str,
    ) -> Result<IndexMap<String, XpcValue>, RestoreError> {
        let response = self.send_command(command, None).await?;
        ensure_success(&response)?;
        Ok(response)
    }

    async fn send_command(
        &mut self,
        command: &str,
        argument: Option<XpcValue>,
    ) -> Result<IndexMap<String, XpcValue>, RestoreError> {
        let request = crate::xpc::message::encode_message(&XpcMessage {
            flags: crate::xpc::message::flags::ALWAYS_SET
                | crate::xpc::message::flags::DATA
                | crate::xpc::message::flags::WANTING_REPLY,
            msg_id: self.next_msg_id,
            body: Some(build_command_request(command, argument)),
        })
        .map_err(|err| RestoreError::Protocol(format!("restore request encode failed: {err}")))?;
        self.framer
            .write_client_server(&request)
            .await
            .map_err(|err| RestoreError::Protocol(format!("restore request failed: {err}")))?;
        self.next_msg_id += 1;
        let response = self.recv_control_message().await?;
        response_dict(response)
    }

    async fn recv_control_message(&mut self) -> Result<XpcMessage, RestoreError> {
        loop {
            if let Some(message) = self.try_take_pending_control_message()? {
                if message.flags & crate::xpc::message::flags::FILE_TX_STREAM_REQUEST != 0 {
                    continue;
                }
                if message.body.is_none() {
                    continue;
                }
                if message
                    .body
                    .as_ref()
                    .and_then(XpcValue::as_dict)
                    .is_some_and(|dict| dict.is_empty())
                {
                    continue;
                }
                return Ok(message);
            }

            let frame = self.framer.read_next_data_frame().await.map_err(|err| {
                RestoreError::Protocol(format!("restore response read failed: {err}"))
            })?;
            self.pending_control_data.extend_from_slice(&frame.payload);
        }
    }

    fn try_take_pending_control_message(&mut self) -> Result<Option<XpcMessage>, RestoreError> {
        if self.pending_control_data.len() < 24 {
            return Ok(None);
        }

        let body_len = u64::from_le_bytes(
            self.pending_control_data[8..16]
                .try_into()
                .map_err(|_| RestoreError::Protocol("invalid XPC response header".into()))?,
        ) as usize;
        let total_len = 24usize
            .checked_add(body_len)
            .ok_or_else(|| RestoreError::Protocol("XPC response length overflow".into()))?;
        if self.pending_control_data.len() < total_len {
            return Ok(None);
        }

        let payload = self.pending_control_data.split_to(total_len).freeze();
        let message = crate::xpc::message::decode_message(payload).map_err(|err| {
            RestoreError::Protocol(format!("restore response decode failed: {err}"))
        })?;
        Ok(Some(message))
    }
}

fn build_command_request(command: &str, argument: Option<XpcValue>) -> XpcValue {
    let mut dict = IndexMap::new();
    dict.insert("command".to_string(), XpcValue::String(command.to_string()));
    if let Some(argument) = argument {
        dict.insert("argument".to_string(), argument);
    }
    XpcValue::Dictionary(dict)
}

fn response_dict(response: XpcMessage) -> Result<IndexMap<String, XpcValue>, RestoreError> {
    response
        .body
        .and_then(|value| match value {
            XpcValue::Dictionary(dict) => Some(dict),
            _ => None,
        })
        .ok_or_else(|| RestoreError::Protocol("restore response missing dictionary body".into()))
}

fn ensure_success(response: &IndexMap<String, XpcValue>) -> Result<(), RestoreError> {
    match response.get("result").and_then(XpcValue::as_str) {
        Some("success") => Ok(()),
        Some(other) => Err(RestoreError::Protocol(format!(
            "restore command failed with result '{other}': {}",
            serde_json::to_string(&xpc_value_to_json(&XpcValue::Dictionary(response.clone())))
                .unwrap_or_else(|_| "null".into())
        ))),
        None => Err(RestoreError::Protocol(format!(
            "restore response missing result: {}",
            serde_json::to_string(&xpc_value_to_json(&XpcValue::Dictionary(response.clone())))
                .unwrap_or_else(|_| "null".into())
        ))),
    }
}

async fn bootstrap_remote_xpc<S>(framer: &mut H2Framer<S>) -> Result<(), RestoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    framer
        .write_client_server(
            &crate::xpc::message::encode_message(&XpcMessage {
                flags: crate::xpc::message::flags::ALWAYS_SET
                    | crate::xpc::message::flags::DATA_PRESENT,
                msg_id: 0,
                body: Some(XpcValue::Dictionary(IndexMap::new())),
            })
            .map_err(|err| {
                RestoreError::Protocol(format!("remote XPC bootstrap encode step 1 failed: {err}"))
            })?,
        )
        .await
        .map_err(|err| {
            RestoreError::Protocol(format!("remote XPC bootstrap step 1 failed: {err}"))
        })?;

    framer
        .write_client_server(
            &crate::xpc::message::encode_message(&XpcMessage {
                flags: crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::REPLY,
                msg_id: 0,
                body: None,
            })
            .map_err(|err| {
                RestoreError::Protocol(format!("remote XPC bootstrap encode step 2 failed: {err}"))
            })?,
        )
        .await
        .map_err(|err| {
            RestoreError::Protocol(format!("remote XPC bootstrap step 2 failed: {err}"))
        })?;

    framer
        .write_server_client(
            &crate::xpc::message::encode_message(&XpcMessage {
                flags: crate::xpc::message::flags::ALWAYS_SET
                    | crate::xpc::message::flags::INIT_HANDSHAKE,
                msg_id: 0,
                body: None,
            })
            .map_err(|err| {
                RestoreError::Protocol(format!("remote XPC bootstrap encode step 3 failed: {err}"))
            })?,
        )
        .await
        .map_err(|err| {
            RestoreError::Protocol(format!("remote XPC bootstrap step 3 failed: {err}"))
        })?;

    Ok(())
}

pub fn xpc_value_to_json(value: &XpcValue) -> serde_json::Value {
    match value {
        XpcValue::Null => serde_json::Value::Null,
        XpcValue::Bool(value) => serde_json::Value::Bool(*value),
        XpcValue::Int64(value) => serde_json::Value::from(*value),
        XpcValue::Uint64(value) => serde_json::Value::from(*value),
        XpcValue::Double(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        XpcValue::Date(value) => serde_json::Value::from(*value),
        XpcValue::Data(bytes) => serde_json::Value::String(hex::encode(bytes)),
        XpcValue::String(value) => serde_json::Value::String(value.clone()),
        XpcValue::Uuid(bytes) => {
            serde_json::Value::String(uuid::Uuid::from_bytes(*bytes).to_string())
        }
        XpcValue::Array(values) => {
            serde_json::Value::Array(values.iter().map(xpc_value_to_json).collect())
        }
        XpcValue::Dictionary(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), xpc_value_to_json(value)))
                .collect(),
        ),
        XpcValue::FileTransfer { msg_id, data } => serde_json::json!({
            "msg_id": msg_id,
            "data": xpc_value_to_json(data),
        }),
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use tokio::io::{AsyncRead, AsyncWrite};

    use super::*;

    #[test]
    fn builds_enter_recovery_command_request() {
        let request = build_command_request("recovery", None);
        let dict = request.as_dict().expect("restore requests should be dicts");
        assert_eq!(
            dict.get("command").and_then(XpcValue::as_str),
            Some("recovery")
        );
    }

    #[test]
    fn builds_argumented_command_request() {
        let request = build_command_request("restorelang", Some(XpcValue::String("en".into())));
        let dict = request.as_dict().expect("restore requests should be dicts");
        assert_eq!(dict.get("argument").and_then(XpcValue::as_str), Some("en"));
    }

    #[test]
    fn converts_xpc_values_to_json() {
        let value = XpcValue::Dictionary(IndexMap::from([
            ("result".to_string(), XpcValue::String("success".into())),
            (
                "nonce".to_string(),
                XpcValue::Data(Bytes::from_static(&[0x12, 0x34])),
            ),
        ]));

        let json = xpc_value_to_json(&value);
        assert_eq!(json["result"], "success");
        assert_eq!(json["nonce"], "1234");
    }

    #[test]
    fn rejects_non_success_restore_result() {
        let response = IndexMap::from([
            ("result".to_string(), XpcValue::String("failure".into())),
            ("error".to_string(), XpcValue::String("denied".into())),
        ]);

        let err = ensure_success(&response).expect_err("non-success must fail");
        assert!(err.to_string().contains("failure"));
        assert!(err.to_string().contains("denied"));
    }

    #[tokio::test]
    async fn get_nonces_roundtrips_over_remote_xpc_stream() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);

        let server_task = tokio::spawn(async move {
            perform_h2_handshake(&mut server).await;
            perform_remote_xpc_bootstrap(&mut server).await;

            let request = read_xpc_request(&mut server, 1).await;
            let dict = request
                .body
                .expect("restore request body")
                .as_dict()
                .expect("restore request dict")
                .clone();
            assert_eq!(dict["command"].as_str(), Some("getnonces"));

            write_xpc_response(
                &mut server,
                3,
                XpcValue::Dictionary(IndexMap::from([
                    (
                        "ApNonce".to_string(),
                        XpcValue::Data(Bytes::from_static(&[0xAA, 0xBB])),
                    ),
                    (
                        "SEPNonce".to_string(),
                        XpcValue::Data(Bytes::from_static(&[0xCC, 0xDD])),
                    ),
                ])),
            )
            .await;
        });

        let mut client = RestoreServiceClient::connect(client)
            .await
            .expect("restore client should connect");
        let response = client.get_nonces().await.expect("nonces should succeed");

        assert_eq!(
            response.get("ApNonce"),
            Some(&XpcValue::Data(Bytes::from_static(&[0xAA, 0xBB])))
        );
        assert_eq!(
            response.get("SEPNonce"),
            Some(&XpcValue::Data(Bytes::from_static(&[0xCC, 0xDD])))
        );

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn get_nonces_skips_empty_dictionary_control_messages() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);

        let server_task = tokio::spawn(async move {
            perform_h2_handshake(&mut server).await;
            perform_remote_xpc_bootstrap(&mut server).await;

            let request = read_xpc_request(&mut server, 1).await;
            let dict = request
                .body
                .expect("restore request body")
                .as_dict()
                .expect("restore request dict")
                .clone();
            assert_eq!(dict["command"].as_str(), Some("getnonces"));

            write_xpc_response(&mut server, 1, XpcValue::Dictionary(IndexMap::new())).await;
            write_xpc_response(
                &mut server,
                1,
                XpcValue::Dictionary(IndexMap::from([(
                    "ApNonce".to_string(),
                    XpcValue::Data(Bytes::from_static(&[0xAA, 0xBB])),
                )])),
            )
            .await;
        });

        let mut client = RestoreServiceClient::connect(client)
            .await
            .expect("restore client should connect");
        let response = client.get_nonces().await.expect("nonces should succeed");

        assert_eq!(
            response.get("ApNonce"),
            Some(&XpcValue::Data(Bytes::from_static(&[0xAA, 0xBB])))
        );

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn reboot_validates_success_response() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);

        let server_task = tokio::spawn(async move {
            perform_h2_handshake(&mut server).await;
            perform_remote_xpc_bootstrap(&mut server).await;

            let request = read_xpc_request(&mut server, 1).await;
            let dict = request
                .body
                .expect("restore request body")
                .as_dict()
                .expect("restore request dict")
                .clone();
            assert_eq!(dict["command"].as_str(), Some("reboot"));
            assert_eq!(dict.get("argument"), None);

            write_xpc_response(
                &mut server,
                3,
                XpcValue::Dictionary(IndexMap::from([(
                    "result".to_string(),
                    XpcValue::String("success".into()),
                )])),
            )
            .await;
        });

        let mut client = RestoreServiceClient::connect(client)
            .await
            .expect("restore client should connect");
        let response = client.reboot().await.expect("reboot should succeed");

        assert_eq!(
            response.get("result").and_then(XpcValue::as_str),
            Some("success")
        );

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn restore_lang_sends_language_argument() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);

        let server_task = tokio::spawn(async move {
            perform_h2_handshake(&mut server).await;
            perform_remote_xpc_bootstrap(&mut server).await;

            let request = read_xpc_request(&mut server, 1).await;
            let dict = request
                .body
                .expect("restore request body")
                .as_dict()
                .expect("restore request dict")
                .clone();
            assert_eq!(dict["command"].as_str(), Some("restorelang"));
            assert_eq!(dict["argument"].as_str(), Some("en"));

            write_xpc_response(
                &mut server,
                3,
                XpcValue::Dictionary(IndexMap::from([(
                    "language".to_string(),
                    XpcValue::String("en".into()),
                )])),
            )
            .await;
        });

        let mut client = RestoreServiceClient::connect(client)
            .await
            .expect("restore client should connect");
        let response = client
            .restore_lang("en")
            .await
            .expect("restore lang should succeed");

        assert_eq!(
            response.get("language").and_then(XpcValue::as_str),
            Some("en")
        );

        server_task.await.unwrap();
    }

    async fn perform_h2_handshake<S>(stream: &mut S)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut preface = [0u8; 24];
        tokio::io::AsyncReadExt::read_exact(stream, &mut preface)
            .await
            .unwrap();
        assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

        let settings = read_raw_frame(stream).await;
        assert_eq!(settings.frame_type, 0x04);

        let window_update = read_raw_frame(stream).await;
        assert_eq!(window_update.frame_type, 0x08);

        write_raw_frame(stream, 0x04, 0, 0, &[]).await;

        let settings_ack = read_raw_frame(stream).await;
        assert_eq!(settings_ack.frame_type, 0x04);
        assert_eq!(settings_ack.flags, 0x01);
    }

    async fn perform_remote_xpc_bootstrap<S>(stream: &mut S)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        read_headers_frame(stream, 1).await;
        let _ = read_xpc_request(stream, 1).await;
        write_empty_xpc(stream, 1).await;

        let _ = read_xpc_request(stream, 1).await;
        write_empty_xpc(stream, 1).await;

        read_headers_frame(stream, 3).await;
        let _ = read_xpc_request(stream, 3).await;
        write_empty_xpc(stream, 3).await;
    }

    async fn read_headers_frame<S>(stream: &mut S, stream_id: u32)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let frame = read_raw_frame(stream).await;
        assert_eq!(frame.frame_type, 0x01);
        assert_eq!(frame.flags, 0x04);
        assert_eq!(frame.stream_id, stream_id);
    }

    async fn read_xpc_request<S>(stream: &mut S, stream_id: u32) -> XpcMessage
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let frame = read_raw_frame(stream).await;
        assert_eq!(frame.frame_type, 0x00);
        assert_eq!(frame.stream_id, stream_id);
        crate::xpc::message::decode_message(bytes::Bytes::from(frame.payload)).unwrap()
    }

    async fn write_empty_xpc<S>(stream: &mut S, stream_id: u32)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        write_raw_frame(
            stream,
            0x00,
            0,
            stream_id,
            &crate::xpc::message::encode_message(&XpcMessage {
                flags: crate::xpc::message::flags::ALWAYS_SET,
                msg_id: 0,
                body: None,
            })
            .unwrap(),
        )
        .await;
    }

    async fn write_xpc_response<S>(stream: &mut S, stream_id: u32, body: XpcValue)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        write_raw_frame(
            stream,
            0x00,
            0,
            stream_id,
            &crate::xpc::message::encode_message(&XpcMessage {
                flags: crate::xpc::message::flags::ALWAYS_SET
                    | crate::xpc::message::flags::DATA
                    | crate::xpc::message::flags::REPLY,
                msg_id: 1,
                body: Some(body),
            })
            .unwrap(),
        )
        .await;
    }

    async fn write_raw_frame<S>(
        stream: &mut S,
        frame_type: u8,
        flags: u8,
        stream_id: u32,
        payload: &[u8],
    ) where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let len = payload.len();
        let mut frame = Vec::with_capacity(9 + len);
        frame.push(((len >> 16) & 0xff) as u8);
        frame.push(((len >> 8) & 0xff) as u8);
        frame.push((len & 0xff) as u8);
        frame.push(frame_type);
        frame.push(flags);
        frame.extend_from_slice(&(stream_id & 0x7fff_ffff).to_be_bytes());
        frame.extend_from_slice(payload);
        tokio::io::AsyncWriteExt::write_all(stream, &frame)
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::flush(stream).await.unwrap();
    }

    async fn read_raw_frame<S>(stream: &mut S) -> TestFrame
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut header = [0u8; 9];
        tokio::io::AsyncReadExt::read_exact(stream, &mut header)
            .await
            .unwrap();
        let len = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
        let mut payload = vec![0u8; len];
        if len > 0 {
            tokio::io::AsyncReadExt::read_exact(stream, &mut payload)
                .await
                .unwrap();
        }
        TestFrame {
            frame_type: header[3],
            flags: header[4],
            stream_id: u32::from_be_bytes([header[5] & 0x7f, header[6], header[7], header[8]]),
            payload,
        }
    }

    struct TestFrame {
        frame_type: u8,
        flags: u8,
        stream_id: u32,
        payload: Vec<u8>,
    }
}
