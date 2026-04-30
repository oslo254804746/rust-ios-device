use std::collections::HashSet;
use std::time::Duration;

use anyhow::Result;
use ios_core::accessibility_audit::{
    AccessibilityAuditClient, FocusElement, MoveDirection, RSD_SERVICE_NAME, SERVICE_NAME,
};
use ios_core::device::{ConnectOptions, ConnectedDevice, ServiceStream};
use ios_core::TunMode;

const MAX_CONSECUTIVE_FOCUS_TIMEOUTS: usize = 5;

#[derive(clap::Args)]
pub struct AccessibilityAuditCmd {
    #[command(subcommand)]
    sub: AccessibilityAuditSub,
}

#[derive(clap::Subcommand)]
enum AccessibilityAuditSub {
    /// Display accessibility audit capabilities
    Capabilities,
    /// List supported accessibility audit types
    AuditTypes,
    /// Dump current accessibility settings
    Settings,
    /// Run one or more accessibility audit types
    RunAudit {
        #[arg(required = true)]
        types: Vec<String>,
    },
    /// Traverse focusable UI elements and print each unique item once
    ListItems {
        #[arg(
            long,
            default_value = "50",
            help = "Maximum number of unique items to collect"
        )]
        limit: usize,
        #[arg(
            long,
            default_value = "1",
            help = "Per-item timeout in seconds while waiting for focus change events"
        )]
        timeout: u64,
    },
    /// Navigate focus in a direction and describe the element
    Navigate {
        /// Direction: next, prev, first, last
        direction: String,
        #[arg(
            long,
            default_value = "2",
            help = "Timeout in seconds for focus change"
        )]
        timeout: u64,
    },
    /// Tap (activate) the currently focused element
    Tap {
        #[arg(
            long,
            default_value = "2",
            help = "Timeout in seconds for initial focus"
        )]
        timeout: u64,
    },
    /// Describe the currently focused element
    Describe {
        #[arg(long, default_value = "2", help = "Timeout in seconds for focus")]
        timeout: u64,
    },
}

impl AccessibilityAuditCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid =
            udid.ok_or_else(|| anyhow::anyhow!("--udid required for accessibility-audit"))?;

        match self.sub {
            AccessibilityAuditSub::Capabilities => run_capabilities(&udid, json).await,
            AccessibilityAuditSub::AuditTypes => run_audit_types(&udid, json).await,
            AccessibilityAuditSub::Settings => run_settings(&udid, json).await,
            AccessibilityAuditSub::RunAudit { types } => run_audit(&udid, &types, json).await,
            AccessibilityAuditSub::ListItems { limit, timeout } => {
                run_list_items(&udid, limit, timeout, json).await
            }
            AccessibilityAuditSub::Navigate { direction, timeout } => {
                run_navigate(&udid, &direction, timeout, json).await
            }
            AccessibilityAuditSub::Tap { timeout } => run_tap(&udid, timeout, json).await,
            AccessibilityAuditSub::Describe { timeout } => run_describe(&udid, timeout, json).await,
        }
    }
}

async fn run_capabilities(udid: &str, json: bool) -> Result<()> {
    let (_device, stream, version, use_rsd) = connect_accessibility_audit(udid).await?;
    let mut client = if use_rsd {
        AccessibilityAuditClient::new_rsd(stream, version)
    } else {
        AccessibilityAuditClient::new(stream, version)
    };
    let capabilities = client.capabilities().await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&capabilities)?);
    } else {
        for capability in capabilities {
            println!("{capability}");
        }
    }
    Ok(())
}

async fn run_audit_types(udid: &str, json: bool) -> Result<()> {
    let (_device, stream, version, use_rsd) = connect_accessibility_audit(udid).await?;
    let mut client = if use_rsd {
        AccessibilityAuditClient::new_rsd(stream, version)
    } else {
        AccessibilityAuditClient::new(stream, version)
    };
    let types = client.supported_audit_types().await?;
    println!("{}", render_json_value(&types, json)?);
    Ok(())
}

async fn run_settings(udid: &str, json: bool) -> Result<()> {
    let (_device, stream, version, use_rsd) = connect_accessibility_audit(udid).await?;
    let mut client = if use_rsd {
        AccessibilityAuditClient::new_rsd(stream, version)
    } else {
        AccessibilityAuditClient::new(stream, version)
    };
    let settings = client.settings().await?;
    println!("{}", render_json_value(&settings, json)?);
    Ok(())
}

