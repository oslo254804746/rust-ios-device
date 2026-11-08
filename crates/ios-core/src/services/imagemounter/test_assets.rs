//! Owned fixtures; never read ambient proxy/cache state or call Apple/DDI hosts.
use std::{io::Write, path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct Directory(pub(super) PathBuf);
impl Directory {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ios-image-native-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        for (name, body) in entries {
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
    }
    bytes
}

pub(super) async fn http<T, F, Fut>(
    response: Option<&[u8]>,
    hold: bool,
    operation: F,
) -> (T, Vec<u8>)
where
    F: FnOnce(reqwest::Client, String) -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/owned-private", listener.local_addr().unwrap());
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let peer = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            assert!(request.len() <= 64 * 1024);
        }
        if let Some(response) = response {
            stream.write_all(response).await.unwrap();
        }
        if !hold {
            stream.shutdown().await.unwrap();
        }
        let mut remaining = Vec::new();
        let _ = stream.read_to_end(&mut remaining).await;
        // The operation owns and drops its client, closing even a held response.
        request.extend(remaining);
        request
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(operation(client, url), peer)
    })
    .await
    .expect("owned HTTP peer must finish")
}
