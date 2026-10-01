# Historical plist/JSON and output contracts

R04 starts from the accepted R02 implementation `07ec8977fd5e8e421982348bcc79e376909d698e`
and its clean documentation checkpoint `612b480ef769575026cdbd49faf15faf5fd8f67d`.
The frozen baseline below records existing behavior before shared conversion.
The new `ios_core::plist_json` API selects that behavior explicitly; its caller
regressions and complete host acceptance are recorded in the roadmap.

## Encoding inventory

The legacy hand-written conversions recurse through arrays/dictionaries, preserve
strings/booleans and signed or unsigned integers, and turn non-finite reals into
JSON null. Explicit metadata support differs between legacy commands.

| Existing owner | Data | Date | Uid | Integer fallback |
| --- | --- | --- | --- | --- |
| core diagnostics/accessibility audit; CLI apps/heartbeat | Numeric byte array | XML UTC string | Unsigned JSON integer | null |
| core dproxy; CLI lockdown/diskspace/springboard/DDI/os-trace; FFI lockdown JSON | Lowercase hexadecimal string | XML UTC string | Unsigned JSON integer | null |
| CLI info/diagnostics | Lowercase hexadecimal string | null | null | zero if neither signed nor unsigned is representable |
| CLI MDM non-secret values | Object with `data_base64` using standard base64 | XML UTC string | Unsigned JSON integer | null |
| Direct serde serialization of `plist::Value` | Numeric byte array | XML UTC string | Unsigned JSON integer | Existing plist/serde serializer |

Direct serde users include IDAM, preboard, power-assert, companion, activation,
profiles and local backup plist result rendering. Those historical paths remain
distinct from explicit hexadecimal rendering. Typed JSON/model output and
XPC/NSObject conversions are separate contracts; they must not be coerced into
plist behavior merely because they also produce JSON.

The existing wildcard branch of every hand-written conversion returns null for
an unsupported future plist value. Current `plist::Value` has no constructible
unknown variant, so this is reviewed behavior, not an executed unknown-value
test. Public `plist::Integer` constructors/parser in locked plist 1.7.1 produce
values representable as either i64 or u64; the legacy zero/null fallback cannot
be reached with those constructors. The distinction remains recorded for the
shared policy rather than silently discarded.

MDM redaction runs before ordinary conversion and handles arrays/dictionaries
with the original key context. Sensitive material remains `<redacted>`;
documented safe security status fields retain their values. Ordinary conversion
does not redact requested data. Diagnostic redaction is independently maintained
by the diagnostic boundary and must not be mixed into result conversion.

## Frozen baseline

Three crate-local test fixtures keep packaging self-contained. Fourteen tests
call the original private conversion functions in their actual owners; three
additional tests check direct plist serde value/pretty serialization. They fix
Data `[0,127,255]`, empty Data, a known Date, Uid/u64 maximum, i64 minimum, finite
and non-finite reals, strings/booleans and nested dictionary/array output. JSON
serialization/reparse also checks the complete fixed result and integer width.

```sh
cargo test --workspace --exclude ios-py --all-features --locked plist_json_
```

Linux 2026-10-01: **17 PASS, zero FAIL/ignored** (CLI eleven, core four, FFI two),
`/tmp/ios-r04-legacy-baseline.log`. Filtered integration targets have zero
selected tests and contribute no passing cases to this result. No conversion
implementation was changed for this baseline. Existing MDM secret-key tests
remain required alongside it during extraction.

## Shared conversion API

`ios_core::plist_json::{to_json, into_json}` convert borrowed or owned
`plist::Value` using `PlistJsonPolicy`. There is no default policy. Select
`DataEncoding::NumericArray`, `HexString` or `Base64Object` deliberately. The
default metadata behavior of `PlistJsonPolicy::new(encoding)` preserves Date
and Uid; the info/diagnostics delegates additionally call
`without_date_and_uid().with_zero_integer_fallback()` for their historical
profile. The new enum is non-exhaustive and policy fields are private.

Container traversal uses depth-first cursors with explicit frames. Owned
ordinary strings and keys move into the result; borrowed values are cloned as
needed. Wide inputs do not stage another queue of every sibling. This is an
implementation property, not a measured throughput or memory improvement.
Both entry points have a 4096-level synthetic traversal regression. It unwinds
fixtures/results iteratively; recursive JSON serialization and drop have
separate caller budgets and are not certified by that test.

These functions operate on constructed values, with no transport, parsing,
automatic redaction, truncation or silent size limit. Callers retain their input,
frame and output budgets. Existing public function signatures and C ABI stay
unchanged. The MDM key-aware redaction traversal remains separate; its seven
existing secret-key regressions are required with the encoding fixtures.

## Fallible stdout and migration

CLI result lines, human display helpers, streaming records and server startup
results use one fallible writer. Partial writes and the terminating newline
are part of the result. Output without a newline flushes explicitly. Six XML
sites retain the existing plist serializer and extra newline, then flush;
binary file-relay output already checks writes/flush, and Bluetooth capture
now checks its final flush too. Errors propagate through the original IO source
chain to the command boundary. Successful bytes and historical JSON formats
are preserved.

| Trigger | Previous behavior | Current behavior |
| --- | --- | --- |
| Result stdout fails, including a closed pipe | `println!`/`print!` could panic, exit 101 | Fallible result error, exit 1, safe `io/BrokenPipe` or other IO category on stderr |
| Help/version stdout fails | clap could discard the print error and exit 0 | Print error reported safely, exit 1 |
| Failure diagnostic stderr also fails | Printing could panic and replace the intended status | Best-effort error report retains the unsuccessful exit 1 or argument exit 2 |

A failure to print a result can follow completed device/file side effects.
It does not imply rollback, an absent operation or a safe retry. Output bytes
already received may be partial; callers must check the exit status before
accepting a final document. Streaming JSON and partial-operation semantics,
regular stderr progress/callback behavior, typed classifications and HTTP/C/
Python mappings remain A02 work. R04 does not certify orderly stream/device
cleanup after an output failure; lifecycle acceptance remains L01.

## Actual CLI regression fixtures

`python3 scripts/compatibility.py` runs the actual binary with an owned loopback
peer that accepts only one `ListDevices` request. Four existing JSON/human
empty/two-device contracts retain their complete output expectations. On Linux,
four additional owned closed-pipe cases cover JSON list, human list, help and
version, requiring exit 1, a safe IO category, no panic and complete process
cleanup. Other platforms explicitly report those cases NOT_RUN.

The accepted R02 binary passed the original four cases and failed the four new
failure contracts; the R04 build passed all eight. Three tool negative cases
reject silent success, panic text and a missing private executable. They passed
under Python 3.9.25, 3.13.15 and the current tooling interpreter. These are host
process tests, not installed-wheel or physical-device evidence. Backup retains
its three independent actual local-output contracts from R01.
