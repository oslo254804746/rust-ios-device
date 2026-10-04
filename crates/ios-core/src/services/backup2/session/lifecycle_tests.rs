use super::*;
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
};
use tokio::{
    io::{DuplexStream, ReadBuf},
    sync::Notify,
};

struct Root(PathBuf);
impl Root {
    fn new(label: &str) -> Self {
        Self(tests::test_backup_root(label, "owned-device"))
    }
    fn layout(&self) -> BackupDirectoryLayout {
        BackupDirectoryLayout {
            root: self.0.clone(),
            device_directory: self.0.join("owned-device"),
            target_identifier: "owned-device".into(),
        }
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("owned fixture cleanup");
    }
}

#[derive(Default)]
struct Probe {
    blocked: AtomicBool,
    fail: AtomicBool,
    polls: AtomicUsize,
    blocked_writes: AtomicUsize,
    read_bytes: AtomicUsize,
    changed: Notify,
}

#[derive(Debug)]
struct CleanupCause;
impl std::fmt::Display for CleanupCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("owned-private-cleanup-value")
    }
}
impl std::error::Error for CleanupCause {}

struct Stream {
    inner: DuplexStream,
    probe: Arc<Probe>,
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.probe.polls.fetch_add(1, Ordering::SeqCst);
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        let count = buf.filled().len() - before;
        if count > 0 {
            self.probe.read_bytes.fetch_add(count, Ordering::SeqCst);
            self.probe.changed.notify_one();
        }
        result
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.probe.polls.fetch_add(1, Ordering::SeqCst);
        if self.probe.fail.load(Ordering::SeqCst) {
            return Poll::Ready(Err(std::io::Error::new(
                ErrorKind::PermissionDenied,
                CleanupCause,
            )));
        }
        if self.probe.blocked.load(Ordering::SeqCst) {
            self.probe.blocked_writes.fetch_add(1, Ordering::SeqCst);
            self.probe.changed.notify_one();
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.probe.polls.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

fn client() -> (Mobilebackup2Client<Stream>, DuplexStream, Arc<Probe>) {
    let (inner, peer) = tokio::io::duplex(4096);
    let probe = Arc::new(Probe::default());
    (
        Mobilebackup2Client::new(Stream {
            inner,
            probe: Arc::clone(&probe),
        }),
        peer,
        probe,
    )
}
fn frame(value: plist::Value) -> Vec<u8> {
    let mut payload = Vec::new();
    plist::to_writer_xml(&mut payload, &value).unwrap();
    let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend(payload);
    bytes
}
fn restore_options() -> RestoreOptions<'static> {
    RestoreOptions {
        system: false,
        reboot: false,
        copy: false,
        settings: false,
        remove: false,
        password: None,
        source_identifier: None,
    }
}
fn timeout(error: &Mobilebackup2Error, issued: bool) {
    let Mobilebackup2Error::Io(error) = error else {
        panic!("expected operation IO timeout")
    };
    assert_eq!(error.kind(), ErrorKind::TimedOut);
    assert_eq!(
        error
            .get_ref()
            .and_then(|source| source.downcast_ref::<tokio::time::error::Elapsed>())
            .is_some(),
        issued
    );
}

#[test]
fn lifecycle_backup2_cleanup_lookup_bounds_source_cycles_and_retains_context_wrappers() {
    #[derive(Debug)]
    struct Cycle(Arc<AtomicUsize>);
    impl std::fmt::Display for Cycle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("owned cyclic source")
        }
    }
    impl std::error::Error for Cycle {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Some(self)
        }
    }
    let visits = Arc::new(AtomicUsize::new(0));
    let cyclic = Mobilebackup2Error::Io(std::io::Error::other(Cycle(Arc::clone(&visits))));
    assert!(cyclic.cleanup_failure().is_none());
    assert!(visits.load(Ordering::SeqCst) > 0 && visits.load(Ordering::SeqCst) <= 32);
    let retained = finish_results::<()>(
        Err(Mobilebackup2Error::Protocol("owned-private-primary".into())),
        Err(Mobilebackup2Error::Io(std::io::Error::new(
            ErrorKind::PermissionDenied,
            CleanupCause,
        ))),
    )
    .unwrap_err();
    let wrapped = Mobilebackup2Error::Io(crate::error::context::retained_io(
        ErrorKind::Other,
        "owned cleanup context",
        retained,
    ));
    let evidence = wrapped
        .cleanup_failure()
        .expect("typed aggregate through IO/context");
    assert_eq!(
        crate::classify_error(evidence.operation_error()),
        crate::ErrorClass::Protocol
    );
    assert_eq!(
        crate::classify_error(evidence.cleanup_error()),
        crate::ErrorClass::PermissionDenied
    );
    assert!(!format!("{wrapped:?} {evidence:?}").contains("owned-private"));
}
fn rejects<T>(result: Result<T, Mobilebackup2Error>) {
    assert!(
        matches!(result, Err(Mobilebackup2Error::Protocol(message)) if message.contains("session is unusable"))
    );
}
async fn rejected<S: AsyncRead + AsyncWrite + Unpin>(
    client: &mut Mobilebackup2Client<S>,
    root: &Path,
) {
    tokio::time::timeout(Duration::from_secs(1), async {
        client.set_operation_timeout(None);
        assert!(!client.is_usable());
        let info = plist::Dictionary::new();
        rejects(client.version_exchange().await);
        rejects(client.backup(root, "owned-device", false, &info).await);
        rejects(
            client
                .backup_with_options(root, "owned-device", false, &info, BackupOptions::default())
                .await,
        );
        rejects(
            client
                .change_password(root, "owned-device", None, None)
                .await,
        );
        rejects(
            client
                .restore(root, "owned-device", restore_options())
                .await,
        );
        rejects(client.info(root, "owned-device", None).await);
        rejects(client.list(root, "owned-device", None).await);
        rejects(client.unback(root, "owned-device", None, None).await);
        rejects(
            client
                .extract(root, "owned-device", "HomeDomain", "file", None, None)
                .await,
        );
    })
    .await
    .expect("invalid session must reject without I/O or host work");
}
async fn request(peer: &mut DuplexStream) {
    tests::complete_device_link_handshake(peer).await;
    let message = tests::read_test_frame(peer).await;
    let array = message.as_array().unwrap();
    assert_eq!(array[0].as_string(), Some("DLMessageProcessMessage"));
    let dictionary = array[1].as_dictionary().unwrap();
    assert_eq!(
        dictionary
            .get("MessageName")
            .and_then(plist::Value::as_string),
        Some("Info")
    );
    assert_eq!(
        dictionary
            .get("TargetIdentifier")
            .and_then(plist::Value::as_string),
        Some("owned-device")
    );
}

