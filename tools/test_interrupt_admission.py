#!/usr/bin/env python3
"""Production interrupt initialization phase and repeat-call refusal.

Fixtures only insert calls in main, preserving all admission and hardware code.
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

from reference_vm import environment
from test_cpu_paging import boot
from test_exception_entry import firmware_path, replace_once, require_clean_source

HALTS = {
    'before-arch': 'INTERRUPT_INIT_REJECTED policy=halt',
    'repeat-before-seal': 'INTERRUPT_INIT_REJECTED policy=halt',
    'repeat-after-seal': 'INTERRUPT_INIT_REJECTED policy=halt',
    'enable-before-init': 'INTERRUPT_ENABLE_REJECTED policy=halt',
}


def patch_fixture(root: Path, case: str) -> str:
    if case not in HALTS:
        raise ValueError('unknown interrupt admission case')
    path = root / 'kernel/src/main.rs'
    before = path.read_text()
    if 'INTERRUPT_ADMISSION_WINDOW' in before:
        raise ValueError('interrupt fixture already installed')
    call = 'interrupts::enable();' if case == 'enable-before-init' else 'interrupts::init();'
    probe = (f'    serial::println("INTERRUPT_ADMISSION_WINDOW case={case}");\n'
             f'    {call}\n'
             '    serial::println("INTERRUPT_ADMISSION_RETURNED"); arch::halt_loop();\n')
    anchor = ('    arch::init();' if case == 'before-arch' else
              '    interrupts::enable();' if case == 'repeat-after-seal' else
              '    interrupts::init();')
    after = replace_once(before, anchor,
        anchor + '\n' + probe if case.startswith('repeat-') else probe + anchor)
    path.write_text(after)
    return ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                   fromfile='a/kernel/src/main.rs', tofile='b/kernel/src/main.rs'))


def validate_log(log: str, case: str) -> None:
    if case not in HALTS:
        raise ValueError('unknown interrupt admission case')
    lines = log.replace('\r', '').splitlines()
    window = f'INTERRUPT_ADMISSION_WINDOW case={case}'
    proofs = ['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED']
    if case != 'before-arch':
        proofs += ['GDT/TSS initialized', 'IDT initialized']
    if case.startswith('repeat-'):
        proofs += ['EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated', 'IRQ_READY']
    if case == 'repeat-after-seal':
        proofs += ['IDT_READONLY_READY',
                   'PHYSICAL_ALIAS_POLICY_READY identity=only user_alias=sealed direct=nx',
                   'IRQ_HARDWARE_ON']
    proofs += [window, HALTS[case]]
    if any(lines.count(proof) != 1 for proof in proofs):
        raise ValueError('missing, embedded or duplicated interrupt proof')
    indexes = [lines.index(proof) for proof in proofs]
    if indexes != sorted(indexes) or lines[lines.index(window):] != [window, HALTS[case]]:
        raise ValueError('unordered interrupt refusal or unexpected continuation')
    readiness = ['GDT/TSS initialized', 'IDT initialized',
                 'EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated', 'IRQ_READY',
                 'IDT_READONLY_READY', 'IRQ_HARDWARE_ON']
    if any(lines.count(proof) != int(proof in proofs) for proof in readiness):
        raise ValueError('unexpected hardware initialization or enablement')
    if [line for line in lines if '_REJECTED' in line] != [HALTS[case]]:
        raise ValueError('wrong or additional refusal')
    if sum('INTERRUPT_ADMISSION_WINDOW' in line for line in lines) != 1:
        raise ValueError('wrong or extra admission window')
    if any(token in log for token in ['EXCEPTION_FRAME', 'EXCEPTION_TERMINAL', 'USER_FAULT',
        'KERNEL PANIC', 'BOOTLOADER_PANIC', 'INTERRUPT_ADMISSION_RETURNED', 'GENOS_READY',
        'USERMODE_READY', 'FAILED']):
        raise ValueError('interrupt probe escaped or faulted')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case',choices=list(HALTS))
    parser.add_argument('--timeout',type=int,default=60)
    options=parser.parse_args()
    if sys.version_info<(3,12) or options.timeout<=0:
        parser.error('Python >=3.12 and positive timeout required')
    source=Path(__file__).resolve().parents[1]
    commit=require_clean_source(source)
    archive=subprocess.check_output(['git','archive','--format=tar',commit],cwd=source,timeout=30)
    campaign=source/'build/interrupt-admission-evidence'/str(time.time_ns())
    failures=[]
    for case in [options.case] if options.case else HALTS:
        evidence=campaign/case;evidence.mkdir(parents=True,exist_ok=False)
        manifest={'status':'incomplete','case':case,'commit':commit,'source_clean':True,
                  'archive_sha256':hashlib.sha256(archive).hexdigest(),
                  'scope':'software boot-phase refusal before IDT/PIC/PIT mutation or STI; no SMP qualification'}
        try:
            manifest.update(environment(Path(firmware_path()),source))
            with tempfile.TemporaryDirectory(prefix='genos-interrupt-') as temporary:
                root=Path(temporary)/'source';root.mkdir()
                with tarfile.open(fileobj=io.BytesIO(archive),mode='r:') as tree:
                    tree.extractall(root,filter='data')
                patch=patch_fixture(root,case)
                (evidence/'fixture.patch').write_text(patch)
                manifest['fixture_sha256']=hashlib.sha256(patch.encode()).hexdigest()
                with (evidence/'build.log').open('w') as output:
                    subprocess.run(['cargo','xtask','build-test'],cwd=root,stdout=output,
                                   stderr=subprocess.STDOUT,check=True,timeout=600)
                manifest['image_sha256']=hashlib.sha256((root/'build/genos.img').read_bytes()).hexdigest()
                boot(root,evidence,options.timeout,halt_marker=HALTS[case],
                     validate_serial=lambda log:validate_log(log,case))
                manifest['status']='passed'
        except BaseException as error:
            manifest.update(status='failed',failure=f'{type(error).__name__}: {error}')
            failures.append(case)
            if isinstance(error,(KeyboardInterrupt,SystemExit)):
                raise
        finally:
            manifest['files_sha256']={str(p.relative_to(evidence)):hashlib.sha256(p.read_bytes()).hexdigest()
                                      for p in sorted(evidence.rglob('*')) if p.is_file()}
            (evidence/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
        print(f'INTERRUPT_ADMISSION_{"OK" if manifest["status"]=="passed" else "FAILED"} case={case} evidence={evidence}',flush=True)
    if failures:
        raise SystemExit(1)


if __name__=='__main__':
    main()
