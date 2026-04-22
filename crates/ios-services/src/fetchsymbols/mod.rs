//! Legacy fetch symbols service client.
//!
//! Service: `com.apple.dt.fetchsymbols`
//! Reference: pymobiledevice3 `dtfetchsymbols.py`

use std::io::Write;

use bytes::BytesMut;
use indexmap::IndexMap;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

pub const SERVICE_NAME: &str = "com.apple.dt.fetchsymbols";
pub const REMOTE_SERVICE_NAME: &str = "com.apple.dt.remoteFetchSymbols";
const CMD_LIST_FILES: u32 = 0x3030_3030;
const CMD_GET_FILE: u32 = 1;
const MAX_CHUNK: usize = 10 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FetchSymbolsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct FetchSymbolsClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> FetchSymbolsClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn list_files(&mut self) -> Result<Vec<String>, FetchSymbolsError> {
        self.start_command(CMD_LIST_FILES).await?;
        let response = recv_plist(&mut self.stream).await?;
        response
            .get("files")
            .and_then(plist::Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|value| value.as_string().map(ToOwned::to_owned))
                    .collect()
            })
            .ok_or_else(|| FetchSymbolsError::Protocol("missing files array".into()))
    }

    pub async fn download<W: Write>(
        &mut self,
        index: u32,
        mut writer: W,
        max_bytes: Option<u64>,
    ) -> Result<u64, FetchSymbolsError> {
        self.start_command(CMD_GET_FILE).await?;
        self.stream.write_all(&index.to_be_bytes()).await?;
        self.stream.flush().await?;

        let size = self.stream.read_u64().await?;
        let limit = max_bytes.map_or(size, |limit| limit.min(size));

        let mut remaining = limit;
        let mut written = 0u64;
        let mut buf = vec![0u8; MAX_CHUNK];
        while remaining > 0 {
            let chunk_size = remaining.min(MAX_CHUNK as u64) as usize;
            self.stream.read_exact(&mut buf[..chunk_size]).await?;
            writer.write_all(&buf[..chunk_size])?;
            written += chunk_size as u64;
            remaining -= chunk_size as u64;
        }

        Ok(written)
    }

    async fn start_command(&mut self, command: u32) -> Result<(), FetchSymbolsError> {
        let encoded = command.to_be_bytes();
        self.stream.write_all(&encoded).await?;
        self.stream.flush().await?;

        let mut ack = [0u8; 4];
        self.stream.read_exact(&mut ack).await?;
        if ack != encoded {
            return Err(FetchSymbolsError::Protocol(format!(
                "unexpected fetchsymbols ack: expected 0x{command:08x}, got 0x{:08x}",
                u32::from_be_bytes(ack)
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSymbolFile {
    pub path: String,
    pub size: u64,
}

pub struct RemoteFetchSymbolsClient<S> {
    framer: ios_xpc::h2_raw::H2Framer<S>,
    next_msg_id: u64,
    pending_control_data: BytesMut,
}

impl<S: AsyncRead + AsyncWrite + Unpin> RemoteFetchSymbolsClient<S> {
    pub async fn connect(stream: S) -> Result<Self, FetchSymbolsError> {
        let mut framer = ios_xpc::h2_raw::H2Framer::connect(stream)
            .await
            .map_err(|err| FetchSymbolsError::Protocol(format!("H2 error: {err}")))?;
        bootstrap_remote_xpc(&mut framer).await?;
        Ok(Self {
            framer,
            next_msg_id: 1,
            pending_control_data: BytesMut::new(),
        })
    }

    pub async fn list_files(&mut self) -> Result<Vec<RemoteSymbolFile>, FetchSymbolsError> {
        self.send_catalog_request().await?;

        let count = self.recv_catalog_count().await?;
        let mut files = Vec::with_capacity(count.min(128));
        for _ in 0..count {
            files.push(self.recv_catalog_entry().await?);
        }
        Ok(files)
    }

    pub async fn download<W: Write>(
        &mut self,
        index: u32,
        mut writer: W,
        max_bytes: Option<u64>,
    ) -> Result<u64, FetchSymbolsError> {
        let files = self.list_files().await?;
        let file = files.get(index as usize).ok_or_else(|| {
            FetchSymbolsError::Protocol(format!("symbol index {index} out of range"))
        })?;

        let stream_id = (index + 1) * 2;
        self.framer
            .write_stream(
                stream_id,
                &ios_xpc::message::encode_message(&ios_xpc::XpcMessage {
                    flags: ios_xpc::message::flags::ALWAYS_SET
                        | ios_xpc::message::flags::FILE_TX_STREAM_RESPONSE,
                    msg_id: 0,
                    body: None,
                })
                .map_err(|err| {
                    FetchSymbolsError::Protocol(format!("file stream encode failed: {err}"))
                })?,
            )
            .await
            .map_err(|err| {
                FetchSymbolsError::Protocol(format!("file stream open failed: {err}"))
            })?;

        let limit = max_bytes.map_or(file.size, |limit| limit.min(file.size));
        let mut remaining = limit;
        let mut written = 0u64;
        let mut buf = vec![0u8; MAX_CHUNK.min(limit.max(1) as usize)];

        while remaining > 0 {
            let chunk_len = remaining.min(buf.len() as u64) as usize;
            let chunk = self
                .framer
                .read_stream(stream_id, chunk_len)
                .await
                .map_err(|err| {
                    FetchSymbolsError::Protocol(format!("file stream read failed: {err}"))
                })?;
            buf[..chunk_len].copy_from_slice(&chunk);
            writer.write_all(&buf[..chunk_len])?;
            written += chunk_len as u64;
            remaining -= chunk_len as u64;
        }

        Ok(written)
    }

    async fn send_catalog_request(&mut self) -> Result<(), FetchSymbolsError> {
        let mut request = IndexMap::new();
        request.insert(
            "XPCDictionary_sideChannel".to_string(),
            ios_xpc::XpcValue::Uuid(*Uuid::new_v4().as_bytes()),
        );
        request.insert(
            "DSCFilePaths".to_string(),
            ios_xpc::XpcValue::Array(Vec::new()),
        );

        self.framer
            .write_client_server(
                &ios_xpc::message::encode_message(&ios_xpc::XpcMessage {
                    flags: ios_xpc::message::flags::ALWAYS_SET
                        | ios_xpc::message::flags::DATA_PRESENT
                        | ios_xpc::message::flags::WANTING_REPLY,
                    msg_id: self.next_msg_id,
                    body: Some(ios_xpc::XpcValue::Dictionary(request)),
                })
                .map_err(|err| {
                    FetchSymbolsError::Protocol(format!("catalog request encode failed: {err}"))
                })?,
            )
            .await
            .map_err(|err| FetchSymbolsError::Protocol(format!("catalog request failed: {err}")))?;
        self.next_msg_id += 1;
        Ok(())
    }

    async fn recv_control_message(&mut self) -> Result<ios_xpc::XpcMessage, FetchSymbolsError> {
        loop {
            if let Some(message) = self.try_take_pending_control_message()? {
                if message.flags & ios_xpc::message::flags::FILE_TX_STREAM_REQUEST != 0 {
                    continue;
                }
                return Ok(message);
            }

            let frame = self.framer.read_next_data_frame().await.map_err(|err| {
                FetchSymbolsError::Protocol(format!("control frame read failed: {err}"))
            })?;
            self.pending_control_data.extend_from_slice(&frame.payload);
        }
    }

    async fn recv_catalog_count(&mut self) -> Result<usize, FetchSymbolsError> {
        let mut last_error = None;
        for _ in 0..32 {
            let message = self.recv_control_message().await?;
            match try_parse_catalog_count(&message) {
                Some(Ok(count)) => return Ok(count),
                Some(Err(err)) => last_error = Some(err),
                None => continue,
            }
        }
        Err(last_error.unwrap_or_else(|| {
            FetchSymbolsError::Protocol("did not receive remote symbols catalog count".into())
        }))
    }

    async fn recv_catalog_entry(&mut self) -> Result<RemoteSymbolFile, FetchSymbolsError> {
        let mut last_error = None;
        for _ in 0..64 {
            let message = self.recv_control_message().await?;
            match try_parse_catalog_entry(&message) {
                Some(Ok(entry)) => return Ok(entry),
                Some(Err(err)) => {
                    tracing::trace!(
                        "remote fetchsymbols catalog entry parse failed: err={err}; body={:?}",
                        message.body
                    );
                    last_error = Some(err);
                }
                None => continue,
            }
        }
        Err(last_error.unwrap_or_else(|| {
            FetchSymbolsError::Protocol("did not receive remote symbols catalog entry".into())
        }))
    }
}

impl<S> RemoteFetchSymbolsClient<S> {
    fn try_take_pending_control_message(
        &mut self,
    ) -> Result<Option<ios_xpc::XpcMessage>, FetchSymbolsError> {
        if self.pending_control_data.len() < 24 {
            return Ok(None);
        }

        let body_len =
            u64::from_le_bytes(self.pending_control_data[8..16].try_into().map_err(|_| {
                FetchSymbolsError::Protocol("invalid control message header".into())
            })?) as usize;
        let total_len = 24usize
            .checked_add(body_len)
            .ok_or_else(|| FetchSymbolsError::Protocol("control message length overflow".into()))?;
        if self.pending_control_data.len() < total_len {
            return Ok(None);
        }

        let payload = self.pending_control_data.split_to(total_len).freeze();
        let message = ios_xpc::message::decode_message(payload)
            .map_err(|err| FetchSymbolsError::Protocol(format!("control decode failed: {err}")))?;
        Ok(Some(message))
    }
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, FetchSymbolsError> {
    let len = stream.read_u32().await? as usize;
    const MAX_PLIST_SIZE: usize = 8 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(FetchSymbolsError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    plist::from_bytes(&buf).map_err(|err| FetchSymbolsError::Plist(err.to_string()))
}

fn try_parse_catalog_count(
    message: &ios_xpc::XpcMessage,
) -> Option<Result<usize, FetchSymbolsError>> {
    let dict = message.body.as_ref()?.as_dict()?;
    let value = dict.get("DSCFilePaths")?;
    Some(as_u64(value).map(|count| count as usize).ok_or_else(|| {
        FetchSymbolsError::Protocol("catalog response missing DSCFilePaths count".into())
    }))
}

fn try_parse_catalog_entry(
    message: &ios_xpc::XpcMessage,
) -> Option<Result<RemoteSymbolFile, FetchSymbolsError>> {
    let dict = message.body.as_ref()?.as_dict()?;
    let entry = match dict.get("DSCFilePaths") {
        Some(value) => value.as_dict()?,
        None => return None,
    };
    let path = entry
        .get("filePath")
        .and_then(ios_xpc::XpcValue::as_str)
        .ok_or_else(|| FetchSymbolsError::Protocol("catalog entry missing filePath".into()));
    let transfer = entry
        .get("fileTransfer")
        .ok_or_else(|| FetchSymbolsError::Protocol("catalog entry missing fileTransfer".into()));

    Some((|| {
        let path = path?.to_string();
        let size = parse_transfer_size(transfer?)?;

        Ok(RemoteSymbolFile { path, size })
    })())
}

fn parse_transfer_size(value: &ios_xpc::XpcValue) -> Result<u64, FetchSymbolsError> {
    if let Some((_, transfer)) = value.as_file_transfer() {
        return transfer
            .as_dict()
            .and_then(|dict| dict.get("s"))
            .and_then(as_u64)
            .ok_or_else(|| {
                FetchSymbolsError::Protocol("catalog entry missing fileTransfer size".into())
            });
    }

    let dict = value.as_dict().ok_or_else(|| {
        FetchSymbolsError::Protocol("catalog entry fileTransfer has unsupported shape".into())
    })?;
    if let Some(size) = dict.get("expectedLength").and_then(as_u64) {
        return Ok(size);
    }
    dict.get("xpcFileTransfer")
        .ok_or_else(|| FetchSymbolsError::Protocol("catalog entry missing xpcFileTransfer".into()))
        .and_then(parse_transfer_size)
}

fn as_u64(value: &ios_xpc::XpcValue) -> Option<u64> {
    match value {
        ios_xpc::XpcValue::Uint64(n) => Some(*n),
        ios_xpc::XpcValue::Int64(n) if *n >= 0 => Some(*n as u64),
        _ => None,
    }
}

async fn bootstrap_remote_xpc<S>(
    framer: &mut ios_xpc::h2_raw::H2Framer<S>,
) -> Result<(), FetchSymbolsError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    framer
        .write_client_server(
            &ios_xpc::message::encode_message(&ios_xpc::XpcMessage {
                flags: ios_xpc::message::flags::ALWAYS_SET | ios_xpc::message::flags::DATA_PRESENT,
                msg_id: 0,
                body: Some(ios_xpc::XpcValue::Dictionary(IndexMap::new())),
            })
            .map_err(|err| {
                FetchSymbolsError::Protocol(format!(
                    "remote XPC bootstrap encode step 1 failed: {err}"
                ))
            })?,
        )
        .await
        .map_err(|err| {
            FetchSymbolsError::Protocol(format!("remote XPC bootstrap step 1 failed: {err}"))
        })?;

    framer
        .write_client_server(
            &ios_xpc::message::encode_message(&ios_xpc::XpcMessage {
                flags: ios_xpc::message::flags::ALWAYS_SET | ios_xpc::message::flags::REPLY,
                msg_id: 0,
                body: None,
            })
            .map_err(|err| {
                FetchSymbolsError::Protocol(format!(
                    "remote XPC bootstrap encode step 2 failed: {err}"
                ))
            })?,
        )
        .await
        .map_err(|err| {
            FetchSymbolsError::Protocol(format!("remote XPC bootstrap step 2 failed: {err}"))
        })?;

    framer
        .write_server_client(
            &ios_xpc::message::encode_message(&ios_xpc::XpcMessage {
                flags: ios_xpc::message::flags::ALWAYS_SET
                    | ios_xpc::message::flags::INIT_HANDSHAKE,
                msg_id: 0,
                body: None,
            })
            .map_err(|err| {
                FetchSymbolsError::Protocol(format!(
                    "remote XPC bootstrap encode step 3 failed: {err}"
                ))
            })?,
        )
        .await
        .map_err(|err| {
            FetchSymbolsError::Protocol(format!("remote XPC bootstrap step 3 failed: {err}"))
        })?;

    Ok(())
}
