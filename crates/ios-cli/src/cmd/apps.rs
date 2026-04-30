use anyhow::{Context, Result};
use ios_core::error::CoreError;

#[derive(clap::Args)]
pub struct AppsCmd {
    #[command(subcommand)]
    sub: AppsSub,
}

#[derive(clap::Subcommand)]
enum AppsSub {
    /// List installed apps
    List {
        #[arg(
            long,
            default_value = "user",
            help = "App type: user, system, hidden, all, file-sharing"
        )]
        app_type: String,
    },
    /// Install an IPA or unpacked .app bundle from the local filesystem
    Install {
        #[arg(help = "Path to the IPA file or unpacked .app directory")]
        ipa_path: String,
        #[arg(
            long,
            help = "Use streaming zip conduit for faster IPA installation (IPA files only)"
        )]
        streaming: bool,
    },
    /// Upgrade an installed app from a staged IPA
    Upgrade {
        #[arg(help = "Path to the IPA file")]
        ipa_path: String,
    },
    /// Archive an installed app by bundle ID
    Archive {
        #[arg(help = "Bundle ID (e.g. com.example.App)")]
        bundle_id: String,
    },
    /// Restore an archived app by bundle ID
    Restore {
        #[arg(help = "Bundle ID (e.g. com.example.App)")]
        bundle_id: String,
    },
    /// Show details for a single installed app
    Show {
        #[arg(help = "Bundle ID (e.g. com.apple.Preferences)")]
        bundle_id: String,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Comma-separated return attributes to request from installation_proxy"
        )]
        attrs: Vec<String>,
    },
    /// List running app processes (iOS 17+ appservice)
    Processes {
        #[arg(long, help = "Only include processes marked as applications")]
        apps: bool,
        #[arg(
            long,
            help = "Only include processes whose name contains this substring, case-insensitive; blank values are ignored"
        )]
        name: Option<String>,
    },
    /// Kill a running process by PID (iOS 17+ appservice)
    Kill {
        #[arg(help = "Process ID")]
        pid: u64,
    },
    /// Send a signal to a process by PID (iOS 17+ appservice)
    Signal {
        #[arg(help = "Process ID")]
        pid: u64,
        #[arg(help = "Signal number (e.g. 9=SIGKILL, 15=SIGTERM, 17=SIGSTOP, 19=SIGCONT)")]
        signal: i64,
    },
    /// Kill processes by name pattern (iOS 17+ appservice)
    Pkill {
        #[arg(help = "Process name pattern (case-insensitive substring match)")]
        pattern: String,
        #[arg(
            long,
            default_value_t = 9,
            help = "Signal to send (default: 9 SIGKILL)"
        )]
        signal: i64,
    },
    /// Launch an app by bundle ID (iOS 17+ appservice)
    Launch {
        #[arg(help = "Bundle ID (e.g. com.example.App)")]
        bundle_id: String,
    },
    /// Uninstall an app by bundle ID
    Uninstall {
        #[arg(help = "Bundle ID (e.g. com.example.App)")]
        bundle_id: String,
    },
}

