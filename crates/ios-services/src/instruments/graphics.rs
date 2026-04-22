use tokio::io::{AsyncRead, AsyncWrite};

use crate::dtx::codec::{DtxConnection, DtxError};
use crate::dtx::primitive_enc::archived_object;
use crate::dtx::types::{DtxMessage, DtxPayload};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct GraphicsSample {
    pub payload: serde_json::Value,
}

pub struct GraphicsMonitorClient<S> {
    conn: DtxConnection<S>,
    channel_code: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> GraphicsMonitorClient<S> {
    pub async fn connect(stream: S) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let channel_code = conn.request_channel(super::GRAPHICS_MONITOR_SVC).await?;
        conn.method_call(
            channel_code,
            "startSamplingAtTimeInterval:",
            &[archived_object(
                ios_proto::nskeyedarchiver_encode::archive_float(0.0),
            )],
        )
        .await?;
        Ok(Self { conn, channel_code })
    }

    pub async fn next_sample(&mut self) -> Result<GraphicsSample, DtxError> {
        loop {
            let msg = self.conn.recv().await?;
            if msg.expects_reply {
                self.conn.send_ack(&msg).await?;
            }
            if let Some(sample) = parse_graphics_message(&msg)? {
                return Ok(sample);
            }
        }
    }

    pub async fn stop(&mut self) -> Result<(), DtxError> {
        self.conn
            .method_call_async(self.channel_code, "stopSampling", &[])
            .await
    }
}

fn parse_graphics_message(msg: &DtxMessage) -> Result<Option<GraphicsSample>, DtxError> {
    let payload = match &msg.payload {
        DtxPayload::Response(value) => value.clone(),
        DtxPayload::Raw(bytes) => match super::unarchive_raw_payload(bytes) {
            Some(value) => value,
            None => return Ok(None),
        },
        DtxPayload::MethodInvocation { args, .. } if !args.is_empty() => {
            let payload = if args.len() == 1 {
                super::nsobject_to_json(&args[0])
            } else {
                serde_json::Value::Array(args.iter().map(super::nsobject_to_json).collect())
            };
            if is_capabilities_payload(&payload) {
                return Ok(None);
            }
            return Ok(Some(GraphicsSample { payload }));
        }
        _ => return Ok(None),
    };

    let payload = super::nsobject_to_json(&payload);
    if is_capabilities_payload(&payload) {
        return Ok(None);
    }

    Ok(Some(GraphicsSample { payload }))
}

fn is_capabilities_payload(payload: &serde_json::Value) -> bool {
    if let Some(object) = payload.as_object() {
        return object.keys().any(|key| key.starts_with("com.apple."));
    }
    let Some(items) = payload.as_array() else {
        return false;
    };
    let Some(first) = items.first().and_then(serde_json::Value::as_object) else {
        return false;
    };
    first.keys().any(|key| key.starts_with("com.apple."))
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::dtx::types::NSObject;

    #[test]
    fn parses_graphics_response_payload() {
        let msg = DtxMessage {
            identifier: 1,
            conversation_idx: 0,
            channel_code: 9,
            expects_reply: false,
            payload: DtxPayload::Response(NSObject::Dict(IndexMap::from_iter([
                ("fps".to_string(), NSObject::Double(59.8)),
                ("renderer".to_string(), NSObject::String("AGX".into())),
            ]))),
        };

        let sample = parse_graphics_message(&msg).unwrap().unwrap();
        assert_eq!(sample.payload["fps"], serde_json::json!(59.8));
        assert_eq!(sample.payload["renderer"], serde_json::json!("AGX"));
    }
}
