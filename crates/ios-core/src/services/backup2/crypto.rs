//! Apple Backup2 encryption primitives used by the host-side Manifest.db helpers.
//!
//! The format implemented here is the modern (iOS 10.3 and newer) format used by
//! `pyiosbackup`: the password derives a 32-byte key, class keys are wrapped with
//! RFC 3394 AES-KW, Manifest.db is AES-256-CBC with a zero IV and no padding, and
//! file payloads use the same CBC primitive with PKCS#7 padding.
//!
//! This module deliberately has no plist or SQLite policy in it.  Callers provide
//! already validated manifest values and paths, which keeps format parsing and
//! filesystem safety independently testable.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use aes::cipher::{Block, BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;
use aes_kw::KekAes256;
use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

const AES_BLOCK_BYTES: usize = 16;
const AES_KEY_BYTES: usize = 32;
const AES_KW_WRAPPED_KEY_BYTES: usize = 40;
const ENCRYPTION_KEY_BYTES: usize = 4 + AES_KW_WRAPPED_KEY_BYTES;

/// The largest BackupKeyBag accepted by the host parser.
///
/// Real keybags are a few kilobytes.  This bound prevents a malformed TLV length
/// from turning a password operation into an unbounded allocation or scan.
pub(crate) const MAX_KEYBAG_BYTES: usize = 1024 * 1024;
/// The largest individual keybag value accepted by the parser.
const MAX_KEYBAG_VALUE_BYTES: usize = 64 * 1024;
/// pyiosbackup fixtures use 10,000,000 iterations for the password stage.
/// Keep that interoperability boundary inclusive while rejecting accidental DoS
/// values above it.
pub(crate) const MAX_PBKDF2_ITERATIONS: u32 = 10_000_000;
const CBC_BUFFER_BYTES: usize = 128 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CryptoError {
    #[error("BackupKeyBag is malformed: {0}")]
    Keybag(&'static str),
    #[error("encrypted backup manifest is malformed: {0}")]
    Manifest(&'static str),
    #[error("encrypted backup payload is malformed: {0}")]
    Payload(&'static str),
    #[error("encrypted backup password or wrapped key is invalid")]
    InvalidPassword,
    #[error("cryptographic operation failed")]
    Crypto,
    #[error("encrypted backup I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A parsed BackupKeyBag and the key used to unwrap Manifest.db/file keys.
///
/// Every key-bearing allocation is wrapped in `Zeroizing`; this type intentionally
/// does not implement `Clone` or expose its internals in a debug representation.
pub(crate) struct BackupCrypto {
    class_keys: HashMap<u32, Zeroizing<[u8; AES_KEY_BYTES]>>,
    manifest_key: Zeroizing<[u8; AES_KEY_BYTES]>,
}

impl BackupCrypto {
    /// Build a modern-backup crypto context from Manifest.plist values.
    pub(crate) fn from_manifest(
        keybag_bytes: &[u8],
        manifest_key_bytes: &[u8],
        password: &str,
        modern: bool,
    ) -> Result<Self, CryptoError> {
        let keybag = ParsedKeybag::parse(keybag_bytes, modern)?;
        let password_key = keybag.derive_password_key(password, modern)?;
        let class_keys = keybag.unwrap_class_keys(&password_key)?;
        let manifest_key = unwrap_encryption_key(&class_keys, manifest_key_bytes, true)?;
        Ok(Self {
            class_keys,
            manifest_key,
        })
    }

    /// Decrypt an AES-CBC/no-padding Manifest.db into `destination`.
    pub(crate) fn decrypt_manifest_file(
        &self,
        source: &Path,
        destination: &Path,
    ) -> Result<u64, CryptoError> {
        let source_file = open_source(source)?;
        let length = source_file.metadata()?.len();
        if length == 0 || length % AES_BLOCK_BYTES as u64 != 0 {
            return Err(CryptoError::Manifest(
                "ciphertext length must be a non-zero multiple of 16",
            ));
        }
        let mut source_file = source_file;
        let mut destination_file = open_destination(destination)?;
        let written = process_cbc_stream(
            &mut source_file,
            &mut destination_file,
            &self.manifest_key,
            false,
            None,
        )?;
        destination_file.sync_all()?;
        Ok(written)
    }

    /// Encrypt a plaintext Manifest.db into `destination` using no padding.
    pub(crate) fn encrypt_manifest_file(
        &self,
        source: &Path,
        destination: &Path,
    ) -> Result<u64, CryptoError> {
        let source_file = open_source(source)?;
        let length = source_file.metadata()?.len();
        if length == 0 || length % AES_BLOCK_BYTES as u64 != 0 {
            return Err(CryptoError::Manifest(
                "plaintext length must be a non-zero multiple of 16",
            ));
        }
        let mut source_file = source_file;
        let mut destination_file = open_destination(destination)?;
        let written = process_cbc_stream(
            &mut source_file,
            &mut destination_file,
            &self.manifest_key,
            true,
            None,
        )?;
        destination_file.sync_all()?;
        Ok(written)
    }

    /// Decrypt one file payload and validate its exact plaintext size and padding.
    pub(crate) fn decrypt_payload_file(
        &self,
        source: &Path,
        destination: &Path,
        encryption_key: &[u8],
        expected_plaintext_len: u64,
    ) -> Result<u64, CryptoError> {
        let source_file = open_source(source)?;
        let ciphertext_len = source_file.metadata()?.len();
        let expected_ciphertext_len = expected_payload_ciphertext_len(expected_plaintext_len)?;
        if ciphertext_len != expected_ciphertext_len {
            return Err(CryptoError::Payload(
                "ciphertext length does not match the metadata plaintext size",
            ));
        }
        let key = unwrap_encryption_key(&self.class_keys, encryption_key, false)?;
        let mut source_file = source_file;
        let mut destination_file = open_destination(destination)?;
        let written = process_cbc_stream(
            &mut source_file,
            &mut destination_file,
            &key,
            false,
            Some(expected_plaintext_len),
        )?;
        destination_file.sync_all()?;
        Ok(written)
    }
}

struct ParsedKeybag {
    root: Vec<TlvRecord>,
    classes: Vec<Vec<TlvRecord>>,
}

#[derive(Clone)]
struct TlvRecord {
    tag: [u8; 4],
    value: TlvValue,
}

#[derive(Clone)]
enum TlvValue {
    Integer(u32),
    Bytes(Vec<u8>),
}

impl ParsedKeybag {
    fn parse(data: &[u8], modern: bool) -> Result<Self, CryptoError> {
        if data.is_empty() || data.len() > MAX_KEYBAG_BYTES {
            return Err(CryptoError::Keybag("size is outside the supported range"));
        }

        let mut records = Vec::new();
        let mut offset = 0usize;
        while offset < data.len() {
            let header_end = offset
                .checked_add(8)
                .ok_or(CryptoError::Keybag("record offset overflow"))?;
            if header_end > data.len() {
                return Err(CryptoError::Keybag("truncated record header"));
            }
            let tag: [u8; 4] = data[offset..offset + 4]
                .try_into()
                .map_err(|_| CryptoError::Keybag("invalid record tag"))?;
            let size = u32::from_be_bytes(
                data[offset + 4..offset + 8]
                    .try_into()
                    .map_err(|_| CryptoError::Keybag("invalid record length"))?,
            );
            let size = usize::try_from(size)
                .map_err(|_| CryptoError::Keybag("record length does not fit host usize"))?;
            if size > MAX_KEYBAG_VALUE_BYTES {
                return Err(CryptoError::Keybag("record value is too large"));
            }
            let value_end = header_end
                .checked_add(size)
                .ok_or(CryptoError::Keybag("record length overflow"))?;
            if value_end > data.len() {
                return Err(CryptoError::Keybag("truncated record value"));
            }
            let value_bytes = &data[header_end..value_end];
            let value = if size == 4 {
                TlvValue::Integer(u32::from_be_bytes(
                    value_bytes
                        .try_into()
                        .map_err(|_| CryptoError::Keybag("invalid integer value"))?,
                ))
            } else {
                TlvValue::Bytes(value_bytes.to_vec())
            };
            records.push(TlvRecord { tag, value });
            offset = value_end;
        }
        if offset != data.len() {
            return Err(CryptoError::Keybag("trailing bytes after records"));
        }

        let first_class = records
            .iter()
            .position(|record| record.tag == *b"CLAS")
            .ok_or(CryptoError::Keybag("no class records"))?;
        let class_start = first_class
            .checked_sub(1)
            .ok_or(CryptoError::Keybag("root records are missing"))?;
        if records[class_start].tag != *b"UUID" {
            return Err(CryptoError::Keybag("root records are missing"));
        }
        let class_records = &records[class_start..];
        if class_records.len() % 5 != 0 {
            return Err(CryptoError::Keybag(
                "class records are not complete five-field groups",
            ));
        }
        let root = records[..class_start].to_vec();
        reject_duplicate_tags(&root, "root")?;
        let mut classes = Vec::new();
        for group in class_records.chunks_exact(5) {
            let class = group.to_vec();
            reject_duplicate_tags(&class, "class")?;
            let class_map = unique_map(&class)?;
            require_bytes_len(&class_map, b"UUID", 16, "class UUID must be 16 bytes")?;
            require_integer(&class_map, b"CLAS", "class identifier")?;
            let wrap_flags = require_integer(&class_map, b"WRAP", "class wrap flags")?;
            require_integer(&class_map, b"KTYP", "class key type")?;
            if wrap_flags & 2 != 0 {
                require_bytes_len(
                    &class_map,
                    b"WPKY",
                    AES_KW_WRAPPED_KEY_BYTES,
                    "wrapped class key must be 40 bytes",
                )?;
            }
            classes.push(class);
        }

        let root_map = unique_map(&root)?;
        require_bytes_len(&root_map, b"SALT", 20, "root salt must be 20 bytes")?;
        let iterations = require_integer(&root_map, b"ITER", "root iterations")?;
        check_iterations(iterations)?;
        if modern {
            require_bytes_len(
                &root_map,
                b"DPSL",
                20,
                "modern password salt must be 20 bytes",
            )?;
            let password_iterations = require_integer(&root_map, b"DPIC", "modern iterations")?;
            check_iterations(password_iterations)?;
        }
        Ok(Self { root, classes })
    }

    fn derive_password_key(
        &self,
        password: &str,
        modern: bool,
    ) -> Result<Zeroizing<[u8; AES_KEY_BYTES]>, CryptoError> {
        let root = unique_map(&self.root)?;
        let salt = require_bytes_len(&root, b"SALT", 20, "root salt must be 20 bytes")?;
        let iterations = require_integer(&root, b"ITER", "root iterations")?;
        check_iterations(iterations)?;

        let mut first_stage = Zeroizing::new([0u8; AES_KEY_BYTES]);
        if modern {
            let password_salt =
                require_bytes_len(&root, b"DPSL", 20, "modern password salt must be 20 bytes")?;
            let password_iterations = require_integer(&root, b"DPIC", "modern iterations")?;
            check_iterations(password_iterations)?;
            pbkdf2_hmac::<Sha256>(
                password.as_bytes(),
                password_salt,
                password_iterations,
                first_stage.as_mut(),
            );
        } else {
            // The legacy one-stage form is retained in this primitive for format
            // tests and interoperability, although local Manifest.mbdb operations
            // reject <=10.2 backups at the policy layer.
            first_stage.copy_from_slice(&[0u8; AES_KEY_BYTES]);
            pbkdf2_hmac::<Sha1>(password.as_bytes(), salt, iterations, first_stage.as_mut());
            return Ok(first_stage);
        }

        let mut result = Zeroizing::new([0u8; AES_KEY_BYTES]);
        pbkdf2_hmac::<Sha1>(first_stage.as_ref(), salt, iterations, result.as_mut());
        Ok(result)
    }

    fn unwrap_class_keys(
        &self,
        password_key: &[u8; AES_KEY_BYTES],
    ) -> Result<HashMap<u32, Zeroizing<[u8; AES_KEY_BYTES]>>, CryptoError> {
        let mut class_keys = HashMap::new();
        for class in &self.classes {
            let map = unique_map(class)?;
            let class_id = require_integer(&map, b"CLAS", "class identifier")?;
            let wrap_flags = require_integer(&map, b"WRAP", "class wrap flags")?;
            if wrap_flags & 2 == 0 {
                continue;
            }
            let wrapped = require_bytes(&map, b"WPKY", "wrapped class key")?;
            let key = unwrap_aes_kw(password_key, wrapped)?;
            if class_keys.insert(class_id, key).is_some() {
                return Err(CryptoError::Keybag("duplicate class identifier"));
            }
        }
        if class_keys.is_empty() {
            return Err(CryptoError::Keybag("no wrapped class keys"));
        }
        Ok(class_keys)
    }
}

fn reject_duplicate_tags(records: &[TlvRecord], scope: &'static str) -> Result<(), CryptoError> {
    let mut seen = Vec::new();
    for record in records {
        if seen.contains(&record.tag) {
            return Err(CryptoError::Keybag(match scope {
                "root" => "duplicate root tag",
                _ => "duplicate class tag",
            }));
        }
        seen.push(record.tag);
    }
    Ok(())
}

fn unique_map(records: &[TlvRecord]) -> Result<HashMap<[u8; 4], &TlvValue>, CryptoError> {
    let mut map = HashMap::new();
    for record in records {
        if map.insert(record.tag, &record.value).is_some() {
            return Err(CryptoError::Keybag("duplicate keybag tag"));
        }
    }
    Ok(map)
}

fn require_integer<'a>(
    map: &'a HashMap<[u8; 4], &'a TlvValue>,
    tag: &[u8; 4],
    name: &'static str,
) -> Result<u32, CryptoError> {
    match map.get(tag) {
        Some(TlvValue::Integer(value)) => Ok(*value),
        Some(TlvValue::Bytes(_)) => Err(CryptoError::Keybag("integer field has wrong size")),
        None => Err(CryptoError::Keybag(name)),
    }
}

fn require_bytes<'a>(
    map: &'a HashMap<[u8; 4], &'a TlvValue>,
    tag: &[u8; 4],
    name: &'static str,
) -> Result<&'a [u8], CryptoError> {
    match map.get(tag) {
        Some(TlvValue::Bytes(value)) if !value.is_empty() => Ok(value),
        Some(TlvValue::Bytes(_)) => Err(CryptoError::Keybag("byte field is empty")),
        Some(TlvValue::Integer(_)) => Err(CryptoError::Keybag("byte field has wrong size")),
        None => Err(CryptoError::Keybag(name)),
    }
}

fn require_bytes_len<'a>(
    map: &'a HashMap<[u8; 4], &'a TlvValue>,
    tag: &[u8; 4],
    expected: usize,
    name: &'static str,
) -> Result<&'a [u8], CryptoError> {
    let value = require_bytes(map, tag, name)?;
    if value.len() != expected {
        return Err(CryptoError::Keybag(name));
    }
    Ok(value)
}

