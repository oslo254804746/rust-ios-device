//! Public WebInspector metadata and typed wire-to-model parsing.
//! No stream ownership or mutable service/session state.

use super::wire::{
    optional_bool, optional_string, pid_from_identifier, plist_integer_to_u64, required_bool,
    required_string,
};
use super::WebInspectorError;
use serde::Serialize;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationAvailability {
    NotAvailable,
    Available,
    Unknown(String),
}

impl AutomationAvailability {
    pub(super) fn from_wire(value: &str) -> Self {
        match value {
            "WIRAutomationAvailabilityNotAvailable" => Self::NotAvailable,
            "WIRAutomationAvailabilityAvailable" => Self::Available,
            other => Self::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WirType {
    Automation,
    Itml,
    JavaScript,
    Page,
    ServiceWorker,
    Web,
    WebPage,
    AutomaticallyPause,
    Unknown(String),
}

impl WirType {
    fn from_wire(value: &str) -> Self {
        match value {
            "WIRTypeAutomation" => Self::Automation,
            "WIRTypeITML" => Self::Itml,
            "WIRTypeJavaScript" => Self::JavaScript,
            "WIRTypePage" => Self::Page,
            "WIRTypeServiceWorker" => Self::ServiceWorker,
            "WIRTypeWeb" => Self::Web,
            "WIRTypeWebPage" => Self::WebPage,
            "WIRAutomaticallyPause" => Self::AutomaticallyPause,
            other => Self::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Application {
    pub id: String,
    pub bundle_identifier: String,
    pub pid: u64,
    pub name: String,
    pub availability: AutomationAvailability,
    pub is_active: bool,
    pub is_proxy: bool,
    pub is_ready: bool,
    pub host_application_identifier: Option<String>,
}

impl Application {
    pub(super) fn from_plist(dict: &plist::Dictionary) -> Result<Self, WebInspectorError> {
        let id = required_string(dict, "WIRApplicationIdentifierKey")?.to_string();
        Ok(Self {
            pid: pid_from_identifier(&id)?,
            id,
            bundle_identifier: required_string(dict, "WIRApplicationBundleIdentifierKey")?
                .to_string(),
            name: required_string(dict, "WIRApplicationNameKey")?.to_string(),
            availability: AutomationAvailability::from_wire(required_string(
                dict,
                "WIRAutomationAvailabilityKey",
            )?),
            is_active: required_bool(dict, "WIRIsApplicationActiveKey")?,
            is_proxy: required_bool(dict, "WIRIsApplicationProxyKey")?,
            is_ready: required_bool(dict, "WIRIsApplicationReadyKey")?,
            host_application_identifier: optional_string(dict, "WIRHostApplicationIdentifierKey")
                .map(ToOwned::to_owned),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Page {
    pub id: u64,
    pub listing_key: String,
    pub page_type: WirType,
    pub title: Option<String>,
    pub url: Option<String>,
    pub automation_is_paired: Option<bool>,
    pub automation_name: Option<String>,
    pub automation_version: Option<String>,
    pub automation_session_id: Option<String>,
    pub automation_connection_id: Option<String>,
}

impl Page {
    pub(super) fn from_plist(
        listing_key: &str,
        dict: &plist::Dictionary,
    ) -> Result<Self, WebInspectorError> {
        let id = match dict.get("WIRPageIdentifierKey") {
            Some(value) => plist_integer_to_u64(value).ok_or_else(|| {
                WebInspectorError::Protocol("WIRPageIdentifierKey must be an integer".to_string())
            })?,
            None => listing_key.parse::<u64>().map_err(|error| {
                super::native_error(
                    std::io::ErrorKind::InvalidData,
                    "decode WebInspector page identifier",
                    error,
                )
            })?,
        };

        Ok(Self {
            id,
            listing_key: listing_key.to_string(),
            page_type: WirType::from_wire(required_string(dict, "WIRTypeKey")?),
            title: optional_string(dict, "WIRTitleKey").map(ToOwned::to_owned),
            url: optional_string(dict, "WIRURLKey").map(ToOwned::to_owned),
            automation_is_paired: optional_bool(dict, "WIRAutomationTargetIsPairedKey"),
            automation_name: optional_string(dict, "WIRAutomationTargetNameKey")
                .map(ToOwned::to_owned),
            automation_version: optional_string(dict, "WIRAutomationTargetVersionKey")
                .map(ToOwned::to_owned),
            automation_session_id: optional_string(dict, "WIRSessionIdentifierKey")
                .map(ToOwned::to_owned),
            automation_connection_id: optional_string(dict, "WIRConnectionIdentifierKey")
                .map(ToOwned::to_owned),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplicationPage {
    pub application: Application,
    pub page: Page,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebInspectorEvent {
    CurrentState {
        availability: AutomationAvailability,
    },
    ConnectedApplications {
        applications: Vec<Application>,
    },
    ConnectedDrivers,
    Listing {
        application_id: String,
        pages: Vec<Page>,
    },
    ApplicationUpdated {
        application: Application,
    },
    ApplicationConnected {
        application: Application,
    },
    SocketData {
        application_id: Option<String>,
        message: JsonValue,
    },
    ApplicationDisconnected {
        application_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_discovery_page_id_retains_parse_source_and_explicit_id_precedence() {
        let mut dictionary = plist::Dictionary::from_iter([(
            "WIRTypeKey".to_string(),
            plist::Value::String("WIRTypeWebPage".into()),
        )]);
        let error = Page::from_plist("owned-private", &dictionary).unwrap_err();
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        assert!(crate::test_native::has_source::<std::num::ParseIntError>(
            &error
        ));
        assert!(!format!("{error:?}").contains("owned-private"));
        assert_eq!(Page::from_plist("42", &dictionary).unwrap().id, 42);
        dictionary.insert(
            "WIRPageIdentifierKey".into(),
            plist::Value::Integer(7.into()),
        );
        assert_eq!(
            Page::from_plist("owned-private", &dictionary).unwrap().id,
            7
        );
    }
}
