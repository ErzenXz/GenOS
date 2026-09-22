#!/usr/bin/env python3
"""Reject malformed boot handoffs before allocator/architecture initialization.

Fixtures modify the loader input, never the production validator. Source,
patches, image, serial output and failure manifests are retained per case.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import time

from test_exception_entry import firmware_path, replace_once, require_clean_source
from reference_vm import environment, load_profile, qemu_args

CASES = ("loader-version", "count", "overlap", "initrd-usable", "kernel-usable", "cmdline")


def patch_fixture(root: Path, case: str) -> str:
    path = root / "bootloader/src/main.rs"
    before = path.read_text()
    if case == "loader-version":
        after = replace_once(before, "meta.desc_size, meta.desc_version", "meta.desc_size, 0xffff")
    else:
        mutations = {
            "count": "boot_info.memory_map.region_count = 257;",
            "overlap": "boot_info.memory_map.regions[1] = boot_info.memory_map.regions[0];",
            "initrd-usable": "boot_info.initrd.base = boot_info.memory_map.regions.iter().find(|r| r.kind == genos_abi::MemoryRegionKind::Usable).unwrap().start; boot_info.initrd.size = 4096;",
            "kernel-usable": "for r in &mut boot_info.memory_map.regions { if r.start <= loaded_kernel.entry && loaded_kernel.entry < r.start.saturating_add(r.size) { r.kind = genos_abi::MemoryRegionKind::Usable; } }",
            "cmdline": "boot_info.cmdline_len = 129;",
        }
        if case not in mutations:
            raise ValueError(f"unknown boot memory case: {case}")
        anchor = "    // SAFETY: boot_info_ptr is a retained, aligned page allocation;"
        after = replace_once(before, anchor, f"    {mutations[case]}\n\n" + anchor)
    path.write_text(after)
    return "".join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile="a/bootloader/src/main.rs", tofile="b/bootloader/src/main.rs"))


def validate_log(log: str, case: str) -> None:
    lines = log.replace("\r", "").splitlines()
    marker = "BOOT_MEMORY_MAP_REJECTED" if case == "loader-version" else "BOOT_INFO_REJECTED"
    if lines.count(marker) != 1:
        raise ValueError("missing or duplicated exact boot rejection")
    if lines.count("GenOS kernel entered") != (0 if case == "loader-version" else 1):
        raise ValueError("wrong rejection phase")
    for forbidden in ("BOOT_MEMORY_MAP_VALIDATED", "FRAME_ALLOCATOR_BITMAP_READY",
                      "GDT/TSS initialized", "GENOS_READY", "NORMAL_SHELL_READY",
                      "EXCEPTION_FRAME", "KERNEL PANIC", "BOOTLOADER_PANIC"):
        if any(forbidden in line for line in lines):
            raise ValueError(f"rejected handoff continued or faulted: {forbidden}")


def boot(root: Path, evidence: Path, case: str, timeout: int) -> None:
    # Keep this fault campaign explicit; the manifest retains exact arguments.
    log_path = evidence / "serial.log"
    args = qemu_args(root=root) + [
            "-drive", f"if=pflash,format=raw,readonly=on,file={firmware_path()}",
            "-drive", load_profile(root)["boot_drive"] + ",file=build/genos.img", "-net", "none",
            "-display", "none", "-monitor", "none", "-serial", f"file:{log_path}", "-no-reboot"]
    (evidence / "qemu-command.json").write_text(json.dumps(args, indent=2) + "\n")
    marker = "BOOT_MEMORY_MAP_REJECTED" if case == "loader-version" else "BOOT_INFO_REJECTED"
    with (evidence / "qemu.log").open("w") as output:
        process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                log = log_path.read_text(errors="replace") if log_path.exists() else ""
                if process.poll() is not None:
                    raise RuntimeError(f"QEMU exited unexpectedly: {process.returncode}")
                if marker in log:
                    time.sleep(0.5)
                    if process.poll() is not None:
                        raise RuntimeError("QEMU exited/reset after apparent rejection")
                    validate_log(log_path.read_text(errors="replace"), case)
                    return
                time.sleep(0.05)
            raise TimeoutError(f"no {marker} within {timeout}s")
        finally:
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
    run = source / "build/boot-memory-evidence" / str(time.time_ns())
    for case in ([options.case] if options.case else CASES):
        evidence = run / case
        evidence.mkdir(parents=True, exist_ok=False)
        manifest = {"case": case, "status": "incomplete", "commit": commit, "source_clean": True}
        try:
            firmware = Path(firmware_path())
            manifest.update(environment(firmware, source))
            with tempfile.TemporaryDirectory(prefix="genos-boot-memory-") as temporary:
                root = Path(temporary) / "source"
                root.mkdir()
                with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as source_archive:
                    source_archive.extractall(root, filter="data")
                fixture = patch_fixture(root, case)
                (evidence / "fixture.patch").write_text(fixture)
                manifest["fixture_sha256"] = hashlib.sha256(fixture.encode()).hexdigest()
                with (evidence / "build.log").open("w") as output:
                    subprocess.run(["cargo", "xtask", "build"], cwd=root, stdout=output,
                                   stderr=subprocess.STDOUT, check=True, timeout=600)
                manifest["image_sha256"] = hashlib.sha256((root / "build/genos.img").read_bytes()).hexdigest()
                boot(root, evidence, case, options.timeout)
                manifest["status"] = "passed"
        except Exception as error:
            manifest["status"] = "failed"
            manifest["failure"] = str(error)
            raise
        finally:
            serial = evidence / "serial.log"
            if serial.exists():
                manifest["serial_sha256"] = hashlib.sha256(serial.read_bytes()).hexdigest()
            (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(f"BOOT_MEMORY_PROBE_OK case={case} evidence={evidence}", flush=True)


if __name__ == "__main__":
    main()
