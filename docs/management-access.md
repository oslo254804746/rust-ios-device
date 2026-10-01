# Management access configuration

The tunnel manager requires a private scoped policy or explicit local trust.
`ios tunnel auth create|check` prepares/checks credentials without starting a
server or contacting a device. Start the manager with `--auth-config`; clients
use `--token-file`. Raw forwarding listeners have been audited and restricted
to loopback. Debug-server admission is still pending; browser access and
protected federation are S02/S03.
Track acceptance in the [roadmap](engineering-roadmap-2026-09-30.md).

For Unix shells, choose two new paths in a trusted private directory:

```sh
ios tunnel auth create \
  --config ~/.config/rust-ios-device/manager/policy.json \
  --token-file ~/.config/rust-ios-device/manager/client-token \
  --permission list --permission connect \
  --device DEVICE_UDID --port 58783 --expires-in 86400

ios tunnel auth check \
  --config ~/.config/rust-ios-device/manager/policy.json \
  --token-file ~/.config/rust-ios-device/manager/client-token \
  --permission connect --device DEVICE_UDID --port 58783

ios tunnel serve --userspace \
  --auth-config ~/.config/rust-ios-device/manager/policy.json

ios tunnel list --token-file ~/.config/rust-ios-device/manager/client-token
ios doctor --token-file ~/.config/rust-ios-device/manager/client-token
```

Replace `DEVICE_UDID` with the selector whose scope you intend to grant. This
port example grants one port only; it does not establish that port's service or
device availability. Check validates the credential and precise policy inputs
without a device connection. An invalid/expired token or a denied scope exits 1;
only an allowed check emits `credential_check` with `data: true`. Successful
creation emits `credential_create`, expiry and `data: true`, never the token.

Tokens contain 32 random bytes encoded as 64 lowercase hexadecimal characters.
The server policy stores SHA-256 of those exact ASCII characters, not the
decoded 32 bytes, plus explicit scopes. Digest comparison uses `subtle`; token
and read buffers use Zeroizing wrappers and Debug omits credential material.
No token option accepts a plaintext command-line or environment value.

| Permission CLI value | JSON value | Scope required |
| --- | --- | --- |
| list | list | Exact device selector |
| connect | connect | Exact device selector and nonzero port |
| manage-tunnels | manage_tunnels | Exact device selector |
| manage-server | manage_server | Server permission only; no implied device/port privileges |
| debug | debug | Exact device selector; not an arbitrary TCP grant |

Grant scopes deny by default. `--all-devices` and `--all-ports` explicitly grant
their dimensions and conflict with corresponding lists. Broad dimensions do
not add permissions. Port zero is always denied. Multiple permissions use
repeated `--permission`; multiple devices/ports use repeated options. Lifetime
is one second through 365 days, default 24 hours. Expiry is checked against the
host Unix clock, with equality expired; keep that clock correct.

Policy JSON version 1 has a `grants` array. Each grant requires `token_sha256`,
`expires_unix` and `permissions`; `devices`, `ports`, `all_devices` and `all_ports`
default to empty/false. Empty grants deny all. Unknown fields/versions, duplicate
credential hashes/permissions, ambiguous broad/list scopes, missing required
device/port scopes, zero ports and overlong future expiry fail. Limits: 64 KiB
file, 64 grants, 128 device selectors per grant (each at most 256 bytes), and
256 ports per grant. The server reads/validates its configured file before
opening a listener or scanning devices. Changes on disk take effect only on a
successful authorized reload.

## Manager admission and reload

Every route, including health/readiness, unknown routes, extractor rejections
and the WebSocket handshake, requires exactly one `Authorization: Bearer TOKEN`
header in policy mode. Missing, malformed, duplicate, invalid and expired
credentials receive the same HTTP 401 with `WWW-Authenticate: Bearer` and a
fixed message. Valid credentials with an insufficient scope receive HTTP 403.
Replies are `no-store`; credential text and selectors are omitted from rejection
diagnostics. Only private token files are accepted by CLI clients. Their HTTP
requests disable environment proxies and redirects, and bound successful bodies
to 1 MiB. Failure bodies are discarded without reading them into diagnostics.
The host doctor uses the same private token reader and scoped list endpoint.
Omitting credentials leaves the optional protected manager as a warning;
explicit unreadable/rejected credentials or denied list permission are a failed
check. Other host probes retain their separate results.

| Manager operation | Required permission and scope |
| --- | --- |
| GET `/`, `/tunnels` | list; records are filtered to allowed device selectors |
| GET `/tunnel/:udid` | list for that exact device |
| GET `/connect` WebSocket upgrade | connect for the exact device and nonzero port |
| Start/delete a device tunnel | manage-tunnels for the exact device |
| List/add/delete upstreams; shutdown; POST `/auth/reload` | manage-server |
| Health/readiness | Any valid, unexpired grant |

