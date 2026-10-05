//! Inspector attachment, Target transport envelopes and response state.
//! Operates on a supplied WebInspector client; no device dialing or HTTP.

use super::client::{await_operation, WebInspectorOperation};
use super::retained::admit_id;
use super::{WebInspectorClient, WebInspectorError, WebInspectorEvent};
use serde_json::{json, Value as JsonValue};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct InspectorSession {
    application_id: String,
    page_id: u64,
    session_id: String,
    target_id: Option<String>,
    next_transport_id: Option<u64>,
    next_command_id: Option<u64>,
}

impl InspectorSession {
    pub fn new(application_id: impl Into<String>, page_id: u64) -> Self {
        Self::with_session_id(
            application_id,
            page_id,
            Uuid::new_v4().to_string().to_uppercase(),
        )
    }

    pub fn with_session_id(
        application_id: impl Into<String>,
        page_id: u64,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            application_id: application_id.into(),
            page_id,
            session_id: session_id.into(),
            target_id: None,
            next_transport_id: Some(1),
            next_command_id: Some(1),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn application_id(&self) -> &str {
        &self.application_id
    }

    pub fn page_id(&self) -> u64 {
        self.page_id
    }

    pub fn target_id(&self) -> Option<&str> {
        self.target_id.as_deref()
    }

    /// One total budget covers setup and target discovery, capped by the client
    /// operation budget. An abandoned committed setup invalidates the client.
    pub async fn attach<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        wait_for_target: bool,
        timeout_duration: Duration,
    ) -> Result<(), WebInspectorError> {
        self.attach_with_pause(client, wait_for_target, timeout_duration, true)
            .await
    }

