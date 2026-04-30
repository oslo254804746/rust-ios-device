use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use ios_core::springboard::{Icon, SpringboardClient};
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};
use tokio::fs;

#[derive(clap::Args)]
pub struct SpringboardCmd {
    #[command(subcommand)]
    sub: SpringboardSub,
}

#[derive(clap::Subcommand)]
enum SpringboardSub {
    /// List the Home Screen icon layout
    Icons {
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Dump the raw SpringBoard icon state payload
    State {
        #[arg(
            long,
            default_value = "2",
            help = "SpringBoard icon state format version"
        )]
        format_version: String,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Set the Home Screen icon layout from a plist file
    StateSet {
        #[arg(help = "Path to icon state plist file (XML or binary)")]
        input: PathBuf,
    },
    /// Find a bundle ID in the Home Screen layout
    Find {
        #[arg(help = "Bundle ID (e.g. com.apple.Preferences)")]
        bundle_id: String,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Fetch the PNG icon data for a bundle ID
    Icon {
        #[arg(help = "Bundle ID (e.g. com.apple.Preferences)")]
        bundle_id: String,
        #[arg(help = "Output path (defaults to <bundle_id>.png)")]
        output: Option<PathBuf>,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Fetch PNG icon data for every app shown on the Home Screen
    IconAll {
        #[arg(help = "Output directory (defaults to springboard-icons)")]
        output_dir: Option<PathBuf>,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Read the current SpringBoard interface orientation
    Orientation {
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Read Home Screen icon layout metrics
    IconMetrics {
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Save the preview image for a specific wallpaper
    WallpaperPreviewImage {
        #[arg(help = "Wallpaper name (e.g. homescreen, lockscreen)")]
        wallpaper_name: String,
        #[arg(help = "Output path for the preview PNG")]
        output: PathBuf,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Read SpringBoard wallpaper metadata
    WallpaperInfo {
        #[arg(help = "Wallpaper name (e.g. homescreen, lockscreen)")]
        wallpaper_name: String,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Save the full-size Home Screen wallpaper PNG
    WallpaperHomeScreen {
        #[arg(help = "Output path for the wallpaper PNG")]
        output: PathBuf,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
}

impl SpringboardCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for springboard"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;
        let stream = device
            .connect_service(ios_core::springboard::SERVICE_NAME)
            .await?;
        let mut client = SpringboardClient::new(stream);

        match self.sub {
            SpringboardSub::Icons { json } => {
                let screens = client.list_icons().await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&screens_to_json(&screens))?
                    );
                } else {
                    print_screens(&screens);
                }
            }
            SpringboardSub::State {
                format_version,
                json,
            } => {
                let state = client.get_icon_state_raw(&format_version).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&plist_to_json(&state))?);
                } else {
                    let mut stdout = std::io::stdout().lock();
                    plist::to_writer_xml(&mut stdout, &state)?;
                    writeln!(&mut stdout)?;
                }
            }
            SpringboardSub::StateSet { input } => {
                let data = fs::read(&input).await?;
                let icon_state: plist::Value = plist::from_bytes(&data)?;
                client.set_icon_state(&icon_state).await?;
                println!("Set icon state from {}", input.display());
            }
            SpringboardSub::Find { bundle_id, json } => {
                let screens = client.list_icons().await?;
                let locations = find_bundle_locations(&screens, &bundle_id);
                if locations.is_empty() {
                    anyhow::bail!("bundle not found in SpringBoard layout: {bundle_id}");
                }
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&find_locations_to_json(
                            &bundle_id, &locations
                        ))?
                    );
                } else {
                    for location in locations {
                        println!("{location}");
                    }
                }
            }
            SpringboardSub::Icon {
                bundle_id,
                output,
                json,
            } => {
                let png = client.get_icon_png_data(&bundle_id).await?;
                let output = output.unwrap_or_else(|| PathBuf::from(format!("{bundle_id}.png")));
                let bytes = png.len();
                fs::write(&output, png).await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&icon_result_to_json(
                            &bundle_id,
                            output.to_string_lossy().as_ref(),
                            bytes,
                        ))?
                    );
                } else {
                    println!("Saved icon for {bundle_id} to {}", output.display());
                }
            }
            SpringboardSub::IconAll { output_dir, json } => {
                let screens = client.list_icons().await?;
                let bundle_ids = collect_bundle_ids(&screens);
                let output_dir = output_dir.unwrap_or_else(|| PathBuf::from("springboard-icons"));
                fs::create_dir_all(&output_dir).await?;

                let mut saved = 0usize;
                let mut entries = Vec::new();
                for bundle_id in bundle_ids {
                    let png = client.get_icon_png_data(&bundle_id).await?;
                    let output = output_dir.join(format!("{bundle_id}.png"));
                    let bytes = png.len();
                    fs::write(&output, png).await?;
                    if json {
                        entries.push(IconAllSavedEntry {
                            bundle_id,
                            output: output.to_string_lossy().into_owned(),
                            bytes,
                        });
                    } else {
                        println!("Saved {bundle_id} -> {}", output.display());
                    }
                    saved += 1;
                }
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&icon_all_result_to_json(
                            output_dir.to_string_lossy().as_ref(),
                            saved,
                            &entries,
                        ))?
                    );
                } else {
                    println!("Saved {saved} icon(s) to {}", output_dir.display());
                }
            }
            SpringboardSub::Orientation { json } => {
                let orientation = client.get_interface_orientation().await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "raw": orientation.raw_value(),
                            "label": orientation.label(),
                        }))?
                    );
                } else {
                    println!("{} ({})", orientation.raw_value(), orientation.label());
                }
            }
            SpringboardSub::IconMetrics { json } => {
                let metrics = client.get_homescreen_icon_metrics().await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&plist_to_json(&metrics))?
                    );
                } else if let Some(dict) = metrics.as_dictionary() {
                    let mut entries: Vec<_> = dict.iter().collect();
                    entries.sort_by(|a, b| a.0.cmp(b.0));
                    for (key, value) in entries {
                        println!("{key}: {}", format_json_value(&plist_to_json(value)));
                    }
                } else {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&plist_to_json(&metrics))?
                    );
                }
            }
            SpringboardSub::WallpaperPreviewImage {
                wallpaper_name,
                output,
                json,
            } => {
                let png = client.get_wallpaper_preview_image(&wallpaper_name).await?;
                let bytes = png.len();
                fs::write(&output, png).await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&png_result_to_json(
                            &wallpaper_name,
                            output.to_string_lossy().as_ref(),
                            bytes,
                        ))?
                    );
                } else {
                    println!(
                        "Saved {wallpaper_name} wallpaper preview to {}",
                        output.display()
                    );
                }
            }
            SpringboardSub::WallpaperInfo {
                wallpaper_name,
                json,
            } => {
                let info = client.get_wallpaper_info(&wallpaper_name).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&plist_to_json(&info))?);
                } else if let Some(dict) = info.as_dictionary() {
                    let mut entries: Vec<_> = dict.iter().collect();
                    entries.sort_by(|a, b| a.0.cmp(b.0));
                    for (key, value) in entries {
                        println!("{key}: {}", format_json_value(&plist_to_json(value)));
                    }
                } else {
                    println!("{}", serde_json::to_string_pretty(&plist_to_json(&info))?);
                }
            }
            SpringboardSub::WallpaperHomeScreen { output, json } => {
                let png = client.get_homescreen_wallpaper_pngdata().await?;
                let bytes = png.len();
                fs::write(&output, png).await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&png_result_to_json(
                            "homescreen-fullsize",
                            output.to_string_lossy().as_ref(),
                            bytes,
                        ))?
                    );
                } else {
                    println!(
                        "Saved full-size Home Screen wallpaper to {} ({bytes} bytes)",
                        output.display()
                    );
                }
            }
        }

        Ok(())
    }
}

