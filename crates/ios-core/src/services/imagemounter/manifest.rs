//! BuildManifest.plist parser for personalized DDI mounting.
//!
//! Extracts BuildIdentity entries and matches them against device board/chip IDs.
//!
//! Reference: go-ios/ios/imagemounter/manifest.go

use super::protocol::ImageMounterError;

/// Parsed BuildManifest.plist.
pub struct BuildManifest {
    pub build_identities: Vec<plist::Dictionary>,
}

impl BuildManifest {
    /// Parse a BuildManifest.plist from bytes.
    pub fn parse(data: &[u8]) -> Result<Self, ImageMounterError> {
        let val: plist::Value = plist::from_bytes(data)
            .map_err(|e| ImageMounterError::Plist(format!("parse BuildManifest: {e}")))?;

        let dict = val
            .as_dictionary()
            .ok_or_else(|| ImageMounterError::Plist("BuildManifest is not a dictionary".into()))?;

        let identities = dict
            .get("BuildIdentities")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ImageMounterError::Plist("missing BuildIdentities array".into()))?;

        let build_identities = identities
            .iter()
            .filter_map(|v| v.as_dictionary().cloned())
            .collect();

        Ok(Self { build_identities })
    }

    /// Find the matching BuildIdentity for a device's board ID and chip ID.
    ///
    /// `board_id` and `chip_id` come from the device's personalization identifiers.
    pub fn find_identity(
        &self,
        board_id: u64,
        chip_id: u64,
    ) -> Result<&plist::Dictionary, ImageMounterError> {
        for identity in &self.build_identities {
            let id_board = get_int_value(identity, "ApBoardID").unwrap_or(u64::MAX);
            let id_chip = get_int_value(identity, "ApChipID").unwrap_or(u64::MAX);

            if id_board == board_id && id_chip == chip_id {
                return Ok(identity);
            }
        }

        // Fallback: try matching chip ID only (some manifests don't have per-board entries)
        for identity in &self.build_identities {
            let id_chip = get_int_value(identity, "ApChipID").unwrap_or(u64::MAX);
            if id_chip == chip_id {
                return Ok(identity);
            }
        }

        Err(ImageMounterError::Protocol(format!(
            "no matching BuildIdentity for board_id={board_id:#x}, chip_id={chip_id:#x}"
        )))
    }
}

/// Extract an integer value from a plist dictionary.
/// Handles both Integer and String representations (hex "0x8030" or decimal).
fn get_int_value(dict: &plist::Dictionary, key: &str) -> Option<u64> {
    match dict.get(key)? {
        plist::Value::Integer(n) => n.as_unsigned(),
        plist::Value::String(s) => {
            if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(hex, 16).ok()
            } else {
                s.parse().ok()
            }
        }
        _ => None,
    }
}
