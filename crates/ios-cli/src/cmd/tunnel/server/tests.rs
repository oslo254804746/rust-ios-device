use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Debug)]
struct Probe(Arc<AtomicUsize>);
impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private-terminal-HTTP-IO")
    }
}
impl Error for Probe {}
impl Drop for Probe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn terminal_inline_phase_transfer_keeps_original_http_io_over_native_and_outer_expiry() {
    let registry = Registry::new().unwrap();
    let native = registry
        .drain_retired(CleanupClock::new(Duration::ZERO).unwrap())
        .await
        .unwrap_err();
    assert!(native.primary_is_elapsed());
    let elapsed = tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
        .await
        .unwrap_err();
    let drops = Arc::new(AtomicUsize::new(0));
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        Probe(drops.clone()),
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let phases = StdMutex::new(CleanupPhases {
        native: Some(native),
        metadata_http: Some(original),
        native_complete: true,
        native_selected: 0,
        metadata_http_complete: true,
    });
    let acquired = phases.into_inner().unwrap_or_else(|p| p.into_inner());
    let terminal = TerminalCleanupFailure {
        native: acquired.native,
        history: None,
        scan_history: None,
        metadata_http: acquired.metadata_http,
        elapsed: Some(elapsed),
        snapshot: registry.snapshot(),
        native_complete: acquired.native_complete,
        metadata_http_complete: acquired.metadata_http_complete,
    };
    assert_eq!(
        terminal.primary().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert!(terminal.native.as_ref().unwrap().primary_is_elapsed());
    assert!(terminal.elapsed.is_some());
    assert!(!format!("{terminal:?} {terminal}").contains("private-terminal"));
    let returned = anyhow::Error::new(terminal);
    let moved = returned.downcast_ref::<TerminalCleanupFailure>().unwrap();
    assert_eq!(
        moved.primary().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(
        ios_core::classify_error(moved.primary()),
        ios_core::ErrorClass::Disconnected
    );
    drop(returned);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn terminal_inline_expiry_borrows_the_same_original_timer_token_before_and_after_return() {
    let registry = Registry::new().unwrap();
    let elapsed = tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
        .await
        .unwrap_err();
    let terminal = TerminalCleanupFailure {
        native: None,
        history: None,
        scan_history: None,
        metadata_http: None,
        elapsed: Some(elapsed),
        snapshot: registry.snapshot(),
        native_complete: false,
        metadata_http_complete: false,
    };
    assert!(std::ptr::eq(
        terminal
            .primary()
            .downcast_ref::<tokio::time::error::Elapsed>()
            .unwrap(),
        terminal.elapsed.as_ref().unwrap(),
    ));
    let returned = anyhow::Error::new(terminal);
    let moved = returned.downcast_ref::<TerminalCleanupFailure>().unwrap();
    assert!(std::ptr::eq(
        moved
            .source()
            .unwrap()
            .downcast_ref::<tokio::time::error::Elapsed>()
            .unwrap(),
        moved.elapsed.as_ref().unwrap(),
    ));
    assert_eq!(
        ios_core::classify_error(moved.primary()),
        ios_core::ErrorClass::Timeout
    );
}
