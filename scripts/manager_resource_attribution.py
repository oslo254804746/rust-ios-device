#!/usr/bin/env python3
"""Attribute a recorded default-heap diagnostic without rerunning its workload.

This diagnostic is separate from the before/after throughput comparison. It
does not compile, control a device, walk foreign heaps, or certify leak absence.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path

from manager_resource_baseline import receipt_requirements


LIMIT = 4 * 1024 * 1024


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(65536), b""):
            value.update(chunk)
    return value.hexdigest()


def read_json(path):
    with path.open("rb") as stream:
        data = stream.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError("diagnostic receipt exceeds 4 MiB")
    return json.loads(data)


def series(values):
    if not values or any(type(value) is not int or value < 0 for value in values):
        raise ValueError("incomplete unsigned diagnostic counter")
    return {
        "samples": len(values), "first": values[0], "last": values[-1],
        "delta": values[-1] - values[0], "minimum": min(values),
        "maximum": max(values), "range": max(values) - min(values),
        "upward_steps": sum(b > a for a, b in zip(values, values[1:])),
        "downward_steps": sum(b < a for a, b in zip(values, values[1:])),
    }


def correlation(first, second):
    if len(first) != len(second) or len(first) < 2:
        return None
    average_a, average_b = sum(first) / len(first), sum(second) / len(second)
    a = [value - average_a for value in first]
    b = [value - average_b for value in second]
    denominator = math.sqrt(sum(value * value for value in a) * sum(value * value for value in b))
    return sum(x * y for x, y in zip(a, b)) / denominator if denominator else None


def analyze(directory):
    directory = Path(directory)
    run = read_json(directory / "run.json")
    fixture_path = directory / "fixture.json"
    if digest(fixture_path) != run["fixture_json_sha256"]:
        raise ValueError("fixture hash differs from original run")
    binary = Path(run["test_binary"])
    if not (run["test_binary_sha256"] == run["test_binary_sha256_after"] == digest(binary)):
        raise ValueError("original diagnostic executable changed")
    receipt = read_json(fixture_path)
    receipt_requirements(receipt)
    rows = [row for row in receipt["resource_samples"]
            if row["scenario"] == "longrun" and row["phase"] == "released"]
    if len(rows) != receipt["longrun"]["cycles"]:
        raise ValueError("missing released-phase observation")
    counters = {key: [] for key in (
        "busy_bytes", "region_commit_bytes", "private_commit_bytes",
        "private_working_set_bytes", "non_private_working_set_bytes",
        "process_handle_count", "tcp_endpoint_count", "tokio_alive_tasks",
    )}
    for row in rows:
        process = row["process"]
        heap = process["default_heap"]
        if not (heap.get("complete") is True and heap.get("locked") is True
                and heap.get("unlocked") is True and heap.get("bound_reached") is False
                and heap.get("terminal_win32_code") == 259):
            raise ValueError("incomplete original default-heap walk")
        registry = row["native_registry"]
        if not (registry["occupied"] == 0 and row["manager_records"] == 0
                and row["connect_permits"] == row["connect_capacity"]
                and row["failure_permits"] == row["failure_capacity"]):
            raise ValueError("released native owners or permits did not recover")
        for key in counters:
            source = heap if key in ("busy_bytes", "region_commit_bytes") else (
                row if key == "tokio_alive_tasks" else process)
            counters[key].append(source[key])
    stable = all(len(set(counters[key])) == 1 for key in (
        "process_handle_count", "tcp_endpoint_count", "tokio_alive_tasks"))
    return {
        "schema_version": 1, "status": "observed", "production_source_commit": run["production_source_commit"],
        "fixture_sha256": run["fixture_json_sha256"], "test_binary_sha256": run["test_binary_sha256"],
        "run_sha256": digest(directory / "run.json"), "input": receipt["input"],
        "longrun": receipt["longrun"], "released_counters": {key: series(values) for key, values in counters.items()},
        "released_handle_endpoint_task_counts_stable": stable,
        "all_original_cleanup_certificates": receipt["cleanup"],
        "region_commit_private_commit_correlation": correlation(
            counters["region_commit_bytes"], counters["private_commit_bytes"]),
        "inference": "default-heap capacity variation is consistent with the observed private-commit variation; this is an inference, not causal proof or a universal leak certificate",
        "limitations": [
            "default heap excludes other heaps, stacks and mappings",
            "busy bytes include HeapWalk block overhead; LFH internal free space is not individually reported",
            "heap walk occurs only after release, separately from the throughput comparison",
            "process counters include fixture and manager allocations",
            "run metadata records executable and pre-run fixture hashes; it does not establish a whole-workspace source freeze",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    arguments = parser.parse_args()
    report = analyze(arguments.directory)
    encoded = json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n"
    if len(encoded.encode("utf-8")) > LIMIT:
        raise ValueError("attribution report exceeds 4 MiB")
    arguments.output.write_text(encoded, encoding="utf-8")
    print(f"observed: {report['longrun']['cycles']} released rounds; output={arguments.output}")


if __name__ == "__main__":
    main()
