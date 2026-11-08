//! Local directory observations never read pairing-record contents.

use std::path::Path;

use super::{device::State, Check, Status};

const MAX_DIRECTORY_ENTRIES: usize = 4096;

#[cfg(any(target_os = "windows", target_os = "macos", test))]
pub(super) fn windows_usb_output(stdout: &[u8]) -> Check {
    let parsed = serde_json::from_slice::<serde_json::Value>(stdout);
    let value = match parsed {
        Ok(value) => value,
        Err(error) => {
            return Check::io_failure(
                "platform_usb",
                std::io::Error::new(std::io::ErrorKind::InvalidData, error),
                "USB PnP enumeration returned malformed JSON; USB visibility is unknown.",
            )
        }
    };
    let counts = value["total"]
        .as_u64()
        .zip(value["apple"].as_u64())
        .zip(value["failed"].as_u64());
    let Some(((total, apple), failed)) =
        counts.filter(|((total, apple), failed)| apple <= total && failed <= total)
    else {
        return Check::new("platform_usb", Status::Unknown,
            "USB PnP enumeration returned missing, invalid or inconsistent counts; USB visibility is unknown.").state(State::MetadataMissing);
    };
    Check::new("platform_usb", if failed == 0 { Status::Ok } else { Status::Warning },
        format!("Enumerated {total} present USB PnP entries, {apple} Apple entries, {failed} entries with a driver error; enumeration does not establish selected-device identity, trust or service connectivity."))
        .state(State::Available)
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
pub(super) fn ioreg_usb_output(stdout: &[u8]) -> Check {
    let text = match std::str::from_utf8(stdout) {
        Ok(text) => text,
        Err(error) => {
            return Check::io_failure(
                "platform_usb",
                std::io::Error::new(std::io::ErrorKind::InvalidData, error),
                "IOUSB enumeration returned invalid text; USB visibility is unknown.",
            )
        }
    };
    let mut total = 0usize;
    let mut apple = 0usize;
    for line in text.lines().filter(|line| line.contains("\"idVendor\"")) {
        let vendor = line
            .split_once("\"idVendor\" = ")
            .and_then(|(_, value)| value.trim().parse::<u16>().ok());
        let Some(vendor) = vendor else {
            return Check::new(
                "platform_usb",
                Status::Unknown,
                "An IOUSB vendor field was malformed; USB visibility is incomplete.",
            )
            .state(State::MetadataMissing);
        };
        total += 1;
        apple += usize::from(vendor == 1452);
    }
    Check::new("platform_usb", if total > 0 { Status::Ok } else { Status::Unknown },
        format!("Enumerated {total} IOUSB vendor entries, including {apple} Apple entries; enumeration does not establish selected-device identity, trust or service connectivity."))
        .state(if total > 0 { State::Available } else { State::MetadataMissing })
}

pub(super) async fn probe_pairing_directory() -> Check {
    // Derive the directory from the public resolver used by PairRecord::load;
    // do not maintain a second platform path table. A missing Windows profile
    // would otherwise produce a relative path into the current working tree.
    let record_path = ios_core::default_pair_record_path("doctor-unused");
    let Some(path) = record_path.parent().filter(|path| path.is_absolute()) else {
        return Check::new(
            "pairing_directory",
            Status::Unknown,
            "The existing pairing-store resolver did not yield an absolute directory; its location was not inspected.",
        )
        .state(State::MetadataMissing)
        .hint("Check the platform pairing-store configuration; on Windows ALLUSERSPROFILE must identify its absolute root.");
    };
    let path = path.to_owned();
    match tokio::task::spawn_blocking(move || inspect_pairing_directory(&path)).await {
        Ok(check) => check,
        Err(_) => Check::new(
            "pairing_directory",
            Status::Unknown,
            "The pairing-directory observation worker did not complete.",
        )
        .state(State::Unreachable),
    }
}

fn inspect_pairing_directory(path: &Path) -> Check {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            return Check::io_failure(
                "pairing_directory",
                error,
                "The resolved existing pairing directory could not be enumerated; no trust-record contents were read.",
            )
            .hint("Check the existing platform pairing-store location and its read permissions. Doctor does not create directories or pairing records.");
        }
    };
    let mut regular_records = 0usize;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES {
            return Check::new(
                "pairing_directory",
                Status::Unknown,
                "Pairing-directory observation reached its 4096-entry limit; enumeration was incomplete and no record contents were read.",
            )
            .state(State::MetadataMissing);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return pairing_entry_failure(error),
        };
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) => return pairing_entry_failure(error),
        };
        if kind.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|value| value == "plist")
        {
            regular_records += 1;
        }
    }
    Check::new(
        "pairing_directory",
        Status::Ok,
        format!("The resolved existing pairing directory was enumerated completely: {regular_records} regular plist file(s). Names, contents, selected-target trust and write access were not inspected."),
    )
    .state(State::Available)
}

