use bytes::Bytes;

use super::frame::H264Frame;
#[cfg(test)]
use super::protocol::decode_packet;
use super::protocol::Tag;
use super::ValeriaError;

const MAGIC_SBUF: &[u8; 4] = b"fubs";
const MAGIC_OPTS: &[u8; 4] = b"stpo";
const MAGIC_STIA: &[u8; 4] = b"aits";
const MAGIC_SDAT: &[u8; 4] = b"tads";
const MAGIC_SATT: &[u8; 4] = b"ttas";
const MAGIC_SSIZ: &[u8; 4] = b"ziss";
const MAGIC_NSMP: &[u8; 4] = b"pmsn";
const MAGIC_SARY: &[u8; 4] = b"yras";

const MAGIC_FDSC: &[u8; 4] = b"csdf";
const MAGIC_MDIA: &[u8; 4] = b"aidm";
const MAGIC_VDIM: &[u8; 4] = b"midv";
const MAGIC_CODC: &[u8; 4] = b"cdoc";
const MAGIC_EXTN: &[u8; 4] = b"ntxe";

const MAGIC_DICT: &[u8; 4] = b"tcid";
const MAGIC_KEYV: &[u8; 4] = b"vyek";
const MAGIC_STRK: &[u8; 4] = b"krts";
const MAGIC_IDXK: &[u8; 4] = b"kxdi";
const MAGIC_BULV: &[u8; 4] = b"vlub";
const MAGIC_STRV: &[u8; 4] = b"vrts";
const MAGIC_DATV: &[u8; 4] = b"vtad";
const MAGIC_NMBV: &[u8; 4] = b"vbmn";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CmTime {
    pub(crate) value: i64,
    pub(crate) scale: i32,
    pub(crate) flags: u32,
    pub(crate) epoch: i64,
}

