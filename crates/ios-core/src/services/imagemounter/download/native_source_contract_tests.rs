use super::*;
use crate::services::imagemounter::test_assets::{http, zip, Directory};
use crate::test_native::has_source;
use std::{
    error::Error,
    io,
    pin::Pin,
    task::{Context, Poll},
};

fn original_io(error: &ImageMounterError) -> &io::Error {
    let ImageMounterError::Io(outer) = error else {
        panic!("native IO was flattened")
    };
    let original = outer.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(outer.kind(), original.kind());
    assert_eq!(
        crate::classify_error(error),
        crate::classify_error(original)
    );
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
    original
}

#[test]
fn native_source_extraction_file_open_retains_original_os_error() {
    let root = Directory::new();
    let error = extract_archive_file(&root.0.join("missing.zip"), &root.0).unwrap_err();
    assert_eq!(original_io(&error).kind(), io::ErrorKind::NotFound);
    assert!(original_io(&error).raw_os_error().is_some());
}

#[tokio::test]
async fn native_source_cache_reads_and_writes_keep_original_io() {
    let mut errors = Vec::new();
    for signature in [false, true] {
        let root = Directory::new();
        let dir = root.0.join("standard/16.4");
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("DeveloperDiskImage.dmg");
        let sig = dir.join("DeveloperDiskImage.dmg.signature");
        if signature {
            std::fs::write(&image, b"image").unwrap();
            std::fs::create_dir(&sig).unwrap();
        } else {
            std::fs::create_dir(&image).unwrap();
            std::fs::write(&sig, b"signature").unwrap();
        }
        errors.push(
            DdiDownloader::new(Some(root.0.clone()))
                .download_standard(&semver::Version::new(16, 4, 0))
                .await
                .err()
                .unwrap(),
        );
    }
    let root = Directory::new();
    std::fs::create_dir(root.0.join("Restore")).unwrap();
    for name in [
        "PersonalizedDMG.dmg",
        "PersonalizedDMG.trustcache",
        "BuildManifest.plist",
    ] {
        errors.push(
            load_personalized_from_restore_dir(&root.0.join("Restore"))
                .await
                .err()
                .unwrap(),
        );
        std::fs::write(root.0.join("Restore").join(name), b"owned-private").unwrap();
    }
    let file = root.0.join("file");
    std::fs::write(&file, b"regular file").unwrap();
    errors.push(
        cache_files(&file, &[("entry", b"bytes")])
            .await
            .unwrap_err(),
    );
    std::fs::create_dir(root.0.join("entry")).unwrap();
    errors.push(
        cache_files(&root.0, &[("entry", b"bytes")])
            .await
            .unwrap_err(),
    );
    for error in errors {
        assert!(original_io(&error).raw_os_error().is_some());
    }
}

#[test]
fn native_source_zip_open_and_entry_errors_keep_zip_type() {
    let root = Directory::new();
    let mut errors = vec![
        extract_standard_ddi(b"owned-private invalid zip", "16.4")
            .err()
            .unwrap(),
        extract_zip(b"owned-private invalid zip", &root.0).unwrap_err(),
    ];
    let mut bytes = zip(&[("Restore/DeveloperDiskImage.dmg", b"image")]);
    bytes[6] |= 1;
    let central = bytes
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    bytes[central + 8] |= 1;
    assert!(
        zip::ZipArchive::new(std::io::Cursor::new(&bytes)).is_ok(),
        "archive opens; entry access requires an unavailable password"
    );
    errors.push(extract_standard_ddi(&bytes, "16.4").err().unwrap());
    errors.push(extract_zip(&bytes, &root.0).unwrap_err());
    for error in errors {
        assert!(has_source::<zip::result::ZipError>(&error));
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        assert!(!format!("{error:?}").contains("owned-private"));
    }
}

