use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};

service_error!(DeviceLinkError);

const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;

impl From<super::plist_frame::PlistFrameError> for DeviceLinkError {
    fn from(error: super::plist_frame::PlistFrameError) -> Self {
        match error {
            super::plist_frame::PlistFrameError::Io(error) => Self::Io(error),
            super::plist_frame::PlistFrameError::Plist(error) => Self::Plist(error),
            super::plist_frame::PlistFrameError::Protocol(message) => Self::Protocol(message),
        }
    }
}

pub struct DeviceLinkClient<S> {
    stream: S,
}

impl<S> DeviceLinkClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub fn into_inner(self) -> S {
        self.stream
    }

    pub fn stream_mut(&mut self) -> &mut S {
        &mut self.stream
    }
}

impl<S> DeviceLinkClient<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn version_exchange(&mut self) -> Result<u64, DeviceLinkError> {
        let response = self.recv_message().await?;
        let message = response.as_array().ok_or_else(|| {
            DeviceLinkError::Protocol(format!(
                "device link version exchange expected array, got {response:?}"
            ))
        })?;

        let message_type = message
            .first()
            .and_then(plist::Value::as_string)
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!(
                    "device link version exchange missing message type: {response:?}"
                ))
            })?;
        if message_type != "DLMessageVersionExchange" {
            return Err(DeviceLinkError::Protocol(format!(
                "expected DLMessageVersionExchange, got {message_type}"
            )));
        }

        let version = message
            .get(1)
            .and_then(|value| match value {
                plist::Value::Integer(value) => value.as_unsigned(),
                _ => None,
            })
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!(
                    "device link version exchange missing major version: {response:?}"
                ))
            })?;

        self.send_message(&vec![
            plist::Value::String("DLMessageVersionExchange".into()),
            plist::Value::String("DLVersionsOk".into()),
            plist::Value::Integer(version.into()),
        ])
        .await?;

        let ready = self.recv_message().await?;
        let ready_message = ready.as_array().ok_or_else(|| {
            DeviceLinkError::Protocol(format!("device ready expected array, got {ready:?}"))
        })?;
        let ready_type = ready_message
            .first()
            .and_then(plist::Value::as_string)
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!("device ready missing message type: {ready:?}"))
            })?;
        if ready_type != "DLMessageDeviceReady" {
            return Err(DeviceLinkError::Protocol(format!(
                "expected DLMessageDeviceReady, got {ready_type}"
            )));
        }

        Ok(version)
    }

    pub async fn send_process_message<T>(&mut self, message: &T) -> Result<(), DeviceLinkError>
    where
        T: Serialize,
    {
        self.send_message(&("DLMessageProcessMessage", message))
            .await
    }

    pub async fn recv_process_message(&mut self) -> Result<plist::Dictionary, DeviceLinkError> {
        let response = self.recv_message().await?;
        let message = response.as_array().ok_or_else(|| {
            DeviceLinkError::Protocol(format!("process message expected array, got {response:?}"))
        })?;

        let message_type = message
            .first()
            .and_then(plist::Value::as_string)
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!(
                    "process message missing message type: {response:?}"
                ))
            })?;
        if message_type != "DLMessageProcessMessage" {
            return Err(DeviceLinkError::Protocol(format!(
                "expected DLMessageProcessMessage, got {message_type}"
            )));
        }

        message
            .get(1)
            .and_then(plist::Value::as_dictionary)
            .cloned()
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!(
                    "process message missing dictionary payload: {response:?}"
                ))
            })
    }

    pub async fn send_message<T>(&mut self, message: &T) -> Result<(), DeviceLinkError>
    where
        T: Serialize,
    {
        super::plist_frame::write_xml_plist_frame(&mut self.stream, message, MAX_PLIST_SIZE)
            .await
            .map_err(DeviceLinkError::from)
    }

    pub async fn recv_message(&mut self) -> Result<plist::Value, DeviceLinkError> {
        super::plist_frame::read_plist_frame(&mut self.stream, MAX_PLIST_SIZE)
            .await
            .map_err(DeviceLinkError::from)
    }

    pub async fn disconnect(&mut self) -> Result<(), DeviceLinkError> {
        self.send_message(&vec![
            plist::Value::String("DLMessageDisconnect".into()),
            plist::Value::String("___EmptyParameterString___".into()),
        ])
        .await
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn encode_frame(value: &plist::Value) -> Vec<u8> {
        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, value).expect("plist serialization");
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload);
        frame
    }

    async fn read_frame(stream: &mut tokio::io::DuplexStream) -> plist::Value {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.expect("frame length");
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stream
            .read_exact(&mut payload)
            .await
            .expect("frame payload");
        plist::from_bytes(&payload).expect("plist decode")
    }

    #[tokio::test]
    async fn version_exchange_sends_versions_ok_and_returns_major_version() {
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = DeviceLinkClient::new(client_stream);
            client.version_exchange().await.unwrap()
        });

        server_stream
            .write_all(&encode_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300u64.into()),
            ])))
            .await
            .unwrap();

        let versions_ok = read_frame(&mut server_stream).await;
        assert_eq!(
            versions_ok.as_array(),
            Some(&vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::String("DLVersionsOk".into()),
                plist::Value::Integer(300u64.into()),
            ])
        );

        server_stream
            .write_all(&encode_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .unwrap();

        assert_eq!(task.await.unwrap(), 300);
    }

    #[tokio::test]
    async fn recv_process_message_requires_dictionary_payload() {
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = DeviceLinkClient::new(client_stream);
            client.recv_process_message().await
        });

        server_stream
            .write_all(&encode_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::String("not-a-dict".into()),
            ])))
            .await
            .unwrap();

        let err = task
            .await
            .unwrap()
            .expect_err("non-dictionary payload must fail");
        assert!(err
            .to_string()
            .contains("process message missing dictionary payload"));
    }

    #[tokio::test]
    async fn disconnect_sends_expected_message() {
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = DeviceLinkClient::new(client_stream);
            client.disconnect().await.unwrap();
        });

        let disconnect = read_frame(&mut server_stream).await;
        assert_eq!(
            disconnect.as_array(),
            Some(&vec![
                plist::Value::String("DLMessageDisconnect".into()),
                plist::Value::String("___EmptyParameterString___".into()),
            ])
        );

        task.await.unwrap();
    }
}
