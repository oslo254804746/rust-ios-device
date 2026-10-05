//! Bounded interactive input and shell result rendering, using a supplied core session.
use super::workflows::runtime_remote_object;
use anyhow::{anyhow, Context, Result};
use serde_json::Value as JsonValue;
#[cfg(test)]
use tokio::io::BufReader;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

// A shell expression is sent as one Web Inspector message. Keep input bounded
// so an unterminated pipe line cannot consume memory indefinitely. This is
// large enough for normal scripts while remaining below the plist message cap.
const MAX_SHELL_INPUT_BYTES: usize = 1024 * 1024;
pub(super) async fn read_bounded_shell_line<R: AsyncBufRead + Unpin>(
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

pub(super) fn runtime_evaluation_human(value: &JsonValue) -> Result<String> {
    if let Some(value) = runtime_remote_object(value).and_then(|result| result.get("value")) {
        Ok(value
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| value.to_string()))
    } else if let Some(description) =
        runtime_remote_object(value).and_then(|result| result.get("description"))
    {
        Ok(description.to_string())
    } else {
        Ok(serde_json::to_string_pretty(value)?)
    }
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
