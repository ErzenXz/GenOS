#!/usr/bin/env python3
"""Deny a falsified post-CR3 observation through the production CPU-local gate.

Only the observed value is corrupted after a real private-root activation. The
MMU operation, metadata validator, fatal reporter and entry exclusion stay intact.
This is software denial, not a CPU exception or complete MMU qualification.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
from pathlib import Path
import re
import socket
import subprocess
import sys
import tarfile
import tempfile
import time

from reference_vm import environment, load_profile, qemu_args
from test_exception_entry import firmware_path, replace_once, require_clean_source
from test_kernel_elf import CAPABILITIES_ID, START_ID, QmpMonitor, connect_qmp

OBSERVATION = '    let observed = read_cr3();'
HALT = 'CPU_LOCAL_ROOT_REJECTED reason=hardware-mismatch policy=halt'
ARMED = re.compile(r'^CPU_ROOT_OBSERVATION_ARMED target=0x([0-9a-f]+) actual=0x([0-9a-f]+)'
                   r' observed=0x([0-9a-f]+) previous=0x([0-9a-f]+)$', re.MULTILINE)
STATUS_ID = 'genos-root-status'
FIXTURE = '''    let actual = read_cr3();
    let observed = if CPU_PAGING.sealed_root().is_some_and(|root| target != root) {
        let corrupted = actual ^ PAGE_SIZE;
        crate::serial::print("CPU_ROOT_OBSERVATION_ARMED");
        for (label, value) in [(" target=0x", target), (" actual=0x", actual),
                               (" observed=0x", corrupted), (" previous=0x", active_root())] {
            crate::serial::print(label);
            crate::serial::print_hex(value);
        }
        crate::serial::println("");
        corrupted
    } else { actual };'''


def patch_fixture(root: Path) -> str:
    path = root / 'kernel/src/paging.rs'
    before = path.read_text()
    after = replace_once(before, OBSERVATION, FIXTURE)
    path.write_text(after)
    return ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile='a/kernel/src/paging.rs', tofile='b/kernel/src/paging.rs'))


def validate_log(log: str) -> None:
    lines = log.replace('\r', '').splitlines()
    matches = ARMED.findall('\n'.join(lines))
    if len(matches) != 1 or sum('CPU_ROOT_OBSERVATION_ARMED' in line for line in lines) != 1:
        raise ValueError('missing, duplicated or malformed post-switch observation')
    target, actual, observed, previous = [int(field, 16) for field in matches[0]]
    if (target != actual or observed != actual ^ 4096 or target == previous
            or any(not value or value & ~0x000ffffffffff000 for value in [target, actual, previous])):
        raise ValueError('fixture did not observe a real distinct aligned private-root switch')
    armed = ARMED.search('\n'.join(lines)).group(0)
    proofs = ['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED',
              'PHYSICAL_ALIAS_POLICY_READY identity=only user_alias=sealed direct=nx', armed, HALT]
    if any(lines.count(proof) != 1 for proof in proofs):
        raise ValueError('missing, embedded or duplicated root-denial proof')
    if sum('CPU_LOCAL_ROOT_REJECTED' in line for line in lines) != 1:
        raise ValueError('extra or malformed root refusal')
    positions = [lines.index(proof) for proof in proofs]
    if positions != sorted(positions):
        raise ValueError('root observation or refusal out of order')
    for forbidden in ['EXCEPTION_FRAME', 'EXCEPTION_TERMINAL', 'EXCEPTION_FATAL_HALT', 'USER_FAULT', 'KERNEL PANIC', 'BOOTLOADER_PANIC', '_FAILED',
                      'USERMODE_READY', 'USER_SHELL_READY', 'GENOS_READY', 'CPU_ROOT_PROBE_RETURNED']:
        if any(forbidden in line for line in lines):
            raise ValueError(f'root refusal escaped or faulted: {forbidden}')


def validate_qmp(messages: list[dict]) -> None:
    if (not messages or any(not isinstance(message, dict) for message in messages)
            or not isinstance(messages[0].get('QMP'), dict)
            or sum('QMP' in message for message in messages) != 1):
        raise ValueError('missing QMP greeting')
    for identity in [CAPABILITIES_ID, START_ID]:
        replies = [message for message in messages if message.get('id') == identity]
        if replies != [{'return': {}, 'id': identity}]:
            raise ValueError('missing or invalid QMP capabilities/start acknowledgement')
    replies = [message for message in messages if message.get('id') == STATUS_ID]
    if (len(replies) != 1 or 'error' in replies[0]
            or not isinstance(replies[0].get('return'), dict)
            or replies[0]['return'].get('running') is not True
            or replies[0]['return'].get('status') != 'running'):
        raise ValueError('guest stopped or missing exact live QMP status')
    ordered = [next(index for index, message in enumerate(messages) if message.get('id') == identity)
               for identity in [CAPABILITIES_ID, START_ID, STATUS_ID]]
    resumes = [index for index, message in enumerate(messages) if message.get('event') == 'RESUME']
    if ordered != sorted(ordered) or len(resumes) != 1 or not ordered[0] < resumes[0] < ordered[-1]:
        raise ValueError('missing/duplicate resume or unordered QMP phases')
    if any(('return' in message or 'error' in message)
           and message.get('id') not in {CAPABILITIES_ID, START_ID, STATUS_ID} for message in messages):
        raise ValueError('unexpected QMP response')
    if any(message.get('event') in {'RESET', 'STOP', 'SHUTDOWN', 'WATCHDOG', 'GUEST_PANICKED'}
           for message in messages):
        raise ValueError('guest reset/stopped/exited around claimed containment')


def boot(root: Path, evidence: Path, timeout: int) -> None:
    serial = evidence / 'serial.log'
    with tempfile.TemporaryDirectory(prefix='genos-root-qmp-') as temporary:
        channel = Path(temporary) / 'monitor.sock'
        args = qemu_args(root=root) + [
            '-drive', f'if=pflash,format=raw,readonly=on,file={firmware_path()}',
            '-drive', load_profile(root)['boot_drive'] + ',file=build/genos.img', '-net', 'none',
            '-display', 'none', '-monitor', 'none', '-serial', f'file:{serial}', '-S',
            '-qmp', f'unix:{channel},server=on,wait=off', '-no-reboot']
        (evidence / 'qemu-command.json').write_text(json.dumps(args, indent=2) + '\n')
        with (evidence / 'qemu.log').open('w') as output:
            process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + timeout
                with connect_qmp(channel, process, deadline) as connection, \
                     (evidence / 'qmp-received.jsonl').open('wb') as received, \
                     (evidence / 'qmp-sent.jsonl').open('wb') as sent:
                    monitor = QmpMonitor(connection, received, sent)
                    try:
                        monitor.start(deadline)
                        while time.monotonic() < deadline:
                            if process.poll() is not None:
                                raise RuntimeError('QEMU reset/exited instead of deliberate refusal')
                            log = serial.read_text(errors='replace') if serial.exists() else ''
                            if HALT in log:
                                time.sleep(.5)
                                if process.poll() is not None:
                                    raise RuntimeError('QEMU exited after claimed refusal')
                                wire = (json.dumps({'execute': 'query-status', 'id': STATUS_ID}) + '\n').encode()
                                sent.write(wire); sent.flush()
                                connection.settimeout(min(1.0, max(.001, deadline - time.monotonic())))
                                connection.sendall(wire)
                                while True:
                                    reply = monitor.read(deadline)
                                    if reply is None:
                                        raise RuntimeError('QMP disconnected before status response')
                                    if 'event' in reply:
                                        continue
                                    if reply.get('id') != STATUS_ID:
                                        raise ValueError('unexpected QMP status response identity')
                                    break
                                validate_qmp(monitor.messages)
                                validate_log(serial.read_text(errors='replace'))
                                return
                            if 'KERNEL PANIC' in log or '_FAILED' in log or 'GENOS_READY' in log:
                                raise RuntimeError('guest faulted or continued instead of refusing root observation')
                            time.sleep(.05)
                        raise TimeoutError('no completed root denial within declared deadline')
                    finally:
                        (evidence / 'qmp.json').write_text(json.dumps(monitor.messages, indent=2) + '\n')
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill(); process.wait()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--timeout', type=int, default=60)
    options = parser.parse_args()
    if sys.version_info < (3, 12) or options.timeout <= 0:
        parser.error('Python >=3.12 and positive timeout required')
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.check_output(['git', 'archive', '--format=tar', commit], cwd=source, timeout=30)
    evidence = source / 'build/cpu-paging-evidence' / str(time.time_ns())
    evidence.mkdir(parents=True, exist_ok=False)
    manifest = {'status': 'incomplete', 'commit': commit, 'source_clean': True,
                'archive_sha256': hashlib.sha256(archive).hexdigest(),
                'scope': 'software refusal of falsified observation after real CR3 activation'}
    try:
        manifest.update(environment(Path(firmware_path()), source))
        with tempfile.TemporaryDirectory(prefix='genos-cpu-paging-') as temporary:
            root = Path(temporary) / 'source'; root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as tree:
                tree.extractall(root, filter='data')
            patch = patch_fixture(root)
            (evidence / 'fixture.patch').write_text(patch)
            manifest['fixture_sha256'] = hashlib.sha256(patch.encode()).hexdigest()
            with (evidence / 'build.log').open('w') as output:
                subprocess.run(['cargo', 'xtask', 'build-test'], cwd=root, stdout=output,
                               stderr=subprocess.STDOUT, check=True, timeout=600)
            manifest['image_sha256'] = hashlib.sha256((root / 'build/genos.img').read_bytes()).hexdigest()
            boot(root, evidence, options.timeout)
            manifest['status'] = 'passed'
    except BaseException as error:
        manifest.update(status='failed', failure=f'{type(error).__name__}: {error}')
        raise
    finally:
        manifest['files_sha256'] = {str(path.relative_to(evidence)): hashlib.sha256(path.read_bytes()).hexdigest()
                                    for path in sorted(evidence.rglob('*')) if path.is_file()}
        (evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'CPU_PAGING_PROBE_OK evidence={evidence}', flush=True)


if __name__ == '__main__':
    main()
