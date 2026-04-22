use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use comfy_table::{Cell, Table};
use ios_core::{connect, ConnectOptions};
use ios_tunnel::TunMode;
use plist::Value;
#[derive(clap::Args)]
pub struct ProfilesCmd {
    #[command(subcommand)]
    sub: ProfilesSub,
}

#[derive(clap::Subcommand)]
enum ProfilesSub {
    /// List installed configuration profiles
    List {
        #[arg(
            long,
            help = "Show the raw GetProfileList response instead of flattened rows"
        )]
        raw: bool,
    },
    /// Install a configuration profile from a local .mobileconfig file
    Add {
        #[arg(help = "Path to a .mobileconfig file")]
        path: PathBuf,
        #[arg(long, help = "Supervisor identity in .p12 format for silent install")]
        p12: Option<PathBuf>,
        #[arg(long, env = "P12_PASSWORD", help = "Password for the .p12 file")]
        password: Option<String>,
    },
    /// Remove a configuration profile by identifier
    Remove {
        #[arg(help = "Profile identifier")]
        identifier: String,
    },
    /// Show detailed metadata for a single configuration profile
    Show {
        #[arg(help = "Profile identifier or UUID")]
        query: String,
    },
    /// Show cloud-configuration data as plist JSON
    CloudConfig,
    /// Show the raw stored profile for a given purpose
    Stored {
        #[arg(
            long,
            default_value = "PostSetupInstallation",
            help = "Stored profile purpose"
        )]
        purpose: String,
    },
}

impl ProfilesCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for profiles"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;
        let stream = device
            .connect_service(ios_services::mcinstall::SERVICE_NAME)
            .await?;
        let mut client = ios_services::mcinstall::McInstallClient::new(stream);

        match self.sub {
            ProfilesSub::List { raw } => {
                if raw {
                    let value = client.get_profile_list_raw().await?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&value)?);
                    } else {
                        let mut stdout = std::io::stdout().lock();
                        plist::to_writer_xml(&mut stdout, &value)?;
                        writeln!(&mut stdout)?;
                    }
                } else {
                    let profiles = client.list_profiles().await?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&profiles)?);
                    } else {
                        print_profiles(&profiles);
                    }
                }
            }
            ProfilesSub::Add {
                path,
                p12,
                password,
            } => {
                let payload = std::fs::read(&path)?;
                if let Some(p12) = p12 {
                    let p12_bytes = std::fs::read(&p12)?;
                    client
                        .install_profile_silent(
                            &payload,
                            &p12_bytes,
                            password.as_deref().unwrap_or(""),
                        )
                        .await?;
                    println!("Installed {} silently", path.display());
                } else {
                    client.install_profile(&payload).await?;
                    println!(
                        "Install request sent for {}. Confirm on the device if prompted.",
                        path.display()
                    );
                }
            }
            ProfilesSub::Remove { identifier } => {
                client.remove_profile(&identifier).await?;
                println!(
                    "Remove request sent for {identifier}. Confirm on the device if prompted."
                );
            }
            ProfilesSub::Show { query } => {
                let profiles = client.list_profiles().await?;
                let profile = profiles
                    .into_iter()
                    .find(|profile| profile_matches_query(profile, &query))
                    .ok_or_else(|| anyhow::anyhow!("profile not found: {query}"))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&profile)?);
                } else {
                    print_profile_details(&profile);
                }
            }
            ProfilesSub::CloudConfig => {
                let cloud_config = client.get_cloud_configuration().await?;
                let value = Value::Dictionary(cloud_config);
                if json {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    let mut stdout = std::io::stdout().lock();
                    plist::to_writer_xml(&mut stdout, &value)?;
                    writeln!(&mut stdout)?;
                }
            }
            ProfilesSub::Stored { purpose } => {
                let value = client.get_stored_profile_raw(&purpose).await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    let mut stdout = std::io::stdout().lock();
                    plist::to_writer_xml(&mut stdout, &value)?;
                    writeln!(&mut stdout)?;
                }
            }
        }

        Ok(())
    }
}

