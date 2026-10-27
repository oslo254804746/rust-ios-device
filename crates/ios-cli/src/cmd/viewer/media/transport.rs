//! Minimal single HEVC track MPEG-TS transport to preserve RTP PTS through
//! FFmpeg decoding (RFC7798 90kHz clock; ISO13818-1 PES/PSI as in FFmpeg mpegtsenc).
use anyhow::{ensure, Context, Result};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
};

pub(super) const PTS_OFFSET: i64 = 90_000;
const VIDEO_PID: u16 = 0x101;

#[derive(Default)]
pub(super) struct HevcTs {
    counters: [u8; 3],
    previous: Option<u32>,
    relative: i64,
    pcr: i64,
}
impl HevcTs {
    pub async fn write_access_unit<W: AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        data: &[u8],
        timestamp: u32,
    ) -> Result<()> {
        ensure!(
            !data.is_empty() && data.len() <= super::MAX_ENCODED_BYTES,
            "invalid timestamped HEVC access unit"
        );
        if let Some(previous) = self.previous {
            self.relative = self
                .relative
                .checked_add(i64::from(timestamp.wrapping_sub(previous) as i32))
                .context("RTP timestamp overflow")?;
        }
        self.previous = Some(timestamp);
        let pts = self
            .relative
            .checked_add(PTS_OFFSET)
            .filter(|pts| *pts >= 0)
            .context("RTP presentation precedes stream origin")?;
        self.pcr = self.pcr.max(pts);
        // Repeat PAT/PMT so a decoder can recover after a parameter refresh.
        self.section(
            writer,
            0,
            &[0, 0xb0, 13, 0, 1, 0xc1, 0, 0, 0, 1, 0xe1, 0],
            0,
        )
        .await?;
        self.section(
            writer,
            0x100,
            &[
                2, 0xb0, 18, 0, 1, 0xc1, 0, 0, 0xe1, 1, 0xf0, 0, 0x24, 0xe1, 1, 0xf0, 0,
            ],
            1,
        )
        .await?;
        let pts = (pts as u64) & ((1 << 33) - 1);
        let mut pes = vec![0, 0, 1, 0xe0, 0, 0, 0x80, 0x80, 5];
        pes.extend_from_slice(&[
            0x21 | (((pts >> 30) as u8 & 7) << 1),
            (pts >> 22) as u8,
            (((pts >> 15) as u8 & 0x7f) << 1) | 1,
            (pts >> 7) as u8,
            ((pts as u8 & 0x7f) << 1) | 1,
        ]);
        let mut bytes = pes.iter().chain(data.iter());
        let mut remaining = pes.len() + data.len();
        let mut first = true;
        while remaining > 0 {
            let count = remaining.min(if first { 176 } else { 184 });
            let adaptation = first || count < 184;
            let mut packet = [0xff; 188];
            packet[0] = 0x47;
            packet[1] = (VIDEO_PID >> 8) as u8 | if first { 0x40 } else { 0 };
            packet[2] = VIDEO_PID as u8;
            packet[3] = if adaptation { 0x30 } else { 0x10 } | self.counters[2];
            self.counters[2] = (self.counters[2] + 1) & 15;
            let offset = if adaptation {
                let length = 183 - count;
                packet[4] = length as u8;
                if length > 0 {
                    packet[5] = if first { 0x10 } else { 0 };
                }
                if first {
                    let pcr = (self.pcr as u64) & ((1 << 33) - 1);
                    packet[6..12].copy_from_slice(&[
                        (pcr >> 25) as u8,
                        (pcr >> 17) as u8,
                        (pcr >> 9) as u8,
                        (pcr >> 1) as u8,
                        ((pcr as u8 & 1) << 7) | 0x7e,
                        0,
                    ]);
                }
                5 + length
            } else {
                4
            };
            for destination in &mut packet[offset..offset + count] {
                *destination = *bytes.next().expect("counted PES bytes");
            }
            writer.write_all(&packet).await?;
            remaining -= count;
            first = false;
        }
        writer.flush().await?;
        Ok(())
    }
    async fn section<W: AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        pid: u16,
        table: &[u8],
        counter: usize,
    ) -> Result<()> {
        let mut packet = [0xff; 188];
        packet[0] = 0x47;
        packet[1] = 0x40 | (pid >> 8) as u8;
        packet[2] = pid as u8;
        packet[3] = 0x10 | self.counters[counter];
        self.counters[counter] = (self.counters[counter] + 1) & 15;
        packet[4] = 0;
        packet[5..5 + table.len()].copy_from_slice(table);
        packet[5 + table.len()..9 + table.len()].copy_from_slice(&mpeg_crc(table).to_be_bytes());
        writer.write_all(&packet).await?;
        Ok(())
    }
}
fn mpeg_crc(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x80000000 != 0 {
                (crc << 1) ^ 0x04c11db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

pub(super) fn spawn_decoder() -> Result<tokio::process::Child> {
    use std::process::Stdio;
    tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "info",
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
            "-copyts",
            "-f",
            "mpegts",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-threads",
            "1",
            "-vf",
            "format=bgra,settb=expr=1/90000,showinfo",
            "-fps_mode",
            "passthrough",
            "-noautoscale",
            "-f",
            "rawvideo",
            "-vcodec",
            "rawvideo",
            "-flush_packets",
            "1",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("start timestamped HEVC decoder")
}

/// Consume bounded diagnostic lines alongside raw pixels; showinfo reports the
/// decoded frame's PTS, so B-frame reordering does not require a guessed FIFO.
pub(super) async fn read_frame<S: AsyncRead + Unpin, R: AsyncBufRead + Unpin>(
    stdout: &mut S,
    stderr: &mut R,
) -> Result<super::DecodedFrame> {
    loop {
        let mut line = Vec::new();
        let count = (&mut *stderr)
            .take(8193)
            .read_until(b'\n', &mut line)
            .await?;
        ensure!(count > 0, "FFmpeg ended before reporting presentation time");
        ensure!(count <= 8192, "FFmpeg diagnostic line exceeds limit");
        let line = std::str::from_utf8(&line).context("FFmpeg diagnostic encoding")?;
        if !line.contains("showinfo") || !line.contains(" n:") {
            continue;
        }
        let pts = line
            .split(" pts:")
            .nth(1)
            .and_then(|text| text.split_whitespace().next())
            .context("FFmpeg frame omitted PTS")?
            .parse::<i64>()
            .context("FFmpeg frame PTS")?;
        let relative = pts
            .checked_sub(PTS_OFFSET)
            .filter(|pts| *pts >= 0)
            .context("invalid decoded presentation time")?;
        let timestamp = (relative as u64)
            .checked_mul(1000)
            .context("presentation timestamp overflow")?
            / 90_000;
        let (width, height) = line
            .split(" s:")
            .nth(1)
            .and_then(|text| text.split_whitespace().next())
            .and_then(|size| size.split_once('x'))
            .context("FFmpeg frame omitted dimensions")?;
        let width = width.parse::<u16>().context("FFmpeg frame width")?;
        let height = height.parse::<u16>().context("FFmpeg frame height")?;
        let size = usize::from(width) * usize::from(height) * 4;
        ensure!(
            size > 0 && size <= super::MAX_PIXEL_BYTES,
            "decoded frame exceeds pixel budget"
        );
        let mut pixels = vec![0; size];
        stdout
            .read_exact(&mut pixels)
            .await
            .context("read FFmpeg BGRA frame")?;
        // PNG's encoder context cannot change dimensions midstream. Encode in
        // Rust from each actual rawvideo frame instead; no reference decoder
        // reset and no second full-size pixel allocation are needed.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let mut png = LimitedPng(Vec::new());
        {
            let mut encoder = png::Encoder::new(&mut png, width.into(), height.into());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header()?.write_image_data(&pixels)?;
        }
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        return Ok(super::DecodedFrame {
            png: png.0.into(),
            bgra: std::sync::Arc::new(pixels),
            width,
            height,
            timestamp_ms: timestamp,
            timestamp_clock: "rtp-relative-ms",
        });
    }
}

struct LimitedPng(Vec<u8>);
impl std::io::Write for LimitedPng {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > super::MAX_ENCODED_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("encoded frame exceeds PNG budget"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_crc_vector() {
        assert_eq!(mpeg_crc(b"123456789"), 0x0376e6e7);
    }

    fn access_units(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut units = Vec::new();
        let mut current = Vec::new();
        for (kind, nal) in super::super::parse_hevc_annex_b_nals(bytes) {
            if kind == super::super::HevcNalType::AccessUnitDelimiter && !current.is_empty() {
                units.push(std::mem::take(&mut current));
            }
            current.extend_from_slice(&[0, 0, 0, 1]);
            current.extend_from_slice(nal);
        }
        if !current.is_empty() {
            units.push(current);
        }
        units
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg; real RTP PTS wrap, resolution refresh and live pixels gate"]
    async fn timestamped_decoder_preserves_rtp_intervals_wrap_and_resolution_changes() {
        let mut units = access_units(include_bytes!("../hevc_timed_sample.bin"));
        assert_eq!(units.len(), 6);
        units.extend(access_units(include_bytes!("../hevc_resized_sample.bin")));
        assert_eq!(units.len(), 9);
        let times = [
            0u32, 18_000, 45_000, 72_000, 90_000, 117_000, 144_000, 162_000, 198_000,
        ];
        let mut child = spawn_decoder().expect("explicit FFmpeg gate requires decoder");
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = tokio::io::BufReader::new(child.stderr.take().unwrap());
        let (published, web) = tokio::sync::watch::channel(None);
        let vnc = web.clone();
        let (first, saw_first) = tokio::sync::oneshot::channel();
        let ((), frames) =
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                tokio::join!(
                    async {
                        let mut mux = HevcTs::default();
                        for (unit, ticks) in units.iter().zip(times) {
                            mux.write_access_unit(
                                &mut stdin,
                                unit,
                                (u32::MAX - 9000).wrapping_add(ticks),
                            )
                            .await
                            .unwrap();
                        }
                        saw_first.await.unwrap();
                        drop(stdin);
                    },
                    async {
                        let mut frames = Vec::new();
                        let mut first_pixels = None;
                        let mut first = Some(first);
                        for frame_index in 0..9 {
                            let frame = read_frame(&mut stdout, &mut stderr).await.unwrap_or_else(
                                |error| panic!("decoded frame {frame_index}: {error:#}"),
                            );
                            if frame_index == 0 {
                                first_pixels = Some(std::sync::Arc::downgrade(&frame.bgra));
                            }
                            frames.push((
                                frame.timestamp_ms,
                                frame.width,
                                frame.height,
                                frame.bgra.as_slice().to_vec(),
                            ));
                            super::super::publish_frame(&published, frame);
                            if frame_index > 0 {
                                assert!(
                                    first_pixels.as_ref().unwrap().upgrade().is_none(),
                                    "production slow consumers must release old pixels"
                                );
                            }
                            if let Some(first) = first.take() {
                                first.send(()).unwrap();
                            }
                        }
                        frames
                    }
                )
            })
            .await
            .unwrap();
        assert!(child.wait().await.unwrap().success());
        assert_eq!(
            frames.iter().map(|f| f.0).collect::<Vec<_>>(),
            times
                .iter()
                .map(|t| u64::from(*t) * 1000 / 90_000)
                .collect::<Vec<_>>()
        );
        assert!(frames[..6].iter().all(|f| (f.1, f.2) == (32, 32)));
        assert!(frames[6..].iter().all(|f| (f.1, f.2) == (48, 32)));
        assert_ne!(frames[0].3, frames[5].3);
        let web = web.borrow();
        let vnc = vnc.borrow();
        assert!(std::sync::Arc::ptr_eq(
            &web.as_ref().unwrap().bgra,
            &vnc.as_ref().unwrap().bgra
        ));
        assert_eq!(web.as_ref().unwrap().timestamp_clock, "rtp-relative-ms");
        let last = web.as_ref().unwrap().clone();
        drop((web, vnc));
        super::super::super::tests::verify_decoded_video_endpoints(last).await;
    }
}
