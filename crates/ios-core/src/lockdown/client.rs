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
}
