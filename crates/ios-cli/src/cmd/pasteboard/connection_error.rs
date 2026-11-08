//! Keep snapshot startup's original CoreError through the existing XPC variant.

#[derive(Debug)]
struct ConnectionError(ios_core::CoreError);

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "connect pasteboard XPC: {}", self.0)
    }
}

impl std::error::Error for ConnectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

pub(super) fn retain(source: ios_core::CoreError) -> ios_core::pasteboard::PasteboardError {
    let kind = match &source {
        ios_core::CoreError::Io(error) => error.kind(),
        _ => std::io::ErrorKind::Other,
    };
    ios_core::pasteboard::PasteboardError::Xpc(ios_core::XpcError::Io(std::io::Error::new(
        kind,
        ConnectionError(source),
    )))
}
