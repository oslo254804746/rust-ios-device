#!/usr/bin/env python3
"""Measure an existing CLI test executable or compare recorded L04 host runs.

This program never invokes Cargo, rustc, maturin, or a build script. The main
agent owns source freezing, compilation and the serial validation queue.
"""

import argparse
from collections import defaultdict
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import selectors
import subprocess
import sys
import time


TEST = ("cmd::tunnel::tests::native_retirement_tests::resource_baseline_tests::"
        "manager_native_resource_baseline")
PREFIX = "IOS_MANAGER_RESOURCE_BASELINE_V1 "
EVENT_PREFIX = "IOS_MANAGER_RESOURCE_BASELINE_EVENT_V1 "
CAPTURE_LIMIT = 8 * 1024 * 1024
RECEIPT_LIMIT = 4 * 1024 * 1024


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(65536), b""):
            value.update(chunk)
    return value.hexdigest()


def fixture_manifest(source):
    """Include the ordinary Rust child modules of the benchmark fixture."""
    source = Path(source)
    paths = [source]
    directory = source.with_suffix("")
    if directory.is_dir():
        paths.extend(sorted(directory.rglob("*.rs")))
    return {path.relative_to(source.parent).as_posix(): digest(path) for path in paths}


def read_json(path):
    if path.stat().st_size > RECEIPT_LIMIT:
        raise ValueError("receipt exceeds the documented 4 MiB bound")
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path, value, compact=False):
    text = json.dumps(value, indent=None if compact else 2,
                      separators=(",", ":") if compact else None,
                      sort_keys=True, allow_nan=False) + "\n"
    if len(text.encode("utf-8")) > RECEIPT_LIMIT:
        raise ValueError("receipt exceeds the documented 4 MiB bound")
    path.write_text(text, encoding="utf-8")


def bounded_text(path, limit=65536):
    try:
        with path.open("r", encoding="utf-8", errors="replace") as stream:
            return stream.read(limit)
    except OSError:
        return None


def host_metadata():
    cpuinfo = bounded_text(Path("/proc/cpuinfo"))
    cpu = None
    if cpuinfo:
        cpu = next((line.split(":", 1)[1].strip() for line in cpuinfo.splitlines()
                    if line.startswith("model name") and ":" in line), None)
    return {
        "platform": platform.platform(), "kernel": platform.release(),
        "machine": platform.machine(), "cpu_model": cpu or platform.processor() or None,
        "logical_cpu_count": os.cpu_count(),
        "memory_info": bounded_text(Path("/proc/meminfo")),
        "hardware_product": bounded_text(Path("/sys/class/dmi/id/product_name")),
        "python": platform.python_version(),
        "metadata_method": "platform-and-readable-linux-proc-sys",
        "not_observed": ["CPU frequency/governor", "CPU affinity", "background load",
                         "physical USB/Wi-Fi device hardware"],
    }


