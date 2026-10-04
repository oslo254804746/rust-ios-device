//! Bounded interactive input and shell result rendering, using a supplied core session.
use super::workflows::{runtime_evaluate_params, runtime_evaluation_error, runtime_remote_object};
use anyhow::{anyhow, Context, Result};
use ios_core::webinspector::{InspectorSession, WebInspectorClient};
use serde_json::{json, Value as JsonValue};
use std::io::{IsTerminal, Write as StdWrite};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};

// A shell expression is sent as one Web Inspector message. Keep input bounded
// so an unterminated pipe line cannot consume memory indefinitely. This is
// large enough for normal scripts while remaining below the plist message cap.
const MAX_SHELL_INPUT_BYTES: usize = 1024 * 1024;
pub(super) async fn run_js_shell_loop<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    client: &mut WebInspectorClient<S>,
    session: &mut InspectorSession,
    timeout: Duration,
    continue_on_error: bool,
    json_output: bool,
) -> Result<()> {
    let interactive = std::io::stdin().is_terminal();
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());

    loop {
        if interactive {
            crate::output::out!("> ");
            std::io::stdout().flush()?;
        }
        let mut line = String::new();
        let bytes = tokio::select! {
            signal = &mut ctrl_c => {
                signal.context("failed waiting for Ctrl+C")?;
                return Ok(());
            }
            result = read_bounded_shell_line(&mut reader, &mut line) => result?
        };
        if bytes == 0 {
            if interactive {
                crate::output::outln!();
            }
            return Ok(());
        }

        let expression = line.trim();
        if expression.is_empty() {
            continue;
        }
        if matches!(expression, ".exit" | "exit" | "quit") {
            return Ok(());
        }

        let evaluation = session.send_command_and_wait(
            client,
            "Runtime.evaluate",
            runtime_evaluate_params(expression),
            timeout,
        );
        tokio::pin!(evaluation);
        let result = tokio::select! {
            signal = &mut ctrl_c => {
                signal.context("failed waiting for Ctrl+C")?;
                return Ok(());
            }
            result = &mut evaluation => result
        };
        match result {
            Ok(value) => {
                if let Some(error) = runtime_evaluation_error(&value) {
                    if continue_on_error {
                        print_shell_error(expression, &error, json_output)?;
                    } else {
                        return Err(anyhow!("webinspector evaluate error: {error}"));
                    }
                } else {
                    print_shell_value(expression, value, json_output)?;
                }
            }
            Err(error) if continue_on_error => {
                print_shell_error(
                    expression,
                    &format!(
                        "evaluation request failed: {}",
                        ios_core::redaction::diagnostic_error(&error)
                    ),
                    json_output,
                )?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn read_bounded_shell_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut String,
) -> Result<usize> {
    line.clear();
    let mut bytes = Vec::new();
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            break;
        }
        let take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |position| position + 1);
        let Some(new_len) = bytes.len().checked_add(take) else {
            return Err(anyhow!(
                "JavaScript input line exceeds {MAX_SHELL_INPUT_BYTES} bytes"
            ));
        };
        if new_len > MAX_SHELL_INPUT_BYTES {
            return Err(anyhow!(
                "JavaScript input line exceeds {MAX_SHELL_INPUT_BYTES} bytes"
            ));
        }
        let has_newline = buffer[take - 1] == b'\n';
        bytes.extend_from_slice(&buffer[..take]);
        reader.consume(take);
        if has_newline {
            break;
        }
    }
    if bytes.is_empty() {
        return Ok(0);
    }
    *line = String::from_utf8(bytes).context("JavaScript input is not valid UTF-8")?;
    Ok(line.len())
}

fn print_shell_value(expression: &str, value: JsonValue, json_output: bool) -> Result<()> {
    if json_output {
        crate::output::outln!(
            "{}",
            serde_json::to_string(&json!({
                "kind": "result",
                "expression": expression,
                "result": value,
            }))?
        );
    } else {
        print_runtime_evaluation_human(&value)?;
    }
    Ok(())
}

fn print_shell_error(expression: &str, error: &str, json_output: bool) -> Result<()> {
    if json_output {
        crate::output::outln!(
            "{}",
            serde_json::to_string(&json!({
                "kind": "error",
                "expression": expression,
                "error": error,
            }))?
        );
    } else {
        crate::output::write_error_line(format_args!("{error}"))?;
    }
    Ok(())
}

pub(super) fn print_runtime_evaluation_human(value: &JsonValue) -> Result<()> {
    if let Some(value) = runtime_remote_object(value).and_then(|result| result.get("value")) {
        crate::output::outln!("{}", value.as_str().unwrap_or(&value.to_string()));
    } else if let Some(description) =
        runtime_remote_object(value).and_then(|result| result.get("description"))
    {
        crate::output::outln!("{description}");
    } else {
        crate::output::outln!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shell_input_preserves_newline_and_eof_semantics() {
        let mut reader = BufReader::new(&b"1 + 1\n2 + 2"[..]);
        let mut line = String::new();
        assert_eq!(
            read_bounded_shell_line(&mut reader, &mut line)
                .await
                .unwrap(),
            6
        );
        assert_eq!(line, "1 + 1\n");
        assert_eq!(
            read_bounded_shell_line(&mut reader, &mut line)
                .await
                .unwrap(),
            5
        );
        assert_eq!(line, "2 + 2");
        assert_eq!(
            read_bounded_shell_line(&mut reader, &mut line)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn shell_input_rejects_overlong_unterminated_line() {
        let input = vec![b'x'; MAX_SHELL_INPUT_BYTES + 1];
        let mut reader = BufReader::new(input.as_slice());
        let mut line = String::new();
        let error = read_bounded_shell_line(&mut reader, &mut line)
            .await
            .expect_err("an overlong line must be rejected before evaluation");
        assert!(error.to_string().contains("input line exceeds"));
    }
}