async fn run_audit(udid: &str, types: &[String], json: bool) -> Result<()> {
    let (_device, stream, version, use_rsd) = connect_accessibility_audit(udid).await?;
    let mut client = if use_rsd {
        AccessibilityAuditClient::new_rsd(stream, version)
    } else {
        AccessibilityAuditClient::new(stream, version)
    };
    let result = client.run_audit(types).await?;
    println!("{}", render_json_value(&result, json)?);
    Ok(())
}

async fn run_list_items(udid: &str, limit: usize, timeout: u64, json: bool) -> Result<()> {
    let (_device, stream, version, use_rsd) = connect_accessibility_audit(udid).await?;
    let mut client = if use_rsd {
        AccessibilityAuditClient::new_rsd(stream, version)
    } else {
        AccessibilityAuditClient::new(stream, version)
    };
    let timeout = Duration::from_secs(timeout.max(1));
    let mut traversal = FocusTraversal::default();
    prepare_focus_inspector(&mut client, timeout, &mut traversal, limit).await?;

    if limit > 0 {
        client.move_focus(MoveDirection::Next).await?;
    }

    while traversal.should_continue(limit) {
        match client.next_focus_change_with_idle_timeout(timeout).await? {
            Some(focus) => {
                if !traversal.record_focus(focus) {
                    break;
                }
                if traversal.should_continue(limit) {
                    client.move_focus(MoveDirection::Next).await?;
                }
            }
            None => {
                if !traversal.record_timeout() {
                    if !traversal.is_empty() {
                        break;
                    }
                    return Err(anyhow::anyhow!("timed out waiting for focus change"));
                }
                client.move_focus(MoveDirection::Next).await?;
            }
        }
    }
    let items = traversal.into_items();

    if json {
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        for (index, item) in items.iter().enumerate() {
            print_focus(index + 1, item);
        }
    }

    Ok(())
}

async fn prepare_focus_inspector(
    client: &mut AccessibilityAuditClient<ServiceStream>,
    timeout: Duration,
    traversal: &mut FocusTraversal,
    limit: usize,
) -> Result<()> {
    client.set_app_monitoring_enabled(true).await?;

    // Match go-ios TurnOff -> SwitchToDevice -> EnableSelectionMode ordering.
    client.set_monitored_event_type(0).await?;
    let _ = tokio::time::timeout(timeout, client.wait_for_monitored_event_type_changed()).await;
    client.focus_on_element().await?;
    let _ = client.next_focus_change_with_idle_timeout(timeout).await?;
    client.preview_on_element().await?;
    client.highlight_issue().await?;
    client.set_show_visuals(false).await?;

    let _ = client.settings().await?;
    client.set_show_ignored_elements(false).await?;
    client.set_audit_target_pid(0).await?;
    client.focus_on_element().await?;
    if let Some(focus) = client.next_focus_change_with_idle_timeout(timeout).await? {
        if limit > 0 {
            let _ = traversal.record_focus(focus);
        }
    }
    client.preview_on_element().await?;
    client.highlight_issue().await?;

    client.set_monitored_event_type(2).await?;
    client.set_show_visuals(true).await?;
    let _ = tokio::time::timeout(timeout, client.wait_for_monitored_event_type_changed()).await;
    Ok(())
}

#[derive(Default)]
struct FocusTraversal {
    items: Vec<FocusElement>,
    seen: HashSet<String>,
    consecutive_timeouts: usize,
}

impl FocusTraversal {
    fn should_continue(&self, limit: usize) -> bool {
        self.items.len() < limit
    }

    fn record_focus(&mut self, focus: FocusElement) -> bool {
        self.consecutive_timeouts = 0;
        if !self.seen.insert(focus.platform_identifier.clone()) {
            return false;
        }
        self.items.push(focus);
        true
    }

    fn record_timeout(&mut self) -> bool {
        self.consecutive_timeouts += 1;
        self.consecutive_timeouts < MAX_CONSECUTIVE_FOCUS_TIMEOUTS
    }

