#!/usr/bin/env python3
"""Static gate check for the release/publish job graph in ci.yml.

Verifies that every node that produces externally visible artifacts
(release, publish-crates, python-wheels, python-sdist, publish-pypi)
transitively depends on the required quality gates, and that no status-check
condition or `continue-on-error` bypass lets a publish node run when a gate
failed, was cancelled, or was skipped.

Purely static: parses the workflow YAML, never triggers any workflow run.

Usage: python scripts/check-release-gates.py [.github/workflows/ci.yml]
"""

from __future__ import annotations

import re
import sys
from collections.abc import Mapping, Sequence
from typing import Any

import yaml

CORE_GATES = ("check", "feature-check", "msrv", "build-and-test", "validate-tag")
PUBLISH_NODES = ("release", "publish-crates", "python-sdist", "python-wheels")
# Nodes whose upstream must be transitively successful for them to run.
TRANSITIVE_CONSUMERS = PUBLISH_NODES + ("publish-pypi",)
REQUIRED_JOBS = frozenset(CORE_GATES + ("python-smoke",) + TRANSITIVE_CONSUMERS)
# GitHub's implicit success() guard is disabled by these status checks. A
# negated success() is equally unsafe because it explicitly admits failed or
# skipped needs. Match case-insensitively because expressions are user-authored.
STATUS_BYPASS_RE = re.compile(
    r"(?i)(?:\b(?:always|failure|cancelled)\s*\(|!\s*success\s*\()"
)
STATUS_FUNCTION_RE = re.compile(r"(?i)\b(?:always|failure|cancelled|success)\s*\(")
TAG_GUARD_RE = re.compile(
    r"(?i)^(?:success\(\)\s*&&\s*)?startsWith\(\s*github\.ref\s*,\s*"
    r"['\"]refs/tags/v['\"]\s*\)$"
)
# Keep the whitelist deliberately narrow. The current workflow needs no
# compound success() conditions; accepting arbitrary suffixes could let
# operator precedence turn `success() && false || true` into an unconditional
# publish step. The tag guard has its own exact success() && startsWith form.
SAFE_SUCCESS_RE = re.compile(r"(?i)^success\(\)$")


def needs_of(job: Mapping[str, Any]) -> list[str]:
    needs = job.get("needs", [])
    if isinstance(needs, str):
        return [needs]
    if needs is None:
        return []
    if not isinstance(needs, Sequence) or isinstance(needs, (bytes, bytearray)):
        raise TypeError(f"needs must be a string or sequence, got {type(needs).__name__}")
    if not all(isinstance(parent, str) for parent in needs):
        raise TypeError("every needs entry must be a job id string")
    return list(needs)


def expression_text(value: Any) -> str:
    """Normalize an Actions expression while preserving its actual logic."""
    condition = str(value).strip()
    if condition.startswith("${{") and condition.endswith("}}"):
        condition = condition[3:-2].strip()
    return condition


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


def cycles(needs: dict[str, list[str]]) -> list[list[str]]:
    """Return graph cycles so an invalid workflow cannot pass by truncation."""
    visiting: set[str] = set()
    visited: set[str] = set()
    found: list[list[str]] = []

    def visit(job: str, path: list[str]) -> None:
        if job in visiting:
            start = path.index(job)
            found.append(path[start:] + [job])
            return
        if job in visited:
            return
        visiting.add(job)
        for parent in needs.get(job, []):
            visit(parent, path + [parent])
        visiting.remove(job)
        visited.add(job)

    for job in needs:
        visit(job, [job])
    return found


def executable(root: str, needs: dict[str, list[str]], states: dict[str, str],
               unsafe_if: set[str], continue_on_error: set[str]) -> tuple[bool, str]:
    """Simulate GitHub Actions: a job runs only if every `needs` parent is
    `success` (skipped/failure/cancelled all block). Status-check functions
    such as `always()`/`failure()`/`cancelled()` would bypass that, which this
    checker flags instead of honoring."""
    for parent in sorted(closure(needs, root)):
        state = states.get(parent, "success")
        if parent in unsafe_if:
            return False, f"bypass: '{parent}' has a status-check if condition"
        if state != "success":
            return False, f"blocked: needs '{parent}' is {state}"
        if parent in continue_on_error:
            return False, f"bypass: '{parent}' uses continue-on-error"
    return True, "would run"


