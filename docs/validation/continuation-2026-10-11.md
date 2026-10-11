# Review repair continuation — 2026-10-11

This batch follows `e9cf144c` (product source `88d8f707`). The original independent
review and the original 30-item scope, excluding Q04 and all its children, still
govern acceptance. This document does not declare the whole scope complete.

## Product changes

The manager now retires an alive paired publication when a complete admitted
observation changes/removes its actual UDID/address. An older observation cannot
remove a newer publication. Failed observations preserve the original owner.
Known metadata charges follow both the table and original native retirement
owner, so removing either alone cannot prematurely release the shared permit.
Automatic Source failures retain first/latest original objects and deferred-write
completion, including cancellation and the independent bookkeeping cause.
Opaque original-error heap accounting and same-address remote RSD freshness
remain unresolved. See the metadata/freshness report for the exact boundary.

Doctor now performs an actual checked CopyDevices mounted observation and a
bounded read-only legacy Instruments `runningProcesses` operation. macOS remoted
coexistence uses before/after launchctl state/PID and selected RSD identity.
Owned host processes have bounded startup/output, cooperative cancellation and
independent kill/wait cleanup. Windows forced process termination can produce a
real ConnectionReset: the fixture records `strict_peer_eof: false` for that
result. It is not converted to EOF or used as native tunnel reclamation evidence.
iOS 17+ checked RSD developer-operation adaptation and actual macOS/hardware
observations remain separate gaps.

XCTest retains immutable attempt histories and atomic `.attempts.json` artifacts;
the first failure survives a passing rerun. Partial results belong to the session
owner, and reporting/cleanup errors preserve primary and secondary causes.
Startup connections and acknowledged PIDs now belong to a cleanup object outside
the cancellable workflow. It closes original DTX transports and terminates only
a known PID. DTX message type 4 retains its error identity and original decoded
object; channel/session requests cannot accept it as a normal reply. Close sends
write shutdown and drains a bounded amount of raw data to actual EOF. A reset
remains an error. A launch whose reply never yields a PID still has unknown
device effects. Crash collection and the remaining platform/native-owner scope
must not be inferred complete from these legacy peer tests.

Windows async command states are split and boxed. That alone did not solve the
actual native CLI stack overflow: the PE now reserves 8 MiB for the Windows CLI
stack with the original 4 KiB initial commit. The rebuilt native application
install/query/launch/stop/uninstall workflow passed. This is an executable build
setting, not a change to DLL host stacks. The linker distinguishes reservation
from commit. [Microsoft linker documentation](https://learn.microsoft.com/en-us/cpp/build/reference/stack-stack-allocations?view=msvc-170).

Q02 gained strict Cryptex and POSIX rich-clipboard observers, keeping fixture
hashes separate from authorization and preserving preexisting state. Actual
application/XCTest TLS-usbmux/ZIP-conduit/DTX/syslog peers now exercise production
commands, failures, deadlines, partial installation, reporting and reruns.
Private MAC/altIRK, modern RSD handshake selection, Display/HID restored-state
and InstallCoordination wire/same-client observers still need production receipts.
Cryptex hardware/TSS and POSIX watch on this Windows host remain NOT_RUN.

## Resource observations

Every measured native ICMP/AFC/Syslog operation uses the same pinned future
throughout its concurrent sampler. A sampling tick cannot cancel a partial
native frame or syslog message. Windows observes working set/private commit,
own handle count, CPU time and IPv4/IPv6 owner-PID TCP endpoint rows. TCP rows
are not relabelled as socket handles or Unix file descriptors. EX2 additionally
observes private/non-private working set when the OS supplies it, with explicit
nulls on older systems. [Microsoft memory counter documentation](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex2).

The initial exact-overlay `af20f6a` run passed: 1/4/16, one scale sample,
16 bounded start/stop/disconnect cycles, 256 KiB transfers and actual 500 ms
Syslog windows. Its long run was 163894 ms. Released handles stayed 154,
TCP endpoint rows 3, Tokio tasks 5 and native occupied slots 0. Private commit
ended 12288 bytes above its first long-run sample and was non-monotonic; RSS
ended 81920 bytes higher. These observations do not identify every allocation.
The EX2 refinement and matching repaired-source run still require completion
before accepting a resource comparison or closing L04.

## Actual gates to date

* Strict workspace Clippy with all targets/features: PASS.
* Core full unit suite: 1625 PASS, 0 FAIL, 1 existing ignored.
* CLI full unit suite: 1296 PASS, 0 FAIL, 6 ignored (including the owned
  child-process fixture, invoked independently by its supervisor test).
* DTX codec suite: 48 PASS, including original partial-reply drain/strict EOF and
  both retained close failures.
* Q02 admission/parser suite: 16 PASS.
* Native workflow suite: initial 12-test run retained four failures/one error;
  repaired run retained one fixture idle-window failure. The observer timeout
  was corrected to cover the unchanged CLI 5 s total plus independent cleanup.
  Both deadline scenarios then passed with actual original EOF; the 12 s owned
  process cap and per-run <9 s assertion remain unchanged. The final adapter
  suite passed 29 executed tests with one POSIX-only skip, including all 12
  actual CLI application/XCTest tests and their stage subcases. The final native
  CLI SHA-256 is `49a94ad5a11aea21451b292e62f815da7c0e362aca463242342655777f09519c`.

`DtxPayload::Error` is a new Rust enum variant. Exhaustive Rust consumers must
handle it; the proxy JSON now reports `type: error` instead of `response` for
wire type 4. C/Python ABI declarations were not changed by this continuation.

Logs, exact binaries, source hashes and earlier failures remain under the repair
artifact directory. No physical device operation, push or publication occurred.
