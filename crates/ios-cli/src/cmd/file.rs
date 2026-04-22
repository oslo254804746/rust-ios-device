use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use ios_services::crashreport::{
    prepare_reports, CRASHREPORT_COPY_MOBILE_SERVICE, CRASHREPORT_MOVER_SERVICE,
};
use serde::Serialize;
use serde_json::json;
use tokio::fs;

#[derive(clap::Args)]
pub struct FileCmd {
    #[arg(
        long,
        conflicts_with = "crash",
        help = "Open the app container via House Arrest instead of com.apple.afc"
    )]
    app: Option<String>,
    #[arg(
        long,
        requires = "app",
        conflicts_with = "crash",
        help = "Use House Arrest VendDocuments instead of VendContainer"
    )]
    documents: bool,
    #[arg(
        long,
        conflicts_with = "app",
        help = "Open crash logs via crashreportcopymobile instead of com.apple.afc"
    )]
    crash: bool,
    #[command(subcommand)]
    sub: FileSub,
}

#[derive(clap::Subcommand)]
enum FileSub {
    /// List directory contents
    Ls {
        #[arg(default_value = "/")]
        path: String,
        #[arg(short, long, help = "Show detailed info")]
        long: bool,
    },
    /// Download a file from the device
    Pull {
        #[arg(help = "Remote path on device")]
        remote: String,
        #[arg(help = "Local destination path")]
        local: String,
    },
    /// Upload a file to the device
    Push {
        #[arg(help = "Local source path")]
        local: String,
        #[arg(help = "Remote destination path on device")]
        remote: String,
    },
    /// Remove a file or directory from the device
    Rm {
        #[arg(help = "Remote path to remove")]
        path: String,
        #[arg(short, long, help = "Remove directory and contents recursively")]
        recursive: bool,
    },
    /// Create a directory on the device
    Mkdir { path: String },
    /// Rename or move a file or directory on the device
    Mv {
        #[arg(help = "Existing remote path")]
        from: String,
        #[arg(help = "New remote path")]
        to: String,
    },
    /// Show AFC filesystem/device metadata
    DeviceInfo,
    /// Show file/directory info
    Stat { path: String },
    /// Show a recursive tree view
    Tree {
        #[arg(default_value = "/")]
        path: String,
    },
}

