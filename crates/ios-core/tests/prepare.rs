#[test]
fn build_cloud_configuration_embeds_supervision_certificate() {
    let cloud = ios_core::services::prepare::build_cloud_configuration(
        &["WiFi".to_string(), "Privacy".to_string()],
        Some(&[1, 2, 3, 4]),
        Some("Example Org"),
    );

    assert_eq!(
        cloud.get("AllowPairing").and_then(plist::Value::as_boolean),
        Some(true)
    );
    assert_eq!(
        cloud.get("IsSupervised").and_then(plist::Value::as_boolean),
        Some(true)
    );
    assert_eq!(
        cloud
            .get("OrganizationName")
            .and_then(plist::Value::as_string),
        Some("Example Org")
    );
}

#[test]
fn generated_supervision_identity_contains_der_and_pkcs12() {
    let identity =
        ios_core::services::prepare::generate_supervision_identity("ios-rs", "secret").unwrap();

    assert!(!identity.certificate_der.is_empty());
    assert!(!identity.certificate_pem.is_empty());
    assert!(!identity.private_key_pem.is_empty());
    assert!(!identity.pkcs12_der.is_empty());
}
