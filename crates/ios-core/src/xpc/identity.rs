//! Stable host identity for RSD. This is deliberately independent of a device's
//! pairing record: all processes connecting to a tunnel must announce one UUID.

use std::io;

use tokio::sync::OnceCell;
use uuid::Uuid;

static HANDSHAKE_UUID: OnceCell<Uuid> = OnceCell::const_new();

pub(crate) async fn default_handshake_uuid() -> io::Result<Uuid> {
    HANDSHAKE_UUID
        .get_or_try_init(resolve_handshake_uuid)
        .await
        .copied()
}

async fn resolve_handshake_uuid() -> io::Result<Uuid> {
    #[cfg(target_os = "macos")]
    if let Some(identity) = host_remoted_uuid().await {
        return Ok(identity);
    }
    Ok(hostname_uuid(&current_hostname()?))
}

/// UUIDv3 in the DNS namespace, matching the deterministic pairing identity.
pub(crate) fn hostname_uuid(hostname: &str) -> Uuid {
    let mut digest = md5::Context::new();
    digest.consume(Uuid::NAMESPACE_DNS.as_bytes());
    digest.consume(hostname.as_bytes());
    let mut bytes = digest.compute().0;
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

/// Read the OS hostname, avoiding environment differences between shells and
/// privileged processes. An unavailable identity is an error, never a random ID.
pub(crate) fn current_hostname() -> io::Result<String> {
    #[cfg(unix)]
    {
        let mut buffer = [0u8; 1024];
        // SAFETY: the writable buffer has exactly the length passed to libc.
        if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let end = buffer.iter().position(|byte| *byte == 0).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "host name exceeds identity limit",
            )
        })?;
        parse_hostname(&buffer[..end]).map(str::to_owned)
    }
    #[cfg(not(unix))]
    {
        parse_environment_hostname(std::env::var("COMPUTERNAME"))
    }
}

#[cfg(any(not(unix), test))]
fn parse_environment_hostname(value: Result<String, std::env::VarError>) -> io::Result<String> {
    let hostname = value.map_err(|error| {
        let kind = match &error {
            std::env::VarError::NotPresent => io::ErrorKind::NotFound,
            std::env::VarError::NotUnicode(_) => io::ErrorKind::InvalidData,
        };
        crate::error::context::retained_io(kind, "read host identity environment", error)
    })?;
    if hostname.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "host name unavailable",
        ));
    }
    Ok(hostname)
}

#[cfg(any(unix, test))]
fn parse_hostname(bytes: &[u8]) -> io::Result<&str> {
    let hostname = std::str::from_utf8(bytes).map_err(|error| {
        crate::error::context::retained_io(
            io::ErrorKind::InvalidData,
            "decode host identity name",
            error,
        )
    })?;
    if hostname.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "host name is empty",
        ));
    }
    Ok(hostname)
}

