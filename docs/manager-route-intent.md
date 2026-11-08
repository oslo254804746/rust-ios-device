# Manager route recovery and durable stop

An authenticated successful `POST /start-tunnel` establishes a manual recovery
intent for its explicit device. Recovery retains the transport family and caller
filter of the route that actually succeeded. A live-route shortcut reauthorizes
that published route; a different requested transport has not run Source and
therefore cannot become the recovery route.

Each recovery attempt uses fresh production discovery and `Source::connect_owned`.
USB attachment IDs, paired mobdev2 addresses, RSD directories and service metadata
are rediscovered through the existing Source path. Hostless Wi-Fi uses that path's
current discovery; a pinned filter remains pinned. A partial discovery failure
retains the intent and existing bounded retry episode. Recovery cannot substitute
a different transport or borrow another credential's device permission.

The intent retains the admitting grant's policy revision and expiration evidence,
without retaining its bearer or an obsolete policy snapshot. Manager scans reject
expired/reloaded authority, cancel a pending recovery and retire its original
owner. This is scan-driven enforcement, not a promise of an independent immediate
expiry timer. An independently authorized later successful POST establishes a new
version that obsolete scans and cleanup cannot stop.

## Stop, reset and admission

`DELETE /tunnel/{udid}` records a durable stopped intent even when its response is
404 because no live route exists. Subsequent discovery, replug and scanner
restarts within the same manager state cannot recreate that device's tunnel.
Only a successful authorized explicit start or reusable global clear/reset admits
it again. Denied, failed, cancelled or late starts leave it stopped. The state is
in memory; restarting the manager creates a fresh state.

Automatic gates, manual intents and stopped keys share the existing 4096 device
coordinator limit. Keys and retained host filters are each limited to 4096 UTF-8
bytes before admission. Idle Automatic gates may be pruned; persistent Manual and
Stopped states are never evicted to make an unknown key appear successful. At
capacity, a new unknown stop/start returns a typed resource-limit failure before
Source. A known key can still be queried, stopped and recovered. Unknown discovery
keys rejected by coordinator capacity do not prevent an otherwise admitted known
manual candidate from reaching the four-connection window. Snapshot and combined
candidate admission limits still apply to the whole batch.

A clear advances the global epoch. Lazy reset keeps canonical gates while old
tickets retain them, so repeated clear cannot bypass the live coordinator limit.
Terminal shutdown seals admission. Persistent intent adds no independent device
worker, retry history, failure permit or unbounded event queue.

## Publication and original errors

Manual publication holds its lifecycle and authority revision fences. After every
mutable intent lock is acquired it checks both authorization and connection
clocks; it checks again after writing the candidate. A late candidate restores the
exact prior desired version and pending-success identities without notifying
subscribers. A late new tunnel publication also restores the exact displaced row.
Retirement of the late original native owner occurs after table/watch guards are
released. Synchronous locks and OS work retain the existing cooperative-clock
limitations.

Successful manual recovery invalidates obsolete failure versions. Contested
failure-table cleanup keeps one exact-version pending outcome on the existing
gate. It cannot block a later device's connection window or remove a newer
failure. The original bounded retry and native retirement registries still own
their permits and joins; this does not promise that every retired worker joins
before every subsequent retry.

Discovery retains at most the first acquired USB, paired and coordinator error
in fixed slots. USB, then paired, then coordinator determines the original
primary; one typed safe context retains independent secondary/completion causes.
Repeated unknown-key refusals cannot extend an error chain. Caller Drop transfers
these originals into the existing eight-slot scan history before admitting
another scan. One-source callers retain their existing secondary-cause accessor.
Arbitrary native error payloads are not a global byte/RSS certification.

## Acceptance boundary

The manager host work item requires real authenticated HTTP and production scans,
actual native workers, original join certificates, strict peer EOF and recovered
registry/permits/tasks. Fixtures cover manual route selection, fresh metadata,
stopped replug, original grant revocation, capacity, old/new failure versions,
A/B isolation and twelve real clock crossings at final publication. The original
scan setup/reload and retry contracts remain required.

Darwin `pasteboard watch` recovery, physical unplug/Wi-Fi/service restart/address
changes, macOS/Windows/remoted and all original L01–L04 conditions remain separate.
The user will arrange physical testing later. No implicit pairing or mutation
replay is introduced, and capability maturity does not change from host fixtures.
