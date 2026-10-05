# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Keep a direct CDP page's lease cancellation inside its resource owner. Policy
  expiry/reload stops page work and awaits independent cleanup before release;
  native cancellation/cleanup causes survive through existing safe diagnostics.
  See [CDP page lease migration](docs/validation/cdp-page-lease-2026-10-05.md).
- Close WebDriver's owned forwarding socket after its Automation context, with
  one 100ms cleanup total and retained context/socket failures. Starting stop
  disarms duplicate cleanup after success, failure or cancellation. See
  [WebDriver cleanup evidence](docs/validation/webdriver-cleanup-2026-10-05.md).
- Retain CDP page teardown failures after WebSocket shutdown. Cleanup uses an
  independent 2s budget; an existing primary error and the cleanup error both
  remain owned. See [CDP page cleanup migration](docs/validation/cdp-page-cleanup-2026-10-05.md).
- Share one checked setup deadline across CDP discovery/attachment and WebDriver
  runtime construction. Exact CDP targets attach when discovered; full lists
  retain a separate 500ms quiet period inside the total. See
  [WebInspector server setup migration](docs/validation/webinspector-server-setup-2026-10-05.md).
- Preserve a CDP page bridge's original timeout after partial input or blocked
  automatic listing invalidates its WebInspector connection. Healthy empty idle
  still resumes. See [CDP page lifecycle evidence](docs/validation/cdp-page-idle-lifecycle-2026-10-05.md).
- Own WebInspector js-shell setup, cancellable input and per-evaluation output
  deadlines with one Ctrl+C future and independent session cleanup. Linux pipe
  and terminal input no longer leave a blocking stdin task at shutdown.
  Zero rejects before input acquisition/discovery; native transport failures
  stop even with --continue-on-error. See
  [WebInspector shell migration](docs/validation/webinspector-shell-lifecycle-2026-10-05.md).
- Give WebInspector opened-tabs, eval and launch one total covering default
  selection, setup, commands and output, with one Ctrl+C future and independent
  bounded session cleanup. Zero rejects before discovery; cleanup failure can
  now make completed output exit unsuccessfully. See
  [WebInspector command migration](docs/validation/webinspector-command-lifecycle-2026-10-05.md).
- Preserve the original WebInspector CLI discovery timeout after partial input
  or blocked automatic listing. Retry idle discovery only while the client
  permits reuse, retaining original native sources and page selection. See
  [WebInspector discovery migration](docs/validation/webinspector-discovery-lifecycle-2026-10-05.md).
- Bound WebInspector application/page caches and flattened page snapshots by
  local counts and logical JSON bytes. Admit complete updates before replacing
  caches, preserve duplicate-event order, and release removed payloads/capacity.
  Resource rejection keeps the existing typed Io/ResourceLimit family and
  invalidates the receiving client. See
  [WebInspector cache migration](docs/validation/webinspector-cache-lifecycle-2026-10-05.md).
- Share one WebInspector correlated-command/attach deadline across sends and
  replies; invalidate abandoned committed sessions to reject late responses.
  Bound pending/skipped events by count and logical JSON bytes, check full-range
  identifier exhaustion, and inherit finite caps through nested wire operations.
  Preserve matched business errors, public variants and successful wire/output.
  See [WebInspector session migration](docs/validation/webinspector-session-lifecycle-2026-10-05.md).
- Guard WebInspector client frames, startup and automatic event-routing writes
  across cancellation. Add local is_usable and optional operation budgets,
  reject invalid cached reuse, and require empty usable idle for discovery
  quiescence. Preserve legacy/preserving startup mappings and native sources;
  record checked zero/overflow and effective-budget migrations. See
  [WebInspector client lifecycle migration](docs/validation/webinspector-client-lifecycle-2026-10-05.md).
- Bound Companion default discovery/service setup at30 seconds and share one
  finite total and pinned Ctrl+C owner through command work. Reject zero/overflow
  before selection, retain actual timer/signal sources and await independent5s
  forwarding cleanup while preserving primary errors. Add consuming handle
  close APIs that disarm Drop at future creation; legacy borrowed stop/Drop
  remain. See [Companion CLI migration](docs/validation/companion-cli-control-2026-10-05.md).
- Guard shared Companion request/listener/forwarding state after cancelled or
  failed I/O, reject late replies and release terminal listener reads. Add local
  is_usable, checked budgets and a five-second forwarding cleanup cap while
  retaining the existing best-effort Drop policy. See
  [Companion lifecycle migration](docs/validation/companion-lifecycle-2026-10-04.md).
