use zerocopy::byteorder::{BigEndian, LittleEndian, U16, U32};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// DTX message magic – "0x795B3D1F" (big-endian on wire)
/// Source: go-ios/ios/dtx_codec/dtxmessage.go
pub const DTX_MAGIC: u32 = 0x795B3D1F;

/// DTX message header (32 bytes, mostly little-endian).
///
/// Wire layout (from go-ios decoder.go / dtxmessage.go):
///   magic            [4 BE]  – 0x795B3D1F
///   header_length    [4 LE]  – always 32
///   fragment_index   [2 LE]
///   fragment_count   [2 LE]
///   message_length   [4 LE]  – payload section size (excluding this header)
///   identifier       [4 LE]
///   conversation_idx [4 LE]
///   channel_code     [4 LE]
///   expects_reply    [4 LE]  – 1 = yes, 0 = no
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct DtxHeader {
    pub magic: U32<BigEndian>,
    pub header_length: U32<LittleEndian>,
    pub fragment_index: U16<LittleEndian>,
    pub fragment_count: U16<LittleEndian>,
    pub message_length: U32<LittleEndian>,
    pub identifier: U32<LittleEndian>,
    pub conversation_idx: U32<LittleEndian>,
    pub channel_code: U32<LittleEndian>,
    pub expects_reply: U32<LittleEndian>,
}

impl DtxHeader {
    pub const SIZE: usize = 32;
    pub const HEADER_LENGTH: u32 = 32;

    /// True when this message is part of a multi-fragment sequence.
    pub fn is_fragment(&self) -> bool {
        self.fragment_count.get() > 1
    }

    /// True when this is the first fragment of a sequence.
    pub fn is_first_fragment(&self) -> bool {
        self.fragment_index.get() == 0
    }
}

/// DTX payload header (16 bytes, little-endian), follows DtxHeader.
///
///   message_type       [4 LE]  – see MessageType enum
///   auxiliary_length   [4 LE]  – bytes of auxiliary data
///   total_payload_length [4 LE] – auxiliary + payload bytes
///   flags              [4 LE]
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct DtxPayloadHeader {
    pub message_type: U32<LittleEndian>,
    pub auxiliary_length: U32<LittleEndian>,
    pub total_payload_length: U32<LittleEndian>,
    pub flags: U32<LittleEndian>,
}

impl DtxPayloadHeader {
    pub const SIZE: usize = 16;

    /// True when the auxiliary section is present.
    pub fn has_auxiliary(&self) -> bool {
        self.auxiliary_length.get() > 0
    }

    /// Payload-only length (excluding auxiliary).
    pub fn payload_length(&self) -> u32 {
        self.total_payload_length
            .get()
            .saturating_sub(self.auxiliary_length.get())
    }
}

/// DTX auxiliary header (16 bytes, little-endian), precedes auxiliary data.
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct DtxAuxiliaryHeader {
    pub buffer_size: U32<LittleEndian>,
    pub unknown: U32<LittleEndian>,
    pub auxiliary_size: U32<LittleEndian>,
    pub unknown2: U32<LittleEndian>,
}

impl DtxAuxiliaryHeader {
    pub const SIZE: usize = 16;
}

/// DTX message type codes (go-ios dtxmessage.go MessageType enum)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum DtxMessageType {
    MethodInvocation = 2,
    Response = 3,
    Notification = 4,
}
