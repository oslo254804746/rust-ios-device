#!/usr/bin/env python3
"""Run the existing host fixture with an explicitly compiled glibc probe.

Compiles nothing. Diagnostic timings are separate from performance evidence.
"""
import os
from pathlib import Path
import sys
import xml.etree.ElementTree as ET

import manager_resource_baseline as driver


def summarize(path):
    if path.stat().st_size > driver.RECEIPT_LIMIT:
        raise ValueError("allocator XML exceeds 4 MiB")
    root = ET.fromstring(path.read_text())
    rows = []
    for sample in root.findall("sample"):
        allocator = sample.find("malloc")
        if allocator is None or sample.find("error") is not None:
            raise ValueError("allocator snapshot failed")
        def size(tag, kind):
            element = allocator.find(f"{tag}[@type='{kind}']")
            if element is None and tag == "total" and kind == "fast":
                return 0  # glibc without fastbins omits this aggregate.
            if element is None:
                raise ValueError("missing aggregate allocator field")
            return int(element.attrib["size"])
        arena = size("system", "current")
        free = size("total", "fast") + size("total", "rest")
        mapped = size("total", "mmap")
        driver.require(all(int(sample.attrib[key]) >= 0 for key in
                           ("rss_kib", "anon_kib", "file_kib", "shmem_kib")),
                       "process RSS sampler unavailable")
        rows.append({
            "phase": sample.attrib["phase"],
            "seconds": float(sample.attrib["seconds"]),
            "rss_bytes": int(sample.attrib["rss_kib"]) * 1024,
            "anonymous_rss_bytes": int(sample.attrib["anon_kib"]) * 1024,
            "file_rss_bytes": int(sample.attrib["file_kib"]) * 1024,
            "shared_rss_bytes": int(sample.attrib["shmem_kib"]) * 1024,
            "trim_result": int(sample.attrib["trim_result"]),
            "arenas": len(allocator.findall("heap")),
            "arena_system_bytes": arena, "arena_free_bytes": free,
            "mmap_bytes": mapped,
            "non_free_estimate_bytes": arena - free + mapped,
        })
    phases = [row["phase"] for row in rows]
    driver.require(phases[0] == "startup" and phases[-2:] == ["before_trim", "after_trim"],
                   "missing complete probe lifecycle")
    periodic = [row for row in rows if row["phase"] == "periodic"]
    driver.require(len(periodic) >= 4, "insufficient allocator samples")
    windows = []
    for start in range(0, int(periodic[-1]["seconds"]) + 1, 10):
        selected = [row for row in periodic if start <= row["seconds"] < start + 10]
        if selected:
            windows.append({"start_seconds": start, "samples": len(selected),
                            **{key: {"minimum": min(row[key] for row in selected),
                                     "maximum": max(row[key] for row in selected)}
                               for key in ("rss_bytes", "anonymous_rss_bytes", "file_rss_bytes",
                                           "arena_free_bytes", "non_free_estimate_bytes")}})
    return {"samples": rows, "ten_second_windows": windows,
            "trim_rss_released_bytes": rows[-2]["rss_bytes"] - rows[-1]["rss_bytes"],
            "interpretation": "All glibc arenas; non-free estimate includes allocator overhead and tcache, not exact live Rust objects. Asynchronous samples include active rounds. One extra thread/FILE; no performance conclusion or zero-leak proof."}


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--fixture-source", type=Path, required=True)
    parser.add_argument("--rust-version-file", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--label", required=True)
    args = parser.parse_args()
    driver.require(sys.platform == "linux", "probe requires Linux/glibc")
    driver.require(not os.environ.get("LD_PRELOAD"), "existing LD_PRELOAD would confound diagnosis")
    probe = args.probe.resolve(strict=True)
    driver.require(probe.is_file(), "compiled probe is required")
    args.samples = 5
    args.duration_ms = 60000
    args.min_cycles = 16
    args.max_cycles = 256
    args.pause_ms = 250
    args.build_mode = "debug"
    args.output = args.output.resolve()
    source = Path(__file__).with_name("manager_allocator_probe.c")
    freeze = {str(p): driver.digest(p) for p in (probe, source, Path(__file__), Path(driver.__file__))}
    os.environ["LD_PRELOAD"] = str(probe)
    os.environ["IOS_MANAGER_ALLOCATOR_OUTPUT"] = str(args.output / "allocator.xml")
    os.environ["IOS_MANAGER_ALLOCATOR_EXECUTABLE"] = str(args.test_binary.resolve(strict=True))
    try:
        driver.run_command(args)
    finally:
        os.environ.pop("LD_PRELOAD", None)
        os.environ.pop("IOS_MANAGER_ALLOCATOR_OUTPUT", None)
        os.environ.pop("IOS_MANAGER_ALLOCATOR_EXECUTABLE", None)
    driver.require(all(driver.digest(Path(p)) == value for p, value in freeze.items()),
                   "probe/driver changed during diagnosis")
    result = summarize(args.output / "allocator.xml")
    result.update({"diagnostic_only": True, "probe_source_and_driver_sha256": freeze,
                   "allocator_xml_sha256": driver.digest(args.output / "allocator.xml")})
    driver.write_json(args.output / "allocator-summary.json", result)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, ET.ParseError) as error:
        print(f"allocator diagnosis failed: {error}", file=sys.stderr)
        raise SystemExit(1)