- Bound notification default discovery/service setup and send stages at 30
  seconds; share finite command totals from before subscription writes and keep
  one Ctrl+C owner through setup/capture. Reject zero/overflow before selection,
  retain Observe idle timing and unbounded Stream, and complete Stream total
  expiry consistently. See [notification CLI control migration](docs/validation/notification-cli-control-2026-10-04.md).
- Attempt bounded notification shutdown after returned command failures on a
  usable connection. Report cleanup failure after success and retain both causes
  on dual failure, with the primary category preserved. See
  [notification CLI cleanup migration](docs/validation/notification-cli-cleanup-2026-10-04.md).

- Guard NotificationProxy partial frames across cancellation, add optional
  operation budgets, and include subscription writes in wait deadlines. Preserve
  empty idle reuse and close local state after ProxyDeath or bounded Shutdown.
  See [notification lifecycle migration](docs/validation/notification-lifecycle-2026-10-04.md).

- Guard MobileBackup2 operations across negotiation, host work and raw transfer
  cancellation. Add optional total budgets and local usable state; retain both
  primary and cleanup failures. Preserve expected peer closure after a complete
  terminal reply. See [backup lifecycle migration](docs/validation/backup-lifecycle-2026-10-04.md).

- Guard DeviceLink message and version-exchange ownership across cancellation.
  Add optional operation budgets, local usable state and a process call sharing
  send/reply timing. Bound disconnect writes to five seconds and reject reuse
  after successful close or interrupted framing. See
  [DeviceLink migration](docs/validation/device-link-lifecycle-2026-10-04.md).

- Guard Lockdown requests with optional shared send/reply budgets and local
  usable state. Reject reuse after interrupted requests while preserving
  complete business replies. Bound StopSession writes to five seconds, retain
  uncertain session ownership on failure, and close the client after a successful
  write. See [Lockdown lifecycle migration](docs/validation/lockdown-lifecycle-2026-10-04.md).

- Guard borrowed RSD bootstrap and wrapper reads across cancellation after a
  complete send or consumed header. Reject reuse of interrupted initialization,
  including empty response waits, while preserving normal handshake completion
  and idle framing. Keep existing wire and caller deadlines. See
  [RSD lifecycle boundaries](docs/validation/rsd-lifecycle-2026-10-04.md).

- Guard XPC message, call, stream and file ownership across cancellation.
  Add local usable state and shared call/single-read deadline APIs; clear
  pending replies before reuse of an interrupted connection. Preserve finite
  Pasteboard replies and explicit CoreDevice stream completion. See
  [XPC lifecycle migration](docs/validation/xpc-lifecycle-2026-10-04.md).

- Reject H2 connection reuse after transport failure or cancellation with
  reported wire bytes. Clear retained frame state, preserve empty idle waits
  and stream-local shutdown behavior, and add internal operation budgets for
  flow-control waits. See [H2 lifecycle boundaries](docs/validation/h2-lifecycle-2026-10-04.md).

- Guard AFC send/reply ownership and expose optional shared request deadlines
  and local usable state. Refuse reuse after interrupted wire operations;
  preserve complete device Status errors. Attempt file cleanup after read/write
  failures within a separate budget, retain primary and cleanup causes, and
  invalidate cancelled transfers with open handles. See
  [AFC migration](docs/validation/afc-lifecycle-2026-10-04.md).

- Bound FPS capture timestamp retention and setup/cleanup budgets. Own the
  session without a background event queue, preserve partial reads across
  sample ticks and emit due windows before ready events. Keep primary failures
  when stop also fails and report cleanup failure after successful sampling.
  Add an allocation-free frame timestamp iterator while preserving the existing
  parser/output. See [FPS migration](docs/validation/fps-lifecycle-2026-10-04.md).

- Add shared DTX request/read deadlines and local connection-state inspection.
  Interrupted operations with wire progress reject reuse and release decoding
  state; empty idle polling remains usable. Partial initial/idle frames fail
  explicitly in CoreProfile/accessibility. See [lifecycle migration](docs/lifecycle-contract.md).

- Classify CLI-owned invalid timeout inputs and empty accessibility traversal
  expiry, with a bounded typed CLI precondition mapping. Preserve partial
  traversal/confirmation policies and original error-chain scan budgets.
  See [guard acceptance](docs/validation/cli-owner-guards-2026-10-04.md).

- Add preserving Lockdown value-query and service-start APIs with safe typed
  response-code observations and bounded lookup. Backup encryption defaults and
  debugserver fallback use exact request/code evidence; legacy APIs retain
  their behavior. See [migration](docs/validation/lockdown-response-codes-2026-10-04.md).

