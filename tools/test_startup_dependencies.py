#!/usr/bin/env python3
"""Prove a normal Ring 3 terminal survives absent optional storage and NICs."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from queue import Empty, Queue
import subprocess
import threading
import time

from reference_vm import environment, load_profile, qemu_args
from test_exception_entry import firmware_path


ROOT = Path(__file__).resolve().parents[1]
CASES = ("no-controller", "empty-controller", "nic-no-server")
FAILURE = ("KERNEL PANIC", "EXCEPTION_FATAL_HALT", "RECOVERY_CONSOLE_READY", "_FAILED")
FORBIDDEN = ("BOOT_MODE validation", "USER_CONSOLE_WRITE", "USER_INPUT_", "CONSOLE_TRANSCRIPT_READY")


class StartupProof:
    def __init__(self, nonce: str, network_reply: str = "network unavailable"):
        self.policy = False
        self.storage_unavailable = False
        self.kernel_ready = False
        self.shell_ready = False
        self.commands = ((f"echo {nonce}", nonce), ("net", network_reply),
                         ("mem", "consistent=yes"))
        self.step = 0
        self.command_seen = False
        self.memory_seen = False

    @property
    def complete(self) -> bool:
        return self.shell_ready and self.step == len(self.commands)

    def observe(self, line: str) -> bytes | None:
        if any(marker in line for marker in FAILURE + FORBIDDEN):
            raise ValueError(f"failed or validation-only normal boot record: {line}")
        if line == "BOOT_MODE normal":
            if self.policy:
                raise ValueError("duplicate normal policy")
            self.policy = True
        elif line == "PERSISTENT_STORAGE_UNAVAILABLE":
            if not self.policy or self.storage_unavailable or self.kernel_ready:
                raise ValueError("unavailable storage outside startup")
            self.storage_unavailable = True
        elif line == "GENOS_READY":
            if not self.policy or not self.storage_unavailable or self.kernel_ready:
                raise ValueError("invalid kernel readiness")
            self.kernel_ready = True
        elif line == "NORMAL_SHELL_READY":
            if not self.kernel_ready or self.shell_ready:
                raise ValueError("invalid Ring 3 shell readiness")
            self.shell_ready = True
            return (self.commands[0][0] + "\r").encode()
        if not self.shell_ready or self.complete:
            return None
        command, response = self.commands[self.step]
        if line == f"genos> {command}":
            if self.command_seen:
                raise ValueError("duplicate command echo")
            self.command_seen = True
        elif self.step == 2 and self.command_seen and line.startswith("frames_total="):
            if self.memory_seen:
                raise ValueError("duplicate memory counters")
            fields = line.split()
            if len(fields) != 3 or not all(
                field.startswith(prefix) and field[len(prefix):].isdigit()
                for field, prefix in zip(fields, ("frames_total=", "live=", "free="))
            ):
                raise ValueError("malformed memory counters")
            total, live, free = (int(field.split("=", 1)[1]) for field in fields)
            if total != live + free:
                raise ValueError("inconsistent memory counters")
            self.memory_seen = True
        elif line == response:
            if not self.command_seen or (self.step == 2 and not self.memory_seen):
                raise ValueError("reply before its command or counters")
            self.step += 1
            self.command_seen = False
            if not self.complete:
                return (self.commands[self.step][0] + "\r").encode()
        return None


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_case(case: str, timeout: int) -> Path:
    if case not in CASES or timeout < 1 or timeout > 120:
        raise ValueError("invalid case or timeout")
    profile = load_profile(ROOT)
    firmware = firmware_path()
    run_id = str(time.time_ns())
    directory = ROOT / "build/startup-dependency-evidence" / run_id / case
    directory.mkdir(parents=True)
    serial_path = directory / "serial.log"
    qemu_path = directory / "qemu.log"
    manifest = directory / "manifest.json"
    image = ROOT / "build/genos.img"
    if not image.is_file():
        raise FileNotFoundError("build a normal image with cargo xtask build first")
    if (ROOT / "build/image-mode.txt").read_text().splitlines()[0] not in ("Normal", "Release"):
        raise ValueError("startup dependency proof requires a normal image")
    reference = environment(firmware, ROOT)
    if not reference["reference_environment_match"]:
        raise ValueError(f"reference environment mismatch: {reference['reference_mismatches']}")
    args = qemu_args(root=ROOT) + [
        "-drive", f"if=pflash,format=raw,readonly=on,file={firmware}",
        "-drive", profile["boot_drive"] + ",file=build/genos.img",
    ]
    if case == "empty-controller":
        args += ["-device", profile["storage_controller"]]
    if case == "nic-no-server":
        args += ["-netdev", profile["network_backend"],
                 "-device", profile["network_device"]]
    args += ["-net", "none", "-display", "none", "-monitor", "none",
             "-serial", "stdio", "-no-reboot"]
    (directory / "qemu-command.json").write_text(json.dumps(args, indent=2) + "\n")
    record = {"status": "incomplete", "case": case, "run_id": run_id,
              "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"],
                                                       cwd=ROOT, text=True).strip(),
              "source_status": subprocess.check_output(["git", "status", "--porcelain"],
                                                       cwd=ROOT, text=True).strip(),
              "image_sha256": sha256(image), "environment": reference,
              "budget_seconds": timeout, "command": args}
    manifest.write_text(json.dumps(record, indent=2) + "\n")
    network_reply = ("network online - DHCP configuration available"
                     if case == "nic-no-server" else "network unavailable")
    proof = StartupProof("STARTUP_" + run_id, network_reply)
    output: Queue[str | Exception | None] = Queue()
    started = time.monotonic()
    guest = None
    reader = None
    stderr = qemu_path.open("wb")
    serial = serial_path.open("w")
    try:
        guest = subprocess.Popen(args, cwd=ROOT, stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=stderr)
        assert guest.stdout is not None and guest.stdin is not None

        def read_serial() -> None:
            try:
                for raw in guest.stdout:
                    line = raw.decode(errors="replace").rstrip("\r\n")
                    serial.write(line + "\n")
                    serial.flush()
                    output.put(line)
            except Exception as error:
                output.put(error)
            finally:
                output.put(None)

        reader = threading.Thread(target=read_serial, daemon=True)
        reader.start()
        while time.monotonic() - started < timeout:
            if guest.poll() is not None:
                raise RuntimeError(f"QEMU exited before terminal proof: {guest.returncode}")
            try:
                line = output.get(timeout=0.1)
            except Empty:
                continue
            if isinstance(line, Exception):
                raise RuntimeError("serial reader failed") from line
            if line is None:
                raise RuntimeError("serial output ended before terminal proof")
            command = proof.observe(line)
            if line == "NORMAL_SHELL_READY":
                record["input_ready_ms"] = round((time.monotonic() - started) * 1000)
            if command:
                guest.stdin.write(command)
                guest.stdin.flush()
            if proof.complete:
                time.sleep(0.5)
                if guest.poll() is not None:
                    raise RuntimeError("QEMU reset after apparently successful transcript")
                record["completed_ms"] = round((time.monotonic() - started) * 1000)
                record["status"] = "passed"
                break
        else:
            raise TimeoutError(f"normal terminal proof exceeded {timeout}s")
    except Exception as error:
        record["status"] = "failed"
        record["failure"] = repr(error)
        raise
    finally:
        if guest is not None:
            guest.terminate()
            try:
                guest.wait(timeout=5)
            except subprocess.TimeoutExpired:
                guest.kill()
                guest.wait()
        if reader is not None:
            reader.join(timeout=5)
            if reader.is_alive():
                record["status"] = "failed"
                record["failure"] = "serial reader did not stop"
        while not output.empty():
            item = output.get_nowait()
            if isinstance(item, Exception):
                record["status"] = "failed"
                record["failure"] = f"serial reader: {item!r}"
        serial.close()
        stderr.close()
        for name, path in (("serial_sha256", serial_path), ("qemu_sha256", qemu_path)):
            if path.exists():
                record[name] = sha256(path)
        manifest.write_text(json.dumps(record, indent=2) + "\n")
    if record["status"] != "passed":
        raise RuntimeError(record["failure"])
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=CASES)
    parser.add_argument("--timeout", type=int, default=60)
    options = parser.parse_args()
    for case in (options.case,) if options.case else CASES:
        print(run_case(case, options.timeout))


if __name__ == "__main__":
    main()
