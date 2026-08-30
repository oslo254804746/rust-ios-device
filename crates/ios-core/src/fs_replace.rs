//! Atomic file replacement shared by backup, os trace, JUnit, and CLI output
//! writers.
//!
//! Rust's `std::fs::rename` can replace an existing destination on Windows,
//! but this helper uses `MoveFileExW` to request replace-existing and
//! write-through semantics explicitly. The flag values used here match the
//! Microsoft Win32 definitions exactly: `MOVEFILE_REPLACE_EXISTING` is `0x1`
//! and `MOVEFILE_WRITE_THROUGH` is `0x8`, so they are pinned by a unit test
//! below instead of being re-derived at each call site.
//!
//! Failure contract: if the move fails, the destination is left unchanged and
//! the source still exists; the caller owns any temporary-file cleanup.  This
//! deliberately avoids `ReplaceFileW`, whose `ERROR_UNABLE_TO_MOVE_REPLACEMENT`
//! (1176) and `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` (1177) states delete the
//! old destination while the replacement remains at the temporary path when no
//! backup file is supplied — a partially-failed state that would otherwise
//! have to be recovered by hand.  Unlike `ReplaceFileW`, the ACLs and
//! attributes of the replaced file are not preserved.

use std::io;
use std::path::Path;

/// Microsoft-documented value of `MOVEFILE_REPLACE_EXISTING`.
#[cfg(windows)]
const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
/// Microsoft-documented value of `MOVEFILE_WRITE_THROUGH`.
#[cfg(windows)]
const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

// MAX_PATH includes the terminating NUL for the legacy Win32 APIs used here.
#[cfg(windows)]
const WINDOWS_MAX_PATH: usize = 260;
#[cfg(windows)]
const WINDOWS_EXTENDED_PATH_MAX: usize = 32_767;
#[cfg(windows)]
const GET_FULL_PATH_INITIAL_CAPACITY: usize = 256;
#[cfg(windows)]
const GET_FULL_PATH_MAX_ATTEMPTS: usize = 8;

#[cfg(windows)]
const VERBATIM_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
#[cfg(windows)]
const NT_PREFIX: &[u16] = &[b'\\' as u16, b'?' as u16, b'?' as u16, b'\\' as u16];
#[cfg(windows)]
const DEVICE_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'.' as u16, b'\\' as u16];
#[cfg(windows)]
const UNC_PREFIX: &[u16] = &[
    b'\\' as u16,
    b'\\' as u16,
    b'?' as u16,
    b'\\' as u16,
    b'U' as u16,
    b'N' as u16,
    b'C' as u16,
    b'\\' as u16,
];

#[cfg(windows)]
fn validate_windows_path(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let mut length = 0;
    for unit in path.as_os_str().encode_wide() {
        if unit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "paths passed to Win32 cannot contain NULs",
            ));
        }
        length += 1;
    }
    if length > WINDOWS_EXTENDED_PATH_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path exceeds the Windows extended-length limit",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_namespace_path(path: &[u16]) -> bool {
    path.starts_with(VERBATIM_PREFIX)
        || path.starts_with(NT_PREFIX)
        || path.starts_with(DEVICE_PREFIX)
}

#[cfg(windows)]
fn get_full_path_name(path: &[u16]) -> io::Result<Vec<u16>> {
    use std::ptr;

    // GetFullPathNameW is lexical: it resolves relative components and the
    // current directory, but does not inspect or follow filesystem symlinks.
    let mut input = path.to_vec();
    input.push(0);
    let mut capacity = GET_FULL_PATH_INITIAL_CAPACITY;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFullPathNameW(
            file_name: *const u16,
            buffer_length: u32,
            buffer: *mut u16,
            file_part: *mut *mut u16,
        ) -> u32;
    }

    for _ in 0..GET_FULL_PATH_MAX_ATTEMPTS {
        let mut buffer = vec![0u16; capacity];
        let length = unsafe {
            GetFullPathNameW(
                input.as_ptr(),
                capacity as u32,
                buffer.as_mut_ptr(),
                ptr::null_mut(),
            )
        } as usize;
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length < capacity {
            buffer.truncate(length);
            return Ok(buffer);
        }

        let required = length.saturating_add(1);
        if required > WINDOWS_EXTENDED_PATH_MAX + 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "absolute path exceeds the Windows extended-length limit",
            ));
        }
        capacity = capacity
            .saturating_mul(2)
            .max(required)
            .min(WINDOWS_EXTENDED_PATH_MAX + 1);
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "could not determine the full Windows path within the retry limit",
    ))
}

