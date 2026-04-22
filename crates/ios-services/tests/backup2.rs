use std::path::PathBuf;

use ios_services::backup2::{Mobilebackup2Client, RestoreOptions, VersionExchange};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Duration};

fn encode_plist(value: &plist::Value) -> Vec<u8> {
    let mut payload = Vec::new();
    plist::to_writer_xml(&mut payload, value).expect("plist serialization");

    let mut framed = Vec::with_capacity(4 + payload.len());
    framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(&payload);
    framed
}

#[tokio::test]
async fn handshake_reports_negotiated_backup2_protocol_version() {
    let (client_side, mut server_side) = tokio::io::duplex(4096);

    let server = tokio::spawn(async move {
        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300.into()),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write device version exchange");

        let mut request_len = [0u8; 4];
        server_side
            .read_exact(&mut request_len)
            .await
            .expect("read client ack length");
        let request_len = u32::from_be_bytes(request_len) as usize;
        let mut request_payload = vec![0u8; request_len];
        server_side
            .read_exact(&mut request_payload)
            .await
            .expect("read client ack payload");

        let request: plist::Value =
            plist::from_bytes(&request_payload).expect("decode client ack payload");
        assert_eq!(
            request,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::String("DLVersionsOk".into()),
                plist::Value::Integer(300.into()),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");

        let mut hello_len = [0u8; 4];
        server_side
            .read_exact(&mut hello_len)
            .await
            .expect("read hello length");
        let hello_len = u32::from_be_bytes(hello_len) as usize;
        let mut hello_payload = vec![0u8; hello_len];
        server_side
            .read_exact(&mut hello_payload)
            .await
            .expect("read hello payload");

        let hello: plist::Value = plist::from_bytes(&hello_payload).expect("decode hello payload");
        assert_eq!(
            hello,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        String::from("MessageName"),
                        plist::Value::String("Hello".into()),
                    ),
                    (
                        String::from("SupportedProtocolVersions"),
                        plist::Value::Array(
                            vec![plist::Value::Real(2.0), plist::Value::Real(2.1),]
                        ),
                    ),
                ])),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (String::from("ProtocolVersion"), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");
    });

    let mut client = Mobilebackup2Client::new(client_side);
    let version = client
        .version_exchange()
        .await
        .expect("backup2 handshake should succeed");

    assert_eq!(
        version,
        VersionExchange {
            device_link_version: 300,
            protocol_version: 2.1,
            local_versions: vec![2.0, 2.1],
        }
    );

    server.await.expect("server task should finish");
}

fn temp_backup_root() -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("ios-tunnel-backup2-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).expect("create temp backup root");
    path
}

fn encode_upload_frame(device_name: &str, file_name: &str, contents: &[u8]) -> Vec<u8> {
    let mut framed = Vec::new();
    framed.extend_from_slice(&(device_name.len() as u32).to_be_bytes());
    framed.extend_from_slice(device_name.as_bytes());
    framed.extend_from_slice(&(file_name.len() as u32).to_be_bytes());
    framed.extend_from_slice(file_name.as_bytes());
    framed.extend_from_slice(&((contents.len() + 1) as u32).to_be_bytes());
    framed.push(0x0c);
    framed.extend_from_slice(contents);
    framed.extend_from_slice(&(1u32).to_be_bytes());
    framed.push(0x00);
    framed.extend_from_slice(&(0u32).to_be_bytes());
    framed
}

