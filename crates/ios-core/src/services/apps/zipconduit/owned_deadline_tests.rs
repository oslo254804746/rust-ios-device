use super::*;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Ipa {
    dir: PathBuf,
    path: PathBuf,
}
impl Ipa {
    fn new() -> Self {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("ios-zip-deadline-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("Example.ipa");
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive
            .start_file(
                "Payload/Example.app/file",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(b"owned-payload").unwrap();
        std::fs::write(&path, archive.finish().unwrap().into_inner()).unwrap();
        Self { dir, path }
    }
}
impl Drop for Ipa {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn actual_progress_does_not_reset_owned_install_clock_and_original_socket_closes() {
    let ipa = Ipa::new();
    let (stream, mut peer) = tokio::io::duplex(8192);
    // Keep the paused clock at zero until the real async local-file read and
    // transfer finish, so the FS worker cannot make the timer advance early.
    let transferred = Arc::new(AtomicBool::new(false));
    let hold = transferred.clone();
    let clock_holder = tokio::spawn(async move {
        while !hold.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    });
    let remote = tokio::spawn(async move {
        let size = peer.read_u32().await.unwrap();
        assert!(size < 8192);
        let mut init = vec![0; size as usize];
        peer.read_exact(&mut init).await.unwrap();
        let init: plist::Dictionary = plist::from_bytes(&init).unwrap();
        assert_eq!(
            init["MediaSubdir"].as_string(),
            Some("PublicStaging/Example.ipa")
        );
        let mut archive = Vec::new();
        while !archive.ends_with(&[0x50, 0x4b, 0x01, 0x02]) {
            archive.push(peer.read_u8().await.unwrap());
            assert!(archive.len() < 8192);
        }
        transferred.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(60)).await;
        let progress = plist::Dictionary::from_iter([(
            "InstallProgressDict",
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("Status", plist::Value::String("Installing".into())),
                ("PercentComplete", plist::Value::Integer(40u64.into())),
            ])),
        )]);
        let mut bytes = Vec::new();
        plist::to_writer_binary(&mut bytes, &progress).unwrap();
        peer.write_u32(bytes.len() as u32).await.unwrap();
        peer.write_all(&bytes).await.unwrap();
        peer
    });
    let started = tokio::time::Instant::now();
    let error = install_ipa_owned_with_timeout(stream, &ipa.path, None, Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(started.elapsed(), Duration::from_millis(100));
    let ZipConduitError::Io(io) = &error else {
        panic!("native timeout lost")
    };
    assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
    assert!(io.get_ref().unwrap().is::<tokio::time::error::Elapsed>());
    clock_holder.await.unwrap();
    let mut peer = remote.await.unwrap();
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

#[tokio::test]
async fn zero_budget_and_caller_drop_close_the_single_use_service_without_replay() {
    let (stream, mut peer) = tokio::io::duplex(1);
    assert!(install_ipa_owned_with_timeout(
        stream,
        Path::new("must-not-read"),
        None,
        Duration::ZERO
    )
    .await
    .is_err());
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    let ipa = Ipa::new();
    let (stream, mut peer) = tokio::io::duplex(1);
    {
        let install =
            install_ipa_owned_with_timeout(stream, &ipa.path, None, Duration::from_secs(2));
        tokio::pin!(install);
        tokio::select! {result=&mut install=>panic!("partial install completed: {result:?}"),read=peer.read_u8()=>{read.unwrap();}}
    }
    let mut remaining = Vec::new();
    peer.read_to_end(&mut remaining).await.unwrap();
    assert!(remaining.len() <= 1);
}