fn print_profiles(profiles: &[ios_services::mcinstall::ProfileInfo]) {
    let mut table = Table::new();
    table.set_header([
        "Identifier",
        "DisplayName",
        "UUID",
        "Version",
        "Active",
        "Status",
        "RemovalDisallowed",
        "Description",
    ]);

    for profile in profiles {
        table.add_row([
            Cell::new(&profile.identifier),
            Cell::new(&profile.display_name),
            Cell::new(profile.uuid.as_deref().unwrap_or("")),
            Cell::new(
                profile
                    .version
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            ),
            Cell::new(if profile.is_active { "yes" } else { "no" }),
            Cell::new(profile.status.as_deref().unwrap_or("")),
            Cell::new(match profile.removal_disallowed {
                Some(true) => "yes",
                Some(false) => "no",
                None => "",
            }),
            Cell::new(profile.description.as_deref().unwrap_or("")),
        ]);
    }

    println!("{table}");
}

fn print_profile_details(profile: &ios_services::mcinstall::ProfileInfo) {
    for (label, value) in profile_detail_lines(profile) {
        println!("{label:<19} {value}");
    }
}

fn profile_detail_lines(
    profile: &ios_services::mcinstall::ProfileInfo,
) -> Vec<(&'static str, String)> {
    let mut lines = vec![
        ("Identifier:", profile.identifier.clone()),
        ("DisplayName:", profile.display_name.clone()),
    ];

    if let Some(uuid) = &profile.uuid {
        lines.push(("UUID:", uuid.clone()));
    }
    if let Some(version) = profile.version {
        lines.push(("Version:", version.to_string()));
    }
    lines.push((
        "Active:",
        if profile.is_active { "yes" } else { "no" }.to_string(),
    ));
    if let Some(status) = &profile.status {
        lines.push(("Status:", status.clone()));
    }
    if let Some(removal_disallowed) = profile.removal_disallowed {
        lines.push((
            "RemovalDisallowed:",
            if removal_disallowed { "yes" } else { "no" }.to_string(),
        ));
    }
    if let Some(description) = &profile.description {
        lines.push(("Description:", description.clone()));
    }

    lines
}