#[tokio::test]
async fn backup_creates_layout_and_writes_uploaded_files() {
    let backup_root = temp_backup_root();
    let device_id = "00008150-000A584C0E62401C";
    let uploaded_contents = b"hello backup2";
    let (client_side, mut server_side) = tokio::io::duplex(64 * 1024);

    let server = tokio::spawn(async move {
        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300.into()),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write version exchange");

        let mut request_len = [0u8; 4];
        server_side
            .read_exact(&mut request_len)
            .await
            .expect("read version ack length");
        let request_len = u32::from_be_bytes(request_len) as usize;
        let mut request_payload = vec![0u8; request_len];
        server_side
            .read_exact(&mut request_payload)
            .await
            .expect("read version ack payload");

        let request: plist::Value =
            plist::from_bytes(&request_payload).expect("decode version ack payload");
        assert_eq!(
            request,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::String("DLVersionsOk".into()),
                plist::Value::Integer(300.into()),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");

        let mut hello_len = [0u8; 4];
        server_side
            .read_exact(&mut hello_len)
            .await
            .expect("read hello length");
        let hello_len = u32::from_be_bytes(hello_len) as usize;
        let mut hello_payload = vec![0u8; hello_len];
        server_side
            .read_exact(&mut hello_payload)
            .await
            .expect("read hello payload");
        let hello: plist::Value = plist::from_bytes(&hello_payload).expect("decode hello payload");
        assert_eq!(
            hello,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        String::from("MessageName"),
                        plist::Value::String("Hello".into()),
                    ),
                    (
                        String::from("SupportedProtocolVersions"),
                        plist::Value::Array(
                            vec![plist::Value::Real(2.0), plist::Value::Real(2.1),]
                        ),
                    ),
                ])),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (String::from("ProtocolVersion"), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");

        let mut backup_len = [0u8; 4];
        server_side
            .read_exact(&mut backup_len)
            .await
            .expect("read backup length");
        let backup_len = u32::from_be_bytes(backup_len) as usize;
        let mut backup_payload = vec![0u8; backup_len];
        server_side
            .read_exact(&mut backup_payload)
            .await
            .expect("read backup payload");
        let backup_request: plist::Value =
            plist::from_bytes(&backup_payload).expect("decode backup payload");
        assert_eq!(
            backup_request,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        String::from("MessageName"),
                        plist::Value::String("Backup".into()),
                    ),
                    (
                        String::from("TargetIdentifier"),
                        plist::Value::String(device_id.into()),
                    ),
                ])),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageCreateDirectory".into()),
                plist::Value::String("Snapshot".into()),
            ])))
            .await
            .expect("write create directory request");

        let mut status_len = [0u8; 4];
        server_side
            .read_exact(&mut status_len)
            .await
            .expect("read directory status length");
        let status_len = u32::from_be_bytes(status_len) as usize;
        let mut status_payload = vec![0u8; status_len];
        server_side
            .read_exact(&mut status_payload)
            .await
            .expect("read directory status payload");
        let directory_status: plist::Value =
            plist::from_bytes(&status_payload).expect("decode directory status payload");
        assert_eq!(
            directory_status,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                plist::Value::Integer(0.into()),
                plist::Value::String("___EmptyParameterString___".into()),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageUploadFiles".into()),
                plist::Value::Array(vec![plist::Value::String("Manifest.db".into())]),
                plist::Value::Integer(100.into()),
            ])))
            .await
            .expect("write upload files request");
        server_side
            .write_all(&encode_upload_frame(
                "MediaDomain/Manifest.db",
                "Manifest.db",
                uploaded_contents,
            ))
            .await
            .expect("write upload file payload");

        let mut upload_status_len = [0u8; 4];
        server_side
            .read_exact(&mut upload_status_len)
            .await
            .expect("read upload status length");
        let upload_status_len = u32::from_be_bytes(upload_status_len) as usize;
        let mut upload_status_payload = vec![0u8; upload_status_len];
        server_side
            .read_exact(&mut upload_status_payload)
            .await
            .expect("read upload status payload");
        let upload_status: plist::Value =
            plist::from_bytes(&upload_status_payload).expect("decode upload status payload");
        assert_eq!(
            upload_status,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                plist::Value::Integer(0.into()),
                plist::Value::String("___EmptyParameterString___".into()),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (
                        String::from("Content"),
                        plist::Value::String("Backup finished".into()),
                    ),
                ])),
            ])))
            .await
            .expect("write backup completion");
    });

    let mut client = Mobilebackup2Client::new(client_side);
    let info_plist = plist::Dictionary::from_iter([
        (
            "Device Name".to_string(),
            plist::Value::String("Codex iPhone".into()),
        ),
        (
            "Target Identifier".to_string(),
            plist::Value::String(device_id.into()),
        ),
    ]);

    let result = client
        .backup(&backup_root, device_id, true, &info_plist)
        .await
        .expect("backup should succeed");

    assert_eq!(result.device_link_version, 300);
    assert_eq!(result.protocol_version, 2.1);
    assert_eq!(result.layout.device_directory, backup_root.join(device_id));
    assert!(result.layout.device_directory.join("Info.plist").exists());
    assert!(result.layout.device_directory.join("Status.plist").exists());
    assert!(result
        .layout
        .device_directory
        .join("Manifest.plist")
        .exists());
    assert!(result.layout.device_directory.join("Snapshot").is_dir());
    assert_eq!(
        std::fs::read(result.layout.device_directory.join("Manifest.db"))
            .expect("uploaded file should exist"),
        uploaded_contents
    );

    server.await.expect("server task should finish");
    std::fs::remove_dir_all(&backup_root).expect("cleanup temp backup root");
}

