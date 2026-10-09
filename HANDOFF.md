# Antigravity Windows handoff

Continue on branch `codex/antigravity-handoff-20261009` in one checkout.
Accepted baseline: `993a36685f04cf1f2922af772831f9de6b16a46c`.
The `codex/handoff-base-20261009` tag identifies that baseline. The
`codex/antigravity-start-20261009` tag identifies the complete migration starting
point, including these notes. Keep that tag unchanged for the return bundle.

## What is already consolidated

| Commit | Draft | State |
| --- | --- | --- |
| `89d6348` | Python finite existing-trust device queries: binding, examples and installed-query fixture | WIP; patch application checked, compilation and API/runtime acceptance pending |
| `7c67ab1` | C finite existing-trust device queries: handles, header, examples and linked fixture | WIP; patch application checked, compilation/ABI/runtime acceptance pending |
| `a126695` | Darwin pasteboard watch recovery, fresh discovery, baseline/deduplication, budgets and original native cleanup | WIP; patch application checked, compilation and host/physical acceptance pending |

These three drafts apply cleanly to the accepted current Core implementation.
Their historical Core overlays were excluded: the baseline already contains the
accepted Core implementation and later corrections. Do not replace it with a
stale worktree's Core files. The WIP commits preserve unfinished code; their
presence does not declare A03 or L03 complete.

All 21 original dirty worktrees are separately preserved in the migration
package's `worktree-backups/`: original commit/branch, staged/unstaged binary
patches, non-ignored untracked files and content hashes. Many represent earlier
versions of work already integrated and corrected in the baseline. Do not apply
all backups to the handoff branch. Consult one only for provenance or a missing
change, comparing it with current code first. Original Linux worktrees and the
original main branch remain available on the source machine.

## Development scope

Read [development-backlog](docs/development-backlog.md) and
[current scheduling](docs/roadmap-current.md). There are 11 remaining development
items: Q02, A03, L01, L02, L03, L04, F01, F02, F04, F05 and F06.
Q04 and all its children are excluded. Under the user's current accounting,
implemented deliverables merely awaiting verification count as development
complete. Keep actual NOT_RUN results and capability maturity independent.

Start with A03: inspect the consolidated Python/C drafts against the current
Core API, fix integration/compilation issues, run relevant host checks, then
continue applications, files and logs. Review the L03 draft as the next bounded
delivery. Existing host validation scripts are available; use their actual
supported platform scope.

`scripts/ffi_device_session.py` and `device_session_smoke.c` currently implement
a POSIX pthread linked fixture. They explicitly lack a Windows counterpart.
The allocator probe is Linux/glibc only. Do not interpret those platform limits
as missing Windows environment dependencies or convert them into PASS. Develop
a counterpart when required; otherwise keep that validation pending and execute
it on Linux when the changes return for review. Device tests remain deferred to
the user. Physical restore/erase operations have not been authorized.

## Work and review

- Keep one main development checkout and preserve the accepted baseline/history.
- Group related implementation, examples and documentation into complete work
  packages. Commit each reviewable package locally.
- Serialize independent Rust builds, including maturin/uv sdist builds. Keep a
  shared target directory if additional worktrees become necessary.
- Retain the temporary-cache fix in `scripts/artifacts.py`; build outputs and
  environments are recreated on the destination machine.
- Record changed behavior, actual commands/results, pending conditions and the
  final commit. Keep protocol/cleanup assertions when correcting fixtures.
- Continue independently executable work when another item lacks device or
  protocol evidence; report those gaps precisely.

The migration's clean-clone/content and backup recovery checks do not constitute
feature acceptance. The consolidated drafts have not been compiled or tested
as a combined product by the migration task.

## Return to Codex

After committing the finished work on the handoff branch and checking the tree
is clean, export only the commits added after the migration start tag:

```powershell
git status --short
git bundle create antigravity-result.bundle codex/antigravity-start-20261009..codex/antigravity-handoff-20261009
git bundle verify antigravity-result.bundle
```

Return that bundle and a short delivery report. The source machine already has
its prerequisite commits. Codex can fetch into a separate review branch:

```bash
git fetch /absolute/path/antigravity-result.bundle \
  refs/heads/codex/antigravity-handoff-20261009:refs/heads/codex/antigravity-review-20261009
```

Review the final implementation against `codex/handoff-base-20261009`, including
the original WIP drafts and subsequent corrections. Shipping a bundle requires
no GitHub push; pushing or publishing is not part of this handoff.
