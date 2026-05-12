//! iOS 17+ file service via XPC/RSD.

use bytes::{Bytes, BytesMut};
use indexmap::IndexMap;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};

pub const CONTROL_SERVICE_NAME: &str = "com.apple.coredevice.fileservice.control";
pub const DATA_SERVICE_NAME: &str = "com.apple.coredevice.fileservice.data";
pub const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024;
pub const MAX_INLINE_DATA_SIZE: u64 = 500;

const FILE_WIRE_MAGIC: &[u8; 8] = b"rwb!FILE";

#[derive(Debug, thiserror::Error)]
pub enum FileServiceError {
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum Domain {
    AppDataContainer = 1,
    AppGroupDataContainer = 2,
    Temporary = 3,
    RootStaging = 4,
    SystemCrashLogs = 5,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTransferTicket {
    pub response_token: u64,
    pub file_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileWriteOptions {
    pub permissions: i64,
    pub uid: i64,
    pub gid: i64,
    pub creation_time: i64,
    pub last_modification_time: i64,
}

impl FileWriteOptions {
    pub fn mobile_defaults_now() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        Self {
            permissions: 0o644,
            uid: 501,
            gid: 501,
            creation_time: now,
            last_modification_time: now,
        }
    }
}

impl Default for FileWriteOptions {
    fn default() -> Self {
        Self::mobile_defaults_now()
    }
}

pub struct FileServiceClient {
    control: XpcClient,
    session_id: String,
}

impl FileServiceClient {
    pub async fn connect(
        mut control: XpcClient,
        domain: Domain,
        identifier: impl AsRef<str>,
    ) -> Result<Self, FileServiceError> {
        let response = control
            .call(build_create_session_request(domain, identifier.as_ref()))
            .await?;
        let session_id = parse_create_session_response(response)?;
        Ok(Self {
            control,
            session_id,
        })
    }

    pub fn with_session(control: XpcClient, session_id: impl Into<String>) -> Self {
        Self {
            control,
            session_id: session_id.into(),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub async fn list_directory(&mut self, path: &str) -> Result<Vec<String>, FileServiceError> {
        let response = self
            .control
            .call_recv_client_server(build_retrieve_directory_list_request(
                &self.session_id,
                path,
            ))
            .await?;
        parse_directory_list_response(response)
    }

    pub async fn retrieve_file_ticket(
        &mut self,
        path: &str,
    ) -> Result<FileTransferTicket, FileServiceError> {
        let response = self
            .control
            .call(build_retrieve_file_request(&self.session_id, path))
            .await?;
        parse_retrieve_file_response(response)
    }

    pub async fn download_file<S>(
        &mut self,
        path: &str,
        data_stream: &mut S,
    ) -> Result<Bytes, FileServiceError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let ticket = self.retrieve_file_ticket(path).await?;
        send_download_wire_request(data_stream, &ticket).await?;
        receive_file_data(data_stream).await
    }

    pub async fn download_file_to_writer<S, W>(
        &mut self,
        path: &str,
        data_stream: &mut S,
        writer: &mut W,
    ) -> Result<u64, FileServiceError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
        W: AsyncWrite + Unpin,
    {
        let ticket = self.retrieve_file_ticket(path).await?;
        send_download_wire_request(data_stream, &ticket).await?;
        receive_file_data_to_writer(data_stream, writer).await
    }

    pub async fn propose_empty_file(
        &mut self,
        path: &str,
        options: FileWriteOptions,
    ) -> Result<(), FileServiceError> {
        let response = self
            .control
            .call(build_propose_empty_file_request(
                &self.session_id,
                path,
                options,
            ))
            .await?;
        let body = response_body(response)?;
        ensure_no_error(&body)
    }

    pub async fn upload_inline_file(
        &mut self,
        path: &str,
        data: Bytes,
        options: FileWriteOptions,
    ) -> Result<(), FileServiceError> {
        if data.is_empty() {
            return self.propose_empty_file(path, options).await;
        }
        if data.len() as u64 > MAX_INLINE_DATA_SIZE {
            return Err(FileServiceError::Protocol(format!(
                "inline file size {} exceeds maximum inline size {MAX_INLINE_DATA_SIZE}",
                data.len()
            )));
        }

        let response = self
            .control
            .call(build_propose_file_request(
                &self.session_id,
                path,
                data.len() as u64,
                Some(data),
                options,
            ))
            .await?;
        let _ = parse_propose_file_response(response)?;
        Ok(())
    }

    pub async fn propose_file_upload(
        &mut self,
        path: &str,
        file_size: u64,
        options: FileWriteOptions,
    ) -> Result<FileTransferTicket, FileServiceError> {
        if file_size > MAX_FILE_SIZE {
            return Err(FileServiceError::Protocol(format!(
                "file size {file_size} exceeds maximum allowed size {MAX_FILE_SIZE}"
            )));
        }
        if file_size <= MAX_INLINE_DATA_SIZE {
            return Err(FileServiceError::Protocol(format!(
                "file size {file_size} fits inline; use upload_inline_file"
            )));
        }

        let response = self
            .control
            .call(build_propose_file_request(
                &self.session_id,
                path,
                file_size,
                None,
                options,
            ))
            .await?;
        parse_propose_file_response(response)?.ok_or_else(|| {
            FileServiceError::Protocol("ProposeFile response missing upload ticket".into())
        })
    }

    pub async fn upload_file_data<S, R>(
        &mut self,
        data_stream: &mut S,
        ticket: &FileTransferTicket,
        reader: &mut R,
        file_size: u64,
    ) -> Result<(), FileServiceError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
        R: AsyncRead + Unpin,
    {
        upload_file_data(data_stream, ticket, reader, file_size).await
    }
}

fn build_create_session_request(domain: Domain, identifier: &str) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        ("Cmd".to_string(), XpcValue::String("CreateSession".into())),
        ("Domain".to_string(), XpcValue::Uint64(domain as u64)),
        (
            "Identifier".to_string(),
            XpcValue::String(identifier.to_string()),
        ),
        ("Session".to_string(), XpcValue::String(String::new())),
        ("User".to_string(), XpcValue::String("mobile".into())),
    ]))
}

