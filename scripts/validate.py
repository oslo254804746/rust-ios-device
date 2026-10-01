#!/usr/bin/env python3
"""Shared, fail-fast validation entry point (Python 3.9+, no shell required)."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def run(label, command, *, env=None, cwd=ROOT, log=None):
    print(f"[{label}] {' '.join(map(str, command))}", flush=True)
    start = time.monotonic()
    if log is None:
        result = subprocess.run(command, cwd=cwd, env=env)
    else:
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("w", encoding="utf-8") as output:
            result = subprocess.run(command, cwd=cwd, env=env, stdout=output,
                                    stderr=subprocess.STDOUT)
    if result.returncode:
        if log:
            print(log.read_text(encoding="utf-8"), file=sys.stderr)
        raise RuntimeError(f"{label}: exit {result.returncode}")
    print(f"[PASS] {label} ({time.monotonic() - start:.1f}s)"
          + (f"; warnings/output: {log.relative_to(ROOT)}" if log else ""), flush=True)


def metadata():
    return json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"], cwd=ROOT))


def python_env(interpreter):
    # The test executable embeds Python: use the selected runtime's library and
    # base prefix, rather than assuming that activating a venv configures these.
    info = json.loads(subprocess.check_output([interpreter, "-c",
        "import json,sys,sysconfig; print(json.dumps(dict("
        "executable=sys.executable,prefix=sys.base_prefix,"
        "libdir=sysconfig.get_config_var('LIBDIR'))))"], text=True))
    env = os.environ.copy()
    env["PYO3_PYTHON"] = info["executable"]
    env["PYTHONHOME"] = info["prefix"]
    if info["libdir"] and sys.platform != "win32":
        key = "DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"
        env[key] = str(info["libdir"]) + (os.pathsep + env[key] if env.get(key) else "")
    return env


def features():
    core = next(p for p in metadata()["packages"] if p["name"] == "ios-core")
    # Cargo expands implicit optional-dependency features as well as explicit
    # services, groups, aliases and transport features. New entries are automatic.
    cases = [None] + sorted(f for f in core["features"] if f != "default")
    for feature in cases:
        command = ["cargo", "check", "-p", "ios-core", "--all-targets", "--locked"]
        if feature:
            command += ["--no-default-features", "--features", feature]
        run(f"feature/{feature or 'default'}", command,
            log=ROOT / "target/validation/features" / f"{feature or 'default'}.log")
    for group in ("classic", "coredevice-files,coredevice-info", "developer", "management"):
        run(f"feature-tests/{group}", ["cargo", "test", "-p", "ios-core",
            "--no-default-features", "--features", group, "--locked"],
            log=ROOT / "target/validation/features" / f"test-{group}.log")


def docs():
    run("default doctests", ["cargo", "test", "-p", "ios-core", "--doc", "--locked"])
    run("full doctests", ["cargo", "test", "-p", "ios-core", "--doc",
                         "--all-features", "--locked"])
    # Compile the actual minimal Rust snippets, including rustdoc hidden lines.
    # The temporary downstream project has no service features and its own lock.
    sources = ["README.md", "README.zh-CN.md", "crates/ios-core/README.md",
               "crates/ios-cli/README.md", "crates/ios-ffi/README.md",
               "crates/ios-py/README.md", "docs/getting-started.md", "docs/python-binding.md"]
    with tempfile.TemporaryDirectory(prefix="ios-docs-") as directory:
        project = Path(directory)
        (project / "src/bin").mkdir(parents=True)
        core_path = json.dumps(str(ROOT / "crates/ios-core"))
        (project / "Cargo.toml").write_text(
            '[package]\nname="ios-doc-examples"\nversion="0.0.0"\nedition="2021"\n'
            '[dependencies]\nios-core={path=' + core_path + '}\n'
            'tokio={version="1",features=["macros","rt-multi-thread"]}\nanyhow="1"\n',
            encoding="utf-8")
        # Use the project's pinned graph; Cargo updates only the temporary root.
        (project / "Cargo.lock").write_bytes((ROOT / "Cargo.lock").read_bytes())
        count = 0
        for source in sources:
            content = (ROOT / source).read_text(encoding="utf-8")
            for snippet in re.findall(r"```python\n(.*?)```", content, re.S):
                compile(snippet, source, "exec")
            for snippet in re.findall(r"```rust[^\n]*\n(.*?)```", content, re.S):
                code = re.sub(r"(?m)^#(?: |$)", "", snippet)
                if not re.search(r"\bfn main\s*\(", code):
                    code += "\nfn main() {}\n"
                (project / f"src/bin/example{count}.rs").write_text(code, encoding="utf-8")
                count += 1
        if not count:
            raise RuntimeError("no Rust documentation examples found")
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(ROOT / "target")
        run(f"{count} downstream documentation examples", ["cargo", "check", "--offline",
            "--bins"], cwd=project, env=env)


def ffi(build_directory=None):
    if build_directory is None:
        run("FFI build", ["cargo", "build", "-p", "ios-ffi", "--locked"])
    target = build_directory or Path(metadata()["target_directory"]) / "debug"
    output = ROOT / "target/validation/ffi-smoke"
    output.parent.mkdir(parents=True, exist_ok=True)
    source = ROOT / "crates/ios-ffi/examples/smoke.c"
    include = ROOT / "crates/ios-ffi/include"
    example = ROOT / "crates/ios-ffi/examples/list_devices.c"
    if sys.platform == "win32":
        output = output.with_suffix(".exe")
        run("C header/link", [os.environ.get("CC", "cl"), "/nologo", "/std:c11", "/W4", "/WX",
            f"/I{include}", str(source), str(target / "ios_ffi.dll.lib"),
            f"/Fe:{output}", f"/Fo:{output.with_suffix('.obj')}"])
        run("C listing example", [os.environ.get("CC", "cl"), "/nologo", "/std:c11", "/W4",
            "/WX", "/c", f"/I{include}", str(example),
            f"/Fo:{output.with_name('list_devices.obj')}"])
    else:
        run("C header/link", [os.environ.get("CC", "cc"), "-std=c11", "-Wall", "-Wextra",
            "-Werror", "-I", str(include), str(source), "-L", str(target),
            "-lios_ffi", "-o", str(output)])
        run("C listing example", [os.environ.get("CC", "cc"), "-std=c11", "-Wall", "-Wextra",
            "-Werror", "-I", str(include), "-c", str(example), "-o",
            str(output.with_name("list_devices.o"))])
    env = os.environ.copy()
    key = "PATH" if sys.platform == "win32" else (
        "DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH")
    env[key] = str(target) + os.pathsep + env.get(key, "")
    run("C ABI and failure/ownership contract", [str(output)], env=env)
    from diagnostics import assess, capture, secret_variants
    report = assess(capture([str(output)], env=env),
                    secret_variants(b"synthetic-ffi-credential-UDID"), 0)
    if not report["passed"]:
        raise RuntimeError("C ABI raw diagnostic capture failed")
    print("[PASS] C ABI raw diagnostic capture; " + json.dumps(report), flush=True)


def python_package(interpreter):
    destination = ROOT / "target/validation/python-dist"
    destination.mkdir(parents=True, exist_ok=True)
    # A private directory prevents a stale release wheel from satisfying checks.
    with tempfile.TemporaryDirectory(dir=destination, prefix="run-") as directory:
        env = os.environ.copy()
        env["PYO3_PYTHON"] = python_env(interpreter)["PYO3_PYTHON"]
        run("abi3 wheel", ["uvx", "--from", "maturin>=1.5,<2", "maturin", "build",
            "--locked", "--out", directory], cwd=ROOT / "crates/ios-py", env=env)
        run("sdist", ["uvx", "--from", "maturin>=1.5,<2", "maturin", "sdist",
            "--out", directory], cwd=ROOT / "crates/ios-py")
        run("Python artifacts", [sys.executable, "scripts/artifacts.py", "python",
            "--directory", directory, "--python", interpreter, "--build-sdist"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scope", choices=["full", "quick", "fmt", "check", "lint",
        "rust-tests", "python-host", "msrv", "features", "docs", "ffi", "python-package",
        "tooling-tests", "capabilities", "diagnostics", "regression", "compatibility"])
    parser.add_argument("--package", choices=["ios-core", "ios-cli", "ios-ffi"],
                        help="package to test in quick mode")
    parser.add_argument("--python", default=os.environ.get("PYO3_PYTHON", sys.executable))
    args = parser.parse_args()
    if args.package and args.scope != "quick":
        parser.error("--package is only valid with quick")
    actions = {
        "fmt": lambda: run("fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        "check": lambda: run("check", ["cargo", "check", "--workspace", "--all-targets",
                                       "--all-features", "--locked"], env=python_env(args.python)),
        "lint": lambda: run("clippy", ["cargo", "clippy", "--workspace", "--all-targets",
                                      "--all-features", "--locked", "--", "-D", "warnings"],
                                      env=python_env(args.python)),
        "rust-tests": lambda: run("Rust tests", ["cargo", "test", "--workspace", "--exclude",
                                                 "ios-py", "--all-features", "--locked"]),
        "python-host": lambda: run("Python host tests", ["cargo", "test", "-p", "ios-py",
                "--no-default-features", "--locked"], env=python_env(args.python)),
        "msrv": lambda: run("MSRV", ["cargo", "+1.80", "check", "--workspace", "--exclude",
                                     "ios-py", "--locked"]),
        "features": features, "docs": docs, "ffi": ffi,
        "capabilities": lambda: run("capability inventory", [sys.executable,
            "scripts/capabilities.py", "--build-cli"]),
        "diagnostics": lambda: run("raw host diagnostic capture", [sys.executable,
            "scripts/diagnostics.py", "--build-cli"]),
        "regression": lambda: run("host workflow evidence and cleanup", [sys.executable,
            "scripts/regression.py"]),
        "compatibility": lambda: run("public CLI compatibility fixtures", [sys.executable,
            "scripts/compatibility.py"]),
        "tooling-tests": lambda: run("validation failure paths", [sys.executable, "-m",
            "unittest", "discover", "-s", "scripts", "-p", "test_*.py"]),
        "python-package": lambda: python_package(args.python),
        "quick": lambda: run("package tests", ["cargo", "test", "-p",
            args.package or "ios-core", "--all-features", "--locked"]),
    }
    scopes = ["fmt", "check", "lint", "rust-tests", "python-host", "msrv", "features",
              "docs", "ffi", "python-package", "tooling-tests", "capabilities", "diagnostics", "regression", "compatibility"] if args.scope == "full" else [args.scope]
    for scope in scopes:
        actions[scope]()
    print(f"[PASS] validation/{args.scope}", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"[FAIL] {error}", file=sys.stderr)
        sys.exit(1)
