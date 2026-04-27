use zerocopy::byteorder::{LittleEndian, U32};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// usbmuxd protocol header (16 bytes, little-endian)
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct UsbmuxHeader {
    pub length: U32<LittleEndian>,
    pub version: U32<LittleEndian>,
    pub msg_type: U32<LittleEndian>,
    pub tag: U32<LittleEndian>,
}

impl UsbmuxHeader {
    pub const SIZE: usize = 16;
    pub const VERSION_PLIST: u32 = 1;
    pub const MSG_TYPE_PLIST: u32 = 8;
    pub const MSG_TYPE_RESULT: u32 = 1;
    pub const MSG_TYPE_CONNECT: u32 = 3;
    pub const MSG_TYPE_LISTEN: u32 = 4;
    pub const MSG_TYPE_DEVICE_ADD: u32 = 10;
    pub const MSG_TYPE_DEVICE_REMOVE: u32 = 11;

    pub fn new_plist(payload_len: u32, tag: u32) -> Self {
        Self {
            length: U32::new(Self::SIZE as u32 + payload_len),
            version: U32::new(Self::VERSION_PLIST),
            msg_type: U32::new(Self::MSG_TYPE_PLIST),
            tag: U32::new(tag),
        }
    }
}
