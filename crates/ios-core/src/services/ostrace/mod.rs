//! OS trace relay helpers.
//!
//! Service: `com.apple.os_trace_relay`
//! Reference: pymobiledevice3 `os_trace.py`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.os_trace_relay";
pub const SHIM_SERVICE_NAME: &str = "com.apple.os_trace_relay.shim.remote";

service_error!(OsTraceError);

pub struct OsTraceClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> OsTraceClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn get_pid_list(&mut self) -> Result<plist::Dictionary, OsTraceError> {
        let request = plist::Dictionary::from_iter([(
            "Request".to_string(),
            plist::Value::String("PidList".into()),
        )]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;

        let _marker = self.stream.read_u8().await?;
        recv_prefixed_plist(&mut self.stream).await
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), OsTraceError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value).map_err(|err| OsTraceError::Plist(err.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_prefixed_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, OsTraceError> {
    let len = stream.read_u32().await? as usize;
    const MAX_PLIST_SIZE: usize = 8 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(OsTraceError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    plist::from_bytes(&buf).map_err(|err| OsTraceError::Plist(err.to_string()))
}
