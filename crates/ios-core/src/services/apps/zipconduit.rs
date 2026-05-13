//! Streaming Zip Conduit – fast IPA installation via `com.apple.streaming_zip_conduit`.
//!
//! Protocol (from go-ios `ios/zipconduit/`):
//!   1. Send InitTransfer plist (4-byte BE length prefix)
//!   2. Stream ZIP entries (local file headers + data, no compression, no central directory)
//!   3. Send META-INF/ dir + com.apple.ZipMetadata.plist with record counts
//!   4. Send central directory header signature as terminator
//!   5. Poll for DataComplete / progress / error responses

use std::io::Read;
use std::path::Path;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.streaming_zip_conduit";
pub const RSD_SERVICE_NAME: &str = "com.apple.streaming_zip_conduit.shim.remote";

#[derive(Debug, thiserror::Error)]
pub enum ZipConduitError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(#[from] plist::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("zip error: {0}")]
    Zip(String),
    #[error("install error: {0}")]
    Install(String),
}

/// Progress callback for installation status.
pub type ProgressCallback = Box<dyn Fn(u32, &str) + Send>;

/// Install an IPA via Streaming Zip Conduit.
///
/// `stream` must be an already-connected streaming_zip_conduit service connection.
/// `ipa_path` is the local path to the IPA file.
/// `progress` is an optional callback for installation progress updates.
pub async fn install_ipa<S>(
    stream: &mut S,
    ipa_path: &Path,
    progress: Option<ProgressCallback>,
) -> Result<(), ZipConduitError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let filename = ipa_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app.ipa");

    // 1. Read and extract the IPA entries (decompress in memory)
    let ipa_data = tokio::fs::read(ipa_path).await?;
    let entries = extract_zip_entries(&ipa_data)?;

    // Calculate totals for ZipMetadata
    let total_uncompressed: u64 = entries.iter().map(|e| e.data.len() as u64).sum();
    // RecordCount = META-INF dir + ZipMetadata file + all entries
    let record_count = 2 + entries.len() as u64;

    // 2. Send InitTransfer plist
    let init_plist = build_init_transfer(filename);
    send_plist(stream, &init_plist).await?;

    // 3. Stream ZIP data

    // 3a. META-INF/ directory entry
    write_zip_dir_entry(stream, "META-INF/").await?;

    // 3b. com.apple.ZipMetadata.plist
    let metadata = build_zip_metadata(record_count, total_uncompressed);
    let metadata_bytes = plist_to_xml_bytes(&metadata)?;
    write_zip_file_entry(
        stream,
        "META-INF/com.apple.ZipMetadata.plist",
        &metadata_bytes,
    )
    .await?;

    // 3c. All files/dirs from the IPA
    for entry in &entries {
        if entry.is_dir {
            write_zip_dir_entry(stream, &entry.name).await?;
        } else {
            write_zip_file_entry(stream, &entry.name, &entry.data).await?;
        }
    }

    // 4. Send central directory header signature as terminator
    stream.write_all(&[0x50, 0x4b, 0x01, 0x02]).await?;
    stream.flush().await?;

    // 5. Poll for completion
    loop {
        let resp = recv_plist(stream).await?;

        if let Some(status) = resp
            .as_dictionary()
            .and_then(|d| d.get("Status"))
            .and_then(|v| v.as_string())
        {
            if status == "DataComplete" {
                return Ok(());
            }
        }

        if let Some(progress_dict) = resp
            .as_dictionary()
            .and_then(|d| d.get("InstallProgressDict"))
            .and_then(|v| v.as_dictionary())
        {
            if let Some(error) = progress_dict.get("Error").and_then(|v| v.as_string()) {
                let desc = progress_dict
                    .get("ErrorDescription")
                    .and_then(|v| v.as_string())
                    .unwrap_or("unknown");
                return Err(ZipConduitError::Install(format!("{error}: {desc}")));
            }

            let percent = progress_dict
                .get("PercentComplete")
                .and_then(|v| v.as_unsigned_integer())
                .unwrap_or(0) as u32;
            let status = progress_dict
                .get("Status")
                .and_then(|v| v.as_string())
                .unwrap_or("Unknown");

            if status == "Complete" {
                return Ok(());
            }

            if let Some(ref cb) = progress {
                cb(percent, status);
            }
        }
    }
}

// ── ZIP entry types ─────────────────────────────────────────────────────────

