#!/usr/bin/env python3
"""Prove delayed untrusted output cannot corrupt or submit a pending serial command."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile
import time

from reference_vm import environment, load_profile, qemu_args
from test_exception_entry import firmware_path


ROOT = Path(__file__).resolve().parents[1]
PATCH = ROOT / "tools/fixtures/terminal-delayed-output.patch"
PENDING = b"echo PENDING"
BACKGROUND = b"\r\x1b[2KBACKGROUND?[2J?ATTACK\ngenos> echo PENDING"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run() -> Path:
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()
    if status:
        raise ValueError("terminal interleave proof requires committed clean source")
    firmware = firmware_path()
    reference = environment(firmware, ROOT)
    if not reference["reference_environment_match"]:
        raise ValueError(f"reference environment mismatch: {reference['reference_mismatches']}")
    run_id = str(time.time_ns())
    evidence = ROOT / "build/terminal-safety-evidence" / run_id
    evidence.mkdir(parents=True)
    manifest = evidence / "manifest.json"
    serial_path = evidence / "serial.bin"
    stderr_path = evidence / "qemu.log"
    build_path = evidence / "build.log"
    record = {"status": "incomplete", "run_id": run_id, "source_commit": source,
              "source_status": status, "fixture_patch_sha256": digest(PATCH),
              "environment": reference, "boot_budget_seconds": 45,
              "fixture_scope": "delayed INIT hold output only; production serial/kernel/shell unchanged"}
    manifest.write_text(json.dumps(record, indent=2) + "\n")
    fixture = Path(tempfile.mkdtemp(prefix="genos-terminal-fixture-"))
    fixture.rmdir()
    worktree_added = False
    guest = None
    try:
        subprocess.run(["git", "worktree", "add", "--detach", str(fixture), source],
                       cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        worktree_added = True
        subprocess.run(["git", "apply", "--check", str(PATCH)], cwd=fixture, check=True)
        subprocess.run(["git", "apply", str(PATCH)], cwd=fixture, check=True)
        with build_path.open("wb") as build_log:
            subprocess.run(["cargo", "xtask", "build-release"], cwd=fixture,
                           stdout=build_log, stderr=subprocess.STDOUT,
                           check=True, timeout=180)
        image = fixture / "build/genos.img"
        record["fixture_image_sha256"] = digest(image)
        profile = load_profile(fixture)
        args = qemu_args(root=fixture) + [
            "-drive", f"if=pflash,format=raw,readonly=on,file={firmware}",
            "-drive", profile["boot_drive"] + f",file={image}",
            "-net", "none", "-display", "none", "-monitor", "none",
            "-serial", "stdio", "-no-reboot",
        ]
        record["command"] = args
        (evidence / "qemu-command.json").write_text(json.dumps(args, indent=2) + "\n")
        output = bytearray()
        stage = 0
        started = time.monotonic()
        with serial_path.open("wb") as serial, stderr_path.open("wb") as stderr:
            guest = subprocess.Popen(args, cwd=fixture, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=stderr)
            assert guest.stdin is not None and guest.stdout is not None
            while time.monotonic() - started < record["boot_budget_seconds"]:
                if guest.poll() is not None:
                    raise RuntimeError(f"QEMU exited at stage {stage}: {guest.returncode}")
                ready, _, _ = select.select([guest.stdout], [], [], 0.05)
                if ready:
                    chunk = os.read(guest.stdout.fileno(), 4096)
                    if not chunk:
                        raise RuntimeError("serial ended before transcript completed")
                    serial.write(chunk)
                    serial.flush()
                    output.extend(chunk)
                if stage == 0 and b"NORMAL_SHELL_READY\ngenos> " in output:
                    guest.stdin.write(b"run init hold\r")
                    guest.stdin.flush()
                    stage = 1
                if stage == 1 and b"job 1 started" in output:
                    after_job = output.index(b"job 1 started")
                    if b"genos> " in output[after_job:]:
                        guest.stdin.write(PENDING)
                        guest.stdin.flush()
                        stage = 2
                if stage == 2 and BACKGROUND in output:
                    if b"\nPENDING\n" in output:
                        raise ValueError("partial command executed before Enter")
                    guest.stdin.write(b"\r")
                    guest.stdin.flush()
                    stage = 3
                if stage == 3 and b"\nPENDING\ngenos> " in output[output.index(BACKGROUND):]:
                    after_ready = output[output.index(b"NORMAL_SHELL_READY"):]
                    if b"\x1b[2J" in after_ready or b"BACKGROUND\x1b" in after_ready:
                        raise ValueError("untrusted output emitted a control sequence")
                    if after_ready.count(b"\nPENDING\n") != 1:
                        raise ValueError("pending command executed more than once")
                    record["input_ready_ms"] = round((time.monotonic() - started) * 1000)
                    record["status"] = "passed"
                    break
            else:
                raise TimeoutError(f"terminal interleave stopped at stage {stage}")
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
        if worktree_added:
            subprocess.run(["git", "worktree", "remove", "--force", str(fixture)],
                           cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        for name, path in (("serial_sha256", serial_path), ("qemu_sha256", stderr_path),
                           ("build_sha256", build_path)):
            if path.exists():
                record[name] = digest(path)
        manifest.write_text(json.dumps(record, indent=2) + "\n")
    return manifest


if __name__ == "__main__":
    print(run())
