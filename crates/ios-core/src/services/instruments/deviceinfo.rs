//! DeviceInfo service – query sysmon attributes for sysmontap configuration.

use tokio::io::{AsyncRead, AsyncWrite};

use crate::services::dtx::codec::{DtxConnection, DtxError};
use crate::services::dtx::types::{DtxPayload, NSObject};

/// Info about a running process from `runningProcesses`.
#[derive(Debug, Clone)]
pub struct RunningProcess {
    pub pid: u64,
    pub name: String,
    pub real_app_name: String,
    pub is_application: bool,
}

/// Fetch sysmon system/process attributes needed for sysmontap setConfig:.
pub struct DeviceInfoClient<S> {
    conn: DtxConnection<S>,
    channel_code: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> DeviceInfoClient<S> {
    pub async fn connect(stream: S) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let ch = conn.request_channel(super::DEVICE_INFO_SVC).await?;
        Ok(Self {
            conn,
            channel_code: ch,
        })
    }

    pub async fn system_attributes(&mut self) -> Result<Vec<plist::Value>, DtxError> {
        self.get_attrs("sysmonSystemAttributes").await
    }

    pub async fn process_attributes(&mut self) -> Result<Vec<plist::Value>, DtxError> {
        self.get_attrs("sysmonProcessAttributes").await
    }

    /// List all running processes on the device.
    pub async fn running_processes(&mut self) -> Result<Vec<RunningProcess>, DtxError> {
        let msg = self
            .conn
            .method_call(self.channel_code, "runningProcesses", &[])
            .await?;
        tracing::debug!("runningProcesses response: {:?}", msg.payload);

        let arr = match &msg.payload {
            DtxPayload::Response(NSObject::Array(a)) => a.clone(),
            DtxPayload::MethodInvocation { args, .. } => {
                // Some iOS versions return it as a method invocation arg
                args.iter()
                    .find_map(|a| {
                        if let NSObject::Array(arr) = a {
                            Some(arr.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default()
            }
            _ => return Ok(vec![]),
        };

        let mut result = Vec::with_capacity(arr.len());
        for item in &arr {
            if let NSObject::Dict(d) = item {
                let pid = match d.get("pid") {
                    Some(NSObject::Uint(n)) => *n,
                    Some(NSObject::Int(n)) => *n as u64,
                    _ => continue,
                };
                let name = d
                    .get("name")
                    .and_then(|v| {
                        if let NSObject::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                let real_app_name = d
                    .get("realAppName")
                    .and_then(|v| {
                        if let NSObject::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                let is_application = d
                    .get("isApplication")
                    .and_then(|v| {
                        if let NSObject::Bool(b) = v {
                            Some(*b)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(false);
                result.push(RunningProcess {
                    pid,
                    name,
                    real_app_name,
                    is_application,
                });
            }
        }
        Ok(result)
    }

    async fn get_attrs(&mut self, method: &str) -> Result<Vec<plist::Value>, DtxError> {
        let msg = self
            .conn
            .method_call(self.channel_code, method, &[])
            .await?;
        tracing::debug!("{method} response: {:?}", msg.payload);
        match &msg.payload {
            DtxPayload::Response(NSObject::Array(arr)) => Ok(arr
                .iter()
                .map(|v| match v {
                    NSObject::String(s) => plist::Value::String(s.clone()),
                    NSObject::Int(n) => plist::Value::Integer((*n).into()),
                    NSObject::Uint(n) => plist::Value::Integer((*n as i64).into()),
                    _ => plist::Value::String(format!("{v:?}")),
                })
                .collect()),
            _ => Ok(vec![]),
        }
    }
}
