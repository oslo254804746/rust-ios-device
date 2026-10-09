//! Caller-selected existing routes, with no tunnel ownership or service calls.

use std::{error::Error, fmt, net::SocketAddrV6, time::Duration};

use anyhow::Result;
use ios_core::{ErrorClass, ExistingRsdObservation, ExistingRsdRoute, ExistingRsdServiceEvidence};
use serde::Serialize;

use super::super::Status;
use super::{Check, State};

const MAX_SERVICE_BYTES: usize = 4096;

pub(crate) struct Request {
    route: ExistingRsdRoute,
    service: Option<String>,
}

impl Request {
    pub(crate) fn from_selectors(
        udid: Option<&str>,
        address: Option<SocketAddrV6>,
        proxy_port: Option<u16>,
        service: Option<&str>,
    ) -> Result<Option<Self>> {
        let Some(address) = address else {
            if proxy_port.is_some() || service.is_some() {
                return Err(crate::output::input_decision(
                    "doctor RSD proxy/service selectors require an explicit RSD address",
                ));
            }
            return Ok(None);
        };
        let udid = udid.ok_or_else(|| {
            crate::output::input_decision(
                "doctor RSD observation requires an explicit --udid or IOS_UDID",
            )
        })?;
        super::validate_target(udid)?;
        if service.is_some_and(|name| name.len() > MAX_SERVICE_BYTES) {
            return Err(crate::output::resource_decision(
                "doctor RSD service selector exceeds the 4096-byte limit",
            ));
        }
        // Constructors validate ports and proxy scope before any native source
        // identity read, host check, listener access or connection attempt.
        let route = match proxy_port {
            Some(port) => ExistingRsdRoute::via_userspace_proxy(address, port)?,
            None => ExistingRsdRoute::direct(address)?,
        };
        Ok(Some(Self {
            route,
            service: service.map(str::to_owned),
        }))
    }
}

/// Optional schema-2 evidence contains categories/counts, never remote names.
#[derive(Clone, Debug, Serialize)]
pub(super) struct Metadata {
    path: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    directory_complete: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_service: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    features: Option<FeatureMetadata>,
}

#[derive(Clone, Debug, Serialize)]
struct FeatureMetadata {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<usize>,
}

impl Metadata {
    fn selected(request: &Request) -> Self {
        Self {
            path: if request.route.proxy_port().is_some() {
                "userspace_proxy"
            } else {
                "direct_ipv6"
            },
            directory_complete: None,
            selected_service: None,
            features: None,
        }
    }
}

#[derive(Debug)]
struct AdvertisementMissing;

impl fmt::Display for AdvertisementMissing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("selected service was absent from a complete existing RSD directory")
    }
}

impl Error for AdvertisementMissing {}

pub(super) async fn inspect(request: &Request, udid: &str, timeout: Duration) -> [Check; 2] {
    let metadata = Metadata::selected(request);
    if let Err(error) = super::validate_target(udid) {
        return failed_pair(error, ErrorClass::InvalidInput, metadata);
    }
    // One additive API call owns the complete handshake budget. It drops its
    // own sockets on cancellation and opens fresh compatibility streams itself.
    match ios_core::observe_existing_rsd(request.route, timeout).await {
        Ok(observation) => observed(request, udid, observation),
        Err(error) => {
            let class = ios_core::classify_error(&error);
            failed_pair(anyhow::Error::new(error), class, metadata)
        }
    }
}

fn check(id: &'static str, metadata: Metadata) -> Check {
    Check {
        id,
        status: Status::Unknown,
        state: State::MetadataMissing,
        value: None,
        error_class: None,
        detail: "Required existing RSD identity, directory or port metadata was absent; no capability value was inferred.",
        hint: None,
        error: None,
        rsd: Some(metadata),
    }
}

