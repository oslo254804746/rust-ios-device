# Host output and path contract

This Q03 contract covers host destinations for file pulls, backup artifacts,
OS trace, JUnit and screenshots. Device paths are a separate namespace: do not
normalize them with host filesystem APIs. Git Bash/MSYS argument conversion is
documented in [troubleshooting](troubleshooting.md); record actual received
arguments, and use the established `MSYS_NO_PATHCONV=1` workaround for AFC `/`.

Required behavior for migrated writers: keep existing destination bytes until
the complete staged output is flushed and committed; fail explicitly on an
invalid destination or denied replacement; preserve the old target on transfer,
write or commit failure; remove only this operation's temporary file after
failure/cancellation. Successful fixture output must match expected size and
SHA-256. A caller's `--force` choice remains explicit. A transfer's success does
not establish that host output succeeded.

Current implementation audit (2026-09-30):

| Writer | Existing foundation | Remaining Q03 work |
| --- | --- | --- |
| AFC CLI pull | Bounded core file read; overwrite precheck | Direct target write must migrate to staging/commit |
| CoreDevice CLI pull | Bounded stream to async writer | Opens/truncates target before transfer; needs staged streaming and cancellation cleanup |
| Screenshot/CoreDevice image | Private same-directory temporary, flush and replacement | Cancellation needs an owned cleanup guard; unify destination-type and no-clobber behavior |
| OS trace archive/collect | Staging, archive/path validation, cancellation guards | Cross-platform resource/cleanup regression and failure matrix |
| JUnit | Private temporary, flush and replacement | Shared failure/path contract and platform regression |
| Backup2 | Validated root/layout paths and staging | Retain root trust boundary; platform/manifest/cancellation regressions |

Shared `ios_core::fs_replace::move_file_replace` uses Unix rename or Windows
MoveFileExW replace-existing/write-through. It does not silently copy across
volumes. On replacement failure, the source remains caller-owned and the old
destination is preserved. Replacement does not preserve the old Windows ACL
and attributes; a private output mode is intentional.

The test matrix must include existing targets, read-only targets, directory
targets, invalid/NUL paths, Unicode, missing parents, transfer/write/commit
failures and cancellation. macOS system aliases and Windows long/verbatim paths
need execution on those systems. True UNC needs an explicitly available,
authorized share and remains NOT_RUN without one. Linux observations cannot
stand in for these cases. CLI arguments cannot contain NUL, so NUL coverage is
host API evidence only.

All portable path/type checks assume an output root and parents private to the
user. They do not claim atomic protection against a hostile concurrent
replacement of an intermediate directory. See [the backup trust boundary](../SECURITY.md#backup-root-trust-boundary).
The same rule applies to staging and cleanup; do not widen it by describing
prechecks as an atomic dirfd/openat2 guarantee. Unknown filesystem semantics and
cleanup failures must be recorded explicitly rather than reported as success.