#[test]
fn native_source_zip_copy_crc_faults_keep_original_io() {
    for entry in [
        "DeveloperDiskImage.dmg",
        "DeveloperDiskImage.dmg.signature",
        "Restore/payload",
    ] {
        let payload = b"owned-private checksum payload";
        let mut bytes = zip(&[
            (
                "DeveloperDiskImage.dmg",
                if entry == "DeveloperDiskImage.dmg" {
                    payload.as_slice()
                } else {
                    b"image"
                },
            ),
            (
                "DeveloperDiskImage.dmg.signature",
                if entry == "DeveloperDiskImage.dmg.signature" {
                    payload.as_slice()
                } else {
                    b"signature"
                },
            ),
            (
                "Restore/payload",
                if entry == "Restore/payload" {
                    payload.as_slice()
                } else {
                    b"payload"
                },
            ),
        ]);
        let offset = bytes
            .windows(payload.len())
            .position(|slice| slice == payload)
            .unwrap();
        bytes[offset] ^= 1;
        let root = Directory::new();
        let error = if entry.starts_with("Restore/") {
            extract_zip(&bytes, &root.0).unwrap_err()
        } else {
            extract_standard_ddi(&bytes, "16.4").err().unwrap()
        };
        original_io(&error);
    }
}

#[test]
fn native_source_zip_filesystem_faults_keep_original_os_error() {
    let bytes = zip(&[("Restore/payload", b"bytes")]);
    let root = Directory::new();
    let dest = root.0.join("regular-file");
    std::fs::write(&dest, b"bytes").unwrap();
    let first = extract_zip(&bytes, &dest).unwrap_err();
    std::fs::create_dir_all(root.0.join("Restore/payload")).unwrap();
    let second = extract_zip(&bytes, &root.0).unwrap_err();
    for error in [first, second] {
        assert!(original_io(&error).raw_os_error().is_some());
    }
}

#[test]
fn native_source_zip_reader_io_keeps_nested_os_cause() {
    struct FaultReader {
        reader: io::Cursor<Vec<u8>>,
        error: Option<io::Error>,
    }
    impl io::Read for FaultReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(self.error.take().expect("first native read fails"))
        }
    }
    impl io::Seek for FaultReader {
        fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
            io::Seek::seek(&mut self.reader, pos)
        }
    }
    let root = Directory::new();
    let original = std::fs::File::create(&root.0).unwrap_err();
    let kind = original.kind();
    let class = crate::classify_error(&original);
    let error = extract_zip_from_reader(
        FaultReader {
            reader: io::Cursor::new(zip(&[("Restore/payload", b"bytes")])),
            error: Some(original),
        },
        &root.0,
    )
    .unwrap_err();
    let ImageMounterError::Io(outer) = &error else {
        panic!("native ZIP IO lost")
    };
    let zip = outer
        .source()
        .unwrap()
        .downcast_ref::<zip::result::ZipError>()
        .unwrap();
    let zip::result::ZipError::Io(original) = zip else {
        panic!("nested IO source lost")
    };
    assert_eq!(outer.kind(), kind);
    assert_eq!(original.kind(), kind);
    assert!(original.raw_os_error().is_some());
    assert_eq!(crate::classify_error(&error), class);
}

#[test]
fn native_source_proxy_and_builder_errors_keep_reqwest() {
    for error in [
        build_client_with_proxy(reqwest::Client::builder().no_proxy(), Some("http://["))
            .err()
            .unwrap(),
        build_client_with_proxy(reqwest::Client::builder().no_proxy().user_agent("\n"), None)
            .err()
            .unwrap(),
    ] {
        assert!(has_source::<reqwest::Error>(&error));
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::InvalidInput
        );
    }
}

