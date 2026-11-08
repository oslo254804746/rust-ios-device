//! Location simulation over the Instruments DTX service (iOS 17+).

use crate::proto::nskeyedarchiver_encode::archive_float;
use crate::services::dtx::codec::{checked_reply, DtxConnection, DtxError};
use crate::services::dtx::primitive_enc::archived_object;
use tokio::io::{AsyncRead, AsyncWrite};

const SERVICE: &str = "com.apple.instruments.server.services.LocationSimulation";

pub struct LocationSimulationClient<S> {
    conn: DtxConnection<S>,
    channel: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> LocationSimulationClient<S> {
    pub async fn connect(stream: S) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let channel = conn.request_channel(SERVICE).await?;
        Ok(Self { conn, channel })
    }

    pub async fn set_location(&mut self, latitude: f64, longitude: f64) -> Result<(), DtxError> {
        let response = self
            .conn
            .method_call(
                self.channel,
                "simulateLocationWithLatitude:longitude:",
                &[
                    archived_object(archive_float(latitude)),
                    archived_object(archive_float(longitude)),
                ],
            )
            .await?;
        checked_reply(response)?;
        Ok(())
    }

    pub async fn reset_location(&mut self) -> Result<(), DtxError> {
        self.conn
            .method_call_async(self.channel, "stopLocationSimulation", &[])
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::dtx::codec::{decode_dtx_message_from_bytes, encode_dtx, read_dtx_frame};
    use crate::services::dtx::types::{DtxPayload, NSObject};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn location_uses_archived_coordinates_and_oneway_stop() {
        let (client, mut peer) = tokio::io::duplex(8192);
        let task = tokio::spawn(async move {
            let mut client = LocationSimulationClient::connect(client).await.unwrap();
            client.set_location(48.856614, 2.3522219).await.unwrap();
            client.reset_location().await.unwrap();
        });
        let channel = read_dtx_frame(&mut peer).await.unwrap();
        match channel.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_requestChannelWithCode:identifier:");
                assert!(matches!(args.get(1), Some(NSObject::String(value)) if value == SERVICE));
            }
            other => panic!("unexpected channel request: {other:?}"),
        }
        peer.write_all(&encode_dtx(
            channel.identifier,
            1,
            channel.channel_code,
            false,
            3,
            &[],
            &[],
        ))
        .await
        .unwrap();
        let mut header = [0; 32];
        peer.read_exact(&mut header).await.unwrap();
        let length = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
        let mut frame = header.to_vec();
        frame.resize(32 + length, 0);
        peer.read_exact(&mut frame[32..]).await.unwrap();
        let (message, _) = decode_dtx_message_from_bytes(&frame).unwrap().unwrap();
        assert!(message.expects_reply);
        match &message.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "simulateLocationWithLatitude:longitude:");
                assert_eq!(
                    args.as_slice(),
                    [NSObject::Double(48.856614), NSObject::Double(2.3522219)].as_slice()
                );
            }
            other => panic!("unexpected set request: {other:?}"),
        }
        // Each argument is a type-2 buffer containing an NSKeyedArchive,
        // rather than a raw DTX double primitive.
        let mut offset = 64;
        for _ in 0..2 {
            assert_eq!(
                u32::from_le_bytes(frame[offset..offset + 4].try_into().unwrap()),
                10
            );
            assert_eq!(
                u32::from_le_bytes(frame[offset + 4..offset + 8].try_into().unwrap()),
                2
            );
            let len =
                u32::from_le_bytes(frame[offset + 8..offset + 12].try_into().unwrap()) as usize;
            let archive = &frame[offset + 12..offset + 12 + len];
            assert!(crate::proto::nskeyedarchiver::unarchive(archive).is_ok());
            offset += 12 + len;
        }
        peer.write_all(&encode_dtx(
            message.identifier,
            1,
            message.channel_code,
            false,
            3,
            &[],
            &[],
        ))
        .await
        .unwrap();
        let stop = read_dtx_frame(&mut peer).await.unwrap();
        assert!(!stop.expects_reply);
        assert!(
            matches!(stop.payload, DtxPayload::MethodInvocation { ref selector, ref args } if selector == "stopLocationSimulation" && args.is_empty())
        );
        task.await.unwrap();
    }
}
