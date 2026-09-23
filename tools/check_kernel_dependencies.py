#!/usr/bin/env python3
"""Enforce the kernel's presentation/authority dependency direction."""

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "kernel/src"
COMPOSITION = {"main.rs", "lib.rs", "shell.rs"}
PRESENTATION = re.compile(r"\b(?:display|shell)\s*::|::\s*(?:display|shell)\b|\bDisplayManager\b")
AUTHORITY = re.compile(
    r"\b(?:runtime|storage|userspace|network|network_device|paging|memory|ramfs|socket)\s*::"
    r"|::\s*(?:runtime|storage|userspace|network|network_device|paging|memory|ramfs|socket)\b"
)
MUTABLE_AUTHORITY = re.compile(
    r"&\s*mut\s+(?:RamVfs|TaskRegistry|ProcessManager|RuntimeCoordinator|PersistentFs|SocketSet)\b"
)


def violations(path: Path, source: str) -> list[str]:
    """Return forbidden source lines; Rust's type checker enforces read-only borrows."""
    relative = path.as_posix()
    if relative.startswith("kernel/src/display/"):
        forbidden = (AUTHORITY, MUTABLE_AUTHORITY)
    elif relative.startswith("kernel/src/") and path.name not in COMPOSITION:
        forbidden = (PRESENTATION,)
    else:
        return []
    return [
        f"{relative}:{number}: {line.strip()}"
        for number, line in enumerate(source.splitlines(), 1)
        if any(pattern.search(line) for pattern in forbidden)
    ]


def scan() -> list[str]:
    found = []
    for path in sorted(SOURCE.rglob("*.rs")):
        relative = path.relative_to(ROOT)
        found.extend(violations(relative, path.read_text()))
    return found


if __name__ == "__main__":
    errors = scan()
    if errors:
        for error in errors:
            print(error)
        raise SystemExit("kernel presentation/authority dependency violation")
    print("Kernel presentation/authority dependency direction passed")
