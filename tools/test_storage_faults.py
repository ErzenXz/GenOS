#!/usr/bin/env python3
"""Retain production storage faults and compare them with the independent host decoder.

Only a new build/storage-fault-evidence directory is written. No VM or normal
user volume is opened. A successful run identifies source, each raw image,
guest result, independent host result, and complete command output.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


SOURCES = ("kernel/src/storage.rs", "kernel/src/storage_tests.rs", "kernel/src/vfs.rs",
           "kernel/src/path_policy.rs", "tools/xtask/src/main.rs", "tools/test_storage_faults.py")


def hashes(root: Path, paths) -> dict[str, str]:
    return {str(path): hashlib.sha256((root / path).read_bytes()).hexdigest() for path in paths}


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    run = root / "build/storage-fault-evidence" / str(time.time_ns())
    corpus = run / "corpus"
    corpus.mkdir(parents=True)
    manifest = {"status": "incomplete", "source_sha256": hashes(root, SOURCES),
                "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
                "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=root, text=True))}
    commands = [["cargo", "test", "-p", "kernel", "--lib", "storage_under_test"],
                ["cargo", "xtask", "check-storage-corpus", str(corpus)]]
    manifest["commands"] = commands
    try:
        environment = dict(os.environ, GENOS_STORAGE_CORPUS=str(corpus))
        for index, command in enumerate(commands):
            with (run / f"command-{index}.log").open("w") as output:
                subprocess.run(command, cwd=root, env=environment, stdout=output,
                               stderr=subprocess.STDOUT, timeout=180, check=True)
        count = len(list(corpus.glob("*.img")))
        if count < 606:
            raise ValueError(f"incomplete fault corpus: {count} images")
        if hashes(root, SOURCES) != manifest["source_sha256"]:
            raise ValueError("source changed during storage qualification")
        manifest.update(status="passed", cases=count)
    except Exception as error:
        manifest.update(status="failed", failure=f"{type(error).__name__}: {error}")
        raise
    finally:
        manifest["artifacts_sha256"] = hashes(run, sorted(path.relative_to(run) for path in run.rglob("*") if path.is_file()))
        (run / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(f"STORAGE_FAULT_EVIDENCE status={manifest['status']} evidence={run}", flush=True)


if __name__ == "__main__":
    main()