#[tokio::test]
async fn lifecycle_backup2_zero_overflow_unpolled_and_idle_waits_preserve_pre_io_state() {
    tokio::time::timeout(Duration::from_secs(1), async {
        let root = std::env::temp_dir().join(format!("ios-backup-never-created-{}", uuid::Uuid::new_v4()));
        let (mut client, mut peer, probe) = client();
        let info = plist::Dictionary::new();
        client.set_operation_timeout(Some(Duration::ZERO));
        timeout(&client.version_exchange().await.unwrap_err(), false);
        timeout(&client.backup(&root,"owned-device",false,&info).await.unwrap_err(),false);
        timeout(&client.backup_with_options(&root,"owned-device",false,&info,BackupOptions::default()).await.unwrap_err(),false);
        timeout(&client.change_password(&root,"owned-device",None,None).await.unwrap_err(),false);
        timeout(&client.restore(&root,"owned-device",restore_options()).await.unwrap_err(),false);
        timeout(&client.info(&root,"owned-device",None).await.unwrap_err(),false);
        timeout(&client.list(&root,"owned-device",None).await.unwrap_err(),false);
        timeout(&client.unback(&root,"owned-device",None,None).await.unwrap_err(),false);
        timeout(&client.extract(&root,"owned-device","HomeDomain","file",None,None).await.unwrap_err(),false);
        client.set_operation_timeout(Some(Duration::MAX));
        assert!(matches!(client.version_exchange().await,Err(Mobilebackup2Error::Io(error)) if error.kind()==ErrorKind::InvalidInput));
        drop(client.version_exchange());
        drop(client.backup(&root,"owned-device",false,&info));
        drop(client.backup_with_options(&root,"owned-device",false,&info,BackupOptions::default()));
        drop(client.change_password(&root,"owned-device",None,None));
        drop(client.restore(&root,"owned-device",restore_options()));
        drop(client.info(&root,"owned-device",None));
        drop(client.list(&root,"owned-device",None));
        drop(client.unback(&root,"owned-device",None,None));
        drop(client.extract(&root,"owned-device","HomeDomain","file",None,None));
        client.set_operation_timeout(None);
        assert!(matches!(client.info(&root,"../invalid",None).await,Err(Mobilebackup2Error::Protocol(_))));
        assert!(!root.exists());
        assert_eq!(probe.polls.load(Ordering::SeqCst),0);
        assert!(client.is_usable());
        assert!(tokio::time::timeout(Duration::from_millis(5),client.version_exchange()).await.is_err());
        assert!(client.is_usable());
        client.set_operation_timeout(Some(Duration::from_millis(5)));
        timeout(&client.version_exchange().await.unwrap_err(),true);
        assert!(client.is_usable());
        client.set_operation_timeout(None);
        let (version,())=tokio::join!(client.version_exchange(),tests::complete_device_link_handshake(&mut peer));
        assert_eq!(version.unwrap().protocol_version,2.1);
        assert!(client.is_usable());
    }).await.unwrap();
}

