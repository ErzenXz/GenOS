#!/usr/bin/env python3
"""Audit the original100-entry console checklist without changing its denominator."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re

BASELINE_SHA256 = '5886d00657f1f220512112f4198358aa154f2f342ca785202742dcb3b77237d9'

def extract(text: str) -> list[dict]:
    entries = []
    section = ''
    for line in text.splitlines():
        if line.startswith('## '):
            section = 'S' if line.startswith('## Stage 4 continuation') else ''
        elif line.startswith('### '):
            match = re.match(r'### ([FC][0-7]) ', line)
            section = match[1] if match else 'Release' if line == '### Independent console release checklist' else ''
        match = re.fullmatch(r'- \[([ x])\] (.*)', line)
        if section and match:
            entries.append({'section': section, 'text': match[2], 'complete': match[1] == 'x'})
    return entries

def audit(roadmap: str, baseline_bytes: bytes) -> dict:
    if hashlib.sha256(baseline_bytes).hexdigest() != BASELINE_SHA256:
        raise ValueError('Frozen baseline changed; an explicit baseline review is required')
    baseline = json.loads(baseline_bytes)
    current = extract(roadmap)
    expected = [(entry['section'], entry['text']) for entry in baseline['entries']]
    if len(expected) != 100 or sum(entry['baseline_complete'] for entry in baseline['entries']) != 26:
        raise ValueError('Invalid original100-entry,26-completed baseline')
    if [(entry['section'], entry['text']) for entry in current] != expected:
        raise ValueError('Console criteria were added, removed, reordered or rewritten; completion cannot be compared silently')
    groups = {}
    for entry in current:
        section = entry['section']
        group = ('Kernel foundations' if section.startswith('F') else
                 'Storage' if section == 'S' else
                 'Applications and terminal' if section in ('C1','C2','C3','C4') else
                 'Console qualification')
        item = groups.setdefault(group, {'complete': 0, 'total': 0})
        item['complete'] += entry['complete']
        item['total'] += 1
    complete = sum(entry['complete'] for entry in current)
    added = [entry['text'] for entry, original in zip(current, baseline['entries'])
             if entry['complete'] and not original['baseline_complete']]
    return {'baseline_commit': baseline['baseline_commit'], 'baseline_complete': 26,
            'complete': complete, 'total': 100, 'percent': complete, 'target_percent': 40,
            'groups': groups, 'newly_completed': added,
            'meaning': 'Original checklist entries, not effort, time remaining or a stability certification; QEMU console scope excludes GUI and physical hardware.'}

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--json', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        result = audit((root/'ROADMAP.md').read_text(), (root/'tools/console-progress-baseline.json').read_bytes())
    except (OSError, ValueError) as error:
        parser.exit(1, f'Progress audit failed: {error}\n')
    if args.json:
        print(json.dumps(result, indent=2))
    else:
        print(f"{result['percent']}% — {result['complete']}/{result['total']} original console checklist entries (target40%).")
        for name, group in result['groups'].items():
            print(f"{name}: {group['complete']}/{group['total']}")
        print(result['meaning'])
if __name__ == '__main__':
    main()
