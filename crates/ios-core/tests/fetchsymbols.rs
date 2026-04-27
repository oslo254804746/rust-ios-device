use std::pin::Pin;
use std::task::{Context, Poll};

use indexmap::IndexMap;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct MockStream {
    read_buf: Vec<u8>,
    written: Vec<u8>,
    read_pos: usize,
}

impl MockStream {
    fn new(read_buf: Vec<u8>) -> Self {
        Self {
            read_buf,
            written: Vec::new(),
            read_pos: 0,
        }
    }
}

impl AsyncRead for MockStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let remaining = self.read_buf.len().saturating_sub(self.read_pos);
        if remaining == 0 {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "no more test data",
            )));
        }
        let to_copy = remaining.min(buf.remaining());
        let start = self.read_pos;
        let end = start + to_copy;
        buf.put_slice(&self.read_buf[start..end]);
        self.read_pos = end;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for MockStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.written.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn list_files_sends_list_command_and_decodes_response() {
    let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
        "files".to_string(),
        plist::Value::Array(vec![
            plist::Value::String("/dyld_shared_cache_arm64e".into()),
            plist::Value::String("/Symbols/foo".into()),
        ]),
    )]));
    let mut payload = Vec::new();
    payload.extend_from_slice(&0x3030_3030u32.to_be_bytes());
    let mut plist_payload = Vec::new();
    plist::to_writer_xml(&mut plist_payload, &response).unwrap();
    payload.extend_from_slice(&(plist_payload.len() as u32).to_be_bytes());
    payload.extend_from_slice(&plist_payload);

    let mut stream = MockStream::new(payload);
    let mut client = ios_core::fetchsymbols::FetchSymbolsClient::new(&mut stream);

    let files = client.list_files().await.unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(stream.written, 0x3030_3030u32.to_be_bytes());
}

#[tokio::test]
async fn download_sends_index_and_streams_file_bytes() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&1u32.to_be_bytes());
    payload.extend_from_slice(&11u64.to_be_bytes());
    payload.extend_from_slice(b"hello world");

    let mut stream = MockStream::new(payload);
    let mut client = ios_core::fetchsymbols::FetchSymbolsClient::new(&mut stream);
    let mut out = Vec::new();

    let bytes = client.download(7, &mut out, None).await.unwrap();
    assert_eq!(bytes, 11);
    assert_eq!(out, b"hello world");
    assert_eq!(&stream.written[..4], &1u32.to_be_bytes());
    assert_eq!(&stream.written[4..8], &7u32.to_be_bytes());
}

#[tokio::test]
async fn remote_list_files_bootstraps_xpc_and_decodes_catalog() {
    let (client, mut server) = tokio::io::duplex(16 * 1024);

    let server_task = tokio::spawn(async move {
        perform_h2_handshake(&mut server).await;
        perform_remote_xpc_bootstrap(&mut server).await;

        let request = read_xpc_request(&mut server, 1).await;
        let request_body = request.body.expect("request body");
        let request_dict = request_body.as_dict().expect("request dict");
        assert!(matches!(
            request_dict.get("DSCFilePaths"),
            Some(ios_core::xpc::XpcValue::Array(paths)) if paths.is_empty()
        ));
        assert!(matches!(
            request_dict.get("XPCDictionary_sideChannel"),
            Some(ios_core::xpc::XpcValue::Uuid(_))
        ));

        write_xpc_response(
            &mut server,
            3,
            ios_core::xpc::XpcValue::Dictionary(IndexMap::from([(
                "DSCFilePaths".to_string(),
                ios_core::xpc::XpcValue::Uint64(2),
            )])),
        )
        .await;
        write_headers_only(&mut server, 2).await;
        write_file_transfer_request(&mut server, 2).await;
        write_xpc_response_fragmented(
            &mut server,
            1,
            remote_catalog_entry_nested("/System/Library/dyld/dyld_shared_cache_arm64e", 11, 41),
            19,
        )
        .await;
        write_xpc_response(
            &mut server,
            1,
            remote_catalog_entry("/System/Library/Caches/com.apple.dyld/foo.symbols", 7),
        )
        .await;
        read_window_update_pair(&mut server, 2).await;
    });

    let mut client = ios_core::fetchsymbols::RemoteFetchSymbolsClient::connect(client)
        .await
        .expect("remote fetch symbols client should connect");
    let files = client.list_files().await.expect("list should succeed");

    assert_eq!(files.len(), 2);
    assert_eq!(
        files[0].path,
        "/System/Library/dyld/dyld_shared_cache_arm64e"
    );
    assert_eq!(files[0].size, 11);
    assert_eq!(
        files[1].path,
        "/System/Library/Caches/com.apple.dyld/foo.symbols"
    );
    assert_eq!(files[1].size, 7);

    server_task.await.unwrap();
}