#[tokio::test]
async fn change_password_sends_requested_fields() {
    let backup_root = temp_backup_root();
    let device_id = "00008150-000A584C0E62401C";
    let (client_side, mut server_side) = tokio::io::duplex(16 * 1024);

    let server = tokio::spawn(async move {
        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300.into()),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write version exchange");

        let mut ack_len = [0u8; 4];
        server_side
            .read_exact(&mut ack_len)
            .await
            .expect("read ack len");
        let ack_len = u32::from_be_bytes(ack_len) as usize;
        let mut ack_payload = vec![0u8; ack_len];
        server_side
            .read_exact(&mut ack_payload)
            .await
            .expect("read ack payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");

        let mut hello_len = [0u8; 4];
        server_side
            .read_exact(&mut hello_len)
            .await
            .expect("read hello len");
        let hello_len = u32::from_be_bytes(hello_len) as usize;
        let mut hello_payload = vec![0u8; hello_len];
        server_side
            .read_exact(&mut hello_payload)
            .await
            .expect("read hello payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (String::from("ProtocolVersion"), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");

        let mut change_len = [0u8; 4];
        server_side
            .read_exact(&mut change_len)
            .await
            .expect("read change-password len");
        let change_len = u32::from_be_bytes(change_len) as usize;
        let mut change_payload = vec![0u8; change_len];
        server_side
            .read_exact(&mut change_payload)
            .await
            .expect("read change-password payload");
        let change_message: plist::Value =
            plist::from_bytes(&change_payload).expect("decode change-password payload");
        assert_eq!(
            change_message,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        String::from("MessageName"),
                        plist::Value::String("ChangePassword".into()),
                    ),
                    (
                        String::from("TargetIdentifier"),
                        plist::Value::String(device_id.into()),
                    ),
                    (
                        String::from("NewPassword"),
                        plist::Value::String("codex1234".into()),
                    ),
                ])),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    String::from("ErrorCode"),
                    plist::Value::Integer(0.into()),
                )])),
            ])))
            .await
            .expect("write completion");
    });

    let mut client = Mobilebackup2Client::new(client_side);
    client
        .change_password(&backup_root, device_id, None, Some("codex1234"))
        .await
        .expect("change password should succeed");

    server.await.expect("server task should finish");
    std::fs::remove_dir_all(&backup_root).expect("cleanup temp backup root");
}

