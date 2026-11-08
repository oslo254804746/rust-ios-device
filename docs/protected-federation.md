# Protected federation and per-upstream credentials

The manager's `/connect` binary WebSocket and `hop` query are project protocol
extensions. They are not Apple endpoints. Each manager validates the incoming
grant and independently authenticates its own configured next hop; incoming
headers, browser tickets and credentials from listing data are never forwarded.

## Server and transport boundary

The native manager/CDP/Selenium listeners remain plaintext **loopback only**.
Remote callers must reach them through a separately established, protected host
transport. For example, an administrator can use SSH local forwarding with host
key verification, or a TLS reverse proxy that verifies its server certificate.
Do not expose the plaintext listener through an unprotected network forward.
Preserve an explicitly accepted Host authority and remove Forwarded/
X-Forwarded-* headers before the direct listener. Configure exact browser Origin
separately; see [HTTP/browser boundaries](http-boundary.md).

Native upstream calls now support HTTPS listings and WSS connections, with
certificate chain, validity and hostname verification. HTTP/WS is accepted only
for a **literal loopback IP**, including an independently protected local host
forward. `http://localhost` and all remote/DNS plaintext URLs now fail: use
`http://127.0.0.1:PORT/`, `http://[::1]:PORT/`, or verified HTTPS. An HTTPS hostname
can resolve to loopback, but it still needs a certificate valid for that name.
There is no option to disable certificate verification or downgrade WSS to WS.

The HTTP client disables environment proxies and redirects. WebSocket dialing
uses the selected root's WS/WSS scheme and does not retry/follow redirects.
Original HTTP/WS credentials are bound to the configured scheme/host/effective
port. No credential appears in a URL, redirect request, successful listing or
manager's upstream URL list.
The implementations use the pinned
[reqwest TLS/redirect client](https://docs.rs/reqwest/0.12.28/reqwest/struct.ClientBuilder.html)
and [tokio-tungstenite TLS connector](https://docs.rs/tokio-tungstenite/0.24.0/tokio_tungstenite/fn.connect_async_tls_with_config.html);
the gate exercises the actual configured clients, including rejected certificates.

## Private startup configuration

```sh
ios tunnel serve --userspace --auth-config PRIVATE_INCOMING_POLICY \
  --upstream-config PRIVATE_UPSTREAM_CONFIG
```

Use a private version-1 JSON file, bounded to 64 KiB:

```json
{
  "version": 1,
  "upstreams": [
    {
      "url": "https://manager.example.test:8443/",
      "token_file": "manager-one-token",
      "ca_file": "manager-one-ca.pem"
    },
    {
      "url": "http://127.0.0.1:8500/",
      "token_file": "manager-two-token"
    }
  ]
}
```

The second example requires local trust or an independently verified protected
forward. The CLI does not establish SSH/TLS forwarding. Give each upstream its
own private credential and least-privilege grant at that upstream, covering only
the required list/device/port permissions. Admin permission is not required for
ordinary federation. Prepare grants with the existing `tunnel auth create|check`
commands and install the chosen policy at its owning upstream server.

Paths are absolute or relative to this configuration file's directory. Policy,
token and optional CA reads use the shared bounded private-file rules, including
permission/symlink checks on supported hosts. Credentials contain the existing
64 lowercase hexadecimal characters. There are at most 32 normalized, distinct
root URLs; userinfo, query/fragment, non-root paths, zero port, unknown fields,
duplicate normalized URLs and unsupported versions fail startup before binding
or device discovery. URL input is at most 2048 bytes; each private path 4096.
These files are not writable through the HTTP API.

HTTPS uses the pinned WebPKI roots. Optional per-upstream `ca_file` adds up to
16 PEM certificates (64 KiB maximum) to that upstream's trust store only; it does
not change any other upstream's trust. A CA file on HTTP fails. The same root
store/verifier is used for that upstream's HTTP and WebSocket traffic. Custom
trust requires a private, controlled file even though CA certificates are public.
Do not reuse the repository's public test-only fixture CA/key outside tests.

Configured roots are registered at startup. Admin add/delete/list `/upstream`
retain their existing authorization and URL-only response shape. Added roots
without a startup credential use verified HTTPS or literal-loopback local trust;
they do not borrow another root's credential. Removing/readding a configured URL
restores its configured credential. Incoming `/auth/reload` replaces only the
incoming policy; changing upstream credentials/trust requires server restart.
Already admitted streams keep their existing connection until disconnect or
incoming/upstream grant revocation; editing a token file alone does not revoke it.

## Failure, selection and lifecycle

Listing fetches retain a two-second whole-request timeout and 1 MiB response
bound. Dialing retains the 15-second timeout; frames/messages are bounded to
64 KiB. Certificate, authentication, redirect, malformed/oversized body and
transport errors fail federation listing/selection with a safe error rather
than an empty success. Successful local `/tunnels` remains a local-only view.
The aggregate `/` requires all attempted upstream listings to succeed; an
unavailable registered upstream can therefore make that aggregate fail.

Manager-to-manager dial selects only its own registered next hop. A listing's
next-upstream value is a selector sent to that next manager, never an address
that this manager dials with its current credential. Hop limit, cycle refusal,
selected interface/root, EOF, close and backpressure keep their existing contract.
Expiry/reload of the incoming principal still drops both bridge endpoints; an
upstream's revocation propagates closure back through the relay.

Linux tests exercise actual HTTPS/WSS, valid custom CA, untrusted/expired/wrong
name refusal before backend HTTP, redirects, per-root header isolation and two
production managers with distinct hop credentials/bytes/revocation. These are
owned host fixtures, not physical-device or deployed SSH/reverse-proxy evidence.
Platform file/ACL, actual deployment certificates/host keys and device workflows
remain acceptance conditions in the roadmap.