fn check_iterations(iterations: u32) -> Result<(), CryptoError> {
    if iterations == 0 || iterations > MAX_PBKDF2_ITERATIONS {
        return Err(CryptoError::Keybag(
            "PBKDF2 iteration count is outside the safety limit",
        ));
    }
    Ok(())
}

fn unwrap_encryption_key(
    class_keys: &HashMap<u32, Zeroizing<[u8; AES_KEY_BYTES]>>,
    encoded: &[u8],
    manifest: bool,
) -> Result<Zeroizing<[u8; AES_KEY_BYTES]>, CryptoError> {
    if encoded.len() != ENCRYPTION_KEY_BYTES {
        return Err(if manifest {
            CryptoError::Manifest(
                "ManifestKey must contain a little-endian class and 40-byte AES-KW key",
            )
        } else {
            CryptoError::Payload(
                "EncryptionKey must contain a little-endian class and 40-byte AES-KW key",
            )
        });
    }
    let class_id = u32::from_le_bytes(encoded[..4].try_into().map_err(|_| CryptoError::Crypto)?);
    let wrapping_key = class_keys
        .get(&class_id)
        .ok_or(CryptoError::InvalidPassword)?;
    unwrap_aes_kw(wrapping_key.as_ref(), &encoded[4..])
}

