#!/usr/bin/env python3
"""Fail when a repository-local Markdown link points to a missing path."""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote

LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
SKIPPED_SCHEMES = ("http://", "https://", "mailto:", "tel:")


def local_target(markdown: Path, raw_target: str) -> Path | None:
    target = raw_target.strip().split(maxsplit=1)[0].strip("<>")
    if not target or target.startswith("#") or target.lower().startswith(SKIPPED_SCHEMES):
        return None
    target = unquote(target.split("#", 1)[0].split("?", 1)[0])
    if not target:
        return None
    if target.startswith("/"):
        return Path(target.lstrip("/"))
    return markdown.parent / target


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    failures: list[str] = []
    for markdown in sorted(root.rglob("*.md")):
        if any(part in {"target", "build", ".git"} for part in markdown.parts):
            continue
        text = markdown.read_text(encoding="utf-8")
        for match in LINK.finditer(text):
            target = local_target(markdown.relative_to(root), match.group(1))
            if target is None:
                continue
            resolved = (root / target).resolve()
            try:
                resolved.relative_to(root.resolve())
            except ValueError:
                failures.append(f"{markdown.relative_to(root)}: link escapes repository: {match.group(1)}")
                continue
            if not resolved.exists():
                line = text.count("\n", 0, match.start()) + 1
                failures.append(
                    f"{markdown.relative_to(root)}:{line}: missing link target {match.group(1)}"
                )
    if failures:
        print("Markdown link validation failed:", file=sys.stderr)
        print("\n".join(f"- {failure}" for failure in failures), file=sys.stderr)
        return 1
    print("Markdown link validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
