# Screenshot viewer, HEVC and clipboard

The default viewer captures CoreDevice PNG screenshots. Experimental `--hevc`
uses Display/RTP over a kernel tunnel, a single HEVC MPEG-TS track, and one
persistent external FFmpeg process. RTP's 90 kHz timestamps survive MPEG-TS/PES;
FFmpeg `showinfo` supplies decoded frame PTS and dimensions, BGRA rawvideo is
encoded to PNG by Rust, and HTTP/RFB consume the same latest immutable frame.
Reference pictures and parameter changes remain in the decoder. The original
raw RTP API is retained. There is no silent screenshot fallback for HEVC errors.

`ffmpeg` must be in PATH with HEVC decoding, MPEG-TS input, `showinfo` and BGRA
rawvideo output. It is not bundled. The Windows host gate used FFmpeg 9.0.2,
an external GPL/version3 full build. Other builds may be LGPL/GPL depending on
configuration; release packaging and codec/patent review remain pending. Rust
`png`, `des`, `arboard` and `jpeg-decoder` use MIT/Apache-2.0 licenses. macOS/Linux
execution and physical Display interoperability remain NOT_RUN.

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

Input uses Universal HID and Indigo and waits for their original writer. These
one-way reports have no device ACK. Pointer coordinates require a current frame;
loss of capture clears old dimensions. Direction changes use the existing
OrientationService backend and report success only after its request completes.
Unavailable backends and invalid directions return errors. Physical HID focus,
coordinate accuracy and orientation behavior remain NOT_RUN.

`--share-clipboard` enables bounded text, PNG and JPEG device pasteboard adapters
for HTTP/VNC. Device PNG/JPEG use XPC.Data and `public.png`/`public.jpeg`, supported
by the pinned upstream pasteboard protocol; physical iOS image exchange remains
NOT_RUN. Standalone `viewer clipboard` synchronizes desktop/device text only,
starts from the device baseline and suppresses repeats and echoes. Its policy
owner uses `--vnc-token-file` and the selected `--port` (default 5900) scope.

RFB bit 3 is a 124-byte DIB v5 header without a BMP file header. RGB PNG uses
24-bit BGR; alpha PNG uses 32-bit BI_BITFIELDS and preserves RGBA. RGB/grayscale
JPEG decodes to DIB; CMYK and 16-bit JPEG are rejected. Bit 4 is files and stays
unsupported. Profiles/palettes and over-limit payloads are rejected. Image loop
suppression hashes the actual transferred DIB pixels, including PNG/JPEG
re-encoding; text normalizes RFB line endings. Notify/Request/Provide, Caps and
SetEncodings renegotiation follow the standard extended-clipboard flow.

Encoded frames and image clipboard payloads are bounded at 2 MiB, decoded BGRA
at 12 MiB. The request queue has eight slots, HTTP at most two multipart streams,
and RFB one client. Slow consumers retain the newest shared frame; old frame
buffers release. These logical limits do not prove a whole-process RSS cap.
Web timestamps explicitly identify `rtp-relative-ms` or `host-unix-ms`; sequence
numbers distinguish reconnects that reset the RTP origin.

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

The [current implementation and verification](validation/production-completion-2026-10-26.md)
separates host gates from physical Display/HID/clipboard, independent desktop VNC
clients and platform acceptance. Older repair reports record earlier limitations.
