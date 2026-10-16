//! Owned JSON wire bytes, admitted before creation and retained through frames.
//! Record/Value input graphs and Bytes/HTTP private headers are separate owners.
use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use ios_core::ErrorClass;
use serde::{Serialize, Serializer};
use tokio::sync::Semaphore;

use super::metadata_vec::{exact_backing, AdmittedVec};

// Only fixed, reviewed diagnostic envelopes use this buffer. This is not a
// maximum for successful local/federated output or device-provided records.
const DIAGNOSTIC_CAPACITY: usize = 2048;

pub(super) struct Replies {
    bank: Arc<Semaphore>,
    diagnostic: Mutex<Option<AdmittedVec<u8>>>,
}

// The Err is already a final Axum response. Keep it inline rather than adding
// an independent Box allocation after an acquired native failure.
#[allow(clippy::result_large_err)]
impl Replies {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Result<Self, Response> {
        let diagnostic = exact_backing(bank, DIAGNOSTIC_CAPACITY).map_err(|_| {
            classified(
                static_json(
                    StatusCode::SERVICE_UNAVAILABLE,
                    b"{\"message\":\"manager response admission failed\"}",
                ),
                ErrorClass::ResourceLimit,
            )
        })?;
        Ok(Self {
            bank: Arc::clone(bank),
            diagnostic: Mutex::new(Some(diagnostic)),
        })
    }

    pub(super) fn json<T: Serialize + ?Sized>(&self, value: &T) -> Result<Response, Response> {
        encode(&self.bank, value).map(response).map_err(|error| {
            let class = crate::output::operation_error_class(error.as_ref());
            self.operation_error(
                if class == ErrorClass::ResourceLimit {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                },
                "manager response serialization failed",
                error.as_ref(),
            )
        })
    }

    pub(super) fn operation(&self, operation: &'static str) -> Result<Response, Response> {
        #[derive(Serialize)]
        struct Operation {
            data: bool,
            operation: &'static str,
        }
        self.json(&Operation {
            operation,
            data: true,
        })
    }

    pub(super) fn shutdown(&self) -> Result<Response, Response> {
        #[derive(Serialize)]
        struct Shutdown {
            data: bool,
            message: &'static str,
            operation: &'static str,
        }
        self.json(&Shutdown {
            operation: "shutdown",
            data: true,
            message: "Server shutting down...",
        })
    }

