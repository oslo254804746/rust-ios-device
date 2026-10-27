//! Bounded ZIP32 preparation for the single-use install path.
//!
//! The archive and its entries are read from one open file. These are per-install
//! limits, not a process-wide memory or operating-system I/O bound. ZIP64 is
//! deliberately rejected before `zip` can allocate from an untrusted count.

use super::*;
use std::io::{self, Read, Seek, SeekFrom};

const CHUNK: usize = 64 * 1024;
const MAX_ARCHIVE: u64 = u32::MAX as u64;
const MAX_ENTRIES: usize = 16_384;
const MAX_DIRECTORY: u64 = 16 * 1024 * 1024;
const MAX_OPEN_READ: u64 = 32 * 1024 * 1024;
const MAX_ENTRY: u64 = 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;

fn invalid(message: &'static str) -> ZipConduitError {
    io::Error::new(io::ErrorKind::InvalidInput, message).into()
}

fn check_clock(deadline: tokio::time::Instant) -> io::Result<()> {
    if tokio::time::Instant::now() >= deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "ZIP install deadline elapsed",
        ))
    } else {
        Ok(())
    }
}

async fn cooperate(deadline: tokio::time::Instant) -> Result<(), ZipConduitError> {
    check_clock(deadline)?;
    tokio::task::yield_now().await;
    check_clock(deadline)?;
    Ok(())
}

struct GuardedFile {
    file: std::fs::File,
    deadline: tokio::time::Instant,
}

impl Read for GuardedFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        check_clock(self.deadline)?;
        let n = self.file.read(bytes)?;
        check_clock(self.deadline)?;
        Ok(n)
    }
}

impl Seek for GuardedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        check_clock(self.deadline)?;
        let result = self.file.seek(position)?;
        check_clock(self.deadline)?;
        Ok(result)
    }
}

fn bad_archive() -> ZipConduitError {
    error_mapping::zip_error(
        "open IPA ZIP archive",
        zip::result::ZipError::InvalidArchive("invalid ZIP32 directory"),
    )
}

fn le16(data: &[u8], pos: usize) -> u16 {
    u16::from_le_bytes([data[pos], data[pos + 1]])
}
fn le32(data: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap())
}

/// The ZIP library must not fall back to an unadmitted EOCD within a filename,
/// comment, or extra field. It may read those bytes normally, but cannot seek
/// to one as a new footer after selecting our synthetic outer footer.
struct SnapshotReader {
    data: std::io::Cursor<Vec<u8>>,
    footer: u64,
    selected: bool,
    left: u64,
    deadline: tokio::time::Instant,
}
impl Read for SnapshotReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        check_clock(self.deadline)?;
        if self.left == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ZIP directory read limit exceeded",
            ));
        }
        let limit = out.len().min(self.left as usize);
        let n = Read::read(&mut self.data, &mut out[..limit])?;
        self.left -= n as u64;
        check_clock(self.deadline)?;
        Ok(n)
    }
}
impl Seek for SnapshotReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        check_clock(self.deadline)?;
        if let SeekFrom::Start(target) = pos {
            let candidate = usize::try_from(target)
                .ok()
                .and_then(|start| self.data.get_ref().get(start..start.saturating_add(4)))
                == Some(&[0x50, 0x4b, 0x05, 0x06][..]);
            if self.selected && target != self.footer && candidate {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ZIP footer fallback rejected",
                ));
            }
            if target == self.footer {
                self.selected = true;
            }
        }
        self.data.seek(pos)
    }
}