    fn into_items(self) -> Vec<FocusElement> {
        self.items
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn parse_direction(s: &str) -> Result<MoveDirection> {
    match s.to_ascii_lowercase().as_str() {
        "next" | "n" => Ok(MoveDirection::Next),
        "prev" | "previous" | "p" => Ok(MoveDirection::Previous),
        "first" | "f" => Ok(MoveDirection::First),
        "last" | "l" => Ok(MoveDirection::Last),
        _ => Err(anyhow::anyhow!(
            "unknown direction '{s}': use next, prev, first, or last"
        )),
    }
}

async fn run_navigate(udid: &str, direction: &str, timeout: u64, json: bool) -> Result<()> {
    let direction = parse_direction(direction)?;
    let (_device, stream, version, use_rsd) = connect_accessibility_audit(udid).await?;
    let mut client = if use_rsd {
        AccessibilityAuditClient::new_rsd(stream, version)
    } else {
        AccessibilityAuditClient::new(stream, version)
    };

    let timeout = Duration::from_secs(timeout.max(1));
    client.set_app_monitoring_enabled(true).await?;
    client.set_monitored_event_type(2).await?;
    client.set_show_visuals(true).await?;

    match client.navigate(direction, timeout).await? {
        Some(focus) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&focus)?);
            } else {
                print_focus(1, &focus);
            }
        }
        None => {
            eprintln!("No focus change received within timeout");
        }
    }
    Ok(())
}

async fn run_tap(udid: &str, timeout: u64, json: bool) -> Result<()> {
    let (_device, stream, version, use_rsd) = connect_accessibility_audit(udid).await?;
    let mut client = if use_rsd {
        AccessibilityAuditClient::new_rsd(stream, version)
    } else {
        AccessibilityAuditClient::new(stream, version)
    };

    let timeout = Duration::from_secs(timeout.max(1));
    client.set_app_monitoring_enabled(true).await?;
    client.set_monitored_event_type(2).await?;
    client.set_show_visuals(true).await?;

    // Get current element first
    client.move_focus(MoveDirection::First).await?;
    let focus = client.next_focus_change_with_idle_timeout(timeout).await?;
    match focus {
        Some(focus) => {
            let element_bytes = hex::decode(&focus.platform_identifier)
                .map_err(|e| anyhow::anyhow!("invalid platform identifier: {e}"))?;
            client.perform_action_activate(&element_bytes).await?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"action": "activate", "element": focus})
                );
            } else {
                println!(
                    "Activated: {}",
                    focus.caption.as_deref().unwrap_or("<no caption>")
                );
            }
        }
        None => {
            return Err(anyhow::anyhow!("no focused element to tap"));
        }
    }
    Ok(())
}

async fn run_describe(udid: &str, timeout: u64, json: bool) -> Result<()> {
    let (_device, stream, version, use_rsd) = connect_accessibility_audit(udid).await?;
    let mut client = if use_rsd {
        AccessibilityAuditClient::new_rsd(stream, version)
    } else {
        AccessibilityAuditClient::new(stream, version)
    };

    let timeout = Duration::from_secs(timeout.max(1));
    client.set_app_monitoring_enabled(true).await?;
    client.set_monitored_event_type(2).await?;
    client.set_show_visuals(true).await?;

    client.move_focus(MoveDirection::First).await?;
    let focus = client.next_focus_change_with_idle_timeout(timeout).await?;
    match focus {
        Some(focus) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&focus)?);
            } else {
                print_focus(1, &focus);
                if let Some(ref desc) = focus.spoken_description {
                    println!("  voice: {desc}");
                }
            }
        }
        None => {
            eprintln!("No focused element found within timeout");
        }
    }
    Ok(())
}

async fn connect_accessibility_audit(
    udid: &str,
) -> Result<(ConnectedDevice, ServiceStream, u64, bool)> {
    let probe = ios_core::connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let version = probe.product_version().await?;
    let product_major = version.major as u64;
    drop(probe);

    if version.major >= 17 {
        let device = ios_core::connect(
            udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: false,
            },
        )
        .await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let stream = device.connect_rsd_service(RSD_SERVICE_NAME).await?;
        Ok((device, stream, product_major, true))
    } else {
        let device = ios_core::connect(
            udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let stream = device.connect_service(SERVICE_NAME).await?;
        Ok((device, stream, product_major, false))
    }
}