#[cfg(windows)]
fn add_verbatim_prefix(mut absolute: Vec<u16>) -> io::Result<Vec<u16>> {
    if absolute.starts_with(VERBATIM_PREFIX) || absolute.starts_with(NT_PREFIX) {
        return Ok(absolute);
    }

    let prefix = if absolute.starts_with(DEVICE_PREFIX) {
        absolute.drain(..DEVICE_PREFIX.len());
        VERBATIM_PREFIX
    } else if absolute.starts_with(&[b'\\' as u16, b'\\' as u16]) {
        absolute.drain(..2);
        UNC_PREFIX
    } else if absolute.len() >= 3 && absolute[1] == b':' as u16 && absolute[2] == b'\\' as u16 {
        VERBATIM_PREFIX
    } else {
        &[]
    };

    if prefix.len() + absolute.len() > WINDOWS_EXTENDED_PATH_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "absolute path exceeds the Windows extended-length limit",
        ));
    }
    let mut result = Vec::with_capacity(prefix.len() + absolute.len());
    result.extend_from_slice(prefix);
    result.extend_from_slice(&absolute);
    Ok(result)
}

#[cfg(windows)]
fn prepare_windows_move_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    let encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    // Existing namespace paths are already in the form expected by Win32.
    // In particular, preserve verbatim trailing dots and invalid WTF-16
    // components instead of canonicalizing a path that may not exist yet.
    if encoded.is_empty() || is_windows_namespace_path(&encoded) {
        let mut result = encoded;
        result.push(0);
        return Ok(result);
    }

    // Resolve non-namespace inputs before adding a prefix. Prefixing a
    // relative path directly would make it relative to the wrong namespace.
    let absolute = get_full_path_name(&encoded)?;
    if absolute.len() + 1 < WINDOWS_MAX_PATH {
        let mut result = encoded;
        result.push(0);
        return Ok(result);
    }

    let mut result = add_verbatim_prefix(absolute)?;
    result.push(0);
    Ok(result)
}

