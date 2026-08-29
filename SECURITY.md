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
