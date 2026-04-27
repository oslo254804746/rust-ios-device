//! DTX-based screenshot service (iOS 17+ via instruments, no DDI required).
//!
//! Service: `com.apple.instruments.server.services.screenshot`
//! Method: `takeScreenshot` → returns PNG bytes
//!
//! Reference: go-ios/ios/screenshot/screenshot.go

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::services::dtx::codec::{DtxConnection, DtxError};
use crate::services::dtx::types::{DtxPayload, NSObject};

/// Take a screenshot via the DTX instruments screenshot service.
///
/// Returns PNG image bytes.
pub async fn take_screenshot_dtx<S>(stream: S) -> Result<Bytes, DtxError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut conn = DtxConnection::new(stream);
    let ch = conn.request_channel(super::SCREENSHOT_SVC).await?;

    let msg = conn.method_call(ch, "takeScreenshot", &[]).await?;

    match msg.payload {
        DtxPayload::Response(NSObject::Data(data)) => Ok(data),
        DtxPayload::Response(NSObject::Dict(ref d)) => {
            // Some iOS versions wrap the PNG in a dict with "ScreenshotData" key
            if let Some(NSObject::Data(data)) = d.get("ScreenshotData") {
                Ok(data.clone())
            } else {
                Err(DtxError::Protocol(
                    "screenshot response dict missing ScreenshotData".into(),
                ))
            }
        }
        other => Err(DtxError::Protocol(format!(
            "unexpected screenshot response: {:?}",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{duplex, AsyncWriteExt};

    use super::*;
    use crate::services::dtx::{encode_dtx, read_dtx_frame, DtxPayload, NSObject};

    const MSG_RESPONSE: u32 = 3;

    #[tokio::test]
    async fn download_screenshot_accepts_wrapped_screenshot_data() {
        let (client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { take_screenshot_dtx(client).await.unwrap() });

        let channel_request = read_dtx_frame(&mut server).await.unwrap();
        match channel_request.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_requestChannelWithCode:identifier:");
                assert!(
                    matches!(args.get(1), Some(NSObject::String(name)) if name == super::super::SCREENSHOT_SVC)
                );
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

        let request = read_dtx_frame(&mut server).await.unwrap();
        match request.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "takeScreenshot");
                assert!(args.is_empty());
            }
            other => panic!("unexpected screenshot request: {other:?}"),
        }

        let payload = bytes::Bytes::from_static(b"png-bytes");
        let wrapped = crate::proto::nskeyedarchiver_encode::archive_dict(vec![(
            "ScreenshotData".to_string(),
            plist::Value::Data(payload.to_vec()),
        )]);
        server
            .write_all(&encode_dtx(
                request.identifier,
                1,
                request.channel_code,
                false,
                MSG_RESPONSE,
                &wrapped,
                &[],
            ))
            .await
            .unwrap();

        assert_eq!(task.await.unwrap(), payload);
    }

    #[tokio::test]
    async fn download_screenshot_rejects_dict_without_screenshot_data() {
        let (client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { take_screenshot_dtx(client).await });

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

        let request = read_dtx_frame(&mut server).await.unwrap();
        let wrapped = crate::proto::nskeyedarchiver_encode::archive_dict(vec![]);
        server
            .write_all(&encode_dtx(
                request.identifier,
                1,
                request.channel_code,
                false,
                MSG_RESPONSE,
                &wrapped,
                &[],
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().expect_err("missing data must fail");
        assert!(err
            .to_string()
            .contains("screenshot response dict missing ScreenshotData"));
    }
}
