#!/usr/bin/env python3
"""Replay retained lifecycle evidence; short passes cannot satisfy the 10000 gate."""
from __future__ import annotations
import argparse
import ast
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile

from test_cpu_paging import CAPABILITIES_ID, START_ID, STATUS_ID, validate_qmp
from test_lifecycle_churn import QUALIFICATION_CYCLES, check_cycles, patch_fixture, validate_log

REQUIRED_FILES = {'build.log', 'fixture.patch', 'qemu-command.json', 'qemu.log',
                  'qmp-received.jsonl', 'qmp-sent.jsonl', 'qmp.json', 'serial.log'}


def archived_template(source: bytes) -> str:
    # Read the archived literal, never import/execute archived Python. Fixture
    # templates evolve; replay must use the exact reviewed source's version.
    values = [node.value for node in ast.parse(source).body
              if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name)
                  and target.id == 'FIXTURE' for target in node.targets)]
    if len(values) != 1:
        raise ValueError('missing or duplicate archived fixture template')
    value = ast.literal_eval(values[0])
    if not isinstance(value, str) or value.count('__CYCLES__') != 1:
        raise ValueError('malformed archived fixture template')
    return value


def audit(root: Path, cycles: int = QUALIFICATION_CYCLES, source: Path | None = None) -> dict:
    check_cycles(cycles)
    manifest = json.loads((root / 'manifest.json').read_text())
    if (manifest.get('status') != 'passed' or manifest.get('source_clean') is not True
            or manifest.get('cycles') != cycles
            or manifest.get('qualifies_10000_cycles') is not (cycles == QUALIFICATION_CYCLES)):
        raise ValueError('failed, dirty, incomplete or wrong-scope lifecycle evidence')
    if re.fullmatch('[0-9a-f]{40}', manifest.get('commit', '')) is None:
        raise ValueError('invalid exact source identity')
    for field in ['archive_sha256', 'fixture_sha256', 'image_sha256']:
        if re.fullmatch('[0-9a-f]{64}', manifest.get(field, '')) is None:
            raise ValueError('missing artifact identity: ' + field)
    files = manifest.get('files_sha256')
    if not isinstance(files, dict) or not REQUIRED_FILES <= files.keys():
        raise ValueError('missing raw lifecycle artifacts')
    for name, digest in files.items():
        relative = Path(name)
        if relative.is_absolute() or '..' in relative.parts or relative == Path('manifest.json'):
            raise ValueError('invalid retained artifact path')
        path = root / relative
        if (path.is_symlink() or not path.resolve().is_relative_to(root.resolve())
                or not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest):
            raise ValueError('missing, substituted or corrupted artifact: ' + name)
    if files['fixture.patch'] != manifest['fixture_sha256']:
        raise ValueError('fixture identity mismatch')
    result = validate_log((root / 'serial.log').read_text(), cycles)
    if manifest.get('result') != result:
        raise ValueError('manifest totals disagree with actual lifecycle records')
    messages = json.loads((root / 'qmp.json').read_text())
    wire = [json.loads(line) for line in (root / 'qmp-received.jsonl').read_bytes().splitlines()]
    if wire != messages:
        raise ValueError('QMP summary differs from retained wire replies')
    sent = [json.loads(line) for line in (root / 'qmp-sent.jsonl').read_bytes().splitlines()]
    if sent != [{'execute': command, 'id': identity} for command, identity in
                [('qmp_capabilities', CAPABILITIES_ID), ('cont', START_ID), ('query-status', STATUS_ID)]]:
        raise ValueError('missing or unexpected QMP commands')
    validate_qmp(messages)
    if source is not None:
        archive = subprocess.check_output(['git', 'archive', '--format=tar', manifest['commit']], cwd=source, timeout=30)
        if hashlib.sha256(archive).hexdigest() != manifest['archive_sha256']:
            raise ValueError('archive identity differs from exact Git source')
        with tempfile.TemporaryDirectory(prefix='genos-churn-replay-') as temporary:
            fixture = Path(temporary)
            with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as tree:
                template = archived_template(tree.extractfile('tools/test_lifecycle_churn.py').read())
                for name in ['kernel/src/main.rs', 'kernel/src/userspace.rs']:
                    member = tree.getmember(name)
                    if not member.isfile():
                        raise ValueError('source fixture input is not a regular file')
                    path = fixture / name; path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(tree.extractfile(member).read())
            expected = patch_fixture(fixture, cycles, template=template)
            if hashlib.sha256(expected.encode()).hexdigest() != manifest['fixture_sha256']:
                raise ValueError('fixture is not the preserved production source plus probe')
    return {'status': 'passed', 'commit': manifest['commit'], **result,
            'qualifies_10000_cycles': cycles == QUALIFICATION_CYCLES,
            'source_archive_replayed': source is not None,
            'reference_environment_match': manifest.get('reference_environment_match'),
            'scope': 'retained source/trace/hash consistency; not a signed attestation, independent review or 24-hour qualification'}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    parser.add_argument('--cycles', type=int, default=QUALIFICATION_CYCLES)
    parser.add_argument('--source', type=Path, default=Path(__file__).resolve().parents[1])
    options = parser.parse_args()
    print(json.dumps(audit(options.evidence, options.cycles, options.source), indent=2))


if __name__ == '__main__':
    main()
