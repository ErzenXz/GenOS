#!/usr/bin/env python3
"""Run the five native integer-state proofs without depending on external DNS.

Build an unchanged clean source archive as a validation image. Success requires
exact ordered proofs, two real #UD terminations, peer exits and live QEMU; source,
image, environment, commands and failed runs are retained independently.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import time

from reference_vm import environment, load_profile, qemu_args
from test_exception_entry import firmware_path, require_clean_source, replace_once

MARKERS = [
    'USER_INTEGER_FRESH_OK processes=6 gprs=15 rsp=initial flags=0x202',
    'USER_INTEGER_DIRECT_OK processes=6 preserved=14 rax=unknown-syscall flags=exact',
    'USER_INTEGER_YIELD_OK processes=6 gprs=15 args=zero flags=exact',
    'USER_INTEGER_TIMER_OK processes=6 captures=12 flags=arithmetic,df,ac,id',
    'USER_INTEGER_REUSE_OK rounds=2 faults=2 vector=6 exits=4 reclaimed=true',
]
ADMISSION = [
    'BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED',
    'CPU_XSTATE_READY mode=fxsave64 bytes=512 user=x87,mmx,sse,sse2 kernel=soft-float',
    'IRQ_HARDWARE_ON', 'USERMODE_READY',
    'USER_CPU_CONTEXT_OK processes=6 selectors=ds,es,fs,gs bases=zero pending_mf=2 fresh=6 reclaimed=true',
]
FAULT = re.compile(r'USER_FAULT_TERMINATED pid=132 vector=6 error=0x([0-9a-f]+) '
                   r'rip=0x([0-9a-f]+) cr2=0x([0-9a-f]+)')
CASES = ('intact', 'gpr-corrupt', 'flags-corrupt', 'timer-skipped')


def patch_fixture(root: Path, case: str) -> str:
    if case == 'intact':
        return ''
    path = root / 'kernel/src/integer_probe.S'
    before = path.read_text()
    if case in ('gpr-corrupt', 'flags-corrupt'):
        # Change only a user-written observation after the direct syscall;
        # production save/restore, comparisons and cleanup are untouched.
        offset = 0 if case == 'gpr-corrupt' else 128
        anchor = '    integer_snapshot 2\n'
        changed = (anchor + '    push rax\n'
                   f'    movabs rax, {{user_data}} + 64 + 2 * 160 + {offset}\n'
                   '    xor qword ptr [rax], 1\n    pop rax\n')
    elif case == 'timer-skipped':
        # Bypass the entire admitted capture interval, not just its condition.
        # Otherwise a heavily delayed VM could still take two legitimate timer
        # captures before the unconditional branch and make rejection flaky.
        anchor = '    integer_snapshot 5\n'
        changed = anchor + '    jmp genos_integer_timer_end\n'
    else:
        raise ValueError('unknown integer fixture')
    if changed in before:
        raise ValueError('integer fixture already applied')
    after = replace_once(before, anchor, changed)
    path.write_text(after)
    return ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                   fromfile='a/kernel/src/integer_probe.S', tofile='b/kernel/src/integer_probe.S'))


def validate_rejection(log: str, case: str) -> None:
    lines = log.replace('\r', '').splitlines()
    required = ADMISSION + ['USER_INTEGER_MISMATCH',
                            'USER_INTEGER_REJECTED reclaimed=true', 'USER_XSTATE_FAILED']
    if case not in CASES[1:] or any(lines.count(marker) != 1 for marker in required):
        raise ValueError('missing exact negative proof or cleanup')
    positions = [lines.index(marker) for marker in required]
    if positions != sorted(positions):
        raise ValueError('negative integer proof out of phase')
    if any(any(marker.split()[0] in line for marker in MARKERS) for line in lines):
        raise ValueError('rejected integer fixture emitted a positive proof')
    if case in ('gpr-corrupt', 'flags-corrupt'):
        for pid in (130, 131, 132):
            if lines.count(f'USER_INTEGER_SAMPLE_MISMATCH pid={pid} sample=2') != 1:
                raise ValueError('corrupted snapshot was not rejected by every peer')
    if any(token in log for token in ('KERNEL PANIC', 'BOOTLOADER_PANIC',
                                      'EXCEPTION_FATAL_HALT', 'EXCEPTION_TERMINAL')):
        raise ValueError('negative fixture crashed instead of clean rejection')


def validate_log(log: str) -> None:
    lines = log.replace('\r', '').splitlines()
    required = ADMISSION + MARKERS
    if any(lines.count(marker) != 1 for marker in required):
        raise ValueError('missing, duplicate or embedded integer admission/proof')
    positions = [lines.index(marker) for marker in required]
    if positions != sorted(positions):
        raise ValueError('integer proofs out of phase')
    for line in lines:
        if line.startswith('USER_INTEGER_') and line not in MARKERS:
            raise ValueError('malformed or failed integer proof')
        if any(token in line for token in ('_FAILED', 'KERNEL PANIC', 'BOOTLOADER_PANIC',
                                          'EXCEPTION_FATAL_HALT', 'EXCEPTION_TERMINAL',
                                          'RECOVERY_CONSOLE_READY')):
            raise ValueError('guest failed before integer qualification')
    start, end = lines.index(ADMISSION[-1]), lines.index(MARKERS[0])
    phase = lines[start + 1:end]
    faults = [(index, line) for index, line in enumerate(lines)
              if line.startswith('USER_FAULT_TERMINATED pid=132 ')]
    if len(faults) != 2 or any(not start < index < end for index, _ in faults):
        raise ValueError('missing or wrong-phase integer process faults')
    for _, line in faults:
        match = FAULT.fullmatch(line)
        if not match or int(match[1], 16) != 0 or int(match[3], 16) != 0:
            raise ValueError('wrong integer process fault')
        if not 0x400000001000 <= int(match[2], 16) < 0x400000002000:
            raise ValueError('fault outside integer ELF code')
    for pid in (130, 131, 132):
        if lines.count(f'USER_YIELD pid={pid}') != 2 or phase.count(f'USER_YIELD pid={pid}') != 2:
            raise ValueError('missing integer process yields')
    for pid in (130, 131):
        if lines.count(f'USER_EXIT pid={pid} code=0') != 2 or phase.count(f'USER_EXIT pid={pid} code=0') != 2:
            raise ValueError('missing healthy peer exits')
    if any(line.startswith('USER_EXIT pid=132 ') for line in lines):
        raise ValueError('faulting peer exited instead of trapping')


def boot(root: Path, evidence: Path, timeout: int, case: str) -> None:
    serial = evidence / 'serial.log'
    args = qemu_args(root=root) + [
        '-drive', f'if=pflash,format=raw,readonly=on,file={firmware_path()}',
        '-drive', load_profile(root)['boot_drive'] + ',file=build/genos.img', '-net', 'none',
        '-display', 'none', '-monitor', 'none', '-serial', f'file:{serial}', '-no-reboot',
    ]
    (evidence / 'qemu-command.json').write_text(json.dumps(args, indent=2) + '\n')
    with (evidence / 'qemu.log').open('w') as output:
        process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                log = serial.read_text(errors='replace') if serial.exists() else ''
                if process.poll() is not None:
                    raise RuntimeError(f'QEMU exited/reset unexpectedly: {process.returncode}')
                if case == 'intact' and ('USER_INTEGER_MISMATCH' in log or 'USER_XSTATE_FAILED' in log):
                    raise RuntimeError('native integer-state fixture failed')
                terminal = MARKERS[-1] if case == 'intact' else 'USER_XSTATE_FAILED'
                if terminal in log:
                    time.sleep(.5)
                    if process.poll() is not None:
                        raise RuntimeError('QEMU exited/reset after apparent proof')
                    log = serial.read_text(errors='replace')
                    if case == 'intact':
                        validate_log(log)
                    else:
                        validate_rejection(log, case)
                    return
                time.sleep(.05)
            raise TimeoutError(f'no integer context proof within {timeout}s')
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=CASES, default='intact')
    parser.add_argument('--timeout', type=int, default=120)
    options = parser.parse_args()
    if sys.version_info < (3, 12) or options.timeout <= 0:
        parser.error('Python >=3.12 and positive timeout required')
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.run(['git', 'archive', '--format=tar', commit], cwd=source,
                             stdout=subprocess.PIPE, check=True, timeout=30).stdout
    evidence = source / 'build/integer-context-evidence' / str(time.time_ns())
    evidence.mkdir(parents=True, exist_ok=False)
    (evidence / 'source.tar').write_bytes(archive)
    manifest = {'status': 'incomplete', 'commit': commit, 'source_clean': True, 'case': options.case,
                'source_sha256': hashlib.sha256(archive).hexdigest(),
                'scope': 'five bounded native integer-state tasks; no physical CPU qualification'}
    try:
        manifest.update(environment(Path(firmware_path()), source))
        with tempfile.TemporaryDirectory(prefix='genos-integer-context-') as temporary:
            root = Path(temporary) / 'source'
            root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as source_archive:
                source_archive.extractall(root, filter='data')
            fixture = patch_fixture(root, options.case)
            (evidence / 'fixture.patch').write_text(fixture)
            manifest['fixture_sha256'] = hashlib.sha256(fixture.encode()).hexdigest()
            command = ['cargo', 'xtask', 'build-test']
            (evidence / 'build-command.json').write_text(json.dumps(command) + '\n')
            with (evidence / 'build.log').open('w') as output:
                subprocess.run(command, cwd=root, stdout=output, stderr=subprocess.STDOUT,
                               check=True, timeout=600)
            image = root / 'build/genos.img'
            manifest['image_sha256'] = hashlib.sha256(image.read_bytes()).hexdigest()
            boot(root, evidence, options.timeout, options.case)
            manifest['status'] = 'passed'
    except Exception as error:
        manifest.update(status='failed', failure=f'{type(error).__name__}: {error}')
        raise
    finally:
        for name in ('serial.log', 'qemu-command.json', 'build-command.json'):
            path = evidence / name
            if path.exists():
                manifest[name.replace('.', '_') + '_sha256'] = hashlib.sha256(path.read_bytes()).hexdigest()
        (evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'INTEGER_CONTEXT_PROBE_OK case={options.case} evidence={evidence}', flush=True)


if __name__ == '__main__':
    main()