def main(path: str) -> int:
    try:
        with open(path, encoding="utf-8") as fh:
            doc = yaml.safe_load(fh)
        jobs = doc["jobs"]
        if not isinstance(jobs, Mapping):
            raise TypeError("jobs must be a mapping")
        missing_jobs = sorted(REQUIRED_JOBS - set(jobs))
        if missing_jobs:
            print(f"FAIL: required jobs missing: {missing_jobs}")
            return 1
        for name, job in jobs.items():
            if not isinstance(job, Mapping):
                raise TypeError(f"job '{name}' must be a mapping")
        needs = {name: needs_of(job) for name, job in jobs.items()}
    except (OSError, KeyError, TypeError, yaml.YAMLError) as exc:
        print(f"FAIL: cannot parse workflow {path}: {exc}")
        return 1

    unknown = [
        (name, parent)
        for name, parents in needs.items()
        for parent in parents
        if parent not in jobs
    ]
    if unknown:
        print(f"FAIL: unknown needs referenced: {unknown}")
        return 1

    graph_cycles = cycles(needs)
    if graph_cycles:
        print(f"FAIL: needs graph contains cycle(s): {graph_cycles}")
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

    # Structural bypass detection covers every ancestor, including a future
    # gate aggregator. Step-level bypasses matter too: a smoke/upload step
    # marked continue-on-error could otherwise make a failed check look green.
    relevant_jobs = set(TRANSITIVE_CONSUMERS)
    for node in TRANSITIVE_CONSUMERS:
        relevant_jobs.update(closure(needs, node))
    unsafe_if = set()
    continue_on_error = set()
    bypass_details: list[str] = []
    for name in sorted(relevant_jobs):
        job = jobs[name]
        condition = expression_text(job.get("if", ""))
        if STATUS_BYPASS_RE.search(condition):
            unsafe_if.add(name)
            bypass_details.append(f"{name}: status-check if condition")
        if name in PUBLISH_NODES + ("publish-pypi", "validate-tag"):
            if not TAG_GUARD_RE.fullmatch(condition):
                failures.append(
                    f"{name}: job condition must be the tag guard, got {condition!r}"
                )
        elif condition and not SAFE_SUCCESS_RE.fullmatch(condition):
            failures.append(
                f"{name}: unsupported job condition bypasses implicit success(), "
                f"got {condition!r}"
            )
        if job.get("continue-on-error"):
            continue_on_error.add(name)
            bypass_details.append(f"{name}: continue-on-error")
        steps = job.get("steps", [])
        if not isinstance(steps, Sequence) or isinstance(steps, (bytes, bytearray)):
            failures.append(f"{name}: steps must be a sequence")
            continue
        for index, step in enumerate(steps):
            if not isinstance(step, Mapping):
                failures.append(f"{name}.steps[{index}]: step must be a mapping")
                continue
            step_label = str(step.get("name", f"step[{index}]"))
            step_condition = expression_text(step.get("if", ""))
            if STATUS_FUNCTION_RE.search(step_condition) and not SAFE_SUCCESS_RE.fullmatch(
                step_condition
            ):
                failures.append(
                    f"{name}/{step_label}: unsupported status-check if condition "
                    f"{step_condition!r}"
                )
            if STATUS_BYPASS_RE.search(step_condition):
                bypass_details.append(f"{name}/{step_label}: status-check if condition")
            if step.get("continue-on-error"):
                bypass_details.append(f"{name}/{step_label}: continue-on-error")
    if bypass_details:
        failures.append(f"bypass detected: {bypass_details}")

    # Tag-gating preserved: publish nodes must keep the tag condition and must
    # not silently publish on branch/PR pushes.
    for node in PUBLISH_NODES + ("publish-pypi",):
        condition = expression_text(jobs[node].get("if", ""))
        if not TAG_GUARD_RE.fullmatch(condition):
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
                runnable, why = executable(node, needs, states, unsafe_if, continue_on_error)
                if runnable:
                    failures.append(
                        f"negative[{broken}={state}]: {node} would still run ({why})")

    if failures:
        print("FAIL:")
        for item in failures:
            print(f"  - {item}")
        return 1

    print("PASS: all publish nodes transitively require the core gates; "
          "wheel/PyPI paths require python-smoke; tag conditions intact; "
          "no bypasses.")
    print("Negative synthesis: every publish node is blocked when any gate "
          "is failure/cancelled/skipped.")
    for node in TRANSITIVE_CONSUMERS:
        print(f"  {node}: needs={needs[node]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else ".github/workflows/ci.yml"))
