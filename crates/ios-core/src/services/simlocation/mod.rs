//! Raw location simulation service client.
//!
//! Talks to `com.apple.dt.simulatelocation` over a lockdown-started service
//! stream. The protocol is intentionally tiny for this first slice:
//! - `set(lat, lon)` sends mode `0`, followed by two length-prefixed strings
//! - `reset()` sends mode `1` only

crate::redaction::error_debug!(SimLocationError);

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use quick_xml::de::from_str;
use serde::Deserialize;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::time::Instant;

pub const SERVICE_NAME: &str = "com.apple.dt.simulatelocation";

#[cfg(test)]
mod input_source_contract_tests;

#[derive(thiserror::Error)]
pub enum SimLocationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("GPX parse error: {0}")]
    GpxParse(String),
}

fn elapsed_budget() -> SimLocationError {
    SimLocationError::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "simlocation operation budget elapsed",
    ))
}

fn check_deadline<T>(
    deadline: Option<Instant>,
    result: Result<T, SimLocationError>,
) -> Result<T, SimLocationError> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        Err(elapsed_budget())
    } else {
        result
    }
}

async fn await_operation<T, F: Future<Output = Result<T, SimLocationError>>>(
    deadline: Option<Instant>,
    future: F,
) -> Result<T, SimLocationError> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, future)
            .await
            .map_err(|_| elapsed_budget())?,
        None => future.await,
    }
}

pub struct SimLocationClient<S> {
    stream: Option<S>,
    operation_timeout: Option<Duration>,
    usable: bool,
}

impl<S> SimLocationClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream: Some(stream),
            operation_timeout: None,
            usable: true,
        }
    }

    pub fn set_operation_timeout(&mut self, timeout: Option<Duration>) {
        self.operation_timeout = timeout;
    }

    pub fn is_usable(&self) -> bool {
        self.usable && self.stream.is_some()
    }

    fn ensure_usable(&self) -> Result<(), SimLocationError> {
        if !self.is_usable() {
            Err(SimLocationError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "simlocation connection is not usable",
            )))
        } else {
            Ok(())
        }
    }

    fn deadline(&self) -> Result<Option<Instant>, SimLocationError> {
        self.ensure_usable()?;
        match self.operation_timeout {
            Some(budget) if budget.is_zero() => Err(elapsed_budget()),
            Some(budget) => Instant::now().checked_add(budget).map(Some).ok_or_else(|| {
                SimLocationError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "simlocation operation deadline is too large",
                ))
            }),
            None => Ok(None),
        }
    }

    fn begin_request(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<SimLocationRequest<'_, S>, SimLocationError> {
        self.ensure_usable()?;
        let stream = self.stream.take();
        Ok(SimLocationRequest {
            client: self,
            stream,
            deadline,
            progressed: false,
            failed: false,
        })
    }
}

impl<S: AsyncWrite + Unpin> SimLocationClient<S> {
    /// Replay with the client's per-point write budget. Scheduled route delays
    /// are idle time, so they do not consume a short request budget. The held
    /// future is cancellable; a partially written point invalidates the client.
    pub async fn replay_gpx_route(&mut self, gpx: &str) -> Result<usize, SimLocationError> {
        let route = parse_gpx_route(gpx)?;
        for point in &route {
            if !point.delay_from_previous.is_zero() {
                tokio::time::sleep(point.delay_from_previous).await;
            }
            self.set_location(&point.latitude, &point.longitude).await?;
        }
        Ok(route.len())
    }

    pub async fn set_location(
        &mut self,
        latitude: &str,
        longitude: &str,
    ) -> Result<(), SimLocationError> {
        let deadline = self.deadline()?;
        check_deadline(deadline, Ok(()))?;
        let mut request = self.begin_request(deadline)?;
        let result = await_operation(deadline, async {
            let mut io = request.io();
            write_u32_le(&mut io, 0).await?;
            write_prefixed_string(&mut io, latitude).await?;
            write_prefixed_string(&mut io, longitude).await?;
            io.flush().await?;
            Ok(())
        })
        .await;
        request.finish(check_deadline(deadline, result))
    }

