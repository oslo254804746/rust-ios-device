use std::pin::Pin;

use openssl::error::ErrorStack;
use openssl::ssl::{Error, Ssl, SslContext, SslMethod, SslVerifyMode, SslVersion};
use tokio::net::TcpStream;
use tokio_openssl::SslStream;

#[derive(Debug, thiserror::Error)]
pub enum PskTlsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("OpenSSL error: {0}")]
    Openssl(#[from] ErrorStack),
    #[error("OpenSSL handshake error: {0}")]
    OpensslHandshake(#[from] Error),
}

pub async fn connect_psk_tls_stream<S>(
    host: &str,
    stream: S,
    psk: &[u8],
) -> Result<SslStream<S>, PskTlsError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let context = build_psk_client_context(psk)?;
    let ssl = Ssl::new(&context)?;
    let mut stream = SslStream::new(ssl, stream)?;
    let _ = host;
    Pin::new(&mut stream).connect().await?;
    Ok(stream)
}

pub async fn connect_psk_tls(
    host: &str,
    port: u16,
    psk: &[u8],
) -> Result<SslStream<TcpStream>, PskTlsError> {
    let tcp = TcpStream::connect((host, port)).await?;
    connect_psk_tls_stream(host, tcp, psk).await
}

fn build_psk_client_context(psk: &[u8]) -> Result<SslContext, ErrorStack> {
    let mut builder = SslContext::builder(SslMethod::tls_client())?;
    builder.set_verify(SslVerifyMode::NONE);
    builder.set_min_proto_version(Some(SslVersion::TLS1_2))?;
    builder.set_max_proto_version(Some(SslVersion::TLS1_2))?;
    builder.set_cipher_list("PSK")?;

    let psk = psk.to_vec();
    builder.set_psk_client_callback(move |_ssl, _hint, identity, psk_buf| {
        if psk.len() > psk_buf.len() || identity.is_empty() {
            return Err(ErrorStack::get());
        }

        // Mirror pymobiledevice3's `set_psk_client_callback(lambda hint: (None, key))`
        // by sending an empty PSK identity.
        identity[0] = 0;
        psk_buf[..psk.len()].copy_from_slice(&psk);
        Ok(psk.len())
    });

    Ok(builder.build())
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    async fn run_psk_server(psk: Vec<u8>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();

            let mut builder = SslContext::builder(SslMethod::tls_server()).unwrap();
            builder.set_verify(SslVerifyMode::NONE);
            builder
                .set_min_proto_version(Some(SslVersion::TLS1_2))
                .unwrap();
            builder
                .set_max_proto_version(Some(SslVersion::TLS1_2))
                .unwrap();
            builder.set_cipher_list("PSK").unwrap();

            builder.set_psk_server_callback(move |_ssl, identity, psk_buf| {
                if identity != Some(&[][..]) || psk.len() > psk_buf.len() {
                    return Err(ErrorStack::get());
                }
                psk_buf[..psk.len()].copy_from_slice(&psk);
                Ok(psk.len())
            });

            let ssl = Ssl::new(&builder.build()).unwrap();
            let mut stream = SslStream::new(ssl, socket).unwrap();
            if Pin::new(&mut stream).accept().await.is_err() {
                return;
            }

            let mut request = [0u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
            Pin::new(&mut stream).shutdown().await.unwrap();
        });

        port
    }

    #[tokio::test]
    async fn connect_psk_tls_exchanges_application_data() {
        let psk = vec![0x42; 32];
        let port = run_psk_server(psk.clone()).await;

        let mut stream = connect_psk_tls("127.0.0.1", port, &psk).await.unwrap();
        stream.write_all(b"ping").await.unwrap();

        let mut response = [0u8; 4];
        stream.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
    }

    #[tokio::test]
    async fn connect_psk_tls_rejects_wrong_key() {
        let port = run_psk_server(vec![0x24; 32]).await;
        let err = connect_psk_tls("127.0.0.1", port, &[0x42; 32])
            .await
            .expect_err("mismatched PSK should fail");

        let rendered = err.to_string();
        assert!(
            rendered.contains("OpenSSL handshake error")
                || rendered.contains("OpenSSL error")
                || rendered.contains("IO error"),
            "unexpected error: {rendered}"
        );
    }
}
