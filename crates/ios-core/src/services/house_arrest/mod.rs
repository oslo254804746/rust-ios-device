//! Minimal House Arrest client for vending an app container and then
//! reusing the returned stream as AFC.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    time::Instant,
};

use crate::services::afc::{AfcClient, AfcError};

pub const SERVICE_NAME: &str = "com.apple.mobile.house_arrest";

service_error!(
    HouseArrestError,
    #[error("house arrest error: {0}")]
    Service(String),
);

impl From<AfcError> for HouseArrestError {
    fn from(err: AfcError) -> Self {
        match err {
            AfcError::Io(e) => Self::Io(e),
            AfcError::Status(code) => Self::Service(code.to_string()),
            AfcError::Protocol(msg) => Self::Protocol(msg),
        }
    }
}

#[derive(Debug)]
pub struct HouseArrestClient<S> {
    pub(crate) stream: Option<S>,
    usable: bool,
    operation_timeout: Option<Duration>,
}

impl<S> HouseArrestClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream: Some(stream),
            usable: true,
            operation_timeout: None,
        }
    }

    pub fn is_usable(&self) -> bool {
        self.usable && self.stream.is_some()
    }

    pub fn set_operation_timeout(&mut self, timeout: Option<Duration>) {
        self.operation_timeout = timeout;
    }

    pub fn with_operation_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.set_operation_timeout(timeout);
        self
    }

    fn ensure_usable(&self) -> Result<(), HouseArrestError> {
        if self.is_usable() {
            Ok(())
        } else {
            Err(HouseArrestError::Protocol(
                "house arrest connection is unusable; reconnect before another request".into(),
            ))
        }
    }

    fn deadline(&self) -> Result<Option<Instant>, HouseArrestError> {
        self.ensure_usable()?;
        match self.operation_timeout {
            Some(budget) if budget.is_zero() => Err(elapsed_budget()),
            Some(budget) => Instant::now().checked_add(budget).map(Some).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "house arrest operation deadline is too large",
                )
                .into()
            }),
            None => Ok(None),
        }
    }

    fn begin_request(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<HouseArrestRequest<'_, S>, HouseArrestError> {
        self.ensure_usable()?;
        let stream = self.stream.take();
        Ok(HouseArrestRequest {
            client: self,
            stream,
            deadline,
            progressed: false,
            failed: false,
        })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> HouseArrestClient<S> {
    pub async fn vend_container(self, bundle_id: &str) -> Result<AfcClient<S>, HouseArrestError> {
        self.send_command("VendContainer", bundle_id).await
    }

    pub async fn vend_documents(self, bundle_id: &str) -> Result<AfcClient<S>, HouseArrestError> {
        self.send_command("VendDocuments", bundle_id).await
    }

    async fn send_command(
        mut self,
        command: &'static str,
        bundle_id: &str,
    ) -> Result<AfcClient<S>, HouseArrestError> {
        let deadline = self.deadline()?;
        check_deadline(deadline, Ok(()))?;
        let mut request = self.begin_request(deadline)?;
        let vend_req = VendContainerRequest {
            command,
            identifier: bundle_id,
        };
        let result: Result<VendContainerResponse, HouseArrestError> =
            await_operation(deadline, async {
                let mut io = request.io();
                let mut buf = Vec::new();
                plist::to_writer_xml(&mut buf, &vend_req)?;
                let len = buf.len() as u32;
                io.write_all(&len.to_be_bytes()).await?;
                io.write_all(&buf).await?;
                io.flush().await?;

                let mut len_buf = [0u8; 4];
                io.read_exact(&mut len_buf).await?;
                let len = u32::from_be_bytes(len_buf) as usize;
                const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
                if len > MAX_PLIST_SIZE {
                    return Err(HouseArrestError::Protocol(format!(
                        "plist length {len} exceeds max {MAX_PLIST_SIZE}"
                    )));
                }
                let mut resp_buf = vec![0u8; len];
                io.read_exact(&mut resp_buf).await?;
                Ok(plist::from_bytes(&resp_buf)?)
            })
            .await;

        let response = check_deadline(deadline, result);
        match response {
            Ok(resp) => match resp.status.as_deref() {
                Some("Complete") => {
                    let stream = request.take_stream().expect("active stream for handoff");
                    Ok(AfcClient::new(stream))
                }
                Some(status) => request.finish(Err(HouseArrestError::Service(status.to_string()))),
                None => {
                    if let Some(error) = resp.error {
                        request.finish(Err(HouseArrestError::Service(error)))
                    } else {
                        request.finish(Err(HouseArrestError::Protocol(
                            "unknown house arrest response".into(),
                        )))
                    }
                }
            },
            Err(e) => request.finish(Err(e)),
        }
    }
}

struct HouseArrestRequest<'a, S> {
    client: &'a mut HouseArrestClient<S>,
    stream: Option<S>,
    deadline: Option<Instant>,
    progressed: bool,
    failed: bool,
}

