use zerocopy::byteorder::{LittleEndian, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// AFC packet magic bytes: "CFA6LPAA" stored as LE u64
/// Bytes: 43 46 41 36 4C 50 41 41 → 0x4141504C36414643 in LE
pub const AFC_MAGIC: u64 = 0x4141504C36414643;

/// AFC protocol header (40 bytes, all little-endian).
///
/// Wire format (matches go-ios ios/afc/afc.go `header` struct):
///   Magic     [8] – 0x4141504C36414643
///   EntireLen [8] – total packet bytes (header + header_payload + payload)
///   ThisLen   [8] – header + header_payload bytes only
///   PacketNum [8] – incrementing per-request counter
///   Operation [8] – AfcOpcode
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct AfcHeader {
    pub magic: U64<LittleEndian>,
    pub entire_len: U64<LittleEndian>, // header + header_payload + payload
    pub this_len: U64<LittleEndian>,   // header + header_payload
    pub packet_num: U64<LittleEndian>,
    pub operation: U64<LittleEndian>,
}

impl AfcHeader {
    pub const SIZE: usize = 40;

    /// Build a request header.
    pub fn new(
        packet_num: u64,
        opcode: AfcOpcode,
        header_payload_len: usize,
        payload_len: usize,
    ) -> Self {
        let this_len = Self::SIZE as u64 + header_payload_len as u64;
        let entire_len = this_len + payload_len as u64;
        Self {
            magic: U64::new(AFC_MAGIC),
            entire_len: U64::new(entire_len),
            this_len: U64::new(this_len),
            packet_num: U64::new(packet_num),
            operation: U64::new(opcode as u64),
        }
    }
}

/// AFC operation codes (matches go-ios afc.go constants)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum AfcOpcode {
    Status = 0x00000001,
    Data = 0x00000002,
    ReadDir = 0x00000003,
    WritePart = 0x00000006,
    TruncateFile = 0x00000007,
    RemovePath = 0x00000008,
    MakePath = 0x00000009,
    GetFileInfo = 0x0000000A,
    GetDeviceInfo = 0x0000000B,
    WriteFile = 0x0000000C, // not in go-ios but common
    FileRefOpen = 0x0000000D,
    FileRefOpenResult = 0x0000000E,
    FileRefRead = 0x0000000F,
    FileRefWrite = 0x00000010,
    FileRefSeek = 0x00000011,
    FileRefTell = 0x00000012,
    FileRefTellResult = 0x00000013,
    FileRefClose = 0x00000014,
    FileRefSetFileSize = 0x00000015,
    GetConnectionInfo = 0x00000016,
    SetConnectionOptions = 0x00000017,
    RenamePath = 0x00000018,
    SetFSBlockSize = 0x00000019,
    SetSocketBlockSize = 0x0000001A,
    FileRefLock = 0x0000001B,
    MakeLink = 0x0000001C,
    GetFileHash = 0x0000001D,
    SetModTime = 0x0000001E,
    GetFileHashRange = 0x0000001F,
    FileRefFlush = 0x00000020,
    SetFileModTime = 0x00000021,
    RemovePathAndContents = 0x00000022,
}
