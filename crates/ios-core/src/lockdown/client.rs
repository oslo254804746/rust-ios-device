use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};

use super::response_error::ResponseErrorPolicy;
use crate::lockdown::pair_record::PairRecord;
use crate::lockdown::protocol::*;
use crate::lockdown::session::start_lockdown_session;
use crate::lockdown::{LockdownError, ServiceInfo};

/// High-level Lockdown client. Handles session management and service starting.
pub struct LockdownClient {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    session_id: Option<String>,
}

impl LockdownClient {
    /// Create a LockdownClient from an already-connected usbmux stream, performing TLS handshake.
    pub async fn connect_with_stream<S>(
        stream: S,
        pair_record: &PairRecord,
    ) -> Result<Self, LockdownError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (session_id, reader, writer) = start_lockdown_session(stream, pair_record).await?;
        Ok(Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            session_id: Some(session_id),
        })
    }

    /// Get a value from lockdown.
    pub async fn get_value(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<plist::Value, LockdownError> {
        self.get_value_with_policy(domain, key, ResponseErrorPolicy::Legacy)
            .await
    }

    /// Get a value while retaining device Error fields as LockdownResponseError.
    /// Requests and successful values retain the legacy policy. A present
    /// non-string Error field is rejected as malformed received protocol data.
    pub async fn get_value_preserving_errors(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<plist::Value, LockdownError> {
        self.get_value_with_policy(domain, key, ResponseErrorPolicy::Preserve)
            .await
    }

    async fn get_value_with_policy(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
        policy: ResponseErrorPolicy,
    ) -> Result<plist::Value, LockdownError> {
        send_lockdown(
            &mut self.writer,
            &GetValueRequest {
                label: "ios-rs",
                request: "GetValue",
                domain,
                key,
            },
        )
        .await?;
        let resp: plist::Value = recv_lockdown(&mut self.reader).await?;
        match policy {
            ResponseErrorPolicy::Legacy => extract_get_value(resp, domain, key),
            ResponseErrorPolicy::Preserve => {
                extract_get_value_with_policy(resp, domain, key, policy)
            }
        }
    }

    /// Set a lockdown value.
    pub async fn set_value<T>(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
        value: T,
    ) -> Result<(), LockdownError>
    where
        T: Serialize,
    {
        send_lockdown(
            &mut self.writer,
            &SetValueRequest {
                label: "ios-rs",
                request: "SetValue",
                domain,
                key,
                value,
            },
        )
        .await?;
        let resp: ValueOperationResponse = recv_lockdown(&mut self.reader).await?;
        if let Some(err) = resp.error {
            return Err(LockdownError::Protocol(format!(
                "SetValue failed for domain={domain:?} key={key:?}: {err}"
            )));
        }
        Ok(())
    }

    /// Remove a lockdown value.
    pub async fn remove_value(
        &mut self,
        domain: Option<&str>,
        key: Option<&str>,
    ) -> Result<(), LockdownError> {
        send_lockdown(
            &mut self.writer,
            &RemoveValueRequest {
                label: "ios-rs",
                request: "RemoveValue",
                domain,
                key,
            },
        )
        .await?;
        let resp: ValueOperationResponse = recv_lockdown(&mut self.reader).await?;
        if let Some(err) = resp.error {
            return Err(LockdownError::Protocol(format!(
                "RemoveValue failed for domain={domain:?} key={key:?}: {err}"
            )));
        }
        Ok(())
    }

    /// Apply a legacy activation record through lockdown.
    ///
    /// iOS versions predating the session-based mobileactivationd flow expect
    /// the server's `activation-record` value under an `Activate` request.
    /// This remains separate from `HandleActivationInfoWithSessionRequest` so
    /// callers cannot accidentally send one protocol's envelope to the other.
    pub async fn activate(&mut self, activation_record: plist::Value) -> Result<(), LockdownError> {
        self.simple_request("Activate", Some(("ActivationRecord", activation_record)))
            .await
    }

    /// Deactivate through the legacy lockdown protocol.
    pub async fn deactivate(&mut self) -> Result<(), LockdownError> {
        self.simple_request("Deactivate", None).await
    }

    async fn simple_request(
        &mut self,
        request_name: &str,
        field: Option<(&str, plist::Value)>,
    ) -> Result<(), LockdownError> {
        let mut request = plist::Dictionary::from_iter([
            ("Label".to_string(), plist::Value::String("ios-rs".into())),
            (
                "Request".to_string(),
                plist::Value::String(request_name.to_owned()),
            ),
        ]);
        if let Some((key, value)) = field {
            request.insert(key.to_owned(), value);
        }
        send_lockdown(&mut self.writer, &plist::Value::Dictionary(request)).await?;
        let response: plist::Value = recv_lockdown(&mut self.reader).await?;
        let response = response.as_dictionary().ok_or_else(|| {
            LockdownError::Protocol(format!("{request_name} returned a non-dictionary response"))
        })?;
        match response.get("Request").and_then(plist::Value::as_string) {
            Some(request) if request == request_name => {}
            _ => {
                return Err(LockdownError::Protocol(format!(
                    "{request_name} returned an unexpected response request"
                )))
            }
        }
        if let Some(error) = response.get("Error") {
            return Err(LockdownError::Protocol(format!(
                "{request_name} failed: {}",
                redact_request_error(error)
            )));
        }
        if let Some(status) = response.get("Status").and_then(plist::Value::as_string) {
            if matches!(
                status.to_ascii_lowercase().as_str(),
                "error" | "failed" | "failure" | "rejected"
            ) {
                return Err(LockdownError::Protocol(format!(
                    "{request_name} failed with Status={status}"
                )));
            }
        }
        if let Some(chain) = response.get("ErrorChain") {
            let count = chain.as_array().map_or(1, std::vec::Vec::len);
            if count > 0 {
                return Err(LockdownError::Protocol(format!(
                    "{request_name} failed with ErrorChain entries={count}"
                )));
            }
        }
        // Older lockdown versions report failure under `Result` rather than
        // `Error`; accepting that response would falsely acknowledge Activate
        // or Deactivate and then persist a misleading local state marker.
        if response
            .get("Result")
            .and_then(plist::Value::as_string)
            .is_some_and(|result| result == "Failure")
        {
            return Err(LockdownError::Protocol(format!(
                "{request_name} failed with Result=Failure"
            )));
        }
        Ok(())
    }

    /// Start a service and return its port information.
    pub async fn start_service(&mut self, service: &str) -> Result<ServiceInfo, LockdownError> {
        send_lockdown(
            &mut self.writer,
            &StartServiceRequest {
                label: "ios-rs",
                request: "StartService",
                service: service.to_string(),
            },
        )
        .await?;
        let resp: StartServiceResponse = recv_lockdown(&mut self.reader).await?;
        if let Some(err) = resp.error {
            return Err(LockdownError::Protocol(format!(
                "StartService '{service}' failed: {err}"
            )));
        }
        let port = resp.port.ok_or_else(|| {
            LockdownError::Protocol(format!("StartService '{service}': missing Port field"))
        })?;
        Ok(ServiceInfo {
            port,
            enable_service_ssl: resp.enable_service_ssl.unwrap_or(false),
        })
    }

    /// Stop the current session.
    pub async fn stop_session(&mut self) -> Result<(), LockdownError> {
        if let Some(sid) = self.session_id.take() {
            send_lockdown(
                &mut self.writer,
                &StopSessionRequest {
                    label: "ios-rs",
                    request: "StopSession",
                    session_id: sid,
                },
            )
            .await?;
        }
        Ok(())
    }

    /// Get the device product version string.
    pub async fn product_version(&mut self) -> Result<semver::Version, LockdownError> {
        let val = self.get_value(None, Some("ProductVersion")).await?;
        let s = val
            .as_string()
            .ok_or_else(|| LockdownError::Protocol("ProductVersion is not a string".into()))?;
        // iOS may return "15.5" (two-part); semver requires three parts
        let normalized = match s.matches('.').count() {
            0 => format!("{s}.0.0"),
            1 => format!("{s}.0"),
            _ => s.to_string(),
        };
        semver::Version::parse(&normalized).map_err(|error| {
            super::error_context::native(
                std::io::ErrorKind::InvalidData,
                "parse lockdown product version",
                error,
            )
        })
    }
}

fn redact_request_error(value: &plist::Value) -> String {
    match value {
        plist::Value::String(text) => format!("<redacted error: {} chars>", text.chars().count()),
        plist::Value::Data(data) => format!("<redacted data: {} bytes>", data.len()),
        plist::Value::Integer(_) | plist::Value::Real(_) | plist::Value::Boolean(_) => {
            format!("{value:?}")
        }
        _ => "<redacted structured error>".into(),
    }
}

fn extract_get_value(
    response: plist::Value,
    domain: Option<&str>,
    key: Option<&str>,
) -> Result<plist::Value, LockdownError> {
    extract_get_value_with_policy(response, domain, key, ResponseErrorPolicy::Legacy)
}

fn extract_get_value_with_policy(
    response: plist::Value,
    domain: Option<&str>,
    key: Option<&str>,
    policy: ResponseErrorPolicy,
) -> Result<plist::Value, LockdownError> {
    if let plist::Value::Dictionary(mut values) = response {
        if let Some(error) = values.remove("Error") {
            match error {
                plist::Value::String(error) => {
                    return Err(policy.remote_error("GetValue", error, |error| {
                        format!("GetValue failed for domain={domain:?} key={key:?}: {error}")
                    }))
                }
                _ if matches!(policy, ResponseErrorPolicy::Preserve) => {
                    return Err(LockdownError::Protocol(
                        "GetValue Error field is not a string".into(),
                    ))
                }
                _ => {}
            }
        }

        if let Some(value) = values.remove("Value") {
            return Ok(value);
        }

        return Err(LockdownError::Protocol(format!(
            "GetValue missing Value for domain={domain:?} key={key:?}: {:?}",
            plist::Value::Dictionary(values)
        )));
    }

    Err(LockdownError::Protocol(format!(
        "GetValue returned non-dictionary response for domain={domain:?} key={key:?}: {response:?}"
    )))
}

#[cfg(test)]
mod tests {
    use crate::test_util::MockStream;
    use std::error::Error;

    use super::*;

    #[tokio::test]
    async fn response_codes_get_value_preserves_wire_code_and_legacy_policy() {
        use crate::{classify_error, ErrorClass, LockdownResponseError};
        for code in [
            "MissingValue",
            "InvalidService",
            "owned-private-MissingValue",
            "",
        ] {
            let mut requests = Vec::new();
            for preserve in [false, true] {
                let (host, mut peer) = tokio::io::duplex(4096);
                let (reader, writer) = tokio::io::split(host);
                let mut client = LockdownClient {
                    reader: Box::new(reader),
                    writer: Box::new(writer),
                    session_id: None,
                };
                let caller = async {
                    let error = if preserve {
                        client
                            .get_value_preserving_errors(Some("owned-domain"), Some("owned-key"))
                            .await
                    } else {
                        client
                            .get_value(Some("owned-domain"), Some("owned-key"))
                            .await
                    }
                    .unwrap_err();
                    drop(client);
                    error
                };
                let server = async {
                    let request = recv_frame(&mut peer).await.unwrap();
                    let reply = plist::Dictionary::from_iter([
                        ("Error".to_string(), plist::Value::String(code.into())),
                        ("Value".to_string(), plist::Value::Boolean(true)),
                    ]);
                    send_lockdown(&mut peer, &reply).await.unwrap();
                    request
                };
                let (error, request) =
                    tokio::time::timeout(std::time::Duration::from_secs(1), async {
                        tokio::join!(caller, server)
                    })
                    .await
                    .unwrap();
                requests.push(request);
                if preserve {
                    let observed = LockdownResponseError::in_error_chain(&error).unwrap();
                    assert_eq!(observed.operation(), "GetValue");
                    assert_eq!(observed.code(), code);
                    assert_eq!(
                        classify_error(&error),
                        if code == "MissingValue" {
                            ErrorClass::NotFound
                        } else {
                            ErrorClass::ServiceFailure
                        }
                    );
                    assert!(observed.source().is_none());
                    assert!(!format!("{error:?} {error}").contains("owned-private"));
                } else {
                    assert!(LockdownResponseError::in_error_chain(&error).is_none());
                    assert_eq!(classify_error(&error), ErrorClass::Protocol);
                    let LockdownError::Protocol(text) = error else {
                        panic!("legacy variant changed")
                    };
                    assert_eq!(text, format!("GetValue failed for domain=Some(\"owned-domain\") key=Some(\"owned-key\"): {code}"));
                }
            }
            assert_eq!(requests[0], requests[1]);
            let request: plist::Value = plist::from_bytes(&requests[0]).unwrap();
            let request = request.as_dictionary().unwrap();
            assert_eq!(request["Request"].as_string(), Some("GetValue"));
            assert_eq!(request["Label"].as_string(), Some("ios-rs"));
            assert_eq!(request["Domain"].as_string(), Some("owned-domain"));
            assert_eq!(request["Key"].as_string(), Some("owned-key"));
        }
    }

    #[tokio::test]
    async fn response_codes_get_value_retains_success_shapes_and_native_io() {
        use crate::{classify_error, ErrorClass, LockdownResponseError};
        for reply in [
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Value".to_owned(),
                plist::Value::Boolean(false),
            )])),
            plist::Value::Dictionary(plist::Dictionary::new()),
            plist::Value::Array(vec![]),
            // Legacy accepts the Value; preserving refuses this malformed Error.
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("Error".to_owned(), plist::Value::Integer(7.into())),
                ("Value".to_owned(), plist::Value::Boolean(true)),
            ])),
        ] {
            let mut results = Vec::new();
            for preserve in [false, true] {
                let mut client = LockdownClient {
                    reader: Box::new(MockStream::with_response(reply.clone())),
                    writer: Box::new(MockStream::eof()),
                    session_id: None,
                };
                let result = if preserve {
                    client.get_value_preserving_errors(None, None).await
                } else {
                    client.get_value(None, None).await
                };
                match result {
                    Ok(value) => results.push(Ok(value)),
                    Err(error) => {
                        assert_eq!(classify_error(&error), ErrorClass::Protocol);
                        assert!(LockdownResponseError::in_error_chain(&error).is_none());
                        results.push(Err(error.to_string()));
                    }
                }
            }
            if reply
                .as_dictionary()
                .is_some_and(|reply| reply.contains_key("Error"))
            {
                assert_eq!(results[0], Ok(plist::Value::Boolean(true)));
                assert_eq!(
                    results[1],
                    Err("protocol error: GetValue Error field is not a string".to_owned())
                );
            } else {
                assert_eq!(results[0], results[1]);
            }
        }
        let (host, mut peer) = tokio::io::duplex(4096);
        let (reader, writer) = tokio::io::split(host);
        let mut client = LockdownClient {
            reader: Box::new(reader),
            writer: Box::new(writer),
            session_id: None,
        };
        let caller = async {
            client
                .get_value_preserving_errors(None, None)
                .await
                .unwrap_err()
        };
        let server = async {
            let _: plist::Value = recv_lockdown(&mut peer).await.unwrap();
            drop(peer);
        };
        let (error, ()) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(caller, server)
        })
        .await
        .unwrap();
        assert_eq!(classify_error(&error), ErrorClass::Io);
        let LockdownError::Io(io) = &error else {
            panic!("native IO changed")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(LockdownResponseError::in_error_chain(&error).is_none());
    }

    #[tokio::test]
    async fn actual_product_version_parse_retains_semver_source_and_normalization() {
        use crate::{classify_error, ErrorClass};
        for (version, expected) in [
            ("17", Some("17.0.0")),
            ("17.5", Some("17.5.0")),
            ("17.5.1", Some("17.5.1")),
            ("owned-private-invalid", None),
        ] {
            let response =
                plist::Value::Dictionary(plist::Dictionary::from_iter([("Value", version)]));
            let mut client = LockdownClient {
                reader: Box::new(MockStream::with_response(response)),
                writer: Box::new(MockStream::eof()),
                session_id: None,
            };
            let result = client.product_version().await;
            if let Some(expected) = expected {
                assert_eq!(result.unwrap().to_string(), expected);
            } else {
                let error = result.unwrap_err();
                let LockdownError::Io(io) = &error else {
                    panic!("semver source flattened");
                };
                assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);
                assert_eq!(classify_error(&error), ErrorClass::Protocol);
                assert!(io.source().unwrap().is::<semver::Error>());
                assert!(!format!("{error:?} {:?}", io.get_ref()).contains("owned-private"));
            }
        }
    }

    #[test]
    fn missing_get_value_payload_reports_context() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Success".into()),
        )]));

        let err = extract_get_value(
            response,
            Some("com.apple.mobile.wireless_lockdown"),
            Some("EnableWifiConnections"),
        )
        .expect_err("missing value should error");

        let rendered = err.to_string();
        assert!(rendered.contains("EnableWifiConnections"));
        assert!(rendered.contains("com.apple.mobile.wireless_lockdown"));
        assert!(rendered.contains("Status"));
    }

    #[tokio::test]
    async fn legacy_activation_rejects_result_failure() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Request".to_string(),
                plist::Value::String("Activate".into()),
            ),
            ("Result".to_string(), plist::Value::String("Failure".into())),
        ]));
        let reader = MockStream::with_response(response);
        let writer = MockStream::eof();
        let mut client = LockdownClient {
            reader: Box::new(reader),
            writer: Box::new(writer),
            session_id: None,
        };

        let error = client
            .activate(plist::Value::Dictionary(plist::Dictionary::new()))
            .await
            .expect_err("Result=Failure must not be treated as success");
        assert!(error.to_string().contains("Result=Failure"));
    }

    #[tokio::test]
    async fn legacy_activation_rejects_unrelated_response_request() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Request".to_string(),
            plist::Value::String("GetValue".into()),
        )]));
        let reader = MockStream::with_response(response);
        let writer = MockStream::eof();
        let mut client = LockdownClient {
            reader: Box::new(reader),
            writer: Box::new(writer),
            session_id: None,
        };

        let error = client
            .deactivate()
            .await
            .expect_err("a response for another request must be rejected");
        assert!(error.to_string().contains("unexpected response request"));
    }
}
