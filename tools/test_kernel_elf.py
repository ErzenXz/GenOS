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
import socket
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

MAX_QMP_MESSAGE_BYTES = 64 * 1024
MAX_QMP_TOTAL_BYTES = 1024 * 1024
MAX_QMP_MESSAGES = 256
CAPABILITIES_ID = "genos-capabilities"
START_ID = "genos-start"


class QmpMonitor:
    """Bounded JSON-line reader retaining the exact received/sent wire bytes."""
    def __init__(self, connection, received, sent):
        self.connection = connection
        self.received = received
        self.sent = sent
        self.pending = bytearray()
        self.total = 0
        self.messages = []

    def read(self, deadline: float) -> dict | None:
        while True:
            newline = self.pending.find(b"\n")
            if newline >= 0:
                if newline > MAX_QMP_MESSAGE_BYTES:
                    raise ValueError("QMP message exceeds byte budget")
                line = bytes(self.pending[:newline])
                del self.pending[:newline + 1]
                record = json.loads(line)
                if not isinstance(record, dict):
                    raise ValueError("QMP message must be a JSON object")
                if len(self.messages) >= MAX_QMP_MESSAGES:
                    raise ValueError("QMP event/reply count exceeds budget")
                self.messages.append(record)
                return record
            if len(self.pending) > MAX_QMP_MESSAGE_BYTES:
                raise ValueError("QMP message exceeds byte budget")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("QMP receive deadline expired")
            self.connection.settimeout(min(0.1, remaining))
            try:
                data = self.connection.recv(4096)
            except socket.timeout:
                continue
            if not data:
                if self.pending:
                    raise ValueError("QMP stream ended with a truncated message")
                return None
            self.received.write(data)
            self.received.flush()
            self.total += len(data)
            if self.total > MAX_QMP_TOTAL_BYTES:
                raise ValueError("QMP stream exceeds total byte budget")
            self.pending.extend(data)

    def command(self, execute: str, identity: str, deadline: float) -> None:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("QMP command deadline expired")
        wire = (json.dumps({"execute": execute, "id": identity}) + "\n").encode()
        self.sent.write(wire)
        self.sent.flush()
        self.connection.settimeout(min(1.0, remaining))
        self.connection.sendall(wire)
        while True:
            reply = self.read(deadline)
            if reply is None:
                raise RuntimeError("QMP disconnected before command acknowledgement")
            if "event" in reply:
                continue
            if reply.get("id") != identity or reply.get("return") != {} or "error" in reply:
                raise ValueError(f"QMP rejected or mismatched {execute} acknowledgement")
            return

    def start(self, deadline: float) -> None:
        greeting = self.read(deadline)
        if greeting is None or not isinstance(greeting.get("QMP"), dict):
            raise ValueError("missing QMP greeting")
        self.command("qmp_capabilities", CAPABILITIES_ID, deadline)
        # QEMU starts paused: the monitor is subscribed before any guest code
        # can reject an ELF and shut down, so terminal events cannot be missed.
        self.command("cont", START_ID, deadline)


def connect_qmp(path: Path, process, deadline: float):
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("QEMU exited before QMP connection")
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        connection.settimeout(min(0.1, max(0.001, deadline - time.monotonic())))
        try:
            connection.connect(str(path))
            return connection
        except (FileNotFoundError, ConnectionRefusedError, socket.timeout):
            connection.close()
            time.sleep(min(0.02, max(0, deadline - time.monotonic())))
        except Exception:
            connection.close()
            raise
    raise TimeoutError("QMP connection deadline expired")


