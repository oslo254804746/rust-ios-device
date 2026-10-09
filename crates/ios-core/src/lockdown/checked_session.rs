//! Additive existing-trust lockdown sessions confined to one operation lease.
//!
//! Streams are already owned by the caller and pairing material already exists.
//! This module never enumerates, pairs, saves credentials, opens service ports,
//! or starts/stops a tunnel. QueryType, StartSession, TLS and StartService share
//! the borrowed operation context's absolute budget and cancellation signal.

use std::{fmt, io};

use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};

use super::{
    recv_lockdown, send_lockdown, LockdownError, LockdownResponseError, PairRecord,
    QueryTypeRequest, ServiceInfo, StartServiceRequest, StartSessionRequest,
};
use crate::session::OperationContext;

trait SessionIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<S: AsyncRead + AsyncWrite + Unpin + Send> SessionIo for S {}

/// A checked session whose transport cannot outlive its operation context.
///
/// The private stream has no getter/unwrap escape. StartService requests the
/// caller-selected service and returns its port/SSL response; it does not open
/// that port or send service protocol messages. Remote StartService effects
/// are not rolled back or acknowledged as retired on cancellation.
/// A failed or cancelled StartService retires the owned transport; a dropped
/// in-flight method also closes it rather than permit reuse of partial wire IO.
pub(crate) struct CheckedSession<'ctx> {
    context: &'ctx OperationContext,
    stream: Option<Box<dyn SessionIo + 'ctx>>,
    #[cfg(test)]
    session_id: String,
    tls: bool,
}

impl fmt::Debug for CheckedSession<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CheckedSession")
            .field("transport_present", &self.stream.is_some())
            .field("tls", &self.tls)
            .finish_non_exhaustive()
    }
}

impl<'ctx> CheckedSession<'ctx> {
    /// Session metadata is available to explicit callers; redact diagnostics.
    #[cfg(test)]
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn uses_tls(&self) -> bool {
        self.tls
    }

    pub(crate) async fn start_service(
        &mut self,
        service: &str,
    ) -> Result<ServiceInfo, LockdownError> {
        let mut stream = self.stream.take().ok_or_else(|| {
            LockdownError::Protocol("checked existing-trust session has been retired".into())
        })?;
        let result = self
            .context
            .run(async {
                send_lockdown(
                    &mut stream,
                    &StartServiceRequest {
                        label: "ios-rs",
                        request: "StartService",
                        service: service.to_owned(),
                    },
                )
                .await?;
                let reply: plist::Dictionary = recv_lockdown(&mut stream).await?;
                check_response(&reply, "StartService")?;
                let port = reply
                    .get("Port")
                    .and_then(plist::Value::as_unsigned_integer)
                    .ok_or_else(|| protocol("checked StartService Port was absent or invalid"))?;
                let port = u16::try_from(port).map_err(|error| {
                    super::error_context::native(
                        io::ErrorKind::InvalidData,
                        "decode checked StartService Port",
                        error,
                    )
                })?;
                if port == 0 {
                    return Err(protocol("checked StartService Port was zero"));
                }
                // Omission is distinct from a valid false value. The checked path
                // requires explicit metadata; legacy StartService remains unchanged.
                let ssl = reply
                    .get("EnableServiceSSL")
                    .and_then(plist::Value::as_boolean)
                    .ok_or_else(|| {
                        protocol(
                            "checked StartService EnableServiceSSL was absent or not a boolean",
                        )
                    })?;
                Ok::<_, LockdownError>(ServiceInfo {
                    port,
                    enable_service_ssl: ssl,
                })
            })
            .await;
        match result {
            Ok(info) => {
                self.stream = Some(stream);
                Ok(info)
            }
            Err(error) => Err(error),
        }
    }
}

/// Perform QueryType + StartSession and optional real TLS on an owned stream.
///
/// Error-only replies are interpreted before success fields. A valid explicit
/// false EnableSessionSSL creates a plaintext session. No successful session
/// or raw transport can escape the lifetime borrowed from `context`.
pub(crate) async fn start_existing_trust_session<'ctx, S>(
    context: &'ctx OperationContext,
    stream: S,
    pair_record: &PairRecord,
) -> Result<CheckedSession<'ctx>, LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'ctx,
{
    context
        .run(async move {
            let mut stream = context.guard(stream);
            send_lockdown(
                &mut stream,
                &QueryTypeRequest {
                    label: "ios-rs",
                    request: "QueryType",
                },
            )
            .await?;
            let reply: plist::Dictionary = recv_lockdown(&mut stream).await?;
            check_response(&reply, "QueryType")?;
            if reply.get("Type").and_then(plist::Value::as_string)
                != Some("com.apple.mobile.lockdown")
            {
                return Err(protocol("checked QueryType did not identify lockdown"));
            }
            send_lockdown(
                &mut stream,
                &StartSessionRequest {
                    label: "ios-rs",
                    protocol_version: "2",
                    request: "StartSession",
                    host_id: pair_record.host_id.clone(),
                    system_buid: pair_record.system_buid.clone(),
                },
            )
            .await?;
            let reply: plist::Dictionary = recv_lockdown(&mut stream).await?;
            check_response(&reply, "StartSession")?;
            reply
                .get("SessionID")
                .and_then(plist::Value::as_string)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    protocol("checked StartSession SessionID was absent or not a nonempty string")
                })?;
            #[cfg(test)]
            let session_id = reply
                .get("SessionID")
                .and_then(plist::Value::as_string)
                .unwrap()
                .to_owned();
            let tls = reply
                .get("EnableSessionSSL")
                .and_then(plist::Value::as_boolean)
                .ok_or_else(|| {
                    protocol("checked StartSession EnableSessionSSL was absent or not a boolean")
                })?;
            // recv_lockdown reads exactly one unbuffered frame, preserving any
            // coalesced TLS bytes in the same owned guarded transport.
            let stream: Box<dyn SessionIo + 'ctx> = if tls {
                let config = super::session::build_rustls_config(pair_record)?;
                let server_name = ServerName::try_from("lockdown").map_err(|error| {
                    super::error_context::native(
                        io::ErrorKind::InvalidInput,
                        "parse checked lockdown TLS server name",
                        error,
                    )
                })?;
                let tls = tokio_rustls::TlsConnector::from(config)
                    .connect(server_name, stream)
                    .await
                    .map_err(|error| {
                        super::error_context::native(
                            error.kind(),
                            "handshake checked existing-trust lockdown TLS",
                            error,
                        )
                    })?;
                Box::new(tls)
            } else {
                Box::new(stream)
            };
            Ok(CheckedSession {
                context,
                stream: Some(stream),
                #[cfg(test)]
                session_id,
                tls,
            })
        })
        .await
}

fn protocol(message: &'static str) -> LockdownError {
    LockdownError::Protocol(message.into())
}

fn check_response(reply: &plist::Dictionary, operation: &'static str) -> Result<(), LockdownError> {
    if let Some(request) = reply.get("Request") {
        if request.as_string() != Some(operation) {
            return Err(protocol("checked lockdown received an unrelated response"));
        }
    }
    if let Some(error) = reply.get("Error") {
        let code = error
            .as_string()
            .ok_or_else(|| protocol("checked lockdown Error was not a string"))?;
        return Err(LockdownError::Io(io::Error::other(
            LockdownResponseError::new(operation, code.to_owned()),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
