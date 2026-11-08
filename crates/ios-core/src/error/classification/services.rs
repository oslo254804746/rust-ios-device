//! Typed service adapters. Wire/status text is deliberately not inspected.

use super::{typed_class, ErrorClass};
use std::error::Error;

macro_rules! protocol_class {
    ($error:expr, $type:path) => { protocol_class!($error, $type, {}); };
    ($error:expr, $type:path, {$($pattern:pat => $class:expr),* $(,)?}) => {{
        use $type as Kind;
        typed_class!($error, Kind, {
            Kind::Protocol(_) => ErrorClass::Protocol,
            $($pattern => $class,)*
        });
    }};
}

pub(super) fn class(error: &(dyn Error + 'static)) -> Option<ErrorClass> {
    if let Some(error) = error.downcast_ref::<crate::session::SessionFailure>() {
        return Some(error.class());
    }
    if let Some(error) = error.downcast_ref::<crate::session::MuxResponseError>() {
        return Some(error.class());
    }
    #[cfg(feature = "tunnel")]
    if let Some(error) = error.downcast_ref::<crate::ExistingRsdProbeError>() {
        return Some(error.class());
    }
    #[cfg(feature = "backup2-manifest")]
    if let Some(rusqlite::Error::SqliteFailure(native, _)) = error.downcast_ref::<rusqlite::Error>()
    {
        use rusqlite::ErrorCode as Code;
        return match native.code {
            Code::PermissionDenied | Code::AuthorizationForStatementDenied => {
                Some(ErrorClass::PermissionDenied)
            }
            Code::DatabaseBusy | Code::DatabaseLocked | Code::ConstraintViolation => {
                Some(ErrorClass::Conflict)
            }
            Code::OutOfMemory | Code::DiskFull | Code::TooBig => Some(ErrorClass::ResourceLimit),
            Code::ReadOnly => Some(ErrorClass::Precondition),
            Code::DatabaseCorrupt | Code::NotADatabase => Some(ErrorClass::Protocol),
            Code::SystemIoFailure => Some(ErrorClass::Io),
            // CannotOpen does not establish NotFound/permission; interruption
            // does not acknowledge caller cancellation. Preserve unknown codes.
            _ => None,
        };
    }
    protocol_class!(error, crate::device_link::DeviceLinkError);
    protocol_class!(error, crate::backup2::Mobilebackup2Error);
    {
        use crate::simlocation::SimLocationError as Kind;
        typed_class!(error, Kind, {Kind::GpxParse(_) => ErrorClass::InvalidInput});
    }
    #[cfg(feature = "afc")]
    {
        if error.is::<crate::afc::AfcReadLimit>() {
            return Some(ErrorClass::ResourceLimit);
        }
        use crate::afc::{AfcError as Kind, AfcStatusCode as Status};
        typed_class!(error, Kind, {
            Kind::Protocol(_) => ErrorClass::Protocol,
            Kind::Status(Status::PermDenied) => ErrorClass::PermissionDenied,
            Kind::Status(Status::Timeout) => ErrorClass::Timeout,
            Kind::Status(Status::ServiceNotConnected) => ErrorClass::Disconnected,
            Kind::Status(Status::ObjectNotFound) => ErrorClass::NotFound,
            Kind::Status(Status::OpNotSupported) => ErrorClass::Unsupported,
            Kind::Status(Status::InvalidArgument) => ErrorClass::InvalidInput,
            Kind::Status(Status::ObjectExists | Status::ObjectBusy) => ErrorClass::Conflict,
            Kind::Status(Status::NoResources | Status::NoMem | Status::TooMuchData | Status::NoSpaceLeft) => ErrorClass::ResourceLimit,
            // Unknown device codes, including an interruption, do not acquire
            // guessed permission/cancellation semantics.
            Kind::Status(_) => ErrorClass::ServiceFailure
        });
    }
    #[cfg(feature = "accessibility_audit")]
    protocol_class!(error, crate::accessibility_audit::AccessibilityAuditError);
    #[cfg(feature = "amfi")]
    {
        use crate::amfi::AmfiError as Kind;
        typed_class!(error, Kind, {Kind::Device(_) => ErrorClass::ServiceFailure});
    }
    #[cfg(feature = "apps")]
    {
        if error.is::<crate::apps::installation::BrowseLimit>() {
            return Some(ErrorClass::ResourceLimit);
        }
        if error.is::<crate::apps::installation::BrowseResponseError>() {
            return Some(ErrorClass::ServiceFailure);
        }
        protocol_class!(error, crate::apps::installation::IpError, {
            Kind::Install(_) => ErrorClass::ServiceFailure
        });
        protocol_class!(error, crate::apps::zipconduit::ZipConduitError, {
            Kind::Zip(_) => ErrorClass::InvalidInput,
            Kind::Install(_) => ErrorClass::ServiceFailure
        });
        #[cfg(feature = "tunnel")]
        protocol_class!(error, crate::apps::appservice::AppServiceError);
    }
    #[cfg(feature = "arbitration")]
    protocol_class!(error, crate::arbitration::ArbitrationError);
    #[cfg(feature = "btlogger")]
    protocol_class!(error, crate::bt_packet_logger::BtPacketLoggerError);
    #[cfg(feature = "companion")]
    protocol_class!(error, crate::companion::CompanionError, {
        Kind::Timeout(_) => ErrorClass::Timeout
    });
    #[cfg(feature = "configuration")]
    protocol_class!(error, crate::configuration::ConfigurationError, {
        Kind::LegacyUnsupported => ErrorClass::Unsupported
    });
    #[cfg(feature = "crashreport")]
    protocol_class!(error, crate::crashreport::CrashReportError, {
        Kind::Timeout => ErrorClass::Timeout
    });
    #[cfg(feature = "cryptex")]
    protocol_class!(error, crate::cryptex::CryptexError, {
        Kind::Timeout(_) => ErrorClass::Timeout,
        Kind::Unsupported(_) => ErrorClass::Unsupported,
        Kind::Conflict(_) => ErrorClass::Conflict,
        Kind::Device(_) => ErrorClass::ServiceFailure
    });
    #[cfg(feature = "debugserver")]
    {
        use crate::debugserver::DebugserverError as Kind;
        typed_class!(error, Kind, {
            Kind::InvalidPayload | Kind::Checksum { .. } | Kind::Utf8(_) => ErrorClass::Protocol
        });
    }
    #[cfg(feature = "deviceinfo")]
    protocol_class!(error, crate::deviceinfo::DeviceInfoError);
    #[cfg(feature = "diagnostics")]
    protocol_class!(error, crate::diagnostics::DiagnosticsError, {
        Kind::Deprecated(_) => ErrorClass::Unsupported
    });
    #[cfg(feature = "diagnosticsservice")]
    protocol_class!(error, crate::diagnosticsservice::DiagnosticsServiceError);
    #[cfg(feature = "display")]
    protocol_class!(error, crate::display::DisplayError, {
        Kind::Timeout => ErrorClass::Timeout,
        Kind::LegacyUnsupported | Kind::FeatureMissing(_) => ErrorClass::Unsupported,
        Kind::MediaInUse => ErrorClass::Conflict
    });
    #[cfg(feature = "dproxy")]
    protocol_class!(error, crate::dproxy::DproxyError);
    #[cfg(feature = "dtx")]
    protocol_class!(error, crate::dtx::DtxError, {
        Kind::BadMagic(_) => ErrorClass::Protocol
    });
    #[cfg(feature = "fetchsymbols")]
    protocol_class!(error, crate::fetchsymbols::FetchSymbolsError);
    #[cfg(feature = "file_relay")]
    protocol_class!(error, crate::file_relay::FileRelayError);
    #[cfg(feature = "fileservice")]
    protocol_class!(error, crate::fileservice::FileServiceError);
    #[cfg(feature = "heartbeat")]
    protocol_class!(error, crate::heartbeat::HeartbeatError);
    #[cfg(feature = "hid")]
    protocol_class!(error, crate::hid::HidError, {
        Kind::LegacyUnsupported | Kind::ShimUnsupported(_) | Kind::FeatureMissing(_) => ErrorClass::Unsupported
    });
    #[cfg(feature = "house_arrest")]
    protocol_class!(error, crate::services::house_arrest::HouseArrestError, {
        Kind::Service(_) => ErrorClass::ServiceFailure
    });
    #[cfg(feature = "iconservice")]
    protocol_class!(error, crate::iconservice::IconServiceError, {
        Kind::LegacyUnsupported => ErrorClass::Unsupported
    });
    #[cfg(feature = "idam")]
    protocol_class!(error, crate::idam::IdamError);
    #[cfg(feature = "imagemounter")]
    protocol_class!(error, crate::imagemounter::protocol::ImageMounterError, {
        Kind::DeviceError(_) | Kind::Tss(_) => ErrorClass::ServiceFailure
    });
    #[cfg(feature = "installcoordination")]
    protocol_class!(error, crate::installcoordination::InstallCoordinationError, {
        Kind::Timeout(_) => ErrorClass::Timeout
    });
    #[cfg(feature = "mcinstall")]
    protocol_class!(error, crate::mcinstall::McInstallError, {
        Kind::PasscodeSet => ErrorClass::Precondition
    });
    #[cfg(feature = "misagent")]
    protocol_class!(error, crate::misagent::MisagentError, {
        Kind::Status(_) => ErrorClass::ServiceFailure
    });
    #[cfg(feature = "mobileactivation")]
    protocol_class!(error, crate::mobileactivation::MobileActivationError, {
        Kind::Timeout(_) => ErrorClass::Timeout,
        Kind::Http(_) => ErrorClass::ServiceFailure
    });
    #[cfg(feature = "notificationproxy")]
    protocol_class!(error, crate::notificationproxy::NotificationProxyError, {
        Kind::Timeout => ErrorClass::Timeout,
        Kind::ProxyDeath => ErrorClass::Disconnected
    });
    #[cfg(feature = "orientation")]
    protocol_class!(error, crate::orientation::OrientationError, {
        Kind::LegacyUnsupported => ErrorClass::Unsupported
    });
    #[cfg(feature = "ostrace")]
    protocol_class!(error, crate::ostrace::OsTraceError, {
        Kind::Timeout => ErrorClass::Timeout,
        Kind::ArchiveLimit(_) => ErrorClass::ResourceLimit
    });
    #[cfg(feature = "pasteboard")]
    protocol_class!(error, crate::pasteboard::PasteboardError, {
        Kind::Timeout { .. } => ErrorClass::Timeout,
        Kind::Closed => ErrorClass::Disconnected,
        Kind::Limit(_) => ErrorClass::ResourceLimit,
        Kind::Service { .. } => ErrorClass::ServiceFailure
    });
    #[cfg(feature = "pcap")]
    protocol_class!(error, crate::pcap::PcapError);
    #[cfg(feature = "power_assertion")]
    protocol_class!(error, crate::power_assertion::PowerAssertionError);
    #[cfg(feature = "preboard")]
    protocol_class!(error, crate::preboard::PreboardError);
    #[cfg(feature = "restore")]
    protocol_class!(error, crate::restore::RestoreError);
    #[cfg(feature = "screencapture")]
    protocol_class!(error, crate::screencapture::ScreenCaptureError, {
        Kind::LegacyUnsupported => ErrorClass::Unsupported
    });
    #[cfg(feature = "screenshot")]
    protocol_class!(error, crate::screenshot::ScreenshotError);
    #[cfg(feature = "springboard")]
    protocol_class!(error, crate::springboard::SpringboardError, {
        Kind::Service(_) => ErrorClass::ServiceFailure
    });
    #[cfg(feature = "testmanager")]
    {
        if error.is::<crate::RunXcTestPlanError>() {
            return Some(ErrorClass::InvalidInput);
        }
        use crate::testmanager::xctestrun::XctestRunError as Kind;
        typed_class!(error, Kind, {
            Kind::UnsupportedFormatVersion(_) => ErrorClass::Unsupported,
            Kind::MissingFormatVersion | Kind::EmptyConfigurations => ErrorClass::InvalidInput
        });
    }
    #[cfg(feature = "webinspector")]
    if error.is::<crate::webinspector::WebInspectorLimit>() {
        return Some(ErrorClass::ResourceLimit);
    }
    #[cfg(feature = "webinspector")]
    protocol_class!(error, crate::webinspector::WebInspectorError, {
        Kind::Timeout(_) => ErrorClass::Timeout,
        // Existing startup probes can flatten a timeout/reset into Disabled.
        // It cannot honestly be classified as a permission refusal.
        Kind::Disabled => ErrorClass::Precondition
    });
    None
}