#[tokio::test]
async fn change_password_streams_requested_manifest_files() {
    let backup_root = temp_backup_root();
    let device_id = "00008150-000A584C0E62401C";
    let snapshot_dir = backup_root.join(device_id).join("Snapshot");
    std::fs::create_dir_all(&snapshot_dir).expect("create snapshot dir");
    std::fs::write(snapshot_dir.join("Manifest.plist"), b"<plist/>")
        .expect("write snapshot manifest");
    let (client_side, mut server_side) = tokio::io::duplex(32 * 1024);

    let server = tokio::spawn(async move {
        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300.into()),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write version exchange");

        let mut ack_len = [0u8; 4];
        server_side
            .read_exact(&mut ack_len)
            .await
            .expect("read ack len");
        let ack_len = u32::from_be_bytes(ack_len) as usize;
        let mut ack_payload = vec![0u8; ack_len];
        server_side
            .read_exact(&mut ack_payload)
            .await
            .expect("read ack payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");

        let mut hello_len = [0u8; 4];
        server_side
            .read_exact(&mut hello_len)
            .await
            .expect("read hello len");
        let hello_len = u32::from_be_bytes(hello_len) as usize;
        let mut hello_payload = vec![0u8; hello_len];
        server_side
            .read_exact(&mut hello_payload)
            .await
            .expect("read hello payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (String::from("ProtocolVersion"), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");

        let mut change_len = [0u8; 4];
        server_side
            .read_exact(&mut change_len)
            .await
            .expect("read change-password len");
        let change_len = u32::from_be_bytes(change_len) as usize;
        let mut change_payload = vec![0u8; change_len];
        server_side
            .read_exact(&mut change_payload)
            .await
            .expect("read change-password payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDownloadFiles".into()),
                plist::Value::Array(vec![plist::Value::String(format!(
                    "{device_id}/Snapshot/Manifest.plist"
                ))]),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "DLDeviceIOCallbacks".to_string(),
                    plist::Value::Integer(0.into()),
                )])),
                plist::Value::Real(0.0),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write download-files request");

        let mut name_len = [0u8; 4];
        server_side
            .read_exact(&mut name_len)
            .await
            .expect("read file name len");
        let name_len = u32::from_be_bytes(name_len) as usize;
        let mut name_buf = vec![0u8; name_len];
        server_side
            .read_exact(&mut name_buf)
            .await
            .expect("read file name");
        let file_name = String::from_utf8(name_buf).expect("utf8 file name");
        assert_eq!(file_name, format!("{device_id}/Snapshot/Manifest.plist"));

        let mut chunk_len = [0u8; 4];
        server_side
            .read_exact(&mut chunk_len)
            .await
            .expect("read chunk len");
        let chunk_len = u32::from_be_bytes(chunk_len) as usize;
        let mut chunk_code = [0u8; 1];
        server_side
            .read_exact(&mut chunk_code)
            .await
            .expect("read chunk code");
        assert_eq!(chunk_code[0], 0x0c);
        let mut chunk = vec![0u8; chunk_len - 1];
        server_side
            .read_exact(&mut chunk)
            .await
            .expect("read chunk bytes");
        assert_eq!(chunk, b"<plist/>");

        let mut success_len = [0u8; 4];
        server_side
            .read_exact(&mut success_len)
            .await
            .expect("read success len");
        let success_len = u32::from_be_bytes(success_len) as usize;
        assert_eq!(success_len, 1);
        let mut success_code = [0u8; 1];
        server_side
            .read_exact(&mut success_code)
            .await
            .expect("read success code");
        assert_eq!(success_code[0], 0x00);

        let mut terminator = [0u8; 4];
        server_side
            .read_exact(&mut terminator)
            .await
            .expect("read terminator");
        let terminator = u32::from_be_bytes(terminator);
        assert_eq!(terminator, 0);

        let mut status_len = [0u8; 4];
        server_side
            .read_exact(&mut status_len)
            .await
            .expect("read status len");
        let status_len = u32::from_be_bytes(status_len) as usize;
        let mut status_payload = vec![0u8; status_len];
        server_side
            .read_exact(&mut status_payload)
            .await
            .expect("read status payload");
        let status_message: plist::Value =
            plist::from_bytes(&status_payload).expect("decode status payload");
        assert_eq!(
            status_message,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                plist::Value::Integer(0.into()),
                plist::Value::String("___EmptyParameterString___".into()),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    String::from("ErrorCode"),
                    plist::Value::Integer(0.into()),
                )])),
            ])))
            .await
            .expect("write completion");
    });

    let mut client = Mobilebackup2Client::new(client_side);
    client
        .change_password(&backup_root, device_id, None, Some("codex1234"))
        .await
        .expect("change password should succeed");

    server.await.expect("server task should finish");
    std::fs::remove_dir_all(&backup_root).expect("cleanup temp backup root");
}