fn screens_to_json(screens: &[Vec<Icon>]) -> serde_json::Value {
    serde_json::Value::Array(
        screens
            .iter()
            .enumerate()
            .map(|(screen_idx, screen)| {
                serde_json::json!({
                    "page": screen_idx + 1,
                    "icons": screen.iter().map(icon_to_json).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn icon_to_json(icon: &Icon) -> serde_json::Value {
    match icon {
        Icon::App(app) => serde_json::json!({
            "kind": "app",
            "display_name": app.display_name,
            "display_identifier": app.display_identifier,
            "bundle_id": app.bundle_identifier,
            "bundle_version": app.bundle_version,
        }),
        Icon::WebClip(web_clip) => serde_json::json!({
            "kind": "webclip",
            "display_name": web_clip.display_name,
            "display_identifier": web_clip.display_identifier,
            "url": web_clip.url,
        }),
        Icon::Custom(custom) => serde_json::json!({
            "kind": "custom",
            "icon_type": custom.icon_type,
        }),
        Icon::Folder(folder) => serde_json::json!({
            "kind": "folder",
            "display_name": folder.display_name,
            "pages": folder
                .pages
                .iter()
                .map(|page| page.iter().map(icon_to_json).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
        }),
    }
}

fn print_screens(screens: &[Vec<Icon>]) {
    for (screen_idx, screen) in screens.iter().enumerate() {
        println!("Page {}", screen_idx + 1);
        for icon in screen {
            print_icon(icon, "  ");
        }
    }
}

fn print_icon(icon: &Icon, indent: &str) {
    match icon {
        Icon::App(app) => {
            println!("{indent}- {} ({})", app.display_name, app.bundle_identifier);
        }
        Icon::WebClip(web_clip) => {
            println!(
                "{indent}- {} [webclip: {}]",
                web_clip.display_name, web_clip.url
            );
        }
        Icon::Custom(_) => {
            println!("{indent}- <custom icon>");
        }
        Icon::Folder(folder) => {
            println!("{indent}- Folder: {}", folder.display_name);
            for (page_idx, page) in folder.pages.iter().enumerate() {
                println!("{indent}  Page {}", page_idx + 1);
                for nested in page {
                    print_icon(nested, &format!("{indent}    "));
                }
            }
        }
    }
}

fn find_locations_to_json(bundle_id: &str, locations: &[String]) -> serde_json::Value {
    serde_json::json!({
        "bundle_id": bundle_id,
        "locations": locations,
    })
}

fn icon_result_to_json(bundle_id: &str, output: &str, bytes: usize) -> serde_json::Value {
    serde_json::json!({
        "bundle_id": bundle_id,
        "output": output,
        "bytes": bytes,
    })
}

fn png_result_to_json(name: &str, output: &str, bytes: usize) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "output": output,
        "bytes": bytes,
    })
}

struct IconAllSavedEntry {
    bundle_id: String,
    output: String,
    bytes: usize,
}

fn icon_all_result_to_json(
    output_dir: &str,
    saved_count: usize,
    entries: &[IconAllSavedEntry],
) -> serde_json::Value {
    serde_json::json!({
        "output_dir": output_dir,
        "saved_count": saved_count,
        "saved": entries
            .iter()
            .map(|entry| serde_json::json!({
                "bundle_id": entry.bundle_id,
                "output": entry.output,
                "bytes": entry.bytes,
            }))
            .collect::<Vec<_>>(),
    })
}

fn find_bundle_locations(screens: &[Vec<Icon>], bundle_id: &str) -> Vec<String> {
    let mut locations = Vec::new();

    for (screen_idx, screen) in screens.iter().enumerate() {
        let prefix = format!("Page {}", screen_idx + 1);
        find_bundle_locations_in_icons(screen, bundle_id, &prefix, &mut locations);
    }

    locations
}

fn find_bundle_locations_in_icons(
    icons: &[Icon],
    bundle_id: &str,
    prefix: &str,
    locations: &mut Vec<String>,
) {
    for (icon_idx, icon) in icons.iter().enumerate() {
        let path = format!("{prefix} -> Slot {}", icon_idx + 1);
        match icon {
            Icon::App(app) => {
                if app.bundle_identifier == bundle_id {
                    locations.push(format!("{path} ({})", app.display_name));
                }
            }
            Icon::Folder(folder) => {
                for (page_idx, page) in folder.pages.iter().enumerate() {
                    let nested_prefix = format!(
                        "{path} -> Folder {} -> Page {}",
                        folder.display_name,
                        page_idx + 1
                    );
                    find_bundle_locations_in_icons(page, bundle_id, &nested_prefix, locations);
                }
            }
            Icon::WebClip(_) | Icon::Custom(_) => {}
        }
    }
}

fn collect_bundle_ids(screens: &[Vec<Icon>]) -> BTreeSet<String> {
    let mut bundle_ids = BTreeSet::new();
    for screen in screens {
        collect_bundle_ids_in_icons(screen, &mut bundle_ids);
    }
    bundle_ids
}

fn collect_bundle_ids_in_icons(icons: &[Icon], bundle_ids: &mut BTreeSet<String>) {
    for icon in icons {
        match icon {
            Icon::App(app) => {
                bundle_ids.insert(app.bundle_identifier.clone());
            }
            Icon::Folder(folder) => {
                for page in &folder.pages {
                    collect_bundle_ids_in_icons(page, bundle_ids);
                }
            }
            Icon::WebClip(_) | Icon::Custom(_) => {}
        }
    }
}

fn plist_to_json(value: &plist::Value) -> serde_json::Value {
    match value {
        plist::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(plist_to_json).collect())
        }
        plist::Value::Boolean(v) => serde_json::Value::Bool(*v),
        plist::Value::Data(bytes) => serde_json::Value::String(hex::encode(bytes)),
        plist::Value::Date(date) => serde_json::Value::String(date.to_xml_format()),
        plist::Value::Dictionary(dict) => serde_json::Value::Object(
            dict.iter()
                .map(|(key, value)| (key.clone(), plist_to_json(value)))
                .collect(),
        ),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(serde_json::Value::from)
            .or_else(|| value.as_unsigned().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        plist::Value::Real(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        plist::Value::String(value) => serde_json::Value::String(value.clone()),
        plist::Value::Uid(uid) => serde_json::Value::from(uid.get()),
        _ => serde_json::Value::Null,
    }
}

fn format_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ios_core::springboard::InterfaceOrientation;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: SpringboardSub,
    }

    #[test]
    fn parses_icons_subcommand() {
        let cmd = TestCli::parse_from(["springboard", "icons", "--json"]);
        match cmd.command {
            SpringboardSub::Icons { json } => assert!(json),
            _ => panic!("expected icons subcommand"),
        }
    }

    #[test]
    fn parses_state_subcommand() {
        let cmd = TestCli::parse_from(["springboard", "state", "--format-version", "2", "--json"]);
        match cmd.command {
            SpringboardSub::State {
                format_version,
                json,
            } => {
                assert_eq!(format_version, "2");
                assert!(json);
            }
            _ => panic!("expected state subcommand"),
        }
    }

    #[test]
    fn parses_find_subcommand() {
        let cmd = TestCli::parse_from(["springboard", "find", "com.apple.Preferences", "--json"]);
        match cmd.command {
            SpringboardSub::Find { bundle_id, json } => {
                assert_eq!(bundle_id, "com.apple.Preferences");
                assert!(json);
            }
            _ => panic!("expected find subcommand"),
        }
    }

    #[test]
    fn parses_icon_subcommand() {
        let cmd = TestCli::parse_from([
            "springboard",
            "icon",
            "com.apple.Preferences",
            "settings.png",
            "--json",
        ]);
        match cmd.command {
            SpringboardSub::Icon {
                bundle_id,
                output,
                json,
            } => {
                assert_eq!(bundle_id, "com.apple.Preferences");
                assert_eq!(output, Some(PathBuf::from("settings.png")));
                assert!(json);
            }
            _ => panic!("expected icon subcommand"),
        }
    }

    #[test]
    fn parses_icon_all_subcommand() {
        let cmd = TestCli::parse_from(["springboard", "icon-all", "icons"]);
        match cmd.command {
            SpringboardSub::IconAll { output_dir, json } => {
                assert_eq!(output_dir, Some(PathBuf::from("icons")));
                assert!(!json);
            }
            _ => panic!("expected icon-all subcommand"),
        }
    }

    #[test]
    fn parses_icon_all_json_flag() {
        let cmd = TestCli::parse_from(["springboard", "icon-all", "--json"]);
        match cmd.command {
            SpringboardSub::IconAll { output_dir, json } => {
                assert_eq!(output_dir, None);
                assert!(json);
            }
            _ => panic!("expected icon-all subcommand"),
        }
    }

    #[test]
    fn parses_orientation_subcommand() {
        let cmd = TestCli::parse_from(["springboard", "orientation", "--json"]);
        match cmd.command {
            SpringboardSub::Orientation { json } => assert!(json),
            _ => panic!("expected orientation subcommand"),
        }
    }

    #[test]
    fn parses_icon_metrics_subcommand() {
        let cmd = TestCli::parse_from(["springboard", "icon-metrics", "--json"]);
        match cmd.command {
            SpringboardSub::IconMetrics { json } => assert!(json),
            _ => panic!("expected icon-metrics subcommand"),
        }
    }

    #[test]
    fn parses_wallpaper_info_subcommand() {
        let cmd = TestCli::parse_from(["springboard", "wallpaper-info", "homescreen", "--json"]);
        match cmd.command {
            SpringboardSub::WallpaperInfo {
                wallpaper_name,
                json,
            } => {
                assert_eq!(wallpaper_name, "homescreen");
                assert!(json);
            }
            _ => panic!("expected wallpaper-info subcommand"),
        }
    }

    #[test]
    fn parses_wallpaper_preview_image_subcommand() {
        let cmd = TestCli::parse_from([
            "springboard",
            "wallpaper-preview-image",
            "lockscreen",
            "ios-rs-tmp/lockscreen-preview.png",
            "--json",
        ]);
        match cmd.command {
            SpringboardSub::WallpaperPreviewImage {
                wallpaper_name,
                output,
                json,
            } => {
                assert_eq!(wallpaper_name, "lockscreen");
                assert_eq!(output, PathBuf::from("ios-rs-tmp/lockscreen-preview.png"));
                assert!(json);
            }
            _ => panic!("expected wallpaper-preview-image subcommand"),
        }
    }

    #[test]
    fn finds_bundle_locations_in_nested_folder_pages() {
        let screens = vec![vec![
            Icon::App(ios_core::springboard::AppIcon {
                display_name: "Phone".into(),
                display_identifier: None,
                bundle_identifier: "com.apple.mobilephone".into(),
                bundle_version: None,
            }),
            Icon::Folder(ios_core::springboard::Folder {
                display_name: "Utilities".into(),
                pages: vec![vec![Icon::App(ios_core::springboard::AppIcon {
                    display_name: "Settings".into(),
                    display_identifier: None,
                    bundle_identifier: "com.apple.Preferences".into(),
                    bundle_version: None,
                })]],
            }),
        ]];

        let locations = find_bundle_locations(&screens, "com.apple.Preferences");
        assert_eq!(
            locations,
            vec!["Page 1 -> Slot 2 -> Folder Utilities -> Page 1 -> Slot 1 (Settings)"]
        );
    }

    #[test]
    fn collect_bundle_ids_deduplicates_nested_apps() {
        let screens = vec![vec![
            Icon::App(ios_core::springboard::AppIcon {
                display_name: "Phone".into(),
                display_identifier: None,
                bundle_identifier: "com.apple.mobilephone".into(),
                bundle_version: None,
            }),
            Icon::Folder(ios_core::springboard::Folder {
                display_name: "Utilities".into(),
                pages: vec![vec![
                    Icon::App(ios_core::springboard::AppIcon {
                        display_name: "Settings".into(),
                        display_identifier: None,
                        bundle_identifier: "com.apple.Preferences".into(),
                        bundle_version: None,
                    }),
                    Icon::App(ios_core::springboard::AppIcon {
                        display_name: "Phone".into(),
                        display_identifier: None,
                        bundle_identifier: "com.apple.mobilephone".into(),
                        bundle_version: None,
                    }),
                ]],
            }),
        ]];

        let bundle_ids = collect_bundle_ids(&screens);
        assert_eq!(
            bundle_ids.into_iter().collect::<Vec<_>>(),
            vec![
                "com.apple.Preferences".to_string(),
                "com.apple.mobilephone".to_string(),
            ]
        );
    }

    #[test]
    fn screen_layout_json_includes_nested_folder_icons() {
        let screens = vec![vec![
            Icon::App(ios_core::springboard::AppIcon {
                display_name: "Phone".into(),
                display_identifier: Some("com.apple.mobilephone".into()),
                bundle_identifier: "com.apple.mobilephone".into(),
                bundle_version: Some("1.0".into()),
            }),
            Icon::Folder(ios_core::springboard::Folder {
                display_name: "Utilities".into(),
                pages: vec![vec![Icon::App(ios_core::springboard::AppIcon {
                    display_name: "Settings".into(),
                    display_identifier: None,
                    bundle_identifier: "com.apple.Preferences".into(),
                    bundle_version: None,
                })]],
            }),
        ]];

        let value = screens_to_json(&screens);
        let pages = value.as_array().unwrap();
        assert_eq!(pages.len(), 1);
        let icons = pages[0]["icons"].as_array().unwrap();
        assert_eq!(icons[0]["kind"], "app");
        assert_eq!(icons[0]["bundle_id"], "com.apple.mobilephone");
        assert_eq!(icons[1]["kind"], "folder");
        assert_eq!(
            icons[1]["pages"][0][0]["bundle_id"],
            "com.apple.Preferences"
        );
    }

    #[test]
    fn find_locations_json_includes_bundle_id_and_locations() {
        let value = find_locations_to_json(
            "com.apple.Preferences",
            &["Page 1 -> Slot 2 (Settings)".to_string()],
        );
        assert_eq!(value["bundle_id"], "com.apple.Preferences");
        assert_eq!(value["locations"][0], "Page 1 -> Slot 2 (Settings)");
    }

    #[test]
    fn icon_result_json_includes_bundle_id_output_and_bytes() {
        let value = icon_result_to_json("com.apple.Preferences", "settings.png", 2048);
        assert_eq!(value["bundle_id"], "com.apple.Preferences");
        assert_eq!(value["output"], "settings.png");
        assert_eq!(value["bytes"], 2048);
    }

    #[test]
    fn icon_all_result_json_includes_output_dir_count_and_entries() {
        let value = icon_all_result_to_json(
            "springboard-icons",
            2,
            &[
                IconAllSavedEntry {
                    bundle_id: "com.apple.Preferences".into(),
                    output: "springboard-icons/com.apple.Preferences.png".into(),
                    bytes: 1024,
                },
                IconAllSavedEntry {
                    bundle_id: "com.apple.mobilephone".into(),
                    output: "springboard-icons/com.apple.mobilephone.png".into(),
                    bytes: 2048,
                },
            ],
        );
        assert_eq!(value["output_dir"], "springboard-icons");
        assert_eq!(value["saved_count"], 2);
        assert_eq!(value["saved"][0]["bundle_id"], "com.apple.Preferences");
        assert_eq!(value["saved"][1]["bytes"], 2048);
    }

    #[test]
    fn plist_to_json_converts_metric_dictionary() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("iconWidth".to_string(), plist::Value::Real(60.0)),
            ("iconHeight".to_string(), plist::Value::Integer(62.into())),
        ]));

        assert_eq!(
            plist_to_json(&value),
            serde_json::json!({
                "iconWidth": 60.0,
                "iconHeight": 62
            })
        );
    }

    #[test]
    fn interface_orientation_labels_match_expected_strings() {
        assert_eq!(InterfaceOrientation::Portrait.label(), "portrait");
        assert_eq!(InterfaceOrientation::Landscape.raw_value(), 3);
        assert_eq!(InterfaceOrientation::Unknown(9).label(), "unknown");
    }
}
