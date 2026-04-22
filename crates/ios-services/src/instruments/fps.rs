use serde::Serialize;

const NANOS_PER_SECOND: f64 = 1_000_000_000.0;
const NANOS_PER_MILLI: f64 = 1_000_000.0;
const PENDING_FENCE_TIMESTAMP: i64 = i64::MAX;
const MOVIE_FRAME_COST_NS: f64 = NANOS_PER_SECOND / 24.0;
const TWO_FRAME_THRESHOLD_NS: f64 = MOVIE_FRAME_COST_NS * 2.0;
const THREE_FRAME_THRESHOLD_NS: f64 = MOVIE_FRAME_COST_NS * 3.0;
const MIN_FRAME_DURATION_NS: i64 = 4_000_000;
const KPERF_RECORD_SIZE: usize = 64;
const KDBG_CLASS_MASK: u32 = 0xFF00_0000;
const KDBG_SUBCLASS_MASK: u32 = 0x00FF_0000;
const KDBG_CODE_MASK: u32 = 0x0000_FFFC;
const KDBG_CLASS_OFFSET: u32 = 24;
const KDBG_SUBCLASS_OFFSET: u32 = 16;
const KDBG_CODE_OFFSET: u32 = 2;
const FRAME_COMMIT_CLASS: u32 = 0x31;
const FRAME_COMMIT_SUBCLASS: u32 = 0x80;
const FRAME_COMMIT_CODE: u32 = 0xC6;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FpsSample {
    pub fps: f64,
    pub jank: u32,
    pub big_jank: u32,
    pub stutter: f64,
    pub frame_count: u32,
    pub window_ms: f64,
}

#[derive(Debug, Clone)]
pub struct MachTimeInfo {
    pub numer: u64,
    pub denom: u64,
}

#[derive(Debug, Default)]
pub struct FpsWindowCalculator {
    last_frame_vsync_time: Option<i64>,
    last_three_frame_times: Vec<i64>,
}

impl FpsWindowCalculator {
    pub fn new() -> Self {
        Self {
            last_frame_vsync_time: None,
            last_three_frame_times: Vec::with_capacity(3),
        }
    }

    pub fn push_timestamps(&mut self, timestamps_ns: &[i64]) -> Option<FpsSample> {
        if timestamps_ns.is_empty() {
            return None;
        }

        let mut duration_ns = 0i64;
        let mut frame_count = 0u32;
        let mut jank = 0u32;
        let mut big_jank = 0u32;
        let mut jank_time_ns = 0i64;

        for &timestamp in timestamps_ns {
            if timestamp == PENDING_FENCE_TIMESTAMP {
                continue;
            }
            if self
                .last_frame_vsync_time
                .is_some_and(|last_timestamp| timestamp <= last_timestamp)
            {
                continue;
            }

            let Some(last_frame_vsync_time) = self.last_frame_vsync_time else {
                self.last_frame_vsync_time = Some(timestamp);
                continue;
            };

            let frame_cost = timestamp - last_frame_vsync_time;
            if frame_cost < MIN_FRAME_DURATION_NS {
                continue;
            }

            duration_ns += frame_cost;
            frame_count += 1;

            if self.last_three_frame_times.len() > 2 {
                let last_frame_avg = self.last_three_frame_times.iter().copied().sum::<i64>()
                    / self.last_three_frame_times.len() as i64;

                if frame_cost > last_frame_avg * 2 {
                    if (frame_cost as f64) > THREE_FRAME_THRESHOLD_NS {
                        big_jank += 1;
                        jank += 1;
                        jank_time_ns += frame_cost;
                    } else if (frame_cost as f64) > TWO_FRAME_THRESHOLD_NS {
                        jank += 1;
                        jank_time_ns += frame_cost;
                    }
                }
            }

            self.last_three_frame_times.push(frame_cost);
            if self.last_three_frame_times.len() > 3 {
                self.last_three_frame_times.remove(0);
            }
            self.last_frame_vsync_time = Some(timestamp);
        }

        if frame_count == 0 || duration_ns <= 0 {
            return Some(FpsSample {
                fps: 0.0,
                jank,
                big_jank,
                stutter: 0.0,
                frame_count,
                window_ms: round_to(duration_ns as f64 / NANOS_PER_MILLI, 2),
            });
        }

        Some(FpsSample {
            fps: round_to(
                frame_count as f64 / (duration_ns as f64 / NANOS_PER_SECOND),
                1,
            ),
            jank,
            big_jank,
            stutter: round_to(jank_time_ns as f64 / duration_ns as f64, 2),
            frame_count,
            window_ms: round_to(duration_ns as f64 / NANOS_PER_MILLI, 2),
        })
    }
}