#[tokio::test]
async fn change_password_reports_multi_status_when_requested_file_is_missing() {
    let backup_root = temp_backup_root();
    let device_id = "00008150-000A584C0E62401C";
    let (client_side, mut server_side) = tokio::io::duplex(32 * 1024);

    let server = tokio::spawn(async move {
        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300.into()),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write version exchange");

        let mut ack_len = [0u8; 4];
        server_side
            .read_exact(&mut ack_len)
            .await
            .expect("read ack len");
        let ack_len = u32::from_be_bytes(ack_len) as usize;
        let mut ack_payload = vec![0u8; ack_len];
        server_side
            .read_exact(&mut ack_payload)
            .await
            .expect("read ack payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");

        let mut hello_len = [0u8; 4];
        server_side
            .read_exact(&mut hello_len)
            .await
            .expect("read hello len");
        let hello_len = u32::from_be_bytes(hello_len) as usize;
        let mut hello_payload = vec![0u8; hello_len];
        server_side
            .read_exact(&mut hello_payload)
            .await
            .expect("read hello payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (String::from("ProtocolVersion"), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");

        let mut change_len = [0u8; 4];
        server_side
            .read_exact(&mut change_len)
            .await
            .expect("read change-password len");
        let change_len = u32::from_be_bytes(change_len) as usize;
        let mut change_payload = vec![0u8; change_len];
        server_side
            .read_exact(&mut change_payload)
            .await
            .expect("read change-password payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDownloadFiles".into()),
                plist::Value::Array(vec![plist::Value::String(format!(
                    "{device_id}/Snapshot/Manifest.plist"
                ))]),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "DLDeviceIOCallbacks".to_string(),
                    plist::Value::Integer(0.into()),
                )])),
                plist::Value::Real(0.0),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write download-files request");

        let mut name_len = [0u8; 4];
        server_side
            .read_exact(&mut name_len)
            .await
            .expect("read file name len");
        let name_len = u32::from_be_bytes(name_len) as usize;
        let mut name_buf = vec![0u8; name_len];
        server_side
            .read_exact(&mut name_buf)
            .await
            .expect("read file name");
        let file_name = String::from_utf8(name_buf).expect("utf8 file name");
        assert_eq!(file_name, format!("{device_id}/Snapshot/Manifest.plist"));

        let mut error_len = [0u8; 4];
        server_side
            .read_exact(&mut error_len)
            .await
            .expect("read error len");
        let error_len = u32::from_be_bytes(error_len) as usize;
        let mut error_code = [0u8; 1];
        server_side
            .read_exact(&mut error_code)
            .await
            .expect("read error code");
        assert_eq!(error_code[0], 0x06);
        let mut error_payload = vec![0u8; error_len - 1];
        server_side
            .read_exact(&mut error_payload)
            .await
            .expect("read error payload");
        assert!(!error_payload.is_empty(), "missing local error payload");

        let mut terminator = [0u8; 4];
        server_side
            .read_exact(&mut terminator)
            .await
            .expect("read terminator");
        assert_eq!(u32::from_be_bytes(terminator), 0);

        let mut status_len = [0u8; 4];
        server_side
            .read_exact(&mut status_len)
            .await
            .expect("read status len");
        let status_len = u32::from_be_bytes(status_len) as usize;
        let mut status_payload = vec![0u8; status_len];
        server_side
            .read_exact(&mut status_payload)
            .await
            .expect("read status payload");
        let status_message: plist::Value =
            plist::from_bytes(&status_payload).expect("decode status payload");
        let status_array = status_message.as_array().expect("status should be array");
        assert_eq!(
            status_array[0],
            plist::Value::String("DLMessageStatusResponse".into())
        );
        assert_eq!(status_array[1], plist::Value::Integer((-13).into()));
        assert_eq!(status_array[2], plist::Value::String("Multi status".into()));
        let failures = status_array[3]
            .as_dictionary()
            .expect("status payload should be dictionary");
        let failure = failures
            .get(&format!("{device_id}/Snapshot/Manifest.plist"))
            .and_then(plist::Value::as_dictionary)
            .expect("missing failure entry");
        assert!(failure.contains_key("DLFileErrorString"));

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    String::from("ErrorCode"),
                    plist::Value::Integer(0.into()),
                )])),
            ])))
            .await
            .expect("write completion");
    });

    let mut client = Mobilebackup2Client::new(client_side);
    client
        .change_password(&backup_root, device_id, None, Some("codex1234"))
        .await
        .expect("change password should succeed");

    server.await.expect("server task should finish");
    std::fs::remove_dir_all(&backup_root).expect("cleanup temp backup root");
}