#[tokio::test]
async fn native_source_http_request_and_standard_body_keep_timeout() {
    let (send, _) = http(None, true, |client, url| async move {
        request_archive(&client, &url).await.err().unwrap()
    })
    .await;
    let (body, _) = http(
        Some(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n"),
        true,
        |client, url| async move {
            read_standard_archive(request_archive(&client, &url).await.unwrap())
                .await
                .unwrap_err()
        },
    )
    .await;
    for error in [send, body] {
        assert!(has_source::<reqwest::Error>(&error));
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        assert!(!format!("{error:?}").contains("owned-private"));
    }
}

struct FaultWriter {
    error: Option<io::Error>,
    on_flush: bool,
    written: usize,
}
impl tokio::io::AsyncWrite for FaultWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        if !self.on_flush {
            return Poll::Ready(Err(self.error.take().unwrap()));
        }
        self.written += data.len();
        Poll::Ready(Ok(data.len()))
    }
    fn poll_flush(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Err(self.error.take().unwrap()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn native_source_spool_create_chunk_write_flush_keep_original_causes() {
    let root = Directory::new();
    let path = root.0.clone();
    let (create, _) = http(
        Some(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPK\x03\x04"),
        false,
        |client, url| async move {
            download_response_to_file(client.get(&url).send().await.unwrap(), &path)
                .await
                .unwrap_err()
        },
    )
    .await;
    assert!(original_io(&create).raw_os_error().is_some());
    let path = root.0.join("partial");
    let check_path = path.clone();
    let (chunk, _) = http(
        Some(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n"),
        true,
        |client, url| async move {
            download_response_to_file(client.get(&url).send().await.unwrap(), &path)
                .await
                .unwrap_err()
        },
    )
    .await;
    assert!(has_source::<reqwest::Error>(&chunk));
    assert_eq!(crate::classify_error(&chunk), crate::ErrorClass::Timeout);
    assert_eq!(
        std::fs::metadata(check_path).unwrap().len(),
        0,
        "existing partial-spool behavior remains"
    );
    for on_flush in [false, true] {
        let error = std::fs::File::create(&root.0).unwrap_err();
        let (result, _) = http(
            Some(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPK\x03\x04"),
            false,
            |client, url| async move {
                let response = client.get(&url).send().await.unwrap();
                let url = response.url().clone();
                let mut writer = FaultWriter {
                    error: Some(error),
                    on_flush,
                    written: 0,
                };
                let error = copy_archive_body(response, &mut writer, &url)
                    .await
                    .unwrap_err();
                assert_eq!(writer.written, if on_flush { 4 } else { 0 });
                error
            },
        )
        .await;
        assert!(original_io(&result).raw_os_error().is_some());
    }
}

#[tokio::test]
async fn native_source_extraction_join_preserves_cancel_and_panic() {
    let task = tokio::spawn(std::future::pending::<Result<(), ImageMounterError>>());
    task.abort();
    let cancelled = extraction_result(task).await.unwrap_err();
    let panic = extraction_result(tokio::task::spawn_blocking(
        || -> Result<(), ImageMounterError> { panic!("owned-private") },
    ))
    .await
    .unwrap_err();
    for (error, class) in [
        (cancelled, crate::ErrorClass::Cancelled),
        (panic, crate::ErrorClass::Unknown),
    ] {
        assert!(has_source::<tokio::task::JoinError>(&error));
        assert_eq!(crate::classify_error(&error), class);
        assert!(!format!("{error:?}").contains("owned-private"));
    }
}

#[tokio::test]
async fn asset_guards_keep_missing_archive_and_html_rejections_source_free() {
    let empty = zip(&[]);
    let error = extract_standard_ddi(&empty, "16.4").err().unwrap();
    assert!(matches!(error, ImageMounterError::Download(_)));
    assert!(!has_source::<zip::result::ZipError>(&error));
    let root = Directory::new();
    for response in [
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 4\r\nConnection: close\r\n\r\nHTML".as_slice(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nHTML".as_slice(),
    ] {
        let path=root.0.join(uuid::Uuid::new_v4().to_string());
        let (error,_) = http(Some(response),false,|client,url| async move { download_response_to_file(client.get(&url).send().await.unwrap(),&path).await.unwrap_err() }).await;
        assert!(matches!(error, ImageMounterError::Download(_)));
        assert!(!has_source::<reqwest::Error>(&error));
    }
}
