#!/usr/bin/env python3
"""Validate the authoritative capability inventory against source/public CLI."""
import argparse
from datetime import date
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "docs/capabilities.json"
RESULTS = {"PASS", "FAIL", "INCONCLUSIVE", "NOT_RUN"}


def source_inventory():
    commands = {}
    source = (ROOT / "crates/ios-cli/src/main.rs").read_text(encoding="utf-8")
    body = source.split("enum Commands {", 1)[1].split("\n}", 1)[0]
    for variant, module in re.findall(r"^    (\w+)\(cmd::(\w+)::\w+\),$", body, re.M):
        name = re.sub(r"(?<!^)(?=[A-Z])", "-", variant).lower()
        commands["cli:" + name] = f"crates/ios-cli/src/cmd/{module}.rs"
    services = {}
    source = (ROOT / "crates/ios-core/src/services/mod.rs").read_text(encoding="utf-8")
    for module in re.findall(r"(?m)^pub mod (\w+);$", source):
        path = ROOT / f"crates/ios-core/src/services/{module}"
        # A flat service entry may also own a same-name submodule directory.
        entry = path.with_suffix(".rs")
        if not entry.is_file():
            entry = path / "mod.rs"
        services["service:" + module] = entry.relative_to(ROOT).as_posix()
    if not commands or not services:
        raise ValueError("source inventory is empty; update parser for new declaration syntax")
    return commands, services


def validate(document, expected=None):
    if document["schema_version"] != 1:
        raise ValueError("unsupported capability schema")
    evidence = {item["id"]: item for item in document["evidence"]}
    if len(evidence) != len(document["evidence"]):
        raise ValueError("duplicate evidence ID")
    for item in evidence.values():
        for field in ("level", "host_os", "device", "ios", "transport", "required_state",
                      "tested_commit", "date", "location", "result", "scope"):
            if field not in item:
                raise ValueError(f"evidence {item['id']} missing {field}")
        date.fromisoformat(item["date"])
        if item["result"] not in RESULTS or not re.fullmatch(r"[0-9a-f]{40}", item["tested_commit"]):
            raise ValueError(f"invalid evidence outcome/commit: {item['id']}")
        if item["level"] == "real-device" and item["result"] == "PASS":
            if not item["device"] or not item["ios"] or not item["transport"]:
                raise ValueError(f"device PASS requires device/iOS/transport: {item['id']}")
        path = item["location"].split("#", 1)[0]
        if not (ROOT / path).is_file():
            raise ValueError(f"evidence location missing: {path}")
    records = {item["id"]: item for item in document["capabilities"]}
    if len(records) != len(document["capabilities"]):
        raise ValueError("duplicate capability ID")
    for item in records.values():
        for field in ("surface", "protocol_paths", "source", "implementation", "maturity",
                      "required_state", "compatibility_scope", "observations", "limitations"):
            if field not in item:
                raise ValueError(f"capability {item['id']} missing {field}")
        if item["maturity"] not in {"experimental", "candidate-stable", "stable"}:
            raise ValueError(f"invalid maturity: {item['id']}")
        if not (ROOT / item["source"]).is_file():
            raise ValueError(f"capability source missing: {item['id']}")
        for observation in item["observations"]:
            if observation["result"] not in RESULTS:
                raise ValueError(f"unknown result: {item['id']}")
            if observation["result"] == "NOT_RUN" and not observation.get("reason"):
                raise ValueError(f"NOT_RUN requires a reason: {item['id']}")
            if observation["result"] != "NOT_RUN" and not observation.get("evidence"):
                raise ValueError(f"measured outcome needs evidence: {item['id']}")
            if observation.get("evidence"):
                proof = evidence[observation["evidence"]]
                if proof["result"] != observation["result"] or proof["level"] != observation["level"]:
                    raise ValueError(f"evidence outcome/level mismatch: {item['id']}")
        if item["maturity"] == "stable":
            proven = any(o["level"] == "real-device" and o["result"] == "PASS"
                         for o in item["observations"])
            if not proven or not item["compatibility_scope"]:
                raise ValueError(f"stable needs real-device evidence and scope: {item['id']}")
    if expected is None:
        commands, services = source_inventory()
        expected = {**commands, **services}
    actual = {key for key in records if key.startswith(("cli:", "service:"))}
    if actual != set(expected):
        raise ValueError(f"inventory drift; missing={sorted(set(expected) - actual)}, "
                         f"obsolete={sorted(actual - set(expected))}")
    for key, path in expected.items():
        if records[key]["source"] != path:
            raise ValueError(f"inventory source drift: {key}")
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, help="also compare actual CLI --help commands")
    parser.add_argument("--build-cli", action="store_true", help="check real CLI via cargo run")
    args = parser.parse_args()
    records = validate(json.loads(CATALOG.read_text(encoding="utf-8")))
    if args.binary or args.build_cli:
        command = ([str(args.binary.resolve()), "--help"] if args.binary else
                   ["cargo", "run", "-p", "ios-cli", "--locked", "--", "--help"])
        output = subprocess.check_output(command, cwd=ROOT, text=True)
        section = output.split("Commands:\n", 1)[1].split("\nOptions:", 1)[0]
        actual = {"cli:" + name for name in re.findall(r"(?m)^  ([\w-]+)\s", section)
                  if name != "help"}
        expected = {key for key in records if key.startswith("cli:")}
        if actual != expected:
            raise ValueError(f"public CLI inventory drift: {actual ^ expected}")
    print(f"PASS: {len(records)} capability records; evidence is separate from implementation")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