/// Snapshot only an outer footer whose comment ends at EOF and its declared
/// ZIP32 central directory. Validate every record before allocating the ZIP
/// parser's metadata. Payload and local headers remain on the original file.
async fn snapshot_directory(
    file: &mut GuardedFile,
    length: u64,
) -> Result<(SnapshotReader, Vec<u64>, u64), ZipConduitError> {
    let tail_len = length.min(65_557) as usize;
    let mut tail = vec![0; tail_len];
    file.seek(SeekFrom::Start(length - tail_len as u64))?;
    file.read_exact(&mut tail)?;
    let mut limit_error = None;
    if tail_len >= 22 {
        for i in (0..=tail_len - 22).rev() {
            if tail[i..i + 4] != [0x50, 0x4b, 0x05, 0x06]
                || i + 22 + usize::from(le16(&tail, i + 20)) != tail_len
            {
                continue;
            }
            if le16(&tail, i + 4) != 0
                || le16(&tail, i + 6) != 0
                || le16(&tail, i + 8) != le16(&tail, i + 10)
            {
                continue;
            }
            let count = usize::from(le16(&tail, i + 10));
            let size = u64::from(le32(&tail, i + 12));
            let declared = u64::from(le32(&tail, i + 16));
            if count == u16::MAX as usize || size == u32::MAX as u64 || declared == u32::MAX as u64
            {
                limit_error = Some(invalid("ZIP64 is not supported by the bounded installer"));
                continue;
            }
            if count > MAX_ENTRIES || size > MAX_DIRECTORY {
                limit_error = Some(invalid("ZIP directory exceeds install limits"));
                continue;
            }
            let footer = length - tail_len as u64 + i as u64;
            let Some(start) = footer.checked_sub(size) else {
                continue;
            };
            let Some(prefix) = start.checked_sub(declared) else {
                continue;
            };
            let mut directory = vec![0; size as usize];
            file.seek(SeekFrom::Start(start))?;
            for chunk in directory.chunks_mut(CHUNK) {
                cooperate(file.deadline).await?;
                file.read_exact(chunk)?;
            }
            let mut cursor = 0usize;
            let mut originals = Vec::with_capacity(count);
            let mut records = Vec::with_capacity(count);
            let mut valid = true;
            for _ in 0..count {
                if cursor
                    .checked_add(46)
                    .filter(|end| *end <= directory.len())
                    .is_none()
                    || directory[cursor..cursor + 4] != [0x50, 0x4b, 0x01, 0x02]
                {
                    valid = false;
                    break;
                }
                let name = usize::from(le16(&directory, cursor + 28));
                let extra = usize::from(le16(&directory, cursor + 30));
                let comment = usize::from(le16(&directory, cursor + 32));
                let end = cursor + 46 + name + extra + comment;
                if end > directory.len() || le16(&directory, cursor + 34) != 0 {
                    valid = false;
                    break;
                }
                let local = le32(&directory, cursor + 42);
                if local == u32::MAX
                    || le32(&directory, cursor + 20) == u32::MAX
                    || le32(&directory, cursor + 24) == u32::MAX
                {
                    limit_error = Some(invalid("ZIP64 is not supported by the bounded installer"));
                    valid = false;
                    break;
                }
                originals.push(prefix + u64::from(local));
                records.push(cursor);
                cursor = end;
            }
            if !valid || cursor != directory.len() {
                continue;
            }
            let local_bytes = count * 30;
            let mut synthetic = vec![0u8; local_bytes];
            for n in 0..count {
                synthetic[n * 30..n * 30 + 4].copy_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
                directory[records[n] + 42..records[n] + 46]
                    .copy_from_slice(&((n * 30) as u32).to_le_bytes());
            }
            synthetic.extend_from_slice(&directory);
            let footer_offset = synthetic.len() as u64;
            let mut eocd = [0u8; 22];
            eocd[..4].copy_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
            eocd[8..10].copy_from_slice(&(count as u16).to_le_bytes());
            eocd[10..12].copy_from_slice(&(count as u16).to_le_bytes());
            eocd[12..16].copy_from_slice(&(size as u32).to_le_bytes());
            eocd[16..20].copy_from_slice(&(local_bytes as u32).to_le_bytes());
            synthetic.extend_from_slice(&eocd);
            return Ok((
                SnapshotReader {
                    data: std::io::Cursor::new(synthetic),
                    footer: footer_offset,
                    selected: false,
                    left: MAX_OPEN_READ,
                    deadline: file.deadline,
                },
                originals,
                start,
            ));
        }
    }
    Err(limit_error.unwrap_or_else(bad_archive))
}

pub(super) struct PreparedArchive {
    file: GuardedFile,
    entries: Vec<Entry>,
    total: u64,
    deadline: tokio::time::Instant,
}

struct Entry {
    name: String,
    is_dir: bool,
    method: zip::CompressionMethod,
    offset: u64,
    compressed: u64,
    size: u64,
    crc: u32,
}

