//! Entire public install over an owned service and tiny local IPA; no device.

use super::*;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::io::ReadBuf;

struct Ipa {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
}
impl Ipa {
    fn new() -> Self {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("ios-owned-zip-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("Example.ipa");
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            "Payload/Owned.app/file.txt",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"owned-payload").unwrap();
        std::fs::write(&path, zip.finish().unwrap().into_inner()).unwrap();
        Self { dir, path }
    }
    fn close(self) {
        let dir = self.dir.clone();
        drop(self);
        assert!(!dir.exists());
    }
}
impl Drop for Ipa {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[derive(Default)]
struct Service {
    replies: std::io::Cursor<Vec<u8>>,
    written: Vec<u8>,
    flushes: usize,
}
impl AsyncRead for Service {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let count = std::io::Read::read(&mut self.replies, buf.initialize_unfilled())?;
        buf.advance(count);
        Poll::Ready(Ok(()))
    }
}
impl AsyncWrite for Service {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.written.extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.flushes += 1;
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn frame(value: plist::Value) -> Vec<u8> {
    let mut body = Vec::new();
    plist::to_writer_binary(&mut body, &value).unwrap();
    let mut result = (body.len() as u32).to_be_bytes().to_vec();
    result.extend_from_slice(&body);
    result
}
fn progress(percent: u32, status: &str) -> Vec<u8> {
    frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
        "InstallProgressDict",
        plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("PercentComplete", plist::Value::Integer(percent.into())),
            ("Status", plist::Value::String(status.into())),
        ])),
    )])))
}
fn complete() -> Vec<u8> {
    frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
        "Status",
        plist::Value::String("DataComplete".into()),
    )])))
}

#[tokio::test]
async fn observer_failure_retains_native_source_and_does_not_consume_completion() {
    let ipa = Ipa::new();
    let first = progress(20, "owned-first");
    let second = progress(50, "owned-second");
    for (kind, class) in [
        (
            std::io::ErrorKind::BrokenPipe,
            crate::ErrorClass::Disconnected,
        ),
        (
            std::io::ErrorKind::PermissionDenied,
            crate::ErrorClass::PermissionDenied,
        ),
        (std::io::ErrorKind::TimedOut, crate::ErrorClass::Timeout),
        (std::io::ErrorKind::Other, crate::ErrorClass::Io),
    ] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer = Arc::clone(&seen);
        let mut service = Service {
            replies: std::io::Cursor::new([first.clone(), second.clone(), complete()].concat()),
            ..Service::default()
        };
        let error = install_ipa_with_checked_progress(
            &mut service,
            &ipa.path,
            Some(Box::new(move |percent, status| {
                let mut records = observer.lock().unwrap();
                records.push((percent, status.to_string()));
                if records.len() == 2 {
                    Err(std::io::Error::new(kind, "owned-private-observer-source"))
                } else {
                    Ok(())
                }
            })),
        )
        .await
        .unwrap_err();
        let ZipConduitError::Io(native) = &error else {
            panic!("observer IO was replaced");
        };
        assert_eq!(native.kind(), kind);
        assert_eq!(native.to_string(), "owned-private-observer-source");
        assert_eq!(crate::classify_error(&error), class);
        assert!(!format!("{error:?}").contains("owned-private"));
        assert_eq!(
            *seen.lock().unwrap(),
            [(20, "owned-first".into()), (50, "owned-second".into())]
        );
        assert_eq!(
            service.replies.position() as usize,
            first.len() + second.len()
        );
        assert!(service.written.ends_with(&[0x50, 0x4b, 0x01, 0x02]));
        assert_eq!(service.flushes, 2);
    }
    ipa.close();
}