impl AppsCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for apps commands"))?;

        let skip_tunnel = if apps_subcommand_requires_version_probe(&self.sub) {
            let probe = ios_core::connect(
                &udid,
                ios_core::device::ConnectOptions {
                    tun_mode: ios_core::TunMode::Userspace,
                    pair_record_path: None,
                    skip_tunnel: true,
                },
            )
            .await
            .context("failed to probe device version for apps command")?;
            let product_version = probe.product_version().await?;
            drop(probe);
            !apps_subcommand_prefers_tunnel(&self.sub, product_version.major)
        } else {
            true
        };
        let opts = ios_core::device::ConnectOptions {
            tun_mode: ios_core::TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel,
        };
        let device = ios_core::connect(&udid, opts).await?;

        match self.sub {
            AppsSub::List { app_type } => {
                let stream = device
                    .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
                    .await?;
                let mut proxy = ios_core::apps::InstallationProxy::new(stream);

                let apps = match parse_app_type(&app_type)? {
                    AppType::User => proxy.list_user_apps().await?,
                    AppType::System => proxy.list_system_apps().await?,
                    AppType::Hidden => proxy.list_hidden_apps().await?,
                    AppType::All => proxy.list_all_apps().await?,
                    AppType::FileSharing => proxy.list_file_sharing_apps().await?,
                };

                if json {
                    let list: Vec<_> = apps
                        .iter()
                        .map(|a| {
                            serde_json::json!({
                                "bundle_id":    a.bundle_id,
                                "display_name": a.display_name,
                                "version":      a.version,
                                "app_type":     a.app_type,
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&list)?);
                } else {
                    for a in &apps {
                        println!("{:<45} {} ({})", a.bundle_id, a.display_name, a.version);
                    }
                }
            }
            AppsSub::Install {
                ipa_path,
                streaming,
            } => {
                let source_path = std::path::Path::new(&ipa_path);

                // Use streaming zip conduit for IPA files when --streaming is set
                if streaming {
                    if source_path.extension().and_then(|e| e.to_str()) != Some("ipa") {
                        return Err(anyhow::anyhow!(
                            "--streaming only supports .ipa files, not directories"
                        ));
                    }

                    // Try streaming zip conduit service
                    let svc_name = if skip_tunnel {
                        ios_core::apps::zipconduit::SERVICE_NAME
                    } else {
                        ios_core::apps::zipconduit::RSD_SERVICE_NAME
                    };

                    let mut stream = if skip_tunnel {
                        device
                            .connect_service(svc_name)
                            .await
                            .context("failed to connect streaming_zip_conduit service")?
                    } else {
                        device
                            .connect_rsd_service(svc_name)
                            .await
                            .context("failed to connect streaming_zip_conduit RSD service")?
                    };

                    eprintln!("Installing via streaming zip conduit...");
                    ios_core::apps::install_ipa(
                        &mut stream,
                        source_path,
                        Some(Box::new(|percent, status| {
                            eprintln!("  [{percent}%] {status}");
                        })),
                    )
                    .await
                    .context("streaming zip conduit install failed")?;
                    println!("Installed {ipa_path} (streaming)");
                    return Ok(());
                }

                let install_paths = build_install_paths(source_path)?;

                let afc_stream = device
                    .connect_service("com.apple.afc")
                    .await
                    .context("failed to connect AFC for app staging")?;
                let mut afc = ios_core::afc::AfcClient::new(afc_stream);

                match afc.make_dir("/PublicStaging").await {
                    Ok(()) => {}
                    Err(ios_core::afc::AfcError::Status(
                        ios_core::afc::AfcStatusCode::ObjectExists,
                    )) => {}
                    Err(err) => {
                        return Err(err).context("failed to create /PublicStaging on device");
                    }
                }

                let staged_path = if install_paths.is_directory {
                    remove_staged_path(&mut afc, &install_paths.final_remote_path, true)
                        .await
                        .with_context(|| {
                            format!(
                                "failed to clear staged bundle {}",
                                install_paths.final_remote_path
                            )
                        })?;

                    stage_app_bundle(&mut afc, source_path, &install_paths.final_remote_path)
                        .await
                        .with_context(|| {
                            format!(
                                "failed to upload app bundle to {}",
                                install_paths.final_remote_path
                            )
                        })?;

                    StagedPath {
                        path: install_paths.final_remote_path.clone(),
                        is_directory: true,
                    }
                } else {
                    for stale_path in [
                        &install_paths.temp_remote_path,
                        &install_paths.final_remote_path,
                    ] {
                        remove_staged_path(&mut afc, stale_path, false)
                            .await
                            .with_context(|| format!("failed to clear staged file {stale_path}"))?;
                    }

                    let ipa_bytes = tokio::fs::read(source_path)
                        .await
                        .with_context(|| format!("failed to read IPA file {ipa_path}"))?;
                    afc.write_file(&install_paths.temp_remote_path, &ipa_bytes)
                        .await
                        .context(format!(
                            "failed to upload IPA to {}",
                            install_paths.temp_remote_path
                        ))?;
                    afc.rename(
                        &install_paths.temp_remote_path,
                        &install_paths.final_remote_path,
                    )
                    .await
                    .context(format!(
                        "failed to finalize staged IPA at {}",
                        install_paths.final_remote_path
                    ))?;

                    StagedPath {
                        path: install_paths.final_remote_path.clone(),
                        is_directory: false,
                    }
                };

                let install_result: Result<()> = async {
                    let install_stream = device
                        .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
                        .await
                        .context("failed to connect installation proxy")?;
                    let mut proxy = ios_core::apps::InstallationProxy::new(install_stream);
                    proxy
                        .install(&install_paths.package_path)
                        .await
                        .context(format!(
                            "failed to install staged IPA {}",
                            install_paths.package_path
                        ))?;
                    Ok(())
                }
                .await;

                let cleanup_result =
                    remove_staged_path(&mut afc, &staged_path.path, staged_path.is_directory).await;

                if let Err(err) = install_result {
                    if let Err(cleanup_err) = cleanup_result {
                        return Err(err.context(format!(
                            "failed to clean staged install payload after install error: {cleanup_err}"
                        )));
                    }
                    return Err(err);
                }

                cleanup_result.context("installed app but failed to remove staged payload")?;
                println!("Installed {ipa_path}");
            }
            AppsSub::Upgrade { ipa_path } => {
                let source_path = std::path::Path::new(&ipa_path);
                let install_paths = build_install_paths(source_path)?;
                if install_paths.is_directory {
                    return Err(anyhow::anyhow!(
                        "apps upgrade currently supports IPA input only: {}",
                        source_path.display()
                    ));
                }

                let afc_stream = device
                    .connect_service("com.apple.afc")
                    .await
                    .context("failed to connect AFC for app staging")?;
                let mut afc = ios_core::afc::AfcClient::new(afc_stream);

                match afc.make_dir("/PublicStaging").await {
                    Ok(()) => {}
                    Err(ios_core::afc::AfcError::Status(
                        ios_core::afc::AfcStatusCode::ObjectExists,
                    )) => {}
                    Err(err) => {
                        return Err(err).context("failed to create /PublicStaging on device");
                    }
                }

                for stale_path in [
                    &install_paths.temp_remote_path,
                    &install_paths.final_remote_path,
                ] {
                    remove_staged_path(&mut afc, stale_path, false)
                        .await
                        .with_context(|| format!("failed to clear staged file {stale_path}"))?;
                }

                let ipa_bytes = tokio::fs::read(source_path)
                    .await
                    .with_context(|| format!("failed to read IPA file {ipa_path}"))?;
                afc.write_file(&install_paths.temp_remote_path, &ipa_bytes)
                    .await
                    .context(format!(
                        "failed to upload IPA to {}",
                        install_paths.temp_remote_path
                    ))?;
                afc.rename(
                    &install_paths.temp_remote_path,
                    &install_paths.final_remote_path,
                )
                .await
                .context(format!(
                    "failed to finalize staged IPA at {}",
                    install_paths.final_remote_path
                ))?;

                let upgrade_result: Result<()> = async {
                    let install_stream = device
                        .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
                        .await
                        .context("failed to connect installation proxy")?;
                    let mut proxy = ios_core::apps::InstallationProxy::new(install_stream);
                    proxy
                        .upgrade(&install_paths.package_path)
                        .await
                        .context(format!(
                            "failed to upgrade staged IPA {}",
                            install_paths.package_path
                        ))?;
                    Ok(())
                }
                .await;

                let cleanup_result =
                    remove_staged_path(&mut afc, &install_paths.final_remote_path, false).await;

                if let Err(err) = upgrade_result {
                    if let Err(cleanup_err) = cleanup_result {
                        return Err(err.context(format!(
                            "failed to clean staged upgrade payload after upgrade error: {cleanup_err}"
                        )));
                    }
                    return Err(err);
                }

                cleanup_result.context("upgraded app but failed to remove staged payload")?;
                println!("Upgraded {ipa_path}");
            }
            AppsSub::Archive { bundle_id } => {
                let stream = device
                    .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
                    .await?;
                let mut proxy = ios_core::apps::InstallationProxy::new(stream);
                proxy.archive(&bundle_id).await?;
                println!("Archived {bundle_id}");
            }
            AppsSub::Restore { bundle_id } => {
                let stream = device
                    .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
                    .await?;
                let mut proxy = ios_core::apps::InstallationProxy::new(stream);
                proxy.restore(&bundle_id).await?;
                println!("Restored {bundle_id}");
            }
            AppsSub::Show { bundle_id, attrs } => {
                let stream = device
                    .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
                    .await?;
                let mut proxy = ios_core::apps::InstallationProxy::new(stream);
                let attr_refs: Vec<&str> = attrs.iter().map(String::as_str).collect();
                let app = proxy
                    .lookup_app_with_attributes(&bundle_id, &attr_refs)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("app not found: {bundle_id}"))?;
                let app_json = app_to_json_with_attrs(&app, &attrs);

                if json {
                    println!("{}", serde_json::to_string_pretty(&app_json)?);
                } else {
                    print_app_details(&app_json);
                }
            }
            AppsSub::Processes { apps, name } => {
                let processes = match connect_appservice(&device, &udid).await {
                    Ok(mut client) => client.list_processes().await?,
                    Err(e) if should_fallback_to_instruments(&e) => {
                        let (_device, stream) =
                            super::instruments::connect_instruments(&udid).await?;
                        let mut di = ios_core::instruments::DeviceInfoClient::connect(stream)
                            .await
                            .map_err(|err| anyhow::anyhow!("DTX error: {err}"))?;
                        di.running_processes()
                            .await
                            .map_err(|err| anyhow::anyhow!("runningProcesses error: {err}"))?
                            .into_iter()
                            .map(|p| ios_core::apps::RunningAppProcess {
                                pid: p.pid,
                                bundle_id: None,
                                name: p.name,
                                executable: Some(p.real_app_name),
                                is_application: Some(p.is_application),
                            })
                            .collect()
                    }
                    Err(e) => return Err(e.into()),
                };
                let processes = filter_running_processes(processes, apps);
                let processes = filter_running_processes_by_name(processes, name.as_deref());

                if json {
                    let list: Vec<_> = processes
                        .iter()
                        .map(|p| {
                            serde_json::json!({
                                "pid": p.pid,
                                "bundle_id": p.bundle_id,
                                "name": p.name,
                                "executable": p.executable,
                                "is_application": p.is_application,
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&list)?);
                } else {
                    for process in &processes {
                        let bundle = process.bundle_id.as_deref().unwrap_or("-");
                        println!("{:<8} {:<45} {}", process.pid, bundle, process.name);
                    }
                }
            }
            AppsSub::Kill { pid } => {
                match connect_appservice(&device, &udid).await {
                    Ok(mut client) => client.kill_process(pid).await?,
                    Err(e) if should_fallback_to_instruments(&e) => {
                        let (_device, stream) =
                            super::instruments::connect_instruments(&udid).await?;
                        let mut pc =
                            ios_core::instruments::process_control::ProcessControl::connect(stream)
                                .await
                                .map_err(|err| anyhow::anyhow!("DTX error: {err}"))?;
                        pc.kill(pid)
                            .await
                            .map_err(|err| anyhow::anyhow!("kill error: {err}"))?;
                    }
                    Err(e) => return Err(e.into()),
                }
                println!("Killed process {pid}");
            }
            AppsSub::Signal { pid, signal } => match connect_appservice(&device, &udid).await {
                Ok(mut client) => {
                    client.send_signal(pid, signal).await?;
                    println!("Sent signal {signal} to process {pid}");
                }
                Err(e) if should_fallback_to_instruments(&e) => {
                    return Err(anyhow::anyhow!(
                            "arbitrary signal sending requires iOS 17+ appservice (not available on this device); \
                             use 'apps kill' for SIGKILL which supports DTX fallback"
                        ));
                }
                Err(e) => return Err(e.into()),
            },
            AppsSub::Pkill { pattern, signal } => {
                let processes = match connect_appservice(&device, &udid).await {
                    Ok(mut client) => client.list_processes().await?,
                    Err(e) if should_fallback_to_instruments(&e) => {
                        let (_device, stream) =
                            super::instruments::connect_instruments(&udid).await?;
                        let mut di = ios_core::instruments::DeviceInfoClient::connect(stream)
                            .await
                            .map_err(|err| anyhow::anyhow!("DTX error: {err}"))?;
                        di.running_processes()
                            .await
                            .map_err(|err| anyhow::anyhow!("runningProcesses error: {err}"))?
                            .into_iter()
                            .map(|p| ios_core::apps::RunningAppProcess {
                                pid: p.pid,
                                bundle_id: None,
                                name: p.name,
                                executable: Some(p.real_app_name),
                                is_application: Some(p.is_application),
                            })
                            .collect()
                    }
                    Err(e) => return Err(e.into()),
                };

                let pattern_lower = pattern.to_ascii_lowercase();
                let matched: Vec<_> = processes
                    .iter()
                    .filter(|p| p.name.to_ascii_lowercase().contains(&pattern_lower))
                    .collect();

                if matched.is_empty() {
                    println!("No processes matched pattern '{pattern}'");
                } else {
                    for p in &matched {
                        match connect_appservice(&device, &udid).await {
                            Ok(mut client) => client.send_signal(p.pid, signal).await?,
                            Err(e) if should_fallback_to_instruments(&e) => {
                                let (_device, stream) =
                                    super::instruments::connect_instruments(&udid).await?;
                                let mut pc = ios_core::instruments::process_control::ProcessControl::connect(stream)
                                    .await
                                    .map_err(|err| anyhow::anyhow!("DTX error: {err}"))?;
                                pc.kill(p.pid)
                                    .await
                                    .map_err(|err| anyhow::anyhow!("kill error: {err}"))?;
                            }
                            Err(e) => return Err(e.into()),
                        }
                        println!("Sent signal {signal} to {} (pid {})", p.name, p.pid);
                    }
                    println!("Killed {} process(es)", matched.len());
                }
            }
            AppsSub::Launch { bundle_id } => {
                let pid = match connect_appservice(&device, &udid).await {
                    Ok(mut client) => client.launch_application(&bundle_id).await?,
                    Err(e) if should_fallback_to_instruments(&e) => {
                        use std::collections::HashMap;

                        let (_device, stream) =
                            super::instruments::connect_instruments(&udid).await?;
                        let mut pc =
                            ios_core::instruments::process_control::ProcessControl::connect(stream)
                                .await
                                .map_err(|err| anyhow::anyhow!("DTX error: {err}"))?;
                        let env = HashMap::new();
                        Some(
                            pc.launch(&bundle_id, &[], &env)
                                .await
                                .map_err(|err| anyhow::anyhow!("launch error: {err}"))?,
                        )
                    }
                    Err(e) => return Err(e.into()),
                };
                if let Some(pid) = pid {
                    println!("Launched {bundle_id} (pid {pid})");
                } else {
                    println!("Launched {bundle_id}");
                }
            }
            AppsSub::Uninstall { bundle_id } => {
                let stream = device
                    .connect_service(ios_core::apps::INSTALLATION_PROXY_SERVICE)
                    .await?;
                let mut proxy = ios_core::apps::InstallationProxy::new(stream);
                proxy.uninstall(&bundle_id).await?;
                println!("Uninstalled {bundle_id}");
            }
        }
        Ok(())
    }
}

fn apps_subcommand_requires_version_probe(sub: &AppsSub) -> bool {
    matches!(
        sub,
        AppsSub::Processes { .. }
            | AppsSub::Kill { .. }
            | AppsSub::Signal { .. }
            | AppsSub::Pkill { .. }
            | AppsSub::Launch { .. }
    )
}

fn apps_subcommand_prefers_tunnel(sub: &AppsSub, ios_major: u64) -> bool {
    ios_major >= 17
        && matches!(
            sub,
            AppsSub::Processes { .. }
                | AppsSub::Kill { .. }
                | AppsSub::Signal { .. }
                | AppsSub::Pkill { .. }
                | AppsSub::Launch { .. }
        )
}

async fn connect_appservice(
    device: &ios_core::ConnectedDevice,
    udid: &str,
) -> Result<ios_core::apps::AppServiceClient, CoreError> {
    let xpc = device
        .connect_xpc_service(ios_core::apps::APPSERVICE_SERVICE)
        .await?;
    Ok(ios_core::apps::AppServiceClient::new(xpc, udid.to_string()))
}

fn should_fallback_to_instruments(err: &CoreError) -> bool {
    match err {
        CoreError::Unsupported(message) => {
            message.contains("service 'com.apple.coredevice.appservice' not found")
                || message.contains("RSD not available")
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppType {
    User,
    System,
    Hidden,
    All,
    FileSharing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstallPaths {
    temp_remote_path: String,
    final_remote_path: String,
    package_path: String,
    is_directory: bool,
}

fn build_install_paths(path: &std::path::Path) -> Result<InstallPaths> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "install path must include a file or bundle name: {}",
                path.display()
            )
        })?;
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to inspect install path {}", path.display()))?;
    let is_directory = metadata.is_dir();

    if is_directory {
        if !file_name.ends_with(".app") {
            return Err(anyhow::anyhow!(
                "app directory installs require a .app bundle: {}",
                path.display()
            ));
        }
    } else if !metadata.is_file() || !file_name.ends_with(".ipa") {
        return Err(anyhow::anyhow!(
            "install path must be an .ipa file or unpacked .app directory: {}",
            path.display()
        ));
    }

    Ok(InstallPaths {
        temp_remote_path: format!("/PublicStaging/{file_name}.upload"),
        final_remote_path: format!("/PublicStaging/{file_name}"),
        package_path: format!("/PublicStaging/{file_name}"),
        is_directory,
    })
}

async fn remove_staged_path<S>(
    afc: &mut ios_core::afc::AfcClient<S>,
    path: &str,
    is_directory: bool,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if !is_directory {
        return match afc.remove(path).await {
            Ok(()) => Ok(()),
            Err(ios_core::afc::AfcError::Status(ios_core::afc::AfcStatusCode::ObjectNotFound)) => {
                Ok(())
            }
            Err(err) => Err(err.into()),
        };
    }

    for _ in 0..5 {
        match afc.remove_all(path).await {
            Ok(()) => {}
            Err(ios_core::afc::AfcError::Status(ios_core::afc::AfcStatusCode::ObjectNotFound)) => {
                return Ok(())
            }
            Err(err) => return Err(err.into()),
        }

        match afc.remove(path).await {
            Ok(()) => return Ok(()),
            Err(ios_core::afc::AfcError::Status(ios_core::afc::AfcStatusCode::ObjectNotFound)) => {
                return Ok(())
            }
            Err(ios_core::afc::AfcError::Status(
                ios_core::afc::AfcStatusCode::DirNotEmpty
                | ios_core::afc::AfcStatusCode::ObjectBusy
                | ios_core::afc::AfcStatusCode::OpWouldBlock,
            )) => {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            Err(err) => return Err(err.into()),
        }

        match afc.stat(path).await {
            Err(ios_core::afc::AfcError::Status(ios_core::afc::AfcStatusCode::ObjectNotFound)) => {
                return Ok(())
            }
            Ok(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
            Err(err) => return Err(err.into()),
        }
    }

    Err(anyhow::anyhow!(
        "staged app bundle still exists after recursive cleanup: {path}"
    ))
}

struct StagedPath {
    path: String,
    is_directory: bool,
}

async fn stage_app_bundle<S>(
    afc: &mut ios_core::afc::AfcClient<S>,
    local_root: &std::path::Path,
    remote_root: &str,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    ensure_remote_dir(afc, remote_root).await?;

    let mut stack = vec![(local_root.to_path_buf(), remote_root.to_string())];
    while let Some((local_dir, remote_dir)) = stack.pop() {
        ensure_remote_dir(afc, &remote_dir).await?;

        let mut entries = tokio::fs::read_dir(&local_dir).await.with_context(|| {
            format!(
                "failed to read app bundle directory {}",
                local_dir.display()
            )
        })?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("failed to enumerate {}", local_dir.display()))?
        {
            let local_path = entry.path();
            let file_type = entry
                .file_type()
                .await
                .with_context(|| format!("failed to inspect {}", local_path.display()))?;
            let entry_name = entry.file_name();
            let remote_path = join_remote_path(&remote_dir, &entry_name)?;

            if file_type.is_dir() {
                stack.push((local_path, remote_path));
                continue;
            }

            if file_type.is_file() {
                let data = tokio::fs::read(&local_path)
                    .await
                    .with_context(|| format!("failed to read {}", local_path.display()))?;
                afc.write_file(&remote_path, &data)
                    .await
                    .with_context(|| format!("failed to upload {}", remote_path))?;
                continue;
            }

            if file_type.is_symlink() {
                return Err(anyhow::anyhow!(
                    "symbolic links are not supported inside app bundles: {}",
                    local_path.display()
                ));
            }

            return Err(anyhow::anyhow!(
                "unsupported app bundle entry type: {}",
                local_path.display()
            ));
        }
    }

    Ok(())
}

async fn ensure_remote_dir<S>(afc: &mut ios_core::afc::AfcClient<S>, path: &str) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match afc.make_dir(path).await {
        Ok(()) => Ok(()),
        Err(ios_core::afc::AfcError::Status(ios_core::afc::AfcStatusCode::ObjectExists)) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn join_remote_path(base: &str, name: &std::ffi::OsStr) -> Result<String> {
    let name = name.to_string_lossy();
    if name.is_empty() {
        return Err(anyhow::anyhow!("app bundle entry name cannot be empty"));
    }
    Ok(format!("{base}/{name}"))
}

fn parse_app_type(input: &str) -> Result<AppType> {
    match input.trim().to_ascii_lowercase().as_str() {
        "user" => Ok(AppType::User),
        "system" => Ok(AppType::System),
        "hidden" => Ok(AppType::Hidden),
        "all" => Ok(AppType::All),
        "file-sharing" | "filesharing" => Ok(AppType::FileSharing),
        other => Err(anyhow::anyhow!(
            "unsupported app type '{other}', expected one of: user, system, hidden, all, file-sharing"
        )),
    }
}

fn app_to_json(app: &ios_core::apps::AppInfo) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "bundle_id".to_string(),
        serde_json::Value::String(app.bundle_id.clone()),
    );
    obj.insert(
        "display_name".to_string(),
        serde_json::Value::String(app.display_name.clone()),
    );
    obj.insert(
        "version".to_string(),
        serde_json::Value::String(app.version.clone()),
    );
    obj.insert(
        "app_type".to_string(),
        serde_json::Value::String(app.app_type.clone()),
    );
    obj.insert(
        "path".to_string(),
        serde_json::Value::String(app.path.clone()),
    );

    for (key, value) in &app.extra {
        obj.entry(key.clone())
            .or_insert_with(|| plist_to_json(value));
    }

    serde_json::Value::Object(obj)
}

fn app_to_json_with_attrs(app: &ios_core::apps::AppInfo, attrs: &[String]) -> serde_json::Value {
    let json = app_to_json(app);
    if attrs.is_empty() {
        return json;
    }

    let Some(obj) = json.as_object() else {
        return json;
    };

    let mut filtered = serde_json::Map::new();
    filtered.insert(
        "bundle_id".to_string(),
        serde_json::Value::String(app.bundle_id.clone()),
    );
    for attr in attrs {
        if let Some(value) = obj.get(attr) {
            filtered.insert(attr.clone(), value.clone());
        }
    }
    serde_json::Value::Object(filtered)
}

fn plist_to_json(value: &plist::Value) -> serde_json::Value {
    match value {
        plist::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(plist_to_json).collect())
        }
        plist::Value::Boolean(v) => serde_json::Value::Bool(*v),
        plist::Value::Data(bytes) => serde_json::Value::Array(
            bytes
                .iter()
                .map(|byte| serde_json::Value::from(*byte))
                .collect(),
        ),
        plist::Value::Date(date) => serde_json::Value::String(date.to_xml_format()),
        plist::Value::Dictionary(dict) => serde_json::Value::Object(
            dict.iter()
                .map(|(k, v)| (k.clone(), plist_to_json(v)))
                .collect(),
        ),
        plist::Value::Integer(n) => {
            if let Some(i) = n.as_signed() {
                serde_json::Value::from(i)
            } else if let Some(u) = n.as_unsigned() {
                serde_json::Value::from(u)
            } else {
                serde_json::Value::Null
            }
        }
        plist::Value::Real(v) => serde_json::Number::from_f64(*v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        plist::Value::String(s) => serde_json::Value::String(s.clone()),
        plist::Value::Uid(uid) => serde_json::Value::from(uid.get()),
        _ => serde_json::Value::Null,
    }
}

