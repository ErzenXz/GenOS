#!/usr/bin/env python3
"""Prove BSP-only boot and rejected CPU/table reentry in isolated fixtures.

Requires committed source. Evidence records the commit, exact fixture patch,
image hash, tools, command and serial output. Non-BSP injection clears the BSP
bit in the sampled MSR value; it does not implement or test an AP trampoline.
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

from reference_vm import environment, load_profile, qemu_args

from test_exception_entry import command_output, firmware_path, replace_once, require_clean_source

CASES = ("single", "quad", "repeat-entry", "repeat-init", "non-bsp")
ENTRY = 'pub extern "sysv64" fn _start(boot_info: &\'static BootInfo) -> ! {'
ENTRY_REJECTION = "    if !arch::claim_boot_cpu() {\n        arch::halt_loop();\n    }"
TABLE_REJECTION = "    if !BOOT_CLAIM.begin_table_init(features, apic_base) {\n        halt_loop();\n    }"
IDENTITY_SAMPLE = "    (features, Some((u64::from(high) << 32) | u64::from(low)))"
TOPOLOGY = re.compile(
    r"^SMP_DISABLED policy=bsp-only active_cpus=1 initial_apic_id=(\d+)"
    r" cpuid_max_logical_per_package=(\d+)$", re.MULTILINE)


def patch_fixture(root: Path, case: str) -> str:
    if case in {"single", "quad"}:
        return ""
    main_path = root / "kernel/src/main.rs"
    before = main_path.read_text()
    after = before
    changes = []
    if case in {"repeat-entry", "non-bsp"}:
        after = replace_once(after, ENTRY_REJECTION, ENTRY_REJECTION.replace(
            "        arch::halt_loop();",
            '        serial::println("BSP_PROBE_REJECTED");\n        arch::halt_loop();'))
    if case == "repeat-entry":
        anchor = '    serial::println("GenOS kernel entered");'
        after = replace_once(after, anchor, anchor + '\n'
            '    serial::println("BSP_PROBE_ARMED case=repeat-entry");\n'
            '    _start(boot_info);')
    elif case == "repeat-init":
        after = replace_once(after, "    arch::init();", '    arch::init();\n'
            '    serial::println("BSP_PROBE_ARMED case=repeat-init");\n'
            '    arch::init();\n'
            '    serial::println("BSP_PROBE_RETURNED");\n'
            '    arch::halt_loop();')
        path = root / "kernel/src/arch.rs"
        original = path.read_text()
        modified = replace_once(original, TABLE_REJECTION, TABLE_REJECTION.replace(
            "        halt_loop();",
            '        crate::serial::println("BSP_PROBE_REJECTED");\n        halt_loop();'))
        changes.append((path, original, modified))
    elif case == "non-bsp":
        # QEMU TCG preserves its BSP flag on WRMSR. Inject only the role bit
        # in the sampled value; do not pretend this is a real AP startup.
        injection = '''
    arch::disable_interrupts();
    serial::init();
    serial::println("BSP_PROBE_ARMED case=non-bsp");'''
        after = replace_once(after, ENTRY, ENTRY + injection)
        path = root / "kernel/src/arch.rs"
        original = path.read_text()
        modified = replace_once(original, IDENTITY_SAMPLE,
            '    crate::serial::println("BSP_PROBE_NON_BSP_INJECTED");\n'
            '    (features, Some(((u64::from(high) << 32) | u64::from(low)) & !(1 << 8)))')
        changes.append((path, original, modified))
    else:
        raise ValueError(f"unknown BSP case: {case}")
    changes.append((main_path, before, after))
    patches = []
    for path, original, modified in changes:
        path.write_text(modified)
        relative = path.relative_to(root)
        patches.extend(difflib.unified_diff(original.splitlines(True), modified.splitlines(True),
                                           fromfile=f"a/{relative}", tofile=f"b/{relative}"))
    return "".join(patches)


def validate_log(log: str, case: str) -> None:
    log = log.replace("\r", "")
    lines = log.splitlines()
    for forbidden in ("KERNEL PANIC", "EXCEPTION_FRAME", "EXCEPTION_FATAL_HALT", "BSP_PROBE_RETURNED"):
        if any(forbidden in line for line in lines):
            raise ValueError(f"unexpected fault or probe outcome: {forbidden}")
    if case in {"single", "quad"}:
        required = ("GenOS kernel entered", "GDT/TSS initialized", "IDT initialized",
                    "NORMAL_SHELL_READY", "GENOS_READY")
        if any(lines.count(marker) != 1 for marker in required):
            raise ValueError("normal BSP boot must enter, initialize and launch exactly once")
        topology = TOPOLOGY.findall(log)
        # CPUID.1's package-capacity field is valid only when HTT is set.
        # The pinned qemu64 single-vCPU model clears HTT, so the kernel reports
        # zero (unavailable), not an active-CPU count. Four-vCPU evidence still
        # requires the topology-capacity diagnostic and one admitted kernel CPU.
        capacities = (4,) if case == "quad" else (0, 1)
        if len(topology) != 1 or int(topology[0][1]) not in capacities:
            raise ValueError("missing, duplicate or incorrect BSP-only topology diagnostic")
        if any("BSP_PROBE_" in line for line in lines):
            raise ValueError("normal image contains validation probe output")
        order = ("GenOS kernel entered", "GDT/TSS initialized", "IDT initialized",
                 "GENOS_READY", "NORMAL_SHELL_READY")
        if [lines.index(marker) for marker in order] != sorted(lines.index(marker) for marker in order):
            raise ValueError("normal CPU initialization and shell readiness are out of order")
        return
    if lines.count(f"BSP_PROBE_ARMED case={case}") != 1 or lines.count("BSP_PROBE_REJECTED") != 1:
        raise ValueError("missing exact rejection branch evidence")
    if lines.index(f"BSP_PROBE_ARMED case={case}") >= lines.index("BSP_PROBE_REJECTED"):
        raise ValueError("CPU rejection appeared before the probe was armed")
    if any(marker in lines for marker in ("NORMAL_SHELL_READY", "GENOS_READY")):
        raise ValueError("rejected entrant continued normal boot")
    expected_entries = 0 if case == "non-bsp" else 1
    expected_tables = 1 if case == "repeat-init" else 0
    if lines.count("GenOS kernel entered") != expected_entries:
        raise ValueError("a rejected entrant touched kernel initialization")
    if (lines.count("GDT/TSS initialized") != expected_tables
            or lines.count("IDT initialized") != expected_tables
            or len(TOPOLOGY.findall(log)) != expected_tables):
        raise ValueError("a rejected entrant touched shared architecture tables")
    if case == "non-bsp" and lines.count("BSP_PROBE_NON_BSP_INJECTED") != 1:
        raise ValueError("non-BSP identity sample was not injected")


def boot(root: Path, evidence: Path, case: str, timeout: int) -> None:
    log_path = evidence / "serial.log"
    cpus = 1 if case == "single" else 4
    profile = load_profile(root)
    args = qemu_args(cpus=cpus, root=root) + [
            "-drive", f"if=pflash,format=raw,readonly=on,file={firmware_path()}",
            "-drive", profile["boot_drive"] + ",file=build/genos.img", "-net", "none",
            "-display", "none", "-monitor", "none", "-serial", f"file:{log_path}",
            "-no-reboot"]
    (evidence / "qemu-command.json").write_text(json.dumps(args, indent=2) + "\n")
    marker = "NORMAL_SHELL_READY" if case in {"single", "quad"} else "BSP_PROBE_REJECTED"
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
                        raise RuntimeError("QEMU reset/exited after the apparent success marker")
                    validate_log(log_path.read_text(errors="replace"), case)
                    return
                time.sleep(0.05)
            raise TimeoutError(f"no {marker} within {timeout}s; see {log_path}")
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
    if sys.version_info < (3, 12):
        parser.error("Python 3.12 or newer is required for filtered source-archive extraction")
    if options.timeout <= 0:
        parser.error("timeout must be positive")
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    # Archive the immutable commit once: later working-tree edits cannot be
    # silently attributed to this manifest's earlier source identity.
    archive = subprocess.run(["git", "archive", "--format=tar", commit], cwd=source,
                             stdout=subprocess.PIPE, check=True, timeout=30).stdout
    run = source / "build/bsp-evidence" / str(time.time_ns())
    for case in ([options.case] if options.case else CASES):
        evidence = run / case
        evidence.mkdir(parents=True, exist_ok=False)
        manifest = {"case": case, "status": "incomplete", "commit": commit, "source_clean": True}
        try:
            manifest.update(environment(firmware_path(), source))
            manifest.update({"rust": command_output(["rustc", "-Vv"], source),
                             "qemu": command_output(["qemu-system-x86_64", "--version"], source)})
            with tempfile.TemporaryDirectory(prefix="genos-bsp-") as temporary:
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
            for name in ("serial.log", "qemu.log", "build.log"):
                artifact = evidence / name
                if artifact.is_file():
                    manifest[name + "_sha256"] = hashlib.sha256(artifact.read_bytes()).hexdigest()
            (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(f"BSP_PROBE_OK case={case} evidence={evidence}", flush=True)


if __name__ == "__main__":
    main()
