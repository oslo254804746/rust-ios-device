//! Minimal SpringBoard services client.
//!
//! Current scope: fetch the Home Screen icon layout via
//! `com.apple.springboardservices` and present it as typed pages/items.

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

mod lifecycle;
#[cfg(test)]
mod lifecycle_tests;

use lifecycle::{await_operation, check_deadline, SpringboardState};

pub const SERVICE_NAME: &str = "com.apple.springboardservices";

service_error!(
    SpringboardError,
    #[error("service error: {0}")]
    Service(String),
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    App(AppIcon),
    Folder(Folder),
    WebClip(WebClip),
    Custom(CustomIcon),
    /// An icon entry that doesn't match any known pattern (e.g. widgets,
    /// suggested apps, or other iOS 17+ icon types).
    Unknown(UnknownIcon),
}

impl Icon {
    pub fn display_name(&self) -> &str {
        match self {
            Icon::App(app) => &app.display_name,
            Icon::Folder(folder) => &folder.display_name,
            Icon::WebClip(web_clip) => &web_clip.display_name,
            Icon::Custom(_) => "",
            Icon::Unknown(unknown) => unknown.display_name.as_deref().unwrap_or(""),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIcon {
    pub display_name: String,
    pub display_identifier: Option<String>,
    pub bundle_identifier: String,
    pub bundle_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub display_name: String,
    pub pages: Vec<Vec<Icon>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebClip {
    pub display_name: String,
    pub display_identifier: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomIcon {
    pub icon_type: Option<String>,
}

/// An icon entry that doesn't match any known pattern. Preserves whatever
/// fields the device sent so callers can inspect them if needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownIcon {
    pub display_name: Option<String>,
    pub display_identifier: Option<String>,
    pub icon_type: Option<String>,
    pub list_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceOrientation {
    Portrait,
    PortraitUpsideDown,
    Landscape,
    LandscapeHomeToLeft,
    Unknown(u64),
}

impl InterfaceOrientation {
    pub fn from_raw(value: u64) -> Self {
        match value {
            1 => Self::Portrait,
            2 => Self::PortraitUpsideDown,
            3 => Self::Landscape,
            4 => Self::LandscapeHomeToLeft,
            other => Self::Unknown(other),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::PortraitUpsideDown => "portrait_upside_down",
            Self::Landscape => "landscape",
            Self::LandscapeHomeToLeft => "landscape_home_to_left",
            Self::Unknown(_) => "unknown",
        }
    }

    pub fn raw_value(&self) -> u64 {
        match self {
            Self::Portrait => 1,
            Self::PortraitUpsideDown => 2,
            Self::Landscape => 3,
            Self::LandscapeHomeToLeft => 4,
            Self::Unknown(value) => *value,
        }
    }
}

#[derive(Debug)]
pub struct SpringboardClient<S> {
    stream: Option<S>,
    state: SpringboardState,
}

impl<S: AsyncRead + AsyncWrite + Unpin> SpringboardClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream: Some(stream),
            state: SpringboardState::default(),
        }
    }

    pub async fn list_icons(&mut self) -> Result<Vec<Vec<Icon>>, SpringboardError> {
        self.request(
            &GetIconStateRequest {
                command: "getIconState",
                format_version: "2",
            },
            |response| parse_screens(response?),
        )
        .await
    }

    pub async fn get_icon_state_raw(
        &mut self,
        format_version: &str,
    ) -> Result<plist::Value, SpringboardError> {
        self.request(
            &GetIconStateRequest {
                command: "getIconState",
                format_version,
            },
            |response| response,
        )
        .await
    }

    pub async fn get_icon_png_data(
        &mut self,
        bundle_id: &str,
    ) -> Result<Vec<u8>, SpringboardError> {
        self.request(
            &GetIconPngDataRequest {
                command: "getIconPNGData",
                bundle_id,
            },
            |response| parse_png_data(response?),
        )
        .await
    }

    pub async fn get_interface_orientation(
        &mut self,
    ) -> Result<InterfaceOrientation, SpringboardError> {
        self.request(
            &CommandRequest {
                command: "getInterfaceOrientation",
            },
            |response| parse_interface_orientation(response?),
        )
        .await
    }

    pub async fn get_homescreen_icon_metrics(&mut self) -> Result<plist::Value, SpringboardError> {
        self.request(
            &CommandRequest {
                command: "getHomeScreenIconMetrics",
            },
            |response| parse_metrics(response?),
        )
        .await
    }

    pub async fn get_wallpaper_info(
        &mut self,
        wallpaper_name: &str,
    ) -> Result<plist::Value, SpringboardError> {
        self.request(
            &WallpaperCommandRequest {
                command: "getWallpaperInfo",
                wallpaper_name,
            },
            |response| parse_metrics(response?),
        )
        .await
    }

    pub async fn get_wallpaper_preview_image(
        &mut self,
        wallpaper_name: &str,
    ) -> Result<Vec<u8>, SpringboardError> {
        self.request(
            &WallpaperCommandRequest {
                command: "getWallpaperPreviewImage",
                wallpaper_name,
            },
            |response| parse_png_data(response?),
        )
        .await
    }

    pub async fn set_icon_state(
        &mut self,
        icon_state: &plist::Value,
    ) -> Result<(), SpringboardError> {
        self.request(
            &SetIconStateRequest {
                command: "setIconState",
                icon_state,
            },
            |response| {
                // Preserve the legacy setIconState EOF result. The tracked
                // receive still retires the closed stream before returning.
                match response {
                    Ok(_) => Ok(()),
                    Err(SpringboardError::Io(e))
                        if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                    {
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            },
        )
        .await
    }

    pub async fn get_homescreen_wallpaper_pngdata(&mut self) -> Result<Vec<u8>, SpringboardError> {
        match self
            .request(
                &CommandRequest {
                    command: "getHomeScreenWallpaperPNGData",
                },
                |response| parse_homescreen_wallpaper_png_data(response?),
            )
            .await?
        {
            HomescreenWallpaperPngData::Png(png) => Ok(png),
            HomescreenWallpaperPngData::NoStoredImage => {
                match self.get_wallpaper_preview_image("homescreen").await {
                    Ok(png) => Ok(png),
                    Err(_) => Err(SpringboardError::Service(
                        "This wallpaper has no image".into(),
                    )),
                }
            }
        }
    }

    #[allow(dead_code)]
    async fn get_homescreen_wallpaper_pngdata_strict(
        &mut self,
    ) -> Result<Vec<u8>, SpringboardError> {
        self.request(
            &CommandRequest {
                command: "getHomeScreenWallpaperPNGData",
            },
            |response| parse_png_data(response?),
        )
        .await
    }

    async fn request<T: Serialize, R>(
        &mut self,
        value: &T,
        parse_response: impl FnOnce(
            Result<plist::Value, SpringboardError>,
        ) -> Result<R, SpringboardError>,
    ) -> Result<R, SpringboardError> {
        let deadline = self.deadline()?;
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, value)?;
        let len = u32::try_from(buf.len()).map_err(|error| {
            SpringboardError::Io(crate::error::context::retained_io(
                std::io::ErrorKind::InvalidInput,
                "encode springboard frame length",
                error,
            ))
        })?;
        check_deadline(deadline, Ok(()))?;
        let mut request = self.begin_request(deadline)?;
        let result = await_operation(deadline, async {
            let mut io = request.io();
            io.write_all(&len.to_be_bytes()).await?;
            io.write_all(&buf).await?;
            io.flush().await?;
            parse_response(recv_plist(&mut io).await)
        })
        .await;
        request.finish(check_deadline(deadline, result))
    }
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Value, SpringboardError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 16 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(SpringboardError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(plist::from_bytes(&buf)?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetIconStateRequest<'a> {
    command: &'static str,
    format_version: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetIconPngDataRequest<'a> {
    command: &'static str,
    bundle_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WallpaperCommandRequest<'a> {
    command: &'static str,
    wallpaper_name: &'a str,
}

#[derive(Serialize)]
struct CommandRequest {
    command: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetIconStateRequest<'a> {
    command: &'static str,
    icon_state: &'a plist::Value,
}

fn parse_screens(value: plist::Value) -> Result<Vec<Vec<Icon>>, SpringboardError> {
    let screens = value.into_array().ok_or_else(|| {
        SpringboardError::Protocol("springboard response was not an array".into())
    })?;

    screens
        .into_iter()
        .map(|screen| {
            let icons = screen.into_array().ok_or_else(|| {
                SpringboardError::Protocol("screen entry was not an array".into())
            })?;
            icons.into_iter().map(|icon| parse_icon(icon, 0)).collect()
        })
        .collect()
}

enum HomescreenWallpaperPngData {
    Png(Vec<u8>),
    NoStoredImage,
}

pub fn is_wallpaper_image_unavailable_error(message: &str) -> bool {
    message.contains("This wallpaper has no image")
}

fn parse_homescreen_wallpaper_png_data(
    value: plist::Value,
) -> Result<HomescreenWallpaperPngData, SpringboardError> {
    match parse_png_data(value) {
        Ok(png) => Ok(HomescreenWallpaperPngData::Png(png)),
        Err(SpringboardError::Service(error)) if is_wallpaper_image_unavailable_error(&error) => {
            Ok(HomescreenWallpaperPngData::NoStoredImage)
        }
        Err(error) => Err(error),
    }
}

fn parse_png_data(value: plist::Value) -> Result<Vec<u8>, SpringboardError> {
    let dict = value.into_dictionary().ok_or_else(|| {
        SpringboardError::Protocol("springboard icon response was not a dictionary".into())
    })?;

    if let Some(error) = string_field(&dict, "Error") {
        return Err(SpringboardError::Service(error));
    }

    dict.get("pngData")
        .and_then(plist::Value::as_data)
        .map(|data| data.to_vec())
        .ok_or_else(|| SpringboardError::Protocol("springboard response missing pngData".into()))
}

fn parse_interface_orientation(
    value: plist::Value,
) -> Result<InterfaceOrientation, SpringboardError> {
    let dict = value.into_dictionary().ok_or_else(|| {
        SpringboardError::Protocol("springboard orientation response was not a dictionary".into())
    })?;

    let raw = dict
        .get("interfaceOrientation")
        .and_then(plist_integer_to_u64)
        .ok_or_else(|| {
            SpringboardError::Protocol("springboard response missing interfaceOrientation".into())
        })?;
    Ok(InterfaceOrientation::from_raw(raw))
}

fn parse_metrics(value: plist::Value) -> Result<plist::Value, SpringboardError> {
    let dict = value.into_dictionary().ok_or_else(|| {
        SpringboardError::Protocol("springboard metrics response was not a dictionary".into())
    })?;
    Ok(plist::Value::Dictionary(dict))
}

/// Maximum folder nesting followed when parsing an icon list.
///
/// Folders nest through `iconLists`, and the depth comes from the device.
const MAX_ICON_DEPTH: usize = 16;

fn parse_icon(value: plist::Value, depth: usize) -> Result<Icon, SpringboardError> {
    if depth > MAX_ICON_DEPTH {
        return Err(SpringboardError::Protocol(format!(
            "springboard icon folders nested deeper than {MAX_ICON_DEPTH} levels"
        )));
    }

    let dict = value
        .into_dictionary()
        .ok_or_else(|| SpringboardError::Protocol("icon entry was not a dictionary".into()))?;

    if let Some(bundle_identifier) = string_field(&dict, "bundleIdentifier") {
        return Ok(Icon::App(AppIcon {
            display_name: string_field(&dict, "displayName").unwrap_or_default(),
            display_identifier: string_field(&dict, "displayIdentifier"),
            bundle_identifier,
            bundle_version: string_field(&dict, "bundleVersion"),
        }));
    }

    if let Some(url) = string_field(&dict, "webClipURL") {
        return Ok(Icon::WebClip(WebClip {
            display_name: string_field(&dict, "displayName").unwrap_or_default(),
            display_identifier: string_field(&dict, "displayIdentifier"),
            url,
        }));
    }

    if string_field(&dict, "listType").as_deref() == Some("folder") {
        let pages = dict
            .get("iconLists")
            .and_then(plist::Value::as_array)
            .ok_or_else(|| SpringboardError::Protocol("folder iconLists missing".into()))?;
        let pages = pages
            .iter()
            .map(|page| {
                let page_icons = page.as_array().ok_or_else(|| {
                    SpringboardError::Protocol("folder page was not an array".into())
                })?;
                page_icons
                    .iter()
                    .cloned()
                    .map(|icon| parse_icon(icon, depth + 1))
                    .collect()
            })
            .collect::<Result<Vec<Vec<Icon>>, SpringboardError>>()?;
        return Ok(Icon::Folder(Folder {
            display_name: string_field(&dict, "displayName").unwrap_or_default(),
            pages,
        }));
    }

    if string_field(&dict, "iconType").as_deref() == Some("custom") {
        return Ok(Icon::Custom(CustomIcon {
            icon_type: string_field(&dict, "iconType"),
        }));
    }

    // Unrecognized entry — could be a widget, suggested app, or other iOS 17+
    // icon type. Return it as Unknown rather than failing the entire parse.
    Ok(Icon::Unknown(UnknownIcon {
        display_name: string_field(&dict, "displayName"),
        display_identifier: string_field(&dict, "displayIdentifier"),
        icon_type: string_field(&dict, "iconType"),
        list_type: string_field(&dict, "listType"),
    }))
}

fn string_field(dict: &plist::Dictionary, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(plist::Value::as_string)
        .map(ToOwned::to_owned)
}

fn plist_integer_to_u64(value: &plist::Value) -> Option<u64> {
    match value {
        plist::Value::Integer(value) => value
            .as_unsigned()
            .or_else(|| value.as_signed().and_then(|signed| signed.try_into().ok())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn read_plist_frame<S>(stream: &mut S) -> Vec<u8>
    where
        S: AsyncRead + Unpin,
    {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await.unwrap();
        buf
    }

    #[test]
    fn test_service_name_matches_go_ios() {
        assert_eq!(SERVICE_NAME, "com.apple.springboardservices");
    }

    #[tokio::test]
    async fn list_icons_roundtrips_app_folder_and_custom_items() {
        let (client_side, mut server_side) = tokio::io::duplex(8192);

        tokio::spawn(async move {
            let request = read_plist_frame(&mut server_side).await;
            let req_value: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = req_value.into_dictionary().unwrap();
            assert_eq!(
                dict.get("command").and_then(|v| v.as_string()),
                Some("getIconState")
            );
            assert_eq!(
                dict.get("formatVersion").and_then(|v| v.as_string()),
                Some("2")
            );

            let response = plist::Value::Array(vec![plist::Value::Array(vec![
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        "displayName".to_string(),
                        plist::Value::String("Phone".into()),
                    ),
                    (
                        "displayIdentifier".to_string(),
                        plist::Value::String("com.apple.mobilephone".into()),
                    ),
                    (
                        "bundleIdentifier".to_string(),
                        plist::Value::String("com.apple.mobilephone".into()),
                    ),
                ])),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        "displayName".to_string(),
                        plist::Value::String("Utilities".into()),
                    ),
                    (
                        "listType".to_string(),
                        plist::Value::String("folder".into()),
                    ),
                    (
                        "iconLists".to_string(),
                        plist::Value::Array(vec![plist::Value::Array(vec![
                            plist::Value::Dictionary(plist::Dictionary::from_iter([
                                (
                                    "displayName".to_string(),
                                    plist::Value::String("Calculator".into()),
                                ),
                                (
                                    "bundleIdentifier".to_string(),
                                    plist::Value::String("com.apple.calculator".into()),
                                ),
                            ])),
                        ])]),
                    ),
                ])),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "iconType".to_string(),
                    plist::Value::String("custom".into()),
                )])),
            ])]);

            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &response).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();
        });

        let mut client = SpringboardClient::new(client_side);
        let screens = client.list_icons().await.unwrap();
        assert_eq!(screens.len(), 1);
        assert_eq!(screens[0].len(), 3);
        match &screens[0][0] {
            Icon::App(app) => {
                assert_eq!(app.display_name, "Phone");
                assert_eq!(app.bundle_identifier, "com.apple.mobilephone");
            }
            other => panic!("unexpected first icon: {other:?}"),
        }
        match &screens[0][1] {
            Icon::Folder(folder) => {
                assert_eq!(folder.display_name, "Utilities");
                assert_eq!(folder.pages.len(), 1);
                assert_eq!(folder.pages[0].len(), 1);
            }
            other => panic!("unexpected second icon: {other:?}"),
        }
        match &screens[0][2] {
            Icon::Custom(custom) => assert_eq!(custom.icon_type.as_deref(), Some("custom")),
            other => panic!("unexpected third icon: {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_icon_png_data_roundtrips_png_bytes() {
        let (client_side, mut server_side) = tokio::io::duplex(8192);

        tokio::spawn(async move {
            let request = read_plist_frame(&mut server_side).await;
            let req_value: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = req_value.into_dictionary().unwrap();
            assert_eq!(
                dict.get("command").and_then(|v| v.as_string()),
                Some("getIconPNGData")
            );
            assert_eq!(
                dict.get("bundleId").and_then(|v| v.as_string()),
                Some("com.apple.Preferences")
            );

            let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "pngData".to_string(),
                plist::Value::Data(vec![0x89, b'P', b'N', b'G']),
            )]));

            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &response).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();
        });

        let mut client = SpringboardClient::new(client_side);
        let png = client
            .get_icon_png_data("com.apple.Preferences")
            .await
            .unwrap();
        assert_eq!(png, vec![0x89, b'P', b'N', b'G']);
    }

    #[tokio::test]
    async fn get_interface_orientation_roundtrips_orientation_value() {
        let (client_side, mut server_side) = tokio::io::duplex(8192);

        tokio::spawn(async move {
            let request = read_plist_frame(&mut server_side).await;
            let req_value: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = req_value.into_dictionary().unwrap();
            assert_eq!(
                dict.get("command").and_then(|v| v.as_string()),
                Some("getInterfaceOrientation")
            );

            let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "interfaceOrientation".to_string(),
                plist::Value::Integer(3.into()),
            )]));

            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &response).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();
        });

        let mut client = SpringboardClient::new(client_side);
        let orientation = client.get_interface_orientation().await.unwrap();
        assert_eq!(orientation, InterfaceOrientation::Landscape);
    }

    #[tokio::test]
    async fn get_homescreen_icon_metrics_roundtrips_metric_dictionary() {
        let (client_side, mut server_side) = tokio::io::duplex(8192);

        tokio::spawn(async move {
            let request = read_plist_frame(&mut server_side).await;
            let req_value: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = req_value.into_dictionary().unwrap();
            assert_eq!(
                dict.get("command").and_then(|v| v.as_string()),
                Some("getHomeScreenIconMetrics")
            );

            let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("iconWidth".to_string(), plist::Value::Real(60.0)),
                ("iconHeight".to_string(), plist::Value::Integer(60.into())),
            ]));

            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &response).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();
        });

        let mut client = SpringboardClient::new(client_side);
        let metrics = client.get_homescreen_icon_metrics().await.unwrap();
        let dict = metrics.into_dictionary().unwrap();
        assert_eq!(dict["iconWidth"].as_real(), Some(60.0));
        assert_eq!(dict["iconHeight"].as_signed_integer(), Some(60));
    }

    #[tokio::test]
    async fn get_wallpaper_info_roundtrips_dictionary() {
        let (client_side, mut server_side) = tokio::io::duplex(8192);

        tokio::spawn(async move {
            let request = read_plist_frame(&mut server_side).await;
            let req_value: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = req_value.into_dictionary().unwrap();
            assert_eq!(
                dict.get("command").and_then(|v| v.as_string()),
                Some("getWallpaperInfo")
            );
            assert_eq!(
                dict.get("wallpaperName").and_then(|v| v.as_string()),
                Some("homescreen")
            );

            let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "wallpaperName".to_string(),
                    plist::Value::String("homescreen".into()),
                ),
                ("isDark".to_string(), plist::Value::Boolean(false)),
                (
                    "variation".to_string(),
                    plist::Value::String("default".into()),
                ),
            ]));

            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &response).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();
        });

        let mut client = SpringboardClient::new(client_side);
        let info = client.get_wallpaper_info("homescreen").await.unwrap();
        let dict = info.into_dictionary().unwrap();
        assert_eq!(dict["wallpaperName"].as_string(), Some("homescreen"));
        assert_eq!(dict["isDark"].as_boolean(), Some(false));
        assert_eq!(dict["variation"].as_string(), Some("default"));
    }

    #[tokio::test]
    async fn get_wallpaper_preview_image_roundtrips_png_bytes() {
        let (client_side, mut server_side) = tokio::io::duplex(8192);

        tokio::spawn(async move {
            let request = read_plist_frame(&mut server_side).await;
            let req_value: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = req_value.into_dictionary().unwrap();
            assert_eq!(
                dict.get("command").and_then(|v| v.as_string()),
                Some("getWallpaperPreviewImage")
            );
            assert_eq!(
                dict.get("wallpaperName").and_then(|v| v.as_string()),
                Some("lockscreen")
            );

            let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "pngData".to_string(),
                plist::Value::Data(vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a]),
            )]));

            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &response).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();
        });

        let mut client = SpringboardClient::new(client_side);
        let png = client
            .get_wallpaper_preview_image("lockscreen")
            .await
            .unwrap();
        assert_eq!(png, vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a]);
    }

    #[tokio::test]
    async fn get_icon_state_raw_roundtrips_unparsed_state() {
        let (client_side, mut server_side) = tokio::io::duplex(8192);

        tokio::spawn(async move {
            let request = read_plist_frame(&mut server_side).await;
            let req_value: plist::Value = plist::from_bytes(&request).unwrap();
            let dict = req_value.into_dictionary().unwrap();
            assert_eq!(
                dict.get("command").and_then(|v| v.as_string()),
                Some("getIconState")
            );
            assert_eq!(
                dict.get("formatVersion").and_then(|v| v.as_string()),
                Some("2")
            );

            let response = plist::Value::Array(vec![plist::Value::Dictionary(
                plist::Dictionary::from_iter([
                    (
                        "bundleIdentifier".to_string(),
                        plist::Value::String("com.apple.Preferences".into()),
                    ),
                    (
                        "unknownField".to_string(),
                        plist::Value::String("preserved".into()),
                    ),
                ]),
            )]);

            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &response).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();
        });

        let mut client = SpringboardClient::new(client_side);
        let state = client.get_icon_state_raw("2").await.unwrap();
        let entries = state.as_array().unwrap();
        let dict = entries[0].as_dictionary().unwrap();
        assert_eq!(
            dict["bundleIdentifier"].as_string(),
            Some("com.apple.Preferences")
        );
        assert_eq!(dict["unknownField"].as_string(), Some("preserved"));
    }

    #[test]
    fn parse_png_data_surfaces_service_error() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Error".to_string(),
            plist::Value::String("No such bundle".into()),
        )]));

        let err = parse_png_data(value).unwrap_err();
        assert!(matches!(err, SpringboardError::Service(message) if message == "No such bundle"));
    }

    #[tokio::test]
    async fn get_homescreen_wallpaper_pngdata_falls_back_to_preview_when_poster_wallpaper_has_no_image(
    ) {
        let (client_side, mut server_side) = tokio::io::duplex(8192);

        tokio::spawn(async move {
            let first_req = read_plist_frame(&mut server_side).await;
            let first_val: plist::Value = plist::from_bytes(&first_req).unwrap();
            let first_dict = first_val.into_dictionary().unwrap();
            assert_eq!(
                first_dict.get("command").and_then(|v| v.as_string()),
                Some("getHomeScreenWallpaperPNGData")
            );

            let first_resp = plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Error".to_string(),
                plist::Value::String("This wallpaper has no image".into()),
            )]));
            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &first_resp).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();

            let second_req = read_plist_frame(&mut server_side).await;
            let second_val: plist::Value = plist::from_bytes(&second_req).unwrap();
            let second_dict = second_val.into_dictionary().unwrap();
            assert_eq!(
                second_dict.get("command").and_then(|v| v.as_string()),
                Some("getWallpaperPreviewImage")
            );
            assert_eq!(
                second_dict.get("wallpaperName").and_then(|v| v.as_string()),
                Some("homescreen")
            );

            let second_resp = plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "pngData".to_string(),
                plist::Value::Data(vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
            )]));
            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &second_resp).unwrap();
            let len = buf.len() as u32;
            server_side.write_all(&len.to_be_bytes()).await.unwrap();
            server_side.write_all(&buf).await.unwrap();
        });

        let mut client = SpringboardClient::new(client_side);
        let png = client.get_homescreen_wallpaper_pngdata().await.unwrap();
        assert_eq!(png, vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    }
}