- Preserve original CLI service, parser, file and elapsed timeout errors through
  typed source chains. Distinguish local input from received identifiers,
  retain opaque input strings without inventing native sources, and use the
  existing typed cancellation marker. Return HID signal-registration errors
  instead of treating them as cancellation; keep successful requests and output.

- Return native I/O failures from CLI status writes instead of panicking, while
  preserving an existing primary failure if its diagnostic cannot be written.
  Report sysmontap CPU read failures after partial output, reject non-finite
  CPU JSON values, and retain original directory-path NUL conversion sources.

- Preserve original mDNS, CDTunnel, RSD check-in, lockdown dial and userspace
  proxy conversion/deadline causes. Distinguish local MTU input from received
  protocol data and real elapsed deadlines, retain configured budgets with safe
  private context, and keep discovery, wire, timeout and cleanup policies.

- Retain guarded AFC/DTX/FileService/symbol conversion sources through existing
  IO variants with a received-data Protocol role and safe fixed context.
  Own AFC's original cast bytes on error and enable zerocopy's existing std
  feature for MSRV; preserve guards, wire and successful borrowed decoding.

- Add DProxy recorder/proxy callbacks that deliver owned decode failures before
  fatal returns or event-write errors. Use them in CLI debug diagnostics with
  metadata only, and return original IO errors from event/status stderr writes
  instead of panicking; preserve capture bytes and forwarding order.

- Add `dproxy::StreamDecoder::push_preserving_errors` to retain original parser
  errors, typed local decisions and events preceding a later fatal error.
  Expose owned/borrowed observations and explicit ignored DTX input with safe
  Debug; keep legacy `push`, event JSON, buffering and failure policies.

- Preserve native application/test PID, fixed app plist, IPA integer and GPX
  XML/time parser sources through existing IO/XPC variants with safe context.
  Classify local input failures without text matching; keep wire/validation and
  route timing behavior, and document returned-variant/source migration.

- Add `prefetch_tss_batch_preserving_errors` with owned observations of combined
  request construction, signing failures and missing tickets, including failures
  recovered by fallback. Preserve native error sources, typed phases and rider
  indices with safe Debug; keep legacy result fields, call order and nonce rules.

- Retain restore H2/XPC, IPSW ZIP/read/manifest and TSS HTTP/codec causes through
  existing RestoreError::Io. Preserve native IO categories, distinguish local
  package/input failures from received protocol data and HTTP timeouts, and
  document source/variant migration with unchanged restore requests and limits.

- Preserve MobileBackup2 Unback/Extract operation and disconnect timeout
  causes, including an earlier operation failure when disconnect also expires.
  Add bounded typed deadline evidence with operation, phase and shared budget;
  retain native path UTF-8 and transfer-count conversion causes. Document the
  Protocol-to-Io migration with existing requests and five-minute budget.

- Preserve native DDI cache/download/ZIP/extraction-task and TSS causes through
  existing ImageMounterError::Io, with typed input/received/HTTP/cancellation
  categories and safe context. Keep public families, proxy/wire/budget/guard
  and partial-file behavior; document failure-variant/source migration in the
  [error contract](docs/error-contract.md).

- Add bounded service-issued deadline getters for Cryptex, MobileActivation,
  InstallCoordination and Pasteboard, retaining the former phase, duration or
  whole-seconds payload with native Elapsed. Preserve owner/fallback rules and
  reject promised pasteboard sizes that could bypass a 32-bit usize limit.

- Add typed OS Trace filter compilation while retaining the legacy String
  compiler, and preserve native CrashReport/OS Trace/parser/archive/PID causes.
  Keep the crash watch's named parse retries and existing filter/wire budgets;
  document typed input and returned-variant migration.

- Preserve manual-pairing RSD/H2/XPC, inner crypto, authentication, JSON and
  deadline sources through existing PairingTransportError::Io with fixed safe
  context. Retain native kinds and opaque unknown categories, reject short
  header fields without a panic, and document failure-variant migration with
  unchanged pairing requests, sequence/nonces, limits, trust and success.

- Retain native private-seed, HKDF, cipher and device-info encoding causes in
  existing PairingError::Io with fixed safe context. Classify local length
  faults as invalid input and received authentication failures as protocol;
  keep opaque encryption unknown and enable the same-version HKDF std Error
  implementation. Document failure-variant migration with unchanged crypto
  parameters, successful pairing payloads and trust policy.