#[tokio::test]
async fn remote_download_opens_file_stream_and_copies_bytes() {
    let (client, mut server) = tokio::io::duplex(16 * 1024);

    let server_task = tokio::spawn(async move {
        perform_h2_handshake(&mut server).await;
        perform_remote_xpc_bootstrap(&mut server).await;

        let _ = read_xpc_request(&mut server, 1).await;
        write_xpc_response(
            &mut server,
            3,
            ios_core::xpc::XpcValue::Dictionary(IndexMap::from([(
                "DSCFilePaths".to_string(),
                ios_core::xpc::XpcValue::Uint64(2),
            )])),
        )
        .await;
        write_xpc_response(&mut server, 3, remote_catalog_entry_nested("/first", 5, 17)).await;
        write_headers_only(&mut server, 4).await;
        write_file_transfer_request(&mut server, 4).await;
        write_xpc_response(&mut server, 3, remote_catalog_entry("/second", 11)).await;

        read_window_update_pair(&mut server, 4).await;
        read_headers_frame(&mut server, 4).await;
        let open = read_xpc_request(&mut server, 4).await;
        assert_eq!(
            open.flags,
            ios_core::xpc::message::flags::ALWAYS_SET
                | ios_core::xpc::message::flags::FILE_TX_STREAM_RESPONSE
        );
        assert!(open.body.is_none());

        write_data_frame(&mut server, 4, b"hello world").await;
        let conn_window = read_raw_frame(&mut server).await;
        assert_eq!(conn_window.frame_type, 0x08);
        assert_eq!(conn_window.stream_id, 0);
        let stream_window = read_raw_frame(&mut server).await;
        assert_eq!(stream_window.frame_type, 0x08);
        assert_eq!(stream_window.stream_id, 4);
    });

    let mut client = ios_core::fetchsymbols::RemoteFetchSymbolsClient::connect(client)
        .await
        .expect("remote fetch symbols client should connect");
    let mut out = Vec::new();
    let bytes = client
        .download(1, &mut out, None)
        .await
        .expect("download should succeed");

    assert_eq!(bytes, 11);
    assert_eq!(out, b"hello world");

    server_task.await.unwrap();
}

async fn perform_h2_handshake<S>(stream: &mut S)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut preface = [0u8; 24];
    tokio::io::AsyncReadExt::read_exact(stream, &mut preface)
        .await
        .unwrap();
    assert_eq!(&preface, ios_core::xpc::h2_raw::H2_PREFACE);

    let settings = read_raw_frame(stream).await;
    assert_eq!(settings.frame_type, 0x04);

    let window_update = read_raw_frame(stream).await;
    assert_eq!(window_update.frame_type, 0x08);

    write_raw_frame(stream, 0x04, 0, 0, &[]).await;

    let settings_ack = read_raw_frame(stream).await;
    assert_eq!(settings_ack.frame_type, 0x04);
    assert_eq!(settings_ack.flags, 0x01);
}

async fn perform_remote_xpc_bootstrap<S>(stream: &mut S)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    read_headers_frame(stream, 1).await;
    let _ = read_xpc_request(stream, 1).await;
    write_empty_xpc(stream, 1).await;

    let _ = read_xpc_request(stream, 1).await;
    write_empty_xpc(stream, 1).await;

    read_headers_frame(stream, 3).await;
    let _ = read_xpc_request(stream, 3).await;
    write_empty_xpc(stream, 3).await;
}

fn remote_catalog_entry(path: &str, size: u64) -> ios_core::xpc::XpcValue {
    ios_core::xpc::XpcValue::Dictionary(IndexMap::from([(
        "DSCFilePaths".to_string(),
        ios_core::xpc::XpcValue::Dictionary(IndexMap::from([
            (
                "filePath".to_string(),
                ios_core::xpc::XpcValue::String(path.to_string()),
            ),
            (
                "fileTransfer".to_string(),
                ios_core::xpc::XpcValue::FileTransfer {
                    msg_id: 0,
                    data: Box::new(ios_core::xpc::XpcValue::Dictionary(IndexMap::from([(
                        "s".to_string(),
                        ios_core::xpc::XpcValue::Uint64(size),
                    )]))),
                },
            ),
        ])),
    )]))
}

fn remote_catalog_entry_nested(path: &str, size: u64, msg_id: u64) -> ios_core::xpc::XpcValue {
    ios_core::xpc::XpcValue::Dictionary(IndexMap::from([(
        "DSCFilePaths".to_string(),
        ios_core::xpc::XpcValue::Dictionary(IndexMap::from([
            (
                "filePath".to_string(),
                ios_core::xpc::XpcValue::String(path.to_string()),
            ),
            (
                "fileTransfer".to_string(),
                ios_core::xpc::XpcValue::Dictionary(IndexMap::from([
                    (
                        "expectedLength".to_string(),
                        ios_core::xpc::XpcValue::Uint64(size),
                    ),
                    (
                        "xpcFileTransfer".to_string(),
                        ios_core::xpc::XpcValue::FileTransfer {
                            msg_id,
                            data: Box::new(ios_core::xpc::XpcValue::Dictionary(IndexMap::from([
                                ("s".to_string(), ios_core::xpc::XpcValue::Uint64(size)),
                            ]))),
                        },
                    ),
                ])),
            ),
        ])),
    )]))
}

