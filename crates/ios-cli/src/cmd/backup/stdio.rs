//! Fallible backup result output. Formatting keeps the existing line contract.

use std::fmt::Arguments;
use std::io::{self, Write};

pub(super) fn write_line(arguments: Arguments<'_>) -> io::Result<()> {
    let stdout = io::stdout();
    write_line_to(&mut stdout.lock(), arguments)
}

fn write_line_to(writer: &mut impl Write, arguments: Arguments<'_>) -> io::Result<()> {
    writer.write_fmt(arguments)?;
    writer.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingOutput {
        bytes: Vec<u8>,
        remaining: usize,
    }

    impl Write for FailingOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic output failure",
                ));
            }
            let count = bytes.len().min(self.remaining);
            self.bytes.extend_from_slice(&bytes[..count]);
            self.remaining -= count;
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn partial_result_failure_returns_io_error_without_success_or_panic() {
        let mut output = FailingOutput {
            bytes: Vec::new(),
            remaining: 2,
        };
        let error = write_line_to(&mut output, format_args!("{}", "result"))
            .expect_err("partial output must fail");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(output.bytes, b"re");
    }

    #[test]
    fn trailing_newline_failure_is_part_of_the_result_contract() {
        let mut output = FailingOutput {
            bytes: Vec::new(),
            remaining: 6,
        };
        let error = write_line_to(&mut output, format_args!("{}", "result"))
            .expect_err("missing final newline must fail");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(output.bytes, b"result");
        let mut complete = Vec::new();
        write_line_to(&mut complete, format_args!("{:04x}", 42)).unwrap();
        assert_eq!(complete, b"002a\n");
    }
}