impl<S> HouseArrestRequest<'_, S> {
    fn io(&mut self) -> HouseArrestIo<'_, S> {
        HouseArrestIo {
            inner: self.stream.as_mut().expect("active house arrest stream"),
            deadline: self.deadline,
            progressed: &mut self.progressed,
            failed: &mut self.failed,
        }
    }

    fn take_stream(&mut self) -> Option<S> {
        self.stream.take()
    }

    fn finish<T>(mut self, result: Result<T, HouseArrestError>) -> Result<T, HouseArrestError> {
        if !self.failed && (result.is_ok() || !self.progressed) {
            self.client.stream = self.stream.take();
        } else {
            self.client.usable = false;
        }
        result
    }
}

impl<S> Drop for HouseArrestRequest<'_, S> {
    fn drop(&mut self) {
        if self.stream.is_none() {
            return;
        }
        if self.progressed || self.failed {
            self.client.usable = false;
        } else {
            self.client.stream = self.stream.take();
        }
    }
}

struct HouseArrestIo<'a, S> {
    inner: &'a mut S,
    deadline: Option<Instant>,
    progressed: &'a mut bool,
    failed: &'a mut bool,
}

impl<S> HouseArrestIo<'_, S> {
    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for HouseArrestIo<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let before = buf.filled().len();
        let requested = buf.remaining() > 0;
        let result = Pin::new(&mut *self.inner).poll_read(cx, buf);
        *self.progressed |= buf.filled().len() > before;
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (requested && matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() == before);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for HouseArrestIo<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_write(cx, bytes);
        *self.progressed |= matches!(result, Poll::Ready(Ok(n)) if n > 0);
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (!bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))));
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_flush(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_shutdown(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

fn elapsed_budget() -> HouseArrestError {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "house arrest operation budget elapsed",
    )
    .into()
}

fn check_deadline<T>(
    deadline: Option<Instant>,
    result: Result<T, HouseArrestError>,
) -> Result<T, HouseArrestError> {
    if result.is_ok() && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        Err(elapsed_budget())
    } else {
        result
    }
}