/// Replace `destination` with the file at `temporary`.
///
/// On Windows this is `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING |
/// MOVEFILE_WRITE_THROUGH`, so an existing destination is replaced atomically
/// and the move is flushed to disk before the call returns.  On other
/// platforms it is a plain `rename`.  Both paths must be on the same volume;
/// no cross-volume copy/delete fallback is attempted.
pub fn move_file_replace(temporary: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        validate_windows_path(temporary)?;
        validate_windows_path(destination)?;
        let temporary = prepare_windows_move_path(temporary)?;
        let destination = prepare_windows_move_path(destination)?;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let result = unsafe {
            MoveFileExW(
                temporary.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(temporary, destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_directory(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "ios-core-fs-replace-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&directory).expect("create test directory");
        directory
    }

    #[cfg(windows)]
    #[test]
    fn move_file_ex_constants_match_the_official_win32_values() {
        assert_eq!(MOVEFILE_REPLACE_EXISTING, 0x1);
        assert_eq!(MOVEFILE_WRITE_THROUGH, 0x8);
    }

    #[cfg(windows)]
    #[test]
    fn move_file_replace_rejects_embedded_nuls_before_moving() {
        use std::ffi::{OsStr, OsString};
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        fn with_embedded_nul(path: &OsStr) -> std::path::PathBuf {
            let mut wide: Vec<u16> = path.encode_wide().collect();
            wide.push(0);
            wide.extend("ignored-suffix.bin".encode_utf16());
            std::path::PathBuf::from(OsString::from_wide(&wide))
        }

        let source_case = unique_test_directory("nul-source");
        let source_prefix = source_case.join("source-prefix.bin");
        let destination = source_case.join("destination.bin");
        std::fs::write(&source_prefix, b"source-prefix").expect("write source prefix");
        std::fs::write(&destination, b"old-destination").expect("write destination");
        let malformed_source = with_embedded_nul(source_prefix.as_os_str());

        let error = move_file_replace(&malformed_source, &destination)
            .expect_err("source paths containing NUL must be rejected");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(&source_prefix).unwrap(), b"source-prefix");
        assert_eq!(std::fs::read(&destination).unwrap(), b"old-destination");
        std::fs::remove_dir_all(&source_case).expect("clean up source NUL test");

        let destination_case = unique_test_directory("nul-destination");
        let source = destination_case.join("source.bin");
        let destination_prefix = destination_case.join("destination-prefix.bin");
        std::fs::write(&source, b"source").expect("write source");
        std::fs::write(&destination_prefix, b"old-destination-prefix")
            .expect("write destination prefix");
        let malformed_destination = with_embedded_nul(destination_prefix.as_os_str());

        let error = move_file_replace(&source, &malformed_destination)
            .expect_err("destination paths containing NUL must be rejected");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
        assert_eq!(
            std::fs::read(&destination_prefix).unwrap(),
            b"old-destination-prefix"
        );
        std::fs::remove_dir_all(&destination_case).expect("clean up destination NUL test");
    }

    #[cfg(windows)]
    fn verbatim_path(path: &std::path::Path) -> std::path::PathBuf {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let mut wide: Vec<u16> = "\\\\?\\".encode_utf16().collect();
        wide.extend(path.as_os_str().encode_wide());
        std::path::PathBuf::from(OsString::from_wide(&wide))
    }

    #[cfg(windows)]
    #[test]
    fn move_file_replace_supports_long_plain_paths() {
        use std::os::windows::ffi::OsStrExt;

        let directory = unique_test_directory("long-plain");
        let mut long_directory = directory.clone();
        for index in 0..13 {
            long_directory.push(format!("segment-{index:02}-abcdefghijklmnop"));
        }
        assert!(
            long_directory.as_os_str().encode_wide().count() > 260,
            "the regression path must exceed the legacy Windows limit"
        );

        let long_directory_verbatim = verbatim_path(&long_directory);
        std::fs::create_dir_all(&long_directory_verbatim).expect("create long directory");
        let source = long_directory.join("source.bin");
        let destination = long_directory.join("destination.bin");
        let source_verbatim = long_directory_verbatim.join("source.bin");
        let destination_verbatim = long_directory_verbatim.join("destination.bin");

        std::fs::write(&source_verbatim, b"first long contents").expect("write long source");
        move_file_replace(&source, &destination).expect("replace missing long destination");
        assert_eq!(
            std::fs::read(&destination_verbatim).unwrap(),
            b"first long contents"
        );
        assert!(!source_verbatim.exists(), "long source must be consumed");

        std::fs::write(&source_verbatim, b"second long contents")
            .expect("write second long source");
        move_file_replace(&source, &destination).expect("replace existing long destination");
        assert_eq!(
            std::fs::read(&destination_verbatim).unwrap(),
            b"second long contents"
        );
        assert!(
            !source_verbatim.exists(),
            "second long source must be consumed"
        );

        std::fs::remove_dir_all(&long_directory_verbatim).expect("clean up long path test");
        std::fs::remove_dir_all(&directory).expect("clean up test directory");
    }

    #[cfg(windows)]
    #[test]
    fn move_file_replace_preserves_verbatim_unicode_and_trailing_dot() {
        let directory = unique_test_directory("verbatim-tail-dot");
        let directory = verbatim_path(&directory);
        let source = directory.join("源😀-source.");
        let destination = directory.join("源😀-destination.");

        std::fs::write(&source, b"verbatim source").expect("write verbatim source");
        std::fs::write(&destination, b"old verbatim destination")
            .expect("write verbatim destination");
        move_file_replace(&source, &destination).expect("replace verbatim destination");
        assert_eq!(std::fs::read(&destination).unwrap(), b"verbatim source");
        assert!(!source.exists(), "verbatim source must be consumed");

        std::fs::remove_dir_all(&directory).expect("clean up verbatim test");
    }

    #[cfg(windows)]
    #[test]
    fn long_relative_paths_are_normalized_before_verbatim_prefixing() {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let mut relative = std::path::PathBuf::from(".");
        relative.push("lexical-parent");
        relative.push("..");
        for index in 0..13 {
            relative.push(format!("segment-{index:02}-abcdefghijklmnop"));
        }

        let prepared = prepare_windows_move_path(&relative).expect("prepare relative long path");
        assert_eq!(
            prepared.last(),
            Some(&0),
            "prepared path must be terminated"
        );
        let normalized =
            std::path::PathBuf::from(OsString::from_wide(&prepared[..prepared.len() - 1]));
        assert!(
            normalized.is_absolute(),
            "relative input must become absolute"
        );
        assert!(
            normalized.components().all(|component| !matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )),
            "GetFullPathNameW must normalize relative components lexically"
        );
        assert!(
            normalized
                .as_os_str()
                .encode_wide()
                .collect::<Vec<_>>()
                .starts_with(&"\\\\?\\".encode_utf16().collect::<Vec<_>>()),
            "long relative input must receive a verbatim prefix"
        );
    }

    #[cfg(windows)]
    #[test]
    fn unc_prefix_conversion_is_lexical_without_network_access() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        let absolute = "\\\\server\\share\\nested\\file.bin"
            .encode_utf16()
            .collect::<Vec<_>>();
        let prefixed = add_verbatim_prefix(absolute).expect("prefix synthetic UNC path");
        let prefixed = OsString::from_wide(&prefixed);
        assert_eq!(
            prefixed.to_string_lossy(),
            "\\\\?\\UNC\\server\\share\\nested\\file.bin"
        );
    }

    #[test]
    fn move_file_replace_creates_and_overwrites_and_fails_without_source() {
        let directory = unique_test_directory("basic");
        let destination = directory.join("destination.bin");
        let temporary = directory.join("temporary.bin");

        std::fs::write(&temporary, b"new contents").expect("write temporary");
        move_file_replace(&temporary, &destination).expect("replace missing destination");
        assert_eq!(std::fs::read(&destination).unwrap(), b"new contents");
        assert!(!temporary.exists(), "source must be consumed by the move");

        std::fs::write(&temporary, b"second contents").expect("write temporary again");
        move_file_replace(&temporary, &destination).expect("replace existing destination");
        assert_eq!(std::fs::read(&destination).unwrap(), b"second contents");
        assert!(!temporary.exists());

        let error = move_file_replace(&temporary, &destination)
            .expect_err("moving a missing source must fail");
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"second contents",
            "a failed move must leave the destination unchanged"
        );
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);

        std::fs::remove_dir_all(&directory).expect("clean up test directory");
    }

    #[test]
    fn move_file_replace_failure_keeps_destination_and_temporary() {
        let directory = unique_test_directory("failure");
        let destination = directory.join("destination.bin");
        std::fs::write(&destination, b"original").expect("write destination");

        // A directory at the destination cannot be replaced by a file move on
        // either platform, which makes the failure deterministic without
        // depending on ACLs or file locks.
        let blocked = directory.join("blocked");
        std::fs::create_dir(&blocked).expect("create blocking directory");
        let temporary = directory.join("temporary.bin");
        std::fs::write(&temporary, b"new").expect("write temporary");

        assert!(move_file_replace(&temporary, &blocked).is_err());
        assert!(
            temporary.exists(),
            "a failed move must leave the source in place for caller cleanup"
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"original");

        std::fs::remove_dir_all(&directory).expect("clean up test directory");
    }
}
