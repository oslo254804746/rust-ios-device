# Q02 host regression evidence acceptance — 2026-10-01

Implementation: `e9cc536351d08d2dc1fa3504d69259b9f215e39b`.
Branch: `feat/device-service-updates-20260927`; ten related files only.
Linux x86_64, Rust 1.97.1/MSRV 1.80.1, Python host binding 3.12.13;
tool interpreter 3.14.4, additional tool runtimes 3.9.25/3.13.15.
[Contract](../regression.md), [matrix](../regression-matrix.json),
[FUP review](../real-device-followup-tasks-2026-08-30.md).

| Check | Actual result |
| --- | --- |
| Relevant `python3 -m unittest discover -s scripts -p 'test_*.py'` | 34 PASS; 17 new runner tests |
| `python3 scripts/validate.py regression` | Five host PASS; 22 real-device NOT_RUN; owned file/group cleanup complete |
| Python 3.9/3.13 `-m unittest discover -s scripts -p 'test_regression.py'` and `scripts/regression.py` | Each 17 tests and five host cases PASS; 22 NOT_RUN each |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; final log `/tmp/ios-q02-regression-full-final.log` |
| Split Rust/Python, locked full check/strict Clippy, MSRV | 1651 Rust, eight Python PASS, zero FAIL; two explicit ignored |
| Features/docs/ABI/artifacts | 66 feature builds/four group suites, 7+7 doctests/three snippets, C ABI/19-byte capture, wheel/sdist rebuild/install/import PASS |
| Inventory/raw diagnostics | 118 records/actual help and 38 original plus independent masked-copy captures PASS; no raw archive |
| Workflow/source checks | actionlint, diff/staged whitespace check and Python 3.9 syntax PASS |
| Clean implementation commit runner rerun | HEAD `e9cc536`, dirty=false, five PASS/22 NOT_RUN, complete owned cleanup |

The two Rust ignored tests are the preexisting expensive PBKDF2 case and the
browser ticket probe requiring explicit dependencies. The latter has separate
S02/S03 actual Chromium PASS evidence; this Python-only change did not rerun it
and does not count an ignored test as PASS. The earlier full run also passed;
review found parser-error echo and case-validation ordering gaps, then relevant
tests and the full gate ran again on the final implementation.

The clean-commit metadata report is available locally at
`/tmp/ios-regression-evidence-eur6rg8k/report.json`, outside Git. Its selected
binary version is 0.1.13; SHA-256:
`fc718d5b92f2a25f21ed1a5485edc5cf90d3d49a575b112bbd192df360ea27f9`.
Runner SHA-256: `394f3a4884917ab567bdc4a66ef6cbf06edbb4ff8c37c519432844356f3da327`;
capture SHA-256: `0ebf50bd1c1f84b2897d13a6e18a2bfc5ea054763e24cef97e49a7ddac3f99c1`;
matrix SHA-256: `868150791d815adf861e45ec21e1f38f527d8c1d104f9ce0b5813a943bab8a6a`.
The full precommit run accurately marked its working tree dirty. The later
clean-HEAD rerun confirms the same committed script/matrix and binary hashes.

The report directory is 0700 and its sole `report.json` is 0600. Three owned
private files were removed, with zero unknown/unsafe files or agent failures.
Positive leak controls intentionally produce FAIL records while their archived
metadata stays secret-free. Agent SIGTERM output goes through the same original
and masked-copy scan. Successful parents leaving closed-pipe descendants are
cleaned, while an unrelated owned test session remains running until the test
itself ends it. Unknown files and replaced regular/symlink identities remain
untouched and fail cleanup; an unknown evidence file prevents publication.

These are host fixtures and safe CLI parser/version operations. Device/iOS and
states remain null/not-observed; no physical workflow ran. A per-run keyed argv
tag is not the raw value. Only the synthetic Python child has independently
observed `sys.argv`; other cases explicitly record direct OS inputs. This does
not certify Windows/MSYS argv, Windows process trees/ACLs, macOS aliases/remoted,
actual UNC or a detached third-party agent. The Python Session API accepts only
trusted host case callers; future device adapters still need their exact scopes,
preconditions, output integrity checks and acceptance evidence.

FUP-02's existing public documentation and retained coverage boundary passed
text review. FUP-01's public AFC/MSYS article exists, but both indexed local
historical reports and the old Windows evidence root are absent here, so local
errata and independent historical evidence review remain pending. FUP-03's new
Linux host tool scope passed; the unavailable old external tool was not edited.
All FUP-04 cases remain NOT_RUN: no Windows host, owner-placed ordinary-AFC
synthetic file/known size/hash/exact read scope, or separately authorized UNC.
Modern identities, services, Cryptex, display/input, clipboard, application,
XCTest and InstallCoordination wire observations require their own conditions.
No pairing, DDI/TSS/restore/erase, push, release, history rewrite or reference
checkout edits occurred. Q02 remains pending conditions; class two's current
host delivery is closed and independent class-three work can proceed.
