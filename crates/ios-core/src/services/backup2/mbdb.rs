//! The pre-iOS 10.3 `Manifest.mbdb` backup index.
//!
//! `Manifest.mbdb` is an unencrypted, self-delimiting sequence of records.  The
//! records use two-byte big-endian lengths for strings and byte arrays; `0xffff`
//! means a missing value.  There is no record count in the format.  The parser in
//! this module deliberately keeps the complete record, including fields that are
//! not interpreted by the host, so filtering can rewrite an old backup without
//! changing the order or silently dropping metadata.
//!
//! The layout and the file-id rule are compatible with pyiosbackup 0.2.4's
//! `manifest_dbs/mbdb.py`.  That implementation reads only `Manifest.mbdb`; an
//! MBDX sidecar is not part of the supported legacy interchange and is therefore
//! neither required nor generated here.

use std::fmt;

use sha1::{Digest, Sha1};
use zeroize::{Zeroize, Zeroizing};

const MAGIC: &[u8; 6] = b"mbdb\x05\x00";
const NULL_LENGTH: u16 = u16::MAX;
const MAX_RECORDS: usize = 1_000_000;
const MAX_FIELD_BYTES: usize = 65_534;

type MbdbProperty = (Option<String>, Option<String>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MbdbManifest {
    pub(crate) records: Vec<MbdbRecord>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MbdbRecord {
    pub(crate) domain: Option<String>,
    pub(crate) relative_path: Option<String>,
    pub(crate) link_target: Option<String>,
    pub(crate) data_hash: Option<Vec<u8>>,
    pub(crate) encryption_key: Option<Zeroizing<Vec<u8>>>,
    pub(crate) mode: u16,
    pub(crate) unknown2: u32,
    pub(crate) unknown3: u32,
    pub(crate) user_id: u32,
    pub(crate) group_id: u32,
    pub(crate) mtime: u32,
    pub(crate) atime: u32,
    pub(crate) ctime: u32,
    pub(crate) size: u64,
    pub(crate) flags: u8,
    pub(crate) properties: Vec<MbdbProperty>,
}

impl fmt::Debug for MbdbRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MbdbRecord")
            .field("domain", &self.domain)
            .field("relative_path", &self.relative_path)
            .field("link_target", &self.link_target)
            .field(
                "data_hash",
                &self.data_hash.as_ref().map(|value| value.len()),
            )
            .field(
                "encryption_key",
                &self.encryption_key.as_ref().map(|value| value.len()),
            )
            .field("mode", &format_args!("0x{:04x}", self.mode))
            .field("unknown2", &self.unknown2)
            .field("unknown3", &self.unknown3)
            .field("user_id", &self.user_id)
            .field("group_id", &self.group_id)
            .field("mtime", &self.mtime)
            .field("atime", &self.atime)
            .field("ctime", &self.ctime)
            .field("size", &self.size)
            .field("flags", &self.flags)
            .field("properties", &self.properties)
            .finish()
    }
}