impl FileCmd {
    pub async fn run(self, udid: Option<String>, json_output: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for file commands"))?;

        let opts = ios_core::device::ConnectOptions {
            tun_mode: ios_tunnel::TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let device = ios_core::connect(&udid, opts).await?;
        let mut afc = if self.crash {
            let mut mover = device.connect_service(CRASHREPORT_MOVER_SERVICE).await?;
            prepare_reports(&mut mover).await?;
            let stream = device
                .connect_service(CRASHREPORT_COPY_MOBILE_SERVICE)
                .await?;
            ios_services::afc::AfcClient::new(stream)
        } else if let Some(bundle_id) = self.app.as_deref() {
            let stream = device
                .connect_service(ios_services::afc::house_arrest::SERVICE_NAME)
                .await?;
            let house_arrest = ios_services::afc::house_arrest::HouseArrestClient::new(stream);
            if self.documents {
                house_arrest.vend_documents(bundle_id).await?
            } else {
                house_arrest.vend_container(bundle_id).await?
            }
        } else {
            let stream = device.connect_service("com.apple.afc").await?;
            ios_services::afc::AfcClient::new(stream)
        };

        match self.sub {
            FileSub::Ls { path, long } => {
                let entries = afc.list_dir(&path).await?;
                if json_output {
                    if long {
                        let mut list = Vec::with_capacity(entries.len());
                        for name in &entries {
                            let full = join_device_path(&path, name);
                            match afc.stat_info(&full).await {
                                Ok(info) => list.push(file_info_to_json(name, &full, Some(&info))),
                                Err(_) => list.push(file_info_to_json(name, &full, None)),
                            }
                        }
                        println!("{}", serde_json::to_string_pretty(&list)?);
                    } else {
                        let list: Vec<_> = entries
                            .iter()
                            .map(|name| json!({ "name": name, "path": join_device_path(&path, name) }))
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&list)?);
                    }
                } else if long {
                    for name in &entries {
                        let full = join_device_path(&path, name);
                        match afc.stat_info(&full).await {
                            Ok(info) => {
                                let kind = info.file_type.as_deref().unwrap_or("?");
                                let size = info.size.map(|n| n.to_string()).unwrap_or_default();
                                println!("{:<10} {:>12}  {name}", kind, size);
                            }
                            Err(_) => println!("?              {name}"),
                        }
                    }
                } else {
                    for name in &entries {
                        println!("{name}");
                    }
                }
            }
            FileSub::Pull { remote, local } => {
                eprintln!("Downloading {remote} → {local}");
                let data = afc.read_file_follow_links(&remote).await?;
                fs::write(&local, &data).await?;
                eprintln!("Done ({} bytes)", data.len());
            }
            FileSub::Push { local, remote } => {
                eprintln!("Uploading {local} → {remote}");
                let data = fs::read(&local).await?;
                let len = data.len();
                afc.write_file(&remote, &data).await?;
                eprintln!("Done ({len} bytes)");
            }
            FileSub::Rm { path, recursive } => {
                if recursive {
                    afc.remove_all(&path).await?;
                } else {
                    afc.remove(&path).await?;
                }
                eprintln!("Removed {path}");
            }
            FileSub::Mkdir { path } => {
                afc.make_dir(&path).await?;
                eprintln!("Created {path}");
            }
            FileSub::Mv { from, to } => {
                afc.rename(&from, &to).await?;
                eprintln!("Moved {from} -> {to}");
            }
            FileSub::DeviceInfo => {
                let info = afc.device_info().await?;
                if json_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&afc_device_info_to_json(&info))?
                    );
                } else {
                    for line in afc_device_info_lines(&info) {
                        println!("{line}");
                    }
                }
            }
            FileSub::Stat { path } => {
                let info = afc.stat_info(&path).await?;
                if json_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&file_info_to_json(
                            path.rsplit('/').next().unwrap_or(&path),
                            &path,
                            Some(&info),
                        ))?
                    );
                } else {
                    for (k, v) in &info.raw {
                        if k == "st_mode" {
                            continue;
                        }
                        println!("{k}: {v}");
                    }
                    if let Some(mode) = info.mode {
                        println!("st_mode: {mode:#o}");
                    }
                }
            }
            FileSub::Tree { path } => {
                let tree = build_file_tree(&mut afc, &path).await?;
                if json_output {
                    println!("{}", serde_json::to_string_pretty(&tree)?);
                } else {
                    print!("{}", render_tree(&tree));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct FileTreeEntry {
    name: String,
    path: String,
    file_type: Option<String>,
    size: Option<u64>,
    link_target: Option<String>,
    children: Vec<FileTreeEntry>,
}

fn join_device_path(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", base.trim_end_matches('/'), name)
    }
}

fn file_info_to_json(
    name: &str,
    path: &str,
    info: Option<&ios_services::afc::AfcFileInfo>,
) -> serde_json::Value {
    match info {
        Some(info) => json!({
            "name": name,
            "path": path,
            "file_type": info.file_type,
            "size": info.size,
            "mode": info.mode.map(|mode| format!("{mode:#o}")),
            "link_target": info.link_target,
            "raw": info.raw,
        }),
        None => json!({
            "name": name,
            "path": path,
            "file_type": null,
            "size": null,
            "mode": null,
            "link_target": null,
            "raw": null,
        }),
    }
}

fn afc_device_info_to_json(info: &HashMap<String, String>) -> serde_json::Value {
    let mut entries: Vec<_> = info.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut obj = serde_json::Map::with_capacity(entries.len());
    for (key, value) in entries {
        obj.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    serde_json::Value::Object(obj)
}

fn afc_device_info_lines(info: &HashMap<String, String>) -> Vec<String> {
    let mut entries: Vec<_> = info.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
        .into_iter()
        .map(|(key, value)| format!("{key}: {value}"))
        .collect()
}

fn build_file_tree<'a, S>(
    afc: &'a mut ios_services::afc::AfcClient<S>,
    path: &'a str,
) -> Pin<Box<dyn Future<Output = Result<FileTreeEntry, ios_services::afc::AfcError>> + 'a>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'a,
{
    Box::pin(async move {
        let info = afc.stat_info(path).await?;
        let file_type = info.file_type.clone();
        let mut children = Vec::new();

        if matches!(file_type.as_deref(), Some("S_IFDIR")) {
            let mut entries = afc.list_dir(path).await?;
            entries.sort();
            for name in entries {
                let child_path = join_device_path(path, &name);
                children.push(build_file_tree(afc, &child_path).await?);
            }
        }

        Ok(FileTreeEntry {
            name: tree_entry_name(path),
            path: path.to_string(),
            file_type,
            size: info.size,
            link_target: info.link_target,
            children,
        })
    })
}

fn tree_entry_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.rsplit('/').next().unwrap_or(trimmed).to_string()
    }
}

