use std::{error::Error, fmt, io};

use super::LockdownError;
use crate::ErrorClass;

/// A device-issued Error field retained by the preserving Lockdown APIs.
///
/// This is an explicit response outcome, not an OS or parser failure. Code
/// access can reveal private device data; automatic Debug/Display omit it.
pub struct LockdownResponseError {
    operation: &'static str,
    code: String,
}

impl LockdownResponseError {
    /// The fixed request name, currently GetValue or StartService.
    pub fn operation(&self) -> &'static str {
        self.operation
    }

    /// The exact case-sensitive Error string received from the device.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Classify only known codes in their owning request, never diagnostic text.
    pub fn class(&self) -> ErrorClass {
        match (self.operation, self.code.as_str()) {
            ("GetValue", "MissingValue") => ErrorClass::NotFound,
            ("StartService", "InvalidService") => ErrorClass::ServiceMissing,
            _ => ErrorClass::ServiceFailure,
        }
    }

    /// Find an observation within 32 typed sources, including IO payloads.
    /// None does not prove that a request succeeded or a service exists.
    pub fn in_error_chain<'a>(error: &'a (dyn Error + 'static)) -> Option<&'a Self> {
        let mut current = Some(error);
        for _ in 0..32 {
            let error = current?;
            if let Some(observation) = error.downcast_ref::<Self>() {
                return Some(observation);
            }
            current = if let Some(io) = error.downcast_ref::<io::Error>() {
                io.get_ref()
                    .map(|source| source as &(dyn Error + 'static))
                    .or_else(|| error.source())
            } else {
                error.source()
            };
        }
        None
    }
}

impl fmt::Debug for LockdownResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockdownResponseError")
            .field("operation", &self.operation)
            .field("class", &self.class())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for LockdownResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "lockdown {} rejected ({})", self.operation, self.class())
    }
}

impl Error for LockdownResponseError {}

#[derive(Clone, Copy)]
pub(super) enum ResponseErrorPolicy {
    Legacy,
    Preserve,
}

impl ResponseErrorPolicy {
    pub(super) fn remote_error(
        self,
        operation: &'static str,
        code: String,
        legacy_message: impl FnOnce(&str) -> String,
    ) -> LockdownError {
        match self {
            Self::Legacy => LockdownError::Protocol(legacy_message(&code)),
            Self::Preserve => {
                LockdownError::Io(io::Error::other(LockdownResponseError { operation, code }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_codes_observation_is_bounded_typed_and_private() {
        struct Context(Box<dyn Error + Send + Sync>);
        impl fmt::Display for Context {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                panic!("formatted context")
            }
        }
        impl fmt::Debug for Context {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                panic!("debugged context")
            }
        }
        impl Error for Context {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(self.0.as_ref())
            }
        }
        for depth in [0, 31, 32] {
            let mut error: Box<dyn Error + Send + Sync> = Box::new(LockdownResponseError {
                operation: "GetValue",
                code: "MissingValue".into(),
            });
            for _ in 0..depth {
                error = Box::new(Context(error));
            }
            let found = LockdownResponseError::in_error_chain(error.as_ref());
            assert_eq!(found.is_some(), depth < 32);
            assert_eq!(
                crate::classify_error(error.as_ref()),
                if depth < 32 {
                    ErrorClass::NotFound
                } else {
                    ErrorClass::Unknown
                }
            );
        }
        struct Cycle;
        impl fmt::Display for Cycle {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                panic!("formatted cycle")
            }
        }
        impl fmt::Debug for Cycle {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                panic!("debugged cycle")
            }
        }
        impl Error for Cycle {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(self)
            }
        }
        assert!(LockdownResponseError::in_error_chain(&Cycle).is_none());
        assert_eq!(crate::classify_error(&Cycle), ErrorClass::Unknown);
        let error = ResponseErrorPolicy::Preserve.remote_error(
            "GetValue",
            "owned-private-MissingValue".into(),
            |_| panic!("legacy formatter called"),
        );
        let observation = LockdownResponseError::in_error_chain(&error).unwrap();
        assert_eq!(observation.code(), "owned-private-MissingValue");
        assert_eq!(observation.class(), ErrorClass::ServiceFailure);
        assert!(observation.source().is_none());
        assert!(
            !format!("{observation:?} {observation} {error:?} {error}").contains("owned-private")
        );
        let LockdownError::Io(io) = &error else {
            panic!("response observation lost")
        };
        assert_eq!(io.kind(), io::ErrorKind::Other);
        assert_eq!(io.raw_os_error(), None);
        let expected_code = observation.code().to_owned();
        let nested = io::Error::other(error);
        assert_eq!(
            LockdownResponseError::in_error_chain(&nested)
                .unwrap()
                .code(),
            expected_code
        );
    }
}
