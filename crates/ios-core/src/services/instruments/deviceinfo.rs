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
                    .ok_or_else(|| {
                        DtxError::Protocol(
                            "runningProcesses: expected an array payload argument".into(),
                        )
                    })?
            }
            _ => {
                return Err(DtxError::Protocol(
                    "runningProcesses: expected array payload".into(),
                ));
            }
        };

        parse_running_processes(&arr)
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

/// Decode the required process dictionary fields. The go-ios DeviceInfo
/// decoder rejects malformed entries instead of silently dropping them or
/// treating a missing name as an empty process; doing the same prevents
/// `memlimitoff --process` from acting on an incomplete response.
fn parse_running_processes(items: &[NSObject]) -> Result<Vec<RunningProcess>, DtxError> {
    let mut result = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let dictionary = match item {
            NSObject::Dict(dictionary) => dictionary,
            _ => {
                return Err(DtxError::Protocol(format!(
                    "runningProcesses: entry {index} is not a dictionary"
                )))
            }
        };
        let pid = match dictionary.get("pid") {
            Some(NSObject::Uint(pid)) => *pid,
            Some(NSObject::Int(pid)) if *pid >= 0 => *pid as u64,
            Some(_) => {
                return Err(DtxError::Protocol(format!(
                    "runningProcesses: entry {index} has invalid pid"
                )))
            }
            None => {
                return Err(DtxError::Protocol(format!(
                    "runningProcesses: entry {index} is missing pid"
                )))
            }
        };
        let name = required_process_string(dictionary, "name", index)?;
        let real_app_name = required_process_string(dictionary, "realAppName", index)?;
        let is_application = match dictionary.get("isApplication") {
            Some(NSObject::Bool(value)) => *value,
            Some(_) => {
                return Err(DtxError::Protocol(format!(
                    "runningProcesses: entry {index} has invalid isApplication"
                )))
            }
            None => {
                return Err(DtxError::Protocol(format!(
                    "runningProcesses: entry {index} is missing isApplication"
                )))
            }
        };
        result.push(RunningProcess {
            pid,
            name,
            real_app_name,
            is_application,
        });
    }
    Ok(result)
}

fn required_process_string(
    dictionary: &indexmap::IndexMap<String, NSObject>,
    key: &str,
    index: usize,
) -> Result<String, DtxError> {
    match dictionary.get(key) {
        Some(NSObject::String(value)) => Ok(value.clone()),
        Some(_) => Err(DtxError::Protocol(format!(
            "runningProcesses: entry {index} has invalid {key}"
        ))),
        None => Err(DtxError::Protocol(format!(
            "runningProcesses: entry {index} is missing {key}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(fields: impl IntoIterator<Item = (&'static str, NSObject)>) -> NSObject {
        NSObject::Dict(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    #[test]
    fn process_decoder_preserves_required_fields() {
        let item = process([
            ("pid", NSObject::Uint(42)),
            ("name", NSObject::String("Safari".into())),
            ("realAppName", NSObject::String("MobileSafari".into())),
            ("isApplication", NSObject::Bool(true)),
        ]);
        let result = parse_running_processes(&[item]).unwrap();
        assert_eq!(result[0].pid, 42);
        assert_eq!(result[0].name, "Safari");
        assert_eq!(result[0].real_app_name, "MobileSafari");
        assert!(result[0].is_application);
    }

    #[test]
    fn process_decoder_rejects_missing_or_wrong_name_fields() {
        let missing_name = process([
            ("pid", NSObject::Uint(42)),
            ("realAppName", NSObject::String("MobileSafari".into())),
            ("isApplication", NSObject::Bool(true)),
        ]);
        let error = parse_running_processes(&[missing_name]).unwrap_err();
        assert!(error.to_string().contains("missing name"));

        let wrong_name = process([
            ("pid", NSObject::Uint(42)),
            ("name", NSObject::Uint(7)),
            ("realAppName", NSObject::String("MobileSafari".into())),
            ("isApplication", NSObject::Bool(true)),
        ]);
        let error = parse_running_processes(&[wrong_name]).unwrap_err();
        assert!(error.to_string().contains("invalid name"));
    }

    #[test]
    fn process_decoder_rejects_negative_pid_and_non_dictionary_entries() {
        let error = parse_running_processes(&[process([
            ("pid", NSObject::Int(-1)),
            ("name", NSObject::String("bad".into())),
            ("realAppName", NSObject::String("bad".into())),
            ("isApplication", NSObject::Bool(false)),
        ])])
        .unwrap_err();
        assert!(error.to_string().contains("invalid pid"));

        let error = parse_running_processes(&[NSObject::String("bad".into())]).unwrap_err();
        assert!(error.to_string().contains("not a dictionary"));
    }
}
