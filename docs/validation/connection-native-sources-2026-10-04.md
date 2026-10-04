# Discovery and connection native sources: host validation

Accepted on 2026-10-04; checks started on 2026-10-03. A02/class 3 source
`4b3a9cbe40b696b01cbfdb65ba8a7d614795e9fc`, branch
`feat/device-service-updates-20260927`, baseline
`a2c726785a0fcb3601468619dfb74369366a0139`. Linux x86_64,
Rust 1.97.1/MSRV 1.80.1, workspace Python 3.12.13.

Fifteen original objects previously discarded into strings or .ok now survive
existing CoreError/TunnelError::Io or IO, with private fixed context. Native
IO roles are assigned from the owning boundary; the wrapper is not OS evidence.

| Boundary | Original object / role |
| --- | --- |
| mDNS create/browse, two callers each | Four mdns_sd::Error objects; opaque Unknown preserved |
| Local MTU upper bound | TryFromIntError; InvalidInput, full u32 retained |
| Received CDTunnel port / MTU integer | Two TryFromIntError; Protocol, full received u64 |
| Received supported MTU validation | Original returned TunnelError, including its native integer chain; Protocol |
| Fixed CDTunnel request JSON / length | serde_json::Error / TryFromIntError; local InvalidInput |
| CDTunnel timeout | Original Elapsed / configured Duration; Timeout, previously textual Protocol |
| Received CDTunnel JSON | serde_json::Error; Protocol |
| RSD check-in / lockdown TCP dial | Two original Elapsed / configured Duration; existing Timeout |
| Local userspace proxy port | TryFromIntError; existing InvalidInput |

The minimum-MTU decision remains at the same guard but becomes typed
InvalidInput with no fabricated native source. A received wrapper has Protocol
precedence over the nested local-input validation, distinguishing caller input
from received data. Zero received port remains its original Protocol decision.
Opaque mDNS Msg/ParseIpAddr/Again objects retain Unknown through existing
retained context. mdns-sd itself has already flattened deeper OS failures into
its enum strings; these cannot be recovered or relabelled as known OS errors.

The private deadline owner retains exact configured Duration and native Elapsed
with fixed phase; Debug includes phase/budget only. Lockdown timeout text no
longer includes raw host/port. Native payloads/addresses remain hidden by
existing diagnostic Debug, while explicit Display/source access follows the
[error privacy contract](../error-contract.md).
Returned variant, Display/immediate source and diagnostic correlation can
change; handshake timeout and local MTU categories migrate to Timeout/Input.
Root public enums/fields/signatures and machine binding schemas remain.

Seven source files, 393 insertions/106 deletions. Entire production bodies
match baseline after normalizing only original source callbacks, one minimum
MTU failure representation and additive private helpers. The reviewed port
expansion replaces .ok().filter(nonzero) with the same u16 conversion and
zero comparison, retaining the conversion error rather than discarding it.
Requests, magic/framing, numeric ranges, allocations/limits, packet/daemon/
task/stream logic, configured/shared timeout budget and cleanup policy stay.
Public root/three TunnelError variants/consumer/features/dependencies/classifier
and AFC/DTX/H2/XPC codecs stay unchanged. Handshake helpers remain behind the
private tunnel module; no new public export. No new deadline, retry, invalidation,
daemon shutdown acknowledgement, cancellation, rollback or safe-reuse rule.

Five tests were added internally. Native mDNS enum objects are constructed
private-adapter fixtures, not actual daemon/OS failures. Received malformed
JSON, oversized port/u32 MTU, supported-MTU overflow and zero-MTU responses run
through the actual codec with joined owned duplex peers, exact request and
local EOF after stream destruction, bounded to one second. Chains retain
serde JSON, actual integer overflow and the original nested MTU error while
received classification wins. Local MTU zero/below/overflow preserves range
and distinguishes genuine native sources from source-free decisions.

Actual zero-budget CDTunnel and existing 20ms RSD timeout run against owned
pending peers and retain Elapsed. CDTunnel's request bytes/local EOF are checked.
The private proxy preamble runs over owned loopback TCP, with actual u32 port
overflow, InvalidInput/native source and no reply before local EOF, one-second
join bound. Local destruction does not acknowledge remote cleanup. Lockdown
TCP dial expiry is source-reviewed at the same 15 seconds, without a black-hole
network/device experiment. Numeric fixed JSON into Vec and conversion after
<=255 guard cannot return those native failures under current production guards;
no fake serializer/body-length coverage is claimed.

The first existing handshake scope returned **9 PASS/6 FAIL**, due to old
Protocol-variant assertions. Six tests were migrated to typed class/native
sources; original range/success-frame checks remain. An attempted external
private handshake-path import failed with two E0603 errors and was removed:
the existing public consumer is byte-identical, no export was added. These
are compatibility assertion/compile findings, not behavioral negative controls
or production failures. All final narrow checks and strict Clippy pass.

Logs share `/tmp/ios-a02-connection-native-`: original snapshots, base.txt,
handshake-first.log, narrow.log, narrow-final.log, code-audit-final.log,
source-diff.patch, full.log and full-counts.log.

| Check | Actual result / log suffix |
| --- | --- |
| New all/minimum mdns,tunnel-userspace | 5/5 PASS / new-final.log, minimal.log |
| Handshake/discovery/routing/lockdown/userspace | 15/9/11/4/16 PASS / respective final logs |
| Existing public all/minimum | 37/17 PASS / public-final.log, minimal-public.log |
| Strict core Clippy | terminal exit 0 / clippy.log |
| First canonical full | Final [PASS] validation/full marker / full.log |

Full: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. 2078 Rust/13 Python PASS,
zero FAIL, two existing ignored, fourteen Rust groups and one Python group.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups,
7+7 doctests/three snippets, C ABI/malformed/checked list, 3.12 wheel/sdist
rebuild/install, 53 tooling/118 help/38 raw/five host/22 device NOT_RUN/15 CLI/
three backup contracts pass. No separate Chromium or installed 3.9/3.13 rerun.
After continuation the original process handle was missing; the final PASS
marker and no active workspace cargo/rustc/validator/maturin were verified.
Raw shell exit was not captured across that boundary; the script prints the
marker only after every scope passes and then returns normally. No restart or
duplicate full was used to replace the existing evidence.

All source files staged before full and frozen through documentation.
Source diff SHA256:
`d7fa5e8225c0eeccaeae4fe531eb535aec3d70db82d3c6b80210ce049f0f30aa`.
Tested ios 0.1.13 debug SHA256:
`2efdd42e0b320ce98f0356c3f44ba3a8761ecee0ad406e567ddec76639b078c3`.
Disk about 33 GiB before full and 20 GiB after; no cleanup/storage failure.
Eight documents updated separately; 118 capabilities/implementation/maturity
and historical evidence stay, with one new host entry and updated catalog date.
Seven Markdown files/179 local links, whitespace and source/binary hashes
pass doc-audit.log.

A02/class 3/overall goal stay active; class 4 has not started, all 31 tasks and
subtasks remain. Next: remaining filesystem native adapters and checked CLI
diagnostics, plus finite review of typed decisions/stream/JSON/binding scope.
L01/L03/A03/platform/device conditions stay. No physical device/daemon/
credentials/Apple action, reference edit, push/publication, history rewrite or
maturity upgrade. See the [public contract](../public-contract.md) and
[preceding guarded gate](guarded-native-sources-2026-10-03.md).
