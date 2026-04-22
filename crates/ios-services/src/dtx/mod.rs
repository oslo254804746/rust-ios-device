//! DTX protocol codec and connection manager.
//!
//! Reference: go-ios/ios/dtx_codec/

pub mod codec;
pub mod primitive;
pub mod primitive_enc;
pub mod types;

pub use codec::{
    decode_dtx_message_from_bytes, encode_ack, encode_dtx, read_dtx_frame, DtxConnection, DtxError,
};
pub use primitive_enc::{archived_object, encode_primitive_dict, PrimArg};
pub use types::{DtxMessage, DtxPayload, NSObject};
