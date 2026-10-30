# CLI device supplement reviewed on 2026-10-28

GPT-6 Luna executed run `20261028-165517-luna-cli`; the parent agent reviewed original
attempt evidence and rebuilt the canonical291-operation results.

The run contains131 attempt records:114 CLI attempts (one argument rejection)
and17 comparator/client/host-validator records. It exercised65 device and9 host
operations. Reviewed scoped results are46 device PASS,9 host PASS,
3 UPSTREAM_MATCH and16 INCONCLUSIVE;71 operations remain BLOCKED,
143 NOT_RUN and3 KNOWN_GAP.

Of the195 operations without prior business invocation evidence,48 device and
9 host operations were newly invoked. Prior96 plus57 new equals153/291 with
invocation evidence;138 still have none. Invocation is not functional acceptance.

New verified scopes include positive custom notifications, actual PCAP/GPU/energy
data,62 decoded HomeScreen icons, wallpaper preview, current RSD directory,
and isolated AFC0-byte/Unicode/1MiB transfer/rename/delete with SHA256 and final
absence/sibling-baseline verification. Local crypto/crash/backup fixtures are
counted as host tests.64MiB and other transport/SDK variants were not accepted.

Forward returned EOF through the Rust userspace-preferred path while pmd direct
USB returned QueryType; Rust once exited0. This remains a P2 candidate with route
differences documented. Dproxy transferred Marco/Polo but exited1 on close;
two sysmon probes required explicitly recorded forced termination. Web/VNC
produced no actual frames. These results are not PASS.

Original executor aggregates were rejected after blank-row corruption; final
statistics come from original per-attempt JSON/argv and independent artifact
checks. Incomplete early PTY lifecycle evidence remains inconclusive.

All owned processes/listeners and the dedicated AFC test directory were removed.
No restore/reset/reboot/trust/DDI/profile/system-route or personal-app changes
were performed. The pre-existing ios process was preserved. Product code was
unchanged during this supplement. Fixed binary SHA256:
`49d1a4dc7ba697ee378c253396ee70fbcf2d89dc2f66489a9265717fdd41b448`.

Detailed results remain outside the repository in the run directory's
`SUMMARY.md`, `operation-results.csv`, `attempts.csv`, and `private/reviewer/`.