fn build_retrieve_directory_list_request(session_id: &str, path: &str) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "Cmd".to_string(),
            XpcValue::String("RetrieveDirectoryList".into()),
        ),
        (
            "MessageUUID".to_string(),
            XpcValue::String(uuid::Uuid::new_v4().to_string()),
        ),
        ("Path".to_string(), XpcValue::String(path.to_string())),
        (
            "SessionID".to_string(),
            XpcValue::String(session_id.to_string()),
        ),
    ]))
}

fn build_retrieve_file_request(session_id: &str, path: &str) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        ("Cmd".to_string(), XpcValue::String("RetrieveFile".into())),
        ("Path".to_string(), XpcValue::String(path.to_string())),
        (
            "SessionID".to_string(),
            XpcValue::String(session_id.to_string()),
        ),
    ]))
}

fn build_propose_empty_file_request(
    session_id: &str,
    path: &str,
    options: FileWriteOptions,
) -> XpcValue {
    XpcValue::Dictionary(file_write_metadata(
        "ProposeEmptyFile",
        session_id,
        path,
        options,
    ))
}

fn build_propose_file_request(
    session_id: &str,
    path: &str,
    file_size: u64,
    file_data: Option<Bytes>,
    options: FileWriteOptions,
) -> XpcValue {
    let mut dict = file_write_metadata("ProposeFile", session_id, path, options);
    dict.insert("FileSize".to_string(), XpcValue::Uint64(file_size));
    if let Some(file_data) = file_data {
        dict.insert("FileData".to_string(), XpcValue::Data(file_data));
    }
    XpcValue::Dictionary(dict)
}

