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

fn native_error(
    kind: std::io::ErrorKind,
    phase: &'static str,
    source: impl std::error::Error + Send + Sync + 'static,
) -> ImageMounterError {
    ImageMounterError::Io(crate::error::context::retained_io(kind, phase, source))
}

fn native_io(phase: &'static str, source: std::io::Error) -> ImageMounterError {
    native_error(source.kind(), phase, source)
}

fn native_http(phase: &'static str, source: reqwest::Error) -> ImageMounterError {
    let kind = if source.is_timeout() {
        std::io::ErrorKind::TimedOut
    } else if source.is_builder() {
        std::io::ErrorKind::InvalidInput
    } else {
        std::io::ErrorKind::Other
    };
    native_error(kind, phase, source)
}

fn native_zip(phase: &'static str, source: zip::result::ZipError) -> ImageMounterError {
    let kind = match &source {
        zip::result::ZipError::Io(error) => error.kind(),
        _ => std::io::ErrorKind::InvalidData,
    };
    native_error(kind, phase, source)
}

#[cfg(test)]
mod test_assets;
