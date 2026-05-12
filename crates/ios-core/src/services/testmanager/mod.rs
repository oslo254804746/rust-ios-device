//! Minimal XCTestManager/testmanagerd startup helpers.
//!
//! This covers the DTX protocol shared by the iOS 17+ Remote Service Discovery
//! path and older lockdown testmanager services. The CLI chooses the concrete
//! transport/service name for each iOS generation.

pub mod results;
pub mod workflow;
pub mod xctestrun;

use crate::proto::nskeyedarchiver_encode::{
    archive_uuid, archive_xct_capabilities, archive_xctest_configuration, XcTestConfiguration,
    XctCapabilities,
};
use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use results::TestExecutionEvent;

use crate::services::dtx::{
    archived_object, encode_dtx, DtxConnection, DtxError, DtxMessage, DtxPayload, NSObject, PrimArg,
};

pub const SERVICE_IOS17: &str = "com.apple.dt.testmanagerd.remote";
pub const SERVICE_IOS14: &str = "com.apple.testmanagerd.lockdown.secure";
pub const SERVICE_LEGACY: &str = "com.apple.testmanagerd.lockdown";
pub const SERVICE_NAME: &str = SERVICE_IOS17;
pub const DAEMON_CONNECTION_INTERFACE: &str =
    "dtxproxy:XCTestManager_IDEInterface:XCTestManager_DaemonConnectionInterface";
pub const DRIVER_INTERFACE: &str = "dtxproxy:XCTestDriverInterface:XCTestManager_IDEInterface";
pub const START_EXECUTING_SELECTOR: &str = "_IDE_startExecutingTestPlanWithProtocolVersion:";
pub const INITIATE_SESSION_SELECTOR: &str = "_IDE_initiateSessionWithIdentifier:capabilities:";
pub const INITIATE_CONTROL_SESSION_SELECTOR: &str = "_IDE_initiateControlSessionWithCapabilities:";
pub const AUTHORIZE_TEST_SESSION_SELECTOR: &str = "_IDE_authorizeTestSessionWithProcessID:";
pub const TEST_RUNNER_READY_SELECTOR: &str = "_XCT_testRunnerReadyWithCapabilities:";
pub const TEST_BUNDLE_READY_SELECTOR: &str =
    "_XCT_testBundleReadyWithProtocolVersion:minimumVersion:";
pub const REQUEST_CHANNEL_SELECTOR: &str = "_requestChannelWithCode:identifier:";
pub const PROTOCOL_VERSION: u32 = 36;
const MSG_RESPONSE: u32 = 3;

