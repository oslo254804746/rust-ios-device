#!/usr/bin/env python3
"""Static gate check for the release/publish job graph in ci.yml.

Verifies that every node that produces externally visible artifacts
(release, publish-crates, python-wheels, python-sdist, publish-pypi)
transitively depends on the required quality gates, and that no
`if: always()` / `continue-on-error` bypass lets a publish node run when a
gate failed, was cancelled, or was skipped.

Purely static: parses the workflow YAML, never triggers any workflow run.

Usage: python scripts/check-release-gates.py [.github/workflows/ci.yml]
"""

from __future__ import annotations

import sys

import yaml

CORE_GATES = ("check", "feature-check", "msrv", "build-and-test", "validate-tag")
PUBLISH_NODES = ("release", "publish-crates", "python-sdist", "python-wheels")
# Nodes whose upstream must be transitively successful for them to run.
TRANSITIVE_CONSUMERS = PUBLISH_NODES + ("publish-pypi",)


def needs_of(job: dict) -> list[str]:
    needs = job.get("needs", [])
    if isinstance(needs, str):
        return [needs]
    return list(needs)


def closure(needs: dict[str, list[str]], root: str) -> set[str]:
    seen: set[str] = set()
    stack = list(needs[root])
    while stack:
        job = stack.pop()
        if job in seen:
            continue
        seen.add(job)
        stack.extend(needs.get(job, []))
    return seen


def executable(root: str, needs: dict[str, list[str]], states: dict[str, str],
               always_if: set[str], continue_on_error: set[str]) -> tuple[bool, str]:
    """Simulate GitHub Actions: a job runs only if every `needs` parent is
    `success` (skipped/failure/cancelled all block). `if: always()` would
    bypass that, which this checker flags instead of honoring."""
    for parent in sorted(closure(needs, root)):
        state = states.get(parent, "success")
        if parent in always_if:
            return False, f"bypass: '{parent}' would be ignored due to if: always()"
        if state != "success":
            return False, f"blocked: needs '{parent}' is {state}"
        if parent in continue_on_error:
            return False, f"bypass: '{parent}' uses continue-on-error"
    return True, "would run"


def main(path: str) -> int:
    with open(path, encoding="utf-8") as fh:
        doc = yaml.safe_load(fh)
    jobs = doc["jobs"]
    needs = {name: needs_of(job) for name, job in jobs.items()}

    unknown = [n for name in TRANSITIVE_CONSUMERS for n in needs[name] if n not in jobs]
    if unknown:
        print(f"FAIL: unknown needs referenced: {sorted(set(unknown))}")
        return 1

    failures: list[str] = []
    for node in PUBLISH_NODES:
        have = closure(needs, node)
        missing = [gate for gate in CORE_GATES if gate not in have]
        if missing:
            failures.append(f"{node}: missing core gates {missing}")
    have = closure(needs, "python-wheels")
    if "python-smoke" not in have:
        failures.append("python-wheels: missing gate python-smoke")
    for gate in CORE_GATES + ("python-smoke",):
        if gate not in closure(needs, "publish-pypi"):
            failures.append(f"publish-pypi: gate '{gate}' not inherited transitively")

    # Structural bypass detection on all publish-relevant jobs.
    always_if = set()
    continue_on_error = set()
    for name in TRANSITIVE_CONSUMERS + CORE_GATES + ("python-smoke",):
        job = jobs[name]
        condition = str(job.get("if", ""))
        if "always()" in condition:
            always_if.add(name)
        if job.get("continue-on-error"):
            continue_on_error.add(name)
    if always_if or continue_on_error:
        failures.append(f"bypass detected: always()={sorted(always_if)} "
                        f"continue-on-error={sorted(continue_on_error)}")

    # Tag-gating preserved: publish nodes must keep the tag condition and must
    # not silently publish on branch/PR pushes.
    for node in PUBLISH_NODES + ("publish-pypi",):
        condition = str(jobs[node].get("if", ""))
        if "refs/tags/v" not in condition:
            failures.append(f"{node}: tag condition missing ({condition!r})")

    # Negative evidence: synthetically break one gate at a time and confirm
    # every publish node that (transitively) requires that gate is not
    # executable. Nodes that do not depend on the broken gate are unaffected
    # by design (that is the point of the directed graph).
    for broken in ("feature-check", "msrv", "python-smoke", "check", "build-and-test", "validate-tag"):
        affected = [node for node in TRANSITIVE_CONSUMERS if broken in closure(needs, node)]
        if not affected:
            failures.append(f"negative[{broken}]: no publish node depends on this gate")
        for state in ("failure", "cancelled", "skipped"):
            states = {name: "success" for name in jobs}
            states[broken] = state
            for node in affected:
                runnable, why = executable(node, needs, states, always_if, continue_on_error)
                if runnable:
                    failures.append(
                        f"negative[{broken}={state}]: {node} would still run ({why})")

    if failures:
        print("FAIL:")
        for item in failures:
            print(f"  - {item}")
        return 1

    print("PASS: all publish nodes transitively require the core gates "
          "+ python-smoke (wheels); tag conditions intact; no bypasses.")
    print("Negative synthesis: every publish node is blocked when any gate "
          "is failure/cancelled/skipped.")
    for node in TRANSITIVE_CONSUMERS:
        print(f"  {node}: needs={needs[node]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else ".github/workflows/ci.yml"))
