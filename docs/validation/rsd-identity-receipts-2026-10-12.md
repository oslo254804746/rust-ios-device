# Q02 precise RSD and existing identity observations

This continuation preserves the original protocol implementations and records
their successful production branch/correlation instead of substituting general
device information. The editing agent ran no Cargo/Rust build, indirect build,
device operation or commit. The root agent owns final formatting, compilation,
source hashes and runtime evidence. The cases below are **NOT_RUN** until those
root gates are recorded; helper tests are not physical or native protocol proof.

`ios --udid TARGET rsd observe --address [IPv6]:PORT [--proxy-port PORT] --timeout 10`
uses the existing checked RSD identity/directory reader, one absolute clock and
fresh compatibility sockets. `handshake_mode` reports the actual successful
Active/direct, Queued, Legacy or Passive branch; `attempted_modes` lists branches
actually entered. It never reports userspace/kernel as a bootstrap mode. Missing
or mismatching identity and incomplete catalogue metadata cannot succeed. The
receipt carries the exact selected/current UID and sorted current service ports,
preserving missing feature metadata as null.

The observer's `transport_scope` is `existing-rsd-socket`.
`directory_socket_released` and `directory_release_evidence: local-raii` mean the
directory framer and locally owned socket were dropped before returning the
observation. They are not a peer EOF certificate. `cleanup_complete` refers only
to that local observer release. No new tunnel is created, and no external tunnel
or native worker join is claimed. Root localhost producer gates independently
check directory-worker EOF. The service cleanup receipt below covers only
subsequent service sockets and must not certify the directory socket's peer EOF.

`ExistingRsdSession` shares the same current catalogue and absolute operation
clock across service port selection, direct IPv6 scope or loopback proxy
prelude, optional original two-plist RSDCheckin, protocol bootstrap and queries.
Canonical entries take precedence even when their port metadata is incomplete.
RemoteXPC rejects a resolved shim before socket IO. The returned service stream
enforces actual cumulative read/write bytes (including proxy/checkin traffic)
and a real deadline wake; native Ready(Err) is preserved across a crossing poll.

The session retains original raw service socket owners before proxy/checkin IO.
Business terminal state cannot be reopened by cleanup. `close` seals admission
before taking any snapshot, wakes pending dial owners, registers same-poll late
successful sockets for retirement, and rejects later service opens. Independent
bounded cleanup performs raw shutdown/drain, preserves original errors and
unfinished owners on expiry, and allows retry. Its `strict_eof` certificate is
generated from actual service-stream EOF; drain bytes have their own cumulative
1 MiB ceiling. It creates no background worker. Generic owner Drop is a fallback
socket release, not a successful explicit cleanup certificate.

`ios --udid TARGET discover paired-identity --timeout 15` produces separate
`private-mac-lockdown` and `remote-pairing-altirk` observations, each linked by
type and index to its current browse advertisement. It does not export a MAC,
IRK, authTag, HostID, host address, hostname, credential path, or key. An empty
observation list means no completed matching observation and cannot certify
identity absence or satisfy the matrix case.

The private MAC receipt requires a present saved MAC that differs from the
advertisement and an actual accepted existing Lockdown session followed by a
matching UniqueDeviceID query. Host-tag candidacy alone cannot assign a UID.
MAC inequality does not prove the algorithm that generated the address. The
alternate-IRK receipt uses the validated existing-record loader and the exact
production cryptographic advertisement matcher, including all-record ambiguity
rejection before target filtering. `credential_validation_scope` is explicitly
`structure-and-key-material`; this is not Apple signature authentication or a
new online pair-verify acknowledgement. The two mechanisms never satisfy one
another. Existing file reads create no detached loader worker; physical blocking
filesystem cancellation remains a separate platform limitation.

Root gates to run on the final frozen source:

```text
cargo test -p ios-core --all-features existing_rsd_service_ -- --nocapture
cargo test -p ios-core --all-features rsd_selection_receipt_ -- --nocapture
cargo test -p ios-core --all-features private_identity_real_tcp_tls_ -- --nocapture
cargo test -p ios-core --all-features remote_discovery_keys_ -- --nocapture
cargo test -p ios-core --all-features remote_pairing_offline_matching_ -- --nocapture
```

The service gates traverse current production directory/selection and guarded
IO, canonical missing-port/shim pre-IO rejection, scope/20-byte proxy preservation,
cumulative bytes, independent cleanup, original real timer retention, timeout
retry and close versus late TCP dial. Real TCP peers require strict EOF with no
extra request; no reset is accepted as EOF. The selection gate exercises all
four actual bootstrap wire paths with fresh fallback connections. The private
identity native fixture performs actual localhost TCP, QueryType, StartSession,
TLS, GetValue UniqueDeviceID and StopSession for both matching and mismatching
UIDs before receipt assertions. The separate injected-bool callback fixture is
helper evidence only. Stored-record alternate-IRK tests create and load actual
credential files before cryptographic matching and observed-index assertions.

The regression agent owns parser/schema adapters and actual latest-CLI H2/TCP
producer gates. Final full core/CLI tests and strict Clippy remain required.
Multicast availability, physical devices, real saved identities, platform route
reachability and untriggered hardware modes remain NOT_RUN when not exercised.