struct ZipEntry {
    name: String,
    is_dir: bool,
    data: Vec<u8>,
}

/// Extract all entries from a ZIP file, decompressing as needed.
fn extract_zip_entries(data: &[u8]) -> Result<Vec<ZipEntry>, ZipConduitError> {
    let reader = std::io::Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| ZipConduitError::Zip(e.to_string()))?;

    let mut entries = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| ZipConduitError::Zip(e.to_string()))?;
        let name = file.name().to_string();
        let is_dir = file.is_dir();

        let mut file_data = Vec::new();
        if !is_dir {
            file.read_to_end(&mut file_data)
                .map_err(|e| ZipConduitError::Zip(format!("failed to read {name}: {e}")))?;
        }

        entries.push(ZipEntry {
            name,
            is_dir,
            data: file_data,
        });
    }
    Ok(entries)
}

// ── ZIP local file header writer ────────────────────────────────────────────

/// Fixed timestamp values (from Xcode captures).
const FIXED_MOD_TIME: u16 = 0xBDEF;
const FIXED_MOD_DATE: u16 = 0x52EC;

/// UT extended timestamp extra field (32 bytes, from Xcode capture).
const EXTRA_FIELD: [u8; 32] = [
    0x55, 0x54, 0x0D, 0x00, 0x07, 0xF3, 0xA2, 0xEC, 0x60, 0xF6, 0xA2, 0xEC, 0x60, 0xF3, 0xA2, 0xEC,
    0x60, 0x75, 0x78, 0x0B, 0x00, 0x01, 0x04, 0xF5, 0x01, 0x00, 0x00, 0x04, 0x14, 0x00, 0x00, 0x00,
];

async fn write_zip_dir_entry<S: AsyncWrite + Unpin>(
    stream: &mut S,
    name: &str,
) -> Result<(), ZipConduitError> {
    write_local_file_header(stream, name, 0, 0, 0).await
}

async fn write_zip_file_entry<S: AsyncWrite + Unpin>(
    stream: &mut S,
    name: &str,
    data: &[u8],
) -> Result<(), ZipConduitError> {
    let crc = crc32fast::hash(data);
    let size = data.len() as u32;
    write_local_file_header(stream, name, crc, size, size).await?;
    stream.write_all(data).await?;
    Ok(())
}

async fn write_local_file_header<S: AsyncWrite + Unpin>(
    stream: &mut S,
    filename: &str,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
) -> Result<(), ZipConduitError> {
    let name_bytes = filename.as_bytes();
    let mut header = Vec::with_capacity(30 + name_bytes.len() + EXTRA_FIELD.len());

    // Local file header signature
    header.extend_from_slice(&0x04034b50u32.to_le_bytes());
    // Version needed to extract
    header.extend_from_slice(&20u16.to_le_bytes());
    // General purpose bit flags
    header.extend_from_slice(&0u16.to_le_bytes());
    // Compression method (0 = STORE)
    header.extend_from_slice(&0u16.to_le_bytes());
    // Last modified time
    header.extend_from_slice(&FIXED_MOD_TIME.to_le_bytes());
    // Last modified date
    header.extend_from_slice(&FIXED_MOD_DATE.to_le_bytes());
    // CRC-32
    header.extend_from_slice(&crc32.to_le_bytes());
    // Compressed size
    header.extend_from_slice(&compressed_size.to_le_bytes());
    // Uncompressed size
    header.extend_from_slice(&uncompressed_size.to_le_bytes());
    // Filename length
    header.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    // Extra field length
    header.extend_from_slice(&(EXTRA_FIELD.len() as u16).to_le_bytes());
    // Filename
    header.extend_from_slice(name_bytes);
    // Extra field
    header.extend_from_slice(&EXTRA_FIELD);

    stream.write_all(&header).await?;
    Ok(())
}

// ── Plist helpers ───────────────────────────────────────────────────────────

