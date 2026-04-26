//! Minimal House Arrest client for vending an app container and then
//! reusing the returned stream as AFC.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::services::afc::{AfcClient, AfcError};

pub const SERVICE_NAME: &str = "com.apple.mobile.house_arrest";

#[derive(Debug, thiserror::Error)]
pub enum HouseArrestError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("house arrest error: {0}")]
    Service(String),
}

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
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> HouseArrestClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

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
        self.send_plist(&VendContainerRequest {
            command,
            identifier: bundle_id,
        })
        .await?;

        let response: VendContainerResponse = self.recv_plist().await?;
        match response.status.as_deref() {
            Some("Complete") => Ok(AfcClient::new(self.stream)),
            Some(status) => Err(HouseArrestError::Service(status.to_string())),
            None => {
                if let Some(error) = response.error {
                    Err(HouseArrestError::Service(error))
                } else {
                    Err(HouseArrestError::Protocol(
                        "unknown house arrest response".into(),
                    ))
                }
            }
        }
    }

    async fn send_plist<T: Serialize>(&mut self, value: &T) -> Result<(), HouseArrestError> {
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, value)
            .map_err(|e| HouseArrestError::Plist(e.to_string()))?;
        let len = buf.len() as u32;
        self.stream.write_all(&len.to_be_bytes()).await?;
        self.stream.write_all(&buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn recv_plist<T>(&mut self) -> Result<T, HouseArrestError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
        if len > MAX_PLIST_SIZE {
            return Err(HouseArrestError::Protocol(format!(
                "plist length {len} exceeds max {MAX_PLIST_SIZE}"
            )));
        }
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;
        plist::from_bytes(&buf).map_err(|e| HouseArrestError::Plist(e.to_string()))
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
}
