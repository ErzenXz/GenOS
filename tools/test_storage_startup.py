#!/usr/bin/env python3
"""Prove fresh-volume write-failure quarantine through the production Ring 3 shell.

Only AtaIo::write returns an injected device error. The commit protocol, quarantine,
status publication, capability policy, shell and serial runner remain production.
"""
from __future__ import annotations

import difflib
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time

from test_exception_entry import firmware_path, replace_once, require_clean_source
from reference_vm import environment

WRITE = """    fn write(&mut self, lba: u32, data: &[u8; SECTOR_BYTES]) -> Result<(), StorageError> {
        ata_write_sector(lba, data)
    }"""
FAIL_WRITE = """    fn write(&mut self, lba: u32, data: &[u8; SECTOR_BYTES]) -> Result<(), StorageError> {
        let _ = (lba, data);
        crate::serial::println("STORAGE_CREATE_WRITE_FAILURE_ARMED");
        Err(StorageError::Device)
    }"""
PROOFS = [
    "BOOT_MODE validation",
    "STORAGE_CREATE_WRITE_FAILURE_ARMED",
    "PERSISTENT_STORAGE_QUARANTINED outcome=not-published",
    "PERSISTENT_STORAGE_UNAVAILABLE",
    "USER_STORAGE_STATUS_VISIBLE_OK",
    "USER_RAMFS_TEMP_APP_OK",
    "USER_STORAGE_QUARANTINE_OK writes=denied namespace=denied ram=preserved",
    "HEADLESS_RUNTIME_OK vfs=19 lifecycle=1",
    "USER_SHELL_READY",
    "GENOS_READY",
]


def patch_fixture(root: Path) -> str:
    path = root / "kernel/src/storage.rs"
    before = path.read_text()
    after = replace_once(before, WRITE, FAIL_WRITE)
    path.write_text(after)
    return "".join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile="a/kernel/src/storage.rs", tofile="b/kernel/src/storage.rs"))


def validate_log(log: str) -> None:
    lines = log.replace("\r", "").splitlines()
    runtime = [line for line in lines if "HEADLESS_RUNTIME_OK" in line]
    if len(runtime) != 1 or re.fullmatch(r"HEADLESS_RUNTIME_OK vfs=[1-9][0-9]* lifecycle=1", runtime[0]) is None:
        raise ValueError("missing, duplicated or malformed runtime proof")
    proofs = [runtime[0] if marker.startswith("HEADLESS_RUNTIME_OK") else marker for marker in PROOFS]
    if any(lines.count(marker) != 1 for marker in proofs):
        raise ValueError("missing, embedded or duplicated startup quarantine proof")
    positions = [lines.index(marker) for marker in proofs]
    if positions != sorted(positions):
        raise ValueError("startup quarantine proof out of order")
    for forbidden in ("PERSISTENT_STORAGE_CREATED", "PERSISTENT_STORAGE_RESTORED",
                      "USER_STORAGE_FILE_COMMITTED", "USER_STORAGE_FILE_RESTORED",
                      "EXCEPTION_FRAME", "USER_FAULT", "KERNEL PANIC", "BOOTLOADER_PANIC",
                      "RECOVERY_CONSOLE_READY", "_FAILED", "userspace lifecycle error"):
        if any(forbidden in line for line in lines):
            raise ValueError(f"startup quarantine escaped or falsely acknowledged storage: {forbidden}")


def main() -> None:
    if sys.version_info < (3, 12):
        raise SystemExit("Python >=3.12 required")
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.check_output(["git", "archive", "--format=tar", commit], cwd=source, timeout=30)
    evidence = source / "build/storage-startup-evidence" / str(time.time_ns())
    evidence.mkdir(parents=True, exist_ok=False)
    manifest = {"status": "incomplete", "commit": commit, "source_clean": True,
                "archive_sha256": hashlib.sha256(archive).hexdigest(),
                "scope": "fresh-volume device write failure; production quarantine and shell",
                "command": ["cargo", "xtask", "test-serial"]}
    try:
        manifest.update(environment(Path(firmware_path()), source))
        with tempfile.TemporaryDirectory(prefix="genos-storage-startup-") as temporary:
            root = Path(temporary) / "source"
            root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as bundle:
                bundle.extractall(root, filter="data")
            patch = patch_fixture(root)
            (evidence / "fixture.patch").write_text(patch)
            manifest["fixture_sha256"] = hashlib.sha256(patch.encode()).hexdigest()
            if (root / "build/genos-data-test.img").exists():
                raise RuntimeError("fixture unexpectedly contains a pre-existing data image")
            try:
                with (evidence / "run.log").open("w") as output:
                    process = subprocess.Popen(manifest["command"], cwd=root, stdout=output,
                                               stderr=subprocess.STDOUT, start_new_session=True)
                    try:
                        code = process.wait(timeout=900)
                    except BaseException:
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        process.wait()
                        raise
                if code != 0:
                    raise RuntimeError(f"serial runner exited {code}; see {evidence / 'run.log'}")
                manifests = list((root / "build/validation-evidence").glob("*/serial-input/manifest.txt"))
                if len(manifests) != 1:
                    raise RuntimeError("missing or duplicated serial-run manifest")
                inner = manifests[0].read_text()
                if not inner.startswith("status=passed\n"):
                    raise RuntimeError("serial runner did not record success")
                validate_log(manifests[0].with_name("serial.log").read_text(errors="replace"))
                manifest["status"] = "passed"
            finally:
                retained = root / "build/validation-evidence"
                if retained.exists():
                    shutil.copytree(retained, evidence / "validation-evidence")
    except BaseException as error:
        manifest.update(status="failed", failure=f"{type(error).__name__}: {error}")
        raise
    finally:
        manifest["files_sha256"] = {
            str(path.relative_to(evidence)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(evidence.rglob("*")) if path.is_file()
        }
        (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"STORAGE_STARTUP_PROBE_OK evidence={evidence}", flush=True)


if __name__ == "__main__":
    main()
