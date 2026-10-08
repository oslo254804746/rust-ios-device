# Deferred tunnel failure bookkeeping

Implementation `4415be9a65921a346208f912dbe59e399e471591`,parent `14056003d583f174cb0bc577db577b41d3dcfd77`;accepted 2026-10-08 UTC.
Linux x86_64,Rust1.97.1/MSRV1.80.1,project Python3.12.13.
7 source files 714+/9-;12 additive regressions. All 26 preceding tunnel test files/assertions stay,
except two additive module declarations. Public APIs/options/error enums/Core/bindings/
wire/deps/features/Cargo.lock stay. Ordinary L01/L02,not Q04 or its UID experiment.

A failed candidate now yields its identity and owned failure permit to the scanner.
The scanner moves the original UDID/Ticket into an intent and finishes its four-item
connection window before queueing any writer for these failure outcomes. Snapshot
admission bounds candidates/intents to4096 and their owned ID+route-host strings to
16MiB. The original discovery15s/per-candidate15s and per-device cancellation stay.
Existing cfgtest direct refresh_connect/record_failed_attempt wrappers keep preceding
Pending/backoff/permit/native assertions unchanged;new native fixtures use actual scans.

Nonempty intents share one private checked absolute1s bookkeeping clock,starting
at flush admission and covering failure write-lock wait plus the entire mutation loop.
It is not reset per row. Empty intents require no write/timer. Zero/overflow private
budgets refuse writing with actualElapsed/InvalidInput and release all pending permits.
A stale device intent is skipped;stopA does not cancel B's intent. This slice does not
prove immediate cancellation while the shared write-lock wait is pending.

Each row commits under its original ticket epoch fence. Pure time checks occur before
and after actual insertion while the failure-table/ticket guards are held. A late new
row is removed;an updated old row is restored with its exact timestamp/count/identity/
permit. Displaced row owners drop after both guards release. Earlier timely prefix rows
remain;the batch is not an atomic transaction. Pending/unwritten intents are dropped
on timeout/callerDrop and do not create backoff. The shared1s timeout is a scan
bookkeeping error,not a new failed connection. New readiness waits for successful flush.
Existing prior readiness is not cleared merely by incomplete bookkeeping.

An acquired original paired Source Err or admission Incomplete remains primary when
flush also fails. original.context(FailureContext) preserves direct original IO downcast;
private context retains the independent actualElapsed secondary and redacted class-only
Debug. CLI typed resource classification is checked through actual CLI JSON output,
not assigned to Core's unknown private CLI type. No error enum/class change.

Backoff identity/count/formula/permit rules stay. last_attempt still uses metadata
commit time;deferring commit to window completion can postpone an early failure's
next retry. This timing and new bounded bookkeeping rejection need pre-1.0 review.
Successful candidate failed-row removal still runs inside its candidate clock and may
queue a writer;this slice does not solve every RwLock fairness/metadata wait.
Whole-scan total/global cancellation/durable primary across callerDrop,reconciliation/
ready waits/global retained metadata and all other original conditions remain.

Four actual native fixtures use private metadata/real userspace workers/original Core
reservations plus loopback TCP one-byte EOF. With the failure table read-held after
firstfour native admissions,allfour original EOF failures retire while the fifth native
candidate enters. Its attached pending handle is Unpublished (not Inflight);it is live
before scannerDrop and retires after Drop. Failed intents do not queue early writers.
Held failure-table flush expires before the reader releases;original paired TCP EOF
and oversized Source-success resource error remain primary with actualElapsed secondary.
Every fixture drains original native joins/checks all original Arc statuses/actual duplex
EOF/taskbaseline/registry closed-and-empty/permits before regression assertions.
These are not mobdev2/DNS/pairing/device or physical freshness/reuse/restart proofs.

Old production13133 terminal101/0PASS4FAIL,458 frozen product hashes;allnative recovery
completed before failures. V1 29280=9PASS2FAIL:new fixture sampled fifth entry before
fourth EOF and tested private CLI Resource via Core classification. V2 82588=10PASS1FAIL:
new fixture mislabeled attached pending registry handle as Inflight. Root corrected only
new assertions/observation synchronization;old assertions/Core types unchanged. Exact
failed sources/logs are archived. Disabled-only postinsert rollback69833 terminal101/
0PASS2FAIL proves outerElapsed alone cannot prevent late new rows/oldtime-count mutation.
Canonical rollback bytes were restored before adding shared-two-writes proof/final gates.

Final5784 authoritative terminal0,all7SERIAL gates freeze460 products:
CLI242/1existingignored,minCLIalltargets/originalstack5/strictworkspace/fmt/
section11 full/.venvtools73 PASS. Full2766Rust/13Python,CLI1105/Core1496/
public55,0FAIL/2existingignored. MSRV1.80.1/66features/fourgroups/docs7+7/snippets3/
C ABI/Python3.12 rebuiltwheel-sdist/install/import/help118/raw38/host5/CLI95/backup3 PASS.
22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN;no maturity/ACK/platform upgrade.
PatchSHA256 `3cae15f13acb5092ee0e209a04c7091a1ed053e1b20b540deeeb975c348307e6`;actualiosSHA256 `2c72568a0835e2af5dbd976c534d409db8dc6727a013c35007c73753613e117e`.
Ignored source/command/exit/log/hash evidence `target/validation/tunnel-bookkeeping-2026-10-08/`.
Root alone ALLRustserial/CARGO_INCREMENTAL=0,no product/source/docs edits during live
validation;peers read-only/message-only. Reference HEAD/status unchanged;no cache
deletion/push/publish/history rewrite/device/credential/Apple/elevated TUN actions.

Active all30 original main tasks and all children excluding Q04 ONLY:
9complete/21unfinished(9notstarted/12progress-or-conditions);UID untouched.
Whole-scan independent total/global cancellation,paired/List primary across outerDrop,
out-of-clock reconciliation/readiness/global metadata,physical freshness/reuse/restart/
manual epochs/HTTP/upgrades/blocking/startup,L03-L04/classes5-9/all remaining original
acceptance stay. This logical admission/metadata proof is not allocator/RSS/CPU/
throughput/long-run certification. No main task,class4 or goal completion is claimed.