fn profile_matches_query(profile: &ios_services::mcinstall::ProfileInfo, query: &str) -> bool {
    profile.identifier == query
        || profile.display_name == query
        || profile.uuid.as_deref() == Some(query)
        || profile
            .uuid
            .as_deref()
            .map(|uuid| {
                uuid.to_ascii_lowercase()
                    .starts_with(&query.to_ascii_lowercase())
            })
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: ProfilesSub,
    }

    #[test]
    fn parses_profiles_list_subcommand() {
        let cmd = TestCli::parse_from(["profiles", "list", "--raw"]);
        match cmd.command {
            ProfilesSub::List { raw } => assert!(raw),
            _ => panic!("expected list subcommand"),
        }
    }

    #[test]
    fn parses_profiles_add_subcommand() {
        let cmd = TestCli::parse_from(["profiles", "add", "test.mobileconfig"]);
        match cmd.command {
            ProfilesSub::Add {
                path,
                p12,
                password,
            } => {
                assert_eq!(path, PathBuf::from("test.mobileconfig"));
                assert_eq!(p12, None);
                assert_eq!(password, None);
            }
            _ => panic!("expected add subcommand"),
        }
    }

    #[test]
    fn parses_profiles_add_with_p12_subcommand() {
        let parsed = TestCli::try_parse_from([
            "profiles",
            "add",
            "test.mobileconfig",
            "--p12",
            "identity.p12",
            "--password",
            "secret",
        ]);
        assert!(parsed.is_ok(), "profiles add --p12 command should parse");
    }

    #[test]
    fn parses_profiles_remove_subcommand() {
        let cmd = TestCli::parse_from(["profiles", "remove", "com.example.profile"]);
        match cmd.command {
            ProfilesSub::Remove { identifier } => assert_eq!(identifier, "com.example.profile"),
            _ => panic!("expected remove subcommand"),
        }
    }

    #[test]
    fn parses_profiles_show_subcommand() {
        let cmd = TestCli::parse_from(["profiles", "show", "com.example.profile"]);
        match cmd.command {
            ProfilesSub::Show { query } => assert_eq!(query, "com.example.profile"),
            _ => panic!("expected show subcommand"),
        }
    }

    #[test]
    fn parses_profiles_cloud_config_subcommand() {
        let cmd = TestCli::parse_from(["profiles", "cloud-config"]);
        match cmd.command {
            ProfilesSub::CloudConfig => {}
            _ => panic!("expected cloud-config subcommand"),
        }
    }

    #[test]
    fn parses_profiles_stored_subcommand() {
        let cmd = TestCli::parse_from(["profiles", "stored", "--purpose", "PostSetupInstallation"]);
        match cmd.command {
            ProfilesSub::Stored { purpose } => assert_eq!(purpose, "PostSetupInstallation"),
            _ => panic!("expected stored subcommand"),
        }
    }

    #[test]
    fn print_profiles_includes_extended_metadata_columns() {
        let output = {
            let profiles = vec![ios_services::mcinstall::ProfileInfo {
                identifier: "com.example.profile".into(),
                display_name: "Example".into(),
                description: Some("Example description".into()),
                is_active: true,
                removal_disallowed: Some(false),
                status: Some("Acknowledged".into()),
                uuid: Some("1234-5678".into()),
                version: Some(7),
            }];

            let mut table = comfy_table::Table::new();
            table.set_header([
                "Identifier",
                "DisplayName",
                "UUID",
                "Version",
                "Active",
                "Status",
                "RemovalDisallowed",
                "Description",
            ]);

            for profile in &profiles {
                table.add_row([
                    comfy_table::Cell::new(&profile.identifier),
                    comfy_table::Cell::new(&profile.display_name),
                    comfy_table::Cell::new(profile.uuid.as_deref().unwrap_or("")),
                    comfy_table::Cell::new(
                        profile
                            .version
                            .map(|value| value.to_string())
                            .unwrap_or_default(),
                    ),
                    comfy_table::Cell::new(if profile.is_active { "yes" } else { "no" }),
                    comfy_table::Cell::new(profile.status.as_deref().unwrap_or("")),
                    comfy_table::Cell::new(match profile.removal_disallowed {
                        Some(true) => "yes",
                        Some(false) => "no",
                        None => "",
                    }),
                    comfy_table::Cell::new(profile.description.as_deref().unwrap_or("")),
                ]);
            }

            table.to_string()
        };

        assert!(output.contains("UUID"));
        assert!(output.contains("Version"));
        assert!(output.contains("Status"));
        assert!(output.contains("1234-5678"));
        assert!(output.contains("Acknowledged"));
        assert!(output.contains("7"));
    }

    #[test]
    fn print_profile_details_includes_extended_fields() {
        let profile = ios_services::mcinstall::ProfileInfo {
            identifier: "com.example.profile".into(),
            display_name: "Example".into(),
            description: Some("Example description".into()),
            is_active: true,
            removal_disallowed: Some(false),
            status: Some("Acknowledged".into()),
            uuid: Some("1234-5678".into()),
            version: Some(7),
        };

        let lines = profile_detail_lines(&profile);

        assert!(lines.contains(&("UUID:", "1234-5678".into())));
        assert!(lines.contains(&("Version:", "7".into())));
        assert!(lines.contains(&("Status:", "Acknowledged".into())));
        assert!(lines.contains(&("RemovalDisallowed:", "no".into())));
    }

    #[test]
    fn profile_matches_query_accepts_identifier_or_uuid() {
        let profile = ios_services::mcinstall::ProfileInfo {
            identifier: "com.example.profile".into(),
            display_name: "Example".into(),
            description: None,
            is_active: true,
            removal_disallowed: None,
            status: Some("Acknowledged".into()),
            uuid: Some("1234-5678".into()),
            version: Some(1),
        };

        assert!(profile_matches_query(&profile, "com.example.profile"));
        assert!(profile_matches_query(&profile, "1234-5678"));
        assert!(!profile_matches_query(&profile, "com.example.other"));
    }

    #[test]
    fn profile_matches_query_accepts_display_name_and_uuid_prefix() {
        let profile = ios_services::mcinstall::ProfileInfo {
            identifier: "com.example.profile".into(),
            display_name: "Example".into(),
            description: None,
            is_active: true,
            removal_disallowed: None,
            status: Some("Acknowledged".into()),
            uuid: Some("1234-5678".into()),
            version: Some(1),
        };

        assert!(profile_matches_query(&profile, "Example"));
        assert!(profile_matches_query(&profile, "1234"));
    }
}
