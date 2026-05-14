//! File relay service client.
//!
//! Service: `com.apple.mobile.file_relay`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

pub const SERVICE_NAME: &str = "com.apple.mobile.file_relay";

service_error!(FileRelayError);

impl From<super::plist_frame::PlistFrameError> for FileRelayError {
    fn from(error: super::plist_frame::PlistFrameError) -> Self {
        match error {
            super::plist_frame::PlistFrameError::Io(error) => Self::Io(error),
            super::plist_frame::PlistFrameError::Plist(error) => Self::Plist(error),
            super::plist_frame::PlistFrameError::Protocol(message) => Self::Protocol(message),
        }
    }
}

pub struct FileRelayClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> FileRelayClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn request_sources(&mut self, sources: &[&str]) -> Result<Vec<u8>, FileRelayError> {
        self.send_request_sources(sources).await?;
        let mut data = Vec::new();
        self.stream.read_to_end(&mut data).await?;
        Ok(data)
    }

    pub async fn request_sources_to_writer<W>(
        &mut self,
        sources: &[&str],
        writer: &mut W,
    ) -> Result<u64, FileRelayError>
    where
        W: AsyncWrite + Unpin,
    {
        self.send_request_sources(sources).await?;
        tokio::io::copy(&mut self.stream, writer)
            .await
            .map_err(FileRelayError::from)
    }

    async fn send_request_sources(&mut self, sources: &[&str]) -> Result<(), FileRelayError> {
        let request = plist::Dictionary::from_iter([(
            "Sources".to_string(),
            plist::Value::Array(
                sources
                    .iter()
                    .map(|source| plist::Value::String((*source).to_string()))
                    .collect(),
            ),
        )]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        let response = recv_plist(&mut self.stream).await?;
        match response.get("Status").and_then(plist::Value::as_string) {
            Some("Acknowledged") => {}
            Some(other) => {
                let error = response
                    .get("Error")
                    .and_then(plist::Value::as_string)
                    .unwrap_or(other);
                return Err(FileRelayError::Protocol(error.to_string()));
            }
            None => {
                return Err(FileRelayError::Protocol(
                    "file relay response missing Status".into(),
                ));
            }
        }
        Ok(())
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), FileRelayError> {
    const MAX_PLIST_SIZE: usize = 1024 * 1024;
    super::plist_frame::write_xml_plist_frame(stream, value, MAX_PLIST_SIZE)
        .await
        .map_err(FileRelayError::from)
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, FileRelayError> {
    const MAX_PLIST_SIZE: usize = 1024 * 1024;
    super::plist_frame::read_plist_frame(stream, MAX_PLIST_SIZE)
        .await
        .map_err(FileRelayError::from)
}

#[cfg(test)]
mod tests {
    use crate::test_util::MockStream;

    use super::*;

    #[tokio::test]
    async fn request_sources_reads_acknowledged_archive() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream =
            MockStream::with_plist_response_and_trailing_bytes(response, b"archive-bytes");
        let mut client = FileRelayClient::new(&mut stream);

        let archive = client.request_sources(&["Network"]).await.unwrap();
        assert_eq!(archive, b"archive-bytes");

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        let sources = dict["Sources"].as_array().unwrap();
        assert_eq!(sources[0].as_string(), Some("Network"));
    }

    #[tokio::test]
    async fn request_sources_to_writer_streams_acknowledged_archive() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream =
            MockStream::with_plist_response_and_trailing_bytes(response, b"archive-bytes");
        let mut client = FileRelayClient::new(&mut stream);
        let mut output = Vec::new();

        let bytes = client
            .request_sources_to_writer(&["Network"], &mut output)
            .await
            .unwrap();

        assert_eq!(bytes, 13);
        assert_eq!(output, b"archive-bytes");
    }
}
