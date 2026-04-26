use bytes::Bytes;
use plist::{Dictionary, Uid, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{timeout, Duration};

use super::{unarchive_raw_payload, MachTimeInfo, DEVICE_INFO_SVC};
use crate::services::dtx::codec::{DtxConnection, DtxError};
use crate::services::dtx::primitive_enc::archived_object;
use crate::services::dtx::types::{DtxPayload, NSObject};

pub const CORE_PROFILE_SESSION_SVC: &str =
    "com.apple.instruments.server.services.coreprofilesessiontap";

#[derive(Debug, Clone)]
pub struct CoreProfileConfig {
    pub update_rate: i64,
    pub recording_priority: i64,
    pub buffer_mode: Option<i64>,
    pub kind: i64,
    pub filters: Vec<u32>,
    pub callstack_depth: Option<i64>,
    pub actions: Option<Vec<Vec<i64>>>,
    pub uuid: String,
}

impl CoreProfileConfig {
    pub fn fps_defaults() -> Self {
        Self {
            update_rate: 500,
            recording_priority: 100,
            buffer_mode: Some(0),
            kind: 3,
            filters: vec![u32::MAX],
            callstack_depth: Some(128),
            actions: Some(vec![vec![3], vec![0], vec![2], vec![1, 1, 0]]),
            uuid: default_config_uuid(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreProfileEvent {
    Notice(NSObject),
    RawChunk(Bytes),
}

pub struct CoreProfileSessionClient<S> {
    conn: DtxConnection<S>,
    device_info_channel: i32,
    core_profile_channel: i32,
    mach_time_info: MachTimeInfo,
    pending_event: Option<CoreProfileEvent>,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> CoreProfileSessionClient<S> {
    pub async fn connect(stream: S) -> Result<Self, DtxError> {
        let mut conn = DtxConnection::new(stream);
        let device_info_channel = conn.request_channel(DEVICE_INFO_SVC).await?;
        let core_profile_channel = conn.request_channel(CORE_PROFILE_SESSION_SVC).await?;
        let mach_time_info = request_mach_time_info(&mut conn, device_info_channel).await?;

        Ok(Self {
            conn,
            device_info_channel,
            core_profile_channel,
            mach_time_info,
            pending_event: None,
        })
    }

    pub fn mach_time_info(&self) -> &MachTimeInfo {
        &self.mach_time_info
    }

    pub fn device_info_channel(&self) -> i32 {
        self.device_info_channel
    }

    pub fn core_profile_channel(&self) -> i32 {
        self.core_profile_channel
    }

    pub async fn start(&mut self, config: &CoreProfileConfig) -> Result<(), DtxError> {
        let archived = archive_core_profile_config(config);
        self.conn
            .method_call_async(
                self.core_profile_channel,
                "setConfig:",
                &[archived_object(archived)],
            )
            .await?;
        self.conn
            .method_call_async(self.core_profile_channel, "start", &[])
            .await?;

        match timeout(Duration::from_millis(500), self.recv_next_event()).await {
            Ok(Ok(event)) => {
                self.pending_event = Some(event);
                Ok(())
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Ok(()),
        }
    }

    pub async fn next_event(&mut self) -> Result<CoreProfileEvent, DtxError> {
        if let Some(event) = self.pending_event.take() {
            return Ok(event);
        }
        self.recv_next_event().await
    }

    pub async fn stop(&mut self) -> Result<(), DtxError> {
        self.conn
            .method_call_async(self.core_profile_channel, "stop", &[])
            .await
    }

    async fn recv_next_event(&mut self) -> Result<CoreProfileEvent, DtxError> {
        loop {
            let msg = self.conn.recv().await?;
            if msg.expects_reply {
                self.conn.send_ack(&msg).await?;
            }
            if !is_core_profile_live_channel(msg.channel_code, self.core_profile_channel) {
                continue;
            }
            if let Some(event) = decode_core_profile_payload(msg.payload)? {
                return Ok(event);
            }
        }
    }
}

async fn request_mach_time_info<S: AsyncRead + AsyncWrite + Unpin + Send>(
    conn: &mut DtxConnection<S>,
    channel_code: i32,
) -> Result<MachTimeInfo, DtxError> {
    let response = conn.method_call(channel_code, "machTimeInfo", &[]).await?;
    let values = match response.payload {
        DtxPayload::Response(NSObject::Array(values)) => values,
        DtxPayload::MethodInvocation { args, .. } => args
            .into_iter()
            .find_map(|arg| match arg {
                NSObject::Array(values) => Some(values),
                _ => None,
            })
            .ok_or_else(|| {
                DtxError::Protocol("machTimeInfo response did not contain an array".into())
            })?,
        other => {
            return Err(DtxError::Protocol(format!(
                "unexpected machTimeInfo payload: {other:?}"
            )))
        }
    };

    if values.len() < 3 {
        return Err(DtxError::Protocol(
            "machTimeInfo response did not contain three values".into(),
        ));
    }

    let numer = nsobject_as_u64(&values[1])
        .ok_or_else(|| DtxError::Protocol("machTimeInfo numer was not an integer".into()))?;
    let denom = nsobject_as_u64(&values[2])
        .ok_or_else(|| DtxError::Protocol("machTimeInfo denom was not an integer".into()))?;

    Ok(MachTimeInfo { numer, denom })
}

#[cfg(test)]
fn build_config_dict(config: &CoreProfileConfig) -> Vec<(String, plist::Value)> {
    let mut trigger = plist::dictionary::Dictionary::new();
    if let Some(callstack_depth) = config.callstack_depth {
        trigger.insert(
            "csd".to_string(),
            plist::Value::Integer(callstack_depth.into()),
        );
    }
    trigger.insert(
        "kdf2".to_string(),
        plist::Value::Array(
            config
                .filters
                .iter()
                .map(|value| plist::Value::Integer((*value as i64).into()))
                .collect(),
        ),
    );
    if let Some(actions) = &config.actions {
        trigger.insert(
            "ta".to_string(),
            plist::Value::Array(
                actions
                    .iter()
                    .map(|group| {
                        plist::Value::Array(
                            group
                                .iter()
                                .map(|value| plist::Value::Integer((*value).into()))
                                .collect(),
                        )
                    })
                    .collect(),
            ),
        );
    }
    trigger.insert("tk".to_string(), plist::Value::Integer(config.kind.into()));
    trigger.insert(
        "uuid".to_string(),
        plist::Value::String(config.uuid.clone()),
    );

    // pymobiledevice3's core profile tap config does not send a top-level `ur`.
    let mut dict = vec![(
        "tc".to_string(),
        plist::Value::Array(vec![plist::Value::Dictionary(trigger)]),
    )];
    dict.push((
        "rp".to_string(),
        plist::Value::Integer(config.recording_priority.into()),
    ));
    if let Some(buffer_mode) = config.buffer_mode {
        dict.push(("bm".to_string(), plist::Value::Integer(buffer_mode.into())));
    }

    dict
}

fn is_core_profile_live_channel(channel_code: i32, core_profile_channel: i32) -> bool {
    channel_code == core_profile_channel || channel_code == -1
}

#[derive(Clone)]
enum ConfigArchiveValue {
    Integer(i64),
    String(String),
    Array(Vec<ConfigArchiveValue>),
    Set(Vec<ConfigArchiveValue>),
    Dict(Vec<(String, ConfigArchiveValue)>),
}

fn archive_core_profile_config(config: &CoreProfileConfig) -> Vec<u8> {
    let mut trigger = Vec::new();
    if let Some(callstack_depth) = config.callstack_depth {
        trigger.push((
            "csd".to_string(),
            ConfigArchiveValue::Integer(callstack_depth),
        ));
    }
    trigger.push((
        "kdf2".to_string(),
        ConfigArchiveValue::Set(
            config
                .filters
                .iter()
                .map(|value| ConfigArchiveValue::Integer(*value as i64))
                .collect(),
        ),
    ));
    if let Some(actions) = &config.actions {
        trigger.push((
            "ta".to_string(),
            ConfigArchiveValue::Array(
                actions
                    .iter()
                    .map(|group| {
                        ConfigArchiveValue::Array(
                            group
                                .iter()
                                .map(|value| ConfigArchiveValue::Integer(*value))
                                .collect(),
                        )
                    })
                    .collect(),
            ),
        ));
    }
    trigger.push(("tk".to_string(), ConfigArchiveValue::Integer(config.kind)));
    trigger.push((
        "uuid".to_string(),
        ConfigArchiveValue::String(config.uuid.clone()),
    ));

    let mut root = vec![
        (
            "tc".to_string(),
            ConfigArchiveValue::Array(vec![ConfigArchiveValue::Dict(trigger)]),
        ),
        (
            "rp".to_string(),
            ConfigArchiveValue::Integer(config.recording_priority),
        ),
    ];
    if let Some(buffer_mode) = config.buffer_mode {
        root.push(("bm".to_string(), ConfigArchiveValue::Integer(buffer_mode)));
    }

    let mut objects = vec![Value::String("$null".to_string())];
    let root_uid = archive_config_value_into(ConfigArchiveValue::Dict(root), &mut objects);
    let root_doc = build_keyed_archive(root_uid, objects);

    let mut buf = Vec::new();
    plist::to_writer_binary(&mut buf, &root_doc)
        .expect("core profile config encoding must serialize");
    buf
}

fn archive_config_value_into(value: ConfigArchiveValue, objects: &mut Vec<Value>) -> Value {
    match value {
        ConfigArchiveValue::Integer(value) => push_object(Value::Integer(value.into()), objects),
        ConfigArchiveValue::String(value) => push_object(Value::String(value), objects),
        ConfigArchiveValue::Array(values) => archive_ns_collection_into(values, "NSArray", objects),
        ConfigArchiveValue::Set(values) => archive_ns_collection_into(values, "NSSet", objects),
        ConfigArchiveValue::Dict(pairs) => archive_ns_dict_into(&pairs, objects),
    }
}

fn archive_ns_collection_into(
    values: Vec<ConfigArchiveValue>,
    class_name: &str,
    objects: &mut Vec<Value>,
) -> Value {
    let item_uids: Vec<Value> = values
        .into_iter()
        .map(|value| archive_config_value_into(value, objects))
        .collect();

    let object_idx = objects.len();
    let class_idx = object_idx + 1;

    let mut object = Dictionary::new();
    object.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
    object.insert("NS.objects".to_string(), Value::Array(item_uids));
    objects.push(Value::Dictionary(object));
    objects.push(class_descriptor(class_name, &[class_name, "NSObject"]));

    Value::Uid(Uid::new(object_idx as u64))
}

fn archive_ns_dict_into(pairs: &[(String, ConfigArchiveValue)], objects: &mut Vec<Value>) -> Value {
    let object_idx = objects.len();
    objects.push(Value::Boolean(false));

    let mut key_uids = Vec::new();
    let mut value_uids = Vec::new();
    for (key, value) in pairs {
        key_uids.push(push_object(Value::String(key.clone()), objects));
        value_uids.push(archive_config_value_into(value.clone(), objects));
    }

    let class_idx = objects.len();
    objects.push(class_descriptor(
        "NSDictionary",
        &["NSDictionary", "NSObject"],
    ));

    let mut object = Dictionary::new();
    object.insert("$class".to_string(), Value::Uid(Uid::new(class_idx as u64)));
    object.insert("NS.keys".to_string(), Value::Array(key_uids));
    object.insert("NS.objects".to_string(), Value::Array(value_uids));
    objects[object_idx] = Value::Dictionary(object);

    Value::Uid(Uid::new(object_idx as u64))
}

fn push_object(value: Value, objects: &mut Vec<Value>) -> Value {
    let index = objects.len();
    objects.push(value);
    Value::Uid(Uid::new(index as u64))
}

fn class_descriptor(class_name: &str, classes: &[&str]) -> Value {
    let mut class = Dictionary::new();
    class.insert(
        "$classname".to_string(),
        Value::String(class_name.to_string()),
    );
    class.insert(
        "$classes".to_string(),
        Value::Array(
            classes
                .iter()
                .map(|name| Value::String((*name).to_string()))
                .collect(),
        ),
    );
    Value::Dictionary(class)
}

fn build_keyed_archive(root_uid: Value, objects: Vec<Value>) -> Value {
    let mut top = Dictionary::new();
    top.insert("root".to_string(), root_uid);

    let mut doc = Dictionary::new();
    doc.insert(
        "$archiver".to_string(),
        Value::String("NSKeyedArchiver".to_string()),
    );
    doc.insert("$version".to_string(), Value::Integer(100000.into()));
    doc.insert("$top".to_string(), Value::Dictionary(top));
    doc.insert("$objects".to_string(), Value::Array(objects));
    Value::Dictionary(doc)
}

fn decode_core_profile_payload(payload: DtxPayload) -> Result<Option<CoreProfileEvent>, DtxError> {
    match payload {
        DtxPayload::Empty => Ok(None),
        DtxPayload::Response(NSObject::Null) => Ok(None),
        DtxPayload::Response(object) => decode_core_profile_object(object),
        DtxPayload::Notification { object, .. } => decode_core_profile_object(object),
        DtxPayload::MethodInvocation { args, .. } => decode_core_profile_args(args),
        DtxPayload::Raw(bytes) => decode_core_profile_bytes(bytes),
        DtxPayload::RawWithAux { payload, aux } => {
            for arg in aux {
                if let Some(event) = decode_core_profile_object(arg)? {
                    return Ok(Some(event));
                }
            }
            if payload.is_empty() {
                Ok(None)
            } else {
                decode_core_profile_bytes(payload)
            }
        }
    }
}

fn decode_core_profile_args(args: Vec<NSObject>) -> Result<Option<CoreProfileEvent>, DtxError> {
    if let Some(bytes) = args.iter().find_map(nsobject_as_data_ref).cloned() {
        return decode_core_profile_bytes(bytes);
    }

    let object = if args.len() == 1 {
        args.into_iter().next().unwrap_or(NSObject::Null)
    } else {
        NSObject::Array(args)
    };
    decode_core_profile_object(object)
}

fn decode_core_profile_bytes(bytes: Bytes) -> Result<Option<CoreProfileEvent>, DtxError> {
    if let Some(object) = unarchive_raw_payload(&bytes) {
        decode_core_profile_object(object)
    } else {
        Ok(Some(CoreProfileEvent::RawChunk(bytes)))
    }
}

fn decode_core_profile_object(object: NSObject) -> Result<Option<CoreProfileEvent>, DtxError> {
    if let Some(bytes) = nsobject_as_data_ref(&object) {
        return decode_core_profile_bytes(bytes.clone());
    }

    if let NSObject::Dict(dict) = &object {
        if let Some(notice) = dict.get("notice").and_then(NSObject::as_str) {
            let status = dict
                .get("status")
                .and_then(nsobject_as_i64_ref)
                .unwrap_or_default();
            if status != 0 {
                return Err(DtxError::Protocol(notice.to_string()));
            }
        }
    }

    Ok(Some(CoreProfileEvent::Notice(object)))
}

fn nsobject_as_data_ref(value: &NSObject) -> Option<&Bytes> {
    match value {
        NSObject::Data(bytes) => Some(bytes),
        _ => None,
    }
}

fn nsobject_as_u64(value: &NSObject) -> Option<u64> {
    match value {
        NSObject::Uint(value) => Some(*value),
        NSObject::Int(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn nsobject_as_i64_ref(value: &NSObject) -> Option<i64> {
    match value {
        NSObject::Int(value) => Some(*value),
        NSObject::Uint(value) => Some(*value as i64),
        _ => None,
    }
}

fn default_config_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use crate::proto::nskeyedarchiver_encode;
    use plist::Value;
    use tokio::io::AsyncWriteExt;

    use super::*;
    use crate::services::dtx::codec::encode_dtx;
    use crate::services::dtx::{read_dtx_frame, DtxPayload};

    const MSG_RESPONSE: u32 = 3;
    const MSG_UNKNOWN_TYPE_ONE: u32 = 1;

    #[test]
    fn default_fps_config_matches_reference_shape() {
        let config = CoreProfileConfig::fps_defaults();
        let dict = build_config_dict(&config);
        assert!(uuid::Uuid::parse_str(&config.uuid).is_ok());

        assert!(dict.iter().any(|(key, value)| key == "rp"
            && matches!(value, Value::Integer(v) if v.as_signed() == Some(100))));
        assert!(dict
            .iter()
            .any(|(key, value)| key == "tc"
                && matches!(value, Value::Array(items) if items.len() == 1)));
        assert!(dict.iter().any(|(key, value)| key == "bm"
            && matches!(value, Value::Integer(v) if v.as_signed() == Some(0))));
        assert!(!dict.iter().any(|(key, _)| key == "ur"));

        let trigger = dict
            .iter()
            .find_map(|(key, value)| match (key.as_str(), value) {
                ("tc", Value::Array(items)) => match items.first() {
                    Some(Value::Dictionary(trigger)) => Some(trigger),
                    _ => None,
                },
                _ => None,
            })
            .expect("trigger config");

        assert!(matches!(
            trigger.get("csd"),
            Some(Value::Integer(value)) if value.as_signed() == Some(128)
        ));
        assert!(matches!(
            trigger.get("kdf2"),
            Some(Value::Array(values))
                if values.len() == 1
                    && matches!(values.first(), Some(Value::Integer(value)) if value.as_unsigned() == Some(u32::MAX as u64))
        ));
        assert!(matches!(
            trigger.get("ta"),
            Some(Value::Array(values))
                if values
                    == &vec![
                        Value::Array(vec![Value::Integer(3.into())]),
                        Value::Array(vec![Value::Integer(0.into())]),
                        Value::Array(vec![Value::Integer(2.into())]),
                        Value::Array(vec![
                            Value::Integer(1.into()),
                            Value::Integer(1.into()),
                            Value::Integer(0.into()),
                        ]),
                    ]
        ));
        assert!(matches!(
            trigger.get("tk"),
            Some(Value::Integer(value)) if value.as_signed() == Some(3)
        ));
        assert_eq!(
            trigger.get("uuid"),
            Some(&Value::String(config.uuid.clone()))
        );
    }

    #[test]
    fn archived_core_profile_config_encodes_kdf2_as_nsset() {
        let data = archive_core_profile_config(&CoreProfileConfig::fps_defaults());
        let plist: Value = plist::from_bytes(&data).expect("plist");
        let objects = plist
            .as_dictionary()
            .and_then(|dict| dict.get("$objects"))
            .and_then(Value::as_array)
            .expect("$objects");

        let kdf2_classname = objects.iter().find_map(|object| {
            let dict = object.as_dictionary()?;
            let class_ref = match dict.get("$class")? {
                Value::Uid(uid) => uid.get() as usize,
                _ => return None,
            };
            let keys = dict.get("NS.keys")?.as_array()?;
            let values = dict.get("NS.objects")?.as_array()?;
            let key_index = keys.iter().position(|value| {
                matches!(value, Value::Uid(uid) if objects[uid.get() as usize].as_string() == Some("kdf2"))
            })?;
            let set_ref = match values.get(key_index)? {
                Value::Uid(uid) => uid.get() as usize,
                _ => return None,
            };
            objects
                .get(set_ref)
                .and_then(Value::as_dictionary)
                .and_then(|set_object| match set_object.get("$class")? {
                    Value::Uid(uid) => objects.get(uid.get() as usize),
                    _ => None,
                })
                .and_then(Value::as_dictionary)
                .and_then(|class| class.get("$classname"))
                .and_then(Value::as_string)
                .map(str::to_string)
                .or_else(|| {
                    objects
                        .get(class_ref)
                        .and_then(Value::as_dictionary)
                        .and_then(|class| class.get("$classname"))
                        .and_then(Value::as_string)
                        .map(str::to_string)
                })
        });

        assert_eq!(kdf2_classname.as_deref(), Some("NSSet"));
    }

    #[test]
    fn detects_notice_errors_from_archived_raw_payload() {
        let payload = nskeyedarchiver_encode::archive_dict(vec![
            (
                "notice".to_string(),
                Value::String("kperf already owned".to_string()),
            ),
            ("status".to_string(), Value::Integer(1.into())),
        ]);

        let error = decode_core_profile_payload(DtxPayload::Raw(Bytes::from(payload)))
            .expect_err("notice payload should be rejected");
        assert!(error.to_string().contains("kperf already owned"));
    }

    #[test]
    fn treats_method_invocation_data_argument_as_raw_chunk() {
        let chunk = Bytes::from_static(&[1, 2, 3, 4]);

        let event = decode_core_profile_payload(DtxPayload::MethodInvocation {
            selector: String::new(),
            args: vec![NSObject::Data(chunk.clone())],
        })
        .expect("payload should decode")
        .expect("event should be emitted");

        assert_eq!(event, CoreProfileEvent::RawChunk(chunk));
    }

    #[test]
    fn accepts_core_profile_channel_and_broadcast_only() {
        assert!(is_core_profile_live_channel(2, 2));
        assert!(is_core_profile_live_channel(-1, 2));
        assert!(!is_core_profile_live_channel(1, 2));
        assert!(!is_core_profile_live_channel(99, 2));
    }

    #[tokio::test]
    async fn next_event_ignores_other_channels_and_accepts_broadcasts() {
        let (client, mut server) = tokio::io::duplex(4096);

        let task = tokio::spawn(async move {
            let mut session = CoreProfileSessionClient {
                conn: DtxConnection::new(client),
                device_info_channel: 1,
                core_profile_channel: 2,
                mach_time_info: MachTimeInfo { numer: 1, denom: 1 },
                pending_event: None,
            };
            session.next_event().await.unwrap()
        });

        let ignored_notice = nskeyedarchiver_encode::archive_string("device-info");
        server
            .write_all(&encode_dtx(
                10,
                0,
                -1,
                false,
                MSG_RESPONSE,
                &ignored_notice,
                &[],
            ))
            .await
            .unwrap();
        server
            .write_all(&encode_dtx(
                11,
                0,
                1,
                false,
                MSG_UNKNOWN_TYPE_ONE,
                b"broadcast-trace",
                &[],
            ))
            .await
            .unwrap();

        let event = task.await.unwrap();
        assert_eq!(
            event,
            CoreProfileEvent::RawChunk(Bytes::from_static(b"broadcast-trace"))
        );
    }

    #[tokio::test]
    async fn connect_start_and_stop_roundtrip() {
        let (client, mut server) = tokio::io::duplex(4096);

        let task = tokio::spawn(async move {
            let mut session = CoreProfileSessionClient::connect(client).await.unwrap();
            assert_eq!(session.mach_time_info().numer, 125);
            assert_eq!(session.mach_time_info().denom, 3);
            assert_eq!(session.device_info_channel(), 1);
            assert_eq!(session.core_profile_channel(), 2);
            session
                .start(&CoreProfileConfig::fps_defaults())
                .await
                .unwrap();
            session.stop().await.unwrap();
        });

        let req = read_dtx_frame(&mut server).await.unwrap();
        match req.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_requestChannelWithCode:identifier:");
                assert!(
                    matches!(args.get(1), Some(NSObject::String(name)) if name == DEVICE_INFO_SVC)
                );
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        server
            .write_all(&encode_dtx(
                req.identifier,
                1,
                0,
                false,
                MSG_RESPONSE,
                &[],
                &[],
            ))
            .await
            .unwrap();

        let req = read_dtx_frame(&mut server).await.unwrap();
        match req.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_requestChannelWithCode:identifier:");
                assert!(
                    matches!(args.get(1), Some(NSObject::String(name)) if name == CORE_PROFILE_SESSION_SVC)
                );
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        server
            .write_all(&encode_dtx(
                req.identifier,
                1,
                0,
                false,
                MSG_RESPONSE,
                &[],
                &[],
            ))
            .await
            .unwrap();

        let req = read_dtx_frame(&mut server).await.unwrap();
        match req.payload {
            DtxPayload::MethodInvocation { selector, .. } => assert_eq!(selector, "machTimeInfo"),
            other => panic!("unexpected payload: {other:?}"),
        }
        let mach_time_payload = nskeyedarchiver_encode::archive_array(vec![
            Value::Integer(0.into()),
            Value::Integer(125.into()),
            Value::Integer(3.into()),
        ]);
        server
            .write_all(&encode_dtx(
                req.identifier,
                1,
                1,
                false,
                MSG_RESPONSE,
                &mach_time_payload,
                &[],
            ))
            .await
            .unwrap();

        let req = read_dtx_frame(&mut server).await.unwrap();
        match req.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "setConfig:");
                assert!(matches!(
                    args.first(),
                    Some(NSObject::Dict(dict))
                        if !dict.contains_key("ur")
                            && dict.get("rp") == Some(&NSObject::Int(100))
                            && dict.get("bm") == Some(&NSObject::Int(0))
                            && matches!(
                                dict.get("tc"),
                                Some(NSObject::Array(items))
                                    if matches!(
                                        items.first(),
                                        Some(NSObject::Dict(trigger))
                                            if trigger.get("csd") == Some(&NSObject::Int(128))
                                                && trigger.get("kdf2") == Some(&NSObject::Array(vec![NSObject::Int(u32::MAX as i64)]))
                                                && trigger.get("ta") == Some(&NSObject::Array(vec![
                                                    NSObject::Array(vec![NSObject::Int(3)]),
                                                    NSObject::Array(vec![NSObject::Int(0)]),
                                                    NSObject::Array(vec![NSObject::Int(2)]),
                                                    NSObject::Array(vec![
                                                        NSObject::Int(1),
                                                        NSObject::Int(1),
                                                        NSObject::Int(0),
                                                    ]),
                                                ]))
                                                && trigger.get("tk") == Some(&NSObject::Int(3))
                                    )
                            )
                ));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        assert!(!req.expects_reply);

        let req = read_dtx_frame(&mut server).await.unwrap();
        match req.payload {
            DtxPayload::MethodInvocation { selector, .. } => assert_eq!(selector, "start"),
            other => panic!("unexpected payload: {other:?}"),
        }
        assert!(!req.expects_reply);
        let start_notice = nskeyedarchiver_encode::archive_dict(vec![
            (
                "notice".to_string(),
                Value::String("recording started".to_string()),
            ),
            ("status".to_string(), Value::Integer(0.into())),
        ]);
        server
            .write_all(&encode_dtx(
                99,
                0,
                -2,
                false,
                MSG_UNKNOWN_TYPE_ONE,
                &start_notice,
                &[],
            ))
            .await
            .unwrap();

        let req = read_dtx_frame(&mut server).await.unwrap();
        match req.payload {
            DtxPayload::MethodInvocation { selector, .. } => assert_eq!(selector, "stop"),
            other => panic!("unexpected payload: {other:?}"),
        }
        assert!(!req.expects_reply);

        task.await.unwrap();
    }
}