#[tokio::test]
async fn info_sends_requested_fields_and_returns_content() {
    let backup_root = temp_backup_root();
    let device_id = "00008150-000A584C0E62401C";
    let device_dir = backup_root.join(device_id);
    std::fs::create_dir_all(&device_dir).expect("create device dir");
    std::fs::write(device_dir.join("Info.plist"), b"info").expect("write Info.plist");
    std::fs::write(device_dir.join("Manifest.plist"), b"manifest").expect("write Manifest.plist");
    std::fs::write(device_dir.join("Status.plist"), b"status").expect("write Status.plist");
    let (client_side, mut server_side) = tokio::io::duplex(16 * 1024);

    let server = tokio::spawn(async move {
        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300.into()),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write version exchange");

        let mut ack_len = [0u8; 4];
        server_side
            .read_exact(&mut ack_len)
            .await
            .expect("read ack len");
        let ack_len = u32::from_be_bytes(ack_len) as usize;
        let mut ack_payload = vec![0u8; ack_len];
        server_side
            .read_exact(&mut ack_payload)
            .await
            .expect("read ack payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");

        let mut hello_len = [0u8; 4];
        server_side
            .read_exact(&mut hello_len)
            .await
            .expect("read hello len");
        let hello_len = u32::from_be_bytes(hello_len) as usize;
        let mut hello_payload = vec![0u8; hello_len];
        server_side
            .read_exact(&mut hello_payload)
            .await
            .expect("read hello payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (String::from("ProtocolVersion"), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");

        let mut info_len = [0u8; 4];
        server_side
            .read_exact(&mut info_len)
            .await
            .expect("read info len");
        let info_len = u32::from_be_bytes(info_len) as usize;
        let mut info_payload = vec![0u8; info_len];
        server_side
            .read_exact(&mut info_payload)
            .await
            .expect("read info payload");
        let info_message: plist::Value =
            plist::from_bytes(&info_payload).expect("decode info payload");
        assert_eq!(
            info_message,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        String::from("MessageName"),
                        plist::Value::String("Info".into()),
                    ),
                    (
                        String::from("TargetIdentifier"),
                        plist::Value::String(device_id.into()),
                    ),
                ])),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (
                        String::from("Content"),
                        plist::Value::String("Backup metadata".into()),
                    ),
                ])),
            ])))
            .await
            .expect("write completion");
    });

    let mut client = Mobilebackup2Client::new(client_side);
    let result = client
        .info(&backup_root, device_id, None)
        .await
        .expect("info should succeed");

    assert_eq!(result, Some(plist::Value::String("Backup metadata".into())));
    server.await.expect("server task should finish");
    std::fs::remove_dir_all(&backup_root).expect("cleanup temp backup root");
}

#[tokio::test]
async fn info_disconnects_even_when_process_message_returns_error() {
    let backup_root = temp_backup_root();
    let device_id = "00008150-000A584C0E62401C";
    let device_dir = backup_root.join(device_id);
    std::fs::create_dir_all(&device_dir).expect("create device dir");
    std::fs::write(device_dir.join("Info.plist"), b"info").expect("write Info.plist");
    std::fs::write(device_dir.join("Manifest.plist"), b"manifest").expect("write Manifest.plist");
    std::fs::write(device_dir.join("Status.plist"), b"status").expect("write Status.plist");
    let (client_side, mut server_side) = tokio::io::duplex(16 * 1024);

    let server = tokio::spawn(async move {
        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300.into()),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write version exchange");

        let mut ack_len = [0u8; 4];
        server_side
            .read_exact(&mut ack_len)
            .await
            .expect("read ack len");
        let ack_len = u32::from_be_bytes(ack_len) as usize;
        let mut ack_payload = vec![0u8; ack_len];
        server_side
            .read_exact(&mut ack_payload)
            .await
            .expect("read ack payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");

        let mut hello_len = [0u8; 4];
        server_side
            .read_exact(&mut hello_len)
            .await
            .expect("read hello len");
        let hello_len = u32::from_be_bytes(hello_len) as usize;
        let mut hello_payload = vec![0u8; hello_len];
        server_side
            .read_exact(&mut hello_payload)
            .await
            .expect("read hello payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (String::from("ProtocolVersion"), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");

        let mut info_len = [0u8; 4];
        server_side
            .read_exact(&mut info_len)
            .await
            .expect("read info len");
        let info_len = u32::from_be_bytes(info_len) as usize;
        let mut info_payload = vec![0u8; info_len];
        server_side
            .read_exact(&mut info_payload)
            .await
            .expect("read info payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    String::from("ErrorCode"),
                    plist::Value::Integer(10.into()),
                )])),
            ])))
            .await
            .expect("write failing completion");

        let mut disconnect_len = [0u8; 4];
        timeout(
            Duration::from_secs(1),
            server_side.read_exact(&mut disconnect_len),
        )
        .await
        .expect("disconnect frame should arrive")
        .expect("read disconnect len");
        let disconnect_len = u32::from_be_bytes(disconnect_len) as usize;
        let mut disconnect_payload = vec![0u8; disconnect_len];
        timeout(
            Duration::from_secs(1),
            server_side.read_exact(&mut disconnect_payload),
        )
        .await
        .expect("disconnect payload should arrive")
        .expect("read disconnect payload");
        let disconnect: plist::Value =
            plist::from_bytes(&disconnect_payload).expect("decode disconnect payload");
        assert_eq!(
            disconnect,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageDisconnect".into()),
                plist::Value::String("___EmptyParameterString___".into()),
            ])
        );
    });

    let mut client = Mobilebackup2Client::new(client_side);
    let error = client
        .info(&backup_root, device_id, None)
        .await
        .expect_err("info should surface process errors");

    assert!(error.to_string().contains("ErrorCode=10"));
    server.await.expect("server task should finish");
    std::fs::remove_dir_all(&backup_root).expect("cleanup temp backup root");
}

