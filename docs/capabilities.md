# Capability and evidence inventory

Real-device acceptance and protocol verification cover representative devices across generations, including `iPhone16,2 (iOS 27.2)`, `iPhone11,8 (iOS 18.7)`, and `iPhone14,7 (iOS 15.8)`. Host-side simulated workflows (including WDA localhost forwarding and Windows pasteboard TLS startup cancellation) are indexed separately from physical-device observations.

[capabilities.json](capabilities.json) is the authoritative inventory. It lists
all 63 top-level CLI commands, 49 public service families, Rust/C/Python entry
points and the CDP, media, clipboard and restore workflows. Topic
documents explain behavior; the inventory records implementation and evidence
separately. Adding a command or service requires a corresponding record.

Validate the source inventory and optionally the actual public CLI:

```sh
python scripts/capabilities.py
python scripts/capabilities.py --binary target/release/ios
python scripts/capabilities.py --build-cli
python scripts/validate.py capabilities
```

CI and the full validator check missing/obsolete records, source/evidence
locations, outcome/level consistency and maturity requirements. No device is
contacted. Protocol paths list declared service selectors and client imports;
an unresolved route is explicit and must be confirmed for the chosen operation.
These are implementation pointers, not protocol observations.

Each observation has one of PASS, FAIL, INCONCLUSIVE or NOT_RUN. Measured
outcomes refer to an evidence record with host system, device/iOS, USB/Wi-Fi
transport, required state, exact commit, date, scope and accessible location.
NOT_RUN states the missing condition. Unknown device state is not inferred
from ProductVersion or from a service name.

The initial Linux build evidence proves compilation only. The aggregate host
suite is indexed independently and does not certify every command's simulated
workflow. Existing historical reports are not promoted to current real-device
PASS without accessible evidence and a checked compatibility scope. In
particular, the standalone fake-CDP startup probe does not test the Rust bridge;
Cryptex host tests do not prove live TSS acceptance; encoded RTP does not prove
a viewer; restore codecs do not prove a recovery loop.

Maturity is `experimental`, `candidate-stable` or `stable`. All entries start
experimental pending capability-specific review. Stable requires representative
real-device PASS and a defined compatibility scope. Improving host evidence
alone cannot establish device support. Device and platform conditions can remain
unknown independently, and missing services must be recorded as missing, not
silently filtered out. Update each feature's limitation when implementation or
evidence changes; do not delete pending workflow entries to make a check pass.

The opt-in observer additions stay in their existing CLI/service
records and keep experimental maturity. `apps install-record
--observe-zero-budget-recovery` uses an explicit existing RSD route and records
actual integer wire type, same-client zero-budget recovery and owned socket
cleanup. `hid observe-stop` separately records one-way report flush, local
contact ownership, strict media reply, before/after media status and original
native joins. A report flush or worker completion is not a device ACK.

`rsd observe` records the actual RSD bootstrap selection and current selected
identity/directory; userspace/kernel mode does not substitute for that receipt.
`discover paired-identity` keeps accepted private-MAC Lockdown identity separate
from saved-altIRK advertisement authentication. An offline credential match does
not prove an online pair-verify acknowledgement. Their command/peer gates
remain NOT_RUN until the combined frozen-source validation is recorded; old
build or Viewer PASS is not borrowed for them.