def validate_qmp(messages: list[dict]) -> dict:
    if any(not isinstance(message, dict) or sum(key in message for key in
           ("QMP", "return", "event", "error")) != 1 for message in messages):
        raise ValueError("unrecognized or ambiguous QMP message")
    if not messages or not isinstance(messages[0].get("QMP"), dict):
        raise ValueError("missing initial QMP greeting")
    if sum("QMP" in message for message in messages) != 1:
        raise ValueError("duplicated QMP greeting")
    if any("error" in message for message in messages):
        raise ValueError("QMP command error during rejection proof")
    replies = [(index, message) for index, message in enumerate(messages)
               if "return" in message]
    if (len(replies) != 2 or [message.get("id") for _, message in replies]
            != [CAPABILITIES_ID, START_ID] or any(message["return"] != {} for _, message in replies)):
        raise ValueError("missing, duplicated or out-of-order QMP command acknowledgements")
    events = [(index, message) for index, message in enumerate(messages) if "event" in message]
    if any(message["event"] in ("RESET", "GUEST_PANICKED", "WATCHDOG") for _, message in events):
        raise ValueError("QEMU reset, panic or watchdog event during rejection proof")
    resumes = [index for index, message in events if message["event"] == "RESUME"]
    shutdowns = [(index, message) for index, message in events if message["event"] == "SHUTDOWN"]
    if len(resumes) != 1 or len(shutdowns) != 1:
        raise ValueError("missing or duplicated QMP resume/shutdown event")
    at, shutdown = shutdowns[0]
    data = shutdown.get("data")
    if (not isinstance(data, dict) or data.get("guest") is not True
            or data.get("reason") != "guest-shutdown"):
        raise ValueError("QEMU exit was not an explicit guest shutdown")
    if not (replies[0][0] < resumes[0] < at and replies[1][0] < at):
        raise ValueError("QMP shutdown did not follow acknowledged guest execution")
    return data


def collect_shutdown(monitor: QmpMonitor, process, deadline: float) -> dict:
    while time.monotonic() < deadline:
        try:
            record = monitor.read(min(deadline, time.monotonic() + 0.1))
        except TimeoutError:
            record = {}  # Still connected; the overall deadline remains authoritative.
        status = process.poll()
        if record is None and status is None:
            # EOF can precede process reaping. Allow only a short natural-exit
            # grace period; disconnecting a still-running guest is not success.
            try:
                status = process.wait(timeout=min(2, max(0, deadline - time.monotonic())))
            except subprocess.TimeoutExpired as error:
                raise RuntimeError("QMP disconnected while QEMU was still running") from error
        if status is not None:
            # SHUTDOWN can still be buffered when poll first observes exit 0.
            drain_deadline = min(deadline, time.monotonic() + 2)
            while record is not None:
                record = monitor.read(drain_deadline)
            shutdown = validate_qmp(monitor.messages)
            if status != 0:
                raise RuntimeError(f"QEMU exited abnormally after rejection: {status}")
            return shutdown
    raise TimeoutError("no completed loader rejection/shutdown before deadline")


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


def boot(root: Path, evidence: Path, case: str, timeout: int) -> dict:
    serial = evidence / "serial.log"
    # macOS sockaddr_un paths are short. Keep the owned socket in /tmp; all
    # protocol bytes and commands still live in the permanent evidence folder.
    with tempfile.TemporaryDirectory(prefix="genos-qmp-", dir="/tmp") as qmp_dir:
        return boot_with_qmp(root, evidence, case, timeout, serial, Path(qmp_dir) / "monitor")


def boot_with_qmp(root: Path, evidence: Path, case: str, timeout: int,
                  serial: Path, qmp_path: Path) -> dict:
    args = qemu_args(root=root) + [
        "-drive", f"if=pflash,format=raw,readonly=on,file={firmware_path()}",
        "-drive", load_profile(root)["boot_drive"] + ",file=build/genos.img", "-net", "none",
        "-display", "none", "-monitor", "none", "-serial", f"file:{serial}", "-no-reboot",
        "-S", "-qmp", f"unix:{qmp_path},server=on,wait=off"]
    (evidence / "qemu-command.json").write_text(json.dumps(args, indent=2) + "\n")
    with ((evidence / "qemu.log").open("w") as output,
          (evidence / "qmp.log").open("wb") as received,
          (evidence / "qmp-commands.log").open("wb") as sent):
        process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
        connection = None
        try:
            deadline = time.monotonic() + timeout
            connection = connect_qmp(qmp_path, process, deadline)
            monitor = QmpMonitor(connection, received, sent)
            monitor.start(deadline)
            shutdown = collect_shutdown(monitor, process, deadline)
            log = serial.read_text(errors="replace") if serial.exists() else ""
            validate_log(log, case)
            return shutdown
        finally:
            if connection is not None:
                connection.close()
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
                    manifest["qmp_shutdown"] = boot(root, evidence, case, options.timeout)
                    manifest["status"] = "passed"
                except Exception as error:
                    manifest.update(status="failed", failure=f"{type(error).__name__}: {error}")
                    raise
                finally:
                    for name in ("serial.log", "qemu.log", "qmp.log", "qmp-commands.log",
                                 "image.log", "mutation.json"):
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
