//! WebInspector command facade and shared workflow budgets.
//! Each adapter consumes core clients rather than owning wire protocol state.

use std::time::Duration;

mod cdp;
mod commands;
mod connection;
mod diagnostics;
mod http;
mod sessions;
mod shell;
mod tasks;
mod webdriver;
mod workflows;

pub use commands::WebInspectorCmd;

const PAGE_DISCOVERY_IDLE: Duration = Duration::from_millis(500);
const SESSION_CLEANUP_MAX: Duration = Duration::from_millis(100);

#[cfg(test)]
mod access_tests;

#[cfg(test)]
mod bridge_tests;