async fn read_headers_frame<S>(stream: &mut S, stream_id: u32)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let frame = read_raw_frame(stream).await;
    assert_eq!(frame.frame_type, 0x01);
    assert_eq!(frame.flags, 0x04);
    assert_eq!(frame.stream_id, stream_id);
}

async fn read_xpc_request<S>(stream: &mut S, stream_id: u32) -> ios_core::xpc::XpcMessage
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let frame = read_raw_frame(stream).await;
    assert_eq!(frame.frame_type, 0x00);
    assert_eq!(frame.stream_id, stream_id);
    ios_core::xpc::message::decode_message(bytes::Bytes::from(frame.payload)).unwrap()
}

async fn write_empty_xpc<S>(stream: &mut S, stream_id: u32)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_raw_frame(
        stream,
        0x00,
        0,
        stream_id,
        &ios_core::xpc::message::encode_message(&ios_core::xpc::XpcMessage {
            flags: ios_core::xpc::message::flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .unwrap(),
    )
    .await;
}

async fn write_xpc_response<S>(stream: &mut S, stream_id: u32, body: ios_core::xpc::XpcValue)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_raw_frame(
        stream,
        0x00,
        0,
        stream_id,
        &ios_core::xpc::message::encode_message(&ios_core::xpc::XpcMessage {
            flags: ios_core::xpc::message::flags::ALWAYS_SET
                | ios_core::xpc::message::flags::DATA
                | ios_core::xpc::message::flags::REPLY,
            msg_id: 1,
            body: Some(body),
        })
        .unwrap(),
    )
    .await;
}

async fn write_xpc_response_fragmented<S>(
    stream: &mut S,
    stream_id: u32,
    body: ios_core::xpc::XpcValue,
    split_at: usize,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let payload = ios_core::xpc::message::encode_message(&ios_core::xpc::XpcMessage {
        flags: ios_core::xpc::message::flags::ALWAYS_SET
            | ios_core::xpc::message::flags::DATA
            | ios_core::xpc::message::flags::REPLY,
        msg_id: 1,
        body: Some(body),
    })
    .unwrap();
    let split_at = split_at.min(payload.len());
    write_raw_frame(stream, 0x00, 0, stream_id, &payload[..split_at]).await;
    write_raw_frame(stream, 0x00, 0, stream_id, &payload[split_at..]).await;
}

async fn write_data_frame<S>(stream: &mut S, stream_id: u32, payload: &[u8])
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_raw_frame(stream, 0x00, 0, stream_id, payload).await;
}

async fn write_headers_only<S>(stream: &mut S, stream_id: u32)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_raw_frame(stream, 0x01, 0x04, stream_id, &[]).await;
}

async fn write_file_transfer_request<S>(stream: &mut S, stream_id: u32)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_raw_frame(
        stream,
        0x00,
        0,
        stream_id,
        &ios_core::xpc::message::encode_message(&ios_core::xpc::XpcMessage {
            flags: ios_core::xpc::message::flags::ALWAYS_SET
                | ios_core::xpc::message::flags::FILE_TX_STREAM_REQUEST,
            msg_id: 0,
            body: None,
        })
        .unwrap(),
    )
    .await;
}

async fn read_window_update_pair<S>(stream: &mut S, stream_id: u32)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let conn_window = read_raw_frame(stream).await;
    assert_eq!(conn_window.frame_type, 0x08);
    assert_eq!(conn_window.stream_id, 0);

    let stream_window = read_raw_frame(stream).await;
    assert_eq!(stream_window.frame_type, 0x08);
    assert_eq!(stream_window.stream_id, stream_id);
}

async fn write_raw_frame<S>(
    stream: &mut S,
    frame_type: u8,
    flags: u8,
    stream_id: u32,
    payload: &[u8],
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let len = payload.len();
    let mut frame = Vec::with_capacity(9 + len);
    frame.push(((len >> 16) & 0xff) as u8);
    frame.push(((len >> 8) & 0xff) as u8);
    frame.push((len & 0xff) as u8);
    frame.push(frame_type);
    frame.push(flags);
    frame.extend_from_slice(&(stream_id & 0x7fff_ffff).to_be_bytes());
    frame.extend_from_slice(payload);
    tokio::io::AsyncWriteExt::write_all(stream, &frame)
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(stream).await.unwrap();
}

async fn read_raw_frame<S>(stream: &mut S) -> TestFrame
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut header = [0u8; 9];
    tokio::io::AsyncReadExt::read_exact(stream, &mut header)
        .await
        .unwrap();
    let len = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
    let mut payload = vec![0u8; len];
    if len > 0 {
        tokio::io::AsyncReadExt::read_exact(stream, &mut payload)
            .await
            .unwrap();
    }
    TestFrame {
        frame_type: header[3],
        flags: header[4],
        stream_id: u32::from_be_bytes([header[5] & 0x7f, header[6], header[7], header[8]]),
        payload,
    }
}

struct TestFrame {
    frame_type: u8,
    flags: u8,
    stream_id: u32,
    payload: Vec<u8>,
}
