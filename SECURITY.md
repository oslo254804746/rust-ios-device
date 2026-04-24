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