fn file_write_metadata(
    command: &str,
    session_id: &str,
    path: &str,
    options: FileWriteOptions,
) -> IndexMap<String, XpcValue> {
    IndexMap::from([
        ("Cmd".to_string(), XpcValue::String(command.to_string())),
        (
            "FileCreationTime".to_string(),
            XpcValue::Int64(options.creation_time),
        ),
        (
            "FileLastModificationTime".to_string(),
            XpcValue::Int64(options.last_modification_time),
        ),
        (
            "FilePermissions".to_string(),
            XpcValue::Int64(options.permissions),
        ),
        ("FileOwnerUserID".to_string(), XpcValue::Int64(options.uid)),
        ("FileOwnerGroupID".to_string(), XpcValue::Int64(options.gid)),
        ("Path".to_string(), XpcValue::String(path.to_string())),
        (
            "SessionID".to_string(),
            XpcValue::String(session_id.to_string()),
        ),
    ])
}

fn parse_create_session_response(response: XpcMessage) -> Result<String, FileServiceError> {
    let body = response_body(response)?;
    ensure_no_error(&body)?;
    let dict = body_dict(&body)?;
    dict.get("NewSessionID")
        .and_then(XpcValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            FileServiceError::Protocol(format!(
                "CreateSession response missing NewSessionID: {body:?}"
            ))
        })
}

fn parse_directory_list_response(response: XpcMessage) -> Result<Vec<String>, FileServiceError> {
    let body = response_body(response)?;
    ensure_no_error(&body)?;
    let dict = body_dict(&body)?;
    let file_list = dict.get("FileList").ok_or_else(|| {
        FileServiceError::Protocol(format!(
            "RetrieveDirectoryList response missing FileList: {body:?}"
        ))
    })?;
    let XpcValue::Array(items) = file_list else {
        return Err(FileServiceError::Protocol(format!(
            "FileList is not an array: {file_list:?}"
        )));
    };
    Ok(items
        .iter()
        .filter_map(|item| item.as_str().map(ToOwned::to_owned))
        .collect())
}

fn parse_retrieve_file_response(
    response: XpcMessage,
) -> Result<FileTransferTicket, FileServiceError> {
    let body = response_body(response)?;
    ensure_no_error(&body)?;
    let dict = body_dict(&body)?;
    Ok(FileTransferTicket {
        response_token: dict.get("Response").and_then(as_u64).ok_or_else(|| {
            FileServiceError::Protocol(format!(
                "RetrieveFile response missing Response token: {body:?}"
            ))
        })?,
        file_id: dict.get("NewFileID").and_then(as_u64).ok_or_else(|| {
            FileServiceError::Protocol(format!("RetrieveFile response missing NewFileID: {body:?}"))
        })?,
    })
}

fn parse_propose_file_response(
    response: XpcMessage,
) -> Result<Option<FileTransferTicket>, FileServiceError> {
    let body = response_body(response)?;
    ensure_no_error(&body)?;
    let dict = body_dict(&body)?;
    let response_token = dict.get("Response").and_then(as_u64);
    let file_id = dict.get("NewFileID").and_then(as_u64);

    match (response_token, file_id) {
        (Some(response_token), Some(file_id)) => Ok(Some(FileTransferTicket {
            response_token,
            file_id,
        })),
        (None, None) => Ok(None),
        _ => Err(FileServiceError::Protocol(format!(
            "ProposeFile response has incomplete upload ticket: {body:?}"
        ))),
    }
}

async fn send_download_wire_request<S>(
    stream: &mut S,
    ticket: &FileTransferTicket,
) -> Result<(), FileServiceError>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&build_download_wire_request(ticket.clone()))
        .await?;
    stream.flush().await?;
    Ok(())
}

fn build_download_wire_request(ticket: FileTransferTicket) -> [u8; 40] {
    let mut request = [0u8; 40];
    request[0..8].copy_from_slice(FILE_WIRE_MAGIC);
    request[8..16].copy_from_slice(&ticket.response_token.to_be_bytes());
    request[24..32].copy_from_slice(&ticket.file_id.to_be_bytes());
    request
}

fn build_upload_wire_header(ticket: &FileTransferTicket, file_size: u64) -> [u8; 40] {
    let mut request = [0u8; 40];
    request[0..8].copy_from_slice(FILE_WIRE_MAGIC);
    request[24..32].copy_from_slice(&ticket.file_id.to_be_bytes());
    request[32..40].copy_from_slice(&file_size.to_be_bytes());
    request
}