async fn await_operation<T>(
    deadline: Option<Instant>,
    operation: impl Future<Output = Result<T, HouseArrestError>>,
) -> Result<T, HouseArrestError> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, operation)
            .await
            .map_err(|error| {
                HouseArrestError::Io(crate::error::context::retained_io(
                    std::io::ErrorKind::TimedOut,
                    "house arrest operation deadline",
                    error,
                ))
            })?,
        None => operation.await,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct VendContainerRequest<'a> {
    command: &'static str,
    identifier: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VendContainerResponse {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::proto::afc::{AfcHeader, AfcOpcode};
    use zerocopy::{FromBytes, IntoBytes};

    use super::*;

    async fn read_plist_frame<S>(stream: &mut S) -> Vec<u8>
    where
        S: AsyncRead + Unpin,
    {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await.unwrap();
        buf
    }

    async fn read_afc_request<S>(stream: &mut S)
    where
        S: AsyncRead + Unpin,
    {
        let mut hdr_buf = [0u8; AfcHeader::SIZE];
        stream.read_exact(&mut hdr_buf).await.unwrap();
        let hdr = AfcHeader::ref_from_bytes(&hdr_buf).unwrap();
        let header_payload_len = hdr.this_len.get() as usize - AfcHeader::SIZE;
        let payload_len = hdr.entire_len.get() as usize - hdr.this_len.get() as usize;

        let mut header_payload = vec![0u8; header_payload_len];
        let mut payload = vec![0u8; payload_len];
        if header_payload_len > 0 {
            stream.read_exact(&mut header_payload).await.unwrap();
        }
        if payload_len > 0 {
            stream.read_exact(&mut payload).await.unwrap();
        }
        assert_eq!(hdr.operation.get(), AfcOpcode::ReadDir as u64);
    }

    #[test]
    fn test_service_name_matches_house_arrest() {
        assert_eq!(SERVICE_NAME, "com.apple.mobile.house_arrest");
    }

    #[tokio::test]
    async fn test_vend_container_returns_afc_client_over_same_stream() {
        let (client_side, mut server_side) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            let request = read_plist_frame(&mut server_side).await;
            let req_value: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = req_value.into_dictionary().unwrap();
            assert_eq!(
                dict.get("Command").and_then(|v| v.as_string()),
                Some("VendContainer")
            );
            assert_eq!(
                dict.get("Identifier").and_then(|v| v.as_string()),
                Some("com.example.TestApp")
            );

            let response = plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]);
            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &plist::Value::Dictionary(response)).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();

            read_afc_request(&mut server_side).await;

            let names = b".\0..\0Sandbox\0";
            let hdr = AfcHeader::new(1, AfcOpcode::ReadDir, 0, names.len());
            let mut resp = hdr.as_bytes().to_vec();
            resp.extend_from_slice(names);
            server_side.write_all(&resp).await.unwrap();
        });

        let client = HouseArrestClient::new(client_side);
        let mut afc = client.vend_container("com.example.TestApp").await.unwrap();
        let entries = afc.list_dir("/").await.unwrap();
        assert_eq!(entries, vec!["Sandbox"]);
    }

    #[tokio::test]
    async fn test_vend_documents_sends_expected_command() {
        let (client_side, mut server_side) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            let request = read_plist_frame(&mut server_side).await;
            let req_value: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = req_value.into_dictionary().unwrap();
            assert_eq!(
                dict.get("Command").and_then(|v| v.as_string()),
                Some("VendDocuments")
            );
            assert_eq!(
                dict.get("Identifier").and_then(|v| v.as_string()),
                Some("com.example.DocumentsApp")
            );

            let response = plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]);
            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &plist::Value::Dictionary(response)).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();

            read_afc_request(&mut server_side).await;

            let names = b".\0..\0Documents\0";
            let hdr = AfcHeader::new(1, AfcOpcode::ReadDir, 0, names.len());
            let mut resp = hdr.as_bytes().to_vec();
            resp.extend_from_slice(names);
            server_side.write_all(&resp).await.unwrap();
        });

        let client = HouseArrestClient::new(client_side);
        let mut afc = client
            .vend_documents("com.example.DocumentsApp")
            .await
            .unwrap();
        let entries = afc.list_dir("/").await.unwrap();
        assert_eq!(entries, vec!["Documents"]);
    }

    #[tokio::test]
    async fn test_house_arrest_zero_budget_fails_without_io() {
        let (client_side, _) = tokio::io::duplex(1024);
        let client =
            HouseArrestClient::new(client_side).with_operation_timeout(Some(Duration::ZERO));
        let Err(err) = client.vend_container("com.example.TestApp").await else {
            panic!("expected error");
        };
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
    }

    #[tokio::test]
    async fn test_house_arrest_progress_timeout_drops_stream() {
        let (client_side, mut server_side) = tokio::io::duplex(1024);
        let client = HouseArrestClient::new(client_side)
            .with_operation_timeout(Some(Duration::from_millis(50)));

        tokio::spawn(async move {
            let mut buf = [0u8; 4];
            let _ = server_side.read_exact(&mut buf).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        let Err(err) = client.vend_container("com.example.TestApp").await else {
            panic!("expected error");
        };
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
    }

    #[tokio::test]
    async fn test_house_arrest_error_priority_prefers_status_over_error() {
        let (client_side, mut server_side) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            let _ = read_plist_frame(&mut server_side).await;
            let response = plist::Dictionary::from_iter([
                (
                    "Status".to_string(),
                    plist::Value::String("InstallationLookupFailed".into()),
                ),
                (
                    "Error".to_string(),
                    plist::Value::String("SomeErrorDescription".into()),
                ),
            ]);
            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &plist::Value::Dictionary(response)).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();
        });

        let client = HouseArrestClient::new(client_side)
            .with_operation_timeout(Some(Duration::from_secs(5)));
        let Err(err) = client.vend_container("com.example.TestApp").await else {
            panic!("expected error");
        };
        // Priority: Status is preferred over Error field
        assert!(
            matches!(err, HouseArrestError::Service(status) if status == "InstallationLookupFailed")
        );
    }
}
