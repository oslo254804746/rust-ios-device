#!/usr/bin/env python3
"""Build and verify release archives without publishing anything."""

import argparse
from email.parser import Parser
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import zipfile

from validate import ROOT, ffi, metadata, run


def version():
    return next(p["version"] for p in metadata()["packages"] if p["name"] == "ios-core")


def checksum(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def seal(path):
    path.with_name(path.name + ".sha256").write_text(
        f"{checksum(path)}  {path.name}\n", encoding="utf-8")


def verify_seal(path):
    expected = f"{checksum(path)}  {path.name}\n"
    if path.with_name(path.name + ".sha256").read_text(encoding="utf-8") != expected:
        raise RuntimeError(f"checksum mismatch: {path.name}")


def archive(path, files):
    if path.suffix == ".zip":
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as output:
            for file in files:
                output.write(file, file.name)
    else:
        with tarfile.open(path, "w:gz") as output:
            for file in files:
                output.add(file, arcname=file.name)
    seal(path)


def native(target, directory):
    current = version()
    run("release CLI/FFI", ["cargo", "build", "--release", "-p", "ios-cli", "-p",
        "ios-ffi", "--target", target, "--locked"])
    build = Path(metadata()["target_directory"]) / target / "release"
    windows = "windows" in target
    suffix = ".zip" if windows else ".tar.gz"
    cli = build / ("ios.exe" if windows else "ios")
    libraries = ["ios_ffi.dll", "ios_ffi.dll.lib", "ios_ffi.lib"] if windows else [
        "libios_ffi.dylib" if "apple" in target else "libios_ffi.so", "libios_ffi.a"]
    files = [build / name for name in libraries] + [ROOT / "crates/ios-ffi/include/ios_rs.h"]
    for file in [cli] + files:
        if not file.is_file() or not file.stat().st_size:
            raise RuntimeError(f"missing or empty required artifact: {file.name}")
    actual = subprocess.check_output([str(cli), "--version"], text=True).strip().split()[-1]
    if actual != current:
        raise RuntimeError(f"CLI version {actual} != workspace {current}")
    ffi(build)
    directory.mkdir(parents=True, exist_ok=True)
    names = []
    for kind, entries in [("ios", [cli]), ("ios-ffi", files)]:
        path = directory / f"{kind}-{current}-{target}{suffix}"
        archive(path, entries)
        verify_seal(path)
        names.append({"file": path.name, "sha256": checksum(path),
                      "members": [file.name for file in entries]})
    manifest = directory / f"native-{target}.json"
    manifest.write_text(json.dumps({"version": current, "target": target, "artifacts": names},
                                  indent=2) + "\n", encoding="utf-8")
    verify_native(directory, manifest)


def verify_native(directory, manifest):
    document = json.loads(manifest.read_text(encoding="utf-8"))
    current = version()
    target = document["target"]
    windows = "windows" in target
    suffix = ".zip" if windows else ".tar.gz"
    required = {f"ios-{current}-{target}{suffix}": ["ios.exe" if windows else "ios"],
                f"ios-ffi-{current}-{target}{suffix}": (
                    ["ios_ffi.dll", "ios_ffi.dll.lib", "ios_ffi.lib", "ios_rs.h"] if windows
                    else ["libios_ffi.dylib" if "apple" in target else "libios_ffi.so",
                          "libios_ffi.a", "ios_rs.h"])}
    if document["version"] != current or len(document["artifacts"]) != len(required):
        raise RuntimeError("native version/artifact count mismatch")
    if {item["file"] for item in document["artifacts"]} != set(required):
        raise RuntimeError("native artifact list mismatch")
    for item in document["artifacts"]:
        path = directory / item["file"]
        verify_seal(path)
        if checksum(path) != item["sha256"] or item["members"] != required[path.name]:
            raise RuntimeError(f"manifest mismatch: {path.name}")
        if windows:
            with zipfile.ZipFile(path) as content:
                members = content.namelist()
                sizes = [member.file_size for member in content.infolist()]
        else:
            with tarfile.open(path) as content:
                members = content.getnames()
                sizes = [member.size for member in content.getmembers()]
        if members != item["members"] or not all(sizes):
            raise RuntimeError(f"archive members mismatch or empty: {path.name}")
    print(f"[PASS] native manifest {target}", flush=True)


def python_artifacts(directory, interpreter, build_sdist=False):
    current = version()
    wheels = sorted(directory.glob("*.whl"))
    sources = sorted(directory.glob("*.tar.gz"))
    if len(wheels) != 1 or len(sources) > 1 or (build_sdist and len(sources) != 1):
        raise RuntimeError("expected exactly one wheel and, when requested, one sdist")
    wheel = wheels[0]
    with zipfile.ZipFile(wheel) as content:
        broken = content.testzip()
        names = content.namelist()
        if broken:
            raise RuntimeError(f"corrupt wheel member: {broken}")
        fields = Parser().parsestr(content.read(next(n for n in names if n.endswith(
            ".dist-info/METADATA"))).decode())
        tags = content.read(next(n for n in names if n.endswith(".dist-info/WHEEL"))).decode()
        if (fields["Version"] != current or fields["Name"] != "rust-ios-device-tunnel"
                or fields["Requires-Python"] != ">=3.9" or "Tag: cp39-abi3-" not in tags):
            raise RuntimeError("wheel metadata/version/abi3 floor mismatch")
    for source in sources:
        with tarfile.open(source) as content:
            members = content.getnames()
            package = next(n for n in members if n.endswith("/PKG-INFO"))
            info = Parser().parsestr(content.extractfile(package).read().decode())
            if info["Version"] != current or info["Name"] != fields["Name"]:
                raise RuntimeError("sdist/wheel metadata mismatch")
            for suffix in ("Cargo.lock", "crates/ios-core/Cargo.toml", "crates/ios-py/Cargo.toml"):
                if not any(n.endswith("/" + suffix) for n in members):
                    raise RuntimeError(f"sdist missing {suffix}")
    with tempfile.TemporaryDirectory(prefix="ios-wheel-") as temporary:
        venv = Path(temporary) / "venv"
        run("wheel venv", ["uv", "venv", "--python", interpreter, str(venv)])
        python = venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        run("wheel install", ["uv", "pip", "install", "--python", str(python), str(wheel)])
        run("installed Python API/error contract", [str(python), "-I", str(ROOT / "scripts/python-smoke.py")])
        if build_sdist:
            run("sdist build/install", ["uv", "pip", "install", "--python", str(python),
                "--reinstall", "--no-deps", str(sources[0])])
            run("installed sdist API", [str(python), "-I", str(ROOT / "scripts/python-smoke.py")])
    for path in wheels + sources:
        seal(path)
        verify_seal(path)
    print(f"[PASS] Python artifacts on {interpreter}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["native", "verify-native", "python", "tag"])
    parser.add_argument("--target")
    parser.add_argument("--directory", type=Path, default=ROOT / "dist")
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--build-sdist", action="store_true")
    parser.add_argument("--tag", default=os.environ.get("GITHUB_REF_NAME", ""))
    args = parser.parse_args()
    if args.kind == "native":
        if not args.target:
            parser.error("native requires --target")
        native(args.target, args.directory.resolve())
    elif args.kind == "verify-native":
        manifests = sorted(args.directory.glob("native-*.json"))
        if not manifests:
            raise RuntimeError("missing native manifests")
        for manifest in manifests:
            verify_native(args.directory, manifest)
    elif args.kind == "python":
        python_artifacts(args.directory.resolve(), args.python, args.build_sdist)
    elif args.tag != "v" + version():
        raise RuntimeError(f"tag must equal v{version()}")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError, KeyError, StopIteration) as error:
        print(f"[FAIL] {error}", file=sys.stderr)
        sys.exit(1)