#[tokio::test]
async fn list_includes_source_identifier_in_request() {
    let backup_root = temp_backup_root();
    let device_id = "00008150-000A584C0E62401C";
    let source_id = "backup-source-udid";
    let source_dir = backup_root.join(source_id);
    std::fs::create_dir_all(&source_dir).expect("create source dir");
    std::fs::write(source_dir.join("Info.plist"), b"info").expect("write Info.plist");
    std::fs::write(source_dir.join("Manifest.plist"), b"manifest").expect("write Manifest.plist");
    std::fs::write(source_dir.join("Status.plist"), b"status").expect("write Status.plist");
    let (client_side, mut server_side) = tokio::io::duplex(16 * 1024);

    let server = tokio::spawn(async move {
        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300.into()),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write version exchange");

        let mut ack_len = [0u8; 4];
        server_side
            .read_exact(&mut ack_len)
            .await
            .expect("read ack len");
        let ack_len = u32::from_be_bytes(ack_len) as usize;
        let mut ack_payload = vec![0u8; ack_len];
        server_side
            .read_exact(&mut ack_payload)
            .await
            .expect("read ack payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");

        let mut hello_len = [0u8; 4];
        server_side
            .read_exact(&mut hello_len)
            .await
            .expect("read hello len");
        let hello_len = u32::from_be_bytes(hello_len) as usize;
        let mut hello_payload = vec![0u8; hello_len];
        server_side
            .read_exact(&mut hello_payload)
            .await
            .expect("read hello payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (String::from("ProtocolVersion"), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");

        let mut list_len = [0u8; 4];
        server_side
            .read_exact(&mut list_len)
            .await
            .expect("read list len");
        let list_len = u32::from_be_bytes(list_len) as usize;
        let mut list_payload = vec![0u8; list_len];
        server_side
            .read_exact(&mut list_payload)
            .await
            .expect("read list payload");
        let list_message: plist::Value =
            plist::from_bytes(&list_payload).expect("decode list payload");
        assert_eq!(
            list_message,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        String::from("MessageName"),
                        plist::Value::String("List".into()),
                    ),
                    (
                        String::from("TargetIdentifier"),
                        plist::Value::String(device_id.into()),
                    ),
                    (
                        String::from("SourceIdentifier"),
                        plist::Value::String(source_id.into()),
                    ),
                ])),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (
                        String::from("Content"),
                        plist::Value::String("file1.csv".into()),
                    ),
                ])),
            ])))
            .await
            .expect("write completion");
    });

    let mut client = Mobilebackup2Client::new(client_side);
    let result = client
        .list(&backup_root, device_id, Some(source_id))
        .await
        .expect("list should succeed");

    assert_eq!(result, Some(plist::Value::String("file1.csv".into())));
    server.await.expect("server task should finish");
    std::fs::remove_dir_all(&backup_root).expect("cleanup temp backup root");
}