    pub async fn reset_location(&mut self) -> Result<(), SimLocationError> {
        let deadline = self.deadline()?;
        check_deadline(deadline, Ok(()))?;
        let mut request = self.begin_request(deadline)?;
        let result = await_operation(deadline, async {
            let mut io = request.io();
            write_u32_le(&mut io, 1).await?;
            io.flush().await?;
            Ok(())
        })
        .await;
        request.finish(check_deadline(deadline, result))
    }
}

struct SimLocationRequest<'a, S> {
    client: &'a mut SimLocationClient<S>,
    stream: Option<S>,
    deadline: Option<Instant>,
    progressed: bool,
    failed: bool,
}

impl<S> SimLocationRequest<'_, S> {
    fn io(&mut self) -> SimLocationIo<'_, S> {
        SimLocationIo {
            inner: self.stream.as_mut().expect("active simlocation stream"),
            deadline: self.deadline,
            progressed: &mut self.progressed,
            failed: &mut self.failed,
        }
    }

    fn finish<T>(mut self, result: Result<T, SimLocationError>) -> Result<T, SimLocationError> {
        if !self.failed && (result.is_ok() || !self.progressed) {
            self.client.stream = self.stream.take();
        } else {
            self.client.usable = false;
        }
        result
    }
}

impl<S> Drop for SimLocationRequest<'_, S> {
    fn drop(&mut self) {
        if self.stream.is_none() {
            return;
        }
        if self.progressed || self.failed {
            self.client.usable = false;
        } else {
            self.client.stream = self.stream.take();
        }
    }
}

struct SimLocationIo<'a, S> {
    inner: &'a mut S,
    deadline: Option<Instant>,
    progressed: &'a mut bool,
    failed: &'a mut bool,
}

impl<S> SimLocationIo<'_, S> {
    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for SimLocationIo<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let before = buf.filled().len();
        let requested = buf.remaining() > 0;
        let result = Pin::new(&mut *self.inner).poll_read(cx, buf);
        *self.progressed |= buf.filled().len() > before;
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (requested && matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() == before);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for SimLocationIo<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_write(cx, bytes);
        *self.progressed |= matches!(result, Poll::Ready(Ok(n)) if n > 0);
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (!bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))));
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_flush(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let result = Pin::new(&mut *self.inner).poll_shutdown(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpxRoutePoint {
    pub latitude: String,
    pub longitude: String,
    pub delay_from_previous: Duration,
}

/// Send a location simulation request with raw latitude/longitude strings.
pub async fn set_location<S>(
    stream: &mut S,
    latitude: &str,
    longitude: &str,
) -> Result<(), SimLocationError>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    write_u32_le(stream, 0).await?;
    write_prefixed_string(stream, latitude).await?;
    write_prefixed_string(stream, longitude).await?;
    stream.flush().await?;
    Ok(())
}

/// Reset the device back to its actual GPS location.
pub async fn reset_location<S>(stream: &mut S) -> Result<(), SimLocationError>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    write_u32_le(stream, 1).await?;
    stream.flush().await?;
    Ok(())
}

pub fn parse_gpx_route(gpx: &str) -> Result<Vec<GpxRoutePoint>, SimLocationError> {
    let parsed: Gpx = from_str(gpx).map_err(|error| input_error("parse GPX XML", error))?;
    let mut points = Vec::new();
    let mut previous_time: Option<String> = None;

    for track in parsed.tracks {
        for segment in track.segments {
            for point in segment.points {
                let delay_from_previous = match (&previous_time, point.time.as_deref()) {
                    (Some(previous), Some(current)) => parse_delay(previous, current)?,
                    _ => Duration::ZERO,
                };
                if let Some(current) = point.time.as_deref() {
                    previous_time = Some(current.to_string());
                }
                points.push(GpxRoutePoint {
                    latitude: point.latitude,
                    longitude: point.longitude,
                    delay_from_previous,
                });
            }
        }
    }

    Ok(points)
}

