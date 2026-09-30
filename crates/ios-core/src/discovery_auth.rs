//! Offline recognition of Bonjour advertisements using existing pairing keys.
//!
//! A mobdev2 tag identifies a *host*, not a device: several device records may
//! share HostID. RemotePairing's tag uses the device's own alternate IRK.

use std::collections::HashMap;
#[cfg(any(feature = "tunnel", test))]
use std::hash::Hasher;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Sha256, Sha512};
#[cfg(any(feature = "tunnel", test))]
use siphasher::sip::SipHasher24;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

fn is_auth_tag_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("authTag")
        || key.split_once('#').is_some_and(|(prefix, index)| {
            prefix.eq_ignore_ascii_case("authTag")
                && !index.is_empty()
                && index.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn resembles_auth_tag_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("authTag")
        || key
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("authTag#"))
}

fn property<'a>(properties: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    let mut found = properties
        .iter()
        .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.as_str());
    let value = found.next()?;
    // DNS-SD TXT keys are case insensitive; conflicting variants are ambiguous.
    found.all(|other| other == value).then_some(value)
}

fn identifier(properties: &HashMap<String, String>) -> Option<&str> {
    property(properties, "identifier").filter(|value| !value.is_empty() && value.len() <= 255)
}

fn decode_tag<const N: usize>(encoded: &str) -> Option<[u8; N]> {
    if encoded.len() > 16 {
        return None;
    }
    let mut decoded = [0; N];
    (STANDARD.decode_slice(encoded, &mut decoded).ok()? == N).then_some(decoded)
}

pub(crate) fn mobdev2_auth_tag(host_id: &str, identifier: &str) -> [u8; 8] {
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha512>::new(None, host_id.as_bytes())
        .expand(&[], key.as_mut())
        .expect("32 bytes fits HKDF-SHA512 output limit");
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.as_ref()).expect("HMAC accepts any key length");
    mac.update(identifier.as_bytes());
    mac.finalize().into_bytes()[..8].try_into().unwrap()
}

/// Missing tags are permitted for older devices. Present but unusable tags
/// fail closed, so malformed metadata cannot force trying every pair record.
pub(crate) fn mobdev2_host_matches(properties: &HashMap<String, String>, host_id: &str) -> bool {
    if !properties.keys().any(|key| resembles_auth_tag_key(key)) {
        return true;
    }
    let Some(identifier) = identifier(properties) else {
        return false;
    };
    let expected = mobdev2_auth_tag(host_id, identifier);
    properties.iter().take(64).any(|(key, value)| {
        is_auth_tag_key(key)
            && decode_tag::<8>(value).is_some_and(|tag| bool::from(tag.ct_eq(&expected)))
    })
}

#[cfg(any(feature = "tunnel", test))]
fn remote_auth_tag(alt_irk: &[u8; 16], identifier: &str) -> [u8; 6] {
    let mut hash = SipHasher24::new_with_key(alt_irk);
    hash.write(identifier.as_bytes());
    let bytes = hash.finish().to_le_bytes();
    [bytes[5], bytes[4], bytes[3], bytes[2], bytes[1], bytes[0]]
}

#[cfg(any(feature = "tunnel", test))]
pub(crate) fn remote_pairing_matches(
    properties: &HashMap<String, String>,
    alt_irk: &[u8; 16],
) -> bool {
    let Some(identifier) = identifier(properties) else {
        return false;
    };
    let Some(tag) = property(properties, "authTag").and_then(decode_tag::<6>) else {
        return false;
    };
    bool::from(tag.ct_eq(&remote_auth_tag(alt_irk, identifier)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTIFIER: &str = "2BE6E510-0325-4365-923E-B14C6F57DB3A";
    const HOST: &str = "11111111-2222-3333-4444-555555555555";

    #[test]
    fn mobdev2_fixed_protocol_vector_and_multiple_hosts() {
        assert_eq!(
            STANDARD.encode(mobdev2_auth_tag(HOST, IDENTIFIER)),
            "DNKInlok1wk="
        );
        let mut properties = HashMap::from([
            ("identifier".into(), IDENTIFIER.into()),
            ("authTag".into(), "AAAAAAAAAAA=".into()),
            ("authTag#1".into(), "DNKInlok1wk=".into()),
        ]);
        assert!(mobdev2_host_matches(&properties, HOST));
        assert!(!mobdev2_host_matches(&properties, "another-host"));
        properties.insert("identifier".into(), "another-identifier".into());
        assert!(!mobdev2_host_matches(&properties, HOST));
    }

    #[test]
    fn malformed_tags_do_not_downgrade_to_legacy_matching() {
        for invalid in ["", "!invalid!", "AA==", "AAAAAAAA", "AAAAAAAAAAAA"] {
            let properties = HashMap::from([
                ("identifier".into(), IDENTIFIER.into()),
                ("authTag".into(), invalid.into()),
            ]);
            assert!(!mobdev2_host_matches(&properties, HOST));
            assert!(!remote_pairing_matches(&properties, &[0; 16]));
        }
        assert!(mobdev2_host_matches(&HashMap::new(), HOST));
        assert!(!remote_pairing_matches(&HashMap::new(), &[0; 16]));
        assert!(!mobdev2_host_matches(
            &HashMap::from([("authTag".into(), "DNKInlok1wk=".into())]),
            HOST
        ));
        assert!(!mobdev2_host_matches(
            &HashMap::from([("authTag#invalid".into(), "DNKInlok1wk=".into())]),
            HOST
        ));
    }

    #[test]
    fn remote_pairing_fixed_protocol_vector() {
        let mut key = [0u8; 16];
        STANDARD
            .decode_slice("Mgp6ZGPzXM2ku9br46vsiw==", &mut key)
            .unwrap();
        assert_eq!(
            STANDARD.encode(remote_auth_tag(&key, IDENTIFIER)),
            "kXjlTr2l"
        );
        let mut properties = HashMap::from([
            ("IDENTIFIER".into(), IDENTIFIER.into()),
            ("AUTHTAG".into(), "kXjlTr2l".into()),
        ]);
        assert!(remote_pairing_matches(&properties, &key));
        assert!(!remote_pairing_matches(&properties, &[0; 16]));
        properties.insert("identifier".into(), "conflicting-value".into());
        assert!(!remote_pairing_matches(&properties, &key));
    }

    #[test]
    fn siphash_rounds_and_byte_order_match_standard_vectors() {
        let key = std::array::from_fn(|index| index as u8);
        for (length, expected) in [
            (0, 0x726f_db47_dd0e_0e31),
            (1, 0x74f8_39c5_93dc_67fd),
            (7, 0xab02_00f5_8b01_d137),
            (8, 0x93f5_f579_9a93_2462),
        ] {
            let mut hash = SipHasher24::new_with_key(&key);
            hash.write(&(0..length).collect::<Vec<u8>>());
            assert_eq!(hash.finish(), expected);
        }
    }
}