#[cfg(any(target_os = "macos", test))]
fn parse_remotectl_local_uuid(output: &str) -> Option<Uuid> {
    let mut lines = output.lines();
    while let Some(line) = lines.next() {
        if line != "Local device" {
            continue;
        }
        // Only the indented UUID directly under the local-device header is
        // eligible. In particular, never identify as a connected remote device.
        let Some(value) = lines.next() else {
            break;
        };
        if !value.starts_with(char::is_whitespace) {
            continue;
        }
        let Some(value) = value.trim().strip_prefix("UUID: ") else {
            continue;
        };
        if value.len() == 36 {
            if let Ok(identity) = Uuid::parse_str(value) {
                return Some(identity);
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
async fn host_remoted_uuid() -> Option<Uuid> {
    let mut command = tokio::process::Command::new("/usr/libexec/remotectl");
    command.arg("dumpstate");
    let output = bounded_command_output(&mut command, std::time::Duration::from_secs(10)).await;
    match output {
        Ok(output) => parse_remotectl_local_uuid(std::str::from_utf8(&output).ok()?),
        Err(_) => {
            tracing::debug!("RSD local remoted identity unavailable; using stable host identity");
            None
        }
    }
}

#[cfg(any(target_os = "macos", test))]
async fn bounded_command_output(
    command: &mut tokio::process::Command,
    timeout: std::time::Duration,
) -> io::Result<Vec<u8>> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;

    const OUTPUT_LIMIT: u64 = 1024 * 1024;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child.stdout.take().expect("piped command stdout");
    let result = tokio::time::timeout(timeout, async {
        let mut output = Vec::new();
        stdout
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut output)
            .await?;
        if output.len() as u64 > OUTPUT_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "remoted identity output exceeds limit",
            ));
        }
        if !child.wait().await?.success() {
            return Err(io::Error::other("remoted identity command failed"));
        }
        Ok(output)
    })
    .await
    .map_err(|error| {
        crate::error::context::retained_io(
            io::ErrorKind::TimedOut,
            "remoted identity query timed out",
            error,
        )
    })?;
    // kill_on_drop also handles cancellation of this entire future. Explicit
    // cleanup on output errors releases an overproducing child promptly.
    if result.is_err() {
        let _ = child.start_kill();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_hostname_retains_native_errors_and_preserves_empty_and_success() {
        use std::error::Error;
        let key = format!("IOS_A02_OWNED_ABSENT_{}", Uuid::new_v4());
        let error = parse_environment_hostname(std::env::var(key)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(matches!(
            error.source().unwrap().downcast_ref::<std::env::VarError>(),
            Some(std::env::VarError::NotPresent)
        ));
        #[cfg(unix)]
        let invalid = {
            use std::os::unix::ffi::OsStringExt;
            std::ffi::OsString::from_vec(b"owned-private\xff".to_vec())
        };
        #[cfg(windows)]
        let invalid = {
            use std::os::windows::ffi::OsStringExt;
            let units: Vec<_> = "owned-private".encode_utf16().chain([0xd800]).collect();
            std::ffi::OsString::from_wide(&units)
        };
        #[cfg(not(any(unix, windows)))]
        let invalid = std::ffi::OsString::from("owned-private");
        let error =
            parse_environment_hostname(Err(std::env::VarError::NotUnicode(invalid.clone())))
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        assert!(matches!(
            error.source().unwrap().downcast_ref::<std::env::VarError>(),
            Some(std::env::VarError::NotUnicode(value)) if value == &invalid
        ));
        assert!(!format!("{:?}", error.get_ref()).contains("owned-private"));
        assert_eq!(
            parse_environment_hostname(Ok(String::new()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            parse_environment_hostname(Ok("owned-host".into())).unwrap(),
            "owned-host"
        );
    }

    #[test]
    fn hostname_parser_retains_actual_utf8_source_and_existing_admission() {
        use std::error::Error;
        let error = parse_hostname(b"owned-private\xff").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let native = error
            .source()
            .unwrap()
            .downcast_ref::<std::str::Utf8Error>()
            .unwrap();
        assert_eq!(native.valid_up_to(), 13);
        assert_eq!(native.error_len(), Some(1));
        assert!(!format!("{:?}", error.get_ref()).contains("owned-private"));
        assert_eq!(
            parse_hostname(b"").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(parse_hostname(b"owned-host").unwrap(), "owned-host");
    }

    #[test]
    fn stable_identity_matches_dns_uuid3_vectors() {
        assert_eq!(
            hostname_uuid("example-host").to_string(),
            "49800e74-c74b-378c-958f-d88cb0dfdfd0"
        );
        assert_eq!(hostname_uuid("example-host"), hostname_uuid("example-host"));
        assert_ne!(hostname_uuid("example-host"), hostname_uuid("another-host"));
    }

    #[test]
    fn remoted_parser_uses_only_local_identity() {
        const LOCAL: &str = "CDAEC34E-898B-4D74-BA1A-718E98D83C40";
        let output = format!(
            "Remote device\n    UUID: 11bb8663-1d68-4c09-aacc-34bc4ea76cdd\nLocal device\n\tUUID: {LOCAL}\n\tName: example\n"
        );
        assert_eq!(
            parse_remotectl_local_uuid(&output),
            Uuid::parse_str(LOCAL).ok()
        );
        assert_eq!(
            parse_remotectl_local_uuid(&output.replace('\n', "\r\n")),
            Uuid::parse_str(LOCAL).ok()
        );
        assert_eq!(
            parse_remotectl_local_uuid(&format!("Local device\n    UUID: malformed\n{output}")),
            Uuid::parse_str(LOCAL).ok()
        );
        for invalid in [
            "Remote device\n    UUID: CDAEC34E-898B-4D74-BA1A-718E98D83C40",
            "Local device\nRemote device\n    UUID: CDAEC34E-898B-4D74-BA1A-718E98D83C40",
            "Local device\n    UUID: definitely not an identity",
            "Local device\nUUID: CDAEC34E-898B-4D74-BA1A-718E98D83C40",
            "Local device\n    UUID: CDAEC34E898B4D74BA1A718E98D83C40",
        ] {
            assert_eq!(parse_remotectl_local_uuid(invalid), None);
        }
    }

    #[tokio::test]
    async fn resolved_identity_remains_stable_across_calls() {
        let (first, second) = tokio::join!(default_handshake_uuid(), default_handshake_uuid());
        assert_eq!(first.unwrap(), second.unwrap());
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            default_handshake_uuid().await.unwrap(),
            hostname_uuid(&current_hostname().unwrap())
        );
    }

    #[tokio::test]
    async fn identity_child() {
        if let Ok(expected) = std::env::var("IOS_RSD_TEST_EXPECTED_IDENTITY") {
            assert_eq!(
                default_handshake_uuid().await.unwrap(),
                Uuid::parse_str(&expected).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn resolved_identity_matches_independent_processes() {
        let identity = default_handshake_uuid().await.unwrap();
        for _ in 0..2 {
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "xpc::identity::tests::identity_child"])
                .env("IOS_RSD_TEST_EXPECTED_IDENTITY", identity.to_string());
            // The child re-resolves the host identity with a fresh OnceCell.
            // Only the assertion status is used; neither identity is printed.
            bounded_command_output(&mut command, std::time::Duration::from_secs(15))
                .await
                .unwrap();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn identity_command_is_bounded_and_checks_exit_status() {
        use std::time::Duration;

        let mut stalled = tokio::process::Command::new("/bin/sh");
        stalled.args(["-c", "while :; do :; done"]);
        let error = bounded_command_output(&mut stalled, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(std::error::Error::source(&error)
            .unwrap()
            .is::<tokio::time::error::Elapsed>());

        let mut failed = tokio::process::Command::new("/bin/sh");
        failed.args(["-c", "exit 7"]);
        assert!(bounded_command_output(&mut failed, Duration::from_secs(2))
            .await
            .is_err());

        let mut noisy = tokio::process::Command::new("/bin/sh");
        noisy.args(["-c", "exec head -c 1048577 /dev/zero"]);
        let error = bounded_command_output(&mut noisy, Duration::from_secs(2))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