#[tokio::test]
async fn checked_legacy_and_no_observer_keep_transfer_bytes_and_completion_order() {
    let ipa = Ipa::new();
    for terminal in [complete(), progress(100, "Complete")] {
        let replies = [
            progress(20, "owned-first"),
            progress(50, "owned-second"),
            terminal,
        ]
        .concat();
        let legacy_seen = Arc::new(Mutex::new(Vec::new()));
        let checked_seen = Arc::new(Mutex::new(Vec::new()));
        let observer = Arc::clone(&legacy_seen);
        let mut legacy = Service {
            replies: std::io::Cursor::new(replies.clone()),
            ..Service::default()
        };
        install_ipa(
            &mut legacy,
            &ipa.path,
            Some(Box::new(move |percent, status| {
                observer.lock().unwrap().push((percent, status.to_string()));
            })),
        )
        .await
        .unwrap();
        let observer = Arc::clone(&checked_seen);
        let mut checked = Service {
            replies: std::io::Cursor::new(replies.clone()),
            ..Service::default()
        };
        install_ipa_with_checked_progress(
            &mut checked,
            &ipa.path,
            Some(Box::new(move |percent, status| {
                observer.lock().unwrap().push((percent, status.to_string()));
                Ok(())
            })),
        )
        .await
        .unwrap();
        assert_eq!(
            *legacy_seen.lock().unwrap(),
            [(20, "owned-first".into()), (50, "owned-second".into())]
        );
        assert_eq!(*checked_seen.lock().unwrap(), *legacy_seen.lock().unwrap());
        assert_eq!(checked.written, legacy.written);
        assert_eq!(checked.flushes, legacy.flushes);
        assert_eq!(checked.replies.position() as usize, replies.len());
        let mut absent = Service {
            replies: std::io::Cursor::new(replies),
            ..Service::default()
        };
        install_ipa_with_checked_progress(&mut absent, &ipa.path, None)
            .await
            .unwrap();
        assert_eq!(absent.written, legacy.written);
    }
    ipa.close();
}

#[tokio::test]
async fn remote_error_after_progress_retains_existing_error_before_complete() {
    let ipa = Ipa::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observer = Arc::clone(&seen);
    let terminal = frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
        "InstallProgressDict",
        plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("Status", plist::Value::String("Complete".into())),
            (
                "Error",
                plist::Value::String("owned-private-timeout-unsupported".into()),
            ),
            (
                "ErrorDescription",
                plist::Value::String("owned-private-detail".into()),
            ),
        ])),
    )])));
    let mut service = Service {
        replies: std::io::Cursor::new([progress(20, "owned-first"), terminal, complete()].concat()),
        ..Service::default()
    };
    let error = install_ipa_with_checked_progress(
        &mut service,
        &ipa.path,
        Some(Box::new(move |percent, status| {
            observer.lock().unwrap().push((percent, status.to_string()));
            Ok(())
        })),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ZipConduitError::Install(_)));
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ServiceFailure
    );
    assert!(!format!("{error:?}").contains("owned-private"));
    assert_eq!(seen.lock().unwrap().len(), 1);
    ipa.close();
}

#[tokio::test]
async fn malformed_or_truncated_reply_after_progress_preserves_native_failure() {
    let ipa = Ipa::new();
    for truncated in [false, true] {
        let suffix = if truncated {
            vec![0, 0, 0, 5, b'?']
        } else {
            let xml =
                b"<?xml version=\"1.0\"?><plist version=\"1.0\"><integer>bad</integer></plist>";
            let mut bytes = (xml.len() as u32).to_be_bytes().to_vec();
            bytes.extend_from_slice(xml);
            bytes
        };
        let seen = Arc::new(Mutex::new(0));
        let observer = Arc::clone(&seen);
        let mut service = Service {
            replies: std::io::Cursor::new([progress(20, "owned-first"), suffix].concat()),
            ..Service::default()
        };
        let error = install_ipa_with_checked_progress(
            &mut service,
            &ipa.path,
            Some(Box::new(move |_, _| {
                *observer.lock().unwrap() += 1;
                Ok(())
            })),
        )
        .await
        .unwrap_err();
        if truncated {
            let ZipConduitError::Io(native) = &error else {
                panic!("native EOF lost");
            };
            assert_eq!(native.kind(), std::io::ErrorKind::UnexpectedEof);
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Io);
        } else {
            assert!(matches!(error, ZipConduitError::Plist(_)));
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        }
        assert_eq!(*seen.lock().unwrap(), 1);
    }
    ipa.close();
}
