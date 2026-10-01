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

## Browser handshake work still pending

Origin checks alone do not finish browser admission. The browser
[WebSocket constructor](https://websockets.spec.whatwg.org/#the-websocket-interface)
accepts a URL and protocols, without a custom Authorization-header parameter.
Native clients can supply the bearer header, but browser WS tickets, controlled
credential bootstrap and browser tests remain S02 work. No token query parameter
or cookie currently authenticates a WebSocket. Never put a long-lived token in
a URL. `--allow-origin` is an admission restriction, without automatic CORS
headers or a browser authentication flow. These host checks do not certify a
Playwright/Puppeteer, Safari automation or physical-device workflow.
