# L02 persistent packet workers and unique backend port · 2026-10-24

The existing userspace constructor now prepares a packet link and gives smoltcp
one PacketPort containing its original ingress receiver and egress sender. The
port has no Clone or raw transport capability. The authenticated byte stream is
still split exactly once at the original construction point. The same original
reader and writer futures own those halves throughout the runtime.

PreparedPacketLink admits the same header, ingress and egress backings in their
original order. Listener setup, all four actual PreparedWorker payload admissions,
parent roles and TaskHandles initialization retain their original sequence.
There is no new task, second stream reader, transport extraction or untracked
worker. Public userspace APIs, the immediate path and legacy no-bank path remain.

The original read, discard and write function bodies move unchanged into
`userspace/packet_link.rs`. The original helpers remain available to existing cfg
fixtures. Source guards compare those three bodies and the five lifecycle
methods plus task initialization: all nine comparisons are exact. Error recording
before stop, biased read/write selection, pinned original buffers and metadata
refund ownership remain on the same pipeline.

The new meaningful duplex test uses an actual 8-byte transport, banked header,
channels and prepared worker futures. It observes a 17-byte partial IPv6 header
and blocked partial outbound write before transferring the unique port to a new
consumer future. It checks the complete original inbound frame, FIFO A/B outbound
data, no premature original stream Drop, the original two actual owned joins,
strict peer Ok(0), task baseline and final full byte-bank refund.

Controller-only Cargo gates all passed on the same frozen 732 sources:

- Core all-features/all-targets strict Clippy.
- New packet-link1, complete userspace67 and native shutdown7. Filters overlap;
  each also invoked ten integration targets with zero selected tests, which do
  not count as functional coverage.
- Userspace-only and Kernel-only library checks. Each emits 43 feature-specific
  dead-code warnings; these are compilation checks, not strict warning-free gates.
- CLI all-features/all-targets strict Clippy and complete CLI1901 PASS / 0 FAIL /
  13 original ignored. Native-retirement module110 PASS plus one original ignored.
- Current CLI build, fmt/diff, actual readonly manager1 with usbmux trap0 and host
  5 PASS / 0 FAIL / 22 NOT_RUN; real_device_executed=false.

Only three Core sources change, including two new files. CLI, Cargo manifests and
Cargo.lock remain unchanged from 4b11ed6. No SDK or CLI NativeSource constructor
was polled. This test/feature coverage uses scripted/localhost/duplex seams and
performs no real-device, native-interface, address or route operation.

Current CLI and archived verified-ios.exe SHA-256:
`d545bf5bcdb97779995eb70a85a263104eea2e0ee176c12b146b4d2c238109c2`

Commands, terminal logs, 732-source fingerprints, exact body guards, current and
postcommit raw-HEAD proof:
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-packet-link`
Round1 contains Core results/body guards; round2 contains feature combinations,
CLI/host and archive. Frozen source inventories match exactly.

This is the connected transport foundation for endpoint staging. The module is
currently part of the userspace feature. Kernel-private IP composition, FIFO
write-barrier ACK, backend quiescence/port handoff, same-handle attach-before-launch,
verified native exposure/undo and exact replacement/new-record behavior remain to
implement. Kernel-only compilation does not claim those staging behaviors.
128MiB remains an admitted known-object pool, not heap/RSS. Main ledger stays
23 delivered / 6 missing development / L02 pending full audit, effective30, with
Q04 and all descendants excluded. The original seven-main-item goal stays active.
