use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use super::unarchive_raw_payload;
use crate::services::dtx::codec::{DtxConnection, DtxError};
use crate::services::dtx::primitive_enc::archived_object;
use crate::services::dtx::types::{DtxPayload, NSObject};

#[derive(Debug, Clone)]
pub enum TapMessage {
    Data(Bytes),
    Plist(NSObject),
}

pub struct TapClient<S> {
    conn: DtxConnection<S>,
    channel_code: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> TapClient<S> {
    pub async fn connect(
        stream: S,
        service_name: &str,
        config: Vec<(String, plist::Value)>,
    ) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let channel_code = conn.request_channel(service_name).await?;
        conn.method_call_async(
            channel_code,
            "setConfig:",
            &[archived_object(
                crate::proto::nskeyedarchiver_encode::archive_dict(config),
            )],
        )
        .await?;
        conn.method_call_async(channel_code, "start", &[]).await?;

        let mut client = Self { conn, channel_code };
        client.wait_for_start_message().await?;
        Ok(client)
    }

    pub async fn next_message(&mut self) -> Result<TapMessage, DtxError> {
        loop {
            let msg = self.conn.recv().await?;
            if msg.expects_reply {
                self.conn.send_ack(&msg).await?;
            }
            if msg.channel_code != self.channel_code && msg.channel_code != -1 {
                continue;
            }
            if let Some(message) = payload_to_tap_message(msg.payload) {
                return Ok(message);
            }
        }
    }

    pub async fn stop(&mut self) -> Result<(), DtxError> {
        self.conn
            .method_call_async(self.channel_code, "stop", &[])
            .await
    }

    async fn wait_for_start_message(&mut self) -> Result<(), DtxError> {
        loop {
            match self.next_message().await? {
                TapMessage::Plist(_) => return Ok(()),
                TapMessage::Data(bytes) if unarchive_raw_payload(&bytes).is_some() => return Ok(()),
                TapMessage::Data(_) => {}
            }
        }
    }
}