#[tokio::test]
async fn lifecycle_backup2_version_owner_survives_zero_byte_hello_write_wait() {
    tokio::time::timeout(Duration::from_secs(1), async {
        let root = Root::new("hello-blocked");
        let (mut client, mut peer, probe) = client();
        {
            let mut pending = Box::pin(client.version_exchange());
            let remote = async {
                peer.write_all(&frame(plist::Value::Array(vec![
                    "DLMessageVersionExchange".into(),
                    300u64.into(),
                ])))
                .await
                .unwrap();
                assert_eq!(
                    tests::read_test_frame(&mut peer).await,
                    plist::Value::Array(vec![
                        "DLMessageVersionExchange".into(),
                        "DLVersionsOk".into(),
                        300u64.into()
                    ])
                );
                probe.blocked.store(true, Ordering::SeqCst);
                peer.write_all(&frame(plist::Value::Array(vec![
                    "DLMessageDeviceReady".into()
                ])))
                .await
                .unwrap();
                while probe.blocked_writes.load(Ordering::SeqCst) == 0 {
                    probe.changed.notified().await;
                }
            };
            tokio::select! {_=remote=>{},_= &mut pending=>panic!("Hello write must remain blocked")}
        }
        assert!(
            client.device_link.is_usable(),
            "the empty nested write itself retained DeviceLink framing"
        );
        assert!(
            !client.is_usable(),
            "the version session still owned its completed prior messages"
        );
        probe.blocked.store(false, Ordering::SeqCst);
        let before = probe.polls.load(Ordering::SeqCst);
        rejected(&mut client, &root.0).await;
        assert_eq!(probe.polls.load(Ordering::SeqCst), before);
        drop(client);
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lifecycle_backup2_sent_request_cancel_or_fragmented_expiry_rejects_late_reply() {
    for mode in ["cancel", "expiry", "fragmented"] {
        tokio::time::timeout(Duration::from_secs(1),async {
            let root=Root::new(mode);
            let (mut client,mut peer,probe)=client();
            if mode!="cancel" {client.set_operation_timeout(Some(Duration::from_millis(100)));}
            let reply=frame(plist::Value::Array(vec!["DLMessageProcessMessage".into(),plist::Value::Dictionary(plist::Dictionary::from_iter([("ErrorCode",plist::Value::from(0u64))]))]));
            {
                let mut pending=Box::pin(client.info(&root.0,"owned-device",None));
                tokio::select! {_=request(&mut peer)=>{},_= &mut pending=>panic!("operation must await terminal reply")}
                if mode=="fragmented" {peer.write_all(&reply[..6]).await.unwrap();}
                if mode!="cancel" {timeout(&pending.await.unwrap_err(),true);}
            }
            if mode=="fragmented" {peer.write_all(&reply[6..]).await.unwrap();} else {peer.write_all(&reply).await.unwrap();}
            let before=probe.polls.load(Ordering::SeqCst);
            rejected(&mut client,&root.0).await;
            assert_eq!(probe.polls.load(Ordering::SeqCst),before);
            drop(client);
            assert_eq!(peer.read(&mut [0]).await.unwrap(),0);
        }).await.unwrap();
    }
}

#[tokio::test]
async fn lifecycle_backup2_total_budget_spans_negotiation_host_layout_and_final_reply() {
    tokio::time::timeout(Duration::from_secs(1), async {
        let root = Root::new("shared-budget");
        let (mut client, mut peer, _) = client();
        client.set_operation_timeout(Some(Duration::from_millis(200)));
        let remote = async {
            tokio::time::sleep(Duration::from_millis(125)).await;
            request(&mut peer).await;
            tokio::time::sleep(Duration::from_millis(125)).await;
            tests::complete_process_message(&mut peer, 0, None).await;
        };
        let (result, ()) = tokio::join!(client.info(&root.0, "owned-device", None), remote);
        timeout(&result.unwrap_err(), true);
        rejected(&mut client, &root.0).await;
        drop(client);
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lifecycle_backup2_raw_upload_cancel_and_bad_frame_preserve_file_and_close_reuse() {
    for malformed in [false, true] {
        tokio::time::timeout(Duration::from_secs(1),async {
            let root=Root::new("raw-upload");
            let parent=root.0.join("owned-device/HomeDomain");
            std::fs::create_dir(&parent).unwrap();
            let output=parent.join("file");
            std::fs::write(&output,b"original backup").unwrap();
            let (mut client,mut peer,probe)=client();
            {
                let mut pending=Box::pin(client.info(&root.0,"owned-device",None));
                tokio::select! {_=request(&mut peer)=>{},_= &mut pending=>panic!("operation must await commands")}
                let start=probe.read_bytes.load(Ordering::SeqCst);
                let mut bytes=frame(plist::Value::Array(vec!["DLMessageUploadFiles".into()]));
                for name in ["owned-device","HomeDomain/file"] {
                    bytes.extend(u32::try_from(name.len()).unwrap().to_be_bytes());
                    bytes.extend(name.as_bytes());
                }
                bytes.extend((if malformed {0u32} else {129u32}).to_be_bytes());
                bytes.push(FILE_TRANSFER_CODE_FILE_DATA);
                if !malformed {bytes.extend([0x33;3]);}
                peer.write_all(&bytes).await.unwrap();
                if malformed {
                    assert!(matches!(pending.await,Err(Mobilebackup2Error::Protocol(_))));
                } else {
                    tokio::select! {
                        _=async {while probe.read_bytes.load(Ordering::SeqCst)<start+bytes.len(){probe.changed.notified().await;}}=>{},
                        _=&mut pending=>panic!("raw upload must wait for remaining payload"),
                    }
                }
            }
            assert_eq!(std::fs::read(&output).unwrap(),b"original backup");
            assert_eq!(std::fs::read_dir(&parent).unwrap().count(),1,"staged partial was removed");
            if !malformed {peer.write_all(&[0x33;125]).await.unwrap();}
            let before=probe.polls.load(Ordering::SeqCst);
            rejected(&mut client,&root.0).await;
            assert_eq!(probe.polls.load(Ordering::SeqCst),before);
            drop(client);
            assert_eq!(peer.read(&mut [0]).await.unwrap(),0,"no disconnect was interleaved with raw transfer");
        }).await.unwrap();
    }
}

#[tokio::test]
async fn lifecycle_backup2_raw_download_cancel_emits_only_file_prefix_until_drop() {
    tokio::time::timeout(Duration::from_secs(1),async {
        let root=Root::new("raw-download");
        let parent=root.0.join("owned-device/HomeDomain");
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("file"),vec![0x77;8192]).unwrap();
        let (mut client,mut peer,probe)=client();
        {
            let mut pending=Box::pin(client.info(&root.0,"owned-device",None));
            tokio::select! {_=request(&mut peer)=>{},_= &mut pending=>panic!("operation must await commands")}
            let remote=async {
                peer.write_all(&frame(plist::Value::Array(vec!["DLMessageDownloadFiles".into(),plist::Value::Array(vec!["HomeDomain/file".into()])]))).await.unwrap();
                assert_eq!(read_prefixed_string(&mut peer).await.unwrap(),"HomeDomain/file");
                assert_eq!(read_u32_be(&mut peer).await.unwrap(),8193);
                assert_eq!(peer.read_u8().await.unwrap(),FILE_TRANSFER_CODE_FILE_DATA);
                let mut prefix=[0;3];peer.read_exact(&mut prefix).await.unwrap();assert_eq!(prefix,[0x77;3]);
            };
            tokio::select! {_=remote=>{},_= &mut pending=>panic!("raw send must block before completing its file")}
        }
        let before=probe.polls.load(Ordering::SeqCst);
        rejected(&mut client,&root.0).await;
        assert_eq!(probe.polls.load(Ordering::SeqCst),before);
        drop(client);
        let mut remainder=Vec::new();peer.read_to_end(&mut remainder).await.unwrap();
        assert!(!remainder.is_empty()&&remainder.len()<8192);
        assert!(remainder.iter().all(|byte|*byte==0x77),"no framed cleanup was appended to file data");
    }).await.unwrap();
}

#[tokio::test]
async fn lifecycle_backup2_host_wait_cancel_retains_completed_message_ownership() {
    struct Release(Option<std::sync::mpsc::Sender<()>>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    tokio::time::timeout(Duration::from_secs(2),async {
        let root=Root::new("host-wait");let layout=root.layout();
        let (mut client,mut peer,_)=client();
        let (release_tx,release_rx)=std::sync::mpsc::channel();let mut release=Release(Some(release_tx));
        let (entered_tx,entered_rx)=tokio::sync::oneshot::channel();
        let (done_tx,done_rx)=tokio::sync::oneshot::channel();
        {
            let mut pending=Box::pin(async {
                let guard=client.begin_operation(None)?;
                let result=await_operation(guard.deadline,async {
                    guard.client.version_exchange_inner().await?;
                    with_layout(&layout,move |_| {
                        let _=entered_tx.send(());
                        let result=release_rx.recv_timeout(Duration::from_secs(1));
                        let _=done_tx.send(());
                        result.map_err(|error|Mobilebackup2Error::Io(std::io::Error::other(error)))
                    }).await
                }).await;
                guard.finish_timed(result)
            });
            tokio::select! {
                _=async {tests::complete_device_link_handshake(&mut peer).await;entered_rx.await.unwrap();}=>{},
                _=&mut pending=>panic!("production blocking host wait must remain held"),
            }
        }
        assert!(client.device_link.is_usable());
        assert!(!client.is_usable());
        release.0.take().unwrap().send(()).unwrap();
        done_rx.await.unwrap();
        rejected(&mut client,&root.0).await;
        drop(client);assert_eq!(peer.read(&mut [0]).await.unwrap(),0);
    }).await.unwrap();
}

#[tokio::test]
async fn lifecycle_backup2_cleanup_native_and_total_expiry_keep_primary_and_secondary_causes() {
    for elapsed in [false, true] {
        for error_code in [0, 77] {
            tokio::time::timeout(Duration::from_secs(1),async {
            let root=Root::new("cleanup-causes");let (mut client,mut peer,probe)=client();
            if elapsed {client.set_operation_timeout(Some(Duration::from_millis(100)));}
            let remote=async {
                request(&mut peer).await;
                if elapsed {probe.blocked.store(true,Ordering::SeqCst);} else {probe.fail.store(true,Ordering::SeqCst);}
                tests::complete_process_message(&mut peer,error_code,None).await;
            };
            let (result,())=tokio::join!(client.info(&root.0,"owned-device",None),remote);
            let error=result.unwrap_err();
            if error_code!=0 {
                let retained=error.cleanup_failure().expect("both failures are retained");
                assert!(matches!(retained.operation_error(),Mobilebackup2Error::Protocol(message) if message.contains("ErrorCode=77")));
                assert_eq!(crate::classify_error(&error),crate::ErrorClass::Protocol);
                let cleanup=retained.cleanup_error();
                if elapsed {timeout(cleanup,true);} else {
                    let Mobilebackup2Error::DeviceLink(DeviceLinkError::Io(native))=cleanup else {panic!("native cleanup IO")};
                    assert_eq!(native.kind(),ErrorKind::PermissionDenied);
                    assert!(native.get_ref().unwrap().is::<CleanupCause>());
                }
                assert!(!format!("{error:?} {retained:?} {error}").contains("owned-private"));
            } else {
                assert!(error.cleanup_failure().is_none());
                if elapsed {timeout(&error,true);} else {assert_eq!(crate::classify_error(&error),crate::ErrorClass::PermissionDenied);}
            }
            probe.blocked.store(false,Ordering::SeqCst);probe.fail.store(false,Ordering::SeqCst);
            rejected(&mut client,&root.0).await;
            drop(client);assert_eq!(peer.read(&mut [0]).await.unwrap(),0);
        }).await.unwrap();
        }
    }
}

#[tokio::test]
async fn lifecycle_backup2_host_cleanup_expiry_preserves_earlier_failure_after_body_drop() {
    tokio::time::timeout(Duration::from_secs(1),async {
        let root=Root::new("host-cleanup");let (mut client,mut peer,_)=client();
        let (entered_tx,entered_rx)=tokio::sync::oneshot::channel();
        let run=async {
            let guard=client.begin_operation(Some(Duration::from_millis(100)))?;
            let result=await_operation(guard.deadline,async {
                guard.client.version_exchange_inner().await?;
                guard.client.finish_host_cleanup::<()>(Err(Mobilebackup2Error::Protocol("owned-private-primary-value".into())),async {
                    entered_tx.send(()).unwrap();
                    std::future::pending::<Result<(),Mobilebackup2Error>>().await
                }).await
            }).await;
            guard.finish_timed(result)
        };
        let remote=async {tests::complete_device_link_handshake(&mut peer).await;entered_rx.await.unwrap();};
        let (result,())=tokio::join!(run,remote);
        let error=result.unwrap_err();let retained=error.cleanup_failure().unwrap();
        assert!(matches!(retained.operation_error(),Mobilebackup2Error::Protocol(message) if message=="owned-private-primary-value"));
        timeout(retained.cleanup_error(),true);
        assert_eq!(crate::classify_error(&error),crate::ErrorClass::Protocol);
        assert!(!format!("{error:?} {retained:?} {error}").contains("owned-private"));
        rejected(&mut client,&root.0).await;
        drop(client);assert_eq!(peer.read(&mut [0]).await.unwrap(),0);
    }).await.unwrap();
}

#[tokio::test]
async fn lifecycle_backup2_default_cleanup_cap_retains_business_error_and_actual_elapsed() {
    tokio::time::timeout(Duration::from_secs(6),async {
        let root=Root::new("cleanup-default");let (mut client,mut peer,probe)=client();
        let remote=async {request(&mut peer).await;probe.blocked.store(true,Ordering::SeqCst);tests::complete_process_message(&mut peer,77,None).await;};
        let (result,())=tokio::join!(client.info(&root.0,"owned-device",None),remote);
        let error=result.unwrap_err();let retained=error.cleanup_failure().unwrap();
        assert!(matches!(retained.operation_error(),Mobilebackup2Error::Protocol(message) if message.contains("ErrorCode=77")));
        let Mobilebackup2Error::DeviceLink(DeviceLinkError::Io(cleanup))=retained.cleanup_error() else {panic!("DeviceLink cleanup timer")};
        assert_eq!(cleanup.kind(),ErrorKind::TimedOut);
        assert!(cleanup.get_ref().unwrap().is::<tokio::time::error::Elapsed>());
        assert_eq!(crate::classify_error(&error),crate::ErrorClass::Protocol);
        assert!(probe.blocked_writes.load(Ordering::SeqCst)>0);
        probe.blocked.store(false,Ordering::SeqCst);
        rejected(&mut client,&root.0).await;
        drop(client);assert_eq!(peer.read(&mut [0]).await.unwrap(),0);
    }).await.unwrap();
}
