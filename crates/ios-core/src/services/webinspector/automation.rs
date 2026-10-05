//! Remote automation lifecycle, commands, result and element semantics.
//! Operates on a supplied client; selectors and JavaScript retain existing bytes.

use super::client::{await_operation, WebInspectorOperation};
use super::retained::admit_id;
use super::{
    AutomationAvailability, Page, WebInspectorClient, WebInspectorError, WebInspectorEvent,
};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    Id,
    XPath,
    LinkText,
    PartialLinkText,
    Name,
    TagName,
    ClassName,
    CssSelector,
}

impl By {
    fn as_wire(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::XPath => "xpath",
            Self::LinkText => "link text",
            Self::PartialLinkText => "partial link text",
            Self::Name => "name",
            Self::TagName => "tag name",
            Self::ClassName => "class name",
            Self::CssSelector => "css selector",
        }
    }
}

const FIND_NODES_JS: &str = r#"function(strategy,ancestorElement,query,firstResultOnly,timeoutDuration,callback){ancestorElement=ancestorElement||document;switch(strategy){case"id":strategy="css selector";query="[id=\""+escape(query)+"\"]";break;case"name":strategy="css selector";query="[name=\""+escape(query)+"\"]";break;}switch(strategy){case"css selector":case"link text":case"partial link text":case"tag name":case"class name":case"xpath":break;default: throw{name:"InvalidParameter",message:("Unsupported locator strategy: "+strategy+".")};}function escape(string){return string.replace(/\\/g,"\\\\").replace(/"/g,"\\\"");}function tryToFindNode(){try{switch(strategy){case"css selector":if(firstResultOnly)return ancestorElement.querySelector(query)||null;return Array.from(ancestorElement.querySelectorAll(query));case"link text":let linkTextResult=[];for(let link of ancestorElement.getElementsByTagName("a")){if(link.text.trim()==query){linkTextResult.push(link);if(firstResultOnly)break;}}if(firstResultOnly)return linkTextResult[0]||null;return linkTextResult;case"partial link text":let partialLinkResult=[];for(let link of ancestorElement.getElementsByTagName("a")){if(link.text.includes(query)){partialLinkResult.push(link);if(firstResultOnly)break;}}if(firstResultOnly)return partialLinkResult[0]||null;return partialLinkResult;case"tag name":let tagNameResult=ancestorElement.getElementsByTagName(query);if(firstResultOnly)return tagNameResult[0]||null;return Array.from(tagNameResult);case"class name":let classNameResult=ancestorElement.getElementsByClassName(query);if(firstResultOnly)return classNameResult[0]||null;return Array.from(classNameResult);case"xpath":if(firstResultOnly){let xpathResult=document.evaluate(query,ancestorElement,null,XPathResult.FIRST_ORDERED_NODE_TYPE,null);if(!xpathResult)return null;return xpathResult.singleNodeValue;}let xpathResult=document.evaluate(query,ancestorElement,null,XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,null);if(!xpathResult||!xpathResult.snapshotLength)return[];let arrayResult=[];for(let i=0;i<xpathResult.snapshotLength;++i)arrayResult.push(xpathResult.snapshotItem(i));return arrayResult;}}catch(error){ throw{name:"InvalidSelector",message:error.message};}}const pollInterval=50;let pollUntil=performance.now()+timeoutDuration;function pollForNode(){let result=tryToFindNode();if(typeof result==="string"||result instanceof Node||(result instanceof Array&&result.length)){callback(result);return;}let durationRemaining=pollUntil-performance.now();if(durationRemaining<pollInterval){callback(firstResultOnly?null:[]);return;}setTimeout(pollForNode,pollInterval);}pollForNode();}"#;
const CLICK_ELEMENT_JS: &str = r#"function(element) { element.click(); return null; }"#;
const ELEMENT_TEXT_JS: &str =
    r#"function(element) { return element.innerText.replace(/^[^\S\xa0]+|[^\S\xa0]+$/g, ""); }"#;
