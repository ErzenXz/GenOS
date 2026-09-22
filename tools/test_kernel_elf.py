#!/usr/bin/env python3
"""Reject malformed staged KERNEL.ELF files through the production UEFI loader.

Build one immutable source archive, then replace only the kernel file inside its
throwaway boot image. The production loader and validator are never patched.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
from pathlib import Path
import struct
import subprocess
import sys
import tarfile
import tempfile
import time

from reference_vm import environment, load_profile, qemu_args
from test_exception_entry import firmware_path, require_clean_source

CASES = {"truncated-header": "Header", "program-range": "ProgramHeaders",
         "segment-overflow": "MemoryRange", "overlap": "Overlap",
         "writable-code": "Permissions", "entry-data": "Entry"}


def mutate_elf(original: bytes, case: str) -> tuple[bytes, dict]:
    if case not in CASES:
        raise ValueError("unknown kernel ELF fixture")
    if len(original) < 64 or original[:7] != b"\x7fELF\x02\x01\x01":
        raise ValueError("fixture input must be a complete ELF64 little-endian header")
    table = struct.unpack_from("<Q", original, 32)[0]
    stride, count = struct.unpack_from("<HH", original, 54)
    if stride != 56 or not 1 <= count <= 32 or table + stride * count > len(original):
        raise ValueError("fixture input has an invalid program table")
    loads = [table + index * stride for index in range(count)
             if struct.unpack_from("<I", original, table + index * stride)[0] == 1]
    if len(loads) < 2 or struct.unpack_from("<I", original, loads[0] + 4)[0] != 5:
        raise ValueError("fixture requires separate executable and data load segments")
    changed = bytearray(original)
    edits = []
    def put(offset: int, size: int, value: int) -> None:
        before = changed[offset:offset + size]
        after = value.to_bytes(size, "little")
        changed[offset:offset + size] = after
        edits.append({"offset": offset, "before_hex": before.hex(), "after_hex": after.hex()})
    if case == "truncated-header":
        changed = changed[:4]
    elif case == "program-range":
        put(32, 8, (1 << 64) - 1)
    elif case == "segment-overflow":
        put(loads[0] + 40, 8, (1 << 64) - 1)
    elif case == "overlap":
        address = struct.unpack_from("<Q", original, loads[0] + 16)[0]
        put(loads[1] + 16, 8, address)
        put(loads[1] + 24, 8, address)
    elif case == "writable-code":
        put(loads[0] + 4, 4, 7)
    else:
        data = next((at for at in loads if struct.unpack_from("<I", original, at + 4)[0] == 4), None)
        if data is None:
            raise ValueError("fixture requires a read-only non-executable load segment")
        put(24, 8, struct.unpack_from("<Q", original, data + 16)[0])
    return bytes(changed), {"case": case, "expected_error": CASES[case],
                            "original_bytes": len(original), "mutated_bytes": len(changed), "edits": edits}


def validate_log(log: str, case: str) -> None:
    lines = log.replace("\r", "").splitlines()
    required = ["GenOS UEFI loader v1", "Loading kernel ELF",
                "KERNEL_ELF_REJECTED reason=" + CASES[case], "GenOS boot failed: LOAD_ERROR"]
    if any(lines.count(marker) != 1 for marker in required):
        raise ValueError("missing, duplicated or incorrect loader rejection")
    positions = [lines.index(marker) for marker in required]
    if positions != sorted(positions):
        raise ValueError("ELF rejection appeared in the wrong boot phase")
    if sum(line.startswith("KERNEL_ELF_REJECTED") for line in lines) != 1:
        raise ValueError("duplicate or conflicting rejection reason")
    for forbidden in ("Loading initrd", "GenOS kernel entered", "BOOT_MODE ",
                      "BOOT_MEMORY_MAP_", "GENOS_READY", "NORMAL_SHELL_READY",
                      "EXCEPTION_FRAME", "KERNEL PANIC", "BOOTLOADER_PANIC"):
        if any(forbidden in line for line in lines):
            raise ValueError(f"malformed image continued or faulted: {forbidden}")


def boot(root: Path, evidence: Path, case: str, timeout: int) -> None:
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
                status = process.poll()
                if status is not None:
                    # This loader path deliberately requests UEFI shutdown after
                    # printing the error and stalling. A timeout/reset alone is
                    # never sufficient: all ordered rejection records are needed.
                    validate_log(log, case)
                    if status != 0:
                        raise RuntimeError(f"QEMU exited abnormally after rejection: {status}")
                    return
                time.sleep(0.05)
            raise TimeoutError(f"no completed loader rejection/shutdown within {timeout}s")
        finally:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=CASES)
    parser.add_argument("--timeout", type=int, default=120)
    options = parser.parse_args()
    if sys.version_info < (3, 12) or options.timeout <= 0:
        parser.error("Python >=3.12 and positive timeout required")
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.run(["git", "archive", "--format=tar", commit], cwd=source,
                             stdout=subprocess.PIPE, check=True, timeout=30).stdout
    run = source / "build/kernel-elf-evidence" / str(time.time_ns())
    run.mkdir(parents=True)
    campaign = {"commit": commit, "source_clean": True, "status": "incomplete"}
    try:
        with tempfile.TemporaryDirectory(prefix="genos-kernel-elf-") as temporary:
            root = Path(temporary) / "source"
            root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as source_archive:
                source_archive.extractall(root, filter="data")
            campaign.update(environment(Path(firmware_path()), root))
            with (run / "build.log").open("w") as output:
                subprocess.run(["cargo", "xtask", "build"], cwd=root, stdout=output,
                               stderr=subprocess.STDOUT, check=True, timeout=600)
            original = (root / "target/x86_64-unknown-none/debug/kernel").read_bytes()
            campaign["original_kernel_sha256"] = hashlib.sha256(original).hexdigest()
            for case in ([options.case] if options.case else CASES):
                evidence = run / case
                evidence.mkdir()
                manifest = dict(campaign, case=case, status="incomplete")
                try:
                    malformed, changes = mutate_elf(original, case)
                    payload = evidence / "KERNEL.ELF"
                    payload.write_bytes(malformed)
                    (evidence / "mutation.json").write_text(json.dumps(changes, indent=2) + "\n")
                    manifest["kernel_sha256"] = hashlib.sha256(malformed).hexdigest()
                    # Only the disposable image's staged kernel is replaced.
                    command = ["mcopy", "-o", "-i", "build/genos.img", str(payload), "::/EFI/GENOS/KERNEL.ELF"]
                    (evidence / "image-command.json").write_text(json.dumps(command, indent=2) + "\n")
                    with (evidence / "image.log").open("w") as output:
                        subprocess.run(command, cwd=root, stdout=output, stderr=subprocess.STDOUT,
                                       check=True, timeout=30)
                    manifest["image_sha256"] = hashlib.sha256((root / "build/genos.img").read_bytes()).hexdigest()
                    boot(root, evidence, case, options.timeout)
                    manifest["status"] = "passed"
                except Exception as error:
                    manifest.update(status="failed", failure=f"{type(error).__name__}: {error}")
                    raise
                finally:
                    for name in ("serial.log", "qemu.log", "image.log", "mutation.json"):
                        path = evidence / name
                        if path.exists():
                            manifest[name + "_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
                    (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
                print(f"KERNEL_ELF_PROBE_OK case={case} evidence={evidence}", flush=True)
            campaign["status"] = "passed"
    except Exception as error:
        campaign.update(status="failed", failure=f"{type(error).__name__}: {error}")
        raise
    finally:
        (run / "manifest.json").write_text(json.dumps(campaign, indent=2) + "\n")


if __name__ == "__main__":
    main()