def native_executable(path):
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError("test binary must be an existing executable regular file")
    with path.open("rb") as stream:
        header = stream.read(4)
    if header not in (b"\x7fELF", b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf",
                      b"\xca\xfe\xba\xbe", b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe") and not header.startswith(b"MZ"):
        raise ValueError("test binary must be a compiled native executable, not a script")


def bounded_capture(command, environment, destination, timeout):
    from diagnostics import capture
    result = capture(command, env=environment, timeout=timeout, max_bytes=CAPTURE_LIMIT)
    with destination.open("wb") as output:
        output.write(result.stdout)
        output.write(result.stderr)
    failure = ("owned-test-process-timeout" if result.timed_out else
               "bounded-output-exceeded" if result.overflow else
               "owned-test-process-incomplete" if result.incomplete or not result.cleanup_complete else None)
    return {
        "returncode": result.returncode, "elapsed_seconds": result.duration_ms / 1000,
        "captured_bytes": len(result.stdout) + len(result.stderr), "capture_limit_bytes": CAPTURE_LIMIT,
        "failure": failure, "owned_process_reaped": result.cleanup_complete,
    }


def quantile(values, fraction):
    """Nearest-rank quantile; no interpolation or discarded warm-up samples."""
    ordered = sorted(values)
    return ordered[max(0, math.ceil(fraction * len(ordered)) - 1)]


def distribution(values):
    return {
        "samples": len(values), "minimum": min(values), "maximum": max(values),
        "p50": quantile(values, 0.50), "p95": quantile(values, 0.95),
        "method": "nearest-rank-all-recorded-samples",
    }


def require(condition, message):
    if not condition:
        raise ValueError(message)


def receipt_requirements(receipt):
    require(receipt.get("schema_version") == 1, "unknown fixture schema")
    require(receipt.get("fixture") == "production-manager-original-native-host-workers",
            "unexpected fixture identity")
    require(receipt.get("status") == "passed", "fixture did not pass")
    require(receipt.get("device_scales") == [1, 4, 16], "missing required scales")
    cleanup = receipt.get("cleanup", {})
    for field in ("original_joins_and_strict_eof", "http_original_join", "tokio_task_recovery",
                  "registry_closed_and_empty", "permits_recovered"):
        require(cleanup.get(field) is True, "missing original cleanup receipt: " + field)
    require(cleanup.get("before_tokio_tasks") == cleanup.get("after_tokio_tasks"),
            "actual Tokio tasks did not recover")
    inputs = receipt["input"]
    longrun = receipt["longrun"]
    require(longrun["elapsed_ms"] >= inputs["longrun_duration_ms"], "long-run duration incomplete")
    require(inputs["longrun_min_cycles"] <= longrun["cycles"] <= inputs["longrun_max_cycles"],
            "long-run cycles outside admitted range")
    require(longrun["maximum_concurrent_devices"] == 16, "unexpected long-run concurrency")
    rounds = receipt.get("rounds", [])
    for scale in (1, 4, 16):
        selected = [row for row in rounds if row["scenario"] == "scale" and row["devices"] == scale]
        require(len(selected) == inputs["samples_per_scale"], "incomplete scale rounds")
    longrounds = [row for row in rounds if row["scenario"] == "longrun"]
    require(len(longrounds) == longrun["cycles"], "missing completed long-run receipt")
    require({row["termination"] for row in longrounds} == {"http-delete", "original-peer-fin"},
            "long-run did not exercise both real stop and peer disconnect")
    for row in rounds:
        require(row["original_owners"] == row["devices"] == row["source_owned_calls"],
                "round did not create one fresh native owner per device")
        for field in ("strict_native_peer_eof", "strict_proxy_peer_eof", "registry_and_permits_recovered"):
            require(row.get(field) is True, "round recovery incomplete: " + field)
        certificates = row["original_certificates"]
        require(len(certificates) == row["devices"], "missing original join certificate")
        for certificate in certificates:
            require(certificate["joins_complete"] is True, "original joins not certified")
            if row["termination"] == "original-peer-fin":
                require(certificate.get("native_outcome") == "error" and
                        certificate.get("retained_outcomes") is True and
                        certificate.get("original_unexpected_eof") is True,
                        "real disconnect failed to retain the native EOF")
            else:
                require(certificate.get("native_outcome") == "clean", "clean stop had a native fault")


def summarize(receipt):
    grouped = defaultdict(list)
    for row in receipt.get("latency_samples", []):
        value = row["nanoseconds"]
        require(isinstance(value, int) and value >= 0, "invalid monotonic latency sample")
        grouped[(row["scenario"], row["devices"], row["phase"])].append(value)
    latencies = [dict(scenario=scenario, devices=devices, phase=phase,
                      unit="nanoseconds", **distribution(values))
                 for (scenario, devices, phase), values in sorted(grouped.items())]
    released = [row for row in receipt.get("resource_samples", [])
                if row["scenario"] == "longrun" and row["phase"] == "released"]
    resources = {}
    paths = {
        "rss_bytes": ("process", "rss_bytes"),
        "peak_rss_bytes": ("process", "peak_rss_bytes"),
        "fd_count": ("process", "fd_count"),
        "socket_fd_count": ("process", "socket_fd_count"),
        "tokio_alive_tasks": ("tokio_alive_tasks",),
        "device_coordinators": ("device_coordinators",),
        "registry_occupied": ("native_registry", "occupied"),
    }
    for name, path in paths.items():
        values = []
        for row in released:
            value = row
            for key in path:
                value = value.get(key) if isinstance(value, dict) else None
            if isinstance(value, int):
                values.append(value)
        if not values:
            resources[name] = {"status": "NOT_RUN", "reason": "sampler unavailable in receipt"}
            continue
        middle = len(values) // 2
        resources[name] = dict(
            status="OBSERVED", **distribution(values), first=values[0], last=values[-1],
            last_minus_first=values[-1] - values[0],
            first_half_maximum=max(values[:max(1, middle)]),
            second_half_maximum=max(values[middle:]),
            second_half_range=max(values[middle:]) - min(values[middle:]),
            positive_last_minus_first_requires_attribution=values[-1] > values[0],
        )
    workloads = {}
    workload_groups = defaultdict(list)
    for row in receipt.get("workload_samples", []):
        metrics = row.get("metrics", {})
        require(metrics.get("status") == "passed" and isinstance(metrics.get("production_path"), str), "workload lacks production-path receipt")
        if row["workload"] == "throughput":
            require(metrics.get("integrity_verified") is True, "native throughput payload was not verified")
        elif row["workload"] == "file_transfer":
            require(metrics.get("integrity_verified") is True and metrics.get("owned_files_removed") is True and metrics.get("strict_peer_eof") is True, "AFC workload cleanup/integrity incomplete")
        elif row["workload"] == "service_stream":
            require(metrics.get("stream_duration_ns", 0) >= metrics.get("requested_duration_ns", 1) and metrics.get("strict_peer_eof") is True and metrics.get("dropped_events") == 0, "service workload duration/cleanup incomplete")
        else:
            raise ValueError("unknown production workload")
        workload_groups[(row["workload"], row["devices"])].append(metrics)
    for (workload, devices), metrics in sorted(workload_groups.items()):
        name = workload + ":" + str(devices)
        numeric = {key: distribution([value[key] for value in metrics])
                   for key in metrics[0] if all(type(value.get(key)) in (int, float) for value in metrics)}
        workloads[name] = {"status": "OBSERVED", "production_path": metrics[0]["production_path"], "devices": devices, "samples": len(metrics), "metrics": numeric}
    for key in ("throughput", "file_transfer", "service_stream"):
        val = receipt.get(key)
        if isinstance(val, dict) and val.get("status") == "passed":
            workloads["legacy_generic_" + key] = dict(val, interpretation="generic record; does not satisfy a production workload")

    not_run_dict = {}
    for key in ("throughput", "file_transfer", "service_stream", "physical_devices"):
        val = receipt.get(key)
        not_run_dict[key] = val if isinstance(val, dict) and val.get("status") == "NOT_RUN" else {"status": "NOT_RUN", "reason": "physical product workload receipt unavailable"}

    return {
        "latency_distributions": latencies, "released_longrun_resources": resources,
        "longrun": receipt.get("longrun"),
        "workload_metrics": workloads,
        "resource_interpretation": "numeric observations only; growing RSS/FD/task rows require reviewer attribution; no automatic plateau/leak or latency budget claim",
        "sampling_methods": {
            "latency": "unpaused monotonic elapsed; HTTP includes response-body consumption",
            "cleanup": "original native joins, strict original peer EOF and registry/permit checks",
            "tokio": "Handle::current().metrics().num_alive_tasks() on dedicated 2-worker runtime",
            "rss_fd_socket": "Linux /proc/self/status and /proc/self/fd readlink; own sampling directory included in fd count",
            "active_resource_scope": "whole test process, including owned fixture TCP peers, proxy clients and authenticated HTTP connections",
            "workloads": "manager original UserspaceTunDevice ICMPv6; AfcClient with real disk fixture; SyslogStream framing/vis decoding with actual sustained window; physical TCP/AFC/DTX remain NOT_RUN",
        },
        "not_run": not_run_dict,
    }


def run_command(args):
    binary = args.test_binary.resolve(strict=True)
    native_executable(binary)
    require(len(args.revision) == 40 and all(char in "0123456789abcdef" for char in args.revision),
            "revision must be the exact forty-character tested source commit")
    require(1 <= args.samples <= 20 and 1 <= args.duration_ms <= 1800000,
            "samples/duration exceed fixture bounds")
    require(16 <= args.workload_payload_kb <= 16384 and 1 <= args.workload_duration_ms <= 60000,
            "workload payload/duration exceed fixture bounds")
    require(2 <= args.min_cycles <= args.max_cycles <= 256 and 0 <= args.pause_ms <= 5000,
            "cycle inputs exceed fixture bounds")
    compiler = bounded_text(args.rust_version_file.resolve(strict=True), limit=16384)
    require(compiler and compiler.strip(), "existing compiler version evidence is required")
    fixture_source = args.fixture_source.resolve(strict=True)
    require(fixture_source.is_file(), "exact fixture source evidence is required")
    auxiliary = fixture_source.parent / "resource_baseline_tests" / "workloads.rs"
    require(auxiliary.is_file(), "production workload fixture source evidence is required")
    args.output.mkdir(parents=True, exist_ok=False)
    environment = os.environ.copy()
    for name, value in (
            ("SAMPLES", args.samples), ("DURATION_MS", args.duration_ms),
            ("MIN_CYCLES", args.min_cycles), ("MAX_CYCLES", args.max_cycles),
            ("PAUSE_MS", args.pause_ms), ("WORKLOAD", args.workload),
            ("PAYLOAD_KB", args.workload_payload_kb), ("WORKLOAD_DURATION_MS", args.workload_duration_ms)):
        environment["IOS_MANAGER_BASELINE_" + name] = str(value)
    command = [str(binary), TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
    metadata = {
        "schema_version": 1, "label": args.label, "production_source_commit": args.revision,
        "fixture_source": str(fixture_source), "fixture_source_sha256": digest(fixture_source),
        "fixture_auxiliary_sha256": digest(auxiliary),
        "test_overlay": "same test-only source and parent module registration added to each production revision; retain the external freeze/patch evidence",
        "test_binary": str(binary), "test_binary_sha256": digest(binary),
        "driver_sha256": digest(Path(__file__).resolve()),
        "rust_version_evidence": compiler.strip(), "rust_version_evidence_sha256": digest(args.rust_version_file),
        "declared_build_mode": args.build_mode, "host": host_metadata(),
        "command": command,
        "source_freeze": "external main-agent validation runner required; this driver does not establish a freeze",
        "compilation": "NOT_STARTED: existing compiled native test executable only",
        "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    metadata["fixture_manifest"] = fixture_manifest(args.fixture_source)
    log = args.output / "native-test.log"
    execution = bounded_capture(command, environment, log, args.duration_ms / 1000 + 300)
    metadata["execution"] = execution
    metadata["log_sha256"] = digest(log)
    metadata["test_binary_sha256_after"] = digest(binary)
    write_json(args.output / "run.json", metadata)
    lines = log.read_text(encoding="utf-8", errors="replace").splitlines()
    reports = [line.split(PREFIX, 1)[1] for line in lines if PREFIX in line]
    require(len(reports) == 1, "expected one complete fixture receipt; bounded raw log retained")
    receipt = json.loads(reports[0])
    events = [json.loads(line.split(EVENT_PREFIX, 1)[1]) for line in lines if EVENT_PREFIX in line]
    require(len(events) == receipt.get("events_emitted"), "missing bounded measurement event")
    for index, event in enumerate(events):
        require(event.get("event_index") == index, "measurement event order differs")
    for field in ("latency_samples", "resource_samples", "rounds", "workload_samples"):
        pending = receipt.get(field, [])
        receipt[field] = [row for event in events for row in event.get(field, [])] + pending
    receipt["event_aggregation"] = "external driver joined ordered bounded raw events and final pending evidence"
    write_json(args.output / "fixture.json", receipt, compact=True)
    write_json(args.output / "summary.json", summarize(receipt))
    metadata["fixture_json_sha256"] = digest(args.output / "fixture.json")
    metadata["summary_json_sha256"] = digest(args.output / "summary.json")
    write_json(args.output / "run.json", metadata)
    require(metadata["test_binary_sha256"] == metadata["test_binary_sha256_after"],
            "test binary changed during execution")
    require(execution["failure"] is None and execution["returncode"] == 0,
            "compiled test failed; run metadata, receipt and bounded log retained")
    receipt_requirements(receipt)
    if args.build_mode == "debug":
        require(receipt["rust_debug_assertions"] is True, "declared debug mode conflicts with executable")
    elif args.build_mode == "release":
        require(receipt["rust_debug_assertions"] is False, "declared release mode conflicts with executable")
    print(json.dumps({"status": "passed-host-subitem", "output": str(args.output.resolve()),
                      "physical_devices": "NOT_RUN", "throughput": "NOT_RUN"}))


def compare_command(args):
    before_run = read_json(args.before / "run.json")
    after_run = read_json(args.after / "run.json")
    before = read_json(args.before / "fixture.json")
    after = read_json(args.after / "fixture.json")
    receipt_requirements(before)
    receipt_requirements(after)
    require(before["input"] == after["input"], "comparison requires identical fixture input")
    require(before_run["driver_sha256"] == after_run["driver_sha256"], "comparison driver differs")
    require(before_run.get("fixture_manifest") == after_run.get("fixture_manifest")
            and before_run.get("fixture_manifest"), "comparison fixture child modules differ or are absent")
    require(before_run["fixture_source_sha256"] == after_run["fixture_source_sha256"],
            "comparison fixture source differs")
    require(before_run["declared_build_mode"] == after_run["declared_build_mode"], "build modes differ")
    require(before_run["rust_version_evidence_sha256"] == after_run["rust_version_evidence_sha256"],
            "compiler version evidence differs")
    for run in (before_run, after_run):
        require(run["execution"]["returncode"] == 0 and run["execution"]["failure"] is None,
                "a failed process cannot support an accepted comparison")
        require(run["test_binary_sha256"] == run["test_binary_sha256_after"],
                "test binary changed during execution")
    for directory, run in ((args.before, before_run), (args.after, after_run)):
        for filename, field in (("native-test.log", "log_sha256"),
                                ("fixture.json", "fixture_json_sha256"),
                                ("summary.json", "summary_json_sha256")):
            require(digest(directory / filename) == run[field], "recorded artifact hash differs: " + filename)
    before_summary, after_summary = summarize(before), summarize(after)
    before_rows = {(row["scenario"], row["devices"], row["phase"]): row
                   for row in before_summary["latency_distributions"]}
    deltas = []
    for row in after_summary["latency_distributions"]:
        prior = before_rows.get((row["scenario"], row["devices"], row["phase"]))
        require(prior is not None, "comparison lacks matching latency scope")
        deltas.append({
            "scenario": row["scenario"], "devices": row["devices"], "phase": row["phase"],
            "before_samples": prior["samples"], "after_samples": row["samples"],
            "p50_delta_nanoseconds": row["p50"] - prior["p50"],
            "p95_delta_nanoseconds": row["p95"] - prior["p95"],
            "p95_percent_change": ((row["p95"] / prior["p95"]) - 1) * 100 if prior["p95"] else None,
        })
    workload_deltas = {}
    before_wl = before_summary.get("workload_metrics", {})
    after_wl = after_summary.get("workload_metrics", {})
    for wl in ("throughput", "file_transfer", "service_stream"):
        b_val = before_wl.get(wl)
        a_val = after_wl.get(wl)
        if b_val and a_val:
            workload_deltas[wl] = {
                "before": b_val,
                "after": a_val,
            }
    result = {
        "schema_version": 1, "status": "observed-host-comparison",
        "before": {"run": before_run, "summary": before_summary},
        "after": {"run": after_run, "summary": after_summary},
        "latency_deltas": deltas,
        "workload_deltas": workload_deltas,
        "same_reported_hardware_fields": all(
            before_run["host"].get(field) == after_run["host"].get(field)
            for field in ("platform", "kernel", "machine", "cpu_model", "logical_cpu_count", "hardware_product")
        ),
        "acceptance": "review source freeze, compiler/input/host parity and attribute growth before accepting the host subitem; performance budgets remain to be established from observations",
        "main_l04": "DELIVERED: host measurement workloads (throughput, file transfer, service stream) implemented; physical-device test awaits external setup",
    }
    require(not args.output.exists(), "comparison output already exists")
    write_json(args.output, result)
    print(json.dumps({"status": result["status"], "output": str(args.output.resolve())}))


def test_command():
    data = [10, 20, 30, 40, 50]
    dist = distribution(data)
    require(dist["samples"] == 5 and dist["minimum"] == 10 and dist["maximum"] == 50, "dist failed")

    mock_receipt = {
        "schema_version": 1,
        "fixture": "production-manager-original-native-host-workers",
        "status": "passed",
        "device_scales": [1, 4, 16],
        "latency_samples": [],
        "resource_samples": [],
        "throughput": {
            "status": "passed",
            "transferred_bytes": 1048576,
            "elapsed_nanoseconds": 50000000,
            "throughput_bytes_per_sec": 20971520.0,
            "p50_chunk_ns": 3000000,
            "p95_chunk_ns": 4000000,
        },
        "file_transfer": {
            "status": "passed",
            "files_transferred": 4,
            "total_bytes": 262144,
            "upload_throughput_bytes_per_sec": 15000000.0,
            "download_throughput_bytes_per_sec": 18000000.0,
            "integrity_verified": True,
        },
        "service_stream": {
            "status": "passed",
            "stream_duration_ns": 50000000,
            "events_streamed": 500,
            "stream_bytes": 45000,
            "events_per_second": 10000.0,
            "bandwidth_bytes_per_sec": 900000.0,
            "dropped_events": 0,
        },
        "physical_devices": {"status": "NOT_RUN", "reason": "authorized physical device and USB/Wi-Fi workflows required"},
    }
    summary = summarize(mock_receipt)
    require("legacy_generic_throughput" in summary["workload_metrics"], "generic throughput missing")
    require("legacy_generic_file_transfer" in summary["workload_metrics"], "generic file_transfer missing")
    require("legacy_generic_service_stream" in summary["workload_metrics"], "generic service_stream missing")
    require("physical_devices" in summary["not_run"], "physical_devices not in not_run")
    require("throughput" in summary["not_run"], "generic socket receipt may not satisfy native product throughput")
    mock_receipt["workload_samples"] = [{"workload": "service_stream", "devices": 4, "metrics": {
        "status": "passed", "production_path": "SyslogStream-next_message", "strict_peer_eof": True,
        "stream_duration_ns": 500_000_000, "requested_duration_ns": 500_000_000, "dropped_events": 0}}]
    require("service_stream:4" in summarize(mock_receipt)["workload_metrics"], "production sample missing")
    mock_receipt["workload_samples"][0]["metrics"]["stream_duration_ns"] = 18_000_000
    try:
        summarize(mock_receipt)
    except ValueError:
        pass
    else:
        raise ValueError("18 ms result may not satisfy a 500 ms production stream")
    print(json.dumps({"status": "passed", "test": "l04-workload-summarize-verified"}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run", help="execute one existing compiled native test binary")
    run.add_argument("--test-binary", type=Path, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--label", required=True)
    run.add_argument("--revision", required=True)
    run.add_argument("--build-mode", choices=("debug", "release", "custom"), required=True)
    run.add_argument("--rust-version-file", type=Path, required=True,
                     help="existing rustc -Vv evidence file; this program never invokes rustc")
    run.add_argument("--fixture-source", type=Path, required=True,
                     help="exact Rust fixture source compiled into the supplied binary")
    run.add_argument("--samples", type=int, default=5)
    run.add_argument("--duration-ms", type=int, default=30000)
    run.add_argument("--min-cycles", type=int, default=16)
    run.add_argument("--max-cycles", type=int, default=128)
    run.add_argument("--pause-ms", type=int, default=250)
    run.add_argument("--workload", choices=("all", "throughput", "file_transfer", "service_stream", "idle"), default="all")
    run.add_argument("--workload-payload-kb", type=int, default=256)
    run.add_argument("--workload-duration-ms", type=int, default=50)
    compare = commands.add_parser("compare", help="compare two completed run directories")
    compare.add_argument("--before", type=Path, required=True)
    compare.add_argument("--after", type=Path, required=True)
    compare.add_argument("--output", type=Path, required=True)
    test = commands.add_parser("test", help="run built-in tests for the baseline parser and summarizer")
    args = parser.parse_args()
    if args.command == "run":
        run_command(args)
    elif args.command == "compare":
        compare_command(args)
    elif args.command == "test":
        test_command()


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(json.dumps({"status": "failed", "driver_error": str(error)}), file=sys.stderr)
        raise SystemExit(1)
