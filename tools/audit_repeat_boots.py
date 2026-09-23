#!/usr/bin/env python3
"""Verify one complete, clean 1000-boot release lane from retained evidence."""

from __future__ import annotations

import argparse
import hashlib
import re
from pathlib import Path


class AuditError(ValueError):
    pass


def read_manifest(path: Path) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition("=")
        if not separator or key in fields:
            raise AuditError(f"malformed or duplicate manifest field: {path}: {key}")
        fields[key] = value
    return fields


def audit(
    evidence_dir: Path,
    log_path: Path,
    commit: str,
    count: int = 1000,
    artifacts_dir: Path | None = None,
) -> dict[str, str]:
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or not 1 <= count <= 1000:
        raise AuditError("expected a full 40-character commit and a count from 1 to 1000")

    selected: dict[int, tuple[Path, dict[str, str]]] = {}
    for path in evidence_dir.glob("*/repeat-*/manifest.txt"):
        fields = read_manifest(path)
        if fields.get("commit") != commit:
            continue
        match = re.fullmatch(r"repeat-(\d{4})", path.parent.name)
        if match is None:
            raise AuditError(f"invalid repeat label: {path}")
        ordinal = int(match.group(1))
        if ordinal in selected:
            raise AuditError(f"duplicate boot {ordinal}: {selected[ordinal][0]} and {path}")
        selected[ordinal] = path, fields

    expected = set(range(1, count + 1))
    if set(selected) != expected:
        missing = sorted(expected - set(selected))
        extra = sorted(set(selected) - expected)
        raise AuditError(f"incomplete repeat evidence: missing={missing[:8]} extra={extra[:8]}")

    identity_fields = ("reference_profile", "profile_sha256", "firmware_sha256", "image_sha256", "rust", "qemu")
    first = selected[1][1]
    identity = {key: first.get(key, "") for key in identity_fields}
    if any(not value for value in identity.values()):
        raise AuditError("first boot lacks complete source/profile/image identity")
    run_ids: set[str] = set()
    for ordinal in range(1, count + 1):
        path, fields = selected[ordinal]
        if fields.get("status") != "passed":
            raise AuditError(f"first failed boot {ordinal}: {path}: {fields.get('failure', 'unknown')}")
        if fields.get("run_id") != path.parent.parent.name or fields["run_id"] in run_ids:
            raise AuditError(f"missing or repeated run ID at boot {ordinal}: {path}")
        run_ids.add(fields["run_id"])
        if fields.get("mode") != "Release" or fields.get("reference_environment_match") != "true":
            raise AuditError(f"wrong release mode or reference environment at boot {ordinal}: {path}")
        if fields.get("working_tree_status") != '""':
            raise AuditError(f"dirty source at boot {ordinal}: {path}")
        if any(fields.get(key) != value for key, value in identity.items()):
            raise AuditError(f"source/profile/image identity changed at boot {ordinal}: {path}")
        if not all(re.fullmatch(r"[0-9a-f]{64}", fields.get(key, "")) for key in ("serial_sha256", "qemu_log_sha256")):
            raise AuditError(f"missing boot-output hashes at boot {ordinal}: {path}")
        if artifacts_dir is not None:
            for filename, field in (
                (f"serial-repeat-{ordinal:04d}.log", "serial_sha256"),
                (f"normal-repeat-{ordinal:04d}-qemu.log", "qemu_log_sha256"),
            ):
                artifact = artifacts_dir / filename
                actual = hashlib.sha256(artifact.read_bytes()).hexdigest()
                if actual != fields[field]:
                    raise AuditError(f"boot-output hash mismatch at boot {ordinal}: {artifact}")

    lines = log_path.read_text(encoding="utf-8").splitlines()
    successes = [int(match.group(1)) for line in lines if (match := re.fullmatch(r"repeat-(\d{4}) boot passed: .+", line))]
    progress = [int(match.group(1)) for line in lines if (match := re.fullmatch(rf"NORMAL_BOOT_REPETITION_PROGRESS completed=(\d+) requested={count}", line))]
    if successes != list(range(1, count + 1)) or progress != successes:
        raise AuditError("log has missing, repeated, out-of-order or forged boot progress")
    if lines.count(f"NORMAL_BOOT_REPETITION_OK completed={count} requested={count}") != 1:
        raise AuditError("final repeat completion marker missing or duplicated")

    return {
        "commit": commit,
        "count": str(count),
        "log_sha256": hashlib.sha256(log_path.read_bytes()).hexdigest(),
        **identity,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--artifacts-dir", type=Path, help="also hash every retained serial/QEMU log")
    args = parser.parse_args()
    try:
        result = audit(args.evidence_dir, args.log, args.commit, args.count, args.artifacts_dir)
    except (AuditError, OSError) as error:
        parser.exit(1, f"repeat audit failed: {error}\n")
    print("repeat audit passed: " + " ".join(f"{key}={value}" for key, value in result.items()))


if __name__ == "__main__":
    main()
