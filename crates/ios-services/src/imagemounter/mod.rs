//! Image mounter service – mount Developer Disk Images (DDI).
//!
//! Service: `com.apple.mobile.mobile_image_mounter`
//! Protocol: plist-framed (4-byte BE length prefix, same as lockdown)
//!
//! Reference: go-ios/ios/imagemounter/

pub mod download;
pub mod manifest;
pub mod protocol;
pub mod tss;

pub use download::DdiDownloader;
pub use manifest::BuildManifest;
pub use protocol::{ImageMounterClient, ImageMounterError};