- Retain native connection/pairing task, PSK, credential, codec and deadline
  causes through existing error variants with fixed safe context. Distinguish
  local invalid input from malformed remote data, preserve opaque unknown
  categories and document returned-variant/source migration without changing
  trust, requests, routing, retries or timeout values.

- Preserve both failed deactivation attempts in a borrowed Rust observation,
  including lockdown connection failure, and classify by the final cause.
  Retain native activation HTTP/URL sources, classify rejected endpoint input
  as invalid input, and hash endpoint URLs in client Debug. Document returned
  variant and diagnostic migration with unchanged fallback and HTTPS policy.
  Keep capability inventory paths correct for flat service entries with
  same-name submodule directories.

- Preserve remote-symbols H2/XPC, modern activation-record and pasteboard
  snapshot-connection causes through existing error variants. Keep native
  classes, activation deadline/observed-cancellation sources and safe CLI
  envelopes; document returned-variant and diagnostic migration without
  changing requests, routing, fallback conditions or successful output.

- Retain original ZIP/archive-entry decoder and entry-read IO sources during
  streaming IPA installation through the existing IO error variant. Preserve
  local package input categories, classify native IO by kind, and document
  returned-variant/CRC migration with fixed context and safe diagnostics.

- Add fallible streaming-install progress observers, preserving native IO
  sources and stopping before later replies on failure. CLI startup/progress
  stderr writes and flushes are checked, with existing safe progress bytes and
  success JSON preserved; legacy callbacks and wire flow remain available.

- Add checked C runtime initialization and consuming cleanup statuses, clearing
  accepted caller pointers before release and preserving legacy void categories.
  Report cached native initialization errors, poisoned state and caught unwinds;
  document consumed ownership separately from completed device cleanup.

- Add checked C RSD/userspace port queries with explicit NULL/missing/success
  statuses and caller-thread categories, retaining legacy uint16_t behavior.
  Document cached metadata versus current reachability/support and lifecycle.

- Honor JSON mode for application mutation statuses with versioned tagged
  results and ordered pkill progress/completion records. Retain human bytes,
  stop subsequent calls on failure, preserve DTX/deadline/routing sources, and
  retain appservice fallback for named typed service misses.

- Apply the syslog 1 MiB input limit to terminated final chunks before copying;
  preserve exact-limit/next-message behavior and prior CLI NDJSON on rejection.

- Return syslog relay read/EOF failures through the safe CLI failure path with
  exit 1 and retained native sources, preserving prior records and successful
  count/capture-time stops. Keep NDJSON/human bytes, check record write/flush and
  stderr progress IO, and document partial-result migration with owned streams.

- Preserve typed H2/transport sources through RSD and RemoteXPC setup, bootstrap,
  header/body reads, message writes and cryptex transfer writes. Use the existing
  IO error variant, retain native kinds and fixed context, and document migration
  from TLS strings with scoped host faults and an external XpcClient consumer.

- Retain the final on-demand RSD probe error after the existing retry schedule,
  and classify missing entries in a successful directory as service missing.
  Preserve initial snapshots/cache, successful routing and service-specific
  screenshot fallback; retain native XPC reconnector dial sources. Returned
  failure variants have an explicit migration in the error contract.

- Add `ios_list_devices_checked` to reject unencodable received C device records
  with `IOS_ERR_CORE`/`protocol`, cleared outputs and no partial array. Preserve
  the original listing symbol's skip behavior and existing layout/free contract;
  migrate listing examples and validate both entries over owned binary plists.

- Return typed safe Python RuntimeError for ordinary runtime-initialization IO
  failure, replacing the expect/panic path while retaining released-GIL work,
  signatures and existing operation errors. Cache the original failed result
  and keep effects unknown.

- Add a borrowed initial RSD probe observation distinguishing skipped, available
  and failed discovery, with skip reasons and retained owner errors. Preserve
  legacy optional directories and retries; retain native userspace-proxy dial
  failures and keep later retry observations explicitly separate.

- Add versioned typed error headers to protected manager/CDP/WebDriver router
  failures while retaining existing HTTP statuses and JSON bodies. Keep unknown
  sources explicit and effect uncertainty unchanged; expose fixed headers only
  to already approved browser origins and verify actual Chromium fetch.

- Add a caller-thread C error-category getter with borrowed static codes.
  Preserve the original 14 symbols, numeric statuses and layouts; classify
  native sources before conversion and retain runtime IO failures. Verify the
  linked header against an owned malformed ListDevices reply.

- Add versioned error category/phase/effect attributes to Python native factory,
  invalid-mode and unavailable-proxy exceptions. Preserve built-in exception
  types, safe args and public signatures; verify installed bindings against
  owned bounded host replies without device operations.

