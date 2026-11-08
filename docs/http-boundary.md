# HTTP methods, listener authority and browser origins

The tunnel manager, CDP and Selenium apply the same boundary before credential
admission, parsing or device operations. Default binding remains loopback.
Host/Origin checks add restrictions; they do not grant any credential, role,
device or port permission. Invalid requests receive safe HTTP 403 in the
manager/CDP/WebDriver envelope, `Cache-Control: no-store`, and an empty HEAD
body. HTTP syntax errors rejected before routing can instead receive parser 400.

## Host and Origin contract

By default, exactly one Host must match the listener's actual IP and port, or
`localhost` with that port. Ephemeral ports and bracketed IPv6 use the bound
endpoint. Missing/duplicate/unapproved Host is rejected. An absolute-form URI
must be HTTP and have the same authority as Host. Forwarded,
X-Forwarded-Host/Proto/Port are rejected; a proxy must deliberately preserve the
accepted authority and strip these headers. The service does not trust a proxy
header to rewrite its admission boundary.

Origin can be omitted by native clients. When supplied, exactly one origin
must match the listener's HTTP IP/localhost origin or an explicit allowed
origin. `null`, wildcards, userinfo, paths (including a trailing slash), query,
fragment, malformed/zero/out-of-range ports and duplicated headers fail.
Origins compare scheme, normalized host and effective port; HTTP and HTTPS
remain distinct. Native clients still need the scoped bearer header in policy
mode. Origin is a browser restriction, not identity: native clients can forge
it, as described in [RFC 6455 §§10.1–10.2](https://www.rfc-editor.org/rfc/rfc6455.html#section-10.2).

Additional exact authorities/origins can be supplied to each server:

```sh
ios tunnel serve --userspace --auth-config PRIVATE_POLICY_FILE \
  --allow-host 127.0.0.1:8000 --allow-host localhost:8000 \
  --allow-origin http://localhost:8000
```

This example permits the authority used by a separately established protected
host forward to the loopback listener. It does not establish that forward,
provide TLS, or open a browser UI. `--allow-host` requires an explicit nonzero
port. There are at most 32 extra entries per dimension; normalized duplicates
fail. Authority input is ASCII and at most 512 bytes. Settings are validated
before binding or starting the manager's device monitor. Adding an allowed host
does not add its origin. The default original listener origin remains allowed.
The same flags apply to `webinspector cdp` and `webinspector selenium`.

Debug listeners require an explicit global `--udid`/`IOS_UDID` selector instead
of resolving the first device. Missing selectors fail without global discovery;
other WebInspector commands retain the documented first-device behavior.

## Management method migration

GET/HEAD do not shut down the manager or create tunnels. This follows the safe
method contract in [RFC 9110 §9.2.1](https://www.rfc-editor.org/rfc/rfc9110.html#name-safe-methods).
Authenticated legacy GET/HEAD `/shutdown` and `/start-tunnel` return 405 with
`Allow: POST` and a fixed migration notice until **2026-11-01 00:00 UTC**. At that
instant the authenticated tombstone becomes 410, still without mutation. There
is no compatibility switch enabling GET mutation or bypassing authentication.
The old query selector is neither processed nor reflected in this notice.
Legacy shutdown notices require manage-server, start notices manage-tunnels.

| Operation | Method and body | Required scope |
| --- | --- | --- |
| Create selected tunnel | POST `/start-tunnel`, JSON `{"udid":"DEVICE_UDID","connection_type":"usb"}` | manage-tunnels for exact selected device |
| Shut down manager | POST `/shutdown`, no body | manage-server |
| Delete tunnel | DELETE `/tunnel/:udid` | manage-tunnels for exact selected device |
| Read list/record | Existing GET routes | list for exact allowed devices |
| Replace policy | POST `/auth/reload`, no chosen file | manage-server |

The start JSON retains optional `ip` and `connection_type` (`usb`, `usbmux`,
`wifi`), with the existing response shape and device implementation. An allowed
POST does not imply that a device is connected or its tunnel can be created.
The CLI's list GET and stop DELETE already use their correct methods. The
project's `/connect` WebSocket extension remains a protocol upgrade with exact
device/port authorization; an ordinary GET without upgrade headers is rejected
before dialing. It is not a generic reading endpoint or an Apple protocol.

## Browser bootstrap, CORS and single-use handshakes

The browser
[WebSocket constructor](https://websockets.spec.whatwg.org/#the-websocket-interface)
accepts a URL and protocols, without a custom Authorization-header parameter.
Native clients keep their scoped bearer header without requiring Origin.
Policy-mode browser clients obtain a **30-second, single-use** handshake from
POST `/auth/ticket` on the manager or CDP server. Selenium has no WebSocket
endpoint and does not expose this ticket route; approved browser HTTP requests
use their bearer header. No cookie or long-lived URL token authenticates any
server. Explicit local-trust mode retains its documented trust in local
processes; tickets do not make that mode credential protected.

Run only a controlled UI on an explicitly approved origin. Read the private
credential with a user-selected file input into memory and send it in the
bootstrap HTTP Authorization header. Clear that credential when the UI ends;
do not put it in a URL, localStorage, cookie, analytics or diagnostic log. The
server does not host a login UI or distribute private credential files.

```js
// Controlled UI code: base is the configured loopback/protected server URL;
// token is read from the user's selected private file and kept only in memory.
const response = await fetch(`${base}/auth/ticket`, {
  method: 'POST', credentials: 'omit', cache: 'no-store',
  headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
  body: JSON.stringify({
    origin: location.origin,
    target: { kind: 'tunnel', udid: selectedDevice, port: selectedPort },
  }),
});
if (!response.ok) throw new Error('Browser handshake denied');
const { websocket_path } = await response.json();
const endpoint = new URL(base);
endpoint.protocol = endpoint.protocol === 'https:' ? 'wss:' : 'ws:';
const socket = new WebSocket(new URL(websocket_path, endpoint));
```

Manager targets accept `kind: "tunnel"`, exact `udid`/nonzero `port`, optional
`interface`/`upstream`, and `hop` (default 0, maximum 3). They require connect
for that device/port; list/admin permissions do not imply connect. CDP targets
accept `{"kind":"cdp_page","page_id":"PAGE_ID"}` or
`{"kind":"cdp_browser"}` and require debug for the server's selected device.
The response contains `ticket`, `expires_in: 30` and `websocket_path`. Target
fields are bounded, unknown JSON fields fail, and the bootstrap body is at most
16 KiB. The registry retains only ticket digests, with at most 512 pending
entries; capacity exhaustion returns 429 and entropy failure 503.

Use the returned path unchanged. A handshake needs an approved Origin, GET and
WebSocket upgrade, and one canonical `ticket` parameter. Its exact Origin and
path/query (including target, option values, hop and query order) must match
issuance. Origin compares the same normalized key as the HTTP boundary. A
browser bootstrap's JSON origin must match its actual Origin; a native
bootstrap may choose an explicitly approved origin. Missing Origin, duplicated
or encoded ticket aliases, ticket plus Authorization, expired ticket, replay,
target substitution and long-lived credential in the ticket field return safe
401. A redemption on a different approved Origin/target burns the ticket;
unapproved Origin is rejected by the outer boundary before redemption. Tickets
authenticate only their one handshake, never metadata or management HTTP.
They are removed from the routed URI and are not forwarded upstream.

The handshake lifetime ends at 30 seconds or earlier grant expiry/reload. After
admission, the connection follows the original principal's grant lifetime and
revision, rather than the handshake's short deadline. Successful policy reload
invalidates every old pending ticket and connection, including unchanged
grants; failed reload preserves the old snapshot. Browser errors do not promise
a graceful WebSocket close handshake or orderly device shutdown.

Approved browser origins receive exact `Access-Control-Allow-Origin` and
`Vary: Origin` on routed replies. OPTIONS preflight on a known route permits
GET/HEAD/POST/DELETE and only Authorization/Content-Type headers, returns 204
with no-store, and never executes or authenticates an operation. Unknown routes,
unapproved Origin, unsupported methods/headers and malformed preflight fail.
Actual requests still pass scoped admission; cookies and CORS credentials are
not enabled. The preflight describes the shared header/method vocabulary; a
route can still reject an unsupported actual method.

Short-lived tickets necessarily enter handshake URLs and may appear in browser
developer tools or a forwarding server's access logs. Disable URL/header/frame
logging on the controlled UI and protected forwarder. The CLI's reviewed logging
filter does not enable dependency handshake/frame logging. This scheme does not
provide listener TLS: an HTTPS UI may require a protected WSS transport as
described in [protected federation](protected-federation.md), and browser
mixed-content/local-network policy may reject a plaintext local connection.

The explicit browser gate uses actual
Chromium fetch/WebSocket against the Rust production manager and owned host
fixtures. It checks access control, binary bytes and revocation, without an
Apple service. F03 client/CDP protocol acceptance and physical-device workflows
remain separate; F05 must apply this boundary to its future browser entry points.
