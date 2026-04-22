use tokio::io::{AsyncRead, AsyncWrite};

use crate::dtx::codec::{DtxConnection, DtxError};
use crate::dtx::primitive_enc::archived_object;
use crate::dtx::types::{DtxMessage, DtxPayload, NSObject};

#[derive(Debug, Clone, PartialEq)]
pub struct NotificationEvent {
    pub selector: String,
    pub payload: NSObject,
    pub channel_code: i32,
}

pub struct NotificationClient<S> {
    conn: DtxConnection<S>,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> NotificationClient<S> {
    pub async fn connect(stream: S) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let channel_code = conn
            .request_channel(super::MOBILE_NOTIFICATIONS_SVC)
            .await?;

        conn.method_call(
            channel_code,
            "setApplicationStateNotificationsEnabled:",
            &[archived_object(
                ios_proto::nskeyedarchiver_encode::archive_bool(true),
            )],
        )
        .await?;
        conn.method_call(
            channel_code,
            "setMemoryNotificationsEnabled:",
            &[archived_object(
                ios_proto::nskeyedarchiver_encode::archive_bool(true),
            )],
        )
        .await?;

        Ok(Self { conn })
    }

    pub async fn next_notification(&mut self) -> Result<NotificationEvent, DtxError> {
        loop {
            let msg = self.conn.recv().await?;
            if msg.expects_reply {
                self.conn.send_ack(&msg).await?;
            }
            if let Some(event) = parse_notification_message(&msg) {
                return Ok(event);
            }
        }
    }
}

fn parse_notification_message(msg: &DtxMessage) -> Option<NotificationEvent> {
    let (selector, args) = match &msg.payload {
        DtxPayload::MethodInvocation { selector, args } => (selector, args),
        _ => return None,
    };

    if selector != "applicationStateNotification:" && selector != "memoryNotification:" {
        return None;
    }

    let payload = args.first().cloned().unwrap_or(NSObject::Null);
    Some(NotificationEvent {
        selector: selector.clone(),
        payload,
        channel_code: msg.channel_code,
    })
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;

    #[test]
    fn parses_application_state_notification_payload() {
        let msg = DtxMessage {
            identifier: 11,
            conversation_idx: 0,
            channel_code: 7,
            expects_reply: false,
            payload: DtxPayload::MethodInvocation {
                selector: "applicationStateNotification:".into(),
                args: vec![NSObject::Dict(IndexMap::from_iter([
                    (
                        "ApplicationBundleIdentifier".into(),
                        NSObject::String("com.apple.Preferences".into()),
                    ),
                    ("State".into(), NSObject::Int(8)),
                ]))],
            },
        };

        let event = parse_notification_message(&msg).expect("notification");
        assert_eq!(event.selector, "applicationStateNotification:");
        assert_eq!(event.channel_code, 7);
        match event.payload {
            NSObject::Dict(payload) => {
                assert_eq!(
                    payload.get("ApplicationBundleIdentifier"),
                    Some(&NSObject::String("com.apple.Preferences".into()))
                );
                assert_eq!(payload.get("State"), Some(&NSObject::Int(8)));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn ignores_non_notification_messages() {
        let msg = DtxMessage {
            identifier: 12,
            conversation_idx: 0,
            channel_code: 3,
            expects_reply: false,
            payload: DtxPayload::MethodInvocation {
                selector: "runningProcesses".into(),
                args: vec![],
            },
        };

        assert!(parse_notification_message(&msg).is_none());
    }
}
