# Capability and evidence inventory

[capabilities.json](capabilities.json) is the authoritative inventory. It lists
all 62 top-level CLI commands, 49 public service families, Rust/C/Python entry
points and the incomplete CDP, media, clipboard and restore workflows. Topic
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
