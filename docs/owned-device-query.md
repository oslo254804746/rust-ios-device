# Finite existing-trust device queries

`ios_core::session::ExistingTrustDevice` stores one explicit device identifier,
an already loaded `Arc<PairRecord>`, a `SessionOwner` and a `UsbmuxEndpoint`.
Construction performs no IO. Each `prepare_query` reserves a lease immediately,
starts the absolute total before polling and captures immutable routing/credentials
without retaining a hidden client handle. Shared admission is explicit per pool
(1–64); each owner accepts 1–4 concurrent operations, with no waiter queue.

The numeric TCP or Unix endpoint avoids DNS workers. `system()` snapshots the
existing environment/default endpoint without IO and rejects configured hostnames.
A zero TCP port, empty/NUL identifier and identifier above 4096 UTF-8 bytes reject
before dialing. `PairRecord::from_bytes` accepts caller-loaded bytes without files,
rejects empty/above 1MiB input before decoding and retains the original plist parse
cause. Its parsed private key remains zeroized; caller-owned input is not wiped.

Every operation owns a fresh socket: ListDevices, exact-UDID selection, Connect
(port 62078), QueryType, StartSession, optional real TLS and GetValue. The first
case-insensitive USB match wins; otherwise the last matching non-USB entry wins,
as in existing Core/CLI selection. All decoded discovery entries are admitted
before selected metadata is cloned. Missing ProductID differs from advertised zero;
malformed aliases/fields reject the checked query. An optional ListDevices
MessageType must match if present. Legacy APIs retain their existing profile.

Select ProductType, ProductVersion or DeviceName with `DeviceQuery`, or use
`DeviceQuery::root(DeviceInfoFields)` for a nonempty subset of those three fields.
The owned observation returns only selected strings, connection metadata and the
session SSL flag. Missing Value has `value_present=false`; a present empty root
has `true` with absent selected fields; an empty string remains `Some("")`.
Wrong selected types are Protocol. A remote Error is authoritative over Value;
remote MissingValue remains NotFound. Unknown nonzero mux Result codes retain a
public `MuxResponseError`/code and classify ServiceFailure without guessed meanings.

`DeviceQueryLimits::new(reply_wire, output_utf8)` admits nonzero logical budgets.
Wire counts the declared complete plaintext replies, including two 16-byte mux
headers and three 4-byte Lockdown prefixes, before body allocation/read. Its hard
maximum is `2*16MiB + 3*(4 + 4MiB)`. Per-frame builders can lower the existing
16MiB mux/4MiB Lockdown caps. Metadata defaults to 1024 entries/16MiB aggregate
SerialNumber+ConnectionType UTF-8; builders admit 1–1,000,000 entries/1–16MiB.
Output counts ConnectionType plus selected strings before cloning, at most 16MiB.
Requests, TLS records/handshake bytes, credentials and parser temporary allocations
are outside these logical budgets. They do not certify CPU or peak RSS.

Cancellation, total expiry, malformed framing and native errors retire the original
socket; another query rediscovers a fresh DeviceID. No automatic retry, Pair/save,
StartService, advertised-port execution or tunnel is performed. `request_close`
seals admission and cancels active leases. `close(cleanup)` waits for their actual
retirement; timeout keeps the owner closed and counts/permits outstanding, so a
later close can finish after the original jobs retire. Drop broadcasts cancellation
when the last client handle disappears; it does not acknowledge remote cleanup.
The existing owner native-error/late-result rules and cooperative CPU/OS limitations
continue. Acceptance.

This Rust subitem supplies device query inputs for the Python/C work. Bindings,
applications, files, log streams, their examples and representative physical
workflows retain the original A03 acceptance. Hardware testing is deferred to the
user's later arrangement; no physical support is inferred from this host fixture.
