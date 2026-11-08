//! Versioned mutation results and ordered per-process completion records.

use anyhow::Result;
use ios_core::apps::RunningAppProcess;
use serde::Serialize;
use std::future::Future;

#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(super) enum Mutation<'a> {
    Install {
        path: &'a str,
        streaming: bool,
    },
    Upgrade {
        path: &'a str,
    },
    Archive {
        bundle_id: &'a str,
    },
    Restore {
        bundle_id: &'a str,
    },
    Kill {
        pid: u64,
    },
    Signal {
        pid: u64,
        signal: i64,
    },
    Launch {
        bundle_id: &'a str,
        pid: Option<u64>,
    },
    Uninstall {
        bundle_id: &'a str,
    },
    Pkill {
        pattern: &'a str,
        signal: i64,
        matched_count: usize,
        completed_count: usize,
        complete: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        process: Option<SignalledProcess<'a>>,
    },
}

#[derive(Serialize)]
pub(super) struct SignalledProcess<'a> {
    pid: u64,
    name: &'a str,
}

#[derive(Serialize)]
struct MutationRecord<'record, 'values> {
    schema_version: u8,
    #[serde(flatten)]
    mutation: &'record Mutation<'values>,
}

fn render(mutation: &Mutation<'_>, json: bool) -> Result<String> {
    let record = if json {
        serde_json::to_string(&MutationRecord {
            schema_version: 1,
            mutation,
        })?
    } else {
        match mutation {
            Mutation::Install {
                path,
                streaming: true,
            } => format!("Installed {path} (streaming)"),
            Mutation::Install {
                path,
                streaming: false,
            } => format!("Installed {path}"),
            Mutation::Upgrade { path } => format!("Upgraded {path}"),
            Mutation::Archive { bundle_id } => format!("Archived {bundle_id}"),
            Mutation::Restore { bundle_id } => format!("Restored {bundle_id}"),
            Mutation::Kill { pid } => format!("Killed process {pid}"),
            Mutation::Signal { pid, signal } => format!("Sent signal {signal} to process {pid}"),
            Mutation::Launch {
                bundle_id,
                pid: Some(pid),
            } => format!("Launched {bundle_id} (pid {pid})"),
            Mutation::Launch {
                bundle_id,
                pid: None,
            } => format!("Launched {bundle_id}"),
            Mutation::Uninstall { bundle_id } => format!("Uninstalled {bundle_id}"),
            Mutation::Pkill {
                signal,
                process: Some(process),
                ..
            } => format!(
                "Sent signal {signal} to {} (pid {})",
                process.name, process.pid
            ),
            Mutation::Pkill {
                pattern,
                matched_count: 0,
                ..
            } => format!("No processes matched pattern '{pattern}'"),
            Mutation::Pkill {
                completed_count, ..
            } => format!("Killed {completed_count} process(es)"),
        }
    };
    Ok(format!("{record}\n"))
}

pub(super) fn emit(mutation: &Mutation<'_>, json: bool) -> Result<()> {
    crate::output::write_formatted(format_args!("{}", render(mutation, json)?))?;
    Ok(())
}

