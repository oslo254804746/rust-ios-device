# Manual tunnel deadline crossing during a synchronous poll

Implementation `a963679ab0f853b1a38bfcee9ac59471d4920641`, parent `9b2e355f4bd2237afd130beb0b0a45634aa4ea48`; Linux x86_64,
2026-10-07, Rust1.97.1/MSRV1.80.1, project Python3.12.13.
The goal remains active for all30 original main tasks and every child excluding
Q04:9complete/21unfinished (9notstarted/12inprogress-or-conditions).

## Behavior and ownership

The existing manual connection total starts before coordinator admission and
defaults to15s. Its earlier nominal-deadline guard checked only before polling
work. A Source or metadata method that blocked synchronously across the clock
could return Ready and continue querying or publishing in that same poll.
Default-route fallback could also contact paired discovery after acquiring a
late Source IO error. This profile fixes those specific boundaries.

The same original checked deadline now fences admission to Source, the completed
native handoff before metadata, successful metadata before attachment querying,
and synchronous publication before and after the real table insertion. If an
insertion crosses the clock, the table and lifecycle epoch fences remain held
while the unobserved new row is removed and the displaced original row restored.
After releasing both fences, the late runtime's lease retires and requests stop;
its exact original Arc and native joins remain in the existing bounded registry.
The restored old row and runtime stay live. This is local table restoration,
without any device-side transaction or rollback claim.

A private payload-free deadline decision returns control to the independently
polled original Tokio timer, preserving actual Elapsed and Timeout classification.
The wrapper also refuses a late successful Ready result. Already acquired native
IO errors remain the primary result even when their poll crossed the clock;
default-route fallback then stops before another discovery/transport contact.
An expired completed future is never repolled. This remains cooperative: Tokio
cannot interrupt synchronous CPU, metadata, output, destructor or test delays.

Automatic scan candidates still use their previous timeout policy and do not
receive these new manual fences. Whole-scan totals and generic HTTP/upgrades,
blocking workers, startup signals, metadata byte budgets, physical freshness,
ID reuse, daemon restart/manual epochs and other L01-L04 requirements continue.
Original Core/public APIs, options, stdout/HTTP payloads, error enums, dependencies,
features, Cargo.lock and every preceding test assertion remain. Successful routing
and fallback order remain; late success/error timing needs pre-1.0 review.

## Host fixtures and deliberate wrong control

4 source files357+/6-;7 additive tests. Five cases create actual userspace native Tokio
workers over DuplexStream and private CLI discovery/device metadata. Two exercise
the actual private deadline wrapper directly. The controlled fixture uses a200ms
operation budget and a250ms synchronous delay within one poll. These are causal
deadline tests, not scheduling, throughput, RSS or performance acceptance.

- Late metadata cannot query USB or publish.
- Late Source success cannot read metadata, query USB or publish.
- Late acquired BrokenPipe remains Disconnected without paired fallback contact.
- A generic late successful Ready becomes original Tokio Elapsed.
- A generic late PermissionDenied preserves the original native IO kind.
- Actual insertion into an empty table rolls back the unobserved new row.
- Actual replacement restores the exact original live native Arc and retires
  only the new runtime.

The first five tests ran against the preceding production implementation:
session98426 terminal101,1PASS/4FAIL. The original IO wrapper case passed;
late success, metadata/query/publication and default fallback boundaries failed.
All three actual-runtime cases recovered native joins, actual stream EOF and
task count before regression assertions. The exact baseline sources/logs and451
product hashes are retained. The first fix's five cases passed in51593, then
the complete seven passed in38308.

Control81695 disables only the post-insert deadline check/rollback. It exits101,
0PASS/1FAIL at the old-row/live-runtime regression assertion. Before that failure,
both exact original native runtimes joined, their actual streams reached EOF and
the task baseline recovered. Only the planned product file changed; the correct
implementation was restored to all451 frozen product hashes before final gates.
The callback that places a synchronous delay after real insertion is cfg(test),
private and holds no callback mutex while running. It introduces no production
hook or synthetic SDK device constructor.

## Accepted serial validation

Runner70521 terminal0. All seven gates checked451 product hashes before and after
their commands; root alone compiled and made no product/source/doc edits while
any validation process lived. Existing quota-exhausted agents supplied no new
delivery. Changed sources, failed baseline/control logs, every command/exit/log
hash, source review and frozen patch are retained under
`target/validation/deadline-crossing-2026-10-07/`.

| Check | Result |
| --- | --- |
| CLI tunnel behavior/regression |201PASS/1existing ignored, including all7 new |
| Minimum CLI all-targets |PASS |
| Original CLI stack-budget tests |5PASS |
| Strict workspace Clippy/format |PASS/PASS |
| Section11 full Rust/Python split |2725Rust/13Python,0FAIL/2existing ignored |
| Project .venv tooling |73PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full CLI1064/Core1496/public55. Workspace check/fmt/strict Clippy,
MSRV1.80.1,66features/four groups,7+7doctests/three downstream snippets,
C ABI and Python3.12 wheel/sdist rebuild/install/import passed. Tooling73,
actual CLI help118/raw diagnostic38/host workflows5/CLI compatibility95/backup3
passed.22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN. No maturity,
OS TUN, remote ACK, device cleanup, resource/long-run or platform certification.

Frozen patch SHA256 `35bd3891b6a7c152c1ffc1efbb493ca168dc8d62f8b1af988022cbd248f0c42b`.
Actual debug ios SHA256 `3e06a80ec5fd666eee4615c7060bfb9219591778d1565d2aef1ef97f5964fc9b`.
Committed implementation matches the tested freeze; original reference HEADs
and statuses remain. CARGO_INCREMENTAL=0 throughout; no cache deletion,
push, publication, history rewrite, credential/device/Apple service or elevated
TUN action. Q04 and all its children remain excluded; UID risk is untouched.
Every other original acceptance condition remains required. L01/L02/class4
and the overall goal remain in progress; no main-task count changes.
