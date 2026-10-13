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

LOADER_ERRORS = {
    "loader-stride": "DescriptorStride",
    "loader-length": "DescriptorLength",
    "loader-empty": "Empty",
    "loader-capacity": "Capacity",
    "loader-range": "Range",
    "loader-alignment": "Alignment",
    "loader-overlap": "Overlap",
}
LOADER_CASES = ("loader-version", *LOADER_ERRORS)
CASES = (*LOADER_CASES, "count", "overlap", "initrd-usable", "kernel-usable", "cmdline")


def loader_error_record(case: str) -> str:
    return f"BOOT_MEMORY_FIXTURE_ERROR case={case} error={LOADER_ERRORS[case]}"


def patch_fixture(root: Path, case: str) -> str:
    path = root / "bootloader/src/main.rs"
    before = path.read_text()
    if case == "loader-version":
        after = replace_once(before, "meta.desc_size, meta.desc_version", "meta.desc_size, 0xffff")
    elif case in LOADER_ERRORS:
        # Bounded stack-backed bytes replace only decoder inputs. No allocation,
        # firmware call or boot service can occur after ExitBootServices.
        length = {"loader-length": 39, "loader-empty": 0,
                  "loader-capacity": 40 * 257, "loader-overlap": 80}.get(case, 40)
        setup = f"    let mut fixture = [0u8; {length}];\n"
        if case in ("loader-range", "loader-alignment", "loader-overlap"):
            start = 1 if case == "loader-alignment" else 4096
            pages = "u64::MAX" if case == "loader-range" else "1u64"
            setup += (
                "    fixture[..4].copy_from_slice(&7u32.to_le_bytes());\n"
                f"    fixture[8..16].copy_from_slice(&{start}u64.to_le_bytes());\n"
                f"    fixture[24..32].copy_from_slice(&{pages}.to_le_bytes());\n"
            )
            if case == "loader-overlap":
                setup += "    let first = fixture[..40].try_into().unwrap();\n    let first: [u8; 40] = first;\n    fixture[40..].copy_from_slice(&first);\n"
        else:
            setup = setup.replace("let mut fixture", "let fixture")
        stride = 39 if case == "loader-stride" else 40
        expected = LOADER_ERRORS[case]
        anchor = "    boot_info.memory_map = boot_memory::decode_uefi_map(bytes, meta.desc_size, meta.desc_version)?;"
        replacement = (
            "    let _ = bytes;\n" + setup +
            f"    let result = boot_memory::decode_uefi_map(&fixture, {stride}, 1);\n"
            f"    if result.as_ref().err() != Some(&boot_memory::BootMemoryError::{expected}) {{\n"
            "        halt_after_exit(b\"BOOT_MEMORY_FIXTURE_WRONG_ERROR\\r\\n\");\n"
            "    }\n    boot_info.memory_map = result?;"
        )
        after = replace_once(before, anchor, replacement)
        after = replace_once(after,
            ".ok_or(boot_memory::BootMemoryError::DescriptorLength)?;",
            '.unwrap_or_else(|| halt_after_exit(b"BOOT_MEMORY_FIXTURE_INPUT_REJECTED\\r\\n"));')
        rejection = 'halt_after_exit(b"BOOT_MEMORY_MAP_REJECTED\\r\\n");'
        tagged = f'halt_after_exit(b"{loader_error_record(case)}\\r\\nBOOT_MEMORY_MAP_REJECTED\\r\\n");'
        after = replace_once(after, rejection, tagged)
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
    if case not in CASES:
        raise ValueError(f"unknown boot memory case: {case}")
    lines = log.replace("\r", "").splitlines()
    marker = "BOOT_MEMORY_MAP_REJECTED" if case in LOADER_CASES else "BOOT_INFO_REJECTED"
    if lines.count(marker) != 1:
        raise ValueError("missing or duplicated exact boot rejection")
    if lines.count("GenOS kernel entered") != (0 if case in LOADER_CASES else 1):
        raise ValueError("wrong rejection phase")
    records = [line for line in lines if "BOOT_MEMORY_FIXTURE_ERROR" in line]
    expected = [loader_error_record(case)] if case in LOADER_ERRORS else []
    if records != expected or (expected and lines.index(expected[0]) >= lines.index(marker)):
        raise ValueError("missing, duplicated, wrong-case or reordered decoder error")
    for forbidden in ("BOOT_MEMORY_MAP_VALIDATED", "FRAME_ALLOCATOR_BITMAP_READY",
                      "GDT/TSS initialized", "GENOS_READY", "NORMAL_SHELL_READY",
                      "EXCEPTION_FRAME", "KERNEL PANIC", "BOOTLOADER_PANIC",
                      "BOOT_MEMORY_FIXTURE_WRONG_ERROR", "BOOT_MEMORY_FIXTURE_INPUT_REJECTED"):
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
    marker = "BOOT_MEMORY_MAP_REJECTED" if case in LOADER_CASES else "BOOT_INFO_REJECTED"
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
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--case", choices=CASES)
    selection.add_argument("--loader-admission", action="store_true",
                           help="run every loader decoder case, retaining all failures")
    parser.add_argument("--timeout", type=int, default=120)
    options = parser.parse_args()
    if sys.version_info < (3, 12) or options.timeout <= 0:
        parser.error("Python >=3.12 and positive timeout required")
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.run(["git", "archive", "--format=tar", commit], cwd=source,
                             stdout=subprocess.PIPE, check=True, timeout=30).stdout
    run = source / "build/boot-memory-evidence" / str(time.time_ns())
    cases = [options.case] if options.case else LOADER_CASES if options.loader_admission else CASES
    failures = []
    for case in cases:
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
            failures.append((case, str(error)))
        finally:
            serial = evidence / "serial.log"
            if serial.exists():
                manifest["serial_sha256"] = hashlib.sha256(serial.read_bytes()).hexdigest()
            (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        outcome = "OK" if manifest["status"] == "passed" else "FAILED"
        print(f"BOOT_MEMORY_PROBE_{outcome} case={case} evidence={evidence}", flush=True)
    if failures:
        raise RuntimeError(f"boot memory qualification failed: {failures}")


if __name__ == "__main__":
    main()
