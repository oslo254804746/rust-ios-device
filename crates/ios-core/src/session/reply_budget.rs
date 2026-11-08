//! Logical response wire and retained UTF-8 accounting, not a parser/RSS bound.
//!
//! Charge the header and declared body before allocating/reading the body.
//! Requests, TLS records/handshake bytes, and caller-owned credentials are not
//! charged. Plain plist decoding retains its existing temporary allocations.

use std::{io, sync::Mutex};

use super::SessionFailure;
use crate::ErrorClass;

pub(crate) const MAX_MUX_FRAME: usize = 16 * 1024 * 1024;
pub(crate) const MAX_LOCKDOWN_BODY: usize = 4 * 1024 * 1024;
// Exactly ListDevices + Connect + QueryType + StartSession + GetValue.
pub(crate) const MAX_QUERY_REPLY_WIRE: usize = 2 * MAX_MUX_FRAME + 3 * (4 + MAX_LOCKDOWN_BODY);
pub(crate) const MAX_RETAINED_UTF8: usize = 16 * 1024 * 1024;

pub(crate) fn limit(reason: &'static str) -> io::Error {
    io::Error::other(SessionFailure {
        class: ErrorClass::ResourceLimit,
        reason,
    })
}

pub(crate) struct ReplyBudget {
    remaining: Mutex<usize>,
    mux_frame: usize,
    lockdown_body: usize,
}

impl ReplyBudget {
    #[cfg(test)]
    pub(crate) fn remaining(&self) -> usize {
        *self
            .remaining
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn new(total: usize, mux_frame: usize, lockdown_body: usize) -> Self {
        Self {
            remaining: Mutex::new(total),
            mux_frame,
            lockdown_body,
        }
    }

    fn charge(&self, bytes: usize) -> io::Result<()> {
        let mut remaining = self
            .remaining
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *remaining = remaining
            .checked_sub(bytes)
            .ok_or_else(|| limit("device query cumulative reply wire budget exceeded"))?;
        Ok(())
    }

    pub(crate) fn mux(&self, total: usize) -> io::Result<()> {
        if total > MAX_MUX_FRAME || total > self.mux_frame {
            return Err(limit("device query mux frame byte limit exceeded"));
        }
        self.charge(total)
    }

    pub(crate) fn lockdown(&self, body: usize) -> io::Result<()> {
        if body > MAX_LOCKDOWN_BODY || body > self.lockdown_body {
            return Err(limit("device query lockdown body byte limit exceeded"));
        }
        self.charge(
            body.checked_add(4)
                .ok_or_else(|| limit("device query reply wire length overflow"))?,
        )
    }
}

pub(crate) struct OutputBudget {
    remaining: usize,
}

impl OutputBudget {
    pub(crate) fn new(remaining: usize) -> Self {
        Self { remaining }
    }

    pub(crate) fn admit(&mut self, utf8_bytes: usize) -> io::Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(utf8_bytes)
            .ok_or_else(|| limit("device query retained UTF-8 budget exceeded"))?;
        Ok(())
    }
}
