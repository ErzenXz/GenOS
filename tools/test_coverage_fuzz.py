#!/usr/bin/env python3
"""Build and run the host-only GenOS coverage fuzzer with a pinned runtime."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import time
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
FUZZ = ROOT / "tools/coverage-fuzz"
LOCK = FUZZ / "runtime-lock.json"
OUT = ROOT / "build/coverage-fuzz"
TARGETS = (
    "elf", "native-elf", "ipv4", "ipv6", "udp", "tcp", "socket",
    "boot-map", "paths", "descriptors",
)
SEEDS = {
    "elf": ("elf-valid.bin", "elf-overflow.bin", "elf-truncated.bin"),
    "native-elf": ("native-elf-valid.bin", "native-elf-late-wx.bin"),
    "ipv4": ("ipv4-udp-valid.bin", "ipv4-fragment.bin"),
    "ipv6": ("ipv6-udp-valid.bin", "ipv6-options-truncated.bin"),
    "udp": ("udp-valid.bin",),
    "tcp": ("tcp-syn-valid.bin", "tcp-offset-invalid.bin"),
    "socket": ("socket-lifecycle.bin", "socket-zero-slot.bin"),
    "boot-map": (),
    "paths": (),
    "descriptors": (),
}


def run(command: list[str], *, env: dict[str, str] | None = None,
        log: Path | None = None, timeout: int = 600) -> str:
    result = subprocess.run(command, cwd=ROOT, env=env, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            timeout=timeout, check=False)
    if log is not None:
        log.write_text(result.stdout)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {' '.join(command)}\n"
                           f"{result.stdout[-4000:]}")
    return result.stdout


def verified_source(*, offline: bool) -> tuple[Path, dict]:
    lock = json.loads(LOCK.read_text())
    if (lock["schema_version"] != 1
            or lock["upstream"] != "https://github.com/llvm/llvm-project"
            or lock["source_directory"] != "compiler-rt/lib/fuzzer"
            or lock["license_path"] != "compiler-rt/LICENSE.TXT"
            or re.fullmatch(r"[0-9a-f]{40}", lock["commit"]) is None):
        raise ValueError("invalid libFuzzer source lock")
    source = OUT / "source"
    source.mkdir(parents=True, exist_ok=True)
    names = set()
    for entry in lock["files"]:
        name = entry["name"]
        if (name in names or Path(name).name != name
                or not name.endswith((".cpp", ".h", ".def", ".sh", ".TXT"))
                or re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]) is None):
            raise ValueError(f"invalid locked source name: {name}")
        names.add(name)
        path = source / name
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
            if offline:
                raise FileNotFoundError(f"missing pinned source in offline mode: {name}")
            prefix = (f"https://raw.githubusercontent.com/llvm/llvm-project/{lock['commit']}/"
                      f"{lock['source_directory']}/")
            url = (f"https://raw.githubusercontent.com/llvm/llvm-project/{lock['commit']}/"
                   f"{lock['license_path']}") if name == "LICENSE.TXT" else prefix + name
            with urllib.request.urlopen(url, timeout=30) as response:
                data = response.read(1_000_001)
            if hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError(f"pinned source hash mismatch: {name}")
            path.write_bytes(data)
        if hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
            raise ValueError(f"pinned source hash mismatch: {name}")
    return source, lock


def build_runtime(source: Path, lock: dict) -> Path:
    runtime = OUT / "runtime"
    runtime.mkdir(exist_ok=True)
    archive = runtime / "libFuzzer.a"
    compiler = run(["clang++", "--version"]).splitlines()[0]
    fingerprint = hashlib.sha256((json.dumps(lock, sort_keys=True) + compiler).encode()).hexdigest()
    marker = runtime / "archive.json"
    if archive.is_file() and marker.is_file():
        record = json.loads(marker.read_text())
        if (record.get("lock") == fingerprint
                and record.get("archive_sha256") == hashlib.sha256(archive.read_bytes()).hexdigest()):
            return archive
    objects = []
    for path in sorted(source.glob("*.cpp")):
        obj = runtime / (path.stem + ".o")
        run(["clang++", "-std=c++17", "-O2", "-g", "-fno-omit-frame-pointer",
             "-I", str(source), "-c", str(path), "-o", str(obj)],
            log=runtime / (path.stem + ".log"))
        objects.append(str(obj))
    archive.unlink(missing_ok=True)
    run(["ar", "rcs", str(archive), *objects], log=runtime / "archive.log")
    marker.write_text(json.dumps({"lock": fingerprint,
                                  "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest()},
                                 indent=2) + "\n")
    return archive


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, default="elf")
    parser.add_argument("--runs", type=int, default=10000)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.runs <= 10_000_000:
        parser.error("--runs must be between 1 and 10000000")
    OUT.mkdir(parents=True, exist_ok=True)
    source, lock = verified_source(offline=args.offline)
    archive = build_runtime(source, lock)
    env = os.environ.copy()
    env["GENOS_LIBFUZZER_ARCHIVE"] = str(archive)
    # LLVM's sanitizer-coverage counters and PC table supply actual edge feedback.
    # Use a separate target directory so ordinary kernel builds keep their flags.
    env["CARGO_TARGET_DIR"] = str(OUT / "cargo-target")
    env["RUSTFLAGS"] = " ".join((
        "-Cpasses=sancov-module",
        "-Cllvm-args=-sanitizer-coverage-level=3",
        "-Cllvm-args=-sanitizer-coverage-inline-8bit-counters",
        "-Cllvm-args=-sanitizer-coverage-pc-table",
        "-Cllvm-args=-sanitizer-coverage-trace-compares",
    ))
    manifest = str(FUZZ / "Cargo.toml")
    host = re.search(r"^host: (\S+)$", run(["rustc", "-Vv"]), re.MULTILINE)
    if host is None:
        raise RuntimeError("could not identify Rust host target")
    # Explicit --target keeps instrumentation off Cargo build scripts.
    run(["cargo", "build", "--target", host.group(1), "--release", "--offline", "--locked", "--features", "runtime",
         "--manifest-path", manifest], env=env, log=OUT / "build.log")
    executable = OUT / "cargo-target" / host.group(1) / "release/genos-coverage-fuzz"
    if not executable.is_file():
        raise RuntimeError("fuzz executable missing after build")
    corpus = OUT / "corpus" / args.target
    corpus.mkdir(parents=True, exist_ok=True)
    original = ROOT / "tools/parser-stress/corpus"
    for name in SEEDS[args.target]:
        dest = corpus / name
        if not dest.exists():
            dest.write_bytes((original / name).read_bytes())
    boot_map = b"\x28\x00\x01\x00" + struct.pack("<IIQQQQ", 7, 0, 4096, 0, 1, 0)
    for name, value in {"paths-user": b"/USER/NOTE.TXT",
                        "paths-reject": b"/USER/../NOTE.TXT",
                        "boot-map-valid": boot_map,
                        "boot-map-empty": b"\x28\x00\x01\x00",
                        "descriptors-one": struct.pack("<HHII", 0, 1, 0, 16),
                        "descriptors-empty": b"\x00\x00\x00\x00"}.items():
        if name.startswith(args.target) and not (corpus / name).exists():
            (corpus / name).write_bytes(value)
    if not any(corpus.iterdir()):
        (corpus / "empty").write_bytes(b"")
    artifacts = OUT / "artifacts" / args.target
    artifacts.mkdir(parents=True, exist_ok=True)
    env["GENOS_FUZZ_TARGET"] = args.target
    command = [str(executable), str(corpus), f"-runs={args.runs}", "-max_len=65536",
               f"-artifact_prefix={artifacts}/", "-print_final_stats=1"]
    started = time.time()
    log = OUT / f"{args.target}.log"
    try:
        output = run(command, env=env, log=log, timeout=600)
        coverage = re.findall(r"(?:INITED|DONE).*?cov: (\d+).*?ft: (\d+)", output)
        new_units = re.search(r"stat::new_units_added:\s*(\d+)", output)
        if len(coverage) < 2 or int(coverage[-1][0]) == 0 or new_units is None:
            raise RuntimeError("libFuzzer did not report nonzero coverage")
        result = {"status": "pass", "coverage": int(coverage[-1][0]),
                  "features": int(coverage[-1][1]),
                  "new_units_added": int(new_units.group(1))}
    except Exception as exc:
        result = {"status": "fail", "error": str(exc)}
        raise
    finally:
        result.update({"target": args.target, "runs": args.runs, "time": started,
                       "rustc": run(["rustc", "-Vv"]),
                       "clang": run(["clang++", "--version"]).splitlines()[0],
                       "runtime_commit": lock["commit"],
                       "source_revision": run(["git", "rev-parse", "HEAD"]).strip(),
                       "source_dirty": bool(run(["git", "status", "--porcelain"]).strip()),
                       "log": str(log), "artifacts": str(artifacts)})
        (OUT / f"{args.target}.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"PASS {args.target}: cov={result['coverage']} ft={result['features']} "
          f"runs={args.runs}; evidence={OUT / (args.target + '.json')}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(f"coverage-fuzz: {error}", file=sys.stderr)
        sys.exit(1)