- Add a Rust `OperationCancelled` marker for observed local cancellation and
  retain it through WebInspector launch's Ctrl+C context. Machine failures can
  classify cancellation while plain EINTR and failed signal registration keep
  their IO meaning; device cleanup acknowledgement remains separate.

- Add opt-in global `--error-format=json` for versioned final CLI failures on
  stderr, with typed class, safe phase/diagnostic, original exit and explicit
  effect uncertainty. Keep human defaults and successful result formats; add
  actual argument, malformed-reply and closed-help regression fixtures.

- Add `WebInspectorClient::start_preserving_errors` to retain startup IO and
  typed timeouts while the existing `start` keeps its legacy `Disabled` mapping.
  CLI/CDP/WebDriver startup uses the new method; success frames/output and
  failure statuses stay unchanged, with more accurate safe diagnostics.

- Add non-exhaustive Rust error categories from typed sources and additive RSD
  feature-evidence queries. Preserve existing error variants/source chains and
  permissive metadata checks; [binding/output mappings](docs/error-contract.md)
  remain under implementation.

- Add explicit shared plist-to-JSON policies for historical byte-array,
  hexadecimal and tagged base64 output, with borrowed and owned iterative
  traversal. Preserve per-command Date/Uid behavior and separate MDM redaction.

- Separate WebInspector wire/model/client/session ownership from CLI command,
  workflow, HTTP, CDP and WebDriver adapters while retaining public paths and
  command contracts. Add a Rust-only service injection boundary and owned
  synthetic tests through the actual protected HTTP/WebSocket bridge.

- Separate backup wire, filtering, host filesystem and session responsibilities
  behind the existing public facade; use owned blocking disk operations in
  async backup callers and add actual local CLI output/failure fixtures.

- Replace device text inclusion with explicit private module boundaries for
  discovery, strategy selection, lockdown, tunnel activation, RSD and service
  routing while preserving the public connection paths and wire behavior.

- Add maintained compatibility candidate profiles, external Rust consumer
  fixtures, actual CLI list fixtures and expanded C/Python signature checks.
- Add a Linux host regression evidence runner with bounded raw capture, exact
  synthetic argv checks, owned agent cleanup and artifact allowlists; keep
  representative device/Windows fixture cases explicitly NOT_RUN.
- Add bounded, zeroizing private-file reads and exclusive new-secret publication
  for access-control credentials; existing shared directory permissions are
  preserved.
- Add `ios tunnel auth create|check` for private random credentials, explicit
  device/port/permission scopes and expiry checks without a device. This prepares
  policies without device access.
- Require a scoped private policy or explicit loopback trust for the tunnel
  manager. Authenticate HTTP/WebSocket requests, filter device lists, authorize
  device/port operations, and revoke streams at expiry or successful policy
  reload. Private-file CLI credentials disable proxies/redirects and bound
  response bodies. Platform/device acceptance remains separate.
- Restrict raw forward/WDA/dproxy/screenshot-stream listeners to loopback before
  device access; validate WDA proxy ports before startup. Raw userspace proxy
  and kernel routes retain their documented host trust boundary.
- Let the host doctor read private manager credentials with `--token-file`;
  explicit credential read failures and authentication/scope refusals now fail
  its manager check without including remote error bodies or secrets.
- Require private scoped credentials or explicit loopback trust for CDP/Selenium.
  Enforce exact-device debug access before parsing/device I/O, support scoped
  policy reload, revoke HTTP/WS and stored automation leases on expiry/reload,
  and abort owned CDP page tasks when their parent connection ends. Physical-device
  cleanup acceptance remains pending.
- Enforce actual-listener Host and exact allowed Origin on manager/CDP/Selenium,
  reject proxy authority spoofing and require explicit debug device selection
  before global discovery. Move shutdown/start-tunnel to POST; legacy GET/HEAD
  give a read-only notice until 2026-11-01 UTC and 410 thereafter.
- Add scoped browser bootstrap with approved-origin CORS and bounded, 30-second,
  single-use WebSocket tickets for the manager/CDP. Bind handshakes to Origin and
  complete targets; reject replay/ambiguity and revoke pending tickets/streams
  on policy replacement. Native bearer clients keep their existing contract.
- Add private per-upstream credential/CA configuration before manager startup,
  verified HTTPS/WSS federation and literal-loopback plaintext limits. Disable
  HTTP proxies/redirects, isolate every next-hop credential and report failed
  aggregate/target requests explicitly instead of a successful empty listing.