fn render_tree(tree: &FileTreeEntry) -> String {
    let mut output = String::new();
    output.push_str(&render_tree_label(tree));
    output.push('\n');

    for (index, child) in tree.children.iter().enumerate() {
        render_tree_entry(child, "", index + 1 == tree.children.len(), &mut output);
    }

    output
}

fn render_tree_entry(entry: &FileTreeEntry, prefix: &str, is_last: bool, output: &mut String) {
    output.push_str(prefix);
    output.push_str(if is_last { "`-- " } else { "|-- " });
    output.push_str(&render_tree_label(entry));
    output.push('\n');

    if entry.children.is_empty() {
        return;
    }

    let next_prefix = format!("{prefix}{}", if is_last { "    " } else { "|   " });
    for (index, child) in entry.children.iter().enumerate() {
        render_tree_entry(
            child,
            &next_prefix,
            index + 1 == entry.children.len(),
            output,
        );
    }
}

fn render_tree_label(entry: &FileTreeEntry) -> String {
    match entry.file_type.as_deref() {
        Some("S_IFDIR") => {
            if entry.name == "/" {
                "/".to_string()
            } else {
                format!("{}/", entry.name)
            }
        }
        Some("S_IFLNK") => match entry.link_target.as_deref() {
            Some(target) => format!("{} -> {target}", entry.name),
            None => entry.name.clone(),
        },
        _ => entry.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        file: FileCmd,
    }

    #[test]
    fn parses_documents_flag() {
        let cmd = TestCli::parse_from([
            "file",
            "--app",
            "com.example.DocumentsApp",
            "--documents",
            "ls",
            "/",
        ]);
        assert_eq!(cmd.file.app.as_deref(), Some("com.example.DocumentsApp"));
        assert!(cmd.file.documents);
        assert!(!cmd.file.crash);
        match cmd.file.sub {
            FileSub::Ls { path, long } => {
                assert_eq!(path, "/");
                assert!(!long);
            }
            _ => panic!("expected ls subcommand"),
        }
    }

    #[test]
    fn parses_crash_flag() {
        let cmd = TestCli::parse_from(["file", "--crash", "ls", "/"]);
        assert_eq!(cmd.file.app, None);
        assert!(!cmd.file.documents);
        assert!(cmd.file.crash);
        match cmd.file.sub {
            FileSub::Ls { path, long } => {
                assert_eq!(path, "/");
                assert!(!long);
            }
            _ => panic!("expected ls subcommand"),
        }
    }

    #[test]
    fn join_device_path_preserves_root_semantics() {
        assert_eq!(join_device_path("/", "foo"), "/foo");
        assert_eq!(join_device_path("/tmp", "foo"), "/tmp/foo");
        assert_eq!(join_device_path("/tmp/", "foo"), "/tmp/foo");
    }

    #[test]
    fn file_info_to_json_formats_mode_when_present() {
        let info = ios_services::afc::AfcFileInfo {
            name: Some("foo".into()),
            file_type: Some("S_IFREG".into()),
            size: Some(123),
            mode: Some(0o100644),
            link_target: None,
            raw: std::collections::HashMap::from([
                ("st_size".into(), "123".into()),
                ("st_mode".into(), "100644".into()),
            ]),
        };

        let value = file_info_to_json("foo", "/foo", Some(&info));
        assert_eq!(value["name"], "foo");
        assert_eq!(value["path"], "/foo");
        assert_eq!(value["file_type"], "S_IFREG");
        assert_eq!(value["size"], 123);
        assert_eq!(value["mode"], "0o100644");
        assert_eq!(value["raw"]["st_size"], "123");
    }

    #[test]
    fn parses_tree_subcommand() {
        let cmd = TestCli::parse_from(["file", "tree", "/var/mobile"]);
        match cmd.file.sub {
            FileSub::Tree { path } => assert_eq!(path, "/var/mobile"),
            _ => panic!("expected tree subcommand"),
        }
    }

    #[test]
    fn parses_mv_subcommand() {
        let cmd = TestCli::parse_from(["file", "mv", "/Downloads/old.txt", "/Downloads/new.txt"]);
        match cmd.file.sub {
            FileSub::Mv { from, to } => {
                assert_eq!(from, "/Downloads/old.txt");
                assert_eq!(to, "/Downloads/new.txt");
            }
            _ => panic!("expected mv subcommand"),
        }
    }

    #[test]
    fn parses_device_info_subcommand() {
        let cmd = TestCli::parse_from(["file", "device-info"]);
        assert!(matches!(cmd.file.sub, FileSub::DeviceInfo));
    }

    #[test]
    fn render_tree_formats_nested_entries() {
        let tree = FileTreeEntry {
            name: "/".into(),
            path: "/".into(),
            file_type: Some("S_IFDIR".into()),
            size: None,
            link_target: None,
            children: vec![
                FileTreeEntry {
                    name: "Books".into(),
                    path: "/Books".into(),
                    file_type: Some("S_IFDIR".into()),
                    size: None,
                    link_target: None,
                    children: vec![FileTreeEntry {
                        name: "notes.txt".into(),
                        path: "/Books/notes.txt".into(),
                        file_type: Some("S_IFREG".into()),
                        size: Some(42),
                        link_target: None,
                        children: Vec::new(),
                    }],
                },
                FileTreeEntry {
                    name: "shortcut".into(),
                    path: "/shortcut".into(),
                    file_type: Some("S_IFLNK".into()),
                    size: None,
                    link_target: Some("/Books/notes.txt".into()),
                    children: Vec::new(),
                },
            ],
        };

        assert_eq!(
            render_tree(&tree),
            "/\n|-- Books/\n|   `-- notes.txt\n`-- shortcut -> /Books/notes.txt\n"
        );
    }

    #[test]
    fn afc_device_info_json_is_sorted_and_stringified() {
        let info = HashMap::from([
            ("FSFreeBytes".to_string(), "123".to_string()),
            ("Model".to_string(), "AFC2".to_string()),
        ]);

        let value = afc_device_info_to_json(&info);
        assert_eq!(value["FSFreeBytes"], "123");
        assert_eq!(value["Model"], "AFC2");
    }

    #[test]
    fn afc_device_info_lines_are_sorted() {
        let info = HashMap::from([
            ("zeta".to_string(), "last".to_string()),
            ("alpha".to_string(), "first".to_string()),
        ]);

        assert_eq!(
            afc_device_info_lines(&info),
            vec!["alpha: first".to_string(), "zeta: last".to_string()]
        );
    }
}
