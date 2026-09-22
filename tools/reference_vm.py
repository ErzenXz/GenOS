"""Shared frozen VM arguments and honest environment identity for CPU probes."""
from __future__ import annotations

import hashlib
from pathlib import Path
import subprocess

KEYS = frozenset(("profile", "machine", "cpu", "accel", "memory", "smp", "qemu_version",
                  "rust_release", "firmware_sha256", "boot_drive", "boot_device", "boot_order", "storage_controller",
                  "storage_device", "network_backend", "network_device"))


def parse_profile(text: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if (not separator or key not in KEYS or key in values
                or not value or value.strip() != value):
            raise ValueError(f"unknown, empty, malformed or duplicate VM profile field: {key}")
        values[key] = value
    if values.keys() != KEYS:
        raise ValueError("missing VM profile field")
    digest = values["firmware_sha256"]
    if len(digest) != 64 or any(character not in "0123456789abcdef" for character in digest):
        raise ValueError("invalid reference firmware SHA256")
    # Supplying one protocol switch disables the other unless it is explicit.
    network = values["network_backend"].split(",")
    for family in ("ipv4", "ipv6"):
        switches = [part for part in network if part.startswith(family + "=")]
        if switches != [family + "=on"]:
            raise ValueError(f"reference network requires exactly one {family}=on")
    return values


def profile_path(root: Path | None = None) -> Path:
    return (root or Path(__file__).resolve().parents[1]) / "tools/reference-vm.conf"


def load_profile(root: Path | None = None) -> dict[str, str]:
    return parse_profile(profile_path(root).read_text())


def qemu_args(cpus: int = 1, root: Path | None = None) -> list[str]:
    if cpus not in (1, 4):
        raise ValueError("only the reference single CPU and BSP rejection four-CPU variant are defined")
    profile = load_profile(root)
    topology = profile["smp"] if cpus == 1 else "4,sockets=1,cores=4,threads=1"
    return ["qemu-system-x86_64", "-machine", profile["machine"], "-cpu", profile["cpu"],
            "-accel", profile["accel"], "-m", profile["memory"],
            "-boot", profile["boot_order"], "-device", profile["boot_device"], "-smp", topology]


def mismatches(profile: dict[str, str], rust: str, qemu: str, firmware_sha256: str) -> list[str]:
    differences = []
    if f"release: {profile['rust_release']}" not in rust.splitlines():
        differences.append("rust_release")
    if not qemu.splitlines() or qemu.splitlines()[0] != profile["qemu_version"]:
        differences.append("qemu_version")
    if firmware_sha256 != profile["firmware_sha256"]:
        differences.append("firmware_sha256")
    return differences


def environment(firmware: Path, root: Path | None = None) -> dict:
    profile = load_profile(root)
    def output(args: list[str]) -> str:
        return subprocess.run(args, capture_output=True, text=True, check=True, timeout=30).stdout.strip()
    rust = output(["rustc", "-Vv"])
    qemu = output(["qemu-system-x86_64", "--version"])
    digest = hashlib.sha256(firmware.read_bytes()).hexdigest()
    differences = mismatches(profile, rust, qemu, digest)
    return {"reference_profile": profile["profile"], "reference_environment_match": not differences,
            "reference_mismatches": differences, "firmware_path": str(firmware),
            "firmware_sha256": digest, "profile_sha256": hashlib.sha256(profile_path(root).read_bytes()).hexdigest(),
            "rust": rust, "qemu": qemu}
