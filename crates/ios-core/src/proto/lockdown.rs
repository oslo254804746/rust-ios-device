/// Lockdown protocol frame: 4 bytes big-endian length prefix + plist payload.
pub struct LockdownFrame;

impl LockdownFrame {
    pub const HEADER_SIZE: usize = 4;

    /// Encode a plist payload with the 4-byte BE length prefix.
    pub fn encode(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::HEADER_SIZE + payload.len());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// Decode the length from the first 4 bytes (big-endian).
    pub fn decode_length(header: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*header)
    }
}