    pub(super) fn upstream(
        &self,
        operation: &'static str,
        url: &str,
        data: bool,
    ) -> Result<Response, Response> {
        #[derive(Serialize)]
        struct Change<'a> {
            data: bool,
            operation: &'static str,
            url: &'a str,
        }
        self.json(&Change {
            operation,
            url,
            data,
        })
    }

    pub(super) fn cancelled(&self, udid: &str) -> Result<Response, Response> {
        #[derive(Serialize)]
        struct Cancel<'a> {
            data: bool,
            operation: &'static str,
            udid: &'a str,
        }
        self.json(&Cancel {
            operation: "cancel",
            udid,
            data: true,
        })
    }

    pub(super) fn classified_error(
        &self,
        status: StatusCode,
        message: &'static str,
        class: ErrorClass,
    ) -> Response {
        #[derive(Serialize)]
        struct Message {
            message: &'static str,
        }
        self.error(status, class, &Message { message })
    }

    pub(super) fn operation_error(
        &self,
        status: StatusCode,
        message: &'static str,
        error: &(dyn std::error::Error + 'static),
    ) -> Response {
        #[derive(Serialize)]
        struct Failure<'a> {
            error: DisplayText<ios_core::redaction::DiagnosticError<'a>>,
            message: &'static str,
        }
        // The emergency data buffer already exists before the operation. Even
        // a ready native error crossing quota exhaustion keeps its own class.
        self.error(
            status,
            crate::output::operation_error_class(error),
            &Failure {
                message,
                error: DisplayText(ios_core::redaction::diagnostic_error(error)),
            },
        )
    }

    pub(super) fn not_found(&self, udid: &str) -> Response {
        #[derive(Serialize)]
        struct Missing<'a> {
            device: DisplayText<ios_core::redaction::DiagnosticId<'a>>,
            message: &'static str,
        }
        self.error(
            StatusCode::NOT_FOUND,
            ErrorClass::NotFound,
            &Missing {
                message: "tunnel not found",
                device: DisplayText(ios_core::redaction::diagnostic_id(udid)),
            },
        )
    }

    pub(super) fn unsupported(
        &self,
        query: &super::StartTunnelQuery,
        message: &'static str,
    ) -> Response {
        #[derive(Serialize)]
        struct Unsupported<'a> {
            connection_type: Option<&'static str>,
            device: DisplayText<ios_core::redaction::DiagnosticId<'a>>,
            message: &'static str,
        }
        self.error(
            StatusCode::NOT_IMPLEMENTED,
            ErrorClass::Unsupported,
            &Unsupported {
                message,
                device: DisplayText(ios_core::redaction::diagnostic_id(&query.udid)),
                connection_type: query.requested_connection_type(),
            },
        )
    }

    fn error<T: Serialize>(&self, status: StatusCode, class: ErrorClass, value: &T) -> Response {
        let allocation = self
            .diagnostic
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let mut reply = match allocation {
            Some(mut allocation) => match write_fixed(&mut allocation.values, value) {
                Ok(()) => response(allocation),
                // Preserve the operation status/class even for a broken reviewed
                // envelope; never replace a primary cause with pool exhaustion.
                Err(_) => static_json(status, b"{\"message\":\"manager operation failed\"}"),
            },
            None => static_json(status, b"{\"message\":\"manager operation failed\"}"),
        };
        *reply.status_mut() = status;
        classified(reply, class)
    }
}

struct DisplayText<T>(T);
impl<T: fmt::Display> Serialize for DisplayText<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Locked serde_json streams collect_str through escaped write_str;
        // using to_value would allocate an intermediate String instead.
        serializer.collect_str(&self.0)
    }
}

struct Counter(usize);
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("manager JSON length overflow"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Fixed<'a>(&'a mut Vec<u8>);
impl Write for Fixed<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.0.capacity() - self.0.len() {
            return Err(io::Error::other(
                "manager JSON changed during serialization",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn write_fixed<T: Serialize + ?Sized>(values: &mut Vec<u8>, value: &T) -> serde_json::Result<()> {
    serde_json::to_writer(Fixed(values), value)
}
fn encode<T: Serialize + ?Sized>(
    bank: &Arc<Semaphore>,
    value: &T,
) -> anyhow::Result<AdmittedVec<u8>> {
    let mut count = Counter(0);
    serde_json::to_writer(&mut count, value)?;
    let mut allocation = exact_backing(bank, count.0)?;
    write_fixed(&mut allocation.values, value)?;
    anyhow::ensure!(
        allocation.values.len() == count.0,
        "manager JSON changed during serialization"
    );
    Ok(allocation)
}
impl AsRef<[u8]> for AdmittedVec<u8> {
    fn as_ref(&self) -> &[u8] {
        &self.values
    }
}
fn response(allocation: AdmittedVec<u8>) -> Response {
    // Byte frames/clones can outlive both the handler and Body. The allocation
    // owns its grant until the last Bytes owner, not just until IntoResponse.
    let mut response = Body::from(Bytes::from_owner(allocation)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}
fn static_json(status: StatusCode, bytes: &'static [u8]) -> Response {
    let mut response = Body::from(Bytes::from_static(bytes)).into_response();
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}
fn classified(response: Response, class: ErrorClass) -> Response {
    crate::http_diagnostics::classified(
        response,
        class,
        crate::http_diagnostics::ErrorPhase::Manager,
    )
}

#[cfg(test)]
mod tests;
