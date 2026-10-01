//! Bounded host selection policy, independent of DeviceLink/session I/O.
//!
//! Manifest filtering and receive-time filtering share the same predicates.

use super::Mobilebackup2Error;
use std::str::FromStr;

/// Built-in host-side file selections accepted by `backup --only`.
///
/// These names deliberately mirror pymobiledevice3. They are predicates used while
/// receiving a DeviceLink upload; they are not additional MobileBackup2 message names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupSelection {
    Bookmarks,
    CallHistory,
    Contacts,
    Messages,
    Sms,
    Whatsapp,
}

impl BackupSelection {
    pub const NAMES: [&'static str; 6] = [
        "bookmarks",
        "call_history",
        "contacts",
        "messages",
        "sms",
        "whatsapp",
    ];

    pub fn rules(self) -> &'static [BackupSelectionRule] {
        match self {
            Self::Bookmarks => &BOOKMARK_RULES,
            Self::CallHistory => &CALL_HISTORY_RULES,
            Self::Contacts => &CONTACT_RULES,
            Self::Messages => &MESSAGE_RULES,
            Self::Sms => &SMS_RULES,
            Self::Whatsapp => &WHATSAPP_RULES,
        }
    }
}

impl FromStr for BackupSelection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "bookmarks" => Ok(Self::Bookmarks),
            "call_history" | "call-history" => Ok(Self::CallHistory),
            "contacts" => Ok(Self::Contacts),
            "messages" => Ok(Self::Messages),
            "sms" => Ok(Self::Sms),
            "whatsapp" | "whats_app" | "whats-app" => Ok(Self::Whatsapp),
            _ => Err(format!(
                "unknown backup selection {value:?}; expected one of {}",
                Self::NAMES.join(", ")
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupSelectionRule {
    pub domain: &'static str,
    pub relative_path: &'static str,
    pub recursive: bool,
}

impl BackupSelectionRule {
    const fn new(domain: &'static str, relative_path: &'static str, recursive: bool) -> Self {
        Self {
            domain,
            relative_path,
            recursive,
        }
    }

    fn matches(&self, domain: &str, relative_path: &str) -> bool {
        self.domain == domain
            && (self.relative_path == relative_path
                || (self.recursive
                    && relative_path
                        .strip_prefix(self.relative_path)
                        .is_some_and(|suffix| suffix.starts_with('/'))))
    }

    fn matches_device_name(&self, device_name: &str) -> bool {
        let expected = self.relative_path.trim_end_matches('/');
        let exact = [
            format!("{}/{}", self.domain, expected),
            format!("{}-{}", self.domain, expected),
            expected.to_string(),
        ];
        exact.iter().any(|candidate| {
            device_name == candidate
                || (self.recursive && device_name.starts_with(&format!("{candidate}/")))
        }) || device_name.ends_with(&format!("/{expected}"))
            || (self.recursive && device_name.contains(&format!("/{expected}/")))
    }
}

static BOOKMARK_RULES: [BackupSelectionRule; 3] = [
    BackupSelectionRule::new("HomeDomain", "Library/Safari/Bookmarks.db", false),
    BackupSelectionRule::new("HomeDomain", "Library/Safari/Bookmarks.db-shm", false),
    BackupSelectionRule::new("HomeDomain", "Library/Safari/Bookmarks.db-wal", false),
];
static CALL_HISTORY_RULES: [BackupSelectionRule; 3] = [
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/CallHistoryDB/CallHistory.storedata",
        false,
    ),
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/CallHistoryDB/CallHistory.storedata-shm",
        false,
    ),
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/CallHistoryDB/CallHistory.storedata-wal",
        false,
    ),
];
static CONTACT_RULES: [BackupSelectionRule; 3] = [
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/AddressBook/AddressBook.sqlitedb",
        false,
    ),
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/AddressBook/AddressBook.sqlitedb-shm",
        false,
    ),
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/AddressBook/AddressBook.sqlitedb-wal",
        false,
    ),
];
static MESSAGE_RULES: [BackupSelectionRule; 2] = [
    BackupSelectionRule::new("HomeDomain", "Library/SMS", true),
    BackupSelectionRule::new("MediaDomain", "Library/SMS", true),
];
static SMS_RULES: [BackupSelectionRule; 1] = [BackupSelectionRule::new(
    "HomeDomain",
    "Library/SMS/sms.db",
    false,
)];
static WHATSAPP_RULES: [BackupSelectionRule; 6] = [
    BackupSelectionRule::new(
        "AppDomain-net.whatsapp.WhatsApp",
        "Documents/ChatStorage.sqlite",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomain-net.whatsapp.WhatsApp",
        "Documents/ChatStorage.sqlite-shm",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomain-net.whatsapp.WhatsApp",
        "Documents/ChatStorage.sqlite-wal",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
        "ChatStorage.sqlite",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
        "ChatStorage.sqlite-shm",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
        "ChatStorage.sqlite-wal",
        false,
    ),
];

/// A bounded host-side filter applied to received backup entries.
#[derive(Debug, Clone)]
pub struct BackupFilter {
    selections: Vec<BackupSelection>,
    regexes: Vec<regex::Regex>,
}

