# Security policy

## Supported versions

The project is pre-1.0. Security fixes are expected to land on the main branch first.

## Reporting a vulnerability

Please do not open a public issue for a vulnerability report. Use GitHub's private vulnerability reporting for this repository if available, or contact the maintainer through a private channel listed on the GitHub profile.

Include:

- Affected crate, command, or API.
- Host OS and iOS version if device interaction is involved.
- Reproduction steps or a minimal proof of concept.
- Whether credentials, pair records, backups, profiles, or device data may be exposed.

## Sensitive data

This project can handle highly sensitive material, including pair records, private keys, provisioning profiles, backups, syslogs, crash reports, and device identifiers. Do not include those files or values in issues, PRs, logs, test fixtures, or screenshots unless they are synthetic.

Diagnostic identifiers use `ios_core::redaction::diagnostic_id`, a process-local
opaque correlation value. It cannot sanitize arbitrary device data or error
text; callers must avoid logging those payloads. Explicit device-selection
output and public API fields still carry identifiers for their intended purpose.
PairRecord, PersistedCredentials and PairedCredentials Debug implementations
omit all credential fields. Serialization remains intentionally available for
owner-only persistence and must never be used as a diagnostic formatter.

Discovery events, mDNS/Bonjour records, usbmux devices and connection options
also use diagnostic Debug formatting: raw identifiers, addresses, TXT contents
and pair-record paths are omitted or replaced by correlation values.
Device-selection fields and their explicit JSON serialization retain their
selection purpose. CoreError/MuxError Debug output contains the variant and
safe IO metadata; targeted device-not-found and PairRecordError Display output
does not render raw identifiers, paths or malformed credential data. Match
typed errors for programmatic handling rather than parsing diagnostic text.

Owned protocol/service errors use safe Debug views, and supervised-pairing,
supervision identity and verify-pair session Debug omit credential material.
Their typed fields, Display and explicitly traversed error sources can still
carry data for API use. Value-model Debug and serialization are also explicit
data access, not diagnostic formatters. Automatic CLI, HTTP/CDP and Python
boundaries use the [diagnostic contract](docs/diagnostic-contract.md), including
framework HTTP rejection handling. Host fixtures check original output before
any masking; a sanitized copy cannot prove the original output did not leak.
These checks do not certify all device workflows or access-control boundaries.
CLI verbosity applies only to reviewed project log targets. Third-party logs
are excluded even at TRACE because their protocol/transport logging can contain
raw headers or device content. Applications embedding the Rust library choose
their own global logging policy; this CLI filter cannot protect an independent
application's subscriber.

## Scope

Security reports may include memory safety issues in FFI boundaries, credential leakage, unsafe handling of pair records or private keys, unauthorized device operations, and network services exposed by the tunnel manager.

## Backup root trust boundary

MobileBackup2 backup and restore paths must be private to the current user and
must not be writable by an untrusted local process. The implementation rejects
path traversal and symlinks that are present when a filesystem operation starts,
but its portable path checks are not an atomic dirfd/openat2 guarantee against a
concurrent replacement of an intermediate directory. Reports involving a race
against a shared backup root should include the host OS, filesystem, operation,
and whether the root or any parent was writable by another process.

On macOS, the root-owned `/var`, `/tmp`, and `/etc` aliases into `/private` are
accepted when their targets are the exact system paths; backup roots are then
canonicalized before any child is created. Other symlinked components, including
user-created links below those aliases, remain rejected.
