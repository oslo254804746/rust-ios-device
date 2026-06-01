use bytes::{BufMut, Bytes, BytesMut};

use super::ValeriaError;

const EMPTY_CLOCK_REF: u64 = 1;
const MAGIC_DICT: &[u8; 4] = b"tcid";
const MAGIC_KEYV: &[u8; 4] = b"vyek";
const MAGIC_STRK: &[u8; 4] = b"krts";
const MAGIC_BULV: &[u8; 4] = b"vlub";
const MAGIC_STRV: &[u8; 4] = b"vrts";
const MAGIC_DATV: &[u8; 4] = b"vtad";
const MAGIC_NMBV: &[u8; 4] = b"vbmn";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tag {
    Ping,
    Sync,
    Reply,
    Asyn,
    Cwpa,
    Afmt,
    Cvrp,
    Clok,
    Time,
    Skew,
    Og,
    Stop,
    Hpd1,
    Hpa1,
    Hpd0,
    Hpa0,
    Need,
    Feed,
    Eat,
    Sprp,
    Srat,
    Tbas,
    Tjmp,
    Rels,
}

impl Tag {
    pub(crate) fn bytes(self) -> &'static [u8; 4] {
        match self {
            Self::Ping => b"gnip",
            Self::Sync => b"cnys",
            Self::Reply => b"ylpr",
            Self::Asyn => b"nysa",
            Self::Cwpa => b"apwc",
            Self::Afmt => b"tmfa",
            Self::Cvrp => b"prvc",
            Self::Clok => b"kolc",
            Self::Time => b"emit",
            Self::Skew => b"weks",
            Self::Og => b" !og",
            Self::Stop => b"pots",
            Self::Hpd1 => b"1dph",
            Self::Hpa1 => b"1aph",
            Self::Hpd0 => b"0dph",
            Self::Hpa0 => b"0aph",
            Self::Need => b"deen",
            Self::Feed => b"deef",
            Self::Eat => b"!tae",
            Self::Sprp => b"prps",
            Self::Srat => b"tars",
            Self::Tbas => b"sabt",
            Self::Tjmp => b"pmjt",
            Self::Rels => b"sler",
        }
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Some(match bytes {
            b"gnip" => Self::Ping,
            b"cnys" => Self::Sync,
            b"ylpr" => Self::Reply,
            b"nysa" => Self::Asyn,
            b"apwc" => Self::Cwpa,
            b"tmfa" => Self::Afmt,
            b"prvc" => Self::Cvrp,
            b"kolc" => Self::Clok,
            b"emit" => Self::Time,
            b"weks" => Self::Skew,
            b" !og" => Self::Og,
            b"pots" => Self::Stop,
            b"1dph" => Self::Hpd1,
            b"1aph" => Self::Hpa1,
            b"0dph" => Self::Hpd0,
            b"0aph" => Self::Hpa0,
            b"deen" => Self::Need,
            b"deef" => Self::Feed,
            b"!tae" => Self::Eat,
            b"prps" => Self::Sprp,
            b"tars" => Self::Srat,
            b"sabt" => Self::Tbas,
            b"pmjt" => Self::Tjmp,
            b"sler" => Self::Rels,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Packet {
    pub(crate) tag: Tag,
    pub(crate) payload: Bytes,
}

pub(crate) fn decode_packet(bytes: &[u8]) -> Result<Packet, ValeriaError> {
    if bytes.len() < 8 {
        return Err(ValeriaError::Protocol("packet shorter than header".into()));
    }
    let total_len =
        u32::from_le_bytes(bytes[0..4].try_into().expect("header length checked")) as usize;
    if total_len != bytes.len() {
        return Err(ValeriaError::Protocol(format!(
            "packet length {total_len} does not match {} bytes",
            bytes.len()
        )));
    }
    let tag = Tag::from_bytes(&bytes[4..8]).ok_or_else(|| {
        ValeriaError::Protocol(format!("unknown packet tag {:02x?}", &bytes[4..8]))
    })?;
    Ok(Packet {
        tag,
        payload: Bytes::copy_from_slice(&bytes[8..]),
    })
}

pub(crate) fn encode_packet(tag: Tag, payload: &[u8]) -> Bytes {
    let total_len = 8 + payload.len();
    let mut out = BytesMut::with_capacity(total_len);
    out.put_u32_le(total_len as u32);
    out.put_slice(tag.bytes());
    out.put_slice(payload);
    out.freeze()
}

pub(crate) fn encode_ping() -> Bytes {
    encode_packet(Tag::Ping, &[0, 0, 0, 0, 1, 0, 0, 0])
}

pub(crate) fn encode_asyn_simple(tag: Tag, clock_ref: u64) -> Result<Bytes, ValeriaError> {
    match tag {
        Tag::Need | Tag::Hpd0 | Tag::Hpa0 => {
            let mut payload = BytesMut::with_capacity(12);
            payload.put_u64_le(clock_ref);
            payload.put_slice(tag.bytes());
            Ok(encode_packet(Tag::Asyn, &payload))
        }
        other => Err(ValeriaError::Protocol(format!(
            "cannot encode {other:?} as a simple ASYN packet"
        ))),
    }
}

pub(crate) fn encode_start_video() -> Bytes {
    encode_asyn_with_payload(Tag::Hpd1, EMPTY_CLOCK_REF, &hpd1_device_info_dict())
}

pub(crate) fn encode_start_audio(device_audio_clock_ref: u64) -> Bytes {
    encode_asyn_with_payload(Tag::Hpa1, device_audio_clock_ref, &hpa1_device_info_dict())
}

pub(crate) fn encode_stop_video() -> Bytes {
    encode_asyn_simple(Tag::Hpd0, EMPTY_CLOCK_REF).expect("HPD0 is a simple ASYN packet")
}

pub(crate) fn encode_stop_audio(device_audio_clock_ref: u64) -> Bytes {
    encode_asyn_simple(Tag::Hpa0, device_audio_clock_ref).expect("HPA0 is a simple ASYN packet")
}

pub(crate) fn encode_reply_status_ok(correlation: u64) -> Bytes {
    let mut payload = BytesMut::with_capacity(4);
    payload.put_u32_le(0);
    encode_reply(correlation, &payload)
}

pub(crate) fn encode_reply_u64(correlation: u64, value: u64) -> Bytes {
    let mut payload = BytesMut::with_capacity(8);
    payload.put_u64_le(value);
    encode_reply(correlation, &payload)
}

pub(crate) fn encode_reply_f64(correlation: u64, value: f64) -> Bytes {
    let mut payload = BytesMut::with_capacity(8);
    payload.put_f64_le(value);
    encode_reply(correlation, &payload)
}

pub(crate) fn encode_reply_cmtime(correlation: u64, value_ns: i64) -> Bytes {
    let mut payload = BytesMut::with_capacity(24);
    payload.put_i64_le(value_ns);
    payload.put_i32_le(1_000_000_000);
    payload.put_u32_le(1);
    payload.put_i64_le(0);
    encode_reply(correlation, &payload)
}

pub(crate) fn encode_afmt_reply(correlation: u64) -> Bytes {
    let entries = [DictEntry {
        key: "Error",
        value: DictValue::Number(NumberValue::U32(0)),
    }];
    encode_reply(correlation, &serialize_string_key_dict(&entries))
}

fn encode_asyn_with_payload(tag: Tag, clock_ref: u64, body: &[u8]) -> Bytes {
    let mut payload = BytesMut::with_capacity(12 + body.len());
    payload.put_u64_le(clock_ref);
    payload.put_slice(tag.bytes());
    payload.put_slice(body);
    encode_packet(Tag::Asyn, &payload)
}

fn encode_reply(correlation: u64, value: &[u8]) -> Bytes {
    let mut payload = BytesMut::with_capacity(12 + value.len());
    payload.put_u64_le(correlation);
    payload.put_u32_le(0);
    payload.put_slice(value);
    encode_packet(Tag::Reply, &payload)
}

#[derive(Debug, Clone, Copy)]
struct DictEntry<'a> {
    key: &'a str,
    value: DictValue<'a>,
}

#[derive(Debug, Clone, Copy)]
enum DictValue<'a> {
    Bool(bool),
    String(&'a str),
    Data(&'a [u8]),
    Number(NumberValue),
    Dict(&'a [DictEntry<'a>]),
}

#[derive(Debug, Clone, Copy)]
enum NumberValue {
    U32(u32),
    F64(f64),
}

fn hpd1_device_info_dict() -> Bytes {
    let display_size = [
        DictEntry {
            key: "Width",
            value: DictValue::Number(NumberValue::F64(1920.0)),
        },
        DictEntry {
            key: "Height",
            value: DictValue::Number(NumberValue::F64(1200.0)),
        },
    ];
    let entries = [
        DictEntry {
            key: "Valeria",
            value: DictValue::Bool(true),
        },
        DictEntry {
            key: "HEVCDecoderSupports444",
            value: DictValue::Bool(true),
        },
        DictEntry {
            key: "DisplaySize",
            value: DictValue::Dict(&display_size),
        },
    ];
    serialize_string_key_dict(&entries)
}

fn hpa1_device_info_dict() -> Bytes {
    let audio_format = default_audio_stream_basic_description();
    let entries = [
        DictEntry {
            key: "BufferAheadInterval",
            value: DictValue::Number(NumberValue::F64(0.07300000000000001)),
        },
        DictEntry {
            key: "deviceUID",
            value: DictValue::String("Valeria"),
        },
        DictEntry {
            key: "ScreenLatency",
            value: DictValue::Number(NumberValue::F64(0.04)),
        },
        DictEntry {
            key: "formats",
            value: DictValue::Data(&audio_format),
        },
        DictEntry {
            key: "EDIDAC3Support",
            value: DictValue::Number(NumberValue::U32(0)),
        },
        DictEntry {
            key: "deviceName",
            value: DictValue::String("Valeria"),
        },
    ];
    serialize_string_key_dict(&entries)
}

fn serialize_string_key_dict(entries: &[DictEntry<'_>]) -> Bytes {
    let mut out = BytesMut::new();
    put_record(&mut out, MAGIC_DICT, |out| {
        for entry in entries {
            put_record(out, MAGIC_KEYV, |out| {
                put_record(out, MAGIC_STRK, |out| out.put_slice(entry.key.as_bytes()));
                put_dict_value(out, entry.value);
            });
        }
    });
    out.freeze()
}

fn put_dict_value(out: &mut BytesMut, value: DictValue<'_>) {
    match value {
        DictValue::Bool(value) => {
            put_record(out, MAGIC_BULV, |out| out.put_u8(u8::from(value)));
        }
        DictValue::String(value) => {
            put_record(out, MAGIC_STRV, |out| out.put_slice(value.as_bytes()));
        }
        DictValue::Data(value) => {
            put_record(out, MAGIC_DATV, |out| out.put_slice(value));
        }
        DictValue::Number(value) => {
            put_record(out, MAGIC_NMBV, |out| match value {
                NumberValue::U32(value) => {
                    out.put_u8(3);
                    out.put_u32_le(value);
                }
                NumberValue::F64(value) => {
                    out.put_u8(6);
                    out.put_f64_le(value);
                }
            });
        }
        DictValue::Dict(entries) => {
            put_record(out, MAGIC_DICT, |out| {
                for entry in entries {
                    put_record(out, MAGIC_KEYV, |out| {
                        put_record(out, MAGIC_STRK, |out| out.put_slice(entry.key.as_bytes()));
                        put_dict_value(out, entry.value);
                    });
                }
            });
        }
    }
}

fn put_record(out: &mut BytesMut, magic: &[u8; 4], body: impl FnOnce(&mut BytesMut)) {
    let start = out.len();
    out.put_u32_le(0);
    out.put_slice(magic);
    body(out);
    let len = (out.len() - start) as u32;
    out[start..start + 4].copy_from_slice(&len.to_le_bytes());
}

fn default_audio_stream_basic_description() -> [u8; 56] {
    let mut out = [0u8; 56];
    let mut pos = 0usize;
    put_f64_at(&mut out, &mut pos, 48_000.0);
    put_u32_at(&mut out, &mut pos, 0x6c70_636d);
    put_u32_at(&mut out, &mut pos, 12);
    put_u32_at(&mut out, &mut pos, 4);
    put_u32_at(&mut out, &mut pos, 1);
    put_u32_at(&mut out, &mut pos, 4);
    put_u32_at(&mut out, &mut pos, 2);
    put_u32_at(&mut out, &mut pos, 16);
    put_u32_at(&mut out, &mut pos, 0);
    put_f64_at(&mut out, &mut pos, 48_000.0);
    put_f64_at(&mut out, &mut pos, 48_000.0);
    out
}

fn put_u32_at(out: &mut [u8; 56], pos: &mut usize, value: u32) {
    out[*pos..*pos + 4].copy_from_slice(&value.to_le_bytes());
    *pos += 4;
}

fn put_f64_at(out: &mut [u8; 56], pos: &mut usize, value: f64) {
    out[*pos..*pos + 8].copy_from_slice(&value.to_le_bytes());
    *pos += 8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ping_packet_with_total_length_prefix() {
        let packet = decode_packet(&[
            0x10, 0x00, 0x00, 0x00, b'g', b'n', b'i', b'p', 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00,
        ])
        .unwrap();

        assert_eq!(packet.tag, Tag::Ping);
        assert_eq!(packet.payload, &[0, 0, 0, 0, 1, 0, 0, 0][..]);
    }

    #[test]
    fn encodes_ping_reply() {
        assert_eq!(
            encode_ping().as_ref(),
            &[
                0x10, 0x00, 0x00, 0x00, b'g', b'n', b'i', b'p', 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
                0x00, 0x00,
            ]
        );
    }

    #[test]
    fn encodes_need_for_device_video_clock() {
        let bytes = encode_asyn_simple(Tag::Need, 0x0102030405060708).unwrap();
        assert_eq!(&bytes[0..4], &[0x14, 0, 0, 0]);
        assert_eq!(&bytes[4..8], b"nysa");
        assert_eq!(
            &bytes[8..16],
            &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
        assert_eq!(&bytes[16..20], b"deen");
    }

    #[test]
    fn encodes_stream_start_and_stop_packets_like_quicktime_reference() {
        assert_eq!(
            encode_start_video().as_ref(),
            include_bytes!("../../../tests/fixtures/valeria/asyn-hpd1.bin")
        );
        assert_eq!(
            encode_start_audio(0x0000_0001_1453_92f0).as_ref(),
            include_bytes!("../../../tests/fixtures/valeria/asyn-hpa1.bin")
        );
        assert_eq!(
            encode_stop_video().as_ref(),
            include_bytes!("../../../tests/fixtures/valeria/asyn-hpd0.bin")
        );
        assert_eq!(
            encode_stop_audio(0x0000_0001_02c5_fc10).as_ref(),
            include_bytes!("../../../tests/fixtures/valeria/asyn-hpa0.bin")
        );
    }
}
