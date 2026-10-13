#!/usr/bin/env python3
"""Build and run the host-only GenOS coverage fuzzer with a pinned runtime."""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
FUZZ = ROOT / "tools/coverage-fuzz"
LOCK = FUZZ / "runtime-lock.json"
OUT = ROOT / "build/coverage-fuzz"
TARGETS = (
    "elf", "native-elf", "ipv4", "ipv6", "udp", "tcp", "socket",
    "boot-map", "paths", "descriptors", "handles",
    "partition", "snapshot", "dhcp", "dns", "arp", "icmp", "syscalls", "endpoints",
)
SEEDS = {
    "elf": ("elf-valid.bin", "elf-overflow.bin", "elf-truncated.bin"),
    "native-elf": ("native-elf-valid.bin", "native-elf-late-wx.bin"),
    "ipv4": ("ipv4-udp-valid.bin", "ipv4-fragment.bin"),
    "ipv6": ("ipv6-udp-valid.bin", "ipv6-options-truncated.bin"),
    "udp": ("udp-valid.bin",),
    "tcp": ("tcp-syn-valid.bin", "tcp-offset-invalid.bin", "tcp-option-zero-length.bin",
            "tcp-eol-nonzero-padding.bin", "tcp-unknown-option-valid.bin", "tcp-mss-unaligned-valid.bin"),
    "socket": ("socket-lifecycle.bin", "socket-zero-slot.bin"),
    "boot-map": (),
    "paths": (),
    "descriptors": (),
    "handles": (),
    "partition": ('partition-valid.bin', 'partition-overflow.bin', 'partition-short.bin'),
    "snapshot": ('snapshot-valid.bin', 'snapshot-checksum.bin', 'snapshot-path.bin'),
    "dhcp": ('dhcp-valid.bin', 'dhcp-truncated-option.bin'),
    "dns": ("dns-valid.bin", "dns-truncated.bin", "dns-error.bin", "dns-truncated-flag.bin", "dns-forward-pointer.bin", "dns-self-pointer.bin", "dns-wrong-question.bin"),
    "arp": ('arp-valid.bin',),
    "icmp": ('icmp-valid.bin',),
    "syscalls": ('syscalls-sequence.bin',),
    "endpoints": ('endpoints-sequence.bin',),
}


def run(command: list[str], *, env: dict[str, str] | None = None,
        log: Path | None = None, timeout: int = 600, cwd: Path = ROOT) -> str:
    if log is None:
        result = subprocess.run(command, cwd=cwd, env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                timeout=timeout, check=False)
        output = result.stdout
    else:
        log.parent.mkdir(parents=True, exist_ok=True)
        # Stream directly to disk so a timeout cannot discard partial output.
        with log.open("w") as stream:
            result = subprocess.run(command, cwd=cwd, env=env, text=True,
                                    stdout=stream, stderr=subprocess.STDOUT,
                                    timeout=timeout, check=False)
        output = log.read_text(errors="replace")
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {' '.join(command)}\n"
                           f"{output[-4000:]}")
    return output


def check_source_files(source: Path, lock: dict) -> None:
    expected = {entry["name"] for entry in lock["files"]}
    if any(path.name not in expected or not path.is_file() or path.is_symlink()
           for path in source.iterdir()):
        raise ValueError("unexpected or symlinked file in pinned runtime source cache")


def verified_source(*, offline: bool, lock_path: Path = LOCK) -> tuple[Path, dict]:
    lock = json.loads(lock_path.read_text())
    if (lock["schema_version"] != 1
            or lock["upstream"] != "https://github.com/llvm/llvm-project"
            or lock["source_directory"] != "compiler-rt/lib/fuzzer"
            or lock["license_path"] != "compiler-rt/LICENSE.TXT"
            or re.fullmatch(r"[0-9a-f]{40}", lock["commit"]) is None):
        raise ValueError("invalid libFuzzer source lock")
    source = OUT / "source"
    source.mkdir(parents=True, exist_ok=True)
    if source.is_symlink():
        raise ValueError("pinned runtime source cache cannot be a symlink")
    check_source_files(source, lock)
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