pub async fn replay_gpx_route<S>(stream: &mut S, gpx: &str) -> Result<usize, SimLocationError>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    let route = parse_gpx_route(gpx)?;
    for point in &route {
        if !point.delay_from_previous.is_zero() {
            tokio::time::sleep(point.delay_from_previous).await;
        }
        set_location(stream, &point.latitude, &point.longitude).await?;
    }
    Ok(route.len())
}

async fn write_prefixed_string<S>(stream: &mut S, value: &str) -> Result<(), std::io::Error>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    write_u32_le(stream, value.len() as u32).await?;
    stream.write_all(value.as_bytes()).await
}

async fn write_u32_le<S>(stream: &mut S, value: u32) -> Result<(), std::io::Error>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    stream.write_all(&value.to_le_bytes()).await
}

fn parse_delay(previous: &str, current: &str) -> Result<Duration, SimLocationError> {
    let previous = OffsetDateTime::parse(previous, &Rfc3339)
        .map_err(|error| input_error("parse previous GPX timestamp", error))?;
    let current = OffsetDateTime::parse(current, &Rfc3339)
        .map_err(|error| input_error("parse current GPX timestamp", error))?;
    let delta = current - previous;
    if delta.is_negative() {
        Ok(Duration::ZERO)
    } else {
        Ok(Duration::from_secs(delta.whole_seconds() as u64))
    }
}

fn input_error(
    phase: &'static str,
    source: impl std::error::Error + Send + Sync + 'static,
) -> SimLocationError {
    SimLocationError::Io(crate::error::context::retained_io(
        std::io::ErrorKind::InvalidInput,
        phase,
        source,
    ))
}

#[derive(Debug, Deserialize)]
struct Gpx {
    #[serde(rename = "trk", default)]
    tracks: Vec<GpxTrack>,
}

#[derive(Debug, Deserialize)]
struct GpxTrack {
    #[serde(rename = "trkseg", default)]
    segments: Vec<GpxSegment>,
}

#[derive(Debug, Deserialize)]
struct GpxSegment {
    #[serde(rename = "trkpt", default)]
    points: Vec<GpxPoint>,
}

#[derive(Debug, Deserialize)]
struct GpxPoint {
    #[serde(rename = "@lat")]
    latitude: String,
    #[serde(rename = "@lon")]
    longitude: String,
    #[serde(rename = "time")]
    time: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::AsyncWrite;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn client_gpx_idle_is_not_a_short_request_timeout_and_blocked_point_is_bounded() {
        let gpx = r#"<gpx><trk><trkseg>
            <trkpt lat="1" lon="2"><time>2026-10-26T00:00:00Z</time></trkpt>
            <trkpt lat="3" lon="4"><time>2026-10-26T00:00:02Z</time></trkpt>
            </trkseg></trk></gpx>"#;
        let mut client = SimLocationClient::new(Vec::<u8>::new());
        client.set_operation_timeout(Some(Duration::from_millis(100)));
        assert_eq!(client.replay_gpx_route(gpx).await.unwrap(), 2);
        assert!(client.is_usable());
        let (stream, _peer) = tokio::io::duplex(1);
        let mut client = SimLocationClient::new(stream);
        client.set_operation_timeout(Some(Duration::from_millis(100)));
        let started = Instant::now();
        assert!(client.replay_gpx_route(gpx).await.is_err());
        assert_eq!(started.elapsed(), Duration::from_millis(100));
        assert!(!client.is_usable());
    }

    struct MockWriter {
        bytes: Vec<u8>,
    }

    impl MockWriter {
        fn new() -> Self {
            Self { bytes: Vec::new() }
        }
    }

    impl AsyncWrite for MockWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.get_mut().bytes.extend_from_slice(buf);
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
    async fn encodes_set_payload_as_expected() {
        let mut buf = MockWriter::new();
        set_location(&mut buf, "48.856614", "2.3522219")
            .await
            .unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&(9u32).to_le_bytes());
        expected.extend_from_slice(b"48.856614");
        expected.extend_from_slice(&(9u32).to_le_bytes());
        expected.extend_from_slice(b"2.3522219");

