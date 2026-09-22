#!/usr/bin/env python3
"""Prove immutable diagnostic handles through a disposable normal Ring 3 shell.

Only the shell application is replaced. The production file registry, snapshot
table, runtime, allocator, process lifecycle, copy-out and scheduling are intact.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import time

from test_exception_entry import firmware_path, replace_once, require_clean_source
from reference_vm import environment, load_profile, qemu_args

ENTRY = 'pub extern "C" fn _start(console: u64, supervisor: u64) -> ! {'
LIVE = re.compile(r"^MEMORY_SNAPSHOT_LIVE phase=(before|during|after) frames=([0-9]+)$", re.MULTILINE)
READY = "MEMORY_SNAPSHOT_READY"


def patch_fixture(root: Path) -> str:
    path = root / "userspace/shell/src/main.rs"
    before = path.read_text()
    # Require the real entry contract even though the disposable application
    # replaces the whole shell, avoiding unused production shell size/strings.
    replace_once(before, ENTRY, ENTRY)
    after = (root / "tools/fixtures/memory_snapshot.rs").read_text()
    replace_once(after, ENTRY, ENTRY)
    path.write_text(after)
    return "".join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile="a/userspace/shell/src/main.rs", tofile="b/userspace/shell/src/main.rs"))


def validate_log(log: str) -> dict[str, int]:
    log = log.replace("\r", "")
    lines = log.splitlines()
    samples = LIVE.findall(log)
    if len(samples) != 3 or [phase for phase, _ in samples] != ["before", "during", "after"]:
        raise ValueError("missing, duplicated or unordered allocator samples")
    if sum(line.startswith("MEMORY_SNAPSHOT_LIVE") for line in lines) != 3:
        raise ValueError("malformed allocator sample")
    counts = {phase: int(value) for phase, value in samples}
    if not 0 < counts["before"] < counts["during"] or counts["after"] != counts["before"]:
        raise ValueError("held process did not allocate and return to the live-frame baseline")
    required = ["BOOT_MODE normal", "BOOT_MEMORY_MAP_VALIDATED", "GENOS_READY",
                "MEMORY_SNAPSHOT_BEGIN", f"MEMORY_SNAPSHOT_LIVE phase=before frames={counts['before']}",
                f"MEMORY_SNAPSHOT_LIVE phase=during frames={counts['during']}",
                "MEMORY_SNAPSHOT_FIRST_STABLE",
                f"MEMORY_SNAPSHOT_LIVE phase=after frames={counts['after']}",
                "MEMORY_SNAPSHOT_HANDLES_REUSED", READY, "NORMAL_SHELL_READY"]
    if any(lines.count(marker) != 1 for marker in required):
        raise ValueError("missing, embedded or duplicated exact snapshot proof")
    positions = [lines.index(marker) for marker in required]
    if positions != sorted(positions):
        raise ValueError("snapshot admission, coherence or cleanup proof is out of order")
    for forbidden in ("BOOT_MODE validation", "MEMORY_SNAPSHOT_FAILED", "EXCEPTION_FRAME",
                      "USER_FAULT", "KERNEL PANIC", "BOOTLOADER_PANIC", "RECOVERY_CONSOLE_READY",
                      "userspace lifecycle error", "_FAILED"):
        if any(forbidden in line for line in lines):
            raise ValueError(f"snapshot proof faulted or escaped normal execution: {forbidden}")
    return counts


def boot(root: Path, evidence: Path, timeout: int) -> dict[str, int]:
    serial = evidence / "serial.log"
    args = qemu_args(root=root) + [
        "-drive", f"if=pflash,format=raw,readonly=on,file={firmware_path()}",
        "-drive", load_profile(root)["boot_drive"] + ",file=build/genos.img", "-net", "none",
        "-display", "none", "-monitor", "none", "-serial", f"file:{serial}", "-no-reboot"]
    (evidence / "qemu-command.json").write_text(json.dumps(args, indent=2) + "\n")
    with (evidence / "qemu.log").open("w") as output:
        process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                log = serial.read_text(errors="replace") if serial.exists() else ""
                if process.poll() is not None:
                    raise RuntimeError(f"QEMU exited/reset unexpectedly: {process.returncode}")
                if "MEMORY_SNAPSHOT_FAILED" in log or "RECOVERY_CONSOLE_READY" in log:
                    raise RuntimeError("Ring 3 snapshot fixture failed")
                if READY in log and "NORMAL_SHELL_READY" in log:
                    time.sleep(0.5)
                    if process.poll() is not None:
                        raise RuntimeError("QEMU exited/reset after apparent proof")
                    return validate_log(serial.read_text(errors="replace"))
                time.sleep(0.05)
            raise TimeoutError(f"no completed snapshot proof within {timeout}s")
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=int, default=120)
    options = parser.parse_args()
    if sys.version_info < (3, 12) or options.timeout <= 0:
        parser.error("Python >=3.12 and positive timeout required")
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.run(["git", "archive", "--format=tar", commit], cwd=source,
                             stdout=subprocess.PIPE, check=True, timeout=30).stdout
    evidence = source / "build/memory-snapshot-evidence" / str(time.time_ns())
    evidence.mkdir(parents=True, exist_ok=False)
    manifest = {"status": "incomplete", "commit": commit, "source_clean": True,
                "boot_mode": "normal", "fixture": "memory_snapshot.rs"}
    try:
        manifest.update(environment(Path(firmware_path()), source))
        with tempfile.TemporaryDirectory(prefix="genos-memory-snapshot-") as temporary:
            root = Path(temporary) / "source"
            root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as source_archive:
                source_archive.extractall(root, filter="data")
            fixture = patch_fixture(root)
            (evidence / "fixture.patch").write_text(fixture)
            manifest["fixture_sha256"] = hashlib.sha256(fixture.encode()).hexdigest()
            with (evidence / "build.log").open("w") as output:
                subprocess.run(["cargo", "xtask", "build"], cwd=root, stdout=output,
                               stderr=subprocess.STDOUT, check=True, timeout=600)
            manifest["image_sha256"] = hashlib.sha256((root / "build/genos.img").read_bytes()).hexdigest()
            manifest["live_frames"] = boot(root, evidence, options.timeout)
            manifest["status"] = "passed"
    except Exception as error:
        manifest.update(status="failed", failure=f"{type(error).__name__}: {error}")
        raise
    finally:
        serial = evidence / "serial.log"
        if serial.exists():
            manifest["serial_sha256"] = hashlib.sha256(serial.read_bytes()).hexdigest()
        (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"MEMORY_SNAPSHOT_PROBE_OK evidence={evidence}", flush=True)


if __name__ == "__main__":
    main()
