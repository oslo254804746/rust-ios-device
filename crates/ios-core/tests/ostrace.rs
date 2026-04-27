use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct MockStream {
    read_buf: Vec<u8>,
    written: Vec<u8>,
    read_pos: usize,
}

impl MockStream {
    fn with_response(value: plist::Value) -> Self {
        let mut plist_payload = Vec::new();
        plist::to_writer_xml(&mut plist_payload, &value).unwrap();

        let mut read_buf = Vec::new();
        read_buf.push(1);
        read_buf.extend_from_slice(&(plist_payload.len() as u32).to_be_bytes());
        read_buf.extend_from_slice(&plist_payload);

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
async fn get_pid_list_sends_pid_list_request_and_parses_payload() {
    let mut stream =
        MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Payload".to_string(),
            plist::Value::Array(vec![plist::Value::Dictionary(
                plist::Dictionary::from_iter([
                    ("PID".to_string(), plist::Value::Integer(42.into())),
                    (
                        "Name".to_string(),
                        plist::Value::String("SpringBoard".into()),
                    ),
                ]),
            )]),
        )])));
    let mut client = ios_core::ostrace::OsTraceClient::new(&mut stream);

    let response = client.get_pid_list().await.unwrap();
    let payload = response
        .get("Payload")
        .and_then(plist::Value::as_array)
        .expect("payload array");
    assert_eq!(payload.len(), 1);

    let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
    let request: plist::Dictionary = plist::from_bytes(&stream.written[4..4 + len]).unwrap();
    assert_eq!(request["Request"].as_string(), Some("PidList"));
}
