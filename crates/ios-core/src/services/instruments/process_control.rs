//! Process control service – list running processes, launch/kill apps.
//!
//! Service: `com.apple.instruments.server.services.processcontrol`
//! Reference: go-ios/ios/instruments/processcontrol.go

use std::collections::HashMap;

use crate::proto::nskeyedarchiver_encode;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::services::dtx::codec::{DtxConnection, DtxError};
use crate::services::dtx::primitive_enc::archived_object;
use crate::services::dtx::types::{DtxPayload, NSObject};

/// Info about a running process.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u64,
    pub name: String,
    pub real_app_name: String,
    pub is_application: bool,
}

/// Process control service client.
/// Each instance owns its own DTX connection and channel.
pub struct ProcessControl<S> {
    conn: DtxConnection<S>,
    channel_code: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> ProcessControl<S> {
    /// Connect to the process control service.
    pub async fn connect(stream: S) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let ch = conn.request_channel(super::PROCESS_CTRL_SVC).await?;
        Ok(Self {
            conn,
            channel_code: ch,
        })
    }

    /// Launch an app by bundle ID; returns its PID.
    pub async fn launch(
        &mut self,
        bundle_id: &str,
        args: &[&str],
        env: &HashMap<String, String>,
    ) -> Result<u64, DtxError> {
        self.launch_with_options(
            bundle_id,
            args,
            env,
            &[
                (
                    "StartSuspendedKey".to_string(),
                    plist::Value::Boolean(false),
                ),
                ("KillExisting".to_string(), plist::Value::Boolean(false)),
            ],
        )
        .await
    }

    /// Launch an app by bundle ID with explicit process-control options; returns its PID.
    pub async fn launch_with_options(
        &mut self,
        bundle_id: &str,
        args: &[&str],
        env: &HashMap<String, String>,
        options: &[(String, plist::Value)],
    ) -> Result<u64, DtxError> {
        // go-ios: path, bundleID, env, args, opts
        let path_enc = archived_object(nskeyedarchiver_encode::archive_string("/private/"));
        let bid_enc = archived_object(nskeyedarchiver_encode::archive_string(bundle_id));

        // env: merge NSUnbufferedIO=YES with caller-supplied env
        let mut full_env: Vec<(String, plist::Value)> = vec![(
            "NSUnbufferedIO".to_string(),
            plist::Value::String("YES".to_string()),
        )];
        for (k, v) in env {
            full_env.push((k.clone(), plist::Value::String(v.clone())));
        }
        let env_enc = archived_object(nskeyedarchiver_encode::archive_dict(full_env));

        let args_enc = archived_object(nskeyedarchiver_encode::archive_array(
            args.iter()
                .map(|s| plist::Value::String(s.to_string()))
                .collect(),
        ));

        let opts_enc = archived_object(nskeyedarchiver_encode::archive_dict(options.to_vec()));

        let msg = self.conn.method_call(
            self.channel_code,
            "launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:",
            &[path_enc, bid_enc, env_enc, args_enc, opts_enc],
        ).await?;

        if let DtxPayload::Response(NSObject::Int(pid)) = msg.payload {
            return Ok(pid as u64);
        }
        if let DtxPayload::Response(NSObject::Uint(pid)) = msg.payload {
            return Ok(pid);
        }
        Err(DtxError::Protocol(format!(
            "unexpected launch response: {:?}",
            msg.payload
        )))
    }

    /// Send SIGKILL to a process.
    pub async fn kill(&mut self, pid: u64) -> Result<(), DtxError> {
        let pid_enc = archived_object(nskeyedarchiver_encode::archive_int(pid as i64));
        self.conn
            .method_call_async(self.channel_code, "killPid:", &[pid_enc])
            .await
    }

