use std::path::PathBuf;

use anyhow::Result;
use comfy_table::{Cell, Table};
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct ProvisioningCmd {
    #[command(subcommand)]
    sub: ProvisioningSub,
}

#[derive(clap::Subcommand)]
enum ProvisioningSub {
    /// List installed provisioning profiles
    List,
    /// Show detailed metadata for one installed provisioning profile
    Show {
        #[arg(help = "Provisioning profile UUID")]
        query: String,
    },
    /// Install a provisioning profile from a local file
    Install {
        #[arg(help = "Path to a .mobileprovision file")]
        path: PathBuf,
    },
    /// Export one installed provisioning profile to a local file
    Export {
        #[arg(help = "Provisioning profile UUID")]
        query: String,
        #[arg(help = "Destination path for the .mobileprovision file")]
        output: PathBuf,
    },
    /// Export all installed provisioning profiles into a local directory
    Dump {
        #[arg(help = "Destination directory for exported .mobileprovision files")]
        output_dir: PathBuf,
    },
    /// Remove an installed provisioning profile by UUID
    Remove {
        #[arg(help = "Provisioning profile UUID")]
        uuid: String,
    },
}

impl ProvisioningCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for provisioning"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = connect(&udid, opts).await?;
        let stream = device
            .connect_service(ios_core::services::misagent::SERVICE_NAME)
            .await?;
        let mut client = ios_core::services::misagent::MisagentClient::new(stream);

        match self.sub {
            ProvisioningSub::List => {
                let profiles = client.list_profiles().await?;
                if json {
                    let list: Vec<_> = profiles.iter().map(profile_to_json).collect();
                    println!("{}", serde_json::to_string_pretty(&list)?);
                } else {
                    print_profiles(&profiles);
                }
            }
            ProvisioningSub::Show { query } => {
                let profiles = client.list_profiles().await?;
                let profile = find_profile(&profiles, &query)?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&profile_to_json(profile))?
                    );
                } else {
                    print_profile_details(profile);
                }
            }
            ProvisioningSub::Install { path } => {
                let payload = std::fs::read(&path)?;
                client.install(&payload).await?;
                println!("Installed provisioning profile from {}", path.display());
            }
            ProvisioningSub::Export { query, output } => {
                let profiles = client.list_profiles().await?;
                let profile = find_profile(&profiles, &query)?;
                std::fs::write(&output, &profile.raw_data)?;

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&export_result_json(
                            &profile.uuid,
                            &output,
                            profile.raw_data.len(),
                        ))?
                    );
                } else {
                    println!(
                        "{}",
                        export_success_message(&profile.uuid, &output, profile.raw_data.len())
                    );
                }
            }
            ProvisioningSub::Dump { output_dir } => {
                let profiles = client.list_profiles().await?;
                std::fs::create_dir_all(&output_dir)?;

                let mut entries = Vec::with_capacity(profiles.len());
                for profile in &profiles {
                    let output = output_dir.join(format!("{}.mobileprovision", profile.uuid));
                    std::fs::write(&output, &profile.raw_data)?;
                    entries.push(dump_entry_json(
                        &profile.uuid,
                        &output,
                        profile.raw_data.len(),
                    ));
                }

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&dump_result_json(&output_dir, &entries))?
                    );
                } else {
                    for entry in &entries {
                        println!("{}", dump_entry_message(entry));
                    }
                    println!(
                        "Dumped {} provisioning profile(s) to {}",
                        entries.len(),
                        output_dir.display()
                    );
                }
            }
            ProvisioningSub::Remove { uuid } => {
                client.remove(&uuid).await?;
                println!("Removed provisioning profile {uuid}");
            }
        }

        Ok(())
    }
}

fn profile_to_json(profile: &ios_core::services::misagent::Profile) -> serde_json::Value {
    serde_json::json!({
        "uuid": profile.uuid,
        "name": profile.name,
        "app_id": profile.app_id,
        "expiry_date": profile.expiry_date,
        "size": profile.raw_data.len(),
    })
}