The monitor auto-connects only devices covered by a current connect or
manage-tunnels grant. List, debug and server administration alone do not trigger
device discovery. Existing tunnels outside active connect/manage-tunnels scopes
are removed at the next monitor refresh. Roles do not imply each other.

For administrative credentials, include an explicit `--permission manage-server`
when creating the policy. `POST /auth/reload` reloads only the startup config
path; the request cannot choose a file. A bad/missing/insecure file leaves the
previous policy and sessions intact. Successful replacement is atomic for new
admissions and revokes every admitted WebSocket, even if its grant is unchanged.
New requests then use the new credentials/scopes. An admitted stream also ends
at credential expiry; a monotonic deadline prevents clock rollback from extending
its admitted lifetime. Clock jumps forward are checked once a second. Revocation
drops both bridge endpoints; it does not promise a WebSocket close handshake.
Waiting list/get/delete/start and bridge-dial operations are cancelled on
revocation, with HTTP 403 instead of an empty success. Reload rechecks server
authority after serialization and the private file read. Completed HTTP/device
operations are not rolled back by a later policy change.

## Local and remote transport boundary

Default binding remains `127.0.0.1:49151`. Plain HTTP clients and listeners now
require a literal loopback IP or `localhost`; `localhost` resolves to the fixed
IPv4 loopback address in this interface. Nonloopback plaintext is rejected even
with a policy. For remote use, keep this listener on loopback and use a protected
host transport, such as an SSH local forward, with manager credentials still
required. Native HTTPS/WSS, per-upstream private credentials and certificate
acceptance remain S03 work. Current upstream calls have no credentials, so they
cannot federate with a manager in policy mode; do not deploy that topology yet.

`--local-unauthenticated` explicitly trusts every local process and conflicts
with `--auth-config`. It is accepted only on loopback and bypasses roles/scopes;
do not use it where local processes are untrusted. Host/Origin policies and
proper methods for legacy GET management operations are still S02 work.
Browsers cannot supply a custom WebSocket Authorization header; a browser
handshake mechanism will be provided by S02. Long-lived tokens must never be
placed in URLs.

The existing userspace proxy's 20-byte IPv6/port preamble remains unauthenticated
on loopback. Any local process able to reach that proxy can dial device ports
without the manager's HTTP grant. Kernel TUN access follows host route/firewall
permissions. The manager grant protects its HTTP/WebSocket requests; it does not
claim an OS sandbox for raw TCP or kernel routes.

| Existing entry | Enforced boundary | Credentials/protocol |
| --- | --- | --- |
| Userspace TUN proxy | Fixed `127.0.0.1`, ephemeral port; per-tunnel lifetime | No credentials in its existing IPv6/port preamble |
| Kernel TUN | Host route/interface/firewall permissions | No HTTP grant in an OS route |
| `forward` | Literal loopback IP/localhost, checked before device resolution | One CLI-selected device/port; raw TCP local trust |
| `runwda` proxy | Loopback and nonzero ports checked before WDA startup | One selected WDA/device port; raw TCP local trust |
| `dproxy service` | Loopback checked before service access/capture creation | One selected device/service; raw TCP local trust |
| `screenshot --stream` | Loopback checked before binding or capture | Existing raw multipart stream; local trust; F05 will use shared admission |
| CDP/Selenium | Default loopback; additional auth/browser admission unfinished | Pending under S01/S02 |

Explicit nonloopback `--host` values for forward/WDA/dproxy/raw screenshot
streams now fail instead of opening unauthenticated network services. Use a
loopback endpoint with a protected host transport for remote native clients.
`localhost` maps to fixed IPv4 loopback, and IPv6 loopback is accepted without
constructing a malformed host:port string. This migration preserves raw wire
protocols rather than adding an incompatible credential preamble.

Private reads use [secret_file](tunnel.md): Unix requires a regular file owned
by the effective user without group/other access, rejecting leaf symlinks and
FIFOs. Creation uses owner-only staging and no-clobber hard links; existing
destinations are preserved, and an existing shared Unix parent is rejected
without changing its mode. New private leaf directories can be created. Use
trusted parent paths; this is not atomic directory traversal or protection
against another process running as the same account.

The two output files are separate publications. If client-token creation fails
after the new policy was published, the command fails and gives a fixed partial
publication message. It keeps that policy and every existing destination for
inspection; it does not unlink paths another writer may have replaced. Inspect
the two requested paths before retrying. Cleanup failures can also occur after
publication. Windows inherits the parent's ACL and uses reparse-point rejection;
Windows/macOS execution and ACL policy acceptance remain pending.

Host raw-output validation now creates credentials only in its owned temporary
directory, reads the generated token with a fixed bound, and checks original
stdout/stderr before a separately masked copy. Success and a denied port are
both captured at maximum CLI verbosity, without a default device selector. No
real policy, credential, device or sensitive raw-output archive is touched.
The capture also runs the production loopback manager with list/server roles
only, which skip device discovery, and checks rejection, list, failed/successful
reload, CLI client and shutdown responses before examining original server logs.