fn unwrap_aes_kw(
    wrapping_key: &[u8],
    wrapped: &[u8],
) -> Result<Zeroizing<[u8; AES_KEY_BYTES]>, CryptoError> {
    if wrapping_key.len() != AES_KEY_BYTES
        || wrapped.len() != AES_KW_WRAPPED_KEY_BYTES
        || wrapped.len() % 8 != 0
    {
        return Err(CryptoError::InvalidPassword);
    }
    let cipher = KekAes256::try_from(wrapping_key).map_err(|_| CryptoError::Crypto)?;
    let mut unwrapped = Zeroizing::new([0u8; AES_KEY_BYTES]);
    cipher
        .unwrap(wrapped, unwrapped.as_mut())
        .map_err(|_| CryptoError::InvalidPassword)?;
    Ok(unwrapped)
}

/// Return the exact ciphertext length for PKCS#7-padded AES-CBC data.
pub(crate) fn expected_payload_ciphertext_len(plaintext_len: u64) -> Result<u64, CryptoError> {
    let blocks = plaintext_len
        .checked_div(AES_BLOCK_BYTES as u64)
        .and_then(|value| value.checked_add(1))
        .ok_or(CryptoError::Payload("plaintext length overflow"))?;
    blocks
        .checked_mul(AES_BLOCK_BYTES as u64)
        .ok_or(CryptoError::Payload("ciphertext length overflow"))
}

