#!/usr/bin/env python3
"""Qualify missing/mixed CPU features and disabled user instruction sets.

Actual QEMU CPU feature variants exercise the firmware-to-kernel path. Isolated
ISA fixtures retain production context/fault code and deliberately execute one
instruction in Ring3 on a CPU that advertises its hardware support.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import time

from reference_vm import environment, load_profile, qemu_args
from test_exception_entry import (firmware_path, patch_fixture as exception_fixture,
                                  replace_once, require_clean_source, validate_log as exception_log)

MISSING = {'fpu': 'fpu', 'mmx': 'mmx', 'fxsr': 'fxsr', 'sse': 'sse', 'sse2': 'sse2', 'nx': 'nx'}
MIXED = {'base': (0, 0), 'smep': (1, 0), 'smap': (0, 1), 'both': (1, 1)}
# Source statements are confined to the preexisting deliberately faulting init
# instance. None changes production CPUID admission, trapping or cleanup policy.
ISA = {
    'avx': ('ud', 'core::arch::asm!(".byte 0xc5, 0xfc, 0x57, 0xc0", options(nostack));', '__cpuid(1).ecx & (1 << 28) != 0'),
    'xsave': ('ud', 'core::arch::asm!("xor eax, eax", "xor edx, edx", "xsave64 [{buffer}]", buffer = in(reg) core::ptr::addr_of!(PROCESS_DATA), out("eax") _, out("edx") _, options(nostack));', '__cpuid(1).ecx & (1 << 26) != 0'),
    'xgetbv': ('ud', 'core::arch::asm!("xor ecx, ecx", "xgetbv", out("eax") _, out("edx") _, out("ecx") _, options(nostack));', '__cpuid(1).ecx & (1 << 26) != 0'),
    'wrpkru': ('ud', 'core::arch::asm!("xor eax, eax", "xor ecx, ecx", "xor edx, edx", "wrpkru", out("eax") _, out("edx") _, out("ecx") _, options(nostack));', '__cpuid_count(7, 0).ecx & (1 << 3) != 0'),
    'wrfsbase': ('ud', 'core::arch::asm!("xor eax, eax", "wrfsbase rax", out("rax") _, options(nostack));', '__cpuid_count(7, 0).ebx & 1 != 0'),
    'wrgsbase': ('ud', 'core::arch::asm!("xor eax, eax", "wrgsbase rax", out("rax") _, options(nostack));', '__cpuid_count(7, 0).ebx & 1 != 0'),
    'debug-register': ('gp', 'core::arch::asm!("mov rax, dr0", out("rax") _, options(nostack));', 'true'),
    'rdmsr': ('gp', 'core::arch::asm!("mov ecx, 0xc0000100", "rdmsr", out("eax") _, out("edx") _, out("ecx") _, options(nostack));', '__cpuid(1).edx & (1 << 5) != 0'),
}
CASES = [*[f'missing-{name}' for name in MISSING], *[f'mixed-{name}' for name in MIXED], *[f'isa-{name}' for name in ISA]]
POLICY = 'CPU_XSTATE_READY mode=fxsave64 bytes=512 user=x87,mmx,sse,sse2 kernel=soft-float'


def cpu_variant(case: str, profile: dict[str, str]) -> str:
    if case.startswith('missing-'):
        return profile['cpu'] + ',' + MISSING[case[8:]] + '=off'
    if case.startswith('mixed-'):
        smep, smap = MIXED[case[6:]]
        return f'qemu64-v1,smep={"on" if smep else "off"},smap={"on" if smap else "off"}'
    if case.startswith('isa-'):
        return 'max'
    raise ValueError('unknown CPU case')


def patch_fixture(root: Path, case: str) -> str:
    if not case.startswith('isa-'):
        return ''
    name = case[4:]
    fault, statement, feature = ISA[name]
    paths = ['kernel/src/main.rs', 'kernel/src/userspace.rs', 'userspace/init/src/main.rs']
    before = {name: (root / name).read_text() for name in paths}
    exception_fixture(root, 'user', fault)
    path = root / 'userspace/init/src/main.rs'
    old = ('core::arch::asm!("ud2", options(nostack));' if fault == 'ud'
           else 'core::arch::asm!("cli", options(nostack));')
    path.write_text(replace_once(path.read_text(), old, statement))
    path = root / 'kernel/src/main.rs'
    anchor = '    serial::println("IDT_READONLY_READY");'
    feature = feature.replace('__cpuid_count', 'core::arch::x86_64::__cpuid_count').replace('__cpuid(', 'core::arch::x86_64::__cpuid(')
    check = ('\n    {\n'
             f'      if !({feature}) {{ serial::println("CPU_ISA_HARDWARE_MISSING"); arch::halt_loop(); }}\n'
             f'      serial::println("CPU_ISA_FIXTURE_READY instruction={name} advertised=1"); }}\n')
    path.write_text(replace_once(path.read_text(), anchor, anchor + check))
    return ''.join(''.join(difflib.unified_diff(before[name].splitlines(True), (root / name).read_text().splitlines(True),
                   fromfile='a/' + name, tofile='b/' + name)) for name in paths)


def validate_log(log: str, case: str) -> None:
    lines = log.replace('\r', '').splitlines()
    if case.startswith('isa-'):
        name = case[4:]
        fixture = f'CPU_ISA_FIXTURE_READY instruction={name} advertised=1'
        if lines.count(fixture) != 1 or lines.count(POLICY) != 1 or lines.index(POLICY) >= lines.index(fixture):
            raise ValueError('missing advertised-hardware proof before ISA instruction')
        if any('_FAILED' in line or 'HARDWARE_MISSING' in line or 'KERNEL PANIC' in line for line in lines):
            raise ValueError('ISA fixture reported failed qualification')
        exception_log(log, 'user', ISA[name][0])
        if lines.index(fixture) >= next(index for index, line in enumerate(lines) if line.startswith('EXCEPTION_FRAME')):
            raise ValueError('ISA hardware proof came after fault')
        return
    if case.startswith('missing-'):
        name = case[8:]
        marker = ('CPU_PROTECTIONS_UNSUPPORTED required=nx' if name == 'nx'
                  else 'CPU_XSTATE_UNSUPPORTED required=x87,mmx,fxsr,sse,sse2')
        if lines.count(marker) != 1 or lines.count('BOOT_MEMORY_MAP_VALIDATED') != 1:
            raise ValueError('missing exact kernel CPU admission rejection')
        if lines.index('BOOT_MEMORY_MAP_VALIDATED') >= lines.index(marker):
            raise ValueError('CPU admission rejection was in the wrong phase')
        forbidden = ['USERMODE_READY', 'GENOS_READY', 'NORMAL_SHELL_READY', 'EXCEPTION_FRAME', 'KERNEL PANIC']
        forbidden += (['KERNEL_IMAGE_PROTECTED'] if name == 'nx' else ['GDT/TSS initialized', POLICY])
        if any(any(token in line for token in forbidden) for line in lines):
            raise ValueError('unsupported CPU continued or faulted instead of rejection')
        return
    smep, smap = MIXED[case[6:]]
    required = [POLICY, f'CPU_PROTECTIONS_READY nx=1 wp=1 smep={smep} smap={smap}',
                'KERNEL_STACK_GUARDS_READY stacks=7 guards=14 bytes=4096',
                'NORMAL_SHELL_READY', 'GENOS_READY']
    # Normal shell becomes ready after GENOS_READY; require boot milestones in
    # order and the later operational shell marker without inventing its order.
    for marker in required:
        if lines.count(marker) != 1:
            raise ValueError(f'missing or duplicate mixed-feature readiness: {marker}')
    positions = [lines.index(marker) for marker in required[:3]]
    if positions != sorted(positions) or any('_FAILED' in line or 'EXCEPTION_FRAME' in line or 'KERNEL PANIC' in line for line in lines):
        raise ValueError('mixed feature configuration did not boot cleanly')


def boot(root: Path, evidence: Path, case: str, timeout: int) -> None:
    serial = evidence / 'serial.log'
    profile = load_profile(root)
    args = qemu_args(root=root)
    args[args.index('-cpu') + 1] = cpu_variant(case, profile)
    args += ['-drive', f'if=pflash,format=raw,readonly=on,file={firmware_path()}',
             '-drive', profile['boot_drive'] + ',file=build/genos.img', '-net', 'none',
             '-display', 'none', '-monitor', 'none', '-serial', f'file:{serial}', '-no-reboot']
    (evidence / 'qemu-command.json').write_text(json.dumps(args, indent=2) + '\n')
    marker = ('CPU_PROTECTIONS_UNSUPPORTED required=nx' if case == 'missing-nx' else
              'CPU_XSTATE_UNSUPPORTED' if case.startswith('missing-') else
              'USERMODE_READY' if case.startswith('isa-') else 'NORMAL_SHELL_READY')
    with (evidence / 'qemu.log').open('w') as output:
        process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                log = serial.read_text(errors='replace') if serial.exists() else ''
                if process.poll() is not None:
                    raise RuntimeError(f'QEMU exited/reset unexpectedly: {process.returncode}')
                if marker in log:
                    time.sleep(.5)
                    if process.poll() is not None:
                        raise RuntimeError('QEMU exited/reset after apparent CPU result')
                    validate_log(serial.read_text(errors='replace'), case)
                    return
                time.sleep(.05)
            raise TimeoutError(f'no CPU result within {timeout}s')
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=CASES)
    parser.add_argument('--timeout', type=int, default=120)
    options = parser.parse_args()
    if sys.version_info < (3, 12) or options.timeout <= 0:
        parser.error('Python >=3.12 and positive timeout required')
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.run(['git', 'archive', '--format=tar', commit], cwd=source,
                             stdout=subprocess.PIPE, check=True, timeout=30).stdout
    run = source / 'build/cpu-feature-evidence' / str(time.time_ns())
    for case in ([options.case] if options.case else CASES):
        evidence = run / case
        evidence.mkdir(parents=True, exist_ok=False)
        manifest = {'case': case, 'status': 'incomplete', 'commit': commit, 'source_clean': True,
                    'cpu_variant': cpu_variant(case, load_profile(source)), 'scope': 'explicit CPU feature variant'}
        try:
            manifest.update(environment(Path(firmware_path()), source))
            with tempfile.TemporaryDirectory(prefix='genos-cpu-feature-') as temporary:
                root = Path(temporary) / 'source'
                root.mkdir()
                with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as source_archive:
                    source_archive.extractall(root, filter='data')
                fixture = patch_fixture(root, case)
                (evidence / 'fixture.patch').write_text(fixture)
                manifest['fixture_sha256'] = hashlib.sha256(fixture.encode()).hexdigest()
                with (evidence / 'build.log').open('w') as output:
                    subprocess.run(['cargo', 'xtask', 'build-test' if case.startswith('isa-') else 'build'], cwd=root, stdout=output,
                                   stderr=subprocess.STDOUT, check=True, timeout=600)
                manifest['image_sha256'] = hashlib.sha256((root / 'build/genos.img').read_bytes()).hexdigest()
                boot(root, evidence, case, options.timeout)
                manifest['status'] = 'passed'
        except Exception as error:
            manifest.update(status='failed', failure=f'{type(error).__name__}: {error}')
            raise
        finally:
            serial = evidence / 'serial.log'
            if serial.exists():
                manifest['serial_sha256'] = hashlib.sha256(serial.read_bytes()).hexdigest()
            (evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        print(f'CPU_FEATURE_PROBE_OK case={case} evidence={evidence}', flush=True)


if __name__ == '__main__':
    main()
