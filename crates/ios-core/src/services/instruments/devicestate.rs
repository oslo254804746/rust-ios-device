//! Device state / condition inducer service.

use crate::proto::nskeyedarchiver_encode;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::services::dtx::codec::{DtxConnection, DtxError};
use crate::services::dtx::primitive_enc::archived_object;
use crate::services::dtx::types::{DtxPayload, NSObject};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConditionProfile {
    pub description: String,
    pub identifier: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConditionProfileType {
    pub active_profile: Option<String>,
    pub identifier: String,
    pub is_active: bool,
    pub is_destructive: bool,
    pub is_internal: bool,
    pub name: String,
    pub profiles_sorted: bool,
    pub profiles: Vec<ConditionProfile>,
}

pub struct DeviceStateClient<S> {
    conn: DtxConnection<S>,
    channel_code: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> DeviceStateClient<S> {
    pub async fn connect(stream: S) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let channel_code = conn.request_channel(super::CONDITION_INDUCER_SVC).await?;
        Ok(Self { conn, channel_code })
    }

    pub async fn list(&mut self) -> Result<Vec<ConditionProfileType>, DtxError> {
        let msg = self
            .conn
            .method_call(self.channel_code, "availableConditionInducers", &[])
            .await?;
        parse_condition_profile_types(&msg.payload)
    }

    pub async fn enable(
        &mut self,
        profile_type_id: &str,
        profile_id: &str,
    ) -> Result<bool, DtxError> {
        let response = self
            .conn
            .method_call(
                self.channel_code,
                "enableConditionWithIdentifier:profileIdentifier:",
                &[
                    archived_object(nskeyedarchiver_encode::archive_string(profile_type_id)),
                    archived_object(nskeyedarchiver_encode::archive_string(profile_id)),
                ],
            )
            .await?;
        match response.payload {
            DtxPayload::Response(NSObject::Bool(value)) => Ok(value),
            DtxPayload::Response(NSObject::Int(value)) => Ok(value != 0),
            DtxPayload::Response(NSObject::Uint(value)) => Ok(value != 0),
            other => Err(DtxError::Protocol(format!(
                "unexpected device state enable response: {other:?}"
            ))),
        }
    }

    pub async fn disable(&mut self) -> Result<(), DtxError> {
        self.conn
            .method_call_async(self.channel_code, "disableActiveCondition", &[])
            .await
    }
}

fn parse_condition_profile_types(
    payload: &DtxPayload,
) -> Result<Vec<ConditionProfileType>, DtxError> {
    let items = match payload {
        DtxPayload::Response(NSObject::Array(items)) => items,
        DtxPayload::MethodInvocation { args, .. } => args
            .iter()
            .find_map(|arg| {
                if let NSObject::Array(items) = arg {
                    Some(items)
                } else {
                    None
                }
            })
            .ok_or_else(|| DtxError::Protocol("condition inducer response missing array".into()))?,
        other => {
            return Err(DtxError::Protocol(format!(
                "unexpected condition inducer response: {other:?}"
            )))
        }
    };

    items.iter().map(parse_profile_type).collect()
}

fn parse_profile_type(item: &NSObject) -> Result<ConditionProfileType, DtxError> {
    let dict = match item {
        NSObject::Dict(dict) => dict,
        other => {
            return Err(DtxError::Protocol(format!(
                "condition profile type was not a dictionary: {other:?}"
            )))
        }
    };

    let profiles = match dict.get("profiles") {
        Some(NSObject::Array(profiles)) => profiles
            .iter()
            .map(parse_profile)
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };

    Ok(ConditionProfileType {
        active_profile: get_string(dict, "activeProfile"),
        identifier: require_string(dict, "identifier")?,
        is_active: get_bool(dict, "isActive"),
        is_destructive: get_bool(dict, "isDestructive"),
        is_internal: get_bool(dict, "isInternal"),
        name: require_string(dict, "name")?,
        profiles_sorted: get_bool(dict, "profilesSorted"),
        profiles,
    })
}

fn parse_profile(item: &NSObject) -> Result<ConditionProfile, DtxError> {
    let dict = match item {
        NSObject::Dict(dict) => dict,
        other => {
            return Err(DtxError::Protocol(format!(
                "condition profile was not a dictionary: {other:?}"
            )))
        }
    };
    Ok(ConditionProfile {
        description: require_string(dict, "description")?,
        identifier: require_string(dict, "identifier")?,
        name: require_string(dict, "name")?,
    })
}

fn get_string(dict: &indexmap::IndexMap<String, NSObject>, key: &str) -> Option<String> {
    dict.get(key).and_then(|value| match value {
        NSObject::String(value) => Some(value.clone()),
        _ => None,
    })
}

fn require_string(
    dict: &indexmap::IndexMap<String, NSObject>,
    key: &str,
) -> Result<String, DtxError> {
    get_string(dict, key).ok_or_else(|| DtxError::Protocol(format!("missing string key '{key}'")))
}

fn get_bool(dict: &indexmap::IndexMap<String, NSObject>, key: &str) -> bool {
    dict.get(key)
        .and_then(|value| match value {
            NSObject::Bool(value) => Some(*value),
            _ => None,
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;

    #[test]
    fn parses_condition_profile_types_from_response_array() {
        let payload = DtxPayload::Response(NSObject::Array(vec![NSObject::Dict(
            IndexMap::from_iter([
                (
                    "activeProfile".to_string(),
                    NSObject::String("SlowNetwork3GGood".into()),
                ),
                (
                    "identifier".to_string(),
                    NSObject::String("SlowNetworkCondition".into()),
                ),
                ("isActive".to_string(), NSObject::Bool(true)),
                ("isDestructive".to_string(), NSObject::Bool(false)),
                ("isInternal".to_string(), NSObject::Bool(false)),
                ("name".to_string(), NSObject::String("Slow Network".into())),
                ("profilesSorted".to_string(), NSObject::Bool(true)),
                (
                    "profiles".to_string(),
                    NSObject::Array(vec![NSObject::Dict(IndexMap::from_iter([
                        (
                            "description".to_string(),
                            NSObject::String("3G good".into()),
                        ),
                        (
                            "identifier".to_string(),
                            NSObject::String("SlowNetwork3GGood".into()),
                        ),
                        ("name".to_string(), NSObject::String("3G".into())),
                    ]))]),
                ),
            ]),
        )]));

        let parsed = parse_condition_profile_types(&payload).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].identifier, "SlowNetworkCondition");
        assert_eq!(parsed[0].profiles[0].identifier, "SlowNetwork3GGood");
    }
}