- Added a maintained capability/evidence inventory for CLI, Rust service families
  and binding/workflow boundaries, with CI checks for drift and unsupported
  maturity claims.

- Added a cross-platform validation entry point for minimal feature builds,
  split Rust/Python tests, doctests, downstream examples and linked C ABI smoke.
- Added candidate CLI/C/Python artifact validation and complete release gates,
  including MSRV, feature checks and minimum/newer Python wheel imports.
- Expose the CLI version with `ios --version` for installed artifact checks.

- Added Darwin-notification pasteboard monitoring with fresh PULL connections,
  baseline suppression and `(nonce, changeCount)` deduplication. `pasteboard watch`
  now works without experimental opt-in; `--autonotify --experimental` retains the
  low-level subscription path.
- Added device-issued stream-token stopping and a typed `DisplayError::MediaInUse`
  for CoreDevice camera/microphone conflicts (9022).

### Fixed

- Preserve native Pcap/FPS stream errors and XCTest/notification/CrashReport
  timeout sources. Propagate failed OS Trace/crash/power signal waits; expose
  typed notification closure and issued stream deadlines. Keep existing stops,
  result data and partial effects. JSON WebInspector shell omits interactive
  prompt/EOF presentation bytes; see the [stream migration](docs/validation/cli-stream-contract-2026-10-04.md).

- Preserve typed timeout sources through WebInspector CLI connection, launch,
  application-list and expired-budget contexts. Keep timer values, human
  context text and existing failure exits; legacy Core startup remapping and
  machine-output adapters remain tracked in [A02](docs/error-contract.md).

- Propagate CLI stdout write/newline/flush failures through a shared fallible
  boundary. Closed result pipes return exit 1 instead of a possible panic/101;
  failed help/version output returns 1 instead of silent success. Successful
  output formats remain unchanged; see [migration](docs/plist-json-contract.md).

- Preserve exact filtered-file destinations after device move/copy so placeholder
  cleanup does not append a file-invalid trailing separator. Backup output
  failures now return exit 1 instead of a possible println panic/exit 101;
  [migration and partial-result rules](docs/backup-contract.md) retain success formats.

- Read canonical usbmux `ProductID` for listing and attach events, while
  retaining the previously accepted `ProductId` alias and zero for missing
  metadata. CLI JSON fields and types remain unchanged.
- Restrict every CLI verbosity to reviewed project log targets, preventing
  dependency handshake/frame/close logging from exposing identifiers, headers
  or device data. Verify with a real loopback WebSocket exchange and raw control.
- Discard unreviewed HTTP error bodies from manager/CDP/Selenium framework
  rejections while preserving status, Allow and HEAD behavior. Hide credentials
  in supervised-pairing/session Debug and raw payloads in owned error Debug.
  Clipboard promise errors and remaining automatic progress messages use
  diagnostic correlation; explicitly requested data keeps its API purpose.
- Prevent arbitrary error/source text from reaching automatic CLI, manager,
  WebDriver and Python diagnostics; retain phases, categories and correlation.
  Hide credential environment values in help and addresses in Python repr.
  CDP device protocol failures retain routing/error codes while omitting raw
  error data. Explicit debugger results and device selectors retain their use.
- Add bounded raw stdout/stderr checks that independently report original
  secret detection and masked-copy results; timeout and overflow fail closed.

- Redact discovery/connection Debug output and device-not-found/pair-record
  error diagnostics, including credential paths and malformed record data.
  Typed variants, IO categories and process-local device correlation support
  diagnosis; explicit selection fields retain their purpose.
- Preserve existing file-pull and backup output until staged transfers succeed;
  clean owned temporary files on cancellation. Refuse symlink image/file
  destinations with `--force` on every platform and prevent no-force races from
  overwriting concurrently created files. Filesystems without hard links fail
  explicitly for no-force publication.
- Support full-length output filename components without staging suffixes
  exceeding filesystem limits. OS trace collect exclusively publishes a new
  directory and reports temporary cleanup failure.
- Omit credential fields from pairing-record Debug output and use process-local
  identifier correlation in manager/forward/tunnel diagnostics. Instruments
  diagnostics no longer render device response payloads.

- Make independent `fileservice` and `deviceinfo` features enable their required
  RemoteXPC transport; gate service integration tests by their required features.
- Correct default doctests, private transport import paths and binding examples'
  empty-list/error/cleanup behavior.

- Open a fresh RemoteXPC connection for each subsequent Display request and use
  the `stopAll`/`identifiers` stop payload. Direct and userspace routes are retained.
