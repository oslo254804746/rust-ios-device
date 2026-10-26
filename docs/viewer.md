# Screenshot viewer, experimental HEVC and clipboard

The viewer uses CoreDevice PNG screenshots, decodes the actual pixels with the
Rust `png` crate, and shares each image with HTTP and RFB consumers. Experimental `--hevc` selects Display/RTP over a kernel tunnel and a persistent
FFmpeg HEVC-to-PNG subprocess; decoder/reference state survives access units.
The default screenshot mode does not attempt a kernel video connection. `png` and `des` are Rust dependencies under
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
supports Raw, DesktopSize and text/DIB v5 Extended Clipboard with the advertised
little-endian 32-bit true-color pixel format; other formats are rejected.

Input uses the existing Universal HID and Indigo clients and waits for the
original writer to finish. Their one-way reports have no device acknowledgement;
a successful write does not prove a contact changed on the device. Universal HID requires the kernel tunnel/display
authorization side channel. HTTP reports an unavailable backend or failed
operation as an error. Direction injection has no verified service mapping and
returns 501. There is no synthetic orientation success.

`--share-clipboard` enables bounded text and experimental PNG pasteboard reads/writes for HTTP/VNC.
Standalone `viewer clipboard` polls the device and local clipboard, starts with
the device baseline, preserves actual text, and suppresses repeated/echoed
content. Its policy owner uses `--vnc-token-file` and the selected `--port`
(default 5900) scope. HTTP/device adapters contain generic PNG/JPEG UTI read/write paths, but device image interoperability has not been established. Treat those paths as experimental. RFB bit 3 uses a 124-byte DIB v5 header (no BMP file header), with bounded 24/32-bit RGB and standard BGRA bitfields; embedded profiles and palettes are rejected. PNG images convert to 24-bit BGR DIB, discarding alpha. Bit 4 remains unsupported files. JPEG-to-DIB conversion is not implemented. The text limit is 1 MiB; extended clipboard parsing is bounded.

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

F05 remains incomplete: orientation, device image protocol evidence,
reconnect/desktop-platform coverage and actual standard VNC/device workflows
need further implementation or acceptance evidence. No real device was
operated during this repair. See the [repair scope](validation/viewer-cdp-repair-2026-10-10.md).

The [2026-10-26 review](validation/remaining-six-review-2026-10-26.md) removed fabricated RFB PNG/JPEG production messages. The historical private-codec roundtrip is a unit regression only and does not establish RFB or device image support.

HEVC decoder dependencies are external: `ffmpeg` must be in PATH with HEVC and PNG codecs.
FFmpeg licensing depends on the installed build (LGPL/GPL and optional components); the executable is not bundled here. Release packaging and codec/patent review remain pending.
The production pipe drains PNG concurrently with access-unit writes and retains only the latest shared frame for Web/VNC. Decode/read/write errors propagate; there is no silent screenshot fallback after selecting `--hevc`. Stop kills and waits for the decoder, then calls the existing Display stop and native collection. Raw Annex-B to PNG drops RTP timestamp metadata; viewer timestamps are explicitly host decode wall time in milliseconds. RTP-tick wrap/reordering is not represented as presentation time. Real Display startup/cancellation and per-stream stop identifiers still need device evidence. The legacy synchronous `HevcDecoder`/`HevcPipeline` helper tests only independent intra frames; production uses the persistent asynchronous process.

RFB advertises text and DIB with separate zero unsolicited limits. Changes are announced with Notify; Request receives the actual payload. Client Caps restrict outgoing formats, and re-negotiating SetEncodings resets that observation. Peer Notify requests supported data; files remain rejected. The localhost gate verifies both DIB directions through the actual RFB session and the fake device adapter, separately from any claim about physical iOS UTI handling.

Clipboard loop suppression fingerprints the transferred PNG pixels through the same bounded DIB representation, so a PNG/DIB/PNG re-encode cannot evade it just by changing compression or metadata. Text fingerprints use normalized RFB line endings. This does not establish physical-device image UTI behavior, and the current outbound 24-bit DIB still discards alpha.