    /// Disable the jetsam memory limit for a process.
    pub async fn disable_memory_limit(&mut self, pid: u64) -> Result<bool, DtxError> {
        let msg = self
            .conn
            .method_call(
                self.channel_code,
                "requestDisableMemoryLimitsForPid:",
                &[crate::services::dtx::primitive_enc::PrimArg::Int32(
                    pid as i32,
                )],
            )
            .await?;

        match msg.payload {
            DtxPayload::Response(NSObject::Bool(disabled)) => Ok(disabled),
            DtxPayload::Response(NSObject::Int(disabled)) => Ok(disabled != 0),
            DtxPayload::Response(NSObject::Uint(disabled)) => Ok(disabled != 0),
            other => Err(DtxError::Protocol(format!(
                "unexpected disableMemoryLimit response: {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use plist::Value;
    use tokio::io::{duplex, AsyncWriteExt};

    use super::*;
    use crate::services::dtx::{encode_dtx, read_dtx_frame, DtxPayload, NSObject};

    const MSG_RESPONSE: u32 = 3;

    #[tokio::test]
    async fn launch_with_options_sends_expected_archived_arguments() {
        let (client, mut server) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut env = HashMap::new();
            env.insert("TERM".to_string(), "xterm-256color".to_string());

            let mut client = ProcessControl::connect(client).await.unwrap();
            client
                .launch_with_options(
                    "com.example.demo",
                    &["--flag"],
                    &env,
                    &[("KillExisting".to_string(), Value::Boolean(true))],
                )
                .await
                .unwrap()
        });

        let channel_request = read_dtx_frame(&mut server).await.unwrap();
        match channel_request.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_requestChannelWithCode:identifier:");
                assert!(
                    matches!(args.get(1), Some(NSObject::String(name)) if name == super::super::PROCESS_CTRL_SVC)
                );
            }
            other => panic!("unexpected channel request: {other:?}"),
        }
        server
            .write_all(&encode_dtx(
                channel_request.identifier,
                1,
                0,
                false,
                MSG_RESPONSE,
                &[],
                &[],
            ))
            .await
            .unwrap();

        let launch = read_dtx_frame(&mut server).await.unwrap();
        match launch.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(
                    selector,
                    "launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:"
                );
                assert!(
                    matches!(args.first(), Some(NSObject::String(path)) if path == "/private/")
                );
                assert!(
                    matches!(args.get(1), Some(NSObject::String(bundle)) if bundle == "com.example.demo")
                );
                match args.get(2) {
                    Some(NSObject::Dict(env)) => {
                        assert_eq!(
                            env.get("NSUnbufferedIO"),
                            Some(&NSObject::String("YES".into()))
                        );
                        assert_eq!(
                            env.get("TERM"),
                            Some(&NSObject::String("xterm-256color".into()))
                        );
                    }
                    other => panic!("unexpected env payload: {other:?}"),
                }
                assert!(matches!(
                    args.get(3),
                    Some(NSObject::Array(values))
                    if values == &vec![NSObject::String("--flag".into())]
                ));
                assert!(matches!(
                    args.get(4),
                    Some(NSObject::Dict(options))
                    if options.get("KillExisting") == Some(&NSObject::Bool(true))
                ));
            }
            other => panic!("unexpected launch request: {other:?}"),
        }
        server
            .write_all(&encode_dtx(
                launch.identifier,
                1,
                launch.channel_code,
                false,
                MSG_RESPONSE,
                &crate::proto::nskeyedarchiver_encode::archive_int(4242),
                &[],
            ))
            .await
            .unwrap();

        assert_eq!(task.await.unwrap(), 4242);
    }

    #[tokio::test]
    async fn disable_memory_limit_treats_integer_response_as_boolean() {
        let (client, mut server) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = ProcessControl::connect(client).await.unwrap();
            client.disable_memory_limit(99).await.unwrap()
        });

        let channel_request = read_dtx_frame(&mut server).await.unwrap();
        server
            .write_all(&encode_dtx(
                channel_request.identifier,
                1,
                0,
                false,
                MSG_RESPONSE,
                &[],
                &[],
            ))
            .await
            .unwrap();

        let request = read_dtx_frame(&mut server).await.unwrap();
        match request.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "requestDisableMemoryLimitsForPid:");
                assert!(matches!(args.first(), Some(NSObject::Int(99))));
            }
            other => panic!("unexpected disable request: {other:?}"),
        }
        server
            .write_all(&encode_dtx(
                request.identifier,
                1,
                request.channel_code,
                false,
                MSG_RESPONSE,
                &crate::proto::nskeyedarchiver_encode::archive_int(1),
                &[],
            ))
            .await
            .unwrap();

        assert!(task.await.unwrap());
    }
}
