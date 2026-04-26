#[cfg(feature = "debugserver")]
mod tests {
    use ios_core::debugserver::{
        checksum, format_packet, parse_packet, select_service_name, LEGACY_SERVICE_NAME,
        SECURE_SERVICE_NAME,
    };
    use semver::Version;

    #[test]
    fn selects_legacy_service_for_ios_14_and_earlier() {
        assert_eq!(
            select_service_name(&Version::new(14, 8, 1)),
            LEGACY_SERVICE_NAME
        );
    }

    #[test]
    fn selects_secure_service_for_ios_15_and_newer() {
        assert_eq!(
            select_service_name(&Version::new(15, 0, 0)),
            SECURE_SERVICE_NAME
        );
    }

    #[test]
    fn checksum_matches_gdb_remote_expectation() {
        assert_eq!(checksum("qLaunchSuccess"), "a5");
    }

    #[test]
    fn format_packet_prefixes_ack_and_checksum() {
        assert_eq!(format_packet("qLaunchSuccess"), "+$qLaunchSuccess#a5");
    }

    #[test]
    fn parse_packet_skips_noise_and_returns_payload() {
        let parsed = parse_packet(b"+$OK#9aignored").expect("packet should parse");
        assert_eq!(parsed.payload, "OK");
        assert_eq!(parsed.consumed, 7);
    }

    #[test]
    fn parse_packet_waits_for_complete_checksum_suffix() {
        assert!(parse_packet(b"$OK#").is_none());
    }

    #[test]
    fn parse_packet_rejects_separator_before_dollar() {
        assert!(parse_packet(b"#12$OK#9a").is_none());
    }
}
