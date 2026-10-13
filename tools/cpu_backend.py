"""Explicit CPU qualification backends and retained accelerator evidence."""
from __future__ import annotations

import json
import os
from pathlib import Path
import platform
import socket


def preflight(backend: str) -> dict:
    """Reject unavailable hardware acceleration rather than falling back to TCG."""
    if backend not in {"tcg", "kvm"}:
        raise ValueError(f"unknown CPU backend: {backend}")
    result = {"cpu_backend": backend}
    if backend == "tcg":
        return result
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("KVM qualification requires Linux x86_64")
    # fcntl is unavailable on Windows; import only after the platform admission.
    import fcntl

    result["host_uname"] = platform.uname()._asdict()
    cpuinfo = Path("/proc/cpuinfo").read_text()
    first_cpu = cpuinfo.split("\n\n", 1)[0]
    result["host_cpu"] = {
        key.strip(): value.strip()
        for line in first_cpu.splitlines()
        for key, separator, value in [line.partition(":")]
        if separator and key.strip() in {
            "vendor_id", "cpu family", "model", "model name", "stepping",
            "microcode", "flags", "physical id", "core id",
        }
    }
    if not result["host_cpu"].get("vendor_id") or not result["host_cpu"].get("model name"):
        raise RuntimeError("KVM qualification could not retain host CPU identity")
    descriptor = os.open("/dev/kvm", os.O_RDWR | os.O_CLOEXEC)
    try:
        version = fcntl.ioctl(descriptor, 0xAE00, 0)  # KVM_GET_API_VERSION
        result["kvm_api_version"] = version
        if version != 12:
            raise RuntimeError(f"unsupported KVM API version: {version}")
        vm = fcntl.ioctl(descriptor, 0xAE01, 0)  # KVM_CREATE_VM, x86 default type
        try:
            if vm < 0:
                raise RuntimeError("KVM_CREATE_VM did not return a VM descriptor")
        finally:
            if vm >= 0:
                os.close(vm)
    finally:
        os.close(descriptor)
    return result


def qmp_command(channel: Path, request: dict, transcript: list) -> object:
    """Execute one QMP command, retaining the complete exchanged JSON stream."""
    if not isinstance(request, dict) or not isinstance(request.get("execute"), str):
        raise ValueError("QMP request must name an execute command")
    payload = dict(request)
    payload.setdefault("id", "genos-cpu-request")
    capabilities_id = "genos-cpu-capabilities"
    if payload["id"] == capabilities_id:
        raise ValueError("QMP request uses the reserved capabilities ID")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(10)
        connection.connect(str(channel))
        with connection.makefile("rwb") as wire:
            def receive() -> dict:
                line = wire.readline(1024 * 1024 + 1)
                if not line:
                    raise RuntimeError("QMP connection closed before its response")
                if len(line) > 1024 * 1024 or not line.endswith(b"\n"):
                    raise RuntimeError("QMP response exceeded the line limit")
                message = json.loads(line)
                transcript.append({"direction": "received", "message": message})
                if not isinstance(message, dict):
                    raise RuntimeError("QMP response must be an object")
                return message

            greeting = receive()
            if not isinstance(greeting.get("QMP"), dict):
                raise RuntimeError("QMP greeting missing")
            answer = None
            for command in ({"execute": "qmp_capabilities", "id": capabilities_id}, payload):
                transcript.append({"direction": "sent", "message": command})
                wire.write((json.dumps(command) + "\n").encode())
                wire.flush()
                while True:
                    message = receive()
                    if "event" in message:
                        continue
                    if message.get("id") != command["id"]:
                        raise RuntimeError("QMP response ID does not match the command")
                    if "error" in message:
                        raise RuntimeError(f"QMP {command['execute']} failed: {message['error']}")
                    if "return" not in message:
                        raise RuntimeError("QMP command response has no return value")
                    answer = message["return"]
                    break
            return answer


def verify_accelerator(backend: str, result: object) -> None:
    """Admit only the requested accelerator after QMP query-kvm readback."""
    if backend not in {"tcg", "kvm"}:
        raise ValueError(f"unknown CPU backend: {backend}")
    if not isinstance(result, dict) or result.get("enabled") is not (backend == "kvm"):
        raise RuntimeError(f"QMP did not confirm the requested {backend} accelerator")
    if backend == "kvm" and result.get("present") is not True:
        raise RuntimeError("QMP did not confirm KVM support")
