//! InstallationProxy – app listing, install, uninstall.
//!
//! Service: `com.apple.mobile.installation_proxy`
//! Protocol: plist-framed (same 4-byte BE length prefix as lockdown).
//!
//! Reference: go-ios/ios/installationproxy/installationproxy.go

use std::collections::HashMap;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.mobile.installation_proxy";

#[derive(Debug, thiserror::Error)]
pub enum IpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(#[from] plist::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("install error: {0}")]
    Install(String),
}

/// Summary of an installed app.
#[derive(Debug, Clone)]
pub struct AppInfo {
    pub bundle_id: String,
    pub display_name: String,
    pub version: String,
    pub app_type: String,
    pub path: String,
    pub extra: HashMap<String, plist::Value>,
}

/// InstallationProxy client.
pub struct InstallationProxy<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> InstallationProxy<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Install an app from a staged IPA path on the device.
    pub async fn install(&mut self, package_path: &str) -> Result<(), IpError> {
        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command": "Install",
                "PackagePath": package_path,
                "ClientOptions": {},
            }),
        )
        .await?;

        self.wait_for_completion().await
    }

    /// Upgrade an app from a staged IPA path on the device.
    pub async fn upgrade(&mut self, package_path: &str) -> Result<(), IpError> {
        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command": "Upgrade",
                "PackagePath": package_path,
                "ClientOptions": {},
            }),
        )
        .await?;

        self.wait_for_completion().await
    }

    /// List all user-installed apps.
    pub async fn list_user_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        self.browse("User", true, &[]).await
    }

    /// List user-installed apps with a narrowed set of returned attributes.
    pub async fn list_user_apps_with_attributes(
        &mut self,
        return_attributes: &[&str],
    ) -> Result<Vec<AppInfo>, IpError> {
        self.browse("User", true, return_attributes).await
    }

    /// List all apps (user + system).
    pub async fn list_all_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        self.browse("", true, &[]).await
    }

    /// List only system apps.
    pub async fn list_system_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        self.browse("System", false, &[]).await
    }

    /// List only hidden apps.
    pub async fn list_hidden_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        self.browse("Hidden", true, &[]).await
    }

    /// List apps that expose iTunes/File Sharing.
    pub async fn list_file_sharing_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        let apps = self.list_all_apps().await?;
        Ok(apps
            .into_iter()
            .filter(|app| {
                app.extra
                    .get("UIFileSharingEnabled")
                    .and_then(plist::Value::as_boolean)
                    .unwrap_or(false)
            })
            .collect())
    }

    /// Uninstall an app by bundle ID.
    pub async fn uninstall(&mut self, bundle_id: &str) -> Result<(), IpError> {
        self.send_bundle_identifier_command("Uninstall", bundle_id)
            .await
    }

    /// Archive an app by bundle ID.
    pub async fn archive(&mut self, bundle_id: &str) -> Result<(), IpError> {
        self.send_bundle_identifier_command("Archive", bundle_id)
            .await
    }

    /// Restore an archived app by bundle ID.
    pub async fn restore(&mut self, bundle_id: &str) -> Result<(), IpError> {
        self.send_bundle_identifier_command("Restore", bundle_id)
            .await
    }

    async fn send_bundle_identifier_command(
        &mut self,
        command: &str,
        bundle_id: &str,
    ) -> Result<(), IpError> {
        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command":               command,
                "ApplicationIdentifier": bundle_id,
                "ClientOptions":         {},
            }),
        )
        .await?;

        self.wait_for_completion().await
    }

    /// Look up a single app by bundle ID.
    pub async fn lookup_app(&mut self, bundle_id: &str) -> Result<Option<AppInfo>, IpError> {
        self.lookup_app_with_attributes(bundle_id, &[]).await
    }

    /// Look up a single app by bundle ID with optional return-attribute filtering.
    pub async fn lookup_app_with_attributes(
        &mut self,
        bundle_id: &str,
        return_attributes: &[&str],
    ) -> Result<Option<AppInfo>, IpError> {
        let mut options = serde_json::json!({
            "BundleIDs": [bundle_id],
        });
        if !return_attributes.is_empty() {
            options["ReturnAttributes"] = serde_json::Value::Array(
                return_attributes
                    .iter()
                    .map(|attr| serde_json::Value::String((*attr).to_string()))
                    .collect(),
            );
        }

        let response = self.lookup(options).await?;

        Ok(response
            .into_iter()
            .next()
            .map(|(lookup_bundle_id, value)| {
                parse_app_info_with_bundle_id(&lookup_bundle_id, value)
            }))
    }

    async fn browse(
        &mut self,
        app_type: &str,
        show_prohibited: bool,
        return_attributes: &[&str],
    ) -> Result<Vec<AppInfo>, IpError> {
        let mut client_opts = serde_json::json!({});
        if !app_type.is_empty() {
            client_opts["ApplicationType"] = serde_json::Value::String(app_type.to_string());
        }
        if show_prohibited {
            client_opts["ShowLaunchProhibitedApps"] = serde_json::Value::Bool(true);
        }
        if !return_attributes.is_empty() {
            client_opts["ReturnAttributes"] = serde_json::Value::Array(
                return_attributes
                    .iter()
                    .map(|attr| serde_json::Value::String((*attr).to_string()))
                    .collect(),
            );
        }

        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command":       "Browse",
                "ClientOptions": client_opts,
            }),
        )
        .await?;

        let mut apps = Vec::new();
        loop {
            let data = recv_plist_raw(&mut self.stream).await?;
            let resp: plist::Dictionary = plist::from_bytes(&data)?;

            for item in resp
                .get("CurrentList")
                .and_then(plist::Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                apps.push(parse_app_info(item));
            }

            if resp.get("Status").and_then(plist::Value::as_string) == Some("Complete") {
                break;
            }
        }
        Ok(apps)
    }

    async fn lookup(
        &mut self,
        client_options: serde_json::Value,
    ) -> Result<HashMap<String, plist::Value>, IpError> {
        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command": "Lookup",
                "ClientOptions": client_options,
            }),
        )
        .await?;

        let data = recv_plist_raw(&mut self.stream).await?;
        let mut dict: HashMap<String, plist::Value> = plist::from_bytes(&data)?;
        if let Some(e) = dict.get("Error") {
            return Err(IpError::Install(format!("{e:?}")));
        }

        let result = dict
            .remove("LookupResult")
            .and_then(|value| value.into_dictionary())
            .map(|items| items.into_iter().collect())
            .unwrap_or_default();
        Ok(result)
    }

    async fn wait_for_completion(&mut self) -> Result<(), IpError> {
        loop {
            let data = recv_plist_raw(&mut self.stream).await?;
            let dict: HashMap<String, plist::Value> = plist::from_bytes(&data)?;
            if let Some(error) = dict.get("Error") {
                let message = match dict.get("ErrorDescription").and_then(|v| v.as_string()) {
                    Some(description) => format!("{error:?}: {description}"),
                    None => format!("{error:?}"),
                };
                return Err(IpError::Install(message));
            }
            if dict.get("Status").and_then(|s| s.as_string()) == Some("Complete") {
                return Ok(());
            }
        }
    }
}