fn pairing_entry_failure(error: std::io::Error) -> Check {
    Check::io_failure(
        "pairing_directory",
        error,
        "A pairing-directory entry could not be observed; enumeration was incomplete and no record contents were read.",
    )
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn inspect_linux_usb(path: &Path) -> Check {
    use std::io::Read;
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            return Check::io_failure(
                "platform_usb",
                error,
                "The Linux sysfs USB directory could not be enumerated; USB visibility is unknown.",
            );
        }
    };
    let mut total = 0usize;
    let mut apple = 0usize;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES {
            return Check::new(
                "platform_usb",
                Status::Unknown,
                "Linux sysfs USB enumeration reached its 4096-entry limit; USB visibility is incomplete.",
            )
            .state(State::MetadataMissing);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return usb_entry_failure(error),
        };
        let file = match std::fs::File::open(entry.path().join("idVendor")) {
            Ok(file) => file,
            // USB interface entries legitimately have no idVendor file. A
            // disappearing device is equally not evidence of an Apple device.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return usb_entry_failure(error),
        };
        let mut vendor = Vec::new();
        if let Err(error) = file.take(65).read_to_end(&mut vendor) {
            return usb_entry_failure(error);
        }
        let vendor = std::str::from_utf8(&vendor).ok().map(str::trim);
        let Some(vendor) = vendor.filter(|vendor| {
            vendor.len() == 4 && vendor.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) else {
            return Check::io_failure(
                "platform_usb",
                std::io::Error::from(std::io::ErrorKind::InvalidData),
                "A Linux USB vendor entry was malformed or exceeded its observation bound; USB visibility is incomplete.",
            );
        };
        total += 1;
        apple += usize::from(vendor.eq_ignore_ascii_case("05ac"));
    }
    Check::new(
        "platform_usb",
        Status::Ok,
        format!("Enumerated {total} Linux sysfs USB vendor entries, including {apple} Apple entries; enumeration does not establish selected-device identity, trust or service connectivity."),
    )
    .state(State::Available)
}

#[cfg(any(target_os = "linux", test))]
fn usb_entry_failure(error: std::io::Error) -> Check {
    Check::io_failure(
        "platform_usb",
        error,
        "A Linux sysfs USB entry could not be read; USB visibility is incomplete.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("doctor-directories-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn pairing_directory_observes_names_only_and_distinguishes_missing_and_non_directory() {
        let root = directory();
        let secret = root.join("owned-private-name.plist");
        std::fs::write(&secret, b"owned-private-record-contents").unwrap();
        std::fs::create_dir(root.join("other.plist")).unwrap();
        let check = inspect_pairing_directory(&root);
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.state, Some(State::Available));
        assert!(check.detail.contains("1 regular plist"));
        let output = format!("{} {check:?}", serde_json::to_string(&check).unwrap());
        assert!(!output.contains("owned-private"));
        assert!(!output.contains(&root.to_string_lossy().to_string()));
        let missing = inspect_pairing_directory(&root.join("missing"));
        assert_eq!(missing.state, Some(State::MetadataMissing));
        assert_eq!(missing.error_class, Some(ios_core::ErrorClass::NotFound));
        assert!(missing.error.as_ref().unwrap().is::<std::io::Error>());
        let file = inspect_pairing_directory(&secret);
        assert_ne!(file.status, Status::Ok);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn host_io_failure_keeps_original_cause_and_redacts_permission_path() {
        let check = pairing_entry_failure(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "owned-private-path",
        ));
        assert_eq!(check.state, Some(State::PermissionDenied));
        assert_eq!(
            check.error_class,
            Some(ios_core::ErrorClass::PermissionDenied)
        );
        assert_eq!(
            check
                .error
                .as_ref()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert!(
            !format!("{} {check:?}", serde_json::to_string(&check).unwrap())
                .contains("owned-private")
        );
    }

    #[test]
    fn linux_usb_fixture_cannot_promote_missing_or_malformed_entries_to_ok() {
        let root = directory();
        assert_eq!(inspect_linux_usb(&root).status, Status::Ok);
        let device = root.join("fixture-device");
        std::fs::create_dir(&device).unwrap();
        std::fs::write(device.join("idVendor"), b"05AC\n").unwrap();
        let check = inspect_linux_usb(&root);
        assert_eq!(check.state, Some(State::Available));
        assert!(check.detail.contains("including 1 Apple"));
        std::fs::write(device.join("idVendor"), b"1452 private fixture").unwrap();
        let malformed = inspect_linux_usb(&root);
        assert_ne!(malformed.status, Status::Ok);
        assert_eq!(malformed.error_class, Some(ios_core::ErrorClass::Protocol));
        let missing = inspect_linux_usb(&root.join("missing"));
        assert_eq!(missing.state, Some(State::MetadataMissing));
        assert!(!missing.detail.contains("operational"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_usb_output_requires_consistent_counts_and_exact_vendor_values() {
        for body in [
            b"{}".as_slice(),
            br#"{"total":1,"apple":2,"failed":0}"#,
            br#"{"total":1,"apple":0,"failed":2}"#,
            br#"{"total":"1","apple":0,"failed":0}"#,
        ] {
            assert_eq!(windows_usb_output(body).status, Status::Unknown);
        }
        let empty = windows_usb_output(br#"{"total":0,"apple":0,"failed":0}"#);
        assert_eq!(empty.status, Status::Ok);
        assert_eq!(empty.state, Some(State::Available));
        assert_eq!(
            windows_usb_output(b"invalid-private-json").error_class,
            Some(ios_core::ErrorClass::Protocol)
        );
        let errors = windows_usb_output(br#"{"total":3,"apple":1,"failed":2}"#);
        assert_eq!(errors.status, Status::Warning);
        assert!(errors.detail.contains("2 entries with a driver error"));
        let ioreg = ioreg_usb_output(b"| | \"idVendor\" = 1452\n| \"idVendor\" = 14520\n");
        assert_eq!(ioreg.status, Status::Ok);
        assert!(ioreg
            .detail
            .contains("2 IOUSB vendor entries, including 1 Apple"));
        assert_eq!(
            ioreg_usb_output(b"\"idVendor\" = private\n").status,
            Status::Unknown
        );
        assert_eq!(
            ioreg_usb_output(&[0xff]).error_class,
            Some(ios_core::ErrorClass::Protocol)
        );
        assert_eq!(
            ioreg_usb_output(b"empty tree").state,
            Some(State::MetadataMissing)
        );
    }
}