pub fn parse_frame_commit_timestamps(chunk: &[u8], time_info: &MachTimeInfo) -> Vec<i64> {
    if time_info.denom == 0 {
        return Vec::new();
    }

    chunk
        .chunks_exact(KPERF_RECORD_SIZE)
        .filter_map(|record| {
            let mach_time = u64::from_le_bytes(record[0..8].try_into().ok()?);
            let debug_id = u32::from_le_bytes(record[48..52].try_into().ok()?);
            if !is_frame_commit_event(debug_id) {
                return None;
            }
            Some(((mach_time as f64) * (time_info.numer as f64) / (time_info.denom as f64)) as i64)
        })
        .collect()
}

fn is_frame_commit_event(debug_id: u32) -> bool {
    ((debug_id & KDBG_CLASS_MASK) >> KDBG_CLASS_OFFSET) == FRAME_COMMIT_CLASS
        && ((debug_id & KDBG_SUBCLASS_MASK) >> KDBG_SUBCLASS_OFFSET) == FRAME_COMMIT_SUBCLASS
        && ((debug_id & KDBG_CODE_MASK) >> KDBG_CODE_OFFSET) == FRAME_COMMIT_CODE
}

fn round_to(value: f64, precision: i32) -> f64 {
    let ratio = 10f64.powi(precision);
    (value * ratio).round() / ratio
}

#[cfg(test)]
mod tests {
    use super::*;

    const KPERF_RECORD_SIZE: usize = 64;
    const FRAME_COMMIT_DEBUG_ID: u32 = 0x31800318;

    fn build_record(mach_time: u64, debug_id: u32) -> Vec<u8> {
        let mut record = vec![0u8; KPERF_RECORD_SIZE];
        record[0..8].copy_from_slice(&mach_time.to_le_bytes());
        record[48..52].copy_from_slice(&debug_id.to_le_bytes());
        record
    }

    #[test]
    fn parses_frame_commit_records_into_nanosecond_timestamps() {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&build_record(1_000, FRAME_COMMIT_DEBUG_ID));
        chunk.extend_from_slice(&build_record(1_500, 0x1234_5678));
        chunk.extend_from_slice(&build_record(2_000, FRAME_COMMIT_DEBUG_ID));

        let timestamps = parse_frame_commit_timestamps(
            &chunk,
            &MachTimeInfo {
                numer: 125,
                denom: 3,
            },
        );

        assert_eq!(timestamps, vec![41_666, 83_333]);
    }

    #[test]
    fn calculates_fps_jank_and_stutter_from_frame_timestamps() {
        let mut calculator = FpsWindowCalculator::new();
        let sample = calculator
            .push_timestamps(&[
                0,
                16_000_000,
                32_000_000,
                48_000_000,
                64_000_000,
                180_000_000,
            ])
            .expect("sample should be emitted");

        assert_eq!(sample.frame_count, 5);
        assert_eq!(sample.window_ms, 180.0);
        assert_eq!(sample.jank, 1);
        assert_eq!(sample.big_jank, 0);
        assert!(
            (sample.fps - 27.8).abs() < 0.1,
            "unexpected fps: {}",
            sample.fps
        );
        assert!(sample.stutter > 0.60 && sample.stutter < 0.70);
    }
}
