# SDK service repair — 2026-10-10

Scope: review R06/R13/R14 and the apps/AFC/log portion of original A03.
The original accepted existing-trust query/session owner is retained. This
record describes the repair source; final integration command results and
tested source identity are recorded by the main repair report. A code change,
syntax check or stale build does not constitute native ABI acceptance.

## Production changes

- Core `ExistingTrustDevice::prepare_service` reserves the same immediate
  `SessionOwner` lease as queries, opens fresh operation-owned mux/control/
  service sockets, and fences discovery, StartSession/TLS, StartService,
  service Connect, sends, reads and output with one native absolute total and
  caller/owner cancellation. No retry, pairing, installation or AFC mutation.
- Apps use the existing checked Browse limits. AFC bodies are admitted before
  allocation, complete-file reads use the caller ceiling/EOF probe, and
  cumulative actual service bytes and retained text/data/entries are bounded.
  Control-plane frames/replies have their own finite budget. These logical
  bounds exclude parser temporaries/TLS wire/CPU/RSS and caller-retained data.
- Completed syslog messages live outside the outer cancellation fence.
  Timeout/EOF/IO/cancel/output refusal is an error owning that prefix and the
  original typed cause. Python/C inspect the real IO payload through `get_ref`
  before walking its source; `io::Error::source` alone can skip the wrapper.
- Python service jobs share the actual query registry/admission, supervised
  original native JoinHandle, cancellation token and close receipts. Future
  cancellation is native cancellation followed by original join, not an
  executor-thread cancellation. `Device.aclose()` invokes the actual close
  implementation. Tunnel service forwarding caches one checked owner.
- C configured service APIs and the existing apps/AFC/log functions use the
  same checked Core implementation. `ios_device_handle_from_session` supplies
  a services-only alias for explicit routes while sharing the configured
  query owner. `ios_device_services_close` seals/cancels and waits for actual
  leases without consuming the allocation. The C caller still joins its OS
  threads before freeing borrowed handles or output storage. No detached
  service task or remote cleanup acknowledgement is introduced.
- Empty AFC files use NULL/0 in C. Zero-count logs perform no socket IO but
  still require a live owner, admission and an uncancelled token. The prior
  closed-handle zero-count false success is removed; a valid-owner listener
  trap regression preserves legitimate empty results.
- Local caller-provided malformed PairRecord bytes retain their original
  parse cause as InvalidInput, matching the accepted A02 mapping to Python
  ValueError. The installed finite-query validator now distinguishes local
  input from malformed on-wire replies; it preserves the no-IO assertion and
  all original cleanup/admission/receipt checks.

## Executable acceptance

The new gate requires actual built/native artifacts and never builds Rust or
supplies substitute SDK outputs:

```powershell
<installed-wheel-python> -I scripts/python-device-query.py
<installed-wheel-python> -I scripts/sdk-device-services.py
<python> -I scripts/sdk-device-services.py --ffi-library <new-ios_ffi.dll>
```

The shared owned peer speaks production mux, checked Lockdown, service
Connect/TLS, InstallationProxy Browse, AFC directory/open/read/close and raw
syslog framing. Assertions retain exact data/hash, whole-operation timing,
native error categories, shared query/service conflict, partial messages,
strict original socket EOF and original peer/caller-thread joins. The C gate
runs configured and original service exports; Python additionally runs native
async cancellation, admission/observer recovery and the Windows public
Device.aclose case with an isolated temporary trust store.

Static checks actually executed while preparing this source:

- Python AST parsing: new peer, native service validator and original query
  validator passed. This proves syntax only.
- Clang `-fsyntax-only -std=c11 -Wall -Wextra -Werror -I crates/ios-ffi/include`:
  original `device_info.c`, `device_services.c`, `smoke.c`, and new
  `finite_device_services.c` passed against the repaired header. This proves
  C syntax/types only, not linked ABI behavior.

The main agent builds frozen source snapshots and retains exact native logs.
Any native gate failure remains a failure until the source is repaired and
that same gate actually passes; no assertion is weakened to produce PASS.

## Limits and outstanding validation

Physical application/AFC/log/USB/Wi-Fi workflows are NOT_RUN. Other release
platforms and minimum Python 3.9 are NOT_RUN. The existing POSIX pthread
finite-query linked fixture is still separately NOT_RUN on Windows; the new
portable C service gate does not pretend to have executed it. Linux/glibc
allocator, RSS/FD and true global exhaustion probes remain independent.

Legacy `ios_device_open` retains its existing connected-device compatibility
path under a new 15s connection total. Its generic full Lockdown query API is
not represented as the checked finite query profile. Local pairing-file
loading for convenience handles occurs before service admission and cannot
physically preempt a stalled regular filesystem. Configured sessions take
already-loaded bounded credentials and numeric/explicit endpoints, avoiding
file-loader/DNS workers in the checked operation. C serializations are
cooperative; partial output has an independent byte-limited 100ms projection
budget and remains a nonzero result even if that projection fails.

This package can close the identified SDK implementation gaps only after its
actual native gates pass. It does not close every original L01 service profile
or certify physical services by itself.