fn failed_pair(error: anyhow::Error, class: ErrorClass, metadata: Metadata) -> [Check; 2] {
    let state = match class {
        ErrorClass::Timeout
        | ErrorClass::Disconnected
        | ErrorClass::DeviceNotFound
        | ErrorClass::Io => State::Unreachable,
        ErrorClass::PermissionDenied => State::PermissionDenied,
        ErrorClass::ServiceMissing => State::ServiceUnavailable,
        _ => State::Failed,
    };
    let mut services = check("rsd_services", metadata.clone());
    let mut route = check("device_route", metadata);
    for entry in [&mut services, &mut route] {
        entry.status = Status::Failed;
        entry.state = state;
        entry.error_class = Some(class);
        entry.detail = "The explicitly selected existing RSD observation failed; its typed category is reported without private route or response data.";
    }
    // The two checks describe one handshake. Retain its original native cause
    // once, so Assessment's existing typed context preserves exact downcasts.
    services.error = Some(error);
    [services, route]
}

fn observed(request: &Request, udid: &str, observation: ExistingRsdObservation) -> [Check; 2] {
    let mut metadata = Metadata::selected(request);
    match observation.udid() {
        None => {
            return [
                check("rsd_services", metadata.clone()),
                check("device_route", metadata),
            ]
        }
        Some(remote) if remote != udid => {
            return failed_pair(
                crate::output::precondition_decision(
                    "existing RSD route identified a different device",
                ),
                ErrorClass::Precondition,
                metadata,
            )
        }
        Some(_) => {}
    }
    let complete = observation.complete_service_directory();
    metadata.directory_complete = Some(complete);
    if !complete {
        if request.service.is_some() {
            metadata.selected_service = Some("metadata_missing");
        }
        return [
            check("rsd_services", metadata.clone()),
            check("device_route", metadata),
        ];
    }
    let mut route = check("device_route", metadata.clone());
    route.status = Status::Ok;
    route.state = State::Available;
    route.value = Some(true);
    route.detail = "A fresh existing RSD handshake identified the explicit target and supplied a complete service directory. Platform routing and remoted coexistence were not established.";
    let mut services = check("rsd_services", metadata);
    match request.service.as_deref() {
        None => {
            services.status = Status::Ok;
            services.state = State::Available;
            services.value = Some(true);
            services.detail = "The identified target supplied a complete RSD service directory. Advertisements do not establish service execution or readiness.";
        }
        Some(name) => match observation.service_evidence(name) {
            ExistingRsdServiceEvidence::Advertised => {
                services.status = Status::Ok;
                services.state = State::Available;
                services.value = Some(true);
                services.detail = "The complete RSD directory advertised the selected service and its port. No service connection or execution was attempted.";
                let metadata = services.rsd.as_mut().unwrap();
                metadata.selected_service = Some("advertised");
                // Match the checked API's canonical-first rule. Incomplete
                // canonical entries never authorize a shim fallback.
                let entry = observation.services().and_then(|directory| {
                    directory
                        .get(name)
                        .or_else(|| directory.get(&format!("{name}.shim.remote")))
                });
                metadata.features = entry.map(|entry| match entry.features() {
                    None => FeatureMetadata {
                        state: "metadata_missing",
                        count: None,
                    },
                    Some([]) => FeatureMetadata {
                        state: "empty",
                        count: Some(0),
                    },
                    Some(features) => FeatureMetadata {
                        state: "present",
                        count: Some(features.len()),
                    },
                });
            }
            ExistingRsdServiceEvidence::NotAdvertised => {
                services.status = Status::Failed;
                services.state = State::ServiceUnavailable;
                services.value = Some(false);
                services.error_class = Some(ErrorClass::ServiceMissing);
                services.detail = "The complete RSD directory did not advertise the selected service or its shim. This does not establish an explicit rejection of service execution.";
                services.error = Some(AdvertisementMissing.into());
                services.rsd.as_mut().unwrap().selected_service = Some("not_advertised");
            }
            _ => {
                services.rsd.as_mut().unwrap().selected_service = Some("metadata_missing");
            }
        },
    }
    [services, route]
}

#[cfg(test)]
mod tests;
