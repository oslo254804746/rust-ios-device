//! Keep native HTTP/URL causes and typed local endpoint rejections.

use super::MobileActivationError;

#[derive(thiserror::Error)]
#[error("{phase}: {source}")]
struct HttpOperationError {
    phase: &'static str,
    #[source]
    source: reqwest::Error,
}

impl std::fmt::Debug for HttpOperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivationHttpOperationError")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

pub(super) fn retain(phase: &'static str, source: reqwest::Error) -> MobileActivationError {
    // The retained reqwest flags/native chain supply precise categories.
    MobileActivationError::Io(std::io::Error::other(HttpOperationError { phase, source }))
}

#[derive(thiserror::Error)]
pub(super) enum EndpointInputError {
    #[error("invalid endpoint URL: {0}")]
    Url(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("activation endpoints must use HTTPS")]
    HttpsRequired,
    #[error("activation endpoint must not contain user credentials")]
    Credentials,
    #[error("non-official activation endpoint requires explicit unsafe opt-in")]
    CustomOptIn,
}

impl std::fmt::Debug for EndpointInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ActivationEndpointInputError { .. }")
    }
}

pub(super) fn input(source: EndpointInputError) -> MobileActivationError {
    MobileActivationError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        source,
    ))
}
