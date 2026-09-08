#!/usr/bin/env python3
"""Check a reviewable, lexical inventory of Rust unsafe sites and assembly.

This is deliberately not a Rust parser, macro expander, or semantic safety proof.
Only Python's standard library is needed; see docs/UNSAFE_INVENTORY.md.
"""
from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass
import difflib
import hashlib
import json
import os
from pathlib import Path
import sys

SCHEMA_VERSION = 1
EXCLUDED_DIRS = frozenset({".git", ".cache", "target", "build", "dist", "out", "generated", "vendor", "node_modules", "__pycache__"})
SOURCE_SUFFIXES = frozenset({".rs", ".s", ".S", ".asm"})
CONTEXT_LINES = 3
ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class Token:
    text: str
    start: int
    end: int


def digest(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def rust_tokens(source: str) -> list[Token]:
    """Discard comments/literal contents while retaining delimiter positions.

    Keep literals as opaque tokens so their removal cannot accidentally combine
    adjacent keywords. Recognize nested comments, raw/byte/C strings, escaped
    characters, Unicode identifiers, lifetimes, and raw identifiers.
    """
    tokens = []
    size = len(source)
    pos = 0
    while pos < size:
        start = pos
        char = source[pos]
        if char.isspace():
            pos += 1
            continue
        if source.startswith("//", pos):
            end = source.find("\n", pos + 2)
            pos = size if end < 0 else end
            continue
        if source.startswith("/*", pos):
            depth = 1
            pos += 2
            while pos < size and depth:
                if source.startswith("/*", pos):
                    depth += 1
                    pos += 2
                elif source.startswith("*/", pos):
                    depth -= 1
                    pos += 2
                else:
                    pos += 1
            if depth:
                raise ValueError(f"unterminated block comment at offset {start}")
            continue

        # Raw strings: r"...", r#"..."#, br##"..."##, cr"...".
        raw = pos
        if source.startswith(("br", "cr"), pos):
            raw += 1
        if source.startswith("r", raw):
            quote = raw + 1
            while quote < size and source[quote] == "#":
                quote += 1
            if quote < size and source[quote] == '"':
                close = '"' + "#" * (quote - raw - 1)
                end = source.find(close, quote + 1)
                if end < 0:
                    raise ValueError(f"unterminated raw string at offset {start}")
                pos = end + len(close)
                tokens.append(Token("<literal>", start, pos))
                continue

        quote = pos + 1 if source.startswith(('b"', 'c"', "b'"), pos) else pos
        if source[quote] == '"':
            pos = quote + 1
            while pos < size:
                if source[pos] == "\\":
                    pos += 2
                elif source[pos] == '"':
                    pos += 1
                    break
                else:
                    pos += 1
            else:
                raise ValueError(f"unterminated string at offset {start}")
            tokens.append(Token("<literal>", start, pos))
            continue
        if source[quote] == "'":
            end = quote + 1
            if end < size and source[end] == "\\":
                end += 2
                if end <= size and source[end - 1] == "u" and source[end:end + 1] == "{":
                    close = source.find("}", end + 1)
                    end = size if close < 0 else close + 1
                elif end <= size and source[end - 1] == "x":
                    end += 2
            else:
                end += 1
            if end < size and source[end] == "'":
                pos = end + 1
                tokens.append(Token("<literal>", start, pos))
                continue
            # A lifetime is punctuation followed by an identifier, not a string.

        if char == "_" or char.isalpha():
            pos += 1
            if source.startswith("r#", start):
                pos += 1
            while pos < size and (source[pos] == "_" or source[pos].isalnum()):
                pos += 1
            tokens.append(Token(source[start:pos], start, pos))
        else:
            pos += 1
            tokens.append(Token(char, start, pos))
    return tokens


def construct_end(tokens: list[Token], index: int) -> int:
    """Find the first balanced body/argument group or declaration terminator."""
    pairs = {"{": "}", "(": ")", "[": "]"}
    position = index + 1
    while position < len(tokens):
        token = tokens[position]
        if token.text == ";":
            return token.end
        if token.text in pairs:
            # Skip complete parameter/type groups, including array semicolons.
            skip_group = token.text != "{" and tokens[index].text == "unsafe" and position != index + 1
            stack = [pairs[token.text]]
            cursor = position + 1
            while cursor < len(tokens):
                candidate = tokens[cursor]
                if candidate.text in pairs:
                    stack.append(pairs[candidate.text])
                elif candidate.text in pairs.values():
                    if not stack or candidate.text != stack.pop():
                        raise ValueError(f"unbalanced delimiter at offset {candidate.start}")
                    if not stack:
                        if not skip_group:
                            return candidate.end
                        position = cursor
                        break
                cursor += 1
            else:
                raise ValueError(f"unterminated construct at offset {token.start}")
        position += 1
    return tokens[index].end


def classify_unsafe(tokens: list[Token], index: int) -> str:
    following = [token.text for token in tokens[index + 1:index + 8]]
    if following and following[0] == "{":
        return "unsafe_block"
    if following and following[0] == "(":
        return "unsafe_attribute"
    for token in following:
        if token in {"fn", "impl", "trait"}:
            return f"unsafe_{token}"
        if token in {"{", ";"}:
            break
    return "unsafe_extern" if following and following[0] == "extern" else "unsafe_keyword"


def inventory_source(relative: str, source: str) -> list[dict]:
    tokens = rust_tokens(source)
    lines = source.splitlines()
    sites = []
    for index, token in enumerate(tokens):
        if token.text == "unsafe":
            kind = classify_unsafe(tokens, index)
        elif token.text in {"asm", "global_asm", "naked_asm"} and index + 1 < len(tokens) and tokens[index + 1].text == "!":
            kind = f"{token.text}_macro"
        else:
            continue
        end = construct_end(tokens, index)
        line = source.count("\n", 0, token.start) + 1
        end_line = source.count("\n", 0, end) + 1
        context_start = max(1, line - CONTEXT_LINES)
        context_end = min(len(lines), line + CONTEXT_LINES)
        sites.append({
            "path": relative,
            "kind": kind,
            "line": line,
            "column": token.start - source.rfind("\n", 0, token.start),
            "end_line": end_line,
            "construct_sha256": digest(source[token.start:end]),
            "context_start_line": context_start,
            "context": "\n".join(lines[context_start - 1:context_end]),
        })
    return sites


def build_inventory(root: Path) -> dict:
    files = []
    sites = []
    paths = []
    for directory, subdirectories, names in os.walk(root, followlinks=False):
        subdirectories[:] = sorted(name for name in subdirectories if name not in EXCLUDED_DIRS)
        for name in names:
            if Path(name).suffix in SOURCE_SUFFIXES:
                paths.append(Path(directory) / name)
    for path in sorted(paths):
        relative = path.relative_to(root)
        if any(part in EXCLUDED_DIRS for part in relative.parts) or path.suffix not in SOURCE_SUFFIXES:
            continue
        if path.is_symlink():
            raise ValueError(f"source symlinks need explicit review: {relative.as_posix()}")
        if not path.is_file():
            continue
        name = relative.as_posix()
        source = path.read_bytes().decode("utf-8")
        files.append({"path": name, "sha256": digest(source), "lines": len(source.splitlines())})
        if path.suffix == ".rs":
            try:
                sites.extend(inventory_source(name, source))
            except ValueError as error:
                raise ValueError(f"{name}: {error}") from error
        else:
            sites.append({"path": name, "kind": "assembly_file", "line": 1,
                          "column": 1, "end_line": len(source.splitlines()),
                          "construct_sha256": digest(source), "context_start_line": 1,
                          "context": "\n".join(source.splitlines()[:CONTEXT_LINES * 2 + 1])})
    return {
        "schema_version": SCHEMA_VERSION,
        "scope": {"suffixes": sorted(SOURCE_SUFFIXES), "excluded_directory_names": sorted(EXCLUDED_DIRS),
                  "context_lines": CONTEXT_LINES,
                  "semantics": "lexical source inventory; all cfg branches; no macro expansion or safety proof"},
        "counts": dict(sorted(Counter(site["kind"] for site in sites).items())),
        "files": files,
        "sites": sites,
    }


def serialized(inventory: dict) -> str:
    return json.dumps(inventory, indent=2, ensure_ascii=False, sort_keys=True) + "\n"


def describe_diff(before: dict, after: dict) -> list[str]:
    changes = []
    old_files = {item["path"]: item for item in before.get("files", [])}
    new_files = {item["path"]: item for item in after["files"]}
    for name in sorted(old_files.keys() | new_files.keys()):
        if name not in old_files:
            changes.append(f"ADDED source: {name}")
        elif name not in new_files:
            changes.append(f"REMOVED source: {name}")
        elif old_files[name] != new_files[name]:
            changes.append(f"CHANGED source/context: {name}")
    old_counts, new_counts = before.get("counts", {}), after["counts"]
    for kind in sorted(old_counts.keys() | new_counts.keys()):
        old, new = old_counts.get(kind, 0), new_counts.get(kind, 0)
        if old != new:
            changes.append(f"{kind}: {old} -> {new} ({new - old:+d})")
    return changes


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--baseline", type=Path, help="default: ROOT/docs/unsafe-inventory.json")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="fail on any inventory/context change (default)")
    mode.add_argument("--diff", action="store_true", help="show full baseline diff; fail when different")
    mode.add_argument("--write-baseline", action="store_true", help="explicitly replace baseline after source review")
    mode.add_argument("--json", action="store_true", help="print current inventory without checking")
    args = parser.parse_args(argv)
    root = args.root.resolve()
    baseline = args.baseline or root / "docs" / "unsafe-inventory.json"
    try:
        if not root.is_dir():
            raise ValueError(f"source root is not a directory: {root}")
        current = build_inventory(root)
        if args.json:
            print(serialized(current), end="")
            return 0
        previous = json.loads(baseline.read_text()) if baseline.exists() else None
        if previous is not None and not isinstance(previous, dict):
            raise ValueError("baseline must be a JSON object")
        changes = describe_diff(previous, current) if previous is not None else ["No existing baseline"]
        if args.write_baseline:
            for change in changes:
                print(change)
            baseline.parent.mkdir(parents=True, exist_ok=True)
            baseline.write_text(serialized(current))
            print(f"Wrote {len(current['sites'])} lexical sites across {len(current['files'])} source files to {baseline}")
            print("Baseline replacement records source context; it does not certify unsafe code as reviewed or safe.")
            return 0
        if previous == current:
            print(f"Unsafe inventory matches: {len(current['sites'])} lexical sites, {len(current['files'])} source files.")
            return 0
        for change in changes:
            print(change)
        if args.diff and previous is not None:
            print("".join(difflib.unified_diff(serialized(previous).splitlines(True), serialized(current).splitlines(True),
                                               fromfile=str(baseline), tofile="current inventory")), end="")
        print("Unsafe inventory/context changed. Review the source and --diff, then explicitly --write-baseline.", file=sys.stderr)
        return 1
    except (OSError, UnicodeError, ValueError, KeyError, TypeError) as error:
        print(f"Unsafe inventory error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
