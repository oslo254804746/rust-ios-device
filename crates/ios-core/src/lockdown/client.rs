use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};

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
        extract_get_value(resp, domain, key)
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
        semver::Version::parse(&normalized)
            .map_err(|e| LockdownError::Protocol(format!("invalid version '{s}': {e}")))
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
    if let plist::Value::Dictionary(mut values) = response {
        if let Some(plist::Value::String(error)) = values.remove("Error") {
            return Err(LockdownError::Protocol(format!(
                "GetValue failed for domain={domain:?} key={key:?}: {error}"
            )));
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

    use super::*;

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
