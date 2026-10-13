#!/usr/bin/env python3
"""Qualify real firmware lifecycle failures through the pinned UEFI helper.

Only private, checksum-verified vendored sources are patched. Calls still reach
real firmware; the production retry, reset, panic and loader logic is preserved.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib

from reference_vm import environment, load_profile, qemu_args
from test_exception_entry import firmware_path, replace_once, require_clean_source
from test_kernel_elf import QmpMonitor, connect_qmp, CAPABILITIES_ID, START_ID

ROOT = Path(__file__).resolve().parents[1]
CASES = {
    "exit-stale-once": (1, 0, 0, "running"),
    "exit-stale-twice": (2, 0, 0, "guest-reset"),
    "map-grow-small": (3, 0, 1, "running"),
    "map-grow-large": (4, 0, 10, "guest-reset"),
    "alloc-kernel": (5, 1, 0, "guest-shutdown"),
    "alloc-initrd": (5, 2, 0, "guest-shutdown"),
    "alloc-boot-info": (5, 3, 0, "guest-shutdown"),
    "alloc-map-buffer": (5, 4, 0, "panic"),
}
MAP = re.compile(r"FIRMWARE_MAP_RESULT call=(\d+) status=(success|buffer-too-small) size=(\d+) capacity=(\d+)")
GROW = re.compile(r"FIRMWARE_MAP_GROWN pages=(\d+) before=(\d+) capacity=(\d+) stride=(\d+)")
EXIT = re.compile(r"FIRMWARE_EXIT_RESULT attempt=(\d+) status=(success|invalid-parameter)")
DENIED = re.compile(r"FIRMWARE_ALLOCATION_DENIED stage=([1-4]) status=(not-found|out-of-resources)")
STATUS_ID = "genos-firmware-status"


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def replace_file(path: Path, before: str, after: str) -> str:
    original = path.read_text()
    changed = replace_once(original, before, after)
    path.write_text(changed)
    return "".join(difflib.unified_diff(original.splitlines(True), changed.splitlines(True),
        fromfile=str(path.name) + ":before", tofile=str(path.name) + ":fixture"))


def verify_package(package: Path, archive: Path, expected: str) -> None:
    if sha(archive) != expected:
        raise ValueError("pinned UEFI archive checksum mismatch")
    checksums = json.loads((package / ".cargo-checksum.json").read_text())
    if checksums["package"] != expected:
        raise ValueError("vendored package identity differs from Cargo.lock")
    names = set(checksums["files"])
    actual = {str(path.relative_to(package)) for path in package.rglob("*") if path.is_file()}
    if actual != names | {".cargo-checksum.json"} or any(path.is_symlink() for path in package.rglob("*")):
        raise ValueError("unlisted or symlinked vendored source")
    with tarfile.open(archive, "r:gz") as original:
        for name, expected_file in checksums["files"].items():
            path = package / name
            if not path.resolve().is_relative_to(package.resolve()) or sha(path) != expected_file:
                raise ValueError("unverified vendored UEFI file: " + name)
            member = original.extractfile("uefi-0.36.1/" + name)
            if member is None or path.read_bytes() != member.read():
                raise ValueError("vendored UEFI differs from locked package: " + name)


def vendor(root: Path, evidence: Path) -> Path:
    with (evidence / "vendor.log").open("w") as log:
        subprocess.run(["cargo", "vendor", "--offline", "--locked", "--versioned-dirs", "vendor"],
                       cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=120)
    package = root / "vendor/uefi-0.36.1"
    lock = tomllib.loads((root / "Cargo.lock").read_text())
    expected = next(p["checksum"] for p in lock["package"] if p["name"] == "uefi" and p["version"] == "0.36.1")
    cargo = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
    archives = list((cargo / "registry/cache").glob("*/uefi-0.36.1.crate"))
    archive = next((path for path in archives if sha(path) == expected), None)
    if archive is None:
        raise ValueError("pinned UEFI package archive missing or checksum mismatch")
    verify_package(package, archive, expected)
    shutil.copyfile(archive, evidence / "uefi-original.crate")
    config = root / ".cargo/config.toml"
    with config.open("a") as out:
        out.write('\n[source.crates-io]\nreplace-with = "firmware-fixture-vendor"\n'
                  '[source.firmware-fixture-vendor]\ndirectory = "vendor"\n')
    return package


def patch_fixture(root: Path, package: Path, case: str) -> str:
    mode, stage, grow, _ = CASES[case]
    boot = package / "src/boot.rs"
    original = boot.read_text()
    common = (root / "tools/fixtures/firmware_lifecycle.rs").read_text()
    for field, value in [("MODE", mode), ("STAGE", stage), ("GROW_PAGES", grow)]:
        kind = "usize" if field == "GROW_PAGES" else "u8"
        common = replace_once(common, f"const GENOS_FIXTURE_{field}: {kind} = 0;",
                              f"const GENOS_FIXTURE_{field}: {kind} = {value};")
    text = original + "\n" + common
    text = replace_once(text,
        "    unsafe { (bt.allocate_pages)(ty, memory_type, count, &mut addr1) }.to_result()?;",
        """    let injected = genos_fixture_page_failure();
    let requested = if injected { 1usize << 20 } else { count };
    // SAFETY: original firmware ABI and output pointer; only request size changes.
    let status = unsafe { (bt.allocate_pages)(ty, memory_type, requested, &mut addr1) };
    if injected { genos_fixture_allocation_result(status); }
    status.to_result()?;""")
    text = replace_once(text,
        """    let ptr = unsafe { (bt.allocate_pool)(memory_type, size, &mut buffer) }
        .to_result_with_val(|| buffer)?;""",
        """    let injected = genos_fixture_pool_failure();
    let requested = if injected { 1usize << 30 } else { size };
    // SAFETY: original firmware ABI and output pointer; only request size changes.
    let status = unsafe { (bt.allocate_pool)(memory_type, requested, &mut buffer) };
    if injected { genos_fixture_allocation_result(status); }
    let ptr = status.to_result_with_val(|| buffer)?;""")
    # Scope this replacement to get_memory_map; the size-query function is untouched.
    start = text.index("pub(crate) fn get_memory_map(buf:")
    end = text.index("\n/// Creates an event.", start)
    function = text[start:end]
    function = replace_once(function, "    unsafe {\n        (bt.get_memory_map)(", "    let status = unsafe {\n        (bt.get_memory_map)(")
    function = replace_once(function, "    }\n    .to_result_with_val(|| MemoryMapMeta {",
        "    };\n    genos_fixture_map_result(status, map_size, buf.len());\n    status.to_result_with_val(|| MemoryMapMeta {")
    text = text[:start] + function + text[end:]
    text = replace_once(text, "    // Get the memory map.\n    let memory_map = get_memory_map(buf)?;",
        "    genos_fixture_grow(buf);\n    // Get the memory map.\n    let memory_map = get_memory_map(buf)?;")
    text = replace_once(text,
        """    unsafe { (bt.exit_boot_services)(image_handle().as_ptr(), memory_map.map_key.0) }
        .to_result_with_val(|| memory_map)""",
        """    let (key, attempt) = genos_fixture_exit_key(memory_map.map_key.0);
    // SAFETY: the original image/firmware call; only the selected key changes.
    let status = unsafe { (bt.exit_boot_services)(image_handle().as_ptr(), key) };
    genos_fixture_exit_result(status, attempt);
    status.to_result_with_val(|| memory_map)""")
    boot.write_text(text)
    patch = "".join(difflib.unified_diff(original.splitlines(True), text.splitlines(True),
        fromfile="uefi-0.36.1/src/boot.rs", tofile="fixture/uefi-0.36.1/src/boot.rs"))
    if stage:
        if stage == 1:
            path = root / "bootloader/src/elf.rs"
            patch += replace_file(path, "    let destination = boot::allocate_pages(",
                "    boot::genos_fixture_arm_allocation();\n    let destination = boot::allocate_pages(")
        elif stage in (2, 3):
            path = root / "bootloader/src/main.rs"
            anchor = ("    let pages = bytes.len().div_ceil(4096);" if stage == 2
                      else "    let pages = size_of::<BootInfo>().div_ceil(4096);")
            patch += replace_file(path, anchor, "    boot::genos_fixture_arm_allocation();\n" + anchor)
        else:
            path = package / "src/mem/memory_map/impl_.rs"
            patch += replace_file(path, "        let ptr = boot::allocate_pool(memory_type, len)?.as_ptr();",
                "        crate::boot::genos_fixture_arm_allocation();\n        let ptr = boot::allocate_pool(memory_type, len)?.as_ptr();")
    # Deliberate changes are retained as a patch and new per-file hashes. Keep
    # the locked original package receipt separate; never touch registry sources.
    checksum_path = package / ".cargo-checksum.json"
    checksums = json.loads(checksum_path.read_text())
    for name in ("src/boot.rs", "src/mem/memory_map/impl_.rs"):
        checksums["files"][name] = sha(package / name)
    checksum_path.write_text(json.dumps(checksums) + "\n")
    return patch


def validate_log(log: str, case: str) -> None:
    _, stage, pages, outcome = CASES[case]
    proof_lines = {"GenOS kernel entered", "BOOT_MEMORY_MAP_VALIDATED", "GENOS_READY", "BOOTLOADER_PANIC"}
    for line in log.splitlines(keepends=True):
        if ("FIRMWARE_" in line or line.startswith("GenOS boot failed:")
                or line.rstrip("\r\n") in proof_lines) and not line.endswith("\n"):
            raise ValueError("unterminated firmware proof record")
    lines = log.replace("\r", "").splitlines()
    tagged = [line for line in lines if "FIRMWARE_" in line]
    accepted = (MAP, GROW, EXIT, DENIED, re.compile(r"FIRMWARE_ALLOCATION_ARMED stage=[1-4]"))
    if any(not any(pattern.fullmatch(line) for pattern in accepted) for line in tagged):
        raise ValueError("malformed or unexpected firmware evidence")
    maps = [MAP.fullmatch(line) for line in tagged if line.startswith("FIRMWARE_MAP_RESULT")]
    exits = [EXIT.fullmatch(line).groups() for line in tagged if line.startswith("FIRMWARE_EXIT_RESULT")]
    grows = [GROW.fullmatch(line) for line in tagged if line.startswith("FIRMWARE_MAP_GROWN")]
    denied = [DENIED.fullmatch(line).groups() for line in tagged if line.startswith("FIRMWARE_ALLOCATION_DENIED")]
    armed = [line for line in tagged if line.startswith("FIRMWARE_ALLOCATION_ARMED")]
    kinds = [line.split()[0] for line in tagged]
    expected_kinds = (["FIRMWARE_ALLOCATION_ARMED", "FIRMWARE_ALLOCATION_DENIED"] if stage
        else ["FIRMWARE_MAP_RESULT", "FIRMWARE_MAP_GROWN", "FIRMWARE_MAP_RESULT",
              "FIRMWARE_EXIT_RESULT" if pages == 1 else "FIRMWARE_MAP_RESULT"] if pages
        else ["FIRMWARE_MAP_RESULT", "FIRMWARE_EXIT_RESULT", "FIRMWARE_MAP_RESULT", "FIRMWARE_EXIT_RESULT"])
    if kinds != expected_kinds:
        raise ValueError("wrong firmware request/result phases")
    if [int(match[1]) for match in maps] != list(range(1, len(maps) + 1)):
        raise ValueError("missing, duplicated or reordered map call identity")
    for match in maps:
        _, status, size, capacity = match.groups()
        if int(size) <= 0 or int(capacity) <= 0 or (int(size) <= int(capacity)) != (status == "success"):
            raise ValueError("firmware map status/extent mismatch")
    if stage:
        if armed != [f"FIRMWARE_ALLOCATION_ARMED stage={stage}"] or len(denied) != 1 or int(denied[0][0]) != stage:
            raise ValueError("wrong, missing or duplicate allocation denial")
        if maps or exits or grows:
            raise ValueError("allocation failure advanced into firmware exit")
        if lines.index(armed[0]) >= next(i for i, line in enumerate(lines) if DENIED.fullmatch(line)):
            raise ValueError("allocation evidence out of order")
        if stage != 4:
            status_name = "NOT_FOUND" if denied[0][1] == "not-found" else "OUT_OF_RESOURCES"
            marker = f"GenOS boot failed: {status_name}"
            if lines.count(marker) != 1 or log.count("GenOS boot failed:") != 1:
                raise ValueError("allocation denial did not reach loader error handling")
            denial_at = next(i for i, line in enumerate(lines) if DENIED.fullmatch(line))
            if lines.index(marker) <= denial_at:
                raise ValueError("loader error preceded actual allocation denial")
    elif pages:
        if armed or denied or len(grows) != 1 or int(grows[0][1]) != pages:
            raise ValueError("wrong or duplicated map growth")
        before, capacity, stride = map(int, grows[0].groups()[1:])
        if stride < 40 or before > capacity or before % stride or capacity % stride:
            raise ValueError("invalid map growth metadata")
        later = [match for match in maps if lines.index(match[0]) > lines.index(grows[0][0])]
        if len(maps) != (2 if pages == 1 else 3) or maps[0][2] != "success" or int(maps[0][3]) != before:
            raise ValueError("missing or duplicated pre-growth firmware map")
        if not later or any(int(match[3]) <= before for match in later):
            raise ValueError("no actual firmware descriptor growth")
        if case == "map-grow-small":
            if exits != [("1", "success")] or len(later) != 1 or later[0][2] != "success":
                raise ValueError("in-capacity growth did not reach real successful exit")
        elif exits or len(later) != 2 or any(match[2] != "buffer-too-small" for match in later):
            raise ValueError("excess growth did not exhaust exactly two map retries")
    else:
        expected = [("1", "invalid-parameter"), ("2", "success" if case == "exit-stale-once" else "invalid-parameter")]
        if exits != expected or len(maps) != 2 or any(match[2] != "success" for match in maps) or grows or armed or denied:
            raise ValueError("stale-key retry sequence differs from contract")
    if outcome == "running":
        proofs = ["GenOS kernel entered", "BOOT_MEMORY_MAP_VALIDATED", "GENOS_READY"]
        if any(lines.count(proof) != 1 for proof in proofs) or [lines.index(p) for p in proofs] != sorted(lines.index(p) for p in proofs):
            raise ValueError("missing, duplicate or wrong-phase kernel handoff")
        last_exit = max(i for i, line in enumerate(lines) if EXIT.fullmatch(line))
        if last_exit >= lines.index(proofs[0]):
            raise ValueError("kernel entered before actual firmware exit")
    elif any(proof in log for proof in ("GenOS kernel entered", "BOOT_MEMORY_MAP_VALIDATED", "GENOS_READY")):
        raise ValueError("failed firmware lifecycle entered the kernel")
    if lines.count("BOOTLOADER_PANIC") != int(outcome == "panic"):
        raise ValueError("unexpected or missing loader panic")
    if any(word in log for word in ("BOOT_MEMORY_MAP_REJECTED", "KERNEL PANIC", "EXCEPTION_FRAME")):
        raise ValueError("unrelated decoder/kernel fault substituted for firmware outcome")


def validate_qmp(messages: list[dict], outcome: str) -> None:
    if any(not isinstance(row, dict) or sum(key in row for key in ("QMP", "return", "event", "error")) != 1 for row in messages):
        raise ValueError("ambiguous QMP evidence")
    if not messages or "QMP" not in messages[0] or sum("QMP" in row for row in messages) != 1:
        raise ValueError("missing/duplicate QMP greeting")
    if any("error" in row for row in messages):
        raise ValueError("QMP command error")
    replies = [(i, row) for i, row in enumerate(messages) if "return" in row]
    events = [(i, row) for i, row in enumerate(messages) if "event" in row]
    if len(replies) < 2 or [row.get("id") for _, row in replies[:2]] != [CAPABILITIES_ID, START_ID] or any(row["return"] != {} for _, row in replies[:2]):
        raise ValueError("missing or reordered QMP acknowledgements")
    resumes = [i for i, row in events if row["event"] == "RESUME"]
    if len(resumes) != 1 or replies[0][0] >= resumes[0]:
        raise ValueError("missing or reordered guest start")
    if any(row["event"] not in ("RESUME", "SHUTDOWN", "RTC_CHANGE") for _, row in events):
        raise ValueError("unexpected reset/panic/watchdog/paused guest")
    clocks = [(i, row) for i, row in events if row["event"] == "RTC_CHANGE"]
    if len(clocks) > 1:
        raise ValueError("duplicate firmware RTC initialization")
    for position, row in clocks:
        data = row.get("data")
        if (not isinstance(data, dict) or type(data.get("offset")) is not int
                or not isinstance(data.get("qom-path"), str) or not data["qom-path"].startswith("/machine/")
                or not resumes[0] < position < len(messages) - 1):
            raise ValueError("malformed or wrong-phase firmware RTC event")
    shutdowns = [(i, row) for i, row in events if row["event"] == "SHUTDOWN"]
    if outcome in ("running", "panic"):
        if shutdowns or len(replies) != 3 or replies[-1][1].get("id") != STATUS_ID:
            raise ValueError("missing live guest status")
        if replies[-1][0] <= max(resumes[0], replies[1][0]):
            raise ValueError("live status preceded guest execution")
        status = replies[-1][1]["return"]
        if not isinstance(status, dict) or status.get("status") != "running" or status.get("running") is not True:
            raise ValueError("guest is not running after containment")
    else:
        if len(replies) != 2 or len(shutdowns) != 1:
            raise ValueError("missing or duplicated natural guest exit")
        position, row = shutdowns[0]
        if row.get("data", {}).get("guest") is not True or row["data"].get("reason") != outcome:
            raise ValueError("wrong firmware shutdown/reset reason")
        if position <= max(resumes[0], replies[-1][0]):
            raise ValueError("terminal event preceded guest execution")


def boot(root: Path, evidence: Path, case: str, timeout: int) -> None:
    outcome = CASES[case][3]
    serial = evidence / "serial.log"
    qmp_path = root / "build/firmware-qmp.sock"
    args = qemu_args(root=root) + ["-drive", f"if=pflash,format=raw,readonly=on,file={firmware_path()}",
        "-drive", load_profile(root)["boot_drive"] + ",file=build/genos.img", "-net", "none",
        "-display", "none", "-monitor", "none", "-serial", f"file:{serial}", "-no-reboot", "-S",
        "-qmp", f"unix:{qmp_path},server=on,wait=off"]
    (evidence / "qemu-command.json").write_text(json.dumps(args) + "\n")
    with (evidence / "qemu.log").open("w") as output, (evidence / "qmp-received.jsonl").open("wb") as received, (evidence / "qmp-sent.jsonl").open("wb") as sent:
        process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + timeout
            with connect_qmp(qmp_path, process, deadline) as connection:
                monitor = QmpMonitor(connection, received, sent)
                monitor.start(deadline)
                while time.monotonic() < deadline:
                    log = serial.read_text(errors="replace") if serial.exists() else ""
                    marker = ("GENOS_READY" if outcome == "running" else "BOOTLOADER_PANIC") + "\n"
                    if outcome in ("running", "panic") and marker in log.replace("\r", ""):
                        validate_log(log, case)
                        wire = (json.dumps({"execute": "query-status", "id": STATUS_ID}) + "\n").encode()
                        sent.write(wire); sent.flush(); connection.sendall(wire)
                        while True:
                            row = monitor.read(deadline)
                            if row is None: raise RuntimeError("guest disconnected before status proof")
                            if row.get("id") == STATUS_ID: break
                        validate_qmp(monitor.messages, outcome)
                        if process.poll() is not None: raise RuntimeError("guest exited after running proof")
                        return
                    try:
                        row = monitor.read(min(deadline, time.monotonic() + .1))
                    except TimeoutError:
                        row = {}
                    if row is None or process.poll() is not None:
                        process.wait(timeout=2)
                        if process.returncode != 0: raise RuntimeError(f"QEMU exited {process.returncode}")
                        while row is not None:
                            row = monitor.read(deadline)
                        validate_log(serial.read_text(errors="replace"), case)
                        validate_qmp(monitor.messages, outcome)
                        return
                raise TimeoutError("firmware lifecycle proof deadline")
        finally:
            if process.poll() is None:
                process.terminate()
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired: process.kill(); process.wait()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=CASES)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    if sys.version_info < (3, 12) or args.timeout <= 0: parser.error("Python 3.12+ and positive timeout required")
    commit = require_clean_source(ROOT)
    archive = subprocess.check_output(["git", "archive", "--format=tar", commit], cwd=ROOT)
    run = ROOT / "build/firmware-lifecycle-evidence" / str(time.time_ns())
    run.mkdir(parents=True)
    (run / "source.tar").write_bytes(archive)
    failed = []
    for case in ([args.case] if args.case else CASES):
        evidence = run / case
        evidence.mkdir()
        manifest = {"case": case, "status": "incomplete", "commit": commit, "source_clean": True,
                    "source_archive_sha256": hashlib.sha256(archive).hexdigest(), "outcome": CASES[case][3]}
        try:
            manifest.update(environment(Path(firmware_path()), ROOT))
            with tempfile.TemporaryDirectory(prefix="genos-firmware-") as temporary:
                root = Path(temporary) / "source"; root.mkdir()
                with tarfile.open(fileobj=io.BytesIO(archive)) as tree: tree.extractall(root, filter="data")
                package = vendor(root, evidence)
                patch = patch_fixture(root, package, case)
                (evidence / "fixture.patch").write_text(patch)
                manifest["fixture_sha256"] = hashlib.sha256(patch.encode()).hexdigest()
                manifest["original_package_sha256"] = sha(evidence / "uefi-original.crate")
                for name in ("src/boot.rs", "src/mem/memory_map/impl_.rs", ".cargo-checksum.json"):
                    target = evidence / "uefi-fixture" / name; target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(package / name, target)
                with (evidence / "build.log").open("w") as log:
                    subprocess.run(["cargo", "xtask", "build"], cwd=root, stdout=log,
                                   stderr=subprocess.STDOUT, check=True, timeout=600)
                manifest["image_sha256"] = sha(root / "build/genos.img")
                boot(root, evidence, case, args.timeout)
                manifest["status"] = "passed"
        except Exception as error:
            manifest.update(status="failed", failure=f"{type(error).__name__}: {error}")
            failed.append(case)
        finally:
            manifest["artifacts_sha256"] = {str(path.relative_to(evidence)): sha(path)
                for path in sorted(evidence.rglob("*")) if path.is_file() and path.name != "manifest.json"}
            (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(f"FIRMWARE_LIFECYCLE_{manifest['status'].upper()} case={case} evidence={evidence}", flush=True)
    if failed: raise RuntimeError(f"firmware lifecycle failures: {failed}")


if __name__ == "__main__":
    main()
