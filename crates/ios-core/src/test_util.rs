use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Debug, Default)]
pub(crate) struct MockStream {
    pub(crate) read_buf: Vec<u8>,
    pub(crate) written: Vec<u8>,
    pub(crate) read_pos: usize,
    eof_returns_ok: bool,
}

#[allow(dead_code)]
impl MockStream {
    pub(crate) fn new(read_buf: Vec<u8>) -> Self {
        Self {
            read_buf,
            written: Vec::new(),
            read_pos: 0,
            eof_returns_ok: false,
        }
    }

    pub(crate) fn eof() -> Self {
        Self::new(Vec::new())
    }

    pub(crate) fn eof_returns_ok(mut self) -> Self {
        self.eof_returns_ok = true;
        self
    }

    pub(crate) fn with_plist_response(value: plist::Value) -> Self {
        Self::with_plist_responses(vec![value])
    }

    pub(crate) fn with_response(value: plist::Value) -> Self {
        Self::with_plist_response(value)
    }

    pub(crate) fn with_plist_responses(values: Vec<plist::Value>) -> Self {
        let mut read_buf = Vec::new();
        for value in values {
            Self::push_plist_frame(&mut read_buf, value);
        }
        Self::new(read_buf)
    }

    pub(crate) fn with_responses(values: Vec<plist::Value>) -> Self {
        Self::with_plist_responses(values)
    }

    pub(crate) fn with_raw_frames(frames: Vec<Vec<u8>>) -> Self {
        let mut read_buf = Vec::new();
        for frame in frames {
            read_buf.extend_from_slice(&(frame.len() as u32).to_be_bytes());
            read_buf.extend_from_slice(&frame);
        }
        Self::new(read_buf)
    }

    pub(crate) fn with_frames(frames: Vec<Vec<u8>>) -> Self {
        Self::with_raw_frames(frames)
    }

    pub(crate) fn with_plist_response_and_trailing_bytes(
        value: plist::Value,
        trailing: &[u8],
    ) -> Self {
        let mut read_buf = Vec::new();
        Self::push_plist_frame(&mut read_buf, value);
        read_buf.extend_from_slice(trailing);
        Self::new(read_buf).eof_returns_ok()
    }

    pub(crate) fn with_prefixed_plist_response(prefix: &[u8], value: plist::Value) -> Self {
        let mut read_buf = Vec::new();
        read_buf.extend_from_slice(prefix);
        Self::push_plist_frame(&mut read_buf, value);
        Self::new(read_buf)
    }

    pub(crate) fn with_packet_data(data: Vec<u8>) -> Self {
        Self::with_plist_response(plist::Value::Data(data))
    }

    pub(crate) fn plist_frame(value: plist::Value) -> Vec<u8> {
        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, &value).unwrap();
        payload
    }

    pub(crate) fn push_plist_frame(read_buf: &mut Vec<u8>, value: plist::Value) {
        let payload = Self::plist_frame(value);
        read_buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        read_buf.extend_from_slice(&payload);
    }
}

impl AsyncRead for MockStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let remaining = self.read_buf.len().saturating_sub(self.read_pos);
        if remaining == 0 {
            return if self.eof_returns_ok {
                Poll::Ready(Ok(()))
            } else {
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "no more test data",
                )))
            };
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
    ) -> Poll<io::Result<usize>> {
        self.written.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn reads_preloaded_bytes_and_records_writes() {
        let mut stream = MockStream::new(vec![1, 2, 3]);
        let mut buf = [0; 3];

        stream.read_exact(&mut buf).await.unwrap();
        stream.write_all(&[4, 5]).await.unwrap();

        assert_eq!(buf, [1, 2, 3]);
        assert_eq!(stream.written, [4, 5]);
    }

    #[tokio::test]
    async fn encodes_plist_response_with_length_prefix() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_plist_response(value);

        let mut len_buf = [0; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0; len];
        stream.read_exact(&mut payload).await.unwrap();
        let decoded: plist::Dictionary = plist::from_bytes(&payload).unwrap();

        assert_eq!(decoded["Status"].as_string(), Some("Acknowledged"));
    }

    #[tokio::test]
    async fn defaults_to_unexpected_eof_after_preloaded_bytes() {
        let mut stream = MockStream::new(Vec::new());
        let mut buf = [0; 1];

        let err = stream.read_exact(&mut buf).await.unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn can_return_ok_at_eof_for_raw_archive_tests() {
        let mut stream = MockStream::new(Vec::new()).eof_returns_ok();
        let mut buf = [0; 1];

        let read = stream.read(&mut buf).await.unwrap();

        assert_eq!(read, 0);
    }
}
