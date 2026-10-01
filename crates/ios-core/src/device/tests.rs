use std::path::PathBuf;
#[test]
fn device_connection_debug_hides_credential_paths_and_network_identity() {
    let secret = "synthetic-connection-private-identity";
    let options = super::ConnectOptions {
        pair_record_path: Some(PathBuf::from(secret)),
        ..Default::default()
    };
    let paired = super::PairedMobdev2Device {
        udid: secret.into(),
        host: secret.into(),
    };
    let transport = super::LockdownTransport::Tcp {
        host: secret.into(),
    };
    let output =
        format!("{options:?} {options:#?} {paired:?} {paired:#?} {transport:?} {transport:#?}");
    assert!(!output.contains(secret));
    assert!(output.contains("pair_record_configured"));
    assert_eq!(paired.udid, secret);
    assert_eq!(options.pair_record_path, Some(PathBuf::from(secret)));
}
