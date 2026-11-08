//! Unit-test-only, bounded raw tracing capture. Never persists captured fields.

use std::fmt::{self, Write};
use std::sync::{Arc, Mutex};

const MAX_BYTES: usize = 256 * 1024;

#[derive(Default)]
pub(crate) struct Log {
    pub(crate) raw: String,
    pub(crate) overflow: bool,
}

impl fmt::Write for Log {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let available = MAX_BYTES - self.raw.len();
        let mut end = available.min(value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.raw.push_str(&value[..end]);
        if end != value.len() {
            self.overflow = true;
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

impl tracing::field::Visit for Log {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        let _ = write!(self, "{}={value:?};", field.name());
    }
}

struct Collector(Arc<Mutex<Log>>);

impl tracing::Subscriber for Collector {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::always()
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::TRACE)
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        values.record(&mut *self.0.lock().unwrap());
    }
    fn event(&self, event: &tracing::Event<'_>) {
        event.record(&mut *self.0.lock().unwrap());
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
}

pub(crate) fn capture() -> (tracing::Dispatch, Arc<Mutex<Log>>) {
    let log = Arc::new(Mutex::new(Log::default()));
    (tracing::Dispatch::new(Collector(log.clone())), log)
}

#[test]
fn raw_capture_detects_secret_fixture_before_any_masking_and_bounds_utf8() {
    let secret = "synthetic-private-log-fixture";
    let (dispatch, log) = capture();
    tracing::dispatcher::with_default(&dispatch, || tracing::error!(secret, "positive control"));
    assert!(log.lock().unwrap().raw.contains(secret));
    let mut log = Log::default();
    assert!(log.write_str(&"秘密".repeat(MAX_BYTES)).is_err());
    assert!(log.overflow);
    assert!(log.raw.len() <= MAX_BYTES);
    assert!(log.write_str("further content").is_err());
}
