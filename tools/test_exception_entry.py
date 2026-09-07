#!/usr/bin/env python3
"""Run real CPU exception probes in a disposable, auditable source fixture.

The fixture changes only the deliberate probe instruction and its expected
result. Production exception dispatch and process cleanup are never patched.
Run from a full checkout with the repository's normal build dependencies.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

VECTORS = {"de": 0, "ud": 6, "gp": 13, "pf": 14}
FRAME = re.compile(
    r"^EXCEPTION_FRAME vector=(\d+) cpl=(\d+)"
    r" error=0x([0-9a-fA-F]+) rip=0x([0-9a-fA-F]+)"
    r" cs=0x([0-9a-fA-F]+) rflags=0x([0-9a-fA-F]+)"
    r" rsp=0x([0-9a-fA-F]+) ss=0x([0-9a-fA-F]+)"
    r" cr2=0x([0-9a-fA-F]+)$", re.MULTILINE,
)
GUARD_WRITE = "write_volatile(runtime::STACK_GUARD as *mut u64, token);"
USER_READY = ("USER_RECLAIM_OK", "USER_PREEMPT_OK", "USER_FAULT_ISOLATED",
              "USER_ISOLATION_OK", "USERMODE_READY")


def replace_once(text: str, old: str, new: str) -> str:
    if text.count(old) != 1:
        raise ValueError(f"fixture anchor must occur exactly once: {old!r}")
    return text.replace(old, new, 1)


def instruction(fault: str, mode: str) -> str:
    if fault == "de":
        return ('core::arch::asm!("xor eax, eax", "xor edx, edx", "div rax", '
                'out("rax") _, out("rdx") _, options(nostack));')
    if fault == "ud":
        # Do not mark noreturn: the probe's failure sentinel must remain compiled.
        return 'core::arch::asm!("ud2", options(nostack));'
    if fault == "gp":
        if mode == "user":
            return 'core::arch::asm!("cli", options(nostack));'
        return ('core::arch::asm!("mov ax, 0x38", "mov ds, ax", '
                'out("rax") _, options(nostack));')
    if mode == "user":
        return GUARD_WRITE
    return ('core::arch::asm!("mov rax, 0x00007ffffffff000", "mov [rax], rax", '
            'out("rax") _, options(nostack));')


def patch_fixture(root: Path, mode: str, fault: str) -> str:
    vector = VECTORS[fault]
    changes: dict[str, list[tuple[str, str]]] = {}
    if mode == "user" and fault != "pf":
        changes["userspace/init/src/main.rs"] = [(GUARD_WRITE, instruction(fault, mode))]
        changes["kernel/src/userspace.rs"] = [
            ("const FAULT_EXIT_CODE: u8 = 128 + 14;", f"const FAULT_EXIT_CODE: u8 = 128 + {vector};"),
            ("faulting.fault_vector == 14", f"faulting.fault_vector == {vector}"),
            ("faulting.fault_error == 0x6", "faulting.fault_error == 0"),
            ("faulting.fault_address == paging::USER_STACK_GUARD", "faulting.fault_address == 0"),
        ]
    if mode == "kernel":
        probe = ('    interrupts::init();\n'
                 '    serial::println("KERNEL_EXCEPTION_PROBE_ARMED");\n'
                 '    // SAFETY: isolated validation fixture deliberately faults the CPU.\n'
                 f'    unsafe {{ {instruction(fault, mode)} }}\n'
                 '    serial::println("KERNEL_EXCEPTION_PROBE_RETURNED");\n'
                 '    arch::halt_loop();')
        changes["kernel/src/main.rs"] = [("    interrupts::init();", probe)]
    patches = []
    for relative, replacements in changes.items():
        path = root / relative
        before = path.read_text()
        after = before
        for old, new in replacements:
            after = replace_once(after, old, new)
        path.write_text(after)
        patches.extend(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                                           fromfile=f"a/{relative}", tofile=f"b/{relative}"))
    return "".join(patches)


def validate_log(log: str, mode: str, fault: str) -> None:
    log = log.replace("\r", "")
    frames = FRAME.findall(log)
    if len(frames) != 1:
        raise ValueError(f"expected exactly one complete exception frame, found {len(frames)}")
    vector, cpl = map(int, frames[0][:2])
    error, rip, cs, flags, rsp, ss, cr2 = (int(value, 16) for value in frames[0][2:])
    expected_cpl = 3 if mode == "user" else 0
    if (vector, cpl) != (VECTORS[fault], expected_cpl):
        raise ValueError(f"wrong fault identity: vector={vector}, cpl={cpl}")
    if cs & 3 != expected_cpl or ss & 3 != expected_cpl or not rip or not rsp or not flags & 2:
        raise ValueError("invalid saved privilege, instruction, stack or reserved flags bit")
    expected_error = {"de": 0, "ud": 0, "gp": 0 if mode == "user" else 0x38,
                      "pf": 6 if mode == "user" else 2}[fault]
    if error != expected_error or (fault != "pf" and cr2 != 0):
        raise ValueError("wrong normalized error code or fault address policy")
    if fault == "pf" and cr2 == 0:
        raise ValueError("page fault did not retain its nonzero fault address")
    lines = log.splitlines()
    if "EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated" not in lines:
        raise ValueError("normalized entry was not installed")
    if mode == "user":
        if any(marker not in lines for marker in USER_READY) or "EXCEPTION_FATAL_HALT" in lines:
            raise ValueError("fault isolation, healthy-peer progress or reclamation proof missing")
        if not re.search(rf"^USER_FAULT_TERMINATED pid=1 vector={vector} ", log, re.MULTILINE):
            raise ValueError("the deliberate fault did not terminate the exact probe process")
    elif (lines.count("EXCEPTION_FATAL_HALT") != 1
          or "KERNEL_EXCEPTION_PROBE_ARMED" not in lines
          or "KERNEL_EXCEPTION_PROBE_RETURNED" in lines
          or "GENOS_READY" in lines):
        raise ValueError("kernel exception returned, continued boot, or failed to halt explicitly")


def firmware_path() -> Path:
    override = os.environ.get("OVMF_CODE")
    candidates = ([Path(override)] if override else []) + [
        Path("/usr/share/OVMF/OVMF_CODE.fd"), Path("/usr/share/OVMF/OVMF_CODE_4M.fd"),
        Path("/usr/share/edk2/ovmf/OVMF_CODE.fd"),
    ]
    for path in candidates:
        if path.is_file():
            return path
    raise FileNotFoundError("install OVMF or set OVMF_CODE to its code firmware file")


def boot(root: Path, evidence: Path, mode: str, fault: str, timeout: int) -> list[str]:
    log_path = evidence / "serial.log"
    args = ["qemu-system-x86_64", "-machine", "q35", "-m", "512M", "-smp", "1",
            "-drive", f"if=pflash,format=raw,readonly=on,file={firmware_path()}",
            "-drive", "format=raw,file=build/genos.img", "-net", "none",
            "-display", "none", "-monitor", "none", "-serial", f"file:{log_path}",
            "-no-reboot"]
    (evidence / "qemu-command.json").write_text(json.dumps(args, indent=2) + "\n")
    with (evidence / "qemu.log").open("w") as output:
        process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + timeout
            marker = "USERMODE_READY" if mode == "user" else "EXCEPTION_FATAL_HALT"
            while time.monotonic() < deadline:
                log = log_path.read_text(errors="replace") if log_path.exists() else ""
                if process.poll() is not None:
                    raise RuntimeError(f"QEMU exited unexpectedly: {process.returncode}")
                if marker in log:
                    # A double/triple fault must not count as a deliberate halt.
                    # -no-reboot makes a reset observable as process exit.
                    time.sleep(0.5)
                    if process.poll() is not None:
                        raise RuntimeError("QEMU reset/exited after the apparent success marker")
                    validate_log(log_path.read_text(errors="replace"), mode, fault)
                    return args
                time.sleep(0.05)
            raise TimeoutError(f"no {marker} within {timeout}s; see {log_path}")
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def command_output(args: list[str], cwd: Path) -> str:
    result = subprocess.run(args, cwd=cwd, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, check=True, timeout=30)
    return result.stdout.strip()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("user", "kernel"), required=True)
    parser.add_argument("--fault", choices=VECTORS, required=True)
    parser.add_argument("--timeout", type=int, default=120)
    options = parser.parse_args()
    source = Path(__file__).resolve().parents[1]
    evidence = source / "build" / "exception-evidence" / f"{options.mode}-{options.fault}"
    evidence.mkdir(parents=True, exist_ok=False)
    manifest = {"mode": options.mode, "fault": options.fault, "status": "incomplete"}
    try:
        manifest.update({"commit": command_output(["git", "rev-parse", "HEAD"], source),
                         "rust": command_output(["rustc", "-Vv"], source),
                         "qemu": command_output(["qemu-system-x86_64", "--version"], source)})
        with tempfile.TemporaryDirectory(prefix="genos-exception-") as temporary:
            root = Path(temporary) / "source"
            shutil.copytree(source, root, ignore=shutil.ignore_patterns(
                ".git", "target", "build", "__pycache__"))
            patch = patch_fixture(root, options.mode, options.fault)
            (evidence / "fixture.patch").write_text(patch)
            manifest["fixture_sha256"] = hashlib.sha256(patch.encode()).hexdigest()
            with (evidence / "build.log").open("w") as output:
                subprocess.run(["cargo", "xtask", "build"], cwd=root, stdout=output,
                               stderr=subprocess.STDOUT, check=True, timeout=600)
            image = root / "build/genos.img"
            manifest["image_sha256"] = hashlib.sha256(image.read_bytes()).hexdigest()
            boot(root, evidence, options.mode, options.fault, options.timeout)
            manifest["status"] = "passed"
    finally:
        (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"EXCEPTION_PROBE_OK mode={options.mode} fault={options.fault}")


if __name__ == "__main__":
    main()