impl CmTime {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ValeriaError> {
        if bytes.len() < 24 {
            return Err(ValeriaError::Protocol(format!(
                "CMTime requires 24 bytes, got {}",
                bytes.len()
            )));
        }
        Ok(Self {
            value: i64::from_le_bytes(bytes[0..8].try_into().expect("slice length checked")),
            scale: i32::from_le_bytes(bytes[8..12].try_into().expect("slice length checked")),
            flags: u32::from_le_bytes(bytes[12..16].try_into().expect("slice length checked")),
            epoch: i64::from_le_bytes(bytes[16..24].try_into().expect("slice length checked")),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CmDict {
    string_entries: Vec<(String, CmValue)>,
    index_entries: Vec<(u16, CmValue)>,
}

impl CmDict {
    fn empty() -> Self {
        Self {
            string_entries: Vec::new(),
            index_entries: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn get_bool(&self, key: &str) -> Option<bool> {
        self.string_entries.iter().find_map(|(entry_key, value)| {
            if entry_key == key {
                match value {
                    CmValue::Bool(value) => Some(*value),
                    _ => None,
                }
            } else {
                None
            }
        })
    }

    fn get_index(&self, key: u16) -> Option<&CmValue> {
        self.index_entries
            .iter()
            .find_map(|(entry_key, value)| (*entry_key == key).then_some(value))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum CmValue {
    Bool(bool),
    String(String),
    Data(Bytes),
    Number(CmNumber),
    Dict(CmDict),
    FormatDescription(FormatDescription),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CmNumber {
    kind: u32,
    int_value: Option<i64>,
    float_value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FormatDescription {
    width: u32,
    height: u32,
    sps: Option<Bytes>,
    pps: Option<Bytes>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CmSampleBuffer {
    output_presentation_timestamp: CmTime,
    format_description: Option<FormatDescription>,
    sample_data: Bytes,
}

impl CmSampleBuffer {
    pub(crate) fn into_h264_frame(self) -> Result<H264Frame, ValeriaError> {
        if self.sample_data.is_empty() {
            return Err(ValeriaError::Protocol(
                "CMSampleBuffer missing sample data".into(),
            ));
        }

        let format = self.format_description;
        Ok(H264Frame {
            nalu_data: self.sample_data,
            sps: format.as_ref().and_then(|format| format.sps.clone()),
            pps: format.as_ref().and_then(|format| format.pps.clone()),
            width: format.as_ref().map_or(0, |format| format.width),
            height: format.as_ref().map_or(0, |format| format.height),
            pts_value: self.output_presentation_timestamp.value,
            pts_scale: self.output_presentation_timestamp.scale,
        })
    }
}

#[cfg(test)]
pub(crate) fn parse_feed_packet(bytes: &[u8]) -> Result<CmSampleBuffer, ValeriaError> {
    if bytes.len() >= 8 {
        let total_len =
            u32::from_le_bytes(bytes[0..4].try_into().expect("slice length checked")) as usize;
        if total_len == bytes.len() {
            let packet = decode_packet(bytes)?;
            if packet.tag != Tag::Asyn {
                return Err(ValeriaError::Protocol(format!(
                    "expected ASYN packet, got {:?}",
                    packet.tag
                )));
            }
            return parse_feed_payload(&packet.payload);
        }
    }

    if bytes.starts_with(Tag::Asyn.bytes()) {
        parse_feed_payload(&bytes[4..])
    } else {
        parse_sample_buffer(bytes)
    }
}

pub(crate) fn parse_feed_payload(payload: &[u8]) -> Result<CmSampleBuffer, ValeriaError> {
    if payload.len() < 12 {
        return Err(ValeriaError::Protocol(
            "ASYN FEED payload shorter than clock/subtype header".into(),
        ));
    }
    let subtype = &payload[8..12];
    if subtype != Tag::Feed.bytes() {
        return Err(ValeriaError::Protocol(format!(
            "expected FEED payload, got {:02x?}",
            subtype
        )));
    }
    parse_sample_buffer(&payload[12..])
}

pub(crate) fn parse_dict(bytes: &[u8]) -> Result<CmDict, ValeriaError> {
    parse_dict_with_magic(bytes, MAGIC_DICT)
}

fn parse_sample_buffer(bytes: &[u8]) -> Result<CmSampleBuffer, ValeriaError> {
    let end = checked_record_end(bytes, 0, MAGIC_SBUF)?;
    let mut pos = 8usize;
    let mut output_presentation_timestamp = CmTime {
        value: 0,
        scale: 0,
        flags: 0,
        epoch: 0,
    };
    let mut format_description = None;
    let mut sample_data = Bytes::new();

    while pos < end {
        let field_end = checked_any_record_end(bytes, pos, "CMSampleBuffer field")?;
        let magic = magic_at(bytes, pos)?;
        match magic {
            MAGIC_OPTS => {
                output_presentation_timestamp = CmTime::parse(take_record_payload(
                    bytes,
                    pos,
                    field_end,
                    "output presentation timestamp",
                )?)?;
            }
            MAGIC_STIA | MAGIC_SATT | MAGIC_SARY => {}
            MAGIC_SDAT => {
                sample_data = Bytes::copy_from_slice(take_record_payload(
                    bytes,
                    pos,
                    field_end,
                    "sample data",
                )?);
            }
            MAGIC_NSMP | MAGIC_SSIZ => {}
            MAGIC_FDSC => {
                format_description = Some(parse_format_description(&bytes[pos..field_end])?);
            }
            other => {
                return Err(ValeriaError::Protocol(format!(
                    "unknown CMSampleBuffer field {} at byte {pos}",
                    ascii_tag(other)
                )));
            }
        }
        pos = field_end;
    }

    Ok(CmSampleBuffer {
        output_presentation_timestamp,
        format_description,
        sample_data,
    })
}

fn parse_format_description(bytes: &[u8]) -> Result<FormatDescription, ValeriaError> {
    let end = checked_record_end(bytes, 0, MAGIC_FDSC)?;
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut sps = None;
    let mut pps = None;

    while pos < end {
        let field_end = checked_any_record_end(bytes, pos, "format description field")?;
        let magic = magic_at(bytes, pos)?;
        match magic {
            MAGIC_MDIA | MAGIC_CODC => {}
            MAGIC_VDIM => {
                let payload = take_record_payload(bytes, pos, field_end, "video dimensions")?;
                if payload.len() < 8 {
                    return Err(ValeriaError::Protocol(
                        "video dimensions payload shorter than 8 bytes".into(),
                    ));
                }
                width = u32::from_le_bytes(payload[0..4].try_into().unwrap());
                height = u32::from_le_bytes(payload[4..8].try_into().unwrap());
            }
            MAGIC_EXTN => {
                let extensions = parse_dict_with_magic(&bytes[pos..field_end], MAGIC_EXTN)?;
                if let Some(CmValue::Dict(avc_dict)) = extensions.get_index(49) {
                    if let Some(CmValue::Data(avcc)) = avc_dict.get_index(105) {
                        let parameter_sets = parse_avc_decoder_configuration(avcc)?;
                        sps = parameter_sets.0;
                        pps = parameter_sets.1;
                    }
                }
            }
            other => {
                return Err(ValeriaError::Protocol(format!(
                    "unknown format description field {} at byte {pos}",
                    ascii_tag(other)
                )));
            }
        }
        pos = field_end;
    }

    Ok(FormatDescription {
        width,
        height,
        sps,
        pps,
    })
}

fn parse_avc_decoder_configuration(
    bytes: &[u8],
) -> Result<(Option<Bytes>, Option<Bytes>), ValeriaError> {
    if bytes.len() < 8 {
        return Err(ValeriaError::Protocol(
            "AVC decoder configuration too short".into(),
        ));
    }
    let sps_count = bytes[5] & 0x1f;
    let mut pos = 6usize;
    let mut sps = None;
    for index in 0..sps_count {
        let len = read_u16_be(bytes, &mut pos, "SPS length")? as usize;
        let value = take_at(bytes, &mut pos, len, "SPS bytes")?;
        if index == 0 {
            sps = Some(Bytes::copy_from_slice(value));
        }
    }

    let pps_count = *take_at(bytes, &mut pos, 1, "PPS count")?
        .first()
        .expect("take_at returned one byte");
    let mut pps = None;
    for index in 0..pps_count {
        let len = read_u16_be(bytes, &mut pos, "PPS length")? as usize;
        let value = take_at(bytes, &mut pos, len, "PPS bytes")?;
        if index == 0 {
            pps = Some(Bytes::copy_from_slice(value));
        }
    }

    Ok((sps, pps))
}

fn parse_dict_with_magic(bytes: &[u8], magic: &[u8; 4]) -> Result<CmDict, ValeriaError> {
    let end = checked_record_end(bytes, 0, magic)?;
    let mut pos = 8usize;
    let mut dict = CmDict::empty();

    while pos < end {
        let pair_end = checked_record_end(bytes, pos, MAGIC_KEYV)?;
        let mut pair_pos = pos + 8;
        let key_end = checked_any_record_end(bytes, pair_pos, "dictionary key")?;
        let key_magic = magic_at(bytes, pair_pos)?;
        match key_magic {
            MAGIC_STRK => {
                let key_bytes = take_record_payload(bytes, pair_pos, key_end, "string key")?;
                let key = std::str::from_utf8(key_bytes)
                    .map_err(|err| ValeriaError::Protocol(format!("invalid string key: {err}")))?
                    .to_string();
                pair_pos = key_end;
                let value = parse_value(&bytes[pair_pos..pair_end])?;
                dict.string_entries.push((key, value));
            }
            MAGIC_IDXK => {
                let key_bytes = take_record_payload(bytes, pair_pos, key_end, "index key")?;
                if key_bytes.len() < 2 {
                    return Err(ValeriaError::Protocol("index key too short".into()));
                }
                let key = u16::from_le_bytes(key_bytes[0..2].try_into().unwrap());
                pair_pos = key_end;
                let value = parse_value(&bytes[pair_pos..pair_end])?;
                dict.index_entries.push((key, value));
            }
            other => {
                return Err(ValeriaError::Protocol(format!(
                    "unknown dictionary key type {} at byte {pair_pos}",
                    ascii_tag(other)
                )));
            }
        }
        pos = pair_end;
    }

    Ok(dict)
}

fn parse_value(bytes: &[u8]) -> Result<CmValue, ValeriaError> {
    let end = checked_any_record_end(bytes, 0, "dictionary value")?;
    if end != bytes.len() {
        return Err(ValeriaError::Protocol(format!(
            "dictionary value length {end} does not match {} bytes",
            bytes.len()
        )));
    }
    let payload = &bytes[8..end];
    Ok(match magic_at(bytes, 0)? {
        MAGIC_BULV => CmValue::Bool(payload.first().copied() == Some(1)),
        MAGIC_STRV => {
            let value = std::str::from_utf8(payload)
                .map_err(|err| ValeriaError::Protocol(format!("invalid string value: {err}")))?;
            CmValue::String(value.to_string())
        }
        MAGIC_DATV => CmValue::Data(Bytes::copy_from_slice(payload)),
        MAGIC_NMBV => CmValue::Number(parse_number(payload)?),
        MAGIC_DICT => CmValue::Dict(parse_dict(bytes)?),
        MAGIC_FDSC => CmValue::FormatDescription(parse_format_description(bytes)?),
        other => {
            return Err(ValeriaError::Protocol(format!(
                "unknown dictionary value type {}",
                ascii_tag(other)
            )));
        }
    })
}

fn parse_number(bytes: &[u8]) -> Result<CmNumber, ValeriaError> {
    if bytes.len() < 8 {
        return Err(ValeriaError::Protocol("NSNumber payload too short".into()));
    }
    let kind = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let number = &bytes[4..];
    let (int_value, float_value) = match kind {
        3 if number.len() >= 4 => (
            Some(i64::from(i32::from_le_bytes(
                number[0..4].try_into().unwrap(),
            ))),
            None,
        ),
        4 if number.len() >= 8 => (
            Some(i64::from_le_bytes(number[0..8].try_into().unwrap())),
            None,
        ),
        6 if number.len() >= 8 => (
            None,
            Some(f64::from_le_bytes(number[0..8].try_into().unwrap())),
        ),
        _ => (None, None),
    };
    Ok(CmNumber {
        kind,
        int_value,
        float_value,
    })
}

fn checked_record_end(
    bytes: &[u8],
    pos: usize,
    expected_magic: &[u8; 4],
) -> Result<usize, ValeriaError> {
    let expected = ascii_tag(expected_magic);
    let end = checked_any_record_end(bytes, pos, &expected)?;
    let actual = magic_at(bytes, pos)?;
    if actual != expected_magic {
        return Err(ValeriaError::Protocol(format!(
            "expected {} at byte {pos}, got {}",
            ascii_tag(expected_magic),
            ascii_tag(actual)
        )));
    }
    Ok(end)
}

fn checked_any_record_end(bytes: &[u8], pos: usize, what: &str) -> Result<usize, ValeriaError> {
    if pos + 8 > bytes.len() {
        return Err(ValeriaError::Protocol(format!(
            "{what} header at byte {pos} exceeds {} bytes",
            bytes.len()
        )));
    }
    let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
    if len < 8 {
        return Err(ValeriaError::Protocol(format!(
            "{what} length {len} at byte {pos} is shorter than header"
        )));
    }
    let end = pos
        .checked_add(len)
        .ok_or_else(|| ValeriaError::Protocol(format!("{what} length overflows at byte {pos}")))?;
    if end > bytes.len() {
        return Err(ValeriaError::Protocol(format!(
            "{what} length {len} at byte {pos} exceeds {} bytes",
            bytes.len()
        )));
    }
    Ok(end)
}

fn magic_at(bytes: &[u8], pos: usize) -> Result<&[u8; 4], ValeriaError> {
    bytes
        .get(pos + 4..pos + 8)
        .and_then(|magic| magic.try_into().ok())
        .ok_or_else(|| ValeriaError::Protocol(format!("missing magic at byte {pos}")))
}

fn take_record_payload<'a>(
    bytes: &'a [u8],
    pos: usize,
    end: usize,
    what: &str,
) -> Result<&'a [u8], ValeriaError> {
    if pos + 8 > end || end > bytes.len() {
        return Err(ValeriaError::Protocol(format!(
            "{what} payload range {pos}..{end} is invalid"
        )));
    }
    Ok(&bytes[pos + 8..end])
}

fn take_at<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
    len: usize,
    what: &str,
) -> Result<&'a [u8], ValeriaError> {
    let end = pos
        .checked_add(len)
        .ok_or_else(|| ValeriaError::Protocol(format!("{what} length overflows")))?;
    if end > bytes.len() {
        return Err(ValeriaError::Protocol(format!(
            "{what} requires {len} bytes at {}, only {} remain",
            *pos,
            bytes.len().saturating_sub(*pos)
        )));
    }
    let value = &bytes[*pos..end];
    *pos = end;
    Ok(value)
}

fn read_u16_be(bytes: &[u8], pos: &mut usize, what: &str) -> Result<u16, ValeriaError> {
    let value = take_at(bytes, pos, 2, what)?;
    Ok(u16::from_be_bytes(value.try_into().unwrap()))
}

fn ascii_tag(bytes: &[u8; 4]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cmtime_from_little_endian_bytes() {
        let time = CmTime::parse(&[
            0xcb, 0x44, 0x8e, 0xa1, 0xcf, 0x10, 0xcc, 0x15, 0x00, 0xca, 0x9a, 0x3b, 0x01, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .unwrap();

        assert_eq!(time.scale, 1_000_000_000);
        assert_eq!(time.flags, 1);
    }

    #[test]
    fn parses_string_key_boolean_dictionary() {
        let dict = parse_dict(&[
            0x28, 0x00, 0x00, 0x00, b't', b'c', b'i', b'd', 0x20, 0x00, 0x00, 0x00, b'v', b'y',
            b'e', b'k', 0x0f, 0x00, 0x00, 0x00, b'k', b'r', b't', b's', b'V', b'a', b'l', b'e',
            b'r', b'i', b'a', 0x09, 0x00, 0x00, 0x00, b'v', b'l', b'u', b'b', 0x01,
        ])
        .unwrap();

        assert_eq!(dict.get_bool("Valeria"), Some(true));
    }

    #[test]
    fn parses_video_frame_from_feed_fixture() {
        let bytes = include_bytes!("../../../tests/fixtures/valeria/asyn-feed.bin");
        let sample = parse_feed_packet(bytes).unwrap();
        let frame = sample.into_h264_frame().unwrap();

        assert!(!frame.nalu_data.is_empty());
        assert!(frame.width > 0);
        assert!(frame.height > 0);
        assert!(frame.to_annex_b().unwrap().starts_with(b"\x00\x00\x00\x01"));
    }
}