#[derive(Debug, Clone)]
pub enum StartupEvent {
    TestRunnerReady {
        message: DtxMessage,
    },
    TestBundleReady {
        message: DtxMessage,
        protocol_version: u64,
        minimum_version: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupSummary {
    pub protocol_version: u64,
    pub minimum_version: u64,
}

pub struct TestmanagerClient<S1, S2 = S1> {
    session: DtxConnection<S1>,
    session_channel: i32,
    driver_channel: Option<i32>,
    control: Option<(DtxConnection<S2>, i32)>,
}

impl<S1, S2> TestmanagerClient<S1, S2>
where
    S1: AsyncRead + AsyncWrite + Unpin + Send,
    S2: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub async fn connect(session_stream: S1, control_stream: S2) -> Result<Self, DtxError> {
        let mut session = DtxConnection::new(session_stream);
        let mut control = DtxConnection::new(control_stream);

        let session_channel = session.request_channel(DAEMON_CONNECTION_INTERFACE).await?;
        let control_channel = control.request_channel(DAEMON_CONNECTION_INTERFACE).await?;

        Ok(Self {
            session,
            session_channel,
            driver_channel: None,
            control: Some((control, control_channel)),
        })
    }

    pub async fn initiate_session(
        &mut self,
        session_identifier: Bytes,
        capabilities: Bytes,
    ) -> Result<DtxMessage, DtxError> {
        self.session
            .method_call(
                self.session_channel,
                INITIATE_SESSION_SELECTOR,
                &[
                    archived_object(session_identifier),
                    archived_object(capabilities),
                ],
            )
            .await
    }

    pub async fn initiate_control_session(
        &mut self,
        capabilities: Bytes,
    ) -> Result<DtxMessage, DtxError> {
        let (control, channel) = self.control_mut()?;
        control
            .method_call(
                channel,
                INITIATE_CONTROL_SESSION_SELECTOR,
                &[archived_object(capabilities)],
            )
            .await
    }

    pub async fn initiate_session_with_capabilities(
        &mut self,
        session_identifier: Uuid,
        capabilities: XctCapabilities,
    ) -> Result<DtxMessage, DtxError> {
        self.initiate_session(
            Bytes::from(archive_uuid(session_identifier)),
            Bytes::from(archive_xct_capabilities(capabilities)),
        )
        .await
    }

    pub async fn initiate_control_session_with_capabilities(
        &mut self,
        capabilities: XctCapabilities,
    ) -> Result<DtxMessage, DtxError> {
        self.initiate_control_session(Bytes::from(archive_xct_capabilities(capabilities)))
            .await
    }

    pub async fn authorize_test_session_with_process_id(
        &mut self,
        pid: u64,
    ) -> Result<bool, DtxError> {
        let (control, channel) = self.control_mut()?;
        let response = control
            .method_call(
                channel,
                AUTHORIZE_TEST_SESSION_SELECTOR,
                &[PrimArg::Int64(pid as i64)],
            )
            .await?;

        match response.payload {
            DtxPayload::Response(NSObject::Bool(authorized)) => Ok(authorized),
            other => Err(DtxError::Protocol(format!(
                "unexpected authorize test session response: {other:?}"
            ))),
        }
    }

    pub async fn request_driver_channel(&mut self) -> Result<i32, DtxError> {
        let channel = self.session.request_channel(DRIVER_INTERFACE).await?;
        self.driver_channel = Some(channel);
        Ok(channel)
    }

    pub async fn await_driver_channel_request(&mut self) -> Result<i32, DtxError> {
        loop {
            let msg = self.session.recv().await?;
            if let DtxPayload::MethodInvocation { selector, .. } = &msg.payload {
                if selector == REQUEST_CHANNEL_SELECTOR {
                    let (_requested_code, identifier) = decode_channel_request(&msg)?;
                    if msg.expects_reply {
                        self.session.send_ack(&msg).await?;
                    }

                    if identifier == DRIVER_INTERFACE {
                        // testmanagerd often requests code `1` here but then sends traffic on
                        // the default `-1` channel. Match go-ios' compatibility workaround.
                        self.driver_channel = Some(-1);
                        return Ok(-1);
                    }
                    continue;
                }
            }

            if msg.expects_reply {
                self.session.send_ack(&msg).await?;
            }
        }
    }

    pub async fn start_executing_test_plan(&mut self) -> Result<(), DtxError> {
        let channel = self.driver_channel.unwrap_or(-1);
        self.session
            .method_call_async(
                channel,
                START_EXECUTING_SELECTOR,
                &[PrimArg::Int64(PROTOCOL_VERSION as i64)],
            )
            .await
    }

    pub async fn recv_startup_event(&mut self) -> Result<StartupEvent, DtxError> {
        loop {
            let msg = self.session.recv().await?;
            if let DtxPayload::MethodInvocation { selector, .. } = &msg.payload {
                match selector.as_str() {
                    TEST_RUNNER_READY_SELECTOR => {
                        return Ok(StartupEvent::TestRunnerReady { message: msg });
                    }
                    TEST_BUNDLE_READY_SELECTOR => {
                        let (protocol_version, minimum_version) =
                            decode_test_bundle_ready_versions(&msg)?;
                        return Ok(StartupEvent::TestBundleReady {
                            message: msg,
                            protocol_version,
                            minimum_version,
                        });
                    }
                    _ => {}
                }
            }

            if msg.expects_reply {
                self.session.send_ack(&msg).await?;
            }
        }
    }

    pub async fn recv_execution_event(&mut self) -> Result<TestExecutionEvent, DtxError> {
        loop {
            let msg = self.session.recv().await?;
            let event = TestExecutionEvent::from_dtx_message(&msg);

            if msg.expects_reply {
                self.session.send_ack(&msg).await?;
            }

            if let Some(event) = event {
                return Ok(event);
            }
        }
    }

    pub async fn respond_test_runner_ready(
        &mut self,
        msg: &DtxMessage,
        configuration: Bytes,
    ) -> Result<(), DtxError> {
        let frame = encode_dtx(
            msg.identifier,
            msg.conversation_idx + 1,
            msg.channel_code,
            false,
            MSG_RESPONSE,
            &configuration,
            &[],
        );
        self.session.send_raw(&frame).await
    }

    pub async fn respond_test_runner_ready_with_configuration(
        &mut self,
        msg: &DtxMessage,
        configuration: XcTestConfiguration,
    ) -> Result<(), DtxError> {
        self.respond_test_runner_ready(
            msg,
            Bytes::from(archive_xctest_configuration(configuration)),
        )
        .await
    }

    pub async fn complete_startup_with_configuration(
        &mut self,
        configuration: XcTestConfiguration,
    ) -> Result<StartupSummary, DtxError> {
        let mut bundle_ready = None;
        let mut pending_configuration = Some(configuration);

        loop {
            match self.recv_startup_event().await? {
                StartupEvent::TestBundleReady {
                    protocol_version,
                    minimum_version,
                    ..
                } => {
                    bundle_ready = Some(StartupSummary {
                        protocol_version,
                        minimum_version,
                    });
                }
                StartupEvent::TestRunnerReady { message } => {
                    let configuration = pending_configuration.take().ok_or_else(|| {
                        DtxError::Protocol("test runner ready received more than once".into())
                    })?;
                    self.respond_test_runner_ready_with_configuration(&message, configuration)
                        .await?;

                    if let Some(summary) = bundle_ready {
                        return Ok(summary);
                    }
                }
            }
        }
    }

    pub async fn authorize_and_start_test_plan_with_configuration(
        &mut self,
        pid: u64,
        configuration: XcTestConfiguration,
    ) -> Result<StartupSummary, DtxError> {
        if !self.authorize_test_session_with_process_id(pid).await? {
            return Err(DtxError::Protocol(
                "testmanagerd rejected test session authorization".into(),
            ));
        }

        let mut bundle_ready = None;
        let mut pending_configuration = Some(configuration);
        let mut driver_ready = self.driver_channel.is_some();

        loop {
            let msg = self.session.recv().await?;
            if let DtxPayload::MethodInvocation { selector, .. } = &msg.payload {
                match selector.as_str() {
                    TEST_BUNDLE_READY_SELECTOR => {
                        let (protocol_version, minimum_version) =
                            decode_test_bundle_ready_versions(&msg)?;
                        bundle_ready = Some(StartupSummary {
                            protocol_version,
                            minimum_version,
                        });
                    }
                    TEST_RUNNER_READY_SELECTOR => {
                        let configuration = pending_configuration.take().ok_or_else(|| {
                            DtxError::Protocol(
                                "test runner ready received more than once during startup".into(),
                            )
                        })?;
                        self.respond_test_runner_ready_with_configuration(&msg, configuration)
                            .await?;
                    }
                    REQUEST_CHANNEL_SELECTOR => {
                        let (_requested_code, identifier) = decode_channel_request(&msg)?;
                        if msg.expects_reply {
                            self.session.send_ack(&msg).await?;
                        }
                        if identifier == DRIVER_INTERFACE {
                            self.driver_channel = Some(-1);
                            driver_ready = true;
                        }
                        if let Some(summary) = bundle_ready.filter(|_| driver_ready) {
                            if pending_configuration.is_none() {
                                // driver channel arrived last — all three conditions met, start now.
                                self.start_executing_test_plan().await?;
                                return Ok(summary);
                            }
                        }
                        continue;
                    }
                    _ => {}
                }
            }

            if msg.expects_reply
                && !matches!(
                    &msg.payload,
                    DtxPayload::MethodInvocation {
                        selector,
                        ..
                    } if selector == TEST_RUNNER_READY_SELECTOR
                )
            {
                self.session.send_ack(&msg).await?;
            }

            // Check again for non-REQUEST_CHANNEL messages (e.g. bundle ready arriving after
            // runner ready, when driver channel was already established beforehand).
            if let Some(summary) = bundle_ready.filter(|_| driver_ready) {
                if pending_configuration.is_none() {
                    self.start_executing_test_plan().await?;
                    return Ok(summary);
                }
            }
        }
    }

    fn control_mut(&mut self) -> Result<(&mut DtxConnection<S2>, i32), DtxError> {
        self.control
            .as_mut()
            .map(|(control, channel)| (control, *channel))
            .ok_or_else(|| DtxError::Protocol("control connection is not configured".into()))
    }
}

fn decode_test_bundle_ready_versions(msg: &DtxMessage) -> Result<(u64, u64), DtxError> {
    let DtxPayload::MethodInvocation { args, .. } = &msg.payload else {
        return Err(DtxError::Protocol(
            "test bundle ready event did not contain a method invocation".into(),
        ));
    };

    let protocol_version = args
        .first()
        .and_then(|value| value.as_int())
        .ok_or_else(|| DtxError::Protocol("missing test bundle protocol version".into()))?;
    let minimum_version = args
        .get(1)
        .and_then(|value| value.as_int())
        .ok_or_else(|| DtxError::Protocol("missing test bundle minimum version".into()))?;

    if protocol_version < 0 || minimum_version < 0 {
        return Err(DtxError::Protocol(
            "test bundle versions must be non-negative".into(),
        ));
    }

    Ok((protocol_version as u64, minimum_version as u64))
}

fn decode_channel_request(msg: &DtxMessage) -> Result<(i32, String), DtxError> {
    let DtxPayload::MethodInvocation { args, .. } = &msg.payload else {
        return Err(DtxError::Protocol(
            "channel request did not contain a method invocation".into(),
        ));
    };

    let requested_code = args
        .first()
        .and_then(|value| value.as_int())
        .ok_or_else(|| DtxError::Protocol("missing requested channel code".into()))?;
    let identifier = args
        .get(1)
        .and_then(|value| value.as_str())
        .ok_or_else(|| DtxError::Protocol("missing requested channel identifier".into()))?;

    if requested_code < i32::MIN as i64 || requested_code > i32::MAX as i64 {
        return Err(DtxError::Protocol(
            "requested channel code out of range".into(),
        ));
    }

    Ok((requested_code as i32, identifier.to_string()))
}

impl<S> TestmanagerClient<S, S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    /// Test-only constructor: single session connection, no control connection.
    #[cfg(feature = "testmanager")]
    pub fn from_session_connection_for_test(session_stream: S, session_channel: i32) -> Self {
        Self {
            session: DtxConnection::new(session_stream),
            session_channel,
            driver_channel: None,
            control: None,
        }
    }

    /// Test-only constructor: both session and control connections.
    #[cfg(feature = "testmanager")]
    pub fn from_connections_for_test(
        session_stream: S,
        session_channel: i32,
        control_stream: S,
        control_channel: i32,
    ) -> Self {
        Self {
            session: DtxConnection::new(session_stream),
            session_channel,
            driver_channel: None,
            control: Some((DtxConnection::new(control_stream), control_channel)),
        }
    }
}