    /// Attach a CDP debugger without pausing the page as soon as it opens.
    pub async fn attach_unpaused<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        wait_for_target: bool,
        timeout_duration: Duration,
    ) -> Result<(), WebInspectorError> {
        self.attach_with_pause(client, wait_for_target, timeout_duration, false)
            .await
    }

    async fn attach_with_pause<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        wait_for_target: bool,
        timeout_duration: Duration,
        pause: bool,
    ) -> Result<(), WebInspectorError> {
        let deadline = client.deadline(Some(timeout_duration))?;
        let mut operation = WebInspectorOperation::with_deadline(client, deadline)?;
        let result = await_operation(deadline, "attach WebInspector session", async {
            operation
                .client
                .send_socket_setup(&self.session_id, &self.application_id, self.page_id, pause)
                .await?;
            operation.commit();
            if wait_for_target {
                self.wait_for_target(&mut operation).await?;
            }
            Ok(())
        })
        .await;
        operation.finish(result)
    }

    pub async fn next_raw_message<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        timeout_duration: Duration,
    ) -> Result<JsonValue, WebInspectorError> {
        let event = client
            .next_socket_data_with_timeout(timeout_duration)
            .await?;
        if let WebInspectorEvent::SocketData { message, .. } = event {
            self.observe_message(&message)?;
            return Ok(message);
        }
        unreachable!("next_socket_data_with_timeout only returns socket-data events");
    }

    pub async fn send_command<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        method: &str,
        params: JsonValue,
    ) -> Result<u64, WebInspectorError> {
        client.deadline(None)?;
        let params = match params {
            JsonValue::Object(_) => params,
            JsonValue::Null => JsonValue::Object(Default::default()),
            other => {
                return Err(WebInspectorError::Protocol(format!(
                    "webinspector command params must be a JSON object, got {other}"
                )))
            }
        };

        let command_id = admit_id(self.next_command_id)?;
        // Admit both counters before consuming either. MAX is valid once;
        // None records exhaustion and prevents wraparound or identifier reuse.
        let transport_id = self
            .target_id
            .as_ref()
            .map(|_| admit_id(self.next_transport_id))
            .transpose()?;
        self.next_command_id = command_id.checked_add(1);
        if let Some(transport_id) = transport_id {
            self.next_transport_id = transport_id.checked_add(1);
        }

        let payload = if let Some(target_id) = &self.target_id {
            let transport_id = transport_id.expect("target transport was admitted");
            JsonValue::Object(serde_json::Map::from_iter([
                ("id".to_string(), JsonValue::from(transport_id)),
                (
                    "method".to_string(),
                    JsonValue::String("Target.sendMessageToTarget".to_string()),
                ),
                (
                    "params".to_string(),
                    JsonValue::Object(serde_json::Map::from_iter([
                        ("targetId".to_string(), JsonValue::String(target_id.clone())),
                        (
                            "message".to_string(),
                            JsonValue::String(serde_json::to_string(&JsonValue::Object(
                                serde_json::Map::from_iter([
                                    ("id".to_string(), JsonValue::from(command_id)),
                                    ("method".to_string(), JsonValue::String(method.to_string())),
                                    ("params".to_string(), params),
                                ]),
                            ))?),
                        ),
                    ])),
                ),
            ]))
        } else {
            JsonValue::Object(serde_json::Map::from_iter([
                ("id".to_string(), JsonValue::from(command_id)),
                ("method".to_string(), JsonValue::String(method.to_string())),
                ("params".to_string(), params),
            ]))
        };

        client
            .send_socket_data(
                &self.session_id,
                &self.application_id,
                self.page_id,
                &payload,
            )
            .await?;
        Ok(command_id)
    }

    /// Send and correlate a response under one total deadline. Cancellation or
    /// failure after sending invalidates the client, preventing late replies
    /// from being reused by another session. Matched business errors remain
    /// response objects. Unrelated events are restored in order within local
    /// count/compact-JSON byte limits.
    pub async fn send_command_and_wait<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        method: &str,
        params: JsonValue,
        timeout_duration: Duration,
    ) -> Result<JsonValue, WebInspectorError> {
        let deadline = client.deadline(Some(timeout_duration))?;
        let mut operation = WebInspectorOperation::with_deadline(client, deadline)?;
        let result = await_operation(deadline, "execute WebInspector command", async {
            let command_id = self.send_command(operation.client, method, params).await?;
            operation.commit();
            self.wait_for_response(&mut operation, command_id).await
        })
        .await;
        // Only a matched response completes the committed wire obligation.
        // Inspector returns explicit business errors as response objects.
        if result.is_ok() {
            operation.finish_response(result)
        } else {
            operation.finish(result)
        }
    }

    pub async fn send_bridge_message<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        message: &JsonValue,
    ) -> Result<(), WebInspectorError> {
        client.deadline(None)?;
        let payload = if let Some(target_id) = &self.target_id {
            let transport_id = admit_id(self.next_transport_id)?;
            self.next_transport_id = transport_id.checked_add(1);
            json!({
                "id": transport_id,
                "method": "Target.sendMessageToTarget",
                "params": {
                    "targetId": target_id,
                    "message": serde_json::to_string(message)?,
                }
            })
        } else {
            message.clone()
        };

        client
            .send_socket_data(
                &self.session_id,
                &self.application_id,
                self.page_id,
                &payload,
            )
            .await
    }

    pub fn bridge_message(
        &mut self,
        message: &JsonValue,
    ) -> Result<Option<JsonValue>, WebInspectorError> {
        self.observe_message(message)?;
        if self.target_id.is_some()
            && message.get("id").is_some()
            && message.get("method").is_none()
        {
            return Ok(None);
        }
        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.dispatchMessageFromTarget")
        {
            let nested = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("message"))
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    WebInspectorError::Protocol(
                        "Target.dispatchMessageFromTarget missing params.message".to_string(),
                    )
                })?;
            let nested: JsonValue = serde_json::from_str(nested)?;
            self.observe_message(&nested)?;
            return Ok(Some(nested));
        }
        Ok(Some(message.clone()))
    }

    async fn wait_for_target<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        operation: &mut WebInspectorOperation<'_, S>,
    ) -> Result<(), WebInspectorError> {
        while self.target_id.is_none() {
            let event = operation.client.next_socket_data().await?;
            let WebInspectorEvent::SocketData { message, .. } = &event else {
                unreachable!("next_socket_data_with_timeout only returns socket-data events");
            };
            self.observe_message(message)?;
            if self.target_id.is_none() {
                operation.preserve(event)?;
                tokio::task::yield_now().await;
            }
        }
        Ok(())
    }

    async fn wait_for_response<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        operation: &mut WebInspectorOperation<'_, S>,
        command_id: u64,
    ) -> Result<JsonValue, WebInspectorError> {
        loop {
            let event = operation.client.next_socket_data().await?;
            let WebInspectorEvent::SocketData { message, .. } = &event else {
                unreachable!("next_socket_data_with_timeout only returns socket-data events");
            };
            match self.match_response(message, command_id) {
                Ok(Some(response)) => {
                    return Ok(response);
                }
                Ok(None) => {
                    if self.should_preserve_message(message) {
                        operation.preserve(event)?;
                    }
                }
                Err(error) => {
                    return Err(error);
                }
            }
            tokio::task::yield_now().await;
        }
    }

    fn observe_message(&mut self, message: &JsonValue) -> Result<(), WebInspectorError> {
        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.targetCreated")
        {
            let target_id = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("targetInfo"))
                .and_then(JsonValue::as_object)
                .and_then(|info| info.get("targetId"))
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    WebInspectorError::Protocol(
                        "Target.targetCreated missing params.targetInfo.targetId".to_string(),
                    )
                })?;
            self.target_id = Some(target_id.to_string());
        }

        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.targetDestroyed")
        {
            if let Some(target_id) = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("targetId"))
                .and_then(JsonValue::as_str)
            {
                if self.target_id.as_deref() == Some(target_id) {
                    self.target_id = None;
                }
            }
        }

        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.didCommitProvisionalTarget")
        {
            let target_id = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("newTargetId"))
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    WebInspectorError::Protocol(
                        "Target.didCommitProvisionalTarget missing params.newTargetId".to_string(),
                    )
                })?;
            self.target_id = Some(target_id.to_string());
        }

        Ok(())
    }

    fn match_response(
        &mut self,
        message: &JsonValue,
        command_id: u64,
    ) -> Result<Option<JsonValue>, WebInspectorError> {
        if self.target_id.is_none()
            && message
                .get("id")
                .and_then(JsonValue::as_u64)
                .is_some_and(|id| id == command_id)
        {
            return Ok(Some(message.clone()));
        }

        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.dispatchMessageFromTarget")
        {
            let nested = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("message"))
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    WebInspectorError::Protocol(
                        "Target.dispatchMessageFromTarget missing params.message".to_string(),
                    )
                })?;
            let nested: JsonValue = serde_json::from_str(nested)?;
            self.observe_message(&nested)?;
            if nested
                .get("id")
                .and_then(JsonValue::as_u64)
                .is_some_and(|id| id == command_id)
            {
                return Ok(Some(nested));
            }
        }

        Ok(None)
    }

    fn should_preserve_message(&self, message: &JsonValue) -> bool {
        !(self.target_id.is_some()
            && message.get("id").is_some()
            && message.get("method").is_none())
    }
}

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_discovery_inspector_write_deadlines_keep_elapsed() {
        for stage in 0..3 {
            let (stream, _peer) = tokio::io::duplex(1);
            let mut client = WebInspectorClient::with_connection_id(stream, "owned-private");
            let mut session = InspectorSession::with_session_id("PID:42", 1, "owned-private");
            let duration = Duration::from_millis(5);
            let error = match stage {
                0 => session
                    .attach(&mut client, false, duration)
                    .await
                    .unwrap_err(),
                1 => session
                    .attach_unpaused(&mut client, false, duration)
                    .await
                    .unwrap_err(),
                _ => session
                    .send_command_and_wait(&mut client, "Runtime.enable", json!({}), duration)
                    .await
                    .unwrap_err(),
            };
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
            assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
            assert!(!format!("{error:?}").contains("owned-private"));
        }
    }

    #[test]
    fn inspector_session_observe_message_updates_target_after_provisional_commit() {
        let mut session = InspectorSession::with_session_id("PID:42", 1, "TEST-SESSION");
        session
            .observe_message(&json!({
                "method": "Target.targetCreated",
                "params": {
                    "targetInfo": {
                        "targetId": "target-1"
                    }
                }
            }))
            .unwrap();
        assert_eq!(session.target_id(), Some("target-1"));

        session
            .observe_message(&json!({
                "method": "Target.didCommitProvisionalTarget",
                "params": {
                    "newTargetId": "target-2"
                }
            }))
            .unwrap();
        assert_eq!(session.target_id(), Some("target-2"));
    }
}
