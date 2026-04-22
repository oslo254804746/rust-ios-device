//! AMFI (Apple Mobile File Integrity) – developer mode control.
//!
//! Service: `com.apple.amfi.lockdown`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.amfi.lockdown";

#[derive(Debug, thiserror::Error)]
pub enum AmfiError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("error: {0}")]
    Device(String),
}

/// Enable developer mode on the device.
///
/// After calling this, the device needs to be rebooted.
pub async fn enable_developer_mode<S>(stream: &mut S) -> Result<(), AmfiError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let req = plist::Value::Dictionary({
        let mut d = plist::Dictionary::new();
        d.insert("action".to_string(), plist::Value::Integer(1.into()));
        d
    });

    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &req).map_err(|e| AmfiError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(AmfiError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("plist length {len} exceeds maximum of {MAX_PLIST_SIZE}"),
        )));
    }
    let mut resp_buf = vec![0u8; len];
    stream.read_exact(&mut resp_buf).await?;

    let val: plist::Value =
        plist::from_bytes(&resp_buf).map_err(|e| AmfiError::Plist(e.to_string()))?;

    if let Some(dict) = val.as_dictionary() {
        if let Some(err) = dict.get("Error").and_then(|v| v.as_string()) {
            return Err(AmfiError::Device(err.to_string()));
        }
    }
    Ok(())
}
