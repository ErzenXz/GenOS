#!/usr/bin/env python3
"""Run pinned Miri and an exhaustive bounded ownership pilot on immutable source."""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tarfile
import tempfile
import time

from test_exception_entry import require_clean_source

NIGHTLY = "nightly-2026-09-21"
STABLE = "1.97.0"
MIRI_FLAGS = "-Zmiri-strict-provenance -Zmiri-seed=1"
# Explicit scope: the 20,000-operation randomized allocator stress stays native.
# Its Miri exploratory run exceeded 240 seconds; it was not a passing Miri case.
STRESS = "physmem::tests::randomized_fragmented_allocations_never_alias_live_grants"
SUITES = [
    ("capability", "kernel", "capability::tests", 3, []),
    ("request", "kernel", "request::tests", 2, []),
    ("path_policy", "kernel", "path_policy::tests", 3, []),
    ("frame_grant", "kernel", "frame_grant::tests", 9, []),
    ("file_snapshot", "kernel", "file_snapshot::tests", 5, []),
    ("physmem", "kernel", "physmem::tests", 14, ["--skip", STRESS]),
    ("user_copy", "kernel", "user_copy::tests", 3, []),
    ("kernel_elf", "bootloader", "", 14, []),
]
MODEL_PROOF = "OWNERSHIP_MODEL_OK frames=2 owners=2 actions=12 depth=6 sequences=3257436 transitions=19248492 merged_states=0 elapsed_ms="


def validate_test_log(log: str, expected: int) -> None:
    summaries = [line for line in log.splitlines() if "test result:" in line]
    pattern = rf"test result: ok\. {expected} passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9]+(?:\.[0-9]+)?s"
    if len(summaries) != 1 or re.fullmatch(pattern, summaries[0]) is None:
        raise ValueError("missing, duplicate, failing or wrong-count test summary")
    if "error: Undefined Behavior" in log or "error: unsupported operation" in log:
        raise ValueError("Miri reported undefined behavior or unsupported execution")


def validate_model_log(log: str) -> None:
    lines = log.splitlines()
    depths = [f"OWNERSHIP_MODEL_DEPTH_OK depth={depth} sequences={12 ** depth}" for depth in range(1, 7)]
    if any(lines.count(marker) != 1 for marker in depths):
        raise ValueError("missing or duplicate exhaustive-depth record")
    positions = [lines.index(marker) for marker in depths]
    if positions != sorted(positions):
        raise ValueError("exhaustive depths out of order")
    final = [line for line in lines if "OWNERSHIP_MODEL_OK" in line]
    if len(final) != 1 or re.fullmatch(re.escape(MODEL_PROOF) + r"[0-9]+", final[0]) is None:
        raise ValueError("missing, malformed or incomplete exhaustive-model summary")
    if lines.index(final[0]) <= positions[-1] or "OWNERSHIP_MODEL_FAILED" in log:
        raise ValueError("model failed or completed before its final depth")