impl PreparedArchive {
    pub(super) async fn open(
        path: &Path,
        deadline: tokio::time::Instant,
    ) -> Result<Self, ZipConduitError> {
        check_clock(deadline)?;
        let file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        if length > MAX_ARCHIVE {
            return Err(invalid("IPA exceeds install archive limit"));
        }
        let mut file = GuardedFile { file, deadline };
        let (snapshot, originals, directory_start) = snapshot_directory(&mut file, length).await?;
        let mut archive = zip::ZipArchive::new(snapshot)
            .map_err(|e| error_mapping::zip_error("open IPA ZIP archive", e))?;
        if archive.len() > MAX_ENTRIES {
            return Err(invalid("ZIP entry count exceeds install limit"));
        }
        let mut total = 0u64;
        let mut entries = Vec::with_capacity(archive.len());
        for i in 0..archive.len() {
            check_clock(deadline)?;
            {
                let entry = archive
                    .by_index(i)
                    .map_err(|e| error_mapping::zip_error("open IPA ZIP entry", e))?;
                checked_zip_u16_len("filename", entry.name().len())?;
                let is_dir = entry.is_dir();
                let size = if is_dir { 0 } else { entry.size() };
                let method = entry.compression();
                if !is_dir
                    && !matches!(
                        method,
                        zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
                    )
                {
                    return Err(invalid("unsupported ZIP compression method"));
                }
                if size > MAX_ENTRY {
                    return Err(invalid("ZIP entry exceeds install size limit"));
                }
                total = total
                    .checked_add(size)
                    .ok_or_else(|| invalid("ZIP total size overflow"))?;
                if total > MAX_TOTAL {
                    return Err(invalid("ZIP expanded total exceeds install limit"));
                }
                let slot = usize::try_from(entry.header_start() / 30)
                    .map_err(|e| error_mapping::input_error("ZIP local header index", e))?;
                let local = *originals.get(slot).ok_or_else(bad_archive)?;
                let mut header = [0u8; 30];
                file.seek(SeekFrom::Start(local))?;
                file.read_exact(&mut header).map_err(|e| {
                    error_mapping::zip_error("open IPA ZIP archive", zip::result::ZipError::Io(e))
                })?;
                if header[..4] != [0x50, 0x4b, 0x03, 0x04] {
                    return Err(bad_archive());
                }
                let offset = local
                    .checked_add(30 + u64::from(le16(&header, 26)) + u64::from(le16(&header, 28)))
                    .ok_or_else(bad_archive)?;
                let compressed = entry.compressed_size();
                if offset
                    .checked_add(compressed)
                    .filter(|end| *end <= directory_start)
                    .is_none()
                {
                    return Err(error_mapping::entry_read_error(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "truncated ZIP entry",
                    )));
                }
                entries.push(Entry {
                    name: entry.name().to_string(),
                    is_dir,
                    method,
                    offset,
                    compressed,
                    size,
                    crc: entry.crc32(),
                });
            }
            cooperate(deadline).await?;
        }
        drop(archive);
        let mut prepared = Self {
            file,
            entries,
            total,
            deadline,
        };
        for index in 0..prepared.entries.len() {
            if !prepared.entries[index].is_dir {
                prepared.process::<tokio::io::Sink>(index, None).await?;
            }
        }
        Ok(prepared)
    }

    pub(super) fn record_count(&self) -> u64 {
        self.entries.len() as u64 + 2
    }
    pub(super) fn total_size(&self) -> u64 {
        self.total
    }

    pub(super) async fn send<S: AsyncWrite + Unpin>(
        &mut self,
        stream: &mut S,
    ) -> Result<(), ZipConduitError> {
        for i in 0..self.entries.len() {
            cooperate(self.deadline).await?;
            let entry = &self.entries[i];
            if entry.is_dir {
                write_zip_dir_entry(stream, &entry.name).await?;
                continue;
            }
            let size = u32::try_from(entry.size)
                .map_err(|e| error_mapping::input_error("file data", e))?;
            write_local_file_header(stream, &entry.name, entry.crc, size, size).await?;
            self.process(i, Some(&mut *stream)).await?;
        }
        Ok(())
    }

    async fn process<S: AsyncWrite + Unpin>(
        &mut self,
        index: usize,
        mut output: Option<&mut S>,
    ) -> Result<(), ZipConduitError> {
        let entry = &self.entries[index];
        self.file.seek(SeekFrom::Start(entry.offset))?;
        let mut remaining = entry.compressed;
        let mut input = vec![0u8; CHUNK];
        let mut decoded = vec![0u8; CHUNK];
        let mut available = 0usize;
        let mut consumed = 0usize;
        let mut total = 0u64;
        let mut crc = crc32fast::Hasher::new();
        let mut inflater = (entry.method == zip::CompressionMethod::Deflated)
            .then(|| flate2::Decompress::new(false));
        loop {
            cooperate(self.deadline).await?;
            if consumed == available && remaining > 0 {
                let limit = remaining.min(CHUNK as u64) as usize;
                available = self
                    .file
                    .read(&mut input[..limit])
                    .map_err(error_mapping::entry_read_error)?;
                if available == 0 {
                    return Err(error_mapping::entry_read_error(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "truncated ZIP entry",
                    )));
                }
                remaining -= available as u64;
                consumed = 0;
            }
            let (produced, finished) = if let Some(inflate) = &mut inflater {
                let before_in = inflate.total_in();
                let before_out = inflate.total_out();
                let status = inflate
                    .decompress(
                        &input[consumed..available],
                        &mut decoded,
                        flate2::FlushDecompress::None,
                    )
                    .map_err(|e| {
                        error_mapping::entry_read_error(io::Error::new(
                            io::ErrorKind::InvalidData,
                            e,
                        ))
                    })?;
                let used = (inflate.total_in() - before_in) as usize;
                let produced = (inflate.total_out() - before_out) as usize;
                consumed += used;
                if used == 0 && produced == 0 && status != flate2::Status::StreamEnd {
                    if remaining == 0 || available - consumed == CHUNK {
                        return Err(error_mapping::entry_read_error(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated ZIP deflate stream",
                        )));
                    }
                    input.copy_within(consumed..available, 0);
                    available -= consumed;
                    consumed = 0;
                    let limit = remaining.min((CHUNK - available) as u64) as usize;
                    let n = self
                        .file
                        .read(&mut input[available..available + limit])
                        .map_err(error_mapping::entry_read_error)?;
                    if n == 0 {
                        return Err(error_mapping::entry_read_error(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated ZIP entry",
                        )));
                    }
                    available += n;
                    remaining -= n as u64;
                    continue;
                }
                (produced, status == flate2::Status::StreamEnd)
            } else {
                let n = available - consumed;
                decoded[..n].copy_from_slice(&input[consumed..available]);
                consumed = available;
                (n, remaining == 0)
            };
            total = total
                .checked_add(produced as u64)
                .ok_or_else(|| invalid("ZIP entry size overflow"))?;
            if total > entry.size {
                return Err(invalid("ZIP entry expanded beyond its declared size"));
            }
            crc.update(&decoded[..produced]);
            if let Some(stream) = &mut output {
                stream.write_all(&decoded[..produced]).await?;
            }
            if finished {
                if remaining != 0 || consumed != available || total != entry.size {
                    return Err(invalid("ZIP entry size differs from its declaration"));
                }
                if crc.finalize() != entry.crc {
                    return Err(error_mapping::entry_read_error(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid checksum",
                    )));
                }
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn directory_snapshot_does_not_parse_later_mutated_archive_metadata() {
        let bytes = super::super::extraction_contract_tests::stored_zip();
        let path = std::env::temp_dir().join(format!("zip-snapshot-{}.ipa", uuid::Uuid::new_v4()));
        std::fs::write(&path, &bytes).unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut file = GuardedFile {
            file: std::fs::File::open(&path).unwrap(),
            deadline,
        };
        let (snapshot, originals, _) = snapshot_directory(&mut file, bytes.len() as u64)
            .await
            .unwrap();
        let footer = bytes
            .windows(4)
            .rposition(|window| window == [0x50, 0x4b, 0x05, 0x06])
            .unwrap();
        let mut changed = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        changed.seek(SeekFrom::Start((footer + 8) as u64)).unwrap();
        changed.write_all(&[0xff; 12]).unwrap();
        changed.flush().unwrap();
        drop(changed);
        let mut archive = zip::ZipArchive::new(snapshot).unwrap();
        assert_eq!(archive.len(), 2);
        assert_eq!(originals.len(), 2);
        assert_eq!(
            archive.by_index(1).unwrap().name(),
            "Payload/owned-private.app/file.txt"
        );
        drop(archive);
        drop(file);
        std::fs::remove_file(path).unwrap();
    }
}