def build_runtime(source: Path, lock: dict, evidence: Path) -> Path:
    check_source_files(source, lock)
    runtime = OUT / "runtime"
    runtime.mkdir(exist_ok=True)
    archive = runtime / "libFuzzer.a"
    compiler = run(["clang++", "--version"]).splitlines()[0]
    recipe = ["clang++", "-std=c++17", "-O2", "-g", "-fno-omit-frame-pointer"]
    fingerprint = hashlib.sha256(json.dumps({"lock": lock, "compiler": compiler,
                                            "recipe": recipe, "archiver": ["ar", "rcs"]}, sort_keys=True).encode()).hexdigest()
    marker = runtime / "archive.json"
    if archive.is_file() and marker.is_file():
        record = json.loads(marker.read_text())
        if (record.get("lock") == fingerprint
                and record.get("archive_sha256") == hashlib.sha256(archive.read_bytes()).hexdigest()):
            return archive
    objects = []
    # Never compile an unlocked file left in a mutable cache directory.
    for name in sorted(entry["name"] for entry in lock["files"] if entry["name"].endswith(".cpp")):
        path = source / name
        obj = runtime / (path.stem + ".o")
        run([*recipe, "-I", str(source), "-c", str(path), "-o", str(obj)],
            log=evidence / "runtime" / (path.stem + ".log"))
        objects.append(str(obj))
    archive.unlink(missing_ok=True)
    run(["ar", "rcs", str(archive), *objects], log=evidence / "runtime/archive.log")
    marker.write_text(json.dumps({"lock": fingerprint,
                                  "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest()},
                                 indent=2) + "\n")
    return archive


def parse_statistics(output: str, runs: int) -> dict:
    phases = [re.findall(r"^#(\d+)\s+" + phase + r"\s+cov: (\d+)\s+ft: (\d+)",
                         output, re.MULTILINE) for phase in ("INITED", "DONE")]
    units = re.findall(r"^stat::number_of_executed_units:\s*(\d+)$", output, re.MULTILINE)
    added = re.findall(r"^stat::new_units_added:\s*(\d+)$", output, re.MULTILINE)
    if (any(len(phase) != 1 for phase in phases) or len(units) != 1 or len(added) != 1
            or int(units[0]) != runs or int(phases[-1][0][0]) != runs
            or int(phases[-1][0][1]) == 0):
        raise RuntimeError("missing exact completed coverage statistics")
    return {"coverage": int(phases[-1][0][1]), "features": int(phases[-1][0][2]),
            "new_units_added": int(added[0])}


def campaign(args, evidence: Path, root: Path, result: dict) -> None:
    lock_path = root / "tools/coverage-fuzz/runtime-lock.json"
    # Cache updates are serialized; each run retains its own runtime archive.
    with (OUT / "runtime.lock").open("a") as guard:
        fcntl.flock(guard, fcntl.LOCK_EX)
        source, lock = verified_source(offline=args.offline, lock_path=lock_path)
        cached_archive = build_runtime(source, lock, evidence)
        archive = evidence / "runtime/libFuzzer.a"
        archive.parent.mkdir(exist_ok=True)
        shutil.copyfile(cached_archive, archive)
    result.update(runtime_commit=lock["commit"], runtime_lock_sha256=hashlib.sha256(lock_path.read_bytes()).hexdigest(),
                  runtime_archive_sha256=hashlib.sha256(archive.read_bytes()).hexdigest())
    env = os.environ.copy()
    env["GENOS_LIBFUZZER_ARCHIVE"] = str(archive)
    # LLVM's sanitizer-coverage counters and PC table supply actual edge feedback.
    # Use a separate target directory so ordinary kernel builds keep their flags.
    env["CARGO_TARGET_DIR"] = str(root / "target/coverage")
    env["RUSTFLAGS"] = " ".join((
        "-Cpasses=sancov-module",
        "-Cllvm-args=-sanitizer-coverage-level=3",
        "-Cllvm-args=-sanitizer-coverage-inline-8bit-counters",
        "-Cllvm-args=-sanitizer-coverage-pc-table",
        "-Cllvm-args=-sanitizer-coverage-trace-compares",
    ))
    manifest = str(root / "tools/coverage-fuzz/Cargo.toml")
    host = re.search(r"^host: (\S+)$", run(["rustc", "-Vv"]), re.MULTILINE)
    if host is None:
        raise RuntimeError("could not identify Rust host target")
    # Explicit --target keeps instrumentation off Cargo build scripts.
    run(["cargo", "build", "--target", host.group(1), "--release", "--offline", "--locked", "--features", "runtime",
         "--manifest-path", manifest], env=env, log=evidence / "build.log", cwd=root)
    built = Path(env["CARGO_TARGET_DIR"]) / host.group(1) / "release/genos-coverage-fuzz"
    executable = evidence / "genos-coverage-fuzz"
    shutil.copy2(built, executable)
    if not executable.is_file():
        raise RuntimeError("fuzz executable missing after build")
    result["executable_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()
    result["rustflags"] = env["RUSTFLAGS"]
    corpus = evidence / "corpus"
    corpus.mkdir()
    original = root / "tools/parser-stress/corpus"
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
    if args.target == "handles":
        records = [(0, 0, 0, handle, 3) for handle in range(6)]
        records += [(1, 1, 0, 1, 1), (2, 0, 6, 1, 0), (2, 0, 0, 1, 0),
                    (0, 1, 6, 1, 1 << 63), (3, 0, 0, 0, 0), (1, 1, 6, 1, 1 << 63)]
        (corpus / "handles-sequence").write_bytes(b"".join(
            struct.pack("<BBBBQQ", action, owner, kind, 0, handle, rights)
            for action, owner, kind, handle, rights in records))
    if not any(corpus.iterdir()):
        (corpus / "empty").write_bytes(b"")
    result["initial_corpus_sha256"] = {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                                       for path in sorted(corpus.iterdir())}
    artifacts = evidence / "artifacts"
    artifacts.mkdir()
    env["GENOS_FUZZ_TARGET"] = args.target
    command = [str(executable), str(corpus), f"-runs={args.runs}", f"-seed={args.seed}", "-max_len=65536",
               f"-artifact_prefix={artifacts}/", "-print_final_stats=1"]
    result["command"] = command
    output = run(command, env=env, log=evidence / "run.log", timeout=600, cwd=root)
    result.update(status="pass", **parse_statistics(output, args.runs))
    if args.require_growth and result["new_units_added"] == 0:
        raise RuntimeError("coverage run discovered no new corpus units")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, default="elf")
    parser.add_argument("--runs", type=int, default=10000)
    parser.add_argument("--seed", type=int, default=47454)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--require-growth", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.runs <= 10_000_000 or not 1 <= args.seed <= 0xffffffff:
        parser.error("--runs must be 1..10000000 and --seed must be 1..4294967295")
    evidence = OUT / "evidence" / str(time.time_ns())
    evidence.mkdir(parents=True, exist_ok=False)
    result = {"status": "incomplete", "target": args.target, "runs": args.runs, "seed": args.seed}
    (evidence / "manifest.json").write_text(json.dumps(result, indent=2) + "\n")
    try:
        if run(["git", "status", "--porcelain"]).strip():
            raise RuntimeError("coverage qualification requires a clean committed source tree")
        result["source_revision"] = run(["git", "rev-parse", "HEAD"]).strip()
        result.update(source_tree=run(["git", "rev-parse", result["source_revision"] + "^{tree}"]).strip(), source_dirty=False,
                      rustc=run(["rustc", "-Vv"]), clang=run(["clang++", "--version"]))
        archive = subprocess.check_output(["git", "archive", "--format=tar", result["source_revision"]], cwd=ROOT)
        (evidence / "source.tar").write_bytes(archive)
        result["source_archive_sha256"] = hashlib.sha256(archive).hexdigest()
        with tempfile.TemporaryDirectory(prefix="genos-coverage-") as temporary:
            root = Path(temporary) / "source"
            root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tree:
                tree.extractall(root, filter="data")
            campaign(args, evidence, root, result)
    except BaseException as exc:
        result.update(status="fail", error=f"{type(exc).__name__}: {exc}")
        raise
    finally:
        result["files_sha256"] = {str(path.relative_to(evidence)): hashlib.sha256(path.read_bytes()).hexdigest()
                                  for path in sorted(evidence.rglob("*")) if path.is_file() and path.name != "manifest.json"}
        (evidence / "manifest.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"PASS {args.target}: cov={result['coverage']} ft={result['features']} "
          f"runs={args.runs}; evidence={evidence / 'manifest.json'}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(f"coverage-fuzz: {error}", file=sys.stderr)
        sys.exit(1)