def run(command: list[str], *, root: Path, evidence: Path, label: str,
        env: dict[str, str], timeout: int = 240) -> str:
    log_path = evidence / f"{label}.log"
    record = {"command": command, "cwd": str(root), "timeout_seconds": timeout, "status": "incomplete"}
    record_path = evidence / f"{label}.json"
    record_path.write_text(json.dumps(record, indent=2) + "\n")
    started = time.monotonic()
    try:
        with log_path.open("w") as log:
            process = subprocess.Popen(command, cwd=root, env=env, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            try:
                status = process.wait(timeout=timeout)
            except BaseException:
                # cargo may have spawned a still-running interpreter: terminate
                # the entire task-owned process group, then reap its leader.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
                raise
        record["returncode"] = status
        if status != 0:
            raise RuntimeError(f"{label} exited {status}; see {log_path}")
        record["status"] = "passed"
        return log_path.read_text(errors="replace")
    except BaseException as error:
        record.update(status="failed", failure=f"{type(error).__name__}: {error}")
        raise
    finally:
        record["elapsed_seconds"] = time.monotonic() - started
        if log_path.exists():
            record["log_sha256"] = hashlib.sha256(log_path.read_bytes()).hexdigest()
        record_path.write_text(json.dumps(record, indent=2) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lane", choices=("all", "miri", "model"), default="all")
    options = parser.parse_args()
    if sys.version_info < (3, 12):
        parser.error("Python >=3.12 required")
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.check_output(["git", "archive", "--format=tar", commit], cwd=source, timeout=30)
    evidence = source / "build/pure-rust-evidence" / str(time.time_ns())
    evidence.mkdir(parents=True, exist_ok=False)
    manifest = {"status": "incomplete", "commit": commit, "source_clean": True,
                "archive_sha256": hashlib.sha256(archive).hexdigest(), "lane": options.lane,
                "miri_toolchain": NIGHTLY, "native_toolchain": STABLE, "miri_flags": MIRI_FLAGS,
                "miri_scope": SUITES, "miri_exclusions": [STRESS, "CPU assembly", "UEFI calls",
                "MMIO/PIO", "DMA", "hardware interrupts", "page-table/TLB hardware semantics"]}
    try:
        with tempfile.TemporaryDirectory(prefix="genos-pure-rust-") as temporary:
            root = Path(temporary) / "source"
            root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as source_archive:
                source_archive.extractall(root, filter="data")
            env = os.environ.copy()
            for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "MIRI_SYSROOT"):
                env.pop(key, None)
            env.update(CARGO_TARGET_DIR=str(Path(temporary) / "target"), MIRIFLAGS=MIRI_FLAGS)
            manifest["stable_rustc"] = run(["rustc", f"+{STABLE}", "-Vv"], root=root,
                                           evidence=evidence, label="stable-version", env=env)
            if options.lane in ("all", "miri"):
                manifest["miri_rustc"] = run(["rustc", f"+{NIGHTLY}", "-Vv"], root=root,
                                             evidence=evidence, label="miri-version", env=env)
                run(["cargo", f"+{NIGHTLY}", "miri", "setup", "--print-sysroot"], root=root,
                    evidence=evidence, label="miri-setup", env=env)
                for label, package, query, expected, extra in SUITES:
                    command = ["cargo", f"+{NIGHTLY}", "miri", "test", "--locked", "-p", package,
                               "--lib", query, "--", "--test-threads=1"] + extra
                    validate_test_log(run(command, root=root, evidence=evidence,
                                          label=f"miri-{label}", env=env), expected)
                # Preserve the deliberately excluded randomized stress as an
                # executed native check rather than silently dropping it.
                validate_test_log(run(["cargo", f"+{STABLE}", "test", "--locked", "-p", "kernel", "--lib",
                    STRESS, "--", "--exact"], root=root, evidence=evidence, label="native-allocator-stress", env=env), 1)
            if options.lane in ("all", "model"):
                model_manifest = root / "tools/ownership_model/Cargo.toml"
                run(["cargo", f"+{STABLE}", "build", "--release", "--locked", "--manifest-path", str(model_manifest)],
                    root=root, evidence=evidence, label="model-build", env=env)
                binary = str(Path(temporary) / "target/release/genos-ownership-model")
                validate_model_log(run([binary], root=evidence, evidence=evidence, label="ownership-model", env=env))
                oracle = run([binary, "--oracle-self-test"], root=evidence, evidence=evidence, label="model-oracle", env=env)
                if oracle.splitlines() != ["OWNERSHIP_MODEL_ORACLE_OK rejected=release-with-live-pin trace=[0, 3, 9]"]:
                    raise ValueError("negative adapter oracle did not reject the known violation")
                counterexample = json.loads((evidence / "oracle-counterexample.json").read_text())
                if counterexample["trace"] != [0, 3, 9]:
                    raise ValueError("negative adapter counterexample was not retained")
                replay = run([binary, "--replay", "0,3,9"], root=evidence, evidence=evidence, label="model-replay", env=env)
                if replay.splitlines() != ["OWNERSHIP_MODEL_REPLAY_OK trace=[0, 3, 9]"]:
                    raise ValueError("production adapter did not safely reject the retained negative sequence")
                manifest["bounded_model"] = {"frames": 2, "owners": 2, "actions": 12, "max_depth": 6,
                    "sequences": 3257436, "transitions": 19248492, "merged_states": 0}
            manifest["status"] = "passed"
    except BaseException as error:
        manifest.update(status="failed", failure=f"{type(error).__name__}: {error}")
        raise
    finally:
        (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"PURE_RUST_EVIDENCE_OK lane={options.lane} evidence={evidence}", flush=True)


if __name__ == "__main__":
    main()
