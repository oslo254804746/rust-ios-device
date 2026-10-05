//! Local retained-event policy, independent of the 16 MiB wire-frame ceiling.
//! Counts compact JSON bytes without allocating an encoded copy; this is not
//! an allocator/RSS or peak-memory measurement.

use super::{local_limit, WebInspectorError, WebInspectorEvent};
use std::io::{self, Write};

pub(super) const MAX_EVENTS: usize = 1024;
pub(super) const MAX_EVENT_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn event_size(
    event: &WebInspectorEvent,
    remaining: usize,
) -> Result<usize, WebInspectorError> {
    struct Counter {
        bytes: usize,
        remaining: usize,
        exceeded: bool,
    }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.remaining {
                self.exceeded = true;
                return Err(io::Error::other("local retained-event limit"));
            }
            self.remaining -= bytes.len();
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter {
        bytes: 0,
        remaining,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut counter, event);
    if counter.exceeded {
        return Err(local_limit("retained event bytes", MAX_EVENT_BYTES as u64));
    }
    result?;
    Ok(counter.bytes)
}

pub(super) fn admit_id(next: Option<u64>) -> Result<u64, WebInspectorError> {
    next.ok_or_else(|| local_limit("command identifiers", u64::MAX))
}