fn process_cbc_stream<R: Read, W: Write>(
    source: &mut R,
    destination: &mut W,
    key: &[u8; AES_KEY_BYTES],
    encrypt: bool,
    expected_plaintext_len: Option<u64>,
) -> Result<u64, CryptoError> {
    let cipher = Aes256::new_from_slice(key).map_err(|_| CryptoError::Crypto)?;
    let mut previous = [0u8; AES_BLOCK_BYTES];
    let mut input = vec![0u8; CBC_BUFFER_BYTES];
    let mut output = vec![0u8; CBC_BUFFER_BYTES];
    let mut pending_plaintext = [0u8; AES_BLOCK_BYTES];
    let mut pending = false;
    let mut total_input = 0u64;
    let mut total_output = 0u64;

    loop {
        let read = read_aligned_chunk(source, &mut input)?;
        if read == 0 {
            break;
        }
        total_input = total_input
            .checked_add(u64::try_from(read).map_err(|_| CryptoError::Crypto)?)
            .ok_or(CryptoError::Payload("CBC input length overflow"))?;
        for (in_block, out_block) in input[..read]
            .chunks_exact(AES_BLOCK_BYTES)
            .zip(output[..read].chunks_exact_mut(AES_BLOCK_BYTES))
        {
            let mut block = Block::<Aes256>::clone_from_slice(in_block);
            if encrypt {
                for (value, prior) in block.iter_mut().zip(previous) {
                    *value ^= prior;
                }
                cipher.encrypt_block(&mut block);
                previous.copy_from_slice(&block);
                out_block.copy_from_slice(&block);
            } else {
                let ciphertext = block;
                cipher.decrypt_block(&mut block);
                for (value, prior) in block.iter_mut().zip(previous) {
                    *value ^= prior;
                }
                previous.copy_from_slice(&ciphertext);
                if expected_plaintext_len.is_none() {
                    destination.write_all(&block)?;
                    total_output = total_output
                        .checked_add(AES_BLOCK_BYTES as u64)
                        .ok_or(CryptoError::Manifest("CBC output length overflow"))?;
                    continue;
                }
                if pending {
                    destination.write_all(&pending_plaintext)?;
                    total_output = total_output
                        .checked_add(AES_BLOCK_BYTES as u64)
                        .ok_or(CryptoError::Payload("CBC output length overflow"))?;
                }
                pending_plaintext.copy_from_slice(&block);
                pending = true;
                // The decrypted block is held back for strict PKCS#7 validation.
                continue;
            }
        }
        if encrypt {
            destination.write_all(&output[..read])?;
            total_output = total_output
                .checked_add(u64::try_from(read).map_err(|_| CryptoError::Crypto)?)
                .ok_or(CryptoError::Payload("CBC output length overflow"))?;
        }
    }

    if encrypt {
        if total_input == 0 || total_input % AES_BLOCK_BYTES as u64 != 0 {
            return Err(CryptoError::Manifest(
                "CBC input must be a non-zero multiple of 16",
            ));
        }
        return Ok(total_output);
    }
    if !pending || total_input == 0 || total_input % AES_BLOCK_BYTES as u64 != 0 {
        if expected_plaintext_len.is_none() && total_input > 0 {
            return Ok(total_output);
        }
        return Err(CryptoError::Payload(
            "CBC ciphertext must be a non-zero multiple of 16",
        ));
    }
    let padding = usize::from(pending_plaintext[AES_BLOCK_BYTES - 1]);
    if padding == 0 || padding > AES_BLOCK_BYTES {
        return Err(CryptoError::Payload("invalid PKCS#7 padding"));
    }
    if pending_plaintext[AES_BLOCK_BYTES - padding..]
        .iter()
        .any(|value| usize::from(*value) != padding)
    {
        return Err(CryptoError::Payload("invalid PKCS#7 padding"));
    }
    let final_plaintext = AES_BLOCK_BYTES - padding;
    if let Some(expected) = expected_plaintext_len {
        let plaintext_len = total_output
            .checked_add(u64::try_from(final_plaintext).map_err(|_| CryptoError::Crypto)?)
            .ok_or(CryptoError::Payload("plaintext length overflow"))?;
        if plaintext_len != expected {
            return Err(CryptoError::Payload(
                "decrypted payload size does not match Manifest.db",
            ));
        }
    }
    destination.write_all(&pending_plaintext[..final_plaintext])?;
    total_output
        .checked_add(u64::try_from(final_plaintext).map_err(|_| CryptoError::Crypto)?)
        .ok_or(CryptoError::Payload("CBC output length overflow"))
}