#[tokio::test]
async fn restore_sends_requested_fields_and_options() {
    let backup_root = temp_backup_root();
    let device_id = "00008150-000A584C0E62401C";
    let source_id = "backup-source-udid";
    let source_dir = backup_root.join(source_id);
    std::fs::create_dir_all(&source_dir).expect("create source dir");
    std::fs::write(source_dir.join("Info.plist"), b"info").expect("write Info.plist");
    plist::to_file_xml(
        source_dir.join("Manifest.plist"),
        &plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "IsEncrypted".to_string(),
            plist::Value::Boolean(true),
        )])),
    )
    .expect("write Manifest.plist");
    std::fs::write(source_dir.join("Status.plist"), b"status").expect("write Status.plist");
    let (client_side, mut server_side) = tokio::io::duplex(16 * 1024);

    let server = tokio::spawn(async move {
        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300.into()),
                plist::Value::Integer(0.into()),
            ])))
            .await
            .expect("write version exchange");

        let mut ack_len = [0u8; 4];
        server_side
            .read_exact(&mut ack_len)
            .await
            .expect("read ack len");
        let ack_len = u32::from_be_bytes(ack_len) as usize;
        let mut ack_payload = vec![0u8; ack_len];
        server_side
            .read_exact(&mut ack_payload)
            .await
            .expect("read ack payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");

        let mut hello_len = [0u8; 4];
        server_side
            .read_exact(&mut hello_len)
            .await
            .expect("read hello len");
        let hello_len = u32::from_be_bytes(hello_len) as usize;
        let mut hello_payload = vec![0u8; hello_len];
        server_side
            .read_exact(&mut hello_payload)
            .await
            .expect("read hello payload");

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (String::from("ProtocolVersion"), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");

        let mut restore_len = [0u8; 4];
        server_side
            .read_exact(&mut restore_len)
            .await
            .expect("read restore len");
        let restore_len = u32::from_be_bytes(restore_len) as usize;
        let mut restore_payload = vec![0u8; restore_len];
        server_side
            .read_exact(&mut restore_payload)
            .await
            .expect("read restore payload");
        let restore_message: plist::Value =
            plist::from_bytes(&restore_payload).expect("decode restore payload");
        assert_eq!(
            restore_message,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        String::from("MessageName"),
                        plist::Value::String("Restore".into()),
                    ),
                    (
                        String::from("TargetIdentifier"),
                        plist::Value::String(device_id.into()),
                    ),
                    (
                        String::from("SourceIdentifier"),
                        plist::Value::String(source_id.into()),
                    ),
                    (
                        String::from("Password"),
                        plist::Value::String("codex1234".into()),
                    ),
                    (
                        String::from("Options"),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([
                            (
                                String::from("RestoreShouldReboot"),
                                plist::Value::Boolean(false),
                            ),
                            (
                                String::from("RestoreDontCopyBackup"),
                                plist::Value::Boolean(true),
                            ),
                            (
                                String::from("RestorePreserveSettings"),
                                plist::Value::Boolean(false),
                            ),
                            (
                                String::from("RestoreSystemFiles"),
                                plist::Value::Boolean(true),
                            ),
                            (
                                String::from("RemoveItemsNotRestored"),
                                plist::Value::Boolean(true),
                            ),
                        ])),
                    ),
                ])),
            ])
        );

        server_side
            .write_all(&encode_plist(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (String::from("ErrorCode"), plist::Value::Integer(0.into())),
                    (
                        String::from("Content"),
                        plist::Value::String("Restore finished".into()),
                    ),
                ])),
            ])))
            .await
            .expect("write completion");
    });

    let mut client = Mobilebackup2Client::new(client_side);
    let result = client
        .restore(
            &backup_root,
            device_id,
            RestoreOptions {
                system: true,
                reboot: false,
                copy: false,
                settings: false,
                remove: true,
                password: Some("codex1234"),
                source_identifier: Some(source_id),
            },
        )
        .await
        .expect("restore should succeed");

    assert_eq!(result.device_link_version, 300);
    assert_eq!(result.protocol_version, 2.1);
    assert_eq!(result.layout.device_directory, backup_root.join(source_id));
    server.await.expect("server task should finish");
    std::fs::remove_dir_all(&backup_root).expect("cleanup temp backup root");
}

#[tokio::test]
async fn restore_rejects_encrypted_backup_without_password() {
    let backup_root = temp_backup_root();
    let device_id = "00008150-000A584C0E62401C";
    let device_dir = backup_root.join(device_id);
    std::fs::create_dir_all(&device_dir).expect("create device dir");
    std::fs::write(device_dir.join("Info.plist"), b"info").expect("write Info.plist");
    plist::to_file_xml(
        device_dir.join("Manifest.plist"),
        &plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "IsEncrypted".to_string(),
            plist::Value::Boolean(true),
        )])),
    )
    .expect("write Manifest.plist");
    std::fs::write(device_dir.join("Status.plist"), b"status").expect("write Status.plist");
    let (client_side, _server_side) = tokio::io::duplex(1024);

    let mut client = Mobilebackup2Client::new(client_side);
    let error = client
        .restore(&backup_root, device_id, RestoreOptions::default())
        .await
        .expect_err("restore should require a password for encrypted backups");

    assert!(error.to_string().contains("requires a password"));
    std::fs::remove_dir_all(&backup_root).expect("cleanup temp backup root");
}
