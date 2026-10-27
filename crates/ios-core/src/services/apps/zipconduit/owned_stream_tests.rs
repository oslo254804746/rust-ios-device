//! Offline tests of the production owned stream and bounded local decoder.
use super::*;
use std::{io::Write, path::PathBuf, time::Duration};

struct Ipa {
    dir: PathBuf,
    path: PathBuf,
}
impl Ipa {
    fn new(bytes: &[u8]) -> Self {
        let dir = std::env::temp_dir().join(format!("ios-zip-stream-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("Example.ipa");
        std::fs::write(&path, bytes).unwrap();
        Self { dir, path }
    }
}
impl Drop for Ipa {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn compressed_package() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let block = vec![0x5a; 64 * 1024];
    for i in 0..3 {
        zip.start_file(format!("Payload/App.app/{i}"), options)
            .unwrap();
        for _ in 0..32 {
            zip.write_all(&block).unwrap();
        }
    }
    zip.finish().unwrap().into_inner()
}

async fn file_record(peer: &mut tokio::io::DuplexStream) -> (String, u32, u32) {
    let mut head = [0; 30];
    peer.read_exact(&mut head).await.unwrap();
    assert_eq!(&head[..4], &[0x50, 0x4b, 0x03, 0x04]);
    assert_eq!(u16::from_le_bytes([head[8], head[9]]), 0);
    let size = u32::from_le_bytes(head[18..22].try_into().unwrap());
    let crc = u32::from_le_bytes(head[14..18].try_into().unwrap());
    let name_len = u16::from_le_bytes(head[26..28].try_into().unwrap()) as usize;
    let extra_len = u16::from_le_bytes(head[28..30].try_into().unwrap()) as usize;
    let mut name = vec![0; name_len];
    peer.read_exact(&mut name).await.unwrap();
    let mut extra = vec![0; extra_len];
    peer.read_exact(&mut extra).await.unwrap();
    (String::from_utf8(name).unwrap(), size, crc)
}

#[tokio::test]
async fn compressed_multi_entry_install_streams_valid_store_records_over_duplex() {
    let bytes = compressed_package();
    assert!(bytes.len() < 64 * 1024);
    let ipa = Ipa::new(&bytes);
    let (client, mut peer) = tokio::io::duplex(4096);
    let path = ipa.path.clone();
    // Spawning also enforces that the public owned future remains Send.
    let install = tokio::spawn(async move {
        install_ipa_owned_with_timeout(client, &path, None, Duration::from_secs(30)).await
    });
    let size = peer.read_u32().await.unwrap() as usize;
    let mut init = vec![0; size];
    peer.read_exact(&mut init).await.unwrap();
    let init: plist::Dictionary = plist::from_bytes(&init).unwrap();
    assert_eq!(
        init["MediaSubdir"].as_string(),
        Some("PublicStaging/Example.ipa")
    );
    let (name, size, _) = file_record(&mut peer).await;
    assert_eq!((name.as_str(), size), ("META-INF/", 0));
    let (name, size, _) = file_record(&mut peer).await;
    assert_eq!(name, "META-INF/com.apple.ZipMetadata.plist");
    let mut meta = vec![0; size as usize];
    peer.read_exact(&mut meta).await.unwrap();
    let meta: plist::Dictionary = plist::from_bytes(&meta).unwrap();
    assert_eq!(meta["RecordCount"].as_signed_integer(), Some(5));
    assert_eq!(
        meta["TotalUncompressedBytes"].as_signed_integer(),
        Some(3 * 2 * 1024 * 1024)
    );
    for i in 0..3 {
        let (name, size, expected_crc) = file_record(&mut peer).await;
        assert_eq!(name, format!("Payload/App.app/{i}"));
        assert_eq!(size, 2 * 1024 * 1024);
        let mut left = size as usize;
        let mut block = [0; 8192];
        let mut crc = crc32fast::Hasher::new();
        while left != 0 {
            let n = left.min(block.len());
            peer.read_exact(&mut block[..n]).await.unwrap();
            assert!(block[..n].iter().all(|b| *b == 0x5a));
            crc.update(&block[..n]);
            left -= n;
        }
        assert_eq!(crc.finalize(), expected_crc);
    }
    let mut end = [0; 4];
    peer.read_exact(&mut end).await.unwrap();
    assert_eq!(end, [0x50, 0x4b, 0x01, 0x02]);
    let complete =
        plist::Dictionary::from_iter([("Status", plist::Value::String("DataComplete".into()))]);
    let mut body = Vec::new();
    plist::to_writer_binary(&mut body, &complete).unwrap();
    peer.write_u32(body.len() as u32).await.unwrap();
    peer.write_all(&body).await.unwrap();
    install.await.unwrap().unwrap();
    assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn corrupt_packages_fail_before_the_first_wire_byte_with_sources() {
    use std::error::Error;
    for (bytes, class, phase) in super::extraction_contract_tests::damaged_packages() {
        let ipa = Ipa::new(&bytes);
        let (client, mut peer) = tokio::io::duplex(1);
        let error =
            install_ipa_owned_with_timeout(client, &ipa.path, None, Duration::from_secs(10))
                .await
                .unwrap_err();
        assert_eq!(crate::classify_error(&error), class);
        assert!(error.to_string().contains(phase));
        assert!(error.source().is_some());
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn central_directory_limit_and_zip64_are_rejected_before_wire() {
    for zip64 in [false, true] {
        let mut bytes = compressed_package();
        let end = bytes
            .windows(4)
            .rposition(|x| x == [0x50, 0x4b, 0x05, 0x06])
            .unwrap();
        let count = if zip64 { u16::MAX } else { 16_385 };
        bytes[end + 10..end + 12].copy_from_slice(&count.to_le_bytes());
        let ipa = Ipa::new(&bytes);
        let (client, mut peer) = tokio::io::duplex(1);
        let error =
            install_ipa_owned_with_timeout(client, &ipa.path, None, Duration::from_secs(10))
                .await
                .unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::InvalidInput
        );
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn nested_zip_and_footer_like_resource_bytes_are_valid_outer_payloads() {
    let nested = super::extraction_contract_tests::stored_zip();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer
        .start_file("Payload/App.app/resource.zip", options)
        .unwrap();
    writer.write_all(&nested).unwrap();
    writer.start_file("Payload/App.app/blob", options).unwrap();
    // This is data, not a footer; its count fields resemble ZIP64.
    let mut footer_like = [0xff; 22];
    footer_like[..4].copy_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
    writer.write_all(&footer_like).unwrap();
    let ipa = Ipa::new(&writer.finish().unwrap().into_inner());
    let prepared = owned_archive::PreparedArchive::open(
        &ipa.path,
        tokio::time::Instant::now() + Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(prepared.record_count(), 4);
    assert_eq!(
        prepared.total_size(),
        nested.len() as u64 + footer_like.len() as u64
    );
}

#[tokio::test]
async fn oversized_entry_and_truncated_deflate_are_rejected_before_wire() {
    for oversized in [false, true] {
        let mut bytes = compressed_package();
        let central = bytes
            .windows(4)
            .position(|x| x == [0x50, 0x4b, 0x01, 0x02])
            .unwrap();
        if oversized {
            bytes[central + 24..central + 28]
                .copy_from_slice(&(1024u32 * 1024 * 1024 + 1).to_le_bytes());
        } else {
            bytes[central + 20..central + 24].copy_from_slice(&1u32.to_le_bytes());
        }
        let ipa = Ipa::new(&bytes);
        let (client, mut peer) = tokio::io::duplex(1);
        let error =
            install_ipa_owned_with_timeout(client, &ipa.path, None, Duration::from_secs(10))
                .await
                .unwrap_err();
        if oversized {
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::InvalidInput
            );
        } else {
            assert!(matches!(error, ZipConduitError::Io(_)));
            assert!(error.to_string().contains("read IPA ZIP entry"));
        }
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn deadline_and_caller_drop_during_directory_scan_close_the_peer() {
    // Directory snapshot and entry validation yield during preparation.
    let ipa = Ipa::new(&compressed_package());
    for cancel in [false, true] {
        let (client, mut peer) = tokio::io::duplex(1);
        let path = ipa.path.clone();
        let install = tokio::spawn(async move {
            install_ipa_owned_with_timeout(client, &path, None, Duration::from_millis(100)).await
        });
        tokio::task::yield_now().await;
        if cancel {
            install.abort();
            assert!(install.await.unwrap_err().is_cancelled());
        } else {
            tokio::time::advance(Duration::from_millis(100)).await;
            let error = install.await.unwrap().unwrap_err();
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        }
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    }
}
