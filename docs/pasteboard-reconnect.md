# Darwin pasteboard watch recovery

`ios --udid DEVICE pasteboard watch` keeps recovery disabled by default. The
watch registers for Darwin notifications, takes a silent `allPromised` baseline,
and opens a fresh RemoteXPC PULL connection for each relevant notification. The
normal snapshot output and `--show-data` behavior remain unchanged. A failure
ends the default watch after bounded cleanup of its original native owner.

To opt into finite recovery:

```sh
ios --udid DEVICE pasteboard watch --reconnect \
  --reconnect-attempts 3 --reconnect-elapsed 60 \
  --reconnect-backoff-ms 250 --reconnect-max-backoff-ms 5000 \
  --connect-timeout 30 --cleanup-timeout 10 --timeout 600
```

An exact, nonempty UDID and pasteboard name are required; each selector is at
most 4096 UTF-8 bytes and cannot contain NUL. Recovery options apply to Darwin
notifications. `--autonotify --experimental` retains the experimental PUSH
path and rejects every Darwin recovery/timing option before device IO, including
explicitly supplied default values. Recovery does not set or resolve clipboard
contents, create trust records, or enable pairing. The native Source uses the
existing `connect_with_owner` path and reads existing trust on every attempt.

## Budgets, cancellation and cleanup

The initial connection is attempt zero. `--reconnect-attempts` allows 0–32
additional Source admissions across the watch. An attempt is counted only when
its fresh Source future is actually polled. Backoff doubles from the configured
minimum and saturates at the configured maximum; both values are 1–60000 ms.
Neither successful recovery nor a repeated notification resets the attempt cap
or the retained failure history.

`--connect-timeout` covers each fresh device connection, service opening and
silent baseline (1–300 seconds). `--reconnect-elapsed` is one absolute recovery
deadline starting at the first failure (1–3600 seconds), including cleanup,
backoff and subsequent setup. It is not reset by recovery. A healthy established
watch may continue after that recovery deadline; a later failure cannot start
another recovery. Optional `--timeout` covers the whole watch, including initial
setup, idle waits, reads, recovery and cleanup (1–604800 seconds).

Cleanup always requests stop on the exact original `TunnelHandle`. Its native
shutdown call gets the shortest of `--cleanup-timeout` (1–60 seconds), remaining
whole-watch time and remaining recovery time for a failed attempt. Cancellation
uses the whole-watch and cleanup clocks; it does not consume an additional
recovery attempt. Expired clocks do not permit a late Source poll, snapshot
output or new connection. A synchronous poll cannot be physically preempted;
an observed clock crossing still rejects its late success and preserves an
already acquired original error.

One Ctrl-C future is retained across connection, watch and backoff. A ready
signal fences each poll before work; cancellation during a partial frame drops
that service and monitor. Recovery never reuses the invalid parser. Original
native cleanup runs before another attempt. A local stop or an owned handle
alone does not certify collected native joins or a device acknowledgement.

If native shutdown cannot certify complete original joins within its allowed
clock, the watch ends with a cleanup failure and retains that exact owner in
the error. It does not start another Source or claim reclamation. An original
operation failure stays primary; native cleanup and completion-clock causes
remain independently retained. Even with completed joins, a fatal native
cleanup cause blocks recovery. Dropping the entire caller future requests stop
on its original owner; asynchronous joins cannot be certified by `Drop`.

## Recovery and clipboard gaps

Each admitted retry performs fresh device discovery, creates a new native
owner as needed, obtains fresh RSD/service metadata, registers a new notification
connection and takes another silent baseline. Addresses, service ports,
`ConnectedDevice`, XPC clients, streams and parser state are not replayed from
the failed attempt. Existing native ownership is attached before activation
and initial RSD work can be canceled.

Snapshots are suppressed when their `(nonce, changeCount)` equals the current
monitor baseline/last observation, or the last ID successfully written by the
watch across attempts. A nonce change is significant even when the count is
unchanged. A snapshot without a usable ID cannot be deduplicated by that ID;
it does not erase the last successfully emitted known ID. Output failures end
the watch and are never evidence to reconnect a transport.

Copies made during a disconnect are not replayed. The new baseline is silent,
so the watch does not promise lossless delivery, complete copy history,
exactly-once delivery or remote cleanup acknowledgement. `--reconnect` emits
fixed state records on stderr; `--json` makes those records JSON. Snapshot
records stay on stdout. State includes admitted attempt count, error category,
native reclamation, backoff and `changes_during_disconnect_not_replayed` after
an admitted retry. Raw selectors, payloads and error text are omitted from
these state records. Without `--reconnect`, the added state records are silent.

Transport loss, typed timeout/device-not-found, actual notification proxy death,
and EOF/refused connections under known XPC/notification/mux/tunnel owners may
recover. Protocol, capability, permission, input, resource and explicit
cancellation failures do not. Untyped host-file EOF is not guessed to mean a
device disconnect. Both original operation and retained native cleanup causes
must permit recovery.

## Host validation and remaining physical conditions

The new private fixtures exercise the production watch driver with actual
notification plist frames, H2/RemoteXPC initialization and PULL replies, decoded
RSD directories, and original `UserspaceTunDevice` packet/proxy workers. The
Source seam replaces device/USB discovery and route dialing. Every fixture
recovers strict `UnexpectedEof` on original peers, original native joins,
the single device/native owner slot and the exact Tokio task baseline before
evaluating the regression. Expiry fixtures retain and externally drain pending
original owners before assertions; they do not turn an incomplete receipt into
a successful driver cleanup result.

Run the focused fixtures, then the existing pasteboard CLI tests, serially:

```sh
cargo test -p ios-cli pasteboard_reconnect_ -- --test-threads=1
cargo test -p ios-cli cmd::pasteboard:: -- --test-threads=1
```

Coverage includes default behavior and PUSH rejection, selector/budget admission,
fresh identity and service metadata, silent initial/recovery baselines, unrelated
and duplicate notifications, nonce changes, cross-attempt last-output ID,
missing IDs, bounded retry/history/backoff, typed retry exclusions, cancellation
before setup/during setup/backoff/partial notification frames, partial RSD/XPC,
late ready Source/snapshot polls, output failure, original native capacity,
operation plus cleanup causes, cleanup/recovery/whole clocks and caller drop.

This delivery is statically checked only. Rust compilation, formatting and test
execution are **NOT_RUN** here and remain the root agent's serial gate. Actual
USB unplug/replug and device ID changes, Wi-Fi interruption/address changes,
device pasteboard/notification daemon restarts, device capability/permission
responses, pairing/trust state, RSD/remoted coexistence and macOS/Windows runs
remain **NOT_RUN**. Host protocol peers do not replace those conditions or
complete L03/L04 acceptance.