fn find_profile<'a>(
    profiles: &'a [ios_core::services::misagent::Profile],
    query: &str,
) -> Result<&'a ios_core::services::misagent::Profile> {
    if let Some(profile) = unique_profile_match(profiles, query, |profile| {
        profile.uuid.eq_ignore_ascii_case(query)
    })? {
        return Ok(profile);
    }

    if let Some(profile) = unique_profile_match(profiles, query, |profile| profile.name == query)? {
        return Ok(profile);
    }

    let query_lower = query.to_ascii_lowercase();
    if let Some(profile) = unique_profile_match(profiles, query, |profile| {
        profile.uuid.to_ascii_lowercase().starts_with(&query_lower)
    })? {
        return Ok(profile);
    }

    Err(anyhow::anyhow!("provisioning profile not found: {query}"))
}

fn unique_profile_match<'a, F>(
    profiles: &'a [ios_core::services::misagent::Profile],
    query: &str,
    predicate: F,
) -> Result<Option<&'a ios_core::services::misagent::Profile>>
where
    F: Fn(&ios_core::services::misagent::Profile) -> bool,
{
    let mut matches = profiles.iter().filter(|profile| predicate(profile));
    let first = matches.next();
    let second = matches.next();

    match (first, second) {
        (None, _) => Ok(None),
        (Some(profile), None) => Ok(Some(profile)),
        (Some(_), Some(_)) => Err(anyhow::anyhow!(
            "multiple provisioning profiles match query: {query}"
        )),
    }
}

fn export_result_json(uuid: &str, output: &std::path::Path, bytes: usize) -> serde_json::Value {
    serde_json::json!({
        "uuid": uuid,
        "output": output.display().to_string(),
        "bytes": bytes,
    })
}

fn export_success_message(uuid: &str, output: &std::path::Path, bytes: usize) -> String {
    format!(
        "Exported provisioning profile {uuid} to {} ({bytes} bytes)",
        output.display()
    )
}

fn dump_entry_json(uuid: &str, output: &std::path::Path, bytes: usize) -> serde_json::Value {
    serde_json::json!({
        "uuid": uuid,
        "output": output.display().to_string(),
        "bytes": bytes,
    })
}

fn dump_result_json(
    output_dir: &std::path::Path,
    entries: &[serde_json::Value],
) -> serde_json::Value {
    serde_json::json!({
        "output_dir": output_dir.display().to_string(),
        "saved_count": entries.len(),
        "entries": entries,
    })
}

fn dump_entry_message(entry: &serde_json::Value) -> String {
    let uuid = entry
        .get("uuid")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<unknown>");
    let output = entry
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<unknown>");
    let bytes = entry
        .get("bytes")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    format!("Dumped provisioning profile {uuid} to {output} ({bytes} bytes)")
}

fn print_profiles(profiles: &[ios_core::services::misagent::Profile]) {
    let mut table = Table::new();
    table.set_header(["UUID", "Name", "App ID", "Expiry", "Size"]);
    for profile in profiles {
        table.add_row([
            Cell::new(&profile.uuid),
            Cell::new(&profile.name),
            Cell::new(&profile.app_id),
            Cell::new(profile.expiry_date.as_deref().unwrap_or("")),
            Cell::new(profile.raw_data.len()),
        ]);
    }
    println!("{table}");
}

fn print_profile_details(profile: &ios_core::services::misagent::Profile) {
    for (label, value) in profile_detail_lines(profile) {
        println!("{label:<12} {value}");
    }
}

