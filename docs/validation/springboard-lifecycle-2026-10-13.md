# L01 SpringBoard request ownership — 2026-10-13

Started from `987c2f3` on `codex/review-fixes-20261010`. This is one service
profile under the [original L01 definition](../engineering-roadmap-2026-09-30.md),
not a closeout of L01. The implementation agent ran no Cargo, rustc, maturin,
device discovery or device operations. Root owns formatting and serial gates;
completed results and still-pending final gates are distinguished below.

## Original gap and implementation

[`SpringboardClient`](../../crates/ios-core/src/services/springboard/mod.rs)
previously sent a length/body/flush and then read a length/body in independent,
unbounded awaits. It held no request state. Dropping a request after a partial
send/receive left the original client available for another command, which
could consume the abandoned request's late response.

All nine public operations now use a single tracked request: icon listing/raw
state, icon PNG, orientation, icon metrics, wallpaper info/preview/Home Screen
PNG, and setIconState. The original command names, serialized fields, receive
limit of 16 MiB, typed result parsing, public call signatures, and error enum
remain. Request-length conversion now checks u32 instead of silently truncating
an oversized encoded request and retains its original TryFromIntError.

The additive `set_operation_timeout(Some(duration))` starts one checked absolute
clock before encoding and shares it through send/flush/receive/response parsing.
`None`, including the default, preserves legacy unbounded waits. A zero or
overflowing duration rejects before encoding or IO and keeps the stream usable.
Encoding/parsing is synchronous and cannot be preempted; the clock is checked
after encoding and after a completed response so exhausted CPU work cannot
start a request or return success. Original failures take precedence over the
post-completion clock check. Real Tokio expiry retains its actual Elapsed;
zero/admission/post-completion expiry has no fabricated Elapsed source.

The same absolute clock also fences each original read/write/flush/shutdown poll.
Tokio timeout polls its body before its timer, so an earlier non-yielding Ready
IO poll can spend the remaining budget. An expired fence returns Pending without
polling `S`; the enclosing timeout then observes its actual timer/Elapsed. An
original Ready(Err) already observed while crossing the clock remains primary.

[`SpringboardRequest`](../../crates/ios-core/src/services/springboard/lifecycle.rs)
owns the exact original `S` while work is pending. Accepted write/read bytes
commit the request. Native IO failure, EOF/WriteZero, a committed failed result,
or caller Drop after progress retires that owner before returning and makes
every old entry point reject reuse. Untouched pending IO cancelled/expired
before any progress restores the same stream. Successful complete requests
restore the stream unless the transport reported a terminal condition. This is
a local reuse rule, not peer liveness or permission to retry a mutation.

The historical `setIconState` rule tolerates UnexpectedEof and still returns
success for that condition, including a partial response header. Tracking makes
that stream terminal and releases it. Other getter EOF remains the original IO
error. This change does not claim EOF proves a remote icon mutation or introduce
an acknowledgement that the existing protocol code did not require.

No async shutdown command or remote cleanup acknowledgement is invented.
Retirement is destruction of the client's original stream; its primary failure
is returned intact. A caller-retained alias or a custom `S::Drop` which blocks
is outside this ownership/timer guarantee. There is no new blanket native-worker
join or hard bound on arbitrary synchronous Drop implementations.

## Fixtures and serial validation

CLI production wiring sets a positive 30,000ms per-request default through
`--request-timeout-ms`; the flag is also accepted after the subcommand. Core
callers may still explicitly retain the unbounded compatibility setting. Device
connection/service startup and a whole multi-request CLI workflow are separate
remaining budgets; this addition does not silently claim to cover them.

Twelve new tests in
[`lifecycle_tests.rs`](../../crates/ios-core/src/services/springboard/lifecycle_tests.rs)
cover all-entry preflight/rejection, unpolled calls, before-progress and partial
send cancellation/expiry, fragmented headers/bodies, late replies, one combined
send/read total, queued malformed/oversized/typed failures, legacy set EOF and
strict getter EOF, original native IO payload identity, and synchronous parse
completion after the clock with primary-error precedence. Existing successful
wire/typed tests are retained with their original assertions.
Two controlled non-yielding IO cases count actual underlying polls and require
no next-stage send/read after a header/flush crosses the clock; a native failure
crossing the clock retains the original IO payload and does not acquire a timer
source.