fn read_aligned_chunk<R: Read>(source: &mut R, buffer: &mut [u8]) -> Result<usize, CryptoError> {
    debug_assert_eq!(buffer.len() % AES_BLOCK_BYTES, 0);
    let mut read = 0usize;
    while read < buffer.len() {
        let count = source.read(&mut buffer[read..])?;
        if count == 0 {
            break;
        }
        read = read.checked_add(count).ok_or(CryptoError::Crypto)?;
    }
    if read % AES_BLOCK_BYTES != 0 {
        return Err(CryptoError::Manifest("CBC stream ended on a partial block"));
    }
    Ok(read)
}

fn open_source(path: &Path) -> Result<File, CryptoError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(path)?)
}

fn open_destination(path: &Path) -> Result<File, CryptoError> {
    let mut options = OpenOptions::new();
    options.write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(path)?)
}

impl Drop for ParsedKeybag {
    fn drop(&mut self) {
        for record in &mut self.root {
            if let TlvValue::Bytes(value) = &mut record.value {
                value.zeroize();
            }
        }
        for class in &mut self.classes {
            for record in class {
                if let TlvValue::Bytes(value) = &mut record.value {
                    value.zeroize();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn record(tag: &[u8; 4], value: &[u8]) -> Vec<u8> {
        let mut result = Vec::new();
        result.extend_from_slice(tag);
        result.extend_from_slice(&(u32::try_from(value.len()).expect("test length")).to_be_bytes());
        result.extend_from_slice(value);
        result
    }

    fn integer_record(tag: &[u8; 4], value: u32) -> Vec<u8> {
        record(tag, &value.to_be_bytes())
    }

    fn fixture_keybag() -> Vec<u8> {
        let mut result = Vec::new();
        result.extend(integer_record(b"VERS", 5));
        result.extend(integer_record(b"TYPE", 1));
        result.extend(record(b"UUID", &[0u8; 16]));
        result.extend(record(b"HMCK", &[0u8; 40]));
        result.extend(integer_record(b"WRAP", 0));
        result.extend(record(b"SALT", &[0u8; 20]));
        result.extend(integer_record(b"ITER", 10_000));
        result.extend(integer_record(b"DPWT", 1));
        result.extend(integer_record(b"DPIC", 10_000_000));
        result.extend(record(b"DPSL", &[0u8; 20]));
        for class_id in 1..=11 {
            result.extend(record(b"UUID", &[0u8; 16]));
            result.extend(integer_record(b"CLAS", class_id));
            result.extend(integer_record(b"WRAP", 3));
            result.extend(integer_record(b"KTYP", 0));
            result.extend(record(
                b"WPKY",
                &hex_bytes(
                    "528c9c5171bf8bef1ea397cf29d2a838c90529df02a6ad2f823a1c6f3a5b2040e2a309c520c6a2ab",
                ),
            ));
        }
        result
    }

    fn hex_bytes(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|offset| u8::from_str_radix(&value[offset..offset + 2], 16).expect("hex"))
            .collect()
    }

    fn manifest_key_fixture() -> Vec<u8> {
        let mut key = vec![2, 0, 0, 0];
        key.extend(hex_bytes(
            "97317494343807e690fd1e431413963dc0e3deb4907fb89fa36ce65126d0ea1301381fb3a2941e2f",
        ));
        key
    }

    #[test]
    #[ignore = "10M PBKDF2 interoperability vector; run with --ignored"]
    fn pyiosbackup_keybag_and_payload_vector() {
        // Source: pyiosbackup 0.2.4 tests/test_keybag.py and tests/conftest.py.
        let crypto =
            BackupCrypto::from_manifest(&fixture_keybag(), &manifest_key_fixture(), "0000", true)
                .expect("public keybag fixture");
        let encrypted = hex_bytes("78b51ca5374c3ad575174288688cda49");
        let root = fixture_root("payload");
        fs::create_dir_all(&root).expect("fixture root");
        let source = root.join("encrypted");
        let destination = root.join("decrypted");
        fs::write(&source, encrypted).expect("source");
        fs::File::create(&destination).expect("destination");
        // EncryptionKey = little-endian class 2 plus the fixture's AES-KW blob.
        let key = {
            let mut value = vec![2, 0, 0, 0];
            value.extend(hex_bytes(
                "97317494343807e690fd1e431413963dc0e3deb4907fb89fa36ce65126d0ea1301381fb3a2941e2f",
            ));
            value
        };
        let decrypted = crypto
            .decrypt_payload_file(&source, &destination, &key, 9)
            .expect("payload vector");
        assert_eq!(decrypted, 9);
        assert_eq!(fs::read(&destination).expect("plaintext"), b"Test data");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn malformed_keybag_lengths_duplicates_and_iteration_bounds_are_rejected() {
        let mut truncated = fixture_keybag();
        truncated.pop();
        assert!(ParsedKeybag::parse(&truncated, true).is_err());

        let mut duplicate = fixture_keybag();
        duplicate.splice(0..0, integer_record(b"VERS", 6));
        assert!(ParsedKeybag::parse(&duplicate, true).is_err());

        let mut too_many = fixture_keybag();
        // Replace the first DPIC integer while preserving the strict TLV shape.
        let marker = [b'D', b'P', b'I', b'C', 0, 0, 0, 4];
        let position = too_many
            .windows(marker.len())
            .position(|window| window == marker)
            .expect("DPIC");
        too_many[position + 8..position + 12].copy_from_slice(&10_000_001u32.to_be_bytes());
        assert!(ParsedKeybag::parse(&too_many, true).is_err());

        let mut exact_limit = fixture_keybag();
        let marker = integer_record(b"ITER", 10_000);
        let position = exact_limit
            .windows(marker.len())
            .position(|window| window == marker)
            .expect("ITER");
        exact_limit[position + 8..position + 12]
            .copy_from_slice(&MAX_PBKDF2_ITERATIONS.to_be_bytes());
        ParsedKeybag::parse(&exact_limit, true).expect("inclusive iteration limit");

        let mut overflowing = fixture_keybag();
        let position = overflowing
            .windows(marker.len())
            .position(|window| window == marker)
            .expect("ITER");
        overflowing[position + 8..position + 12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(ParsedKeybag::parse(&overflowing, true).is_err());
    }

    #[test]
    fn aes_kw_fixture_blob_unwraps_with_derived_key() {
        let kek = KekAes256::try_from(
            &hex_bytes("ec8edc160b5745ccae000bfb4787630f322821191fba7eea38feab2c6307cd9b")[..],
        )
        .expect("kek");
        let wrapped = hex_bytes(
            "528c9c5171bf8bef1ea397cf29d2a838c90529df02a6ad2f823a1c6f3a5b2040e2a309c520c6a2ab",
        );
        let mut key = [0u8; 32];
        kek.unwrap(&wrapped, &mut key).expect("fixture unwrap");
        assert_eq!(key, [0u8; 32]);
    }

    #[test]
    fn aes_kw_integrity_failure_is_rejected() {
        let mut wrapped = hex_bytes(
            "528c9c5171bf8bef1ea397cf29d2a838c90529df02a6ad2f823a1c6f3a5b2040e2a309c520c6a2ab",
        );
        wrapped[0] ^= 1;
        assert!(matches!(
            unwrap_aes_kw(&[0u8; 32], &wrapped),
            Err(CryptoError::InvalidPassword)
        ));
    }

    #[test]
    fn fixture_keybag_classes_unwrap_with_known_password_key() {
        let keybag = ParsedKeybag::parse(&fixture_keybag(), true).expect("parse");
        let key = hex_bytes("ec8edc160b5745ccae000bfb4787630f322821191fba7eea38feab2c6307cd9b");
        let key: [u8; 32] = key.try_into().expect("key");
        let classes = keybag.unwrap_class_keys(&key).expect("classes");
        assert_eq!(classes.len(), 11);
    }

    fn zero_crypto() -> BackupCrypto {
        BackupCrypto {
            class_keys: HashMap::from([(2, Zeroizing::new([0u8; 32]))]),
            manifest_key: Zeroizing::new([0u8; 32]),
        }
    }

    fn zero_class_encryption_key(class_id: u32) -> Vec<u8> {
        let kek = KekAes256::from([0u8; 32]);
        let mut wrapped = [0u8; 40];
        kek.wrap(&[0u8; 32], &mut wrapped).expect("wrap test key");
        let mut result = class_id.to_le_bytes().to_vec();
        result.extend_from_slice(&wrapped);
        result
    }

    #[test]
    fn cbc_stream_round_trip_and_strict_payload_failures() {
        let crypto = zero_crypto();
        let root = fixture_root("cbc");
        fs::create_dir_all(&root).expect("root");

        let plain = root.join("plain");
        let encrypted_manifest = root.join("manifest-encrypted");
        let decrypted_manifest = root.join("manifest-decrypted");
        fs::write(&plain, [0x42u8; 32]).expect("plain");
        fs::File::create(&encrypted_manifest).expect("encrypted manifest");
        crypto
            .encrypt_manifest_file(&plain, &encrypted_manifest)
            .expect("manifest encryption");
        fs::File::create(&decrypted_manifest).expect("decrypted manifest");
        crypto
            .decrypt_manifest_file(&encrypted_manifest, &decrypted_manifest)
            .expect("manifest decryption");
        assert_eq!(
            fs::read(&decrypted_manifest).expect("round trip"),
            vec![0x42; 32]
        );

        let key = zero_class_encryption_key(2);
        let payload = root.join("payload");
        let destination = root.join("payload-plain");
        fs::write(&payload, hex_bytes("78b51ca5374c3ad575174288688cda49")).expect("payload");
        fs::File::create(&destination).expect("payload destination");
        assert_eq!(
            crypto
                .decrypt_payload_file(&payload, &destination, &key, 9)
                .expect("payload decryption"),
            9
        );
        assert_eq!(
            fs::read(&destination).expect("payload plaintext"),
            b"Test data"
        );

        let bad_padding = root.join("bad-padding");
        let mut ciphertext = hex_bytes("78b51ca5374c3ad575174288688cda49");
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 1;
        fs::write(&bad_padding, ciphertext).expect("bad padding");
        fs::File::create(root.join("bad-padding-out")).expect("bad destination");
        assert!(crypto
            .decrypt_payload_file(&bad_padding, &root.join("bad-padding-out"), &key, 9)
            .is_err());

        let missing_class = zero_class_encryption_key(3);
        fs::File::create(root.join("missing-class-out")).expect("missing destination");
        assert!(crypto
            .decrypt_payload_file(&payload, &root.join("missing-class-out"), &missing_class, 9)
            .is_err());

        let non_aligned = root.join("non-aligned");
        fs::write(&non_aligned, [0u8; 15]).expect("non-aligned");
        fs::File::create(root.join("non-aligned-out")).expect("non-aligned destination");
        assert!(crypto
            .decrypt_manifest_file(&non_aligned, &root.join("non-aligned-out"))
            .is_err());

        let empty_payload = root.join("empty-payload");
        let cipher = Aes256::new_from_slice(&[0u8; 32]).expect("AES key");
        let mut block = Block::<Aes256>::clone_from_slice(&[0x10u8; 16]);
        cipher.encrypt_block(&mut block);
        fs::write(&empty_payload, block).expect("empty payload");
        let empty_output = root.join("empty-out");
        fs::File::create(&empty_output).expect("empty destination");
        assert_eq!(
            crypto
                .decrypt_payload_file(&empty_payload, &empty_output, &key, 0)
                .expect("empty decryption"),
            0
        );
        assert!(fs::read(&empty_output).expect("empty plaintext").is_empty());
        assert_eq!(expected_payload_ciphertext_len(0).expect("empty size"), 16);

        // A plaintext whose length is exactly one block receives a complete PKCS#7 block.
        let full_block_payload = root.join("full-block-payload");
        let full_block_output = root.join("full-block-output");
        let cipher = Aes256::new_from_slice(&[0u8; 32]).expect("AES key");
        let mut previous = [0u8; 16];
        let mut ciphertext = Vec::new();
        for plaintext in [[0x42u8; 16], [0x10u8; 16]] {
            let mut block = Block::<Aes256>::clone_from_slice(&plaintext);
            for (value, prior) in block.iter_mut().zip(previous) {
                *value ^= prior;
            }
            cipher.encrypt_block(&mut block);
            previous.copy_from_slice(&block);
            ciphertext.extend_from_slice(&block);
        }
        fs::write(&full_block_payload, ciphertext).expect("full-block payload");
        fs::File::create(&full_block_output).expect("full-block destination");
        assert_eq!(
            crypto
                .decrypt_payload_file(&full_block_payload, &full_block_output, &key, 16)
                .expect("full-block decryption"),
            16
        );
        assert_eq!(
            fs::read(&full_block_output).expect("full-block plaintext"),
            [0x42; 16]
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn checked_payload_sizes_cover_empty_and_u64_boundaries() {
        assert_eq!(expected_payload_ciphertext_len(0).expect("empty"), 16);
        assert_eq!(expected_payload_ciphertext_len(9).expect("small"), 16);
        assert_eq!(expected_payload_ciphertext_len(16).expect("block"), 32);
        assert!(expected_payload_ciphertext_len(u64::MAX).is_err());
    }

    fn fixture_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ios-core-backup2-crypto-{}-{label}-{nonce}",
            std::process::id()
        ))
    }
}