/// Report only completed signal calls, in their original sequential order.
/// No final completion record is emitted after a call or output failure.
pub(super) async fn signal_matches<F, Fut, E>(
    processes: &[RunningAppProcess],
    pattern: &str,
    signal: i64,
    json: bool,
    mut send: F,
    mut emit: E,
) -> Result<()>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = Result<()>>,
    E: FnMut(&str) -> std::io::Result<()>,
{
    let pattern_lower = pattern.to_ascii_lowercase();
    let matched: Vec<_> = processes
        .iter()
        .filter(|process| process.name.to_ascii_lowercase().contains(&pattern_lower))
        .collect();
    for (index, process) in matched.iter().enumerate() {
        send(process.pid).await?;
        emit(&render(
            &Mutation::Pkill {
                pattern,
                signal,
                matched_count: matched.len(),
                completed_count: index + 1,
                complete: false,
                process: Some(SignalledProcess {
                    pid: process.pid,
                    name: &process.name,
                }),
            },
            json,
        )?)?;
    }
    emit(&render(
        &Mutation::Pkill {
            pattern,
            signal,
            matched_count: matched.len(),
            completed_count: matched.len(),
            complete: true,
            process: None,
        },
        json,
    )?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::io::{self, ErrorKind};

    #[test]
    fn single_mutations_keep_legacy_human_bytes_and_versioned_json_fields() {
        let path = "directory/\"例子\"\n.app";
        let cases = [
            (
                Mutation::Install {
                    path,
                    streaming: false,
                },
                format!("Installed {path}\n"),
                json!({"operation":"install","path":path,"streaming":false}),
            ),
            (
                Mutation::Install {
                    path,
                    streaming: true,
                },
                format!("Installed {path} (streaming)\n"),
                json!({"operation":"install","path":path,"streaming":true}),
            ),
            (
                Mutation::Upgrade { path },
                format!("Upgraded {path}\n"),
                json!({"operation":"upgrade","path":path}),
            ),
            (
                Mutation::Archive {
                    bundle_id: "com.example.App",
                },
                "Archived com.example.App\n".into(),
                json!({"operation":"archive","bundle_id":"com.example.App"}),
            ),
            (
                Mutation::Restore {
                    bundle_id: "com.example.App",
                },
                "Restored com.example.App\n".into(),
                json!({"operation":"restore","bundle_id":"com.example.App"}),
            ),
            (
                Mutation::Kill { pid: u64::MAX },
                format!("Killed process {}\n", u64::MAX),
                json!({"operation":"kill","pid":u64::MAX}),
            ),
            (
                Mutation::Signal {
                    pid: 42,
                    signal: -1,
                },
                "Sent signal -1 to process 42\n".into(),
                json!({"operation":"signal","pid":42,"signal":-1}),
            ),
            (
                Mutation::Launch {
                    bundle_id: "com.example.App",
                    pid: Some(42),
                },
                "Launched com.example.App (pid 42)\n".into(),
                json!({"operation":"launch","bundle_id":"com.example.App","pid":42}),
            ),
            (
                Mutation::Launch {
                    bundle_id: "com.example.App",
                    pid: None,
                },
                "Launched com.example.App\n".into(),
                json!({"operation":"launch","bundle_id":"com.example.App","pid":null}),
            ),
            (
                Mutation::Uninstall {
                    bundle_id: "com.example.App",
                },
                "Uninstalled com.example.App\n".into(),
                json!({"operation":"uninstall","bundle_id":"com.example.App"}),
            ),
        ];
        for (mutation, human, mut expected) in cases {
            assert_eq!(render(&mutation, false).unwrap(), human);
            expected["schema_version"] = json!(1);
            let record = render(&mutation, true).unwrap();
            assert!(record.ends_with('\n'));
            assert_eq!(record.lines().count(), 1);
            assert_eq!(serde_json::from_str::<Value>(&record).unwrap(), expected);
        }
    }

    fn processes() -> Vec<RunningAppProcess> {
        [
            (7, "other"),
            (42, "Worker-a"),
            (43, "worker-b"),
            (44, "Worker-c"),
        ]
        .into_iter()
        .map(|(pid, name)| RunningAppProcess {
            pid,
            name: name.into(),
            bundle_id: None,
            executable: None,
            is_application: None,
        })
        .collect()
    }

    fn records(output: &str) -> Vec<Value> {
        assert!(output.ends_with('\n'));
        output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn pkill_emits_ordered_completed_calls_then_one_final_summary() {
        let mut calls = Vec::new();
        let mut output = String::new();
        signal_matches(
            &processes(),
            "WORK",
            15,
            true,
            |pid| {
                calls.push(pid);
                std::future::ready(Ok(()))
            },
            |record| {
                output.push_str(record);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(calls, [42, 43, 44]);
        let records = records(&output);
        assert_eq!(records.len(), 4);
        for (index, name) in ["Worker-a", "worker-b", "Worker-c"].into_iter().enumerate() {
            assert_eq!(
                records[index],
                json!({"schema_version":1,"operation":"pkill","pattern":"WORK","signal":15,"matched_count":3,"completed_count":index+1,"complete":false,"process":{"pid":calls[index],"name":name}})
            );
        }
        assert_eq!(
            records[3],
            json!({"schema_version":1,"operation":"pkill","pattern":"WORK","signal":15,"matched_count":3,"completed_count":3,"complete":true})
        );
    }

    fn assert_failure(error: &anyhow::Error, kind: ErrorKind, class: &str) {
        let source = error
            .chain()
            .find_map(|source| source.downcast_ref::<io::Error>())
            .unwrap();
        assert_eq!(source.kind(), kind);
        assert_eq!(source.to_string(), "synthetic-private-mutation-source");
        let message = crate::output::operation_failure_message(
            crate::output::ErrorFormat::Json,
            "apps",
            error,
        )
        .unwrap();
        assert!(!message.contains("synthetic-private-mutation-source"));
        let body: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(body["error"]["class"], class);
        assert_eq!(body["error"]["phase"], "apps");
        assert_eq!(body["error"]["exit_code"], 1);
        assert_eq!(body["error"]["effects"], "unknown");
    }

    #[tokio::test]
    async fn pkill_failure_preserves_prior_records_and_omits_completion() {
        let mut calls = Vec::new();
        let mut output = String::new();
        let error = signal_matches(
            &processes(),
            "worker",
            9,
            true,
            |pid| {
                calls.push(pid);
                std::future::ready(if pid == 43 {
                    Err(anyhow::Error::new(io::Error::new(
                        ErrorKind::PermissionDenied,
                        "synthetic-private-mutation-source",
                    )))
                } else {
                    Ok(())
                })
            },
            |record| {
                output.push_str(record);
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert_eq!(calls, [42, 43]);
        let records = records(&output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["complete"], false);
        assert_eq!(records[0]["completed_count"], 1);
        assert_eq!(records[0]["matched_count"], 3);
        assert_failure(&error, ErrorKind::PermissionDenied, "permission_denied");
    }

    #[tokio::test]
    async fn pkill_output_failure_stops_before_sending_to_next_process() {
        let mut calls = Vec::new();
        let error = signal_matches(
            &processes(),
            "worker",
            9,
            true,
            |pid| {
                calls.push(pid);
                std::future::ready(Ok(()))
            },
            |_| {
                Err(io::Error::new(
                    ErrorKind::BrokenPipe,
                    "synthetic-private-mutation-source",
                ))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(calls, [42]);
        assert_failure(&error, ErrorKind::BrokenPipe, "disconnected");
    }

    #[tokio::test]
    async fn pkill_empty_match_succeeds_without_sending_and_keeps_human_message() {
        for json_output in [false, true] {
            let mut output = String::new();
            signal_matches(
                &processes(),
                "absent",
                9,
                json_output,
                |_| {
                    std::future::ready(Err(anyhow::anyhow!("no-match request signalled a process")))
                },
                |record| {
                    output.push_str(record);
                    Ok(())
                },
            )
            .await
            .unwrap();
            if json_output {
                assert_eq!(
                    records(&output),
                    vec![
                        json!({"schema_version":1,"operation":"pkill","pattern":"absent","signal":9,"matched_count":0,"completed_count":0,"complete":true})
                    ]
                );
            } else {
                assert_eq!(output, "No processes matched pattern 'absent'\n");
            }
        }
    }

    #[tokio::test]
    async fn pkill_human_progress_and_summary_keep_existing_bytes() {
        let mut output = String::new();
        signal_matches(
            &processes(),
            "WORK",
            15,
            false,
            |_| std::future::ready(Ok(())),
            |record| {
                output.push_str(record);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(
            output,
            concat!(
                "Sent signal 15 to Worker-a (pid 42)\n",
                "Sent signal 15 to worker-b (pid 43)\n",
                "Sent signal 15 to Worker-c (pid 44)\n",
                "Killed 3 process(es)\n"
            )
        );
    }
}
