#!/usr/bin/env python3
"""Regression tests for the release-gate topology checker.

These tests mutate the real workflow in temporary files. They do not execute
any Actions step or publish command.
"""

from __future__ import annotations

import copy
import importlib.util
import io
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

import yaml

sys.dont_write_bytecode = True


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
CHECKER_PATH = ROOT / "scripts" / "check-release-gates.py"


def load_checker():
    spec = importlib.util.spec_from_file_location("check_release_gates", CHECKER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {CHECKER_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CHECKER = load_checker()


class ReleaseGateCheckerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))

    def check_mutation(self, mutate) -> tuple[int, str]:
        document = copy.deepcopy(self.workflow)
        mutate(document)
        with tempfile.NamedTemporaryFile("w", suffix=".yml", delete=False, encoding="utf-8") as fh:
            yaml.safe_dump(document, fh, sort_keys=False)
            path = Path(fh.name)
        try:
            output = io.StringIO()
            with redirect_stdout(output):
                result = CHECKER.main(str(path))
            return result, output.getvalue()
        finally:
            path.unlink()

    def assert_rejected(self, mutate, message: str) -> None:
        result, output = self.check_mutation(mutate)
        self.assertNotEqual(result, 0, message + "\n" + output)

    def test_current_workflow_passes(self) -> None:
        self.assertEqual(CHECKER.main(str(WORKFLOW)), 0)

    def test_negated_success_condition_is_rejected(self) -> None:
        self.assert_rejected(
            lambda doc: doc["jobs"]["check"].update({"if": "${{ !success() }}"}),
            "!success() must not bypass a failed gate",
        )

    def test_step_continue_on_error_is_rejected(self) -> None:
        self.assert_rejected(
            lambda doc: doc["jobs"]["check"]["steps"][0].update(
                {"continue-on-error": True}
            ),
            "a gate step must not be allowed to fail while the job stays green",
        )

    def test_unsafe_condition_on_transitive_aggregator_is_rejected(self) -> None:
        def mutate(doc):
            doc["jobs"]["release-gates"] = {
                "needs": list(CHECKER.CORE_GATES),
                "if": "always()",
                "steps": [{"run": "true"}],
            }
            doc["jobs"]["release"]["needs"] = ["release-gates"]

        self.assert_rejected(mutate, "unsafe conditions on future aggregators must fail closed")

    def test_missing_python_smoke_is_rejected_without_crashing(self) -> None:
        self.assert_rejected(
            lambda doc: doc["jobs"].pop("python-smoke"),
            "python-smoke is required for wheel publication",
        )

    def test_success_or_tag_condition_is_rejected(self) -> None:
        self.assert_rejected(
            lambda doc: doc["jobs"]["publish-pypi"].update(
                {"if": "${{ success() || startsWith(github.ref, 'refs/tags/v') }}"}
            ),
            "success() || tag must not make publication conditional on a failed gate",
        )

    def test_compound_success_condition_is_rejected(self) -> None:
        self.assert_rejected(
            lambda doc: doc["jobs"]["release"]["steps"][-1].update(
                {"if": "${{ success() && false || true }}"}
            ),
            "compound status expressions must fail closed",
        )


if __name__ == "__main__":
    unittest.main()
