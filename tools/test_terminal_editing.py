#!/usr/bin/env python3
"""Exercise the bounded serial decoder/editor with visible normal guest results."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import time

from reference_vm import environment, load_profile, qemu_args
from test_exception_entry import firmware_path


ROOT = Path(__file__).resolve().parents[1]
CASES = (
    ("cursor-delete", b"echo ABC\x1b[D\x1b[DX\x1b[3~\r", b"\nAXC\ngenos> "),
    ("history", b"\x1b[A\r", b"\nAXC\ngenos> "),
    ("home-end-right-backspace",
     b"echo ABCD\x1b[H\x1b[C\x1b[C\x1b[C\x1b[C\x1b[CZ\x1b[F\x7f\r",
     b"\nZABC\ngenos> "),
    ("completion", b"un\t\r", b"GenOS v0.56 ring3-shell x86_64 ABI 18\ngenos> "),
    ("paste", b"\x1b[200~echo PASTE\nuname\x1b[201~\r", b"\nPASTE uname\ngenos> "),
    ("invalid-escape", b"echo SAFE\x1b[2J\r", b"\nSAFE\ngenos> "),
    ("cancel", b"echo DROP\x03echo KEEP\r", b"\nKEEP\ngenos> "),
    ("bounded-line", b"echo " + b"x" * 75 + b"BAD\r", b"\n" + b"x" * 75 + b"\ngenos> "),
    ("incomplete-prefix", b"echo LOST\x1b", b"^C\ngenos> "),
    ("after-prefix", b"echo RECOVER\r", b"\nRECOVER\ngenos> "),
)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run() -> Path:
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    source_status = subprocess.check_output(["git", "status", "--porcelain"],
                                            cwd=ROOT, text=True).strip()
    if source_status:
        raise ValueError("terminal editing proof requires committed clean source")
    firmware = firmware_path()
    reference = environment(firmware, ROOT)
    if not reference["reference_environment_match"]:
        raise ValueError("reference environment mismatch")
    if (ROOT / "build/image-mode.txt").read_text().splitlines()[0] != "Release":
        raise ValueError("build a normal release image before this proof")
    image = ROOT / "build/genos.img"
    profile = load_profile(ROOT)
    args = qemu_args(root=ROOT) + [
        "-drive", f"if=pflash,format=raw,readonly=on,file={firmware}",
        "-drive", profile["boot_drive"] + f",file={image}",
        "-net", "none", "-display", "none", "-monitor", "none",
        "-serial", "stdio", "-no-reboot",
    ]
    run_id = str(time.time_ns())
    directory = ROOT / "build/terminal-editing-evidence" / run_id
    directory.mkdir(parents=True)
    serial_path = directory / "serial.bin"
    qemu_path = directory / "qemu.log"
    manifest = directory / "manifest.json"
    record = {"status": "incomplete", "run_id": run_id, "source_commit": source,
              "source_status": source_status, "environment": reference,
              "image_sha256": digest(image), "command": args,
              "budget_seconds": 120, "cases": []}
    manifest.write_text(json.dumps(record, indent=2) + "\n")
    (directory / "qemu-command.json").write_text(json.dumps(args, indent=2) + "\n")
    guest = None
    started = time.monotonic()
    output = bytearray()
    try:
        with serial_path.open("wb") as serial, qemu_path.open("wb") as stderr:
            guest = subprocess.Popen(args, cwd=ROOT, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=stderr)
            assert guest.stdin is not None and guest.stdout is not None

            def wait_for(marker: bytes, start: int, deadline: float) -> bytes:
                while time.monotonic() < deadline:
                    if guest.poll() is not None:
                        raise RuntimeError(f"QEMU exited before {marker!r}: {guest.returncode}")
                    ready, _, _ = select.select([guest.stdout], [], [], 0.05)
                    if ready:
                        chunk = os.read(guest.stdout.fileno(), 4096)
                        if not chunk:
                            raise RuntimeError("serial ended early")
                        serial.write(chunk)
                        serial.flush()
                        output.extend(chunk)
                        if len(output) > 1024 * 1024:
                            raise ValueError("serial byte budget exceeded")
                    if marker in output[start:]:
                        return bytes(output[start:])
                raise TimeoutError(f"missing {marker!r}")

            wait_for(b"NORMAL_SHELL_READY\ngenos> ", 0, started + 45)
            for name, sent, expected in CASES:
                offset = len(output)
                guest.stdin.write(sent)
                guest.stdin.flush()
                section = wait_for(expected, offset, started + record["budget_seconds"])
                if b"RECOVERY_CONSOLE_READY" in section or b"KERNEL PANIC" in section:
                    raise ValueError(f"guest failure during {name}")
                if name == "paste" and b"\nGenOS v0.56" in section:
                    raise ValueError("pasted newline executed a second command")
                if name == "invalid-escape" and b"\x1b[2J" in section:
                    raise ValueError("invalid input escape became output control")
                if name == "cancel" and b"\nDROP\n" in section:
                    raise ValueError("cancelled command executed")
                if name == "bounded-line" and b"BAD" in section:
                    raise ValueError("overlength bytes entered the command")
                if name == "incomplete-prefix" and b"\nLOST\n" in section:
                    raise ValueError("incomplete escape submitted the pending line")
                record["cases"].append({"name": name, "elapsed_ms":
                                        round((time.monotonic() - started) * 1000)})
            if guest.poll() is not None:
                raise RuntimeError("QEMU stopped after apparently successful editing")
            record["status"] = "passed"
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
        for name, path in (("serial_sha256", serial_path), ("qemu_sha256", qemu_path)):
            if path.exists():
                record[name] = digest(path)
        manifest.write_text(json.dumps(record, indent=2) + "\n")
    return manifest


if __name__ == "__main__":
    print(run())