fn parse_app_info(val: plist::Value) -> AppInfo {
    parse_app_info_with_bundle_id("", val)
}

fn parse_app_info_with_bundle_id(lookup_bundle_id: &str, val: plist::Value) -> AppInfo {
    let dict = val.into_dictionary().unwrap_or_default();
    let get_str = |k: &str| {
        dict.get(k)
            .and_then(|v| v.as_string())
            .unwrap_or("")
            .to_string()
    };

    let bundle_id = if lookup_bundle_id.is_empty() {
        get_str("CFBundleIdentifier")
    } else {
        dict.get("CFBundleIdentifier")
            .and_then(|v| v.as_string())
            .unwrap_or(lookup_bundle_id)
            .to_string()
    };
    let display_name = get_str("CFBundleDisplayName");
    let version = get_str("CFBundleShortVersionString");
    let app_type = get_str("ApplicationType");
    let path = get_str("Path");

    let extra = dict.into_iter().collect();

    AppInfo {
        bundle_id,
        display_name,
        version,
        app_type,
        path,
        extra,
    }
}

// ── plist framing ──────────────────────────────────────────────────────────────

async fn send_plist<S>(stream: &mut S, value: &serde_json::Value) -> Result<(), IpError>
where
    S: AsyncWrite + Unpin,
{
    // Convert JSON value to plist
    let plist_val = json_to_plist(value);
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &plist_val)?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist_raw<S>(stream: &mut S) -> Result<Vec<u8>, IpError>
where
    S: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(IpError::Protocol(format!(
            "plist length {len} exceeds maximum of {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

fn json_to_plist(val: &serde_json::Value) -> plist::Value {
    match val {
        serde_json::Value::Null => plist::Value::String(String::new()),
        serde_json::Value::Bool(b) => plist::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                plist::Value::Integer(plist::Integer::from(i))
            } else {
                plist::Value::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => plist::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            plist::Value::Array(arr.iter().map(json_to_plist).collect())
        }
        serde_json::Value::Object(map) => {
            let dict: plist::Dictionary = map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_plist(v)))
                .collect();
            plist::Value::Dictionary(dict)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::*;

    #[derive(Default)]
    struct RecordingStream {
        written: Vec<u8>,
    }

    impl AsyncRead for RecordingStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "test stream has no responses",
            )))
        }
    }

    impl AsyncWrite for RecordingStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn list_system_apps_sends_system_filter() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.list_system_apps().await.unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        assert_eq!(
            client_options["ApplicationType"].as_string(),
            Some("System")
        );
        assert!(!client_options.contains_key("ShowLaunchProhibitedApps"));
    }

    #[tokio::test]
    async fn list_hidden_apps_sends_hidden_filter() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.list_hidden_apps().await.unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        assert_eq!(
            client_options["ApplicationType"].as_string(),
            Some("Hidden")
        );
        assert_eq!(
            client_options["ShowLaunchProhibitedApps"].as_boolean(),
            Some(true)
        );
    }

    #[tokio::test]
    async fn list_file_sharing_apps_filters_on_ui_file_sharing_enabled() {
        let responses = vec![
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "Status".to_string(),
                    plist::Value::String("BrowsingApplications".into()),
                ),
                (
                    "CurrentList".to_string(),
                    plist::Value::Array(vec![
                        plist::Value::Dictionary(plist::Dictionary::from_iter([
                            (
                                "CFBundleIdentifier".to_string(),
                                plist::Value::String("com.example.Files".into()),
                            ),
                            (
                                "CFBundleDisplayName".to_string(),
                                plist::Value::String("Files".into()),
                            ),
                            (
                                "UIFileSharingEnabled".to_string(),
                                plist::Value::Boolean(true),
                            ),
                        ])),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([
                            (
                                "CFBundleIdentifier".to_string(),
                                plist::Value::String("com.example.Hidden".into()),
                            ),
                            (
                                "CFBundleDisplayName".to_string(),
                                plist::Value::String("Hidden".into()),
                            ),
                            (
                                "UIFileSharingEnabled".to_string(),
                                plist::Value::Boolean(false),
                            ),
                        ])),
                    ]),
                ),
            ]))),
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]))),
        ];
        let mut stream = ResponseStream::with_frames(responses);
        let mut proxy = InstallationProxy::new(&mut stream);

        let apps = proxy.list_file_sharing_apps().await.unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].bundle_id, "com.example.Files");
    }

    #[tokio::test]
    async fn lookup_app_sends_lookup_command_with_bundle_ids() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.lookup_app("com.example.test").await.unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Lookup"));
        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        let bundle_ids = client_options["BundleIDs"].as_array().unwrap();
        assert_eq!(bundle_ids.len(), 1);
        assert_eq!(bundle_ids[0].as_string(), Some("com.example.test"));
    }

    #[tokio::test]
    async fn lookup_app_with_attributes_sends_return_attributes() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy
                .lookup_app_with_attributes("com.example.test", &["CFBundleVersion", "Path"])
                .await
                .unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        let attrs = client_options["ReturnAttributes"].as_array().unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].as_string(), Some("CFBundleVersion"));
        assert_eq!(attrs[1].as_string(), Some("Path"));
    }

    #[tokio::test]
    async fn list_user_apps_with_attributes_sends_return_attributes() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy
                .list_user_apps_with_attributes(&[
                    "CFBundleIdentifier",
                    "ApplicationSINF",
                    "iTunesMetadata",
                ])
                .await
                .unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Browse"));

        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        assert_eq!(client_options["ApplicationType"].as_string(), Some("User"));
        assert_eq!(
            client_options["ShowLaunchProhibitedApps"].as_boolean(),
            Some(true)
        );
        let attrs = client_options["ReturnAttributes"].as_array().unwrap();
        assert_eq!(attrs.len(), 3);
        assert_eq!(attrs[0].as_string(), Some("CFBundleIdentifier"));
        assert_eq!(attrs[1].as_string(), Some("ApplicationSINF"));
        assert_eq!(attrs[2].as_string(), Some("iTunesMetadata"));
    }

    #[tokio::test]
    async fn install_sends_package_path_and_waits_for_completion() {
        let responses = vec![
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Installing".into()),
            )]))),
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]))),
        ];
        let mut stream = ResponseStream::with_frames(responses);
        {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.install("/PublicStaging/Example.ipa").await.unwrap();
        }

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Install"));
        assert_eq!(
            dict["PackagePath"].as_string(),
            Some("/PublicStaging/Example.ipa")
        );
        assert_eq!(
            dict["ClientOptions"].as_dictionary(),
            Some(&plist::Dictionary::new())
        );
    }

    #[tokio::test]
    async fn upgrade_sends_upgrade_command() {
        let responses = vec![
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Upgrading".into()),
            )]))),
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]))),
        ];
        let mut stream = ResponseStream::with_frames(responses);
        {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.upgrade("/PublicStaging/Example.ipa").await.unwrap();
        }

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Upgrade"));
    }

    #[tokio::test]
    async fn archive_sends_application_identifier() {
        let responses = vec![plist_frame(plist::Value::Dictionary(
            plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]),
        ))];
        let mut stream = ResponseStream::with_frames(responses);
        {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.archive("com.example.test").await.unwrap();
        }

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Archive"));
        assert_eq!(
            dict["ApplicationIdentifier"].as_string(),
            Some("com.example.test")
        );
    }

    #[tokio::test]
    async fn restore_sends_application_identifier() {
        let responses = vec![plist_frame(plist::Value::Dictionary(
            plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]),
        ))];
        let mut stream = ResponseStream::with_frames(responses);
        {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.restore("com.example.test").await.unwrap();
        }

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Restore"));
        assert_eq!(
            dict["ApplicationIdentifier"].as_string(),
            Some("com.example.test")
        );
    }

    fn plist_frame(value: plist::Value) -> Vec<u8> {
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, &value).unwrap();
        let mut framed = Vec::with_capacity(buf.len() + 4);
        framed.extend_from_slice(&(buf.len() as u32).to_be_bytes());
        framed.extend_from_slice(&buf);
        framed
    }

    struct ResponseStream {
        written: Vec<u8>,
        read_buf: Vec<u8>,
        read_pos: usize,
    }

    impl ResponseStream {
        fn with_frames(frames: Vec<Vec<u8>>) -> Self {
            let read_buf = frames.into_iter().flatten().collect();
            Self {
                written: Vec::new(),
                read_buf,
                read_pos: 0,
            }
        }
    }

    impl AsyncRead for ResponseStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let remaining = self.read_buf.len().saturating_sub(self.read_pos);
            if remaining == 0 {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "no more test data",
                )));
            }

            let to_copy = remaining.min(buf.remaining());
            let start = self.read_pos;
            let end = start + to_copy;
            buf.put_slice(&self.read_buf[start..end]);
            self.read_pos = end;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for ResponseStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}
