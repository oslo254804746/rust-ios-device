# Cryptex Developer Disk Images

The CLI selects the Cryptex backend for `ddi auto`, `ddi mount`, `ddi status`
and `ddi unmount` on iOS 27 and later. Earlier versions continue using
mobile_image_mounter. `ddi list`, `devices`, `lookup` and the legacy
personalization commands still query mobile_image_mounter; use
`ddi cryptex list` to inspect installed Cryptex images.

```sh
ios -u <UDID> ddi auto
ios -u <UDID> ddi mount --path /path/to/unpacked/Restore
ios -u <UDID> ddi status
ios -u <UDID> ddi cryptex list
ios -u <UDID> ddi cryptex personalization-identifiers
ios -u <UDID> ddi cryptex nonce --nonce-domain-handle <HANDLE>
ios -u <UDID> ddi cryptex auto-install --cache-dir /path/to/cache
ios -u <UDID> ddi cryptex uninstall com.apple.MobileAsset.DDI
```

Cryptex commands use a userspace RSD tunnel. Installation requires Developer
Mode and the service's advertised `CryptexInstall` / `ReadIdentifiers`
capabilities when capability metadata is present. An existing DDI Cryptex or
a Personalized image mounted at `/System/Developer` causes a conflict error.
Resolve that conflict explicitly before retrying installation.

Automatic installation downloads the pinned public DDI build `27A5228h` into
the cache, reads its BuildManifest, requests a Cryptex1 signing ticket from
Apple TSS, uploads the five announced payloads and verifies that the DDI
Cryptex appears in the installed list. Apple signing availability is required.
`--restore-dir` on `auto-install` selects local assets. `ddi mount --path`
also accepts the unpacked Restore directory on iOS 27+.

The manifest's nonce-domain handle drives personalization. It is distinct
from a nonce table index: `nonce --nonce-domain INDEX` and
`nonce --nonce-domain-handle HANDLE` are mutually exclusive. A nonce query
without either argument uses index 2; automatic installation always uses the
manifest handle. `uninstall` accepts an optional `--version VERSION`.

Each service routine after the first opens a fresh connection over the same
route. Custom Rust clients must provide `XpcClient::with_reconnector` for
multiple routines. Images stream from disk and are limited to 2 GiB; metadata
and tickets are limited to 16 MiB. Local asset paths must remain inside the
Restore directory. HTTP, service and installation operations have deadlines.

Host tests cover personalization fields, nonce wrappers, asset bounds and
path checks, response validation, five file streams, HTTP/2 flow control and
reconnection. Physical-device installation and live TSS acceptance still need
validation. The unverified upstream `roll-nonce` routine is not exposed.