fn build_init_transfer(filename: &str) -> plist::Value {
    let mut dict = plist::Dictionary::new();
    dict.insert(
        "InstallTransferredDirectory".to_string(),
        plist::Value::Integer(1.into()),
    );
    dict.insert(
        "UserInitiatedTransfer".to_string(),
        plist::Value::Integer(0.into()),
    );
    dict.insert(
        "MediaSubdir".to_string(),
        plist::Value::String(format!("PublicStaging/{filename}")),
    );

    let mut options = plist::Dictionary::new();
    options.insert(
        "InstallDeltaTypeKey".to_string(),
        plist::Value::String("InstallDeltaTypeSparseIPAFiles".to_string()),
    );
    options.insert(
        "DisableDeltaTransfer".to_string(),
        plist::Value::Integer(1.into()),
    );
    options.insert(
        "IsUserInitiated".to_string(),
        plist::Value::Integer(1.into()),
    );
    options.insert("PreferWifi".to_string(), plist::Value::Integer(1.into()));
    options.insert(
        "PackageType".to_string(),
        plist::Value::String("Customer".to_string()),
    );
    dict.insert(
        "InstallOptionsDictionary".to_string(),
        plist::Value::Dictionary(options),
    );

    plist::Value::Dictionary(dict)
}

fn build_zip_metadata(record_count: u64, total_uncompressed: u64) -> plist::Value {
    let mut dict = plist::Dictionary::new();
    dict.insert(
        "RecordCount".to_string(),
        plist::Value::Integer((record_count as i64).into()),
    );
    dict.insert(
        "StandardDirectoryPerms".to_string(),
        plist::Value::Integer(16877.into()), // 0o40755
    );
    dict.insert(
        "StandardFilePerms".to_string(),
        plist::Value::Integer((-32348i64).into()), // 0o37777700644 as signed
    );
    dict.insert(
        "TotalUncompressedBytes".to_string(),
        plist::Value::Integer((total_uncompressed as i64).into()),
    );
    dict.insert("Version".to_string(), plist::Value::Integer(2.into()));
    plist::Value::Dictionary(dict)
}

fn plist_to_xml_bytes(value: &plist::Value) -> Result<Vec<u8>, ZipConduitError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value)?;
    Ok(buf)
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), ZipConduitError> {
    let buf = plist_to_xml_bytes(value)?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(stream: &mut S) -> Result<plist::Value, ZipConduitError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 4 * 1024 * 1024 {
        return Err(ZipConduitError::Protocol(format!("plist too large: {len}")));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(plist::from_bytes(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_init_transfer_has_correct_fields() {
        let plist = build_init_transfer("Example.ipa");
        let dict = plist.as_dictionary().unwrap();
        assert_eq!(
            dict["MediaSubdir"].as_string(),
            Some("PublicStaging/Example.ipa")
        );
        assert_eq!(
            dict["InstallTransferredDirectory"].as_signed_integer(),
            Some(1)
        );
        let opts = dict["InstallOptionsDictionary"].as_dictionary().unwrap();
        assert_eq!(opts["PackageType"].as_string(), Some("Customer"));
        assert_eq!(
            opts["InstallDeltaTypeKey"].as_string(),
            Some("InstallDeltaTypeSparseIPAFiles")
        );
    }

    #[test]
    fn build_zip_metadata_has_correct_structure() {
        let meta = build_zip_metadata(42, 1_000_000);
        let dict = meta.as_dictionary().unwrap();
        assert_eq!(dict["RecordCount"].as_signed_integer(), Some(42));
        assert_eq!(dict["Version"].as_signed_integer(), Some(2));
        assert_eq!(
            dict["TotalUncompressedBytes"].as_signed_integer(),
            Some(1_000_000)
        );
        assert_eq!(
            dict["StandardDirectoryPerms"].as_signed_integer(),
            Some(16877)
        );
    }

    #[test]
    fn local_file_header_has_correct_signature() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut buf = Vec::new();
            write_local_file_header(&mut buf, "test.txt", 0x12345678, 100, 100)
                .await
                .unwrap();
            // Check signature
            assert_eq!(&buf[0..4], &[0x50, 0x4b, 0x03, 0x04]);
            // Check version
            assert_eq!(u16::from_le_bytes([buf[4], buf[5]]), 20);
            // Check compression method = STORE
            assert_eq!(u16::from_le_bytes([buf[8], buf[9]]), 0);
            // Check CRC
            assert_eq!(
                u32::from_le_bytes([buf[14], buf[15], buf[16], buf[17]]),
                0x12345678
            );
        });
    }

    #[test]
    fn extra_field_starts_with_ut_signature() {
        // UT extended timestamp extra field ID = 0x5455
        assert_eq!(EXTRA_FIELD[0], 0x55); // 'U'
        assert_eq!(EXTRA_FIELD[1], 0x54); // 'T'
    }
}
