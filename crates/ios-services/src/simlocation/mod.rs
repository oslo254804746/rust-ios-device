//! Raw location simulation service client.
//!
//! Talks to `com.apple.dt.simulatelocation` over a lockdown-started service
//! stream. The protocol is intentionally tiny for this first slice:
//! - `set(lat, lon)` sends mode `0`, followed by two length-prefixed strings
//! - `reset()` sends mode `1` only

use std::time::Duration;

use quick_xml::de::from_str;
use serde::Deserialize;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.dt.simulatelocation";

#[derive(Debug, thiserror::Error)]
pub enum SimLocationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("GPX parse error: {0}")]
    GpxParse(String),
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
    let parsed: Gpx = from_str(gpx).map_err(|e| SimLocationError::GpxParse(e.to_string()))?;
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
        .map_err(|e| SimLocationError::GpxParse(e.to_string()))?;
    let current = OffsetDateTime::parse(current, &Rfc3339)
        .map_err(|e| SimLocationError::GpxParse(e.to_string()))?;
    let delta = current - previous;
    if delta.is_negative() {
        Ok(Duration::ZERO)
    } else {
        Ok(Duration::from_secs(delta.whole_seconds() as u64))
    }
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
}