fn render_json_value(value: &serde_json::Value, json: bool) -> Result<String> {
    if json {
        Ok(serde_json::to_string_pretty(value)?)
    } else if let Some(array) = value.as_array() {
        Ok(array
            .iter()
            .map(|item| {
                item.as_str()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| serde_json::to_string_pretty(item).unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .join("\n"))
    } else {
        Ok(serde_json::to_string_pretty(value)?)
    }
}

fn print_focus(index: usize, focus: &FocusElement) {
    println!(
        "[{index}] {}",
        focus.caption.as_deref().unwrap_or("<no caption>")
    );
    if let Some(spoken_description) = &focus.spoken_description {
        println!("  spoken: {spoken_description}");
    }
    println!("  platform_identifier: {}", focus.platform_identifier);
    println!("  estimated_uid: {}", focus.estimated_uid);
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{parse_direction, AccessibilityAuditSub, FocusElement, FocusTraversal};
    use ios_core::accessibility_audit::MoveDirection;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: AccessibilityAuditSub,
    }

    #[test]
    fn parses_capabilities_subcommand() {
        let parsed = TestCli::try_parse_from(["accessibility-audit", "capabilities"]);
        assert!(parsed.is_ok(), "capabilities command should parse");
    }

    #[test]
    fn parses_list_items_subcommand() {
        let parsed = TestCli::try_parse_from([
            "accessibility-audit",
            "list-items",
            "--limit",
            "10",
            "--timeout",
            "2",
        ]);
        assert!(parsed.is_ok(), "list-items command should parse");
    }

    #[test]
    fn parses_navigate_subcommand() {
        let parsed =
            TestCli::try_parse_from(["accessibility-audit", "navigate", "next", "--timeout", "3"]);
        assert!(parsed.is_ok(), "navigate command should parse");
    }

    #[test]
    fn parses_tap_subcommand() {
        let parsed = TestCli::try_parse_from(["accessibility-audit", "tap"]);
        assert!(parsed.is_ok(), "tap command should parse");
    }

    #[test]
    fn parses_describe_subcommand() {
        let parsed = TestCli::try_parse_from(["accessibility-audit", "describe", "--timeout", "5"]);
        assert!(parsed.is_ok(), "describe command should parse");
    }

    #[test]
    fn parse_direction_accepts_valid_values() {
        assert_eq!(parse_direction("next").unwrap(), MoveDirection::Next);
        assert_eq!(parse_direction("n").unwrap(), MoveDirection::Next);
        assert_eq!(parse_direction("prev").unwrap(), MoveDirection::Previous);
        assert_eq!(
            parse_direction("previous").unwrap(),
            MoveDirection::Previous
        );
        assert_eq!(parse_direction("first").unwrap(), MoveDirection::First);
        assert_eq!(parse_direction("last").unwrap(), MoveDirection::Last);
        assert!(parse_direction("invalid").is_err());
    }

    #[test]
    fn list_items_tolerates_transient_timeouts_and_resets_after_focus() {
        let mut traversal = FocusTraversal::default();

        for _ in 0..(super::MAX_CONSECUTIVE_FOCUS_TIMEOUTS - 1) {
            assert!(
                traversal.record_timeout(),
                "transient timeout should keep traversal alive"
            );
        }

        assert!(traversal.record_focus(test_focus("A")));

        for _ in 0..(super::MAX_CONSECUTIVE_FOCUS_TIMEOUTS - 1) {
            assert!(
                traversal.record_timeout(),
                "focus event should reset timeout counter"
            );
        }

        assert!(
            !traversal.record_timeout(),
            "fifth consecutive timeout should stop traversal"
        );
    }

    #[test]
    fn list_items_stops_on_duplicate_focus_identifier() {
        let mut traversal = FocusTraversal::default();

        assert!(traversal.record_focus(test_focus("A")));
        assert!(traversal.record_focus(test_focus("B")));
        assert!(
            !traversal.record_focus(test_focus("B")),
            "duplicate focus identifier should stop traversal"
        );

        let items = traversal.into_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].platform_identifier, "A");
        assert_eq!(items[1].platform_identifier, "B");
    }

    fn test_focus(platform_identifier: &str) -> FocusElement {
        FocusElement {
            platform_identifier: platform_identifier.to_owned(),
            estimated_uid: format!("uid-{platform_identifier}"),
            caption: Some(format!("caption-{platform_identifier}")),
            spoken_description: None,
        }
    }
}
