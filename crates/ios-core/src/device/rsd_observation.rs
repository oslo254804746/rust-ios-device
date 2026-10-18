//! Initial connection probe evidence, independent of later service retries.

use crate::{CoreError, ErrorClass};

/// Outcome of the optional RSD probe performed while establishing a device.
/// This is a historical observation, not a current reachability guarantee.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InitialRsdProbeState {
    NotAttempted,
    Available,
    Failed,
}

/// Why connection establishment did not attempt its optional RSD probe.
/// Neither reason is an explicit device-side unsupported response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InitialRsdProbeSkipReason {
    TunnelSkipped,
    DirectProbeFeatureUnavailable,
}

/// Read-only evidence owned by the established connection.
///
/// Failure retains the error reaching the probe owner, including its typed
/// source chain. Some older protocol adapters have already flattened sources;
/// this observation cannot recover those or certify remote cleanup/retry safety.
pub struct InitialRsdProbe {
    state: InitialRsdProbeState,
    skip_reason: Option<InitialRsdProbeSkipReason>,
    error: Option<CoreError>,
}

impl InitialRsdProbe {
    pub fn state(&self) -> InitialRsdProbeState {
        self.state
    }

    pub fn skip_reason(&self) -> Option<InitialRsdProbeSkipReason> {
        self.skip_reason
    }

    /// Original probe-owner error for explicit Rust callers. Its Display/source
    /// data may be sensitive; use the diagnostic policy when reporting it.
    pub fn error(&self) -> Option<&CoreError> {
        self.error.as_ref()
    }

    pub fn error_class(&self) -> Option<ErrorClass> {
        self.error.as_ref().map(CoreError::class)
    }

    pub(super) fn skipped(reason: InitialRsdProbeSkipReason) -> Self {
        Self {
            state: InitialRsdProbeState::NotAttempted,
            skip_reason: Some(reason),
            error: None,
        }
    }
}

impl std::fmt::Debug for InitialRsdProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitialRsdProbe")
            .field("state", &self.state)
            .field("skip_reason", &self.skip_reason)
            .field("error_class", &self.error_class())
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "tunnel")]
pub(super) enum RsdProbeOutcome<T = crate::RsdHandshake> {
    #[cfg(any(not(feature = "mdns"), test))]
    Skipped(InitialRsdProbeSkipReason),
    Completed(Result<T, CoreError>),
}

#[cfg(feature = "tunnel")]
impl<T> RsdProbeOutcome<T> {
    pub(super) fn into_parts(self) -> (Option<T>, InitialRsdProbe) {
        match self {
            #[cfg(any(not(feature = "mdns"), test))]
            Self::Skipped(reason) => (None, InitialRsdProbe::skipped(reason)),
            Self::Completed(Ok(directory)) => (
                Some(directory),
                InitialRsdProbe {
                    state: InitialRsdProbeState::Available,
                    skip_reason: None,
                    error: None,
                },
            ),
            Self::Completed(Err(error)) => (
                None,
                InitialRsdProbe {
                    state: InitialRsdProbeState::Failed,
                    skip_reason: None,
                    error: Some(error),
                },
            ),
        }
    }

    /// Later retries preserve their original failure without changing the
    /// connection's historical initial observation or cached directory.
    pub(super) fn into_result(self) -> Result<T, CoreError> {
        match self {
            Self::Completed(result) => result,
            #[cfg(any(not(feature = "mdns"), test))]
            Self::Skipped(reason) => Err(CoreError::Unsupported(
                match reason {
                    InitialRsdProbeSkipReason::TunnelSkipped => "RSD discovery requires a tunnel",
                    InitialRsdProbeSkipReason::DirectProbeFeatureUnavailable => {
                        "RSD direct probing requires ios-core feature 'mdns'"
                    }
                }
                .into(),
            )),
        }
    }
}

#[cfg(all(test, feature = "tunnel"))]
mod tests {
    use super::*;

    #[test]
    fn later_probe_skips_describe_local_preconditions_without_device_failure() {
        for reason in [
            InitialRsdProbeSkipReason::TunnelSkipped,
            InitialRsdProbeSkipReason::DirectProbeFeatureUnavailable,
        ] {
            let error = RsdProbeOutcome::<crate::RsdHandshake>::Skipped(reason)
                .into_result()
                .unwrap_err();
            assert_eq!(error.class(), ErrorClass::Unsupported);
            assert!(matches!(error, CoreError::Unsupported(_)));
        }
    }

    #[test]
    fn initial_probe_distinguishes_skips_failures_and_directory_evidence() {
        for (state, code) in [
            (InitialRsdProbeState::NotAttempted, "not_attempted"),
            (InitialRsdProbeState::Available, "available"),
            (InitialRsdProbeState::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), code);
        }
        for reason in [
            InitialRsdProbeSkipReason::TunnelSkipped,
            InitialRsdProbeSkipReason::DirectProbeFeatureUnavailable,
        ] {
            let (directory, observation) =
                RsdProbeOutcome::<crate::RsdHandshake>::Skipped(reason).into_parts();
            assert!(directory.is_none());
            assert_eq!(observation.state(), InitialRsdProbeState::NotAttempted);
            assert_eq!(observation.skip_reason(), Some(reason));
            assert!(observation.error().is_none());
            assert!(observation.error_class().is_none());
        }
        let (directory, observation) = RsdProbeOutcome::<crate::RsdHandshake>::Completed(Err(
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "synthetic-private-initial-probe-source",
            )),
        ))
        .into_parts();
        assert!(directory.is_none());
        assert_eq!(observation.state(), InitialRsdProbeState::Failed);
        assert!(observation.skip_reason().is_none());
        assert_eq!(
            observation.error_class(),
            Some(ErrorClass::PermissionDenied)
        );
        let CoreError::Io(source) = observation.error().unwrap() else {
            panic!("original typed IO source lost")
        };
        assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(source.to_string(), "synthetic-private-initial-probe-source");
        assert!(!format!("{observation:?} {observation:#?}").contains("synthetic-private"));

        let (directory, observation) =
            RsdProbeOutcome::<crate::RsdHandshake>::Completed(Ok(crate::RsdHandshake {
                udid: "synthetic-private-probe-device".into(),
                services: std::collections::HashMap::from([(
                    "owned.service".into(),
                    crate::ServiceDescriptor::new(12345),
                )]),
            }))
            .into_parts();
        assert_eq!(observation.state(), InitialRsdProbeState::Available);
        assert!(observation.error().is_none());
        assert!(observation.skip_reason().is_none());
        let directory = directory.unwrap();
        assert_eq!(
            directory.feature_evidence("missing.service", "feature"),
            crate::RsdFeatureEvidence::ServiceMissing
        );
        assert_eq!(
            directory.feature_evidence("owned.service", "feature"),
            crate::RsdFeatureEvidence::MetadataUnavailable
        );
        assert!(directory
            .supports_feature("owned.service", "feature")
            .unwrap());
        assert!(!format!("{observation:?}").contains("synthetic-private"));
    }
}
