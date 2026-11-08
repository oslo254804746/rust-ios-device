//! Shared monotonic WebInspector operation deadlines.

use super::WebInspectorError;
use tokio::time::{Duration, Instant};

pub(super) fn remaining_time(
    deadline: Instant,
    fallback: Duration,
) -> Result<Duration, WebInspectorError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(WebInspectorError::Timeout(fallback));
    }
    Ok(deadline.duration_since(now))
}

pub(super) fn deadline_after(timeout_duration: Duration) -> Result<Instant, WebInspectorError> {
    Instant::now().checked_add(timeout_duration).ok_or_else(|| {
        WebInspectorError::Protocol("webinspector timeout is too large for the system clock".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_time_errors_after_deadline() {
        let fallback = Duration::from_millis(25);
        let err = remaining_time(Instant::now(), fallback)
            .expect_err("expired deadlines must become timeout errors");
        assert!(matches!(err, WebInspectorError::Timeout(duration) if duration == fallback));
    }

    #[test]
    fn deadline_after_rejects_clock_overflow() {
        let err = deadline_after(Duration::from_secs(u64::MAX))
            .expect_err("an unrepresentable deadline must not panic");
        assert!(err.to_string().contains("timeout is too large"));
    }
}
