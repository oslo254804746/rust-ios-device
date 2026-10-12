# Screenshot viewer and text clipboard

The viewer uses CoreDevice PNG screenshots, decodes the actual pixels with the
Rust `png` crate, and shares each image with HTTP and RFB consumers. It does not
implement HEVC/RTP video decoding. `png` and `des` are Rust dependencies under
MIT/Apache-2.0; `arboard` supplies the desktop text clipboard (MIT/Apache-2.0).
Dependency licenses and target packaging still require release verification.

Choose an exact device with the root `--udid` option. HTTP clients need a policy
grant containing both `debug` for that device and `connect` for that device and
the viewer's bound listener port. Native requests use the bearer header. The
browser `/login` page contains no device data and accepts a token into memory;
its authenticated `/frame` and control calls use the bearer header. The token is
not placed in the URL or browser storage. Local anonymous access requires the
existing explicit `--local-unauthenticated` loopback option.

For policy-mode VNC, supply `--vnc-token-file` and `--password`. The private token
selects one scoped policy Principal for the RFB owner. Every RFB session and
pending device request retains that Principal; expiry and policy replacement
revoke the owner. RFB DES authenticates clients to this owner, and does not carry
or substitute for the policy credential. The legacy RFB password must contain
1..8 bytes. VNC remains loopback-only and accepts one controlling client. It
supports Raw, DesktopSize and text Extended Clipboard with the advertised
little-endian 32-bit true-color pixel format; other formats are rejected.

Input uses the existing Universal HID and Indigo clients and waits for the
original writer to finish. Their one-way reports have no device acknowledgement;
a successful write does not prove a contact changed on the device. Universal HID requires the kernel tunnel/display
authorization side channel. HTTP reports an unavailable backend or failed
operation as an error. Direction injection has no verified service mapping and
returns 501. There is no synthetic orientation success.

`--share-clipboard` enables bounded text pasteboard reads/writes for HTTP/VNC.
Standalone `viewer clipboard` polls the device and local clipboard, starts with
the device baseline, preserves actual text, and suppresses repeated/echoed
content. Its policy owner uses `--vnc-token-file` and the selected `--port`
(default 5900) scope. PNG/JPEG device clipboard writes remain unsupported and
are rejected. The text limit is 1 MiB; extended clipboard parsing is bounded.

Encoded PNG frames are limited to 2 MiB and decoded BGRA pixels to 12 MiB.
The device request queue has eight slots; HTTP has at most two multipart
streams, which share immutable frame bytes; RFB has one client. Capture and
device operations have ten-second deadlines. Stop wakes blocked sessions,
releases potentially pressed contacts/keys/buttons, and joins the original
capture/device/server/session tasks. Errors retain the primary and cleanup
objects. These logical limits do not establish a 64 MiB whole-process RSS cap.

Capture and cached pasteboard/HID connections now use the original native
retirement registry with four foreground connection slots. Cached service
clients retain their tunnel connection until service close. Cancellation during
startup preserves the original native owner; capture retirement and final
shutdown collect its real joins. An incomplete native cleanup remains an error
with the original registry retained, rather than an inferred success from task Drop.

F05 remains incomplete: HEVC video decoding, orientation, image clipboard,
reconnect/desktop-platform coverage and actual standard VNC/device workflows
need further implementation or acceptance evidence. No real device was
operated during this repair. See the [repair scope](validation/viewer-cdp-repair-2026-10-10.md).