fn profile_detail_lines(
    profile: &ios_core::services::misagent::Profile,
) -> Vec<(&'static str, String)> {
    vec![
        ("UUID:", profile.uuid.clone()),
        ("Name:", profile.name.clone()),
        ("App ID:", profile.app_id.clone()),
        ("Expiry:", profile.expiry_date.clone().unwrap_or_default()),
        ("Size:", profile.raw_data.len().to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: ProvisioningSub,
    }

    #[test]
    fn parses_provisioning_list_subcommand() {
        let cmd = TestCli::parse_from(["provisioning", "list"]);
        match cmd.command {
            ProvisioningSub::List => {}
            _ => panic!("expected list subcommand"),
        }
    }

    #[test]
    fn parses_provisioning_show_subcommand() {
        let cmd = TestCli::parse_from(["provisioning", "show", "ABC-123"]);
        match cmd.command {
            ProvisioningSub::Show { query } => assert_eq!(query, "ABC-123"),
            _ => panic!("expected show subcommand"),
        }
    }

    #[test]
    fn parses_provisioning_install_subcommand() {
        let cmd = TestCli::parse_from(["provisioning", "install", "profile.mobileprovision"]);
        match cmd.command {
            ProvisioningSub::Install { path } => {
                assert_eq!(path, PathBuf::from("profile.mobileprovision"))
            }
            _ => panic!("expected install subcommand"),
        }
    }

    #[test]
    fn parses_provisioning_remove_subcommand() {
        let cmd = TestCli::parse_from(["provisioning", "remove", "ABC-123"]);
        match cmd.command {
            ProvisioningSub::Remove { uuid } => assert_eq!(uuid, "ABC-123"),
            _ => panic!("expected remove subcommand"),
        }
    }

    #[test]
    fn parses_provisioning_export_subcommand() {
        let cmd = TestCli::parse_from([
            "provisioning",
            "export",
            "ABC-123",
            "profile.mobileprovision",
        ]);
        match cmd.command {
            ProvisioningSub::Export { query, output } => {
                assert_eq!(query, "ABC-123");
                assert_eq!(output, PathBuf::from("profile.mobileprovision"));
            }
            _ => panic!("expected export subcommand"),
        }
    }

    #[test]
    fn parses_provisioning_dump_subcommand() {
        let cmd = TestCli::parse_from(["provisioning", "dump", "profiles"]);
        match cmd.command {
            ProvisioningSub::Dump { output_dir } => {
                assert_eq!(output_dir, PathBuf::from("profiles"));
            }
            _ => panic!("expected dump subcommand"),
        }
    }

    #[test]
    fn profile_json_includes_basic_metadata() {
        let profile = ios_core::services::misagent::Profile {
            uuid: "ABC-123".into(),
            name: "Example Dev Profile".into(),
            app_id: "Example App".into(),
            expiry_date: Some("2026-04-08T00:00:00Z".into()),
            raw_data: vec![1, 2, 3, 4],
        };

        assert_eq!(
            profile_to_json(&profile),
            serde_json::json!({
                "uuid": "ABC-123",
                "name": "Example Dev Profile",
                "app_id": "Example App",
                "expiry_date": "2026-04-08T00:00:00Z",
                "size": 4,
            })
        );
    }

    #[test]
    fn profile_detail_lines_include_size() {
        let profile = ios_core::services::misagent::Profile {
            uuid: "ABC-123".into(),
            name: "Example Dev Profile".into(),
            app_id: "Example App".into(),
            expiry_date: Some("2026-04-08T00:00:00Z".into()),
            raw_data: vec![1, 2, 3, 4],
        };

        let lines = profile_detail_lines(&profile);
        assert!(lines.contains(&("UUID:", "ABC-123".into())));
        assert!(lines.contains(&("Name:", "Example Dev Profile".into())));
        assert!(lines.contains(&("App ID:", "Example App".into())));
        assert!(lines.contains(&("Expiry:", "2026-04-08T00:00:00Z".into())));
        assert!(lines.contains(&("Size:", "4".into())));
    }

    #[test]
    fn export_json_includes_uuid_output_and_bytes() {
        let value = export_result_json(
            "ABC-123",
            &PathBuf::from("profiles/exported.mobileprovision"),
            4,
        );

        assert_eq!(
            value,
            serde_json::json!({
                "uuid": "ABC-123",
                "output": "profiles/exported.mobileprovision",
                "bytes": 4,
            })
        );
    }

    #[test]
    fn export_message_mentions_uuid_output_and_bytes() {
        let message = export_success_message(
            "ABC-123",
            &PathBuf::from("profiles/exported.mobileprovision"),
            4,
        );

        assert_eq!(
            message,
            "Exported provisioning profile ABC-123 to profiles/exported.mobileprovision (4 bytes)"
        );
    }

    #[test]
    fn dump_json_includes_output_dir_count_and_entries() {
        let value = dump_result_json(
            &PathBuf::from("profiles"),
            &[
                dump_entry_json(
                    "ABC-123",
                    &PathBuf::from("profiles/ABC-123.mobileprovision"),
                    4,
                ),
                dump_entry_json(
                    "DEF-456",
                    &PathBuf::from("profiles/DEF-456.mobileprovision"),
                    8,
                ),
            ],
        );

        assert_eq!(
            value,
            serde_json::json!({
                "output_dir": "profiles",
                "saved_count": 2,
                "entries": [
                    {
                        "uuid": "ABC-123",
                        "output": "profiles/ABC-123.mobileprovision",
                        "bytes": 4,
                    },
                    {
                        "uuid": "DEF-456",
                        "output": "profiles/DEF-456.mobileprovision",
                        "bytes": 8,
                    }
                ]
            })
        );
    }

    #[test]
    fn find_profile_returns_not_found_error_for_unknown_uuid() {
        let profiles = vec![ios_core::services::misagent::Profile {
            uuid: "ABC-123".into(),
            name: "Example Dev Profile".into(),
            app_id: "Example App".into(),
            expiry_date: Some("2026-04-08T00:00:00Z".into()),
            raw_data: vec![1, 2, 3, 4],
        }];

        let error = find_profile(&profiles, "MISSING-UUID").unwrap_err();

        assert_eq!(
            error.to_string(),
            "provisioning profile not found: MISSING-UUID"
        );
    }

    #[test]
    fn find_profile_accepts_unique_uuid_prefix() {
        let profiles = vec![
            ios_core::services::misagent::Profile {
                uuid: "ABC-123".into(),
                name: "First".into(),
                app_id: "App A".into(),
                expiry_date: None,
                raw_data: vec![1],
            },
            ios_core::services::misagent::Profile {
                uuid: "DEF-456".into(),
                name: "Second".into(),
                app_id: "App B".into(),
                expiry_date: None,
                raw_data: vec![2],
            },
        ];

        let profile = find_profile(&profiles, "ABC").expect("prefix should resolve");
        assert_eq!(profile.uuid, "ABC-123");
    }

    #[test]
    fn find_profile_accepts_exact_name() {
        let profiles = vec![ios_core::services::misagent::Profile {
            uuid: "ABC-123".into(),
            name: "Example Dev Profile".into(),
            app_id: "Example App".into(),
            expiry_date: None,
            raw_data: vec![1],
        }];

        let profile = find_profile(&profiles, "Example Dev Profile").expect("name should resolve");
        assert_eq!(profile.uuid, "ABC-123");
    }

    #[test]
    fn find_profile_rejects_ambiguous_uuid_prefix() {
        let profiles = vec![
            ios_core::services::misagent::Profile {
                uuid: "ABC-123".into(),
                name: "First".into(),
                app_id: "App A".into(),
                expiry_date: None,
                raw_data: vec![1],
            },
            ios_core::services::misagent::Profile {
                uuid: "ABC-456".into(),
                name: "Second".into(),
                app_id: "App B".into(),
                expiry_date: None,
                raw_data: vec![2],
            },
        ];

        let error = find_profile(&profiles, "ABC").unwrap_err();
        assert_eq!(
            error.to_string(),
            "multiple provisioning profiles match query: ABC"
        );
    }
}