const ELEMENT_TAG_JS: &str = r#"function(element) { return element.tagName.toLowerCase(); }"#;

#[derive(Debug, Clone)]
pub struct AutomationSession {
    application_id: String,
    bundle_identifier: String,
    session_id: String,
    page_id: Option<u64>,
    top_level_handle: Option<String>,
    implicit_wait_timeout_ms: u64,
    page_load_timeout_ms: u64,
    next_command_id: Option<u64>,
}

impl AutomationSession {
    pub fn new(application_id: impl Into<String>, bundle_identifier: impl Into<String>) -> Self {
        Self::with_session_id(
            application_id,
            bundle_identifier,
            Uuid::new_v4().to_string().to_uppercase(),
        )
    }

    pub fn with_session_id(
        application_id: impl Into<String>,
        bundle_identifier: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            application_id: application_id.into(),
            bundle_identifier: bundle_identifier.into(),
            session_id: session_id.into(),
            page_id: None,
            top_level_handle: None,
            implicit_wait_timeout_ms: 0,
            page_load_timeout_ms: 3_000_000,
            next_command_id: Some(1),
        }
    }

    pub fn with_page(
        application_id: impl Into<String>,
        bundle_identifier: impl Into<String>,
        session_id: impl Into<String>,
        page_id: u64,
    ) -> Self {
        let mut session = Self::with_session_id(application_id, bundle_identifier, session_id);
        session.page_id = Some(page_id);
        session.top_level_handle = Some(String::new());
        session
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn bundle_identifier(&self) -> &str {
        &self.bundle_identifier
    }

    pub fn page_id(&self) -> u64 {
        self.page_id.unwrap_or_default()
    }

    pub fn set_implicit_wait_timeout(&mut self, timeout: Duration) {
        self.implicit_wait_timeout_ms = timeout.as_millis() as u64;
    }

    /// One capped total covers the automation request, listings and socket
    /// setup. An abandoned committed request invalidates the client; page_id
    /// is updated only after both discovery stages complete.
    pub async fn attach<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        timeout_duration: Duration,
    ) -> Result<(), WebInspectorError> {
        let deadline = client.deadline(Some(timeout_duration))?;
        if matches!(
            client.automation_availability(),
            Some(AutomationAvailability::NotAvailable)
        ) {
            return Err(WebInspectorError::Protocol(
                "remote automation is not available".to_string(),
            ));
        }
        let mut operation = WebInspectorOperation::with_deadline(client, deadline)?;
        let result = await_operation(deadline, "attach WebInspector automation session", async {
            operation
                .client
                .request_automation_session(&self.session_id, &self.application_id)
                .await?;
            operation.commit();
            operation
                .client
                .request_listing(&self.application_id)
                .await?;
            let page = self
                .wait_for_automation_page(operation.client, false)
                .await?;
            operation
                .client
                .send_socket_setup(&self.session_id, &self.application_id, page.id, true)
                .await?;
            operation
                .client
                .request_listing(&self.application_id)
                .await?;
            self.wait_for_automation_page(operation.client, true).await
        })
        .await;
        let page = operation.finish(result)?;
        self.page_id = Some(page.id);
        Ok(())
    }

    pub async fn start_session<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<String, WebInspectorError> {
        let response = self
            .send_command_and_wait(
                client,
                "createBrowsingContext",
                JsonValue::Object(Default::default()),
                Duration::from_secs(10),
            )
            .await?;
        let handle = response
            .get("handle")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                WebInspectorError::Protocol(
                    "Automation.createBrowsingContext missing result.handle".to_string(),
                )
            })?
            .to_string();
        self.top_level_handle = Some(handle.clone());
        Ok(handle)
    }

    pub async fn stop_session<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<(), WebInspectorError> {
        let Some(handle) = self.top_level_handle.clone() else {
            return Ok(());
        };
        let _ = self
            .send_command_and_wait(
                client,
                "closeBrowsingContext",
                json!({ "handle": handle }),
                Duration::from_secs(10),
            )
            .await?;
        self.top_level_handle = None;
        Ok(())
    }

    pub async fn navigate<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        url: &str,
    ) -> Result<(), WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let _ = self
            .send_command_and_wait(
                client,
                "navigateBrowsingContext",
                json!({
                    "handle": handle,
                    "pageLoadTimeout": self.page_load_timeout_ms,
                    "url": url,
                }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    pub async fn go_back<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<(), WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let _ = self
            .send_command_and_wait(
                client,
                "goBackInBrowsingContext",
                json!({
                    "handle": handle,
                    "pageLoadTimeout": self.page_load_timeout_ms,
                }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    pub async fn go_forward<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<(), WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let _ = self
            .send_command_and_wait(
                client,
                "goForwardInBrowsingContext",
                json!({
                    "handle": handle,
                    "pageLoadTimeout": self.page_load_timeout_ms,
                }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    pub async fn refresh<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<(), WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let _ = self
            .send_command_and_wait(
                client,
                "reloadBrowsingContext",
                json!({
                    "handle": handle,
                    "pageLoadTimeout": self.page_load_timeout_ms,
                }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    pub async fn current_url<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<Option<String>, WebInspectorError> {
        let context = self.get_browsing_context(client).await?;
        Ok(context
            .get("url")
            .and_then(JsonValue::as_str)
            .map(ToOwned::to_owned))
    }

    pub async fn execute_script<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        script: &str,
        args: &[JsonValue],
    ) -> Result<JsonValue, WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let response = self
            .send_command_and_wait(
                client,
                "evaluateJavaScriptFunction",
                json!({
                    "browsingContextHandle": handle,
                    "function": format!("function(){{\n{script}\n}}"),
                    "arguments": args.iter().map(stringify_automation_argument).collect::<Result<Vec<_>, _>>()?,
                }),
                Duration::from_secs(10),
            )
            .await?;
        decode_automation_result(&response)
    }

    pub async fn evaluate_js_function<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        function: &str,
        args: &[JsonValue],
        implicit_callback: bool,
    ) -> Result<JsonValue, WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let mut params = serde_json::Map::from_iter([
            (
                "browsingContextHandle".to_string(),
                JsonValue::String(handle),
            ),
            (
                "function".to_string(),
                JsonValue::String(function.to_string()),
            ),
            (
                "arguments".to_string(),
                JsonValue::Array(
                    args.iter()
                        .map(stringify_automation_argument)
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            ),
        ]);
        if implicit_callback {
            params.insert(
                "expectsImplicitCallbackArgument".to_string(),
                JsonValue::Bool(true),
            );
            if self.implicit_wait_timeout_ms > 0 {
                params.insert(
                    "callbackTimeout".to_string(),
                    JsonValue::from(self.implicit_wait_timeout_ms + 1_000),
                );
            }
        }
        let response = self
            .send_command_and_wait(
                client,
                "evaluateJavaScriptFunction",
                JsonValue::Object(params),
                Duration::from_secs(10),
            )
            .await?;
        decode_automation_result(&response)
    }

    pub async fn get_title<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<String, WebInspectorError> {
        Ok(self
            .evaluate_js_function(client, "function() { return document.title; }", &[], false)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub async fn get_page_source<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<String, WebInspectorError> {
        Ok(self
            .evaluate_js_function(
                client,
                "function() { return document.documentElement.outerHTML; }",
                &[],
                false,
            )
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub async fn screenshot_base64<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<String, WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let response = self
            .send_command_and_wait(
                client,
                "takeScreenshot",
                json!({
                    "handle": handle,
                    "clipToViewport": true,
                }),
                Duration::from_secs(10),
            )
            .await?;
        response
            .get("data")
            .and_then(JsonValue::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                WebInspectorError::Protocol(
                    "Automation.takeScreenshot missing result.data".to_string(),
                )
            })
    }

    pub async fn find_element<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        by: By,
        value: &str,
    ) -> Result<Option<JsonValue>, WebInspectorError> {
        Ok(self
            .find_elements_internal(client, by, value, true, None)
            .await?
            .into_iter()
            .next())
    }

    pub async fn find_elements<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        by: By,
        value: &str,
        single: bool,
    ) -> Result<Vec<JsonValue>, WebInspectorError> {
        self.find_elements_internal(client, by, value, single, None)
            .await
    }

    pub async fn click_element<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        element: &JsonValue,
    ) -> Result<(), WebInspectorError> {
        let _ = self
            .evaluate_js_function(
                client,
                CLICK_ELEMENT_JS,
                std::slice::from_ref(element),
                false,
            )
            .await?;
        Ok(())
    }

    pub async fn element_text<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        element: &JsonValue,
    ) -> Result<String, WebInspectorError> {
        Ok(self
            .evaluate_js_function(
                client,
                ELEMENT_TEXT_JS,
                std::slice::from_ref(element),
                false,
            )
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub async fn element_tag_name<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        element: &JsonValue,
    ) -> Result<String, WebInspectorError> {
        Ok(self
            .evaluate_js_function(client, ELEMENT_TAG_JS, std::slice::from_ref(element), false)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    async fn get_browsing_context<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<JsonValue, WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let response = self
            .send_command_and_wait(
                client,
                "getBrowsingContext",
                json!({ "handle": handle }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(response
            .get("context")
            .cloned()
            .unwrap_or(JsonValue::Object(Default::default())))
    }

    async fn find_elements_internal<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        by: By,
        value: &str,
        single: bool,
        root: Option<JsonValue>,
    ) -> Result<Vec<JsonValue>, WebInspectorError> {
        let (strategy, query) = normalized_locator(by, value);
        let response = self
            .evaluate_js_function(
                client,
                FIND_NODES_JS,
                &[
                    JsonValue::String(strategy),
                    root.unwrap_or(JsonValue::Null),
                    JsonValue::String(query),
                    JsonValue::Bool(single),
                    JsonValue::from(self.implicit_wait_timeout_ms),
                ],
                true,
            )
            .await?;

        Ok(match response {
            JsonValue::Null => Vec::new(),
            JsonValue::Array(values) => values,
            other => vec![other],
        })
    }

    async fn send_command_and_wait<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        method: &str,
        params: JsonValue,
        timeout_duration: Duration,
    ) -> Result<JsonValue, WebInspectorError> {
        let deadline = client.deadline(Some(timeout_duration))?;
        let mut operation = WebInspectorOperation::with_deadline(client, deadline)?;
        let mut matched = false;
        let result = await_operation(deadline, "execute WebInspector automation command", async {
            let command_id = self.send_command(operation.client, method, params).await?;
            operation.commit();
            self.wait_for_response(&mut operation, command_id, &mut matched)
                .await
        })
        .await;
        if matched {
            operation.finish_response(result)
        } else {
            operation.finish(result)
        }
    }

    async fn send_command<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        method: &str,
        params: JsonValue,
    ) -> Result<u64, WebInspectorError> {
        client.deadline(None)?;
        let page_id = self.page_id.ok_or_else(|| {
            WebInspectorError::Protocol("automation session has not attached to a page".to_string())
        })?;
        let command_id = admit_id(self.next_command_id)?;
        self.next_command_id = command_id.checked_add(1);
        client
            .send_socket_data(
                &self.session_id,
                &self.application_id,
                page_id,
                &json!({
                    "id": command_id,
                    "method": format!("Automation.{method}"),
                    "params": params,
                }),
            )
            .await?;
        Ok(command_id)
    }

    async fn wait_for_response<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        operation: &mut WebInspectorOperation<'_, S>,
        command_id: u64,
        matched: &mut bool,
    ) -> Result<JsonValue, WebInspectorError> {
        loop {
            let event = operation.client.next_socket_data().await?;
            if let WebInspectorEvent::SocketData { message, .. } = &event {
                if message.get("id").and_then(JsonValue::as_u64) == Some(command_id) {
                    *matched = true;
                    if let Some(error) = message.get("error") {
                        return Err(WebInspectorError::Protocol(format!(
                            "automation command failed: {error}"
                        )));
                    }
                    return Ok(message
                        .get("result")
                        .cloned()
                        .unwrap_or(JsonValue::Object(Default::default())));
                }
            }
            operation.preserve(event)?;
            tokio::task::yield_now().await;
        }
    }

    async fn wait_for_automation_page<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        client: &mut WebInspectorClient<S>,
        require_connection_id: bool,
    ) -> Result<Page, WebInspectorError> {
        loop {
            if let Some(page) =
                client.automation_page_by_session(&self.application_id, &self.session_id)
            {
                if !require_connection_id || page.automation_connection_id.is_some() {
                    return Ok(page.clone());
                }
            }
            let _ = client.next_event().await?;
            tokio::task::yield_now().await;
        }
    }

    fn require_top_level_handle(&self) -> Result<String, WebInspectorError> {
        self.top_level_handle.clone().ok_or_else(|| {
            WebInspectorError::Protocol(
                "automation session has not started a browsing context".to_string(),
            )
        })
    }
}

fn stringify_automation_argument(value: &JsonValue) -> Result<JsonValue, WebInspectorError> {
    Ok(JsonValue::String(serde_json::to_string(value)?))
}

fn decode_automation_result(value: &JsonValue) -> Result<JsonValue, WebInspectorError> {
    match value.get("result") {
        Some(JsonValue::String(result)) => Ok(serde_json::from_str(result)?),
        Some(other) => Ok(other.clone()),
        None => Ok(JsonValue::Null),
    }
}

fn normalized_locator(by: By, value: &str) -> (String, String) {
    match by {
        By::Id => ("css selector".to_string(), format!("[id=\"{value}\"]")),
        By::Name => ("css selector".to_string(), format!("[name=\"{value}\"]")),
        By::ClassName => ("css selector".to_string(), format!(".{value}")),
        By::TagName => ("css selector".to_string(), value.to_string()),
        _ => (by.as_wire().to_string(), value.to_string()),
    }
}

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn native_discovery_automation_write_deadlines_keep_elapsed() {
        for command in [false, true] {
            let (stream, _peer) = duplex(1);
            let mut client = WebInspectorClient::with_connection_id(stream, "owned-private");
            let mut session = AutomationSession::with_page(
                "PID:42",
                "com.apple.mobilesafari",
                "owned-private",
                1,
            );
            let duration = Duration::from_millis(5);
            let error = if command {
                session
                    .send_command_and_wait(&mut client, "Runtime.evaluate", json!({}), duration)
                    .await
                    .unwrap_err()
            } else {
                session.attach(&mut client, duration).await.unwrap_err()
            };
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
            assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
        }
    }

    #[test]
    fn normalized_locator_rewrites_common_dom_strategies() {
        assert_eq!(
            normalized_locator(By::ClassName, "hero"),
            ("css selector".into(), ".hero".into())
        );
        assert_eq!(
            normalized_locator(By::Id, "main"),
            ("css selector".into(), "[id=\"main\"]".into())
        );
        assert_eq!(
            normalized_locator(By::TagName, "button"),
            ("css selector".into(), "button".into())
        );
    }

    #[tokio::test]
    async fn automation_attach_timeout_covers_initial_writes() {
        // A one-byte duplex with no reader forces the first plist write to
        // remain pending. The public attach timeout must cover that write,
        // rather than only the later application-list wait.
        let (client_stream, _server_stream) = duplex(1);
        let mut client = WebInspectorClient::with_connection_id(client_stream, "TEST");
        let mut session =
            AutomationSession::with_session_id("PID:42", "com.apple.mobilesafari", "TEST-SESSION");

        let error = session
            .attach(&mut client, Duration::from_millis(10))
            .await
            .expect_err("a stalled setup write must time out");
        assert_eq!(error.timeout_duration(), Some(Duration::from_millis(10)));
        assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
    }
}
