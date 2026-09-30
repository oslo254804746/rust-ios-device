# Management access configuration

S01 is in progress. `ios tunnel auth create|check` prepares and checks private
scoped credentials on the host. These commands do not start a server, pair a
device, deploy a policy or change manager admission. The current manager still
has no authentication; HTTP/WS server and client integration, reload/revocation,
CDP/Selenium admission and the existing TCP forwarding boundary remain pending.
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
256 ports per grant. A generated file is ready for the future admission
integration; editing or creating one currently cannot secure a running manager.

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
