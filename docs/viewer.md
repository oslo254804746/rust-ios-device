# Screenshot viewer, HEVC and clipboard

The viewer exposes Web (`ios viewer web`), VNC (`ios viewer vnc`), and
standalone clipboard synchronization (`ios viewer clipboard`) subcommands:

```sh
# Web viewer (default port 8080) with clipboard sharing and local loopback trust:
ios -u <UDID> viewer web --port 8080 --local-unauthenticated --share-clipboard

# VNC RFB 3.8 server (default port 5900) with local loopback trust:
ios -u <UDID> viewer vnc --port 5900 --local-unauthenticated --share-clipboard

# Policy-protected Web or VNC viewer:
ios -u <UDID> viewer web --port 8080 --auth-config PRIVATE_POLICY_FILE --share-clipboard
ios -u <UDID> viewer vnc --port 5900 --auth-config PRIVATE_POLICY_FILE \
  --vnc-token-file PRIVATE_OWNER_TOKEN --password vncpass --share-clipboard
```

The default viewer captures CoreDevice PNG screenshots (`screencaptureservice`),
supporting both standard 8-bit sRGB and Pro Max 16-bit Display P3 PNG frames
(downsampling 16-bit RGBA channels to 8-bit RGBA/BGRA with a 16 MiB pixel buffer
budget for full-resolution 1290×2796 frames). Experimental `--hevc` uses
Display/RTP over a kernel or userspace tunnel, a single HEVC MPEG-TS track, and
one persistent external FFmpeg process. RTP's 90 kHz timestamps survive
MPEG-TS/PES; FFmpeg `showinfo` supplies decoded frame PTS and dimensions, BGRA
rawvideo is encoded to PNG by Rust, and HTTP/RFB consume the same latest
immutable frame. Reference pictures and parameter changes remain in the decoder.
The original raw RTP API is retained. There is no silent screenshot fallback for
HEVC errors.

`ffmpeg` must be in PATH with HEVC decoding, MPEG-TS input, `showinfo` and BGRA
rawvideo output when `--hevc` is selected. It is not bundled. Rust `png`, `des`,
`arboard` and `jpeg-decoder` use MIT/Apache-2.0 licenses.

Choose an exact device with root `--udid`. Policy HTTP clients need both `debug`
for that device and `connect` for its bound listener port. Native requests use
Bearer headers; `/login` holds the token only in memory, never a URL or browser
storage. Local anonymous access requires explicit `--local-unauthenticated`.
Host/Origin checks and existing protected transport rules still apply.

VNC stays loopback-only with one controlling client. Policy mode requires
`--vnc-token-file` and `--password`; RFB DES authenticates to that policy owner
and does not replace its device/port scope. The legacy password is 1..8 bytes.
RFB supports Raw, DesktopSize and text/DIB v5 Extended Clipboard with the
advertised little-endian 32-bit true-color format. Unknown formats are rejected.

Input uses Universal HID and Indigo and waits for their original writer. Universal
HID media authorization tries kernel TUN first and automatically falls back to
the Userspace Tunnel (using userspace IPv6 UDP datagram forwarding for the
Display/RTP authorization stream and a 500ms surface auth settle delay). These
one-way reports have no device ACK. Pointer coordinates require a current frame;
loss of capture clears old dimensions. Direction changes use the
OrientationService backend and report success only after its request completes.
Unavailable backends and invalid directions return errors.

`--share-clipboard` enables bounded text, PNG and JPEG device pasteboard adapters
for HTTP/VNC. Device PNG/JPEG use XPC.Data and `public.png`/`public.jpeg`, supported
by the pinned upstream pasteboard protocol. Standalone `viewer clipboard`
synchronizes desktop/device text only, starts from the device baseline and
suppresses repeats and echoes. Its policy owner uses `--vnc-token-file` and the
selected `--port` (default 5900) scope.

RFB bit 3 is a 124-byte DIB v5 header without a BMP file header. RGB PNG uses
24-bit BGR; alpha PNG uses 32-bit BI_BITFIELDS and preserves RGBA. RGB/grayscale
JPEG decodes to DIB; CMYK and 16-bit JPEG are rejected. Bit 4 is files and stays
unsupported. Profiles/palettes and over-limit payloads are rejected. Image loop
suppression hashes the actual transferred DIB pixels, including PNG/JPEG
re-encoding; text normalizes RFB line endings. Notify/Request/Provide, Caps and
SetEncodings renegotiation follow the standard extended-clipboard flow.

Encoded video access units and image clipboard payloads are bounded at 2 MiB,
and decoded pixel buffers are bounded at 16 MiB (allowing 1290×2796 Pro Max
BGRA frames at 14.43 MiB). The request queue has eight slots, HTTP at most two
multipart streams, and RFB one client. Slow consumers retain the newest shared
frame; old frame buffers release. Web timestamps explicitly identify
`rtp-relative-ms` or `host-unix-ms`; sequence numbers distinguish reconnects
that reset the RTP origin.

Capture/device operations use ten-second deadlines. Media retry attempts default
to three, with configurable 500 ms base delay and exponential backoff capped at
16x. Only eligible device disconnect/timeout/missing-device errors retry, after
original NativeScope collection. Decoder failures and every cleanup failure
stop retry. Read-only pasteboard GET may refresh once; SET/HID are not replayed.

Stop releases potentially pressed contacts/keys/buttons and joins the original
capture/device/server/session/decoder tasks. Retained Display startup survives
caller cancellation for strict cleanup; confirmed MediaInUse does not authorize
stopping someone else's media. Stop requires a real response. `stopAll` remains
a service-level operation: no verified per-stream UInt32 token ownership schema
is claimed. Cleanup keeps original primary/secondary causes and actual native
joins, including a separate failure when the operation itself succeeded.

See the [capability inventory](capabilities.md) for host and physical-device
verification records across iOS 17–27.2.
