# Host output and path contract

This Q03 contract covers host destinations for file pulls, backup artifacts,
OS trace, JUnit and screenshots. Device paths are a separate namespace: do not
normalize them with host filesystem APIs. Git Bash/MSYS argument conversion is
documented in [troubleshooting](troubleshooting.md); record actual received
arguments, and use the established `MSYS_NO_PATHCONV=1` workaround for AFC `/`.

Migrated writers keep existing destination bytes until the complete staged
output is flushed and committed. Invalid destinations and denied replacements
fail explicitly; transfer, write and replacement failures preserve the old
target. A dropped transfer removes only this operation's temporary entry, with
cleanup failures reported by category without disclosing the path. Successful
fixture output must match expected size and SHA-256. A caller's `--force`
choice remains explicit. A transfer's success does not establish host output
success.

Cancellation before publication preserves the old file. Once the filesystem
commit starts it can finish despite cancellation, particularly when an async
caller stops awaiting a blocking filesystem worker. The worker owns its
cleanup guard until it finishes. Callers must inspect an interrupted commit's
destination and validate its size/hash; cancellation cannot promise rollback.
After publication, a cleanup failure reports an error while the complete new
artifact can already exist. This is a per-file contract: a backup, recursive
copy or multi-file initializer is not one atomic transaction. `sync_all` does
not claim crash/power-loss durability for an unsynced parent directory; Drop
cleanup does not run after SIGKILL or process termination.

Current implementation audit (2026-09-30):

| Writer | Implemented host behavior | Remaining conditions |
| --- | --- | --- |
| AFC CLI pull | Bounded core read followed by private staging/commit | Real device transfer and macOS/Windows execution |
| CoreDevice CLI pull | Streams into staging; commits only after successful transfer and sync | RSD fileservice device and macOS/Windows execution |
| Screenshot/CoreDevice image | Same guard and commit as file pull; no-clobber and destination-type policy | Device image validation and platform execution |
| OS trace archive/collect | Staging, archive/path validation, cancellation guards; collect uses exclusive directory publication | Platform execution and filesystem support for exclusive rename |
| JUnit | Internal private staging, sync and shared replacement; longest filename is supported | macOS/Windows execution |
| Backup2 | Root/layout checks; incoming files, metadata and copied files now stage before per-file commit; existing manifest patch staging retained | Platform and real backup acceptance; no transaction across the complete backup |

The earlier audit incorrectly described all Backup2 writes as staged. Only
manifest patch/extraction had that foundation; incoming transfers, metadata
initialization and regular-file copies previously truncated targets. They now
use an internal staging helper, without adding a new public protocol API.

CLI file/image `--force` replaces only regular files and refuses symlink or
directory destinations on all platforms. Without `--force`, atomic hard-link
creation protects a destination created after the initial check; filesystems
without hard-link support fail explicitly. No copy/overwrite fallback weakens
this policy. On Unix, a read-only regular file can still be replaced when the
parent permits rename; the new private file has mode 0600. Windows read-only
replacement failure has a separate regression that awaits Windows execution.

Backup2's established DeviceLink flow tolerates per-file device remote errors
and continues receiving subsequent entries. Such a file's staging is discarded
and its old bytes survive, with a redacted warning. A successful overall
DeviceLink loop does not certify every requested file; verify the expected
artifact manifest. EOF and cancellation fail/drop the operation and preserve
the interrupted file. Filtered placeholders are published only after a
successful transfer; wire field names and command sequencing are unchanged.

Shared `ios_core::fs_replace::move_file_replace` uses Unix rename or Windows
MoveFileExW replace-existing/write-through. It does not silently copy across
volumes. On replacement failure, the source remains caller-owned and the old
destination is preserved. Replacement does not preserve the old Windows ACL
and attributes; a private output mode is intentional.

OS trace collect exclusively publishes its new directory: Linux
[`renameat2(RENAME_NOREPLACE)`](https://man7.org/linux/man-pages/man2/renameat2.2.html),
macOS [`renamex_np(RENAME_EXCL)`](https://developer.apple.com/documentation/foundation/urlresourcevalues/volumesupportsexclusiverenaming),
or Windows [`MoveFileExW` without replace-existing](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).
An existing directory, including an empty one created concurrently, causes a
failure. Unsupported systems/filesystems return an error, with no overwriting
rename fallback. macOS/Windows branches are implemented but not locally run.

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
