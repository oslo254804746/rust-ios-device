# Backup host and output contracts

The [architecture](architecture.md) separates wire framing, host path policy,
selection, local manifest work and DeviceLink session execution. Public Rust
backup paths, fields and signatures remain available. `backup2` is always
compiled; `backup2-manifest` enables local index/filter/extraction operations.
Public local filesystem functions remain synchronous. Async core/CLI callers
use owned blocking jobs for disk operations and retain original error sources.

Mobilebackup2Client now reports local is_usable and accepts an optional total
operation budget. It owns negotiation, host work, raw transfer and cleanup;
interrupted progress requires a fresh transport. Ordinary default timing remains
unbounded; Unback/Extract retain their five-minute ceiling. Cleanup respects the
remaining total budget and the independent five-second disconnect cap, retaining
both primary and unexpected cleanup failures through cleanup_failure(). Expected
peer closure after a complete terminal reply retains the prior completion policy.
Drop cannot await disconnect or stop an already running blocking worker. These
are host-tested ownership rules, with device backup/restore acceptance pending.
See [lifecycle evidence and migration](validation/backup-lifecycle-2026-10-04.md).

`ios backup list-local <BACKUP_DIRECTORY> --source <IDENTIFIER>` defaults to a
pretty JSON array with `file_id`, `domain`, `relative_path`, integer `mode`,
integer `size` and `link_target` (null when absent). With global `--no-json`, each
row remains `domain<TAB>relative_path<TAB>size bytes<TAB>mode=0x...` followed by a
newline. Other backup success formats and command parameters are retained.
Local commands inspect only the named backup; they do not select/contact a
device. Explicit source takes precedence over the global UDID fallback.

Backup result output now returns I/O errors from writing content or the final
newline. A closed pipe produces exit **1**, a redacted `backup failed:` diagnostic
on stderr with the IO kind, and no panic. The earlier `println!` path could panic
and exit **101**. Consumers matching 101 should treat 1 as the command failure;
neither exit indicates a complete result. Partial human/JSON bytes can already
have been written before an error. A successful device/file operation can also
precede an output failure; failure does not imply rollback or authorize retrying
side effects. Successful stdout and JSON schemas remain unchanged.

Restore metadata/application preflight now runs on a blocking worker before
opening the device connection. An encrypted backup without a password keeps
its existing password-required priority. Invalid local application metadata can
therefore fail before device access. Confirmation and password requirements are
retained; this is not a preflight or rollback guarantee for device restoration.

Filtered uploads still use completed empty placeholders when later device
move/copy requests need them. Exact-file relocation now tracks the destination
without an empty trailing path component; previously cleanup could fail with
`NotADirectory` on Linux. The accepted flow verifies move/copy/missing-source/
directory listing/remove replies, final disconnect, placeholder removal and
preservation of unrelated backup data. Directory descendants retain their
relative suffixes. Existing path/symlink guards and file/byte/metadata limits
are retained; native Windows/macOS and physical backup acceptance stay separate.

Run `python3 scripts/validate.py backup-contracts` after building `ios`. The Linux
runner creates one private synthetic plist/SQLite backup, captures original CLI
stdout/stderr in RAM with bounded owned-process cleanup, and checks exact nonempty
human/JSON bytes plus the closed-pipe failure. An owned loopback usbmux trap
requires zero connection attempts. It reports counts/booleans, removes
only unchanged owned files/directories, and preserves unexpected/replaced data
with a failing result. No raw device archive, pairing, restore, DDI or TSS is used.
CI/full validation include this scope. Mainline results and exact commits are
recorded in the [roadmap](engineering-roadmap-2026-09-30.md).