- Omit `VRAE:0` from video negotiation so the device can adapt its encoder.
- Default CLI pasteboard reads to `promisesecondary` to avoid eagerly resolving
  rich secondary representations; explicit `--policy resolved` remains available.
- Disable accessibility app monitoring and inspector visuals when focus commands
  finish, fail or receive Ctrl-C; stop display capture on Ctrl-C as well.
- Include `LiquidGlass` in the default supervised setup skip list.

## [0.1.13] — 2026-08-30

### Fixed

- Made backup integration expectations compare canonical layout paths, so
  macOS's `/var` alias does not turn a successful backup into a false failure.

## [0.1.12] — 2026-08-30

### Fixed

- Validated the directory target after accepting macOS's exact root-owned
  temporary-directory aliases, so OS-trace output works through `/var` and
  `/tmp` without weakening user-symlink rejection.

## [0.1.11] — 2026-08-30

### Fixed

- Accepted macOS's root-owned `/var`, `/tmp`, and `/etc` aliases into
  `/private` while continuing to reject user-controlled symlink components.
- Made backup and OS-trace path fixtures use the same canonical spelling as
  production so macOS system aliases and Windows short/long path forms do not
  cause false failures or premature mock-stream EOFs.

## [0.1.10] — 2026-08-30

### Added

- Added legacy MobileBackup2 backup helpers, crash-report workflows, and
  read-only InstallCoordination install-record queries.
- Added CoreDevice app, file, diagnostics, media, pasteboard, companion, HID,
  configuration, orientation, XCTest, WebInspector, and trace workflows.
- Added matching Rust, C FFI, Python, CLI, protocol, and feature documentation
  for the expanded service surface.

### Changed

- Extended tunnel and RemoteXPC service handling for CoreDevice paths while
  keeping service-dependent behavior explicit when a device omits an endpoint.
- Added bounded deadlines, transfers, streams, and device-input parsing across
  the new service clients, with atomic host-file output where applicable.
- Documented Git Bash/MSYS device-path conversion and the RSD default filter,
  full-directory, prefix, and feature-metadata contracts.

### Fixed

- Hardened AFC framing/status validation, file replacement and Windows long-path
  handling, XPC/HTTP2 response routing, cancellation, and cleanup paths.
- Corrected activation diagnostics to redact daemon-supplied payloads and fixed
  backup manifest persistence and InstallCoordination request/response handling.

## [0.1.9] — 2026-08-29

### Added

- Exposed RSD-advertised service feature identifiers through `ios-core`, the
  `ios rsd services --features` command, the C FFI, and Python tunnel objects.
- Added detailed MobileBackup2 free-space diagnostics after device purge
  requests.
- Added `ServiceDescriptor::features`, `ServiceDescriptor::new`, and RSD
  capability helpers for Rust consumers.
- Added `ios_tunnel_rsd_services_json` to the C ABI and `service_ports` plus
  `service_features` to the Python `Tunnel` object.

### Changed

- Reduced MobileBackup2 host-to-device transfer frames to 32 KiB and streamed
  device-to-host payloads through bounded buffers.
- Decoded BSD `vis(3)` escapes in syslog relay messages and made process filters
  recognize Apple's `Process(Library)` annotation.
- Preserved the existing JSON schema for `ios rsd services` and `rsd check` by
  keeping feature metadata opt-in. Passing `--features` adds `features` to
  JSON and human-readable output; without it, services JSON remains
  `name`/`port` and check JSON remains unchanged.
- TCP dials and initial tunnel/RSD/XPC setup paths now fail after 15 seconds
  instead of inheriting the host operating system's long retry window.

### Fixed

- Bounded tunnel TCP connection attempts, InstallationProxy Browse responses,
  and pending DTX state so stale or malformed device traffic cannot stall or
  grow memory indefinitely.
- Rejected invalid RSD ports and malformed DTX length/fragment metadata instead
  of truncating values or accepting incomplete frames.
- Accepted additional NSKeyedArchiver UID indirections and nullable XCTest
  fields, and encoded object-valued XCTest configuration fields as keyed
  references.
- InstallationProxy Browse now enforces separate response-chunk and app-entry
  limits before retaining more device data.
- MobileBackup2 file transfers use bounded buffers and 32 KiB host-to-device
  frames; insufficient-space failures include the available-space diagnostics
  collected around purge requests.

## [0.1.8] — 2026-07-27

### Added

- Added Bluetooth HCI capture through `ios btlogger`, CoreDevice feature groups, and remote symbol download support.

### Changed