        assert_eq!(buf.bytes, expected);
    }

    #[tokio::test]
    async fn encodes_reset_payload_as_expected() {
        let mut buf = MockWriter::new();
        reset_location(&mut buf).await.unwrap();

        assert_eq!(buf.bytes, 1u32.to_le_bytes());
    }

    #[tokio::test]
    async fn strings_are_written_without_extra_framing() {
        let mut buf = MockWriter::new();
        write_prefixed_string(&mut buf, "abc").await.unwrap();
        assert_eq!(buf.bytes, vec![3, 0, 0, 0, b'a', b'b', b'c']);
    }

    #[test]
    fn parses_gpx_route_and_preserves_timing_deltas() {
        let gpx = r#"
            <gpx>
              <trk>
                <trkseg>
                  <trkpt lat="48.856614" lon="2.3522219">
                    <time>2026-04-03T00:00:00Z</time>
                  </trkpt>
                  <trkpt lat="48.857000" lon="2.353000">
                    <time>2026-04-03T00:00:03Z</time>
                  </trkpt>
                </trkseg>
              </trk>
            </gpx>
        "#;

        let route = parse_gpx_route(gpx).unwrap();
        assert_eq!(route.len(), 2);
        assert_eq!(route[0].latitude, "48.856614");
        assert_eq!(route[0].longitude, "2.3522219");
        assert_eq!(route[0].delay_from_previous, Duration::ZERO);
        assert_eq!(route[1].delay_from_previous, Duration::from_secs(3));
    }

    #[tokio::test]
    async fn replay_gpx_route_sends_each_point_in_order() {
        let gpx = r#"
            <gpx>
              <trk>
                <trkseg>
                  <trkpt lat="48.856614" lon="2.3522219" />
                  <trkpt lat="48.857000" lon="2.353000" />
                </trkseg>
              </trk>
            </gpx>
        "#;

        let mut buf = MockWriter::new();
        let count = replay_gpx_route(&mut buf, gpx).await.unwrap();
        assert_eq!(count, 2);

        let mut expected = Vec::new();
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&(9u32).to_le_bytes());
        expected.extend_from_slice(b"48.856614");
        expected.extend_from_slice(&(9u32).to_le_bytes());
        expected.extend_from_slice(b"2.3522219");
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&(9u32).to_le_bytes());
        expected.extend_from_slice(b"48.857000");
        expected.extend_from_slice(&(8u32).to_le_bytes());
        expected.extend_from_slice(b"2.353000");

        assert_eq!(buf.bytes, expected);
    }

    #[tokio::test]
    async fn simlocation_zero_budget_fails_without_io() {
        let (client_io, _server_io) = tokio::io::duplex(64);
        let mut client = SimLocationClient::new(client_io);
        client.set_operation_timeout(Some(Duration::ZERO));

        let err = client.set_location("37.33", "-122.03").await.unwrap_err();
        assert!(
            matches!(err, SimLocationError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut)
        );
        assert!(client.is_usable());
    }

    #[tokio::test]
    async fn simlocation_partial_write_timeout_invalidates_client() {
        let (client_io, mut server_io) = tokio::io::duplex(2); // tiny buffer triggers backpressure
        let mut client = SimLocationClient::new(client_io);
        client.set_operation_timeout(Some(Duration::from_millis(40)));

        let server_task = tokio::spawn(async move {
            let mut buf = [0u8; 2];
            let _ = tokio::io::AsyncReadExt::read_exact(&mut server_io, &mut buf).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        });

        let err = client.set_location("37.33", "-122.03").await.unwrap_err();
        assert!(
            matches!(err, SimLocationError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut)
        );
        assert!(
            !client.is_usable(),
            "partial write progress must invalidate client"
        );

        let _ = server_task.await;
    }
}