fn print_app_details(json: &serde_json::Value) {
    let mut rows: Vec<(String, String)> = json
        .as_object()
        .into_iter()
        .flat_map(|obj| obj.iter())
        .map(|(key, value)| (key.clone(), format_json_value(value)))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in rows {
        println!("{key}: {value}");
    }
}

fn format_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(v) => v.to_string(),
        serde_json::Value::Number(v) => v.to_string(),
        serde_json::Value::String(v) => v.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn filter_running_processes(
    processes: Vec<ios_core::apps::RunningAppProcess>,
    apps_only: bool,
) -> Vec<ios_core::apps::RunningAppProcess> {
    if !apps_only {
        return processes;
    }

    processes
        .into_iter()
        .filter(|process| process.is_application == Some(true))
        .collect()
}

fn filter_running_processes_by_name(
    processes: Vec<ios_core::apps::RunningAppProcess>,
    name: Option<&str>,
) -> Vec<ios_core::apps::RunningAppProcess> {
    let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
        return processes;
    };
    let name = name.to_ascii_lowercase();

    processes
        .into_iter()
        .filter(|process| process.name.to_ascii_lowercase().contains(&name))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use clap::Parser;

    use super::*;

    fn unique_test_temp_dir() -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ios-apps-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        if path.exists() {
            remove_test_dir(&path);
        }
        path
    }

    fn remove_test_dir(path: &Path) {
        for _ in 0..5 {
            match std::fs::remove_dir_all(path) {
                Ok(()) => return,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
                Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(err) => panic!("failed to remove {}: {err}", path.display()),
            }
        }

        panic!("failed to remove {} after retries", path.display());
    }

    #[test]
    fn parse_app_type_distinguishes_system_and_all() {
        assert_eq!(parse_app_type("system").unwrap(), AppType::System);
        assert_eq!(parse_app_type("hidden").unwrap(), AppType::Hidden);
        assert_eq!(parse_app_type("all").unwrap(), AppType::All);
        assert_eq!(parse_app_type("user").unwrap(), AppType::User);
        assert_eq!(
            parse_app_type("file-sharing").unwrap(),
            AppType::FileSharing
        );
        assert_eq!(parse_app_type("filesharing").unwrap(), AppType::FileSharing);
    }

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: AppsSub,
    }

    #[test]
    fn parses_apps_show_subcommand() {
        let cmd = TestCli::parse_from(["apps", "show", "com.apple.Preferences"]);
        match cmd.command {
            AppsSub::Show { bundle_id, attrs } => {
                assert_eq!(bundle_id, "com.apple.Preferences");
                assert!(attrs.is_empty());
            }
            _ => panic!("expected show subcommand"),
        }
    }

    #[test]
    fn parses_apps_install_subcommand() {
        let cmd = TestCli::parse_from(["apps", "install", r"C:\tmp\Example.ipa"]);
        match cmd.command {
            AppsSub::Install {
                ipa_path,
                streaming,
            } => {
                assert_eq!(ipa_path, r"C:\tmp\Example.ipa");
                assert!(!streaming);
            }
            _ => panic!("expected install subcommand"),
        }
    }

    #[test]
    fn parses_apps_install_streaming_flag() {
        let cmd = TestCli::parse_from(["apps", "install", "--streaming", r"C:\tmp\Example.ipa"]);
        match cmd.command {
            AppsSub::Install {
                ipa_path,
                streaming,
            } => {
                assert_eq!(ipa_path, r"C:\tmp\Example.ipa");
                assert!(streaming);
            }
            _ => panic!("expected install subcommand"),
        }
    }

    #[test]
    fn parses_apps_upgrade_subcommand() {
        let cmd = TestCli::parse_from(["apps", "upgrade", r"C:\tmp\Example.ipa"]);
        match cmd.command {
            AppsSub::Upgrade { ipa_path } => assert_eq!(ipa_path, r"C:\tmp\Example.ipa"),
            _ => panic!("expected upgrade subcommand"),
        }
    }

    #[test]
    fn parses_apps_archive_subcommand() {
        let parsed = TestCli::try_parse_from(["apps", "archive", "com.example.app"]);
        assert!(parsed.is_ok(), "apps archive subcommand should parse");
    }

    #[test]
    fn parses_apps_restore_subcommand() {
        let parsed = TestCli::try_parse_from(["apps", "restore", "com.example.app"]);
        assert!(parsed.is_ok(), "apps restore subcommand should parse");
    }

    #[test]
    fn parses_apps_show_attrs_flag() {
        let parsed = TestCli::try_parse_from([
            "apps",
            "show",
            "com.example.app",
            "--attrs",
            "CFBundleVersion,Path",
        ]);
        assert!(parsed.is_ok(), "apps show --attrs command should parse");
    }

    #[test]
    fn build_install_paths_uses_public_staging() {
        let temp_dir = unique_test_temp_dir();
        std::fs::create_dir_all(&temp_dir).unwrap();
        let ipa_path = temp_dir.join("Example.ipa");
        std::fs::write(&ipa_path, b"ipa").unwrap();

        let paths = build_install_paths(&ipa_path).unwrap();
        assert_eq!(paths.temp_remote_path, "/PublicStaging/Example.ipa.upload");
        assert_eq!(paths.final_remote_path, "/PublicStaging/Example.ipa");
        assert_eq!(paths.package_path, "/PublicStaging/Example.ipa");
        assert!(!paths.is_directory);

        remove_test_dir(&temp_dir);
    }

    #[test]
    fn build_install_paths_uses_bundle_name_for_directories() {
        let temp_dir = unique_test_temp_dir();
        let app_dir = temp_dir.join("Example.app");
        std::fs::create_dir_all(&app_dir).unwrap();

        let paths = build_install_paths(&app_dir).unwrap();
        assert_eq!(paths.final_remote_path, "/PublicStaging/Example.app");
        assert_eq!(paths.package_path, paths.final_remote_path);
        assert!(paths.is_directory);

        remove_test_dir(&temp_dir);
    }

    #[test]
    fn build_install_paths_rejects_non_installable_inputs() {
        let temp_dir = unique_test_temp_dir();
        std::fs::create_dir_all(&temp_dir).unwrap();
        let plain_dir = temp_dir.join("NotAnApp");
        std::fs::create_dir_all(&plain_dir).unwrap();
        let plain_file = temp_dir.join("NotAnIpa.zip");
        std::fs::write(&plain_file, b"zip").unwrap();

        let dir_err = build_install_paths(&plain_dir).unwrap_err().to_string();
        assert!(dir_err.contains(".app bundle"));

        let file_err = build_install_paths(&plain_file).unwrap_err().to_string();
        assert!(file_err.contains(".ipa file"));

        remove_test_dir(&temp_dir);
    }

    #[test]
    fn parses_apps_processes_apps_flag() {
        let cmd = TestCli::parse_from(["apps", "processes", "--apps"]);
        match cmd.command {
            AppsSub::Processes { apps, name } => {
                assert!(apps);
                assert_eq!(name, None);
            }
            _ => panic!("expected processes subcommand"),
        }
    }

    #[test]
    fn parses_apps_processes_name_flag() {
        let cmd = TestCli::parse_from(["apps", "processes", "--name", "Phone"]);
        match cmd.command {
            AppsSub::Processes { apps, name } => {
                assert!(!apps);
                assert_eq!(name.as_deref(), Some("Phone"));
            }
            _ => panic!("expected processes subcommand"),
        }
    }

    #[test]
    fn filter_running_processes_keeps_only_apps_when_requested() {
        let processes = vec![
            ios_core::apps::RunningAppProcess {
                pid: 1,
                bundle_id: Some("com.apple.mobilephone".into()),
                name: "Phone".into(),
                executable: Some("MobilePhone".into()),
                is_application: Some(true),
            },
            ios_core::apps::RunningAppProcess {
                pid: 2,
                bundle_id: None,
                name: "mediaserverd".into(),
                executable: Some("mediaserverd".into()),
                is_application: Some(false),
            },
            ios_core::apps::RunningAppProcess {
                pid: 3,
                bundle_id: None,
                name: "unknown".into(),
                executable: None,
                is_application: None,
            },
        ];

        let filtered = filter_running_processes(processes, true);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].pid, 1);
    }

    #[test]
    fn filter_running_processes_by_name_uses_substring_matching() {
        let processes = vec![
            ios_core::apps::RunningAppProcess {
                pid: 1,
                bundle_id: Some("com.apple.mobilephone".into()),
                name: "Phone".into(),
                executable: Some("MobilePhone".into()),
                is_application: Some(true),
            },
            ios_core::apps::RunningAppProcess {
                pid: 2,
                bundle_id: Some("com.apple.Preferences".into()),
                name: "Settings".into(),
                executable: Some("Settings".into()),
                is_application: Some(true),
            },
            ios_core::apps::RunningAppProcess {
                pid: 3,
                bundle_id: None,
                name: "mediaremoted".into(),
                executable: Some("mediaremoted".into()),
                is_application: Some(false),
            },
        ];

        let filtered = filter_running_processes_by_name(processes, Some("Phone"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].pid, 1);
    }

    #[test]
    fn filter_running_processes_by_name_is_case_insensitive() {
        let processes = vec![
            ios_core::apps::RunningAppProcess {
                pid: 1,
                bundle_id: Some("com.apple.mobilephone".into()),
                name: "Phone".into(),
                executable: Some("MobilePhone".into()),
                is_application: Some(true),
            },
            ios_core::apps::RunningAppProcess {
                pid: 2,
                bundle_id: Some("com.apple.Preferences".into()),
                name: "Settings".into(),
                executable: Some("Settings".into()),
                is_application: Some(true),
            },
        ];

        let filtered = filter_running_processes_by_name(processes, Some("phone"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].pid, 1);
    }

    #[test]
    fn filter_running_processes_by_name_ignores_empty_filter() {
        let processes = vec![
            ios_core::apps::RunningAppProcess {
                pid: 1,
                bundle_id: Some("com.apple.mobilephone".into()),
                name: "Phone".into(),
                executable: Some("MobilePhone".into()),
                is_application: Some(true),
            },
            ios_core::apps::RunningAppProcess {
                pid: 2,
                bundle_id: Some("com.apple.Preferences".into()),
                name: "Settings".into(),
                executable: Some("Settings".into()),
                is_application: Some(false),
            },
        ];

        let filtered = filter_running_processes_by_name(processes.clone(), Some(""));
        assert_eq!(filtered, processes);
        assert_eq!(filter_running_processes_by_name(processes, None).len(), 2);
    }

    #[test]
    fn filter_running_processes_applies_name_after_apps_only() {
        let processes = vec![
            ios_core::apps::RunningAppProcess {
                pid: 1,
                bundle_id: Some("com.apple.mobilephone".into()),
                name: "Phone".into(),
                executable: Some("MobilePhone".into()),
                is_application: Some(true),
            },
            ios_core::apps::RunningAppProcess {
                pid: 2,
                bundle_id: None,
                name: "Phone Helper".into(),
                executable: Some("PhoneHelper".into()),
                is_application: Some(false),
            },
            ios_core::apps::RunningAppProcess {
                pid: 3,
                bundle_id: Some("com.apple.Preferences".into()),
                name: "Settings".into(),
                executable: Some("Settings".into()),
                is_application: Some(true),
            },
        ];

        let filtered = filter_running_processes_by_name(
            filter_running_processes(processes, true),
            Some("phone"),
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].pid, 1);
    }

    #[test]
    fn tunnel_probe_targets_only_runtime_process_commands() {
        assert!(apps_subcommand_requires_version_probe(
            &AppsSub::Processes {
                apps: false,
                name: None,
            }
        ));
        assert!(apps_subcommand_requires_version_probe(&AppsSub::Kill {
            pid: 1
        }));
        assert!(apps_subcommand_requires_version_probe(&AppsSub::Signal {
            pid: 1,
            signal: 15,
        }));
        assert!(apps_subcommand_requires_version_probe(&AppsSub::Pkill {
            pattern: "test".into(),
            signal: 9,
        }));
        assert!(apps_subcommand_requires_version_probe(&AppsSub::Launch {
            bundle_id: "com.example.app".into(),
        }));
        assert!(!apps_subcommand_requires_version_probe(&AppsSub::List {
            app_type: "user".into(),
        }));
    }

    #[test]
    fn tunnel_preference_is_ios17_plus_only() {
        let launch = AppsSub::Launch {
            bundle_id: "com.example.app".into(),
        };
        let list = AppsSub::List {
            app_type: "user".into(),
        };

        assert!(apps_subcommand_prefers_tunnel(&launch, 17));
        assert!(apps_subcommand_prefers_tunnel(&launch, 26));
        assert!(!apps_subcommand_prefers_tunnel(&launch, 15));
        assert!(!apps_subcommand_prefers_tunnel(&list, 26));
    }

    #[test]
    fn instruments_fallback_accepts_non_rsd_devices() {
        assert!(should_fallback_to_instruments(&CoreError::Unsupported(
            "RSD not available (no tunnel or iOS <17)".into()
        )));
        assert!(should_fallback_to_instruments(&CoreError::Unsupported(
            "service 'com.apple.coredevice.appservice' not found".into()
        )));
    }
}
