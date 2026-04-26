use tokio::io::{AsyncRead, AsyncWrite};

use crate::services::dtx::codec::{DtxConnection, DtxError};
use crate::services::dtx::primitive_enc::archived_object;
use crate::services::dtx::types::DtxPayload;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EnergySample {
    pub payload: serde_json::Value,
}

pub struct EnergyMonitorClient<S> {
    conn: DtxConnection<S>,
    channel_code: i32,
    pids: Vec<i32>,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> EnergyMonitorClient<S> {
    pub async fn connect(stream: S, pids: &[i32]) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let channel_code = conn.request_channel(super::ENERGY_MONITOR_SVC).await?;
        let mut client = Self {
            conn,
            channel_code,
            pids: pids.to_vec(),
        };
        client.stop_sampling().await?;
        client.start_sampling().await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(client)
    }

    pub async fn sample(&mut self) -> Result<EnergySample, DtxError> {
        let response = self
            .conn
            .method_call(
                self.channel_code,
                "sampleAttributes:forPIDs:",
                &[
                    archived_object(crate::proto::nskeyedarchiver_encode::archive_dict(vec![])),
                    archived_object(encode_pid_array(&self.pids)),
                ],
            )
            .await?;
        parse_energy_sample(&response.payload)
    }

    pub async fn stop_sampling(&mut self) -> Result<(), DtxError> {
        self.conn
            .method_call_async(
                self.channel_code,
                "stopSamplingForPIDs:",
                &[archived_object(encode_pid_array(&self.pids))],
            )
            .await
    }

    async fn start_sampling(&mut self) -> Result<(), DtxError> {
        self.conn
            .method_call_async(
                self.channel_code,
                "startSamplingForPIDs:",
                &[archived_object(encode_pid_array(&self.pids))],
            )
            .await
    }
}

fn encode_pid_array(pids: &[i32]) -> Vec<u8> {
    crate::proto::nskeyedarchiver_encode::archive_array(
        pids.iter()
            .map(|pid| plist::Value::Integer((*pid as i64).into()))
            .collect(),
    )
}

fn parse_energy_sample(payload: &DtxPayload) -> Result<EnergySample, DtxError> {
    let payload = match payload {
        DtxPayload::Response(value) => value.clone(),
        DtxPayload::Raw(bytes) => super::unarchive_raw_payload(bytes).ok_or_else(|| {
            DtxError::Protocol("energy response was not a valid archived payload".into())
        })?,
        other => {
            return Err(DtxError::Protocol(format!(
                "unexpected energy response payload: {other:?}"
            )))
        }
    };

    Ok(EnergySample {
        payload: super::nsobject_to_json(&payload),
    })
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::services::dtx::types::NSObject;

    #[test]
    fn parses_energy_sample_from_response_dict() {
        let payload = DtxPayload::Response(NSObject::Dict(IndexMap::from_iter([
            ("energy".to_string(), NSObject::Double(17.5)),
            (
                "processes".to_string(),
                NSObject::Array(vec![NSObject::Dict(IndexMap::from_iter([(
                    "pid".to_string(),
                    NSObject::Int(42),
                )]))]),
            ),
        ])));

        let sample = parse_energy_sample(&payload).unwrap();
        assert_eq!(sample.payload["energy"], serde_json::json!(17.5));
        assert_eq!(sample.payload["processes"][0]["pid"], serde_json::json!(42));
    }

    #[test]
    fn encodes_pid_array() {
        let encoded = encode_pid_array(&[12, 34]);
        let decoded = crate::proto::nskeyedarchiver::unarchive(&encoded).unwrap();
        let values = decoded.as_array().unwrap();
        assert_eq!(values[0].as_int(), Some(12));
        assert_eq!(values[1].as_int(), Some(34));
    }
}