async fn upload_file_data<S, R>(
    stream: &mut S,
    ticket: &FileTransferTicket,
    reader: &mut R,
    file_size: u64,
) -> Result<(), FileServiceError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    stream
        .write_all(&build_upload_wire_header(ticket, file_size))
        .await?;

    let mut remaining = file_size;
    let mut buffer = [0u8; 256 * 1024];
    while remaining > 0 {
        let to_read = remaining.min(buffer.len() as u64) as usize;
        let n = reader.read(&mut buffer[..to_read]).await?;
        if n == 0 {
            return Err(FileServiceError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "file upload source ended before declared size",
            )));
        }
        stream.write_all(&buffer[..n]).await?;
        remaining -= n as u64;
    }
    stream.flush().await?;

    let mut confirmation = [0u8; 32];
    stream.read_exact(&mut confirmation).await?;
    if &confirmation[0..8] != FILE_WIRE_MAGIC {
        return Err(FileServiceError::Protocol(format!(
            "invalid upload confirmation magic: {:?}",
            &confirmation[0..8]
        )));
    }
    Ok(())
}

async fn receive_file_data<S>(stream: &mut S) -> Result<Bytes, FileServiceError>
where
    S: AsyncRead + Unpin,
{
    let file_size = read_file_data_header(stream).await?;
    if file_size > MAX_FILE_SIZE {
        return Err(FileServiceError::Protocol(format!(
            "file size {file_size} exceeds maximum allowed size {MAX_FILE_SIZE}"
        )));
    }

    let mut data = BytesMut::with_capacity(file_size as usize);
    data.resize(file_size as usize, 0);
    stream.read_exact(&mut data).await?;
    Ok(data.freeze())
}

async fn receive_file_data_to_writer<S, W>(
    stream: &mut S,
    writer: &mut W,
) -> Result<u64, FileServiceError>
where
    S: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let file_size = read_file_data_header(stream).await?;
    if file_size > MAX_FILE_SIZE {
        return Err(FileServiceError::Protocol(format!(
            "file size {file_size} exceeds maximum allowed size {MAX_FILE_SIZE}"
        )));
    }

    let mut remaining = file_size;
    let mut buffer = [0u8; 256 * 1024];
    while remaining > 0 {
        let to_read = remaining.min(buffer.len() as u64) as usize;
        stream.read_exact(&mut buffer[..to_read]).await?;
        writer.write_all(&buffer[..to_read]).await?;
        remaining -= to_read as u64;
    }
    writer.flush().await?;
    Ok(file_size)
}

async fn read_file_data_header<S>(stream: &mut S) -> Result<u64, FileServiceError>
where
    S: AsyncRead + Unpin,
{
    let mut header = [0u8; 40];
    stream.read_exact(&mut header).await?;
    if &header[0..8] != FILE_WIRE_MAGIC {
        return Err(FileServiceError::Protocol(format!(
            "invalid file data magic: {:?}",
            &header[0..8]
        )));
    }
    Ok(u32::from_be_bytes(
        header[36..40]
            .try_into()
            .map_err(|_| FileServiceError::Protocol("invalid file data size header".into()))?,
    ) as u64)
}

fn response_body(response: XpcMessage) -> Result<XpcValue, FileServiceError> {
    response
        .body
        .ok_or_else(|| FileServiceError::Protocol("missing response body".into()))
}

fn body_dict(value: &XpcValue) -> Result<&IndexMap<String, XpcValue>, FileServiceError> {
    value.as_dict().ok_or_else(|| {
        FileServiceError::Protocol(format!("response body is not a dict: {value:?}"))
    })
}

fn ensure_no_error(value: &XpcValue) -> Result<(), FileServiceError> {
    if let Some(message) = error_message(value) {
        return Err(FileServiceError::Protocol(message));
    }
    Ok(())
}

fn error_message(value: &XpcValue) -> Option<String> {
    let dict = value.as_dict()?;
    let encoded_error = dict.get("EncodedError")?;
    if matches!(encoded_error, XpcValue::Null) {
        return None;
    }
    if let Some(message) = nested_error_message(encoded_error) {
        return Some(message);
    }
    dict.get("LocalizedDescription")
        .and_then(XpcValue::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| Some(format!("{encoded_error:?}")))
}