fn payload_to_tap_message(payload: DtxPayload) -> Option<TapMessage> {
    match payload {
        DtxPayload::Raw(bytes) => Some(TapMessage::Data(bytes)),
        DtxPayload::RawWithAux { payload, aux } => aux
            .into_iter()
            .find_map(|value| match value {
                NSObject::Data(bytes) => Some(TapMessage::Data(bytes)),
                _ => None,
            })
            .or_else(|| (!payload.is_empty()).then_some(TapMessage::Data(payload))),
        DtxPayload::Response(value) => Some(TapMessage::Plist(value)),
        DtxPayload::MethodInvocation { args, .. } => Some(TapMessage::Plist(if args.len() == 1 {
            args.into_iter().next().unwrap_or(NSObject::Null)
        } else {
            NSObject::Array(args)
        })),
        DtxPayload::Notification { object, .. } => Some(TapMessage::Plist(object)),
        DtxPayload::Empty => None,
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{duplex, AsyncWriteExt};

    use super::*;
    use crate::services::dtx::{encode_dtx, read_dtx_frame, DtxPayload, NSObject};

    const MSG_RESPONSE: u32 = 3;
    const MSG_UNKNOWN_TYPE_ONE: u32 = 1;

    #[test]
    fn payload_to_tap_message_prefers_auxiliary_data_for_raw_with_aux() {
        let payload = DtxPayload::RawWithAux {
            payload: bytes::Bytes::from_static(b"body"),
            aux: vec![NSObject::Data(bytes::Bytes::from_static(b"aux"))],
        };

        let message = payload_to_tap_message(payload).expect("tap message");
        match message {
            TapMessage::Data(data) => assert_eq!(data.as_ref(), b"aux"),
            other => panic!("unexpected tap message: {other:?}"),
        }
    }

    #[test]
    fn payload_to_tap_message_wraps_multiple_method_invocation_args_as_array() {
        let message = payload_to_tap_message(DtxPayload::MethodInvocation {
            selector: "event".into(),
            args: vec![NSObject::String("a".into()), NSObject::Int(2)],
        })
        .expect("tap message");

        match message {
            TapMessage::Plist(NSObject::Array(values)) => {
                assert_eq!(values, vec![NSObject::String("a".into()), NSObject::Int(2)]);
            }
            other => panic!("unexpected tap plist payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn connect_sends_config_and_start_then_waits_for_plist_event() {
        let (client, mut server) = duplex(4096);
        let task = tokio::spawn(async move {
            TapClient::connect(
                client,
                "com.apple.instruments.server.services.exampletap",
                vec![("interval".to_string(), plist::Value::Integer(100.into()))],
            )
            .await
            .unwrap()
        });

        let channel_request = read_dtx_frame(&mut server).await.unwrap();
        match channel_request.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_requestChannelWithCode:identifier:");
                assert!(matches!(
                    args.get(1),
                    Some(NSObject::String(name))
                    if name == "com.apple.instruments.server.services.exampletap"
                ));
            }
            other => panic!("unexpected channel request: {other:?}"),
        }
        server
            .write_all(&encode_dtx(
                channel_request.identifier,
                1,
                0,
                false,
                MSG_RESPONSE,
                &[],
                &[],
            ))
            .await
            .unwrap();

        let set_config = read_dtx_frame(&mut server).await.unwrap();
        match set_config.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "setConfig:");
                assert!(matches!(
                    args.first(),
                    Some(NSObject::Dict(dict))
                    if dict.get("interval") == Some(&NSObject::Int(100))
                ));
            }
            other => panic!("unexpected setConfig request: {other:?}"),
        }

        let start = read_dtx_frame(&mut server).await.unwrap();
        match start.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "start");
                assert!(args.is_empty());
            }
            other => panic!("unexpected start request: {other:?}"),
        }

        let start_notice = crate::proto::nskeyedarchiver_encode::archive_string("started");
        server
            .write_all(&encode_dtx(
                99,
                0,
                -set_config.channel_code,
                false,
                MSG_RESPONSE,
                &start_notice,
                &[],
            ))
            .await
            .unwrap();

        let mut client = task.await.unwrap();
        server
            .write_all(&encode_dtx(
                100,
                0,
                -start.channel_code,
                false,
                MSG_UNKNOWN_TYPE_ONE,
                b"trace-bytes",
                &[],
            ))
            .await
            .unwrap();

        match client.next_message().await.unwrap() {
            TapMessage::Data(bytes) => assert_eq!(bytes.as_ref(), b"trace-bytes"),
            other => panic!("unexpected live tap message: {other:?}"),
        }
    }

    #[tokio::test]
    async fn connect_accepts_raw_archived_plist_as_start_ack() {
        let (client, mut server) = duplex(4096);
        let task = tokio::spawn(async move {
            TapClient::connect(
                client,
                "com.apple.instruments.server.services.exampletap",
                vec![("interval".to_string(), plist::Value::Integer(100.into()))],
            )
            .await
            .unwrap()
        });

        let channel_request = read_dtx_frame(&mut server).await.unwrap();
        server
            .write_all(&encode_dtx(
                channel_request.identifier,
                1,
                0,
                false,
                MSG_RESPONSE,
                &[],
                &[],
            ))
            .await
            .unwrap();

        let set_config = read_dtx_frame(&mut server).await.unwrap();
        match set_config.payload {
            DtxPayload::MethodInvocation { selector, .. } => {
                assert_eq!(selector, "setConfig:");
            }
            other => panic!("unexpected setConfig request: {other:?}"),
        }

        let start = read_dtx_frame(&mut server).await.unwrap();
        match start.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "start");
                assert!(args.is_empty());
            }
            other => panic!("unexpected start request: {other:?}"),
        }

        let start_notice = crate::proto::nskeyedarchiver_encode::archive_string("started");
        server
            .write_all(&encode_dtx(
                99,
                0,
                -start.channel_code,
                false,
                MSG_UNKNOWN_TYPE_ONE,
                &start_notice,
                &[],
            ))
            .await
            .unwrap();

        let mut client = task.await.unwrap();
        server
            .write_all(&encode_dtx(
                100,
                0,
                -start.channel_code,
                false,
                MSG_UNKNOWN_TYPE_ONE,
                b"trace-bytes",
                &[],
            ))
            .await
            .unwrap();

        match client.next_message().await.unwrap() {
            TapMessage::Data(bytes) => assert_eq!(bytes.as_ref(), b"trace-bytes"),
            other => panic!("unexpected live tap message: {other:?}"),
        }
    }
}
