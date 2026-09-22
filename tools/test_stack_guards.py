#!/usr/bin/env python3
"""Prove kernel guard geometry and selected real stack-overflow fault paths.

The disposable fixture changes only main's deliberate fault site. Guard
installation, page tables, TSS/IDT entry and fatal dispatch remain production code.
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

from test_exception_entry import FRAME, firmware_path, replace_once, require_clean_source
from reference_vm import environment, load_profile, qemu_args

STACK_SIZES = {"boot": 2 * 1024 * 1024, "irq": 64 * 1024, "privilege": 64 * 1024,
               "double-fault": 16 * 1024, "nmi": 16 * 1024,
               "machine-check": 16 * 1024, "debug": 16 * 1024}
CASES = {"boot-lower": (0, "boot", "lower"), "privilege-lower": (2, "privilege", "lower"),
         "irq-lower": (1, "irq", "lower"), "double-fault-upper": (3, "double-fault", "upper")}
READY = "KERNEL_STACK_GUARDS_READY stacks=7 guards=14 bytes=4096"
REGION = re.compile(r"^STACK_GUARD_REGION name=([a-z-]+) lower=0x([0-9a-f]+)"
                    r" usable=0x([0-9a-f]+) top=0x([0-9a-f]+) end=0x([0-9a-f]+)$", re.MULTILINE)
TARGET = re.compile(r"^STACK_GUARD_PROBE_TARGET stack=([a-z-]+) side=(lower|upper)"
                    r" address=0x([0-9a-f]+)$", re.MULTILINE)


def patch_fixture(root: Path, case: str) -> str:
    index, name, side = CASES[case]
    path = root / "kernel/src/main.rs"
    before = path.read_text()
    anchor = '    serial::println("IDT_READONLY_READY");'
    access = ('core::arch::asm!("mov r15, rsp", "mov rsp, {floor}", "push 0", "mov rsp, r15", '
              'floor = in(reg) region.usable_start, out("r15") _);'
              if side == "lower" else
              'core::ptr::write_volatile(target as *mut u64, 0);')
    probe = '''
    for (name, region) in arch::stack_regions() {
        serial::print("STACK_GUARD_REGION name=");
        serial::print(name);
        for (label, value) in [(" lower=0x", region.lower_guard), (" usable=0x", region.usable_start),
                               (" top=0x", region.top), (" end=0x", region.allocation_end)] {
            serial::print(label);
            serial::print_hex(value);
        }
        serial::println("");
    }
'''
    probe += f'    let region = arch::stack_regions()[{index}].1;\n'
    probe += ('    let target = region.usable_start - 8;\n' if side == "lower"
              else '    let target = region.top;\n')
    probe += f'    serial::print("STACK_GUARD_PROBE_TARGET stack={name} side={side} address=0x");\n'
    probe += '    serial::print_hex(target); serial::println("");\n'
    probe += ('    // SAFETY: isolated, non-returning fault fixture uses the real production guard.\n'
              f'    unsafe {{ {access} }}\n'
              '    serial::println("STACK_GUARD_PROBE_RETURNED");\n'
              '    arch::halt_loop();\n')
    after = replace_once(before, anchor, anchor + probe)
    path.write_text(after)
    return "".join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile="a/kernel/src/main.rs", tofile="b/kernel/src/main.rs"))


def validate_log(log: str, case: str) -> None:
    _, expected_name, expected_side = CASES[case]
    log = log.replace("\r", "")
    lines = log.splitlines()
    records = REGION.findall(log)
    if len(records) != 7 or {record[0] for record in records} != STACK_SIZES.keys():
        raise ValueError("missing, duplicated or unknown stack geometry")
    regions = {}
    for name, *values in records:
        lower, usable, top, end = (int(value, 16) for value in values)
        if (not lower or any(value % 4096 for value in (lower, usable, top, end))
                or usable != lower + 4096 or top != usable + STACK_SIZES[name]
                or end != top + 4096):
            raise ValueError("invalid guard alignment or usable extent")
        regions[name] = (lower, usable, top, end)
    ordered = sorted(regions.values())
    if any(left[3] > right[0] for left, right in zip(ordered, ordered[1:])):
        raise ValueError("stack allocations overlap")
    targets = TARGET.findall(log)
    if len(targets) != 1 or targets[0][:2] != (expected_name, expected_side):
        raise ValueError("missing, duplicated or wrong guard target")
    lower, usable, top, _ = regions[expected_name]
    expected_address = usable - 8 if expected_side == "lower" else top
    if int(targets[0][2], 16) != expected_address:
        raise ValueError("fault target is outside the selected guard")
    frames = FRAME.findall(log)
    if len(frames) != 1 or sum(line.startswith("EXCEPTION_FRAME") for line in lines) != 1:
        raise ValueError("missing, malformed or repeated exception frame")
    vector, cpl = (int(value) for value in frames[0][:2])
    error, rip, cs, flags, rsp, ss, cr2 = (int(value, 16) for value in frames[0][2:])
    if (vector != 14 or cpl != 0 or error != 2 or cs != 8 or ss != 16
            or not rip or flags & (1 << 9) or cr2 != expected_address):
        raise ValueError("guard did not cause the expected supervisor non-present write fault")
    if expected_side == "lower" and rsp != usable:
        raise ValueError("overflow probe did not fault with RSP at the stack floor")
    required = ["KERNEL_IMAGE_PROTECTED text=rx rodata=r data=rw-nx", READY,
                "EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated", "IDT_READONLY_READY",
                *[match.group(0) for match in REGION.finditer(log)], TARGET.search(log).group(0),
                FRAME.search(log).group(0), "EXCEPTION_FATAL_HALT"]
    if any(lines.count(marker) != 1 for marker in required):
        raise ValueError("missing or duplicated ordered protection/fault proof")
    positions = [lines.index(marker) for marker in required]
    if positions != sorted(positions):
        raise ValueError("guard setup or fatal fault evidence is out of order")
    for forbidden in ("STACK_GUARD_PROBE_RETURNED", "GENOS_READY", "NORMAL_SHELL_READY",
                      "IRQ_HARDWARE_ON", "USER_FAULT_TERMINATED", "KERNEL PANIC",
                      "BOOTLOADER_PANIC", "_FAILED"):
        if any(forbidden in line for line in lines):
            raise ValueError(f"guard failure escaped containment: {forbidden}")


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
                if process.poll() is not None:
                    raise RuntimeError(f"QEMU exited/reset unexpectedly: {process.returncode}")
                if "EXCEPTION_FATAL_HALT" in log:
                    time.sleep(0.5)
                    if process.poll() is not None:
                        raise RuntimeError("QEMU exited/reset after apparent containment")
                    validate_log(serial.read_text(errors="replace"), case)
                    return
                time.sleep(0.05)
            raise TimeoutError(f"no controlled guard fault within {timeout}s")
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
    run = source / "build/stack-guard-evidence" / str(time.time_ns())
    for case in ([options.case] if options.case else CASES):
        evidence = run / case
        evidence.mkdir(parents=True, exist_ok=False)
        manifest = {"case": case, "status": "incomplete", "commit": commit, "source_clean": True}
        try:
            manifest.update(environment(Path(firmware_path()), source))
            with tempfile.TemporaryDirectory(prefix="genos-stack-guard-") as temporary:
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
            manifest.update(status="failed", failure=f"{type(error).__name__}: {error}")
            raise
        finally:
            serial = evidence / "serial.log"
            if serial.exists():
                manifest["serial_sha256"] = hashlib.sha256(serial.read_bytes()).hexdigest()
            (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(f"STACK_GUARD_PROBE_OK case={case} evidence={evidence}", flush=True)


if __name__ == "__main__":
    main()
