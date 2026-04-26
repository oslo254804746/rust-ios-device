//! Application listing service.

use crate::proto::nskeyedarchiver_encode;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::services::dtx::codec::{DtxConnection, DtxError};
use crate::services::dtx::primitive_enc::archived_object;
use crate::services::dtx::types::{DtxPayload, NSObject};

pub struct ApplicationListingClient<S> {
    conn: DtxConnection<S>,
    channel_code: i32,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> ApplicationListingClient<S> {
    pub async fn connect(stream: S) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let channel_code = conn.request_channel(super::APP_LISTING_SVC).await?;
        Ok(Self { conn, channel_code })
    }

    pub async fn installed_applications(&mut self) -> Result<Vec<plist::Value>, DtxError> {
        let options = archived_object(nskeyedarchiver_encode::archive_dict(vec![]));
        let update_token = archived_object(nskeyedarchiver_encode::archive_string(""));
        let response = self
            .conn
            .method_call(
                self.channel_code,
                "installedApplicationsMatching:registerUpdateToken:",
                &[options, update_token],
            )
            .await?;
        parse_application_listing_response(&response.payload)
    }
}

fn parse_application_listing_response(payload: &DtxPayload) -> Result<Vec<plist::Value>, DtxError> {
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
            .ok_or_else(|| {
                DtxError::Protocol("application listing response missing array".into())
            })?,
        other => {
            return Err(DtxError::Protocol(format!(
                "unexpected application listing response: {other:?}"
            )))
        }
    };
    Ok(items.iter().map(nsobject_to_plist).collect())
}

fn nsobject_to_plist(value: &NSObject) -> plist::Value {
    match value {
        NSObject::Int(value) => plist::Value::Integer((*value).into()),
        NSObject::Uint(value) => plist::Value::Integer((*value as i64).into()),
        NSObject::Double(value) => plist::Value::Real(*value),
        NSObject::Bool(value) => plist::Value::Boolean(*value),
        NSObject::String(value) => plist::Value::String(value.clone()),
        NSObject::Data(value) => plist::Value::Data(value.to_vec()),
        NSObject::Array(values) => {
            plist::Value::Array(values.iter().map(nsobject_to_plist).collect())
        }
        NSObject::Dict(dict) => plist::Value::Dictionary(
            dict.iter()
                .map(|(key, value)| (key.clone(), nsobject_to_plist(value)))
                .collect(),
        ),
        NSObject::Null => plist::Value::String(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;

    #[test]
    fn parses_application_listing_array() {
        let payload = DtxPayload::Response(NSObject::Array(vec![NSObject::Dict(
            IndexMap::from_iter([
                (
                    "CFBundleIdentifier".to_string(),
                    NSObject::String("com.apple.Preferences".into()),
                ),
                ("Placeholder".to_string(), NSObject::Bool(false)),
            ]),
        )]));

        let apps = parse_application_listing_response(&payload).unwrap();
        assert_eq!(apps.len(), 1);
        let dict = apps[0].as_dictionary().unwrap();
        assert_eq!(
            dict["CFBundleIdentifier"].as_string(),
            Some("com.apple.Preferences")
        );
        assert_eq!(dict["Placeholder"].as_boolean(), Some(false));
    }
}