impl BackupFilter {
    pub fn matches_device_name(&self, device_name: &str) -> bool {
        self.selections
            .iter()
            .flat_map(|selection| selection.rules())
            .any(|rule| rule.matches_device_name(device_name))
            || self.regexes.iter().any(|regex| regex.is_match(device_name))
    }

    pub fn matches_manifest_entry(&self, domain: &str, relative_path: &str) -> bool {
        self.selections
            .iter()
            .flat_map(|selection| selection.rules())
            .any(|rule| rule.matches(domain, relative_path))
            || self.regexes.iter().any(|regex| {
                candidate_names(domain, relative_path)
                    .iter()
                    .any(|name| regex.is_match(name))
            })
    }

    pub fn is_empty(&self) -> bool {
        self.selections.is_empty() && self.regexes.is_empty()
    }
}

/// Parse and validate the `--only` and `--only-regex` forms before a device session starts.
pub fn build_backup_filter(
    selections: &[String],
    regex_patterns: &[String],
) -> Result<Option<BackupFilter>, Mobilebackup2Error> {
    if selections.is_empty() && regex_patterns.is_empty() {
        return Ok(None);
    }
    if selections.len() + regex_patterns.len() > MAX_FILTER_TERMS {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup filter has too many terms ({}; max {MAX_FILTER_TERMS})",
            selections.len() + regex_patterns.len()
        )));
    }
    let selections = selections
        .iter()
        .map(|selection| BackupSelection::from_str(selection).map_err(Mobilebackup2Error::Protocol))
        .collect::<Result<Vec<_>, _>>()?;
    let regexes = regex_patterns
        .iter()
        .map(|pattern| {
            if pattern.len() > MAX_FILTER_PATTERN_BYTES {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup --only-regex pattern is too large ({} bytes; max {MAX_FILTER_PATTERN_BYTES})",
                    pattern.len()
                )));
            }
            regex::Regex::new(pattern).map_err(|error| {
                Mobilebackup2Error::Protocol(format!(
                    "invalid backup --only-regex pattern {pattern:?}: {error}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(BackupFilter {
        selections,
        regexes,
    }))
}

fn candidate_names<'a>(domain: &'a str, relative_path: &'a str) -> [String; 3] {
    [
        format!("{domain}/{relative_path}"),
        format!("{domain}-{relative_path}"),
        relative_path.to_string(),
    ]
}

const MAX_FILTER_TERMS: usize = 128;
const MAX_FILTER_PATTERN_BYTES: usize = 16 * 1024;

pub(super) fn should_preserve_backup_file(
    file_name: &str,
    device_name: &str,
    filter: &BackupFilter,
) -> bool {
    let metadata_name = file_name.rsplit('/').next().unwrap_or(file_name);
    matches!(
        metadata_name,
        "Info.plist"
            | "Manifest.plist"
            | "Manifest.db"
            | "Manifest.mbdb"
            | "Manifest.mbdx"
            | "Manifest.db-shm"
            | "Manifest.db-wal"
            | "Status.plist"
    ) || filter.matches_device_name(device_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_filter_matches_upstream_presets_and_manifest_candidates() {
        let filter = build_backup_filter(&["sms".into()], &[])
            .expect("selection should parse")
            .expect("selection should create a filter");
        assert!(filter.matches_device_name("HomeDomain/Library/SMS/sms.db"));
        assert!(filter.matches_manifest_entry("HomeDomain", "Library/SMS/sms.db"));
        assert!(!filter.matches_manifest_entry("HomeDomain", "Library/Notes/notes.db"));

        let recursive = build_backup_filter(&["messages".into()], &[])
            .expect("selection should parse")
            .expect("selection should create a filter");
        assert!(recursive.matches_device_name("MediaDomain/Library/SMS/Attachments/a.bin"));
    }

    #[test]
    fn backup_filter_regex_is_or_combined_and_bounded() {
        let filter = build_backup_filter(&["sms".into()], &["Notes/.*\\.db".into()])
            .expect("regex should parse")
            .expect("filter should exist");
        assert!(filter.matches_manifest_entry("HomeDomain", "Library/SMS/sms.db"));
        assert!(filter.matches_manifest_entry("HomeDomain", "Library/Notes/notes.db"));
        assert!(!filter.matches_manifest_entry("HomeDomain", "Library/Notes/notes.plist"));

        let too_long = "x".repeat(MAX_FILTER_PATTERN_BYTES + 1);
        assert!(build_backup_filter(&[], &[too_long]).is_err());
        assert!(build_backup_filter(&[], &["[".into()]).is_err());
    }

    #[test]
    fn backup_filter_keeps_metadata_and_discards_unmatched_device_paths() {
        let filter = build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        assert!(should_preserve_backup_file(
            "aa/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "HomeDomain/Library/SMS/sms.db",
            &filter
        ));
        assert!(!should_preserve_backup_file(
            "bb/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "HomeDomain/Library/Notes/notes.db",
            &filter
        ));
        for metadata in [
            "Info.plist",
            "Manifest.plist",
            "Manifest.db",
            "Manifest.mbdb",
            "Manifest.mbdx",
            "Manifest.db-shm",
            "Manifest.db-wal",
            "Status.plist",
        ] {
            assert!(should_preserve_backup_file(metadata, "", &filter));
        }
    }
}