fn nested_error_message(value: &XpcValue) -> Option<String> {
    match value {
        XpcValue::String(message) => Some(message.clone()),
        XpcValue::Dictionary(dict) => {
            for key in [
                "LocalizedDescription",
                "localizedDescription",
                "NSLocalizedDescription",
                "message",
                "description",
            ] {
                if let Some(XpcValue::String(message)) = dict.get(key) {
                    return Some(message.clone());
                }
            }
            None
        }
        _ => None,
    }
}

fn as_u64(value: &XpcValue) -> Option<u64> {
    match value {
        XpcValue::Uint64(value) => Some(*value),
        XpcValue::Int64(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use indexmap::IndexMap;
    use tokio::io::AsyncWriteExt;

    use super::*;
    use crate::xpc::{XpcMessage, XpcValue};

    #[test]
    fn create_session_request_matches_coredevice_fileservice_shape() {
        let request = build_create_session_request(Domain::AppDataContainer, "com.example.App");
        let dict = request.as_dict().expect("request should be a dictionary");

        assert_eq!(dict["Cmd"].as_str(), Some("CreateSession"));
        assert_eq!(dict["Domain"], XpcValue::Uint64(1));
        assert_eq!(dict["Identifier"].as_str(), Some("com.example.App"));
        assert_eq!(dict["Session"].as_str(), Some(""));
        assert_eq!(dict["User"].as_str(), Some("mobile"));
    }

    #[test]
    fn session_response_extracts_new_session_id() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "NewSessionID".to_string(),
                XpcValue::String("SESSION-1".into()),
            )]))),
        };

        assert_eq!(
            parse_create_session_response(response).unwrap(),
            "SESSION-1"
        );
    }

    #[test]
    fn encoded_error_uses_nested_localized_description() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "EncodedError".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "LocalizedDescription".to_string(),
                    XpcValue::String("No such file".into()),
                )])),
            )]))),
        };

        let err = parse_create_session_response(response).unwrap_err();
        assert!(err.to_string().contains("No such file"));
    }

    #[test]
    fn directory_list_response_keeps_string_entries() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 2,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "FileList".to_string(),
                XpcValue::Array(vec![
                    XpcValue::String("Documents".into()),
                    XpcValue::Uint64(7),
                    XpcValue::String("Library".into()),
                ]),
            )]))),
        };

        assert_eq!(
            parse_directory_list_response(response).unwrap(),
            vec!["Documents".to_string(), "Library".to_string()]
        );
    }

    #[test]
    fn retrieve_file_response_extracts_tokens() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 3,
            body: Some(XpcValue::Dictionary(IndexMap::from([
                ("Response".to_string(), XpcValue::Uint64(0x11)),
                ("NewFileID".to_string(), XpcValue::Uint64(0x22)),
            ]))),
        };

        assert_eq!(
            parse_retrieve_file_response(response).unwrap(),
            FileTransferTicket {
                response_token: 0x11,
                file_id: 0x22,
            }
        );
    }

    #[test]
    fn propose_empty_file_request_includes_metadata() {
        let options = FileWriteOptions {
            permissions: 0o644,
            uid: 501,
            gid: 501,
            creation_time: 100,
            last_modification_time: 200,
        };
        let request = build_propose_empty_file_request("SESSION-1", "empty.txt", options);
        let dict = request.as_dict().expect("request should be a dictionary");

        assert_eq!(dict["Cmd"].as_str(), Some("ProposeEmptyFile"));
        assert_eq!(dict["Path"].as_str(), Some("empty.txt"));
        assert_eq!(dict["SessionID"].as_str(), Some("SESSION-1"));
        assert_eq!(dict["FilePermissions"], XpcValue::Int64(0o644));
        assert_eq!(dict["FileOwnerUserID"], XpcValue::Int64(501));
        assert_eq!(dict["FileOwnerGroupID"], XpcValue::Int64(501));
        assert_eq!(dict["FileCreationTime"], XpcValue::Int64(100));
        assert_eq!(dict["FileLastModificationTime"], XpcValue::Int64(200));
    }

    #[test]
    fn propose_file_request_inlines_small_file_data() {
        let options = FileWriteOptions {
            permissions: 0o600,
            uid: 501,
            gid: 501,
            creation_time: 1,
            last_modification_time: 2,
        };
        let request = build_propose_file_request(
            "SESSION-1",
            "notes.txt",
            5,
            Some(Bytes::from_static(b"hello")),
            options,
        );
        let dict = request.as_dict().expect("request should be a dictionary");

        assert_eq!(dict["Cmd"].as_str(), Some("ProposeFile"));
        assert_eq!(dict["FileSize"], XpcValue::Uint64(5));
        assert_eq!(dict["FilePermissions"], XpcValue::Int64(0o600));
        assert_eq!(
            dict["FileData"],
            XpcValue::Data(Bytes::from_static(b"hello"))
        );
    }

    #[test]
    fn propose_file_response_extracts_large_upload_ticket() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 4,
            body: Some(XpcValue::Dictionary(IndexMap::from([
                ("Response".to_string(), XpcValue::Uint64(0x33)),
                ("NewFileID".to_string(), XpcValue::Uint64(0x44)),
            ]))),
        };

        assert_eq!(
            parse_propose_file_response(response).unwrap(),
            Some(FileTransferTicket {
                response_token: 0x33,
                file_id: 0x44,
            })
        );
    }

    #[test]
    fn download_wire_request_uses_rwb_file_big_endian_header() {
        let header = build_download_wire_request(FileTransferTicket {
            response_token: 0x0102_0304_0506_0708,
            file_id: 0x1112_1314_1516_1718,
        });

        assert_eq!(&header[0..8], b"rwb!FILE");
        assert_eq!(&header[8..16], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&header[16..24], &[0; 8]);
        assert_eq!(
            &header[24..32],
            &[0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18]
        );
        assert_eq!(&header[32..40], &[0; 8]);
    }

    #[test]
    fn upload_wire_header_uses_zero_token_file_id_and_size() {
        let header = build_upload_wire_header(
            &FileTransferTicket {
                response_token: 0x99,
                file_id: 0x1112_1314_1516_1718,
            },
            0x0102_0304_0506_0708,
        );

        assert_eq!(&header[0..8], b"rwb!FILE");
        assert_eq!(&header[8..16], &[0; 8]);
        assert_eq!(
            &header[24..32],
            &[0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18]
        );
        assert_eq!(&header[32..40], &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[tokio::test]
    async fn receive_file_data_reads_size_from_offset_36() {
        let (mut client, mut server) = tokio::io::duplex(128);
        let writer = tokio::spawn(async move {
            let mut header = [0u8; 40];
            header[0..8].copy_from_slice(b"rwb!FILE");
            header[36..40].copy_from_slice(&(5u32.to_be_bytes()));
            server.write_all(&header).await.unwrap();
            server.write_all(b"hello").await.unwrap();
        });

        let data = receive_file_data(&mut client).await.unwrap();

        assert_eq!(data, Bytes::from_static(b"hello"));
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn upload_file_data_streams_header_payload_and_checks_confirmation() {
        let (mut data_client, mut data_server) = tokio::io::duplex(256);
        let (mut reader_client, mut reader_server) = tokio::io::duplex(16);
        let server = tokio::spawn(async move {
            reader_server.write_all(b"hello").await.unwrap();

            let mut header_and_payload = [0u8; 45];
            data_server
                .read_exact(&mut header_and_payload)
                .await
                .unwrap();
            assert_eq!(&header_and_payload[0..8], b"rwb!FILE");
            assert_eq!(&header_and_payload[8..16], &[0; 8]);
            assert_eq!(&header_and_payload[32..40], &(5u64.to_be_bytes()));
            assert_eq!(&header_and_payload[40..45], b"hello");

            let mut confirmation = [0u8; 32];
            confirmation[0..8].copy_from_slice(b"rwb!FILE");
            data_server.write_all(&confirmation).await.unwrap();
        });

        upload_file_data(
            &mut data_client,
            &FileTransferTicket {
                response_token: 7,
                file_id: 9,
            },
            &mut reader_client,
            5,
        )
        .await
        .unwrap();

        server.await.unwrap();
    }
}