The owned TCP fixture binds only `127.0.0.1:0`, repeats two cancellation and two
expiry cycles, requires peer `read` to return exactly zero, and joins the original
server task while the failed client is still alive. It does not accept reset as
EOF or replace the original task. Duplex fixtures also assert actual original
peer EOF; a late write must fail after retirement. No fixture discovers devices,
contacts usbmuxd, pairs, or changes device state.

| Gate, executed serially by root | Status |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo test -p ios-core --no-default-features --features springboard --lib services::springboard` | Initial compile FAIL (two new fixture key types); repaired source: 21 PASS, 12 new + 9 existing |
| `cargo test -p ios-core --all-features --lib services::springboard` | 21 PASS, 0 FAIL; minimal-feature run repeated after fixture safe Debug repair also 21 PASS |
| `cargo clippy -p ios-core --all-features --all-targets -- -D warnings` | PASS |
| CLI budget parser / strict CLI Clippy / CLI build | 1 PASS / PASS / PASS |
| Readonly manager / host runner / capability inventory | 1 PASS with Source zero connections / 5 PASS, 22 NOT_RUN / 119 records PASS |
| Full workspace/SDK feature matrix | NOT_RUN; changed service and CLI scopes checked above |
| Real SpringBoard service/device, other platforms | NOT_RUN |

An additional minimal-feature strict Clippy attempt failed on existing unrelated
dead-code declarations in error/context, session transport and tunnel MTU helpers.
The supported all-feature strict gate passed without adding allow attributes or
changing those declarations. Both attempt logs are retained.

The diagnostic Debug scanner initially rejected the new fixture's derived
Debug. It now uses a fixed safe description; scanner and original runtime
assertions remain unchanged. Diagnostics tooling then passed 7 tests with one
Unix-only skip. Core all-target/all-feature Clippy was rerun and passed after
this fixture-only change. Source/binary/complete log hashes are frozen in
`repair/l01-springboard/summary.json`; the common CLI build also contains the
separately committed desired-list and doctor work. Final HEAD is checked against
that full frozen source manifest after all three commits.

Root's first minimal gate failed compilation with E0283 in two new fixture
dictionary keys using ambiguous `.into()`. Explicit `to_string()` repairs only
fixture key types. The original log remains at root's external
`repair/l01-springboard/springboard-tests.log`; it is not counted as a passing
runtime gate. Root's pre-gate review also identified the body-first timeout IO
gap, now covered by the per-poll fence and added clock-crossing fixtures. No
existing assertion was relaxed to accommodate either correction.

## Concrete remaining service gaps

This focused source audit also confirmed independently actionable gaps in other
production files; none was changed or classified as merely waiting for hardware.

| Path | Existing production evidence and missing contract |
| --- | --- |
| [`diagnostics.rs`](../../crates/ios-core/src/services/diagnostics.rs) | Borrowed `&mut S` query functions directly await send and receive; no client owner/state or total-budget API prevents reuse after cancellation. Reboot/shutdown have separate existing EOF semantics requiring preservation. |
| [`screenshot/mod.rs`](../../crates/ios-core/src/services/screenshot/mod.rs) | Borrowed `take_screenshot` spans version exchange and DeviceReady through independent awaits; no shared total or state invalidation. Owned/checked adaptation must cover the entire handshake rather than only the final image read. |
| [`misagent/mod.rs`](../../crates/ios-core/src/services/misagent/mod.rs) | `MisagentClient` retains raw `S` and directly awaits CopyAll/Install/Remove send/read without a deadline or cancellation guard. A late mutation reply can be consumed by a later request. Remote retry safety is a separate requirement. |
| [`house_arrest/mod.rs`](../../crates/ios-core/src/services/house_arrest/mod.rs) | Vend consumes the client, so cancellation releases its local stream; send/read has no total budget. Successful AFC handoff and existing service-response priority must be preserved by a finite-budget adaptation. |
| [`heartbeat/mod.rs`](../../crates/ios-core/src/services/heartbeat/mod.rs) | `recv_message` reads a frame into temporary buffers without partial-frame cancellation state; `ping` then sends Polo. Idle listening needs its own cancellable wait and must not inherit a short finite-request timer. |

Existing AFC/Browse/Syslog/SDK/Restore/WebInspector/notification and tunnel
profiles retain their own evidence. This audit is not a complete inventory of
all services, HTTP2 flow-control/upgrade/startup paths, blocking work, native
worker recovery or cleanup profiles. SpringBoard defaults and existing CLI
callers remain unbounded unless they explicitly choose the new operation budget.
L01, capability maturity, platform/device support and the task denominator stay
unchanged; Q04 and all of its excluded subitems stay excluded.