impl Drop for MbdbRecord {
    fn drop(&mut self) {
        if let Some(key) = &mut self.encryption_key {
            key.zeroize();
        }
        if let Some(hash) = &mut self.data_hash {
            hash.zeroize();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MbdbError(pub(crate) &'static str);

impl fmt::Display for MbdbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for MbdbError {}

impl MbdbManifest {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, MbdbError> {
        if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
            return Err(MbdbError("Manifest.mbdb has an invalid magic/version"));
        }
        let mut reader = Reader::new(&bytes[MAGIC.len()..]);
        let mut records = Vec::new();
        while !reader.is_empty() {
            if records.len() >= MAX_RECORDS {
                return Err(MbdbError("Manifest.mbdb exceeds the record safety limit"));
            }
            records.push(reader.record()?);
        }
        Ok(Self { records })
    }

    pub(crate) fn serialize(
        &self,
        retain: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<u8>, MbdbError> {
        if self.records.len() > MAX_RECORDS {
            return Err(MbdbError("Manifest.mbdb exceeds the record safety limit"));
        }
        let mut output = Vec::with_capacity(MAGIC.len());
        output.extend_from_slice(MAGIC);
        for record in &self.records {
            if let Some(retain) = retain {
                let id = record.file_id()?;
                if !retain.contains(&id) {
                    continue;
                }
            }
            record.encode(&mut output)?;
        }
        Ok(output)
    }
}

impl MbdbRecord {
    pub(crate) fn file_id(&self) -> Result<String, MbdbError> {
        let domain = self
            .domain
            .as_deref()
            .ok_or(MbdbError("Manifest.mbdb record has no domain"))?;
        let relative_path = self
            .relative_path
            .as_deref()
            .ok_or(MbdbError("Manifest.mbdb record has no relative path"))?;
        let mut input = Vec::with_capacity(domain.len() + 1 + relative_path.len());
        input.extend_from_slice(domain.as_bytes());
        input.push(b'-');
        input.extend_from_slice(relative_path.as_bytes());
        Ok(hex::encode(Sha1::digest(input)))
    }

    fn encode(&self, output: &mut Vec<u8>) -> Result<(), MbdbError> {
        put_string(output, self.domain.as_deref())?;
        put_string(output, self.relative_path.as_deref())?;
        put_string(output, self.link_target.as_deref())?;
        put_bytes(output, self.data_hash.as_deref())?;
        put_bytes(
            output,
            self.encryption_key.as_ref().map(|value| value.as_slice()),
        )?;
        output.extend_from_slice(&self.mode.to_be_bytes());
        output.extend_from_slice(&self.unknown2.to_be_bytes());
        output.extend_from_slice(&self.unknown3.to_be_bytes());
        output.extend_from_slice(&self.user_id.to_be_bytes());
        output.extend_from_slice(&self.group_id.to_be_bytes());
        output.extend_from_slice(&self.mtime.to_be_bytes());
        output.extend_from_slice(&self.atime.to_be_bytes());
        output.extend_from_slice(&self.ctime.to_be_bytes());
        output.extend_from_slice(&self.size.to_be_bytes());
        output.push(self.flags);
        let property_count = u8::try_from(self.properties.len())
            .map_err(|_| MbdbError("Manifest.mbdb has too many properties in one record"))?;
        output.push(property_count);
        for (name, value) in &self.properties {
            put_string(output, name.as_deref())?;
            put_string(output, value.as_deref())?;
        }
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn record(&mut self) -> Result<MbdbRecord, MbdbError> {
        Ok(MbdbRecord {
            domain: self.string("domain")?,
            relative_path: self.string("relative path")?,
            link_target: self.string("link target")?,
            data_hash: self.bytes("data hash")?,
            encryption_key: self.bytes("encryption key")?.map(Zeroizing::new),
            mode: self.u16("mode")?,
            unknown2: self.u32("unknown2")?,
            unknown3: self.u32("unknown3")?,
            user_id: self.u32("user id")?,
            group_id: self.u32("group id")?,
            mtime: self.u32("mtime")?,
            atime: self.u32("atime")?,
            ctime: self.u32("ctime")?,
            size: self.u64("file size")?,
            flags: self.byte("flags")?,
            properties: self.properties()?,
        })
    }

    fn properties(&mut self) -> Result<Vec<MbdbProperty>, MbdbError> {
        let count = usize::from(self.byte("property count")?);
        let mut properties = Vec::with_capacity(count);
        for _ in 0..count {
            properties.push((
                self.string("property name")?,
                self.string("property value")?,
            ));
        }
        Ok(properties)
    }

    fn string(&mut self, field: &'static str) -> Result<Option<String>, MbdbError> {
        let bytes = self.bytes(field)?;
        bytes
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| MbdbError("Manifest.mbdb contains invalid UTF-8 text"))
            })
            .transpose()
            .map(|value| value.flatten())
    }

    fn bytes(&mut self, field: &'static str) -> Result<Option<Vec<u8>>, MbdbError> {
        let length = self.u16(field)?;
        if length == NULL_LENGTH {
            return Ok(None);
        }
        let length = usize::from(length);
        if length > MAX_FIELD_BYTES {
            return Err(MbdbError("Manifest.mbdb field is too large"));
        }
        let end = self
            .offset
            .checked_add(length)
            .ok_or(MbdbError("Manifest.mbdb field offset overflow"))?;
        if end > self.bytes.len() {
            return Err(MbdbError("Manifest.mbdb field is truncated"));
        }
        let value = self.bytes[self.offset..end].to_vec();
        self.offset = end;
        Ok(Some(value))
    }

    fn byte(&mut self, field: &'static str) -> Result<u8, MbdbError> {
        let byte = *self.bytes.get(self.offset).ok_or(MbdbError(field))?;
        self.offset = self
            .offset
            .checked_add(1)
            .ok_or(MbdbError("Manifest.mbdb offset overflow"))?;
        Ok(byte)
    }

    fn u16(&mut self, field: &'static str) -> Result<u16, MbdbError> {
        let end = self
            .offset
            .checked_add(2)
            .ok_or(MbdbError("Manifest.mbdb offset overflow"))?;
        let bytes = self.bytes.get(self.offset..end).ok_or(MbdbError(field))?;
        self.offset = end;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, MbdbError> {
        let end = self
            .offset
            .checked_add(4)
            .ok_or(MbdbError("Manifest.mbdb offset overflow"))?;
        let bytes = self.bytes.get(self.offset..end).ok_or(MbdbError(field))?;
        self.offset = end;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self, field: &'static str) -> Result<u64, MbdbError> {
        let end = self
            .offset
            .checked_add(8)
            .ok_or(MbdbError("Manifest.mbdb offset overflow"))?;
        let bytes = self.bytes.get(self.offset..end).ok_or(MbdbError(field))?;
        self.offset = end;
        Ok(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
}

fn put_string(output: &mut Vec<u8>, value: Option<&str>) -> Result<(), MbdbError> {
    put_bytes(output, value.map(str::as_bytes))
}

fn put_bytes(output: &mut Vec<u8>, value: Option<&[u8]>) -> Result<(), MbdbError> {
    let Some(value) = value else {
        output.extend_from_slice(&NULL_LENGTH.to_be_bytes());
        return Ok(());
    };
    let length = u16::try_from(value.len())
        .ok()
        .filter(|length| *length != NULL_LENGTH)
        .ok_or(MbdbError("Manifest.mbdb field is too large to serialize"))?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    // Exact bytes from pyiosbackup 0.2.4 tests/test_backup.py::example_mbdb.
    const PYIOSBACKUP_RECORD: &[u8] = &[
        0x6d, 0x62, 0x64, 0x62, 0x05, 0x00, 0x00, 0x0c, 0x4d, 0x79, 0x54, 0x65, 0x73, 0x74, 0x44,
        0x6f, 0x6d, 0x61, 0x69, 0x6e, 0x00, 0x0e, 0x4d, 0x65, 0x64, 0x69, 0x61, 0x2f, 0x54, 0x65,
        0x73, 0x74, 0x2e, 0x74, 0x78, 0x74, 0xff, 0xff, 0xff, 0xff, 0x00, 0x2c, 0x04, 0x00, 0x00,
        0x00, 0x97, 0x31, 0x74, 0x94, 0x34, 0x38, 0x07, 0xe6, 0x90, 0xfd, 0x1e, 0x43, 0x14, 0x13,
        0x96, 0x3d, 0xc0, 0xe3, 0xde, 0xb4, 0x90, 0x7f, 0xb8, 0x9f, 0xa3, 0x6c, 0xe6, 0x51, 0x26,
        0xd0, 0xea, 0x13, 0x01, 0x38, 0x1f, 0xb3, 0xa2, 0x94, 0x1e, 0x2f, 0x81, 0xed, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x12, 0xd8, 0x00, 0x00, 0x01, 0xf5, 0x00, 0x00, 0x01, 0xf5, 0x61,
        0x0a, 0x91, 0x1f, 0x61, 0x0a, 0x91, 0x4d, 0x61, 0x08, 0xed, 0x24, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x09, 0x04, 0x00,
    ];

    #[test]
    fn parses_and_reencodes_upstream_record_and_file_id() {
        let manifest = MbdbManifest::parse(PYIOSBACKUP_RECORD).expect("upstream fixture");
        assert_eq!(manifest.records.len(), 1);
        let record = &manifest.records[0];
        assert_eq!(record.domain.as_deref(), Some("MyTestDomain"));
        assert_eq!(record.relative_path.as_deref(), Some("Media/Test.txt"));
        assert_eq!(record.mode, 0x81ed);
        assert_eq!(record.size, 9);
        assert_eq!(record.user_id, 501);
        assert_eq!(record.group_id, 501);
        assert_eq!(
            record.file_id().expect("file id"),
            "5727bd1c5fa1055e15d8b4a75a74793c84b5ffdc"
        );
        assert_eq!(
            manifest.serialize(None).expect("serialize"),
            PYIOSBACKUP_RECORD
        );
    }

    #[test]
    fn preserves_nullable_fields_and_unknown_property_order() {
        let record = MbdbRecord {
            domain: Some("d".into()),
            relative_path: Some("p".into()),
            link_target: None,
            data_hash: Some(vec![1, 2]),
            encryption_key: None,
            mode: 0x4000,
            unknown2: 7,
            unknown3: 8,
            user_id: 9,
            group_id: 10,
            mtime: 11,
            atime: 12,
            ctime: 13,
            size: u64::MAX,
            flags: 14,
            properties: vec![(Some("z".into()), None), (None, Some("é".into()))],
        };
        let manifest = MbdbManifest {
            records: vec![record],
        };
        let bytes = manifest.serialize(None).expect("serialize");
        let parsed = MbdbManifest::parse(&bytes).expect("parse");
        assert_eq!(parsed, manifest);
    }

    #[test]
    fn preserves_unicode_identifiers_as_utf8() {
        let manifest = MbdbManifest {
            records: vec![MbdbRecord {
                domain: Some("AppDomain-日本".into()),
                relative_path: Some("Documents/данные.txt".into()),
                link_target: None,
                data_hash: None,
                encryption_key: None,
                mode: 0x81a4,
                unknown2: 0,
                unknown3: 0,
                user_id: 501,
                group_id: 501,
                mtime: 0,
                atime: 0,
                ctime: 0,
                size: 0,
                flags: 0,
                properties: Vec::new(),
            }],
        };
        let encoded = manifest.serialize(None).expect("serialize unicode record");
        let parsed = MbdbManifest::parse(&encoded).expect("parse unicode record");
        assert_eq!(parsed, manifest);
        assert_eq!(parsed.records[0].domain.as_deref(), Some("AppDomain-日本"));
        assert_eq!(
            parsed.records[0].relative_path.as_deref(),
            Some("Documents/данные.txt")
        );
    }

    #[test]
    fn filtering_keeps_original_record_order() {
        let mut first = MbdbManifest::parse(PYIOSBACKUP_RECORD)
            .expect("fixture")
            .records[0]
            .clone();
        first.relative_path = Some("first".into());
        let mut second = first.clone();
        second.relative_path = Some("second".into());
        let manifest = MbdbManifest {
            records: vec![first, second],
        };
        let keep = HashSet::from([manifest.records[1].file_id().expect("id")]);
        let filtered = MbdbManifest::parse(&manifest.serialize(Some(&keep)).expect("serialize"))
            .expect("parse filtered");
        assert_eq!(filtered.records.len(), 1);
        assert_eq!(filtered.records[0].relative_path.as_deref(), Some("second"));
    }

    #[test]
    fn rejects_invalid_header_and_truncated_record() {
        assert!(MbdbManifest::parse(b"mbdb\x05").is_err());
        let mut truncated = PYIOSBACKUP_RECORD.to_vec();
        truncated.pop();
        assert!(MbdbManifest::parse(&truncated).is_err());
    }
}
