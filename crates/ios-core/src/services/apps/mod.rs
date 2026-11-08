//! App management service – unified iOS <17 (InstallationProxy) and iOS 17+ (coredevice.appservice).

#[cfg(feature = "tunnel")]
pub mod appservice;
pub mod installation;
pub mod zipconduit;

#[cfg(feature = "tunnel")]
pub use appservice::{
    AppServiceClient, AppServiceError, CoreDeviceEnvelopeMode, LaunchApplicationOptions,
    RunningAppProcess,
};
pub use installation::{AppInfo, BrowseLimits, InstallationProxy};
pub use zipconduit::{
    install_ipa, install_ipa_owned_with_timeout, install_ipa_with_checked_progress,
    CheckedProgressCallback, ZipConduitError,
};

/// Service name for legacy (iOS <17) app listing.
pub const INSTALLATION_PROXY_SERVICE: &str = "com.apple.mobile.installation_proxy";
/// Service name for iOS 17+ CoreDevice app management.
pub const APPSERVICE_SERVICE: &str = "com.apple.coredevice.appservice";