- Refactored tunnel, RemoteXPC, and device connection orchestration for direct and userspace-proxy transports.
- Added explicit overwrite and destructive-operation guards across the CLI.
- Bumped workspace crates and internal `ios-core` dependency versions to `0.1.8`.

### Security

- Restricted persisted private-key material to the owner on Unix and zeroized pairing secrets where supported.
- Bounded device-controlled allocations and parser recursion across XPC, DTX, OPACK, NSKeyedArchiver, AFC, fileservice, SpringBoard, and syslog paths.

### Fixed

- Fixed XPC HTTP/2 receive-window replenishment and cancellation-safe tunnel packet reads.
- Corrected personalized DDI selection and download validation.
- Fixed protocol decoding, device discovery, file/path handling, and CLI/service correctness issues.

## [0.1.4] — 2026-04-27

### Changed

- Bumped workspace version to `0.1.4`.
- Code review P0/P1/P2 fixes: allocation guards, FFI safety, async FS, module visibility, dead features, test coverage, shared test utilities, service error macro, house_arrest module structure.

## [0.1.2] — 2026-04-27

### Added

- PyPI package now includes a user-facing README with install instructions, API reference, and usage examples.
- CI validates that the git tag version matches the workspace `Cargo.toml` version before publishing.

### Changed

- Bumped workspace version to `0.1.2`.

### Fixed

- Fixed empty PyPI documentation by adding the `readme` field to `pyproject.toml`.
- Corrected stale crate count and name references in CHANGELOG v0.1.0 infrastructure section.

## [0.1.1] — 2026-04-24

### Added

- Added crate-level README files for every workspace crate so crates.io package pages have documentation.
- Added a reusable crates.io publish script with retry/backoff handling for crates.io rate limits.
- Public README, contribution guide, security policy, and user/developer documentation.
- GitHub issue templates and pull request template.

### Changed

- Bumped workspace crates and internal crate dependency versions to `0.1.1` for a documentation refresh release.
- Updated the tag release workflow to use the shared publish script instead of fixed 30-second sleeps.
- Removed local machine-specific Cargo/PyO3 configuration from the repository.
- Normalized example/test placeholder names and temporary paths for public release.

## [0.1.0] — 2026-04-21

### Added

#### Device Management
- USB and network device discovery via usbmuxd
- Lockdown protocol with TLS session, pair record, and supervised P12 pairing
- WiFi pairing CLI for network-only setups
- iOS 17+ CDTunnel handshake with kernel/userspace TUN forwarding
- XPC/RemoteXPC service discovery (RSD) over HTTP/2
- mDNS/Bonjour device discovery CLI

#### App Management
- App install/uninstall/launch/kill via InstallationProxy
- Streaming Zip Conduit fast install (Xcode-style)
- iOS 17+ CoreDevice appservice support
- Process signal sending (arbitrary signals) and pkill by name

#### File System
- Apple File Conduit (AFC) — ls, pull, push, mkdir, rm
- iOS 17+ XPC file service

#### Instruments & Performance
- CPU/GPU/FPS/network/energy monitoring via sysmontap
- Per-process monitoring with CPU threshold alerts
- Core Profile Session (FPS frame timing)
- KDebug trace event CLI
- HAR (HTTP Archive) logging

#### Screen & UI
- Screenshot capture (single and MJPEG stream)
- SpringBoard icon layout get/set, wallpaper export, orientation
- Accessibility audit and interactive element navigation

#### Diagnostics
- Real-time syslog streaming
- Crash report download and management
- OS trace relay process listing
- Network packet capture (pcapd)
- Developer disk image auto-download and mount

#### Device Configuration
- Configuration profile install/remove (MCInstall)
- Provisioning profile management (misagent)
- Location simulation (coordinate set/reset/GPX playback)
- Device state induction (enable/disable thermal, network conditions)
- Notification subscribe/post
- Backup create/restore

#### Security & Debug
- AMFI developer mode management
- LLDB debugserver connection
- XCTest execution framework (testmanager)
- WebInspector protocol for Safari/WebView debugging

#### Infrastructure
- Workspace with 4 crates, unified dependency management
- Feature-gated service modules (30+ features in ios-core)
- Python bindings (PyO3) — `ios-py`
- C FFI bindings — `ios-ffi`
- Cross-platform CLI binary (`ios`)
- Protocol documentation for AFC, DTX, lockdown, OPACK, XPC

[Unreleased]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.13...HEAD
[0.1.13]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.7...v0.1.8
[0.1.4]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.2...v0.1.4
[0.1.2]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/oslo254804746/rust-ios-device/releases/tag/v0.1.0
