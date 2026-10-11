#!/usr/bin/env python3
"""Production coordinator reentry and repeated-initialization refusal.

These software fixtures insert calls, preserving ScopedCell, adapter, reporter,
network/storage state, driver I/O and interrupt entry. They do not inject faults.
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
    'vfs-borrow': 'VFS_TRANSACTION_REJECTED policy=halt',
    'network-borrow': 'NETWORK_ACCESS_REJECTED policy=halt',
    'network-repeat': 'NETWORK_INIT_REJECTED policy=halt',
    'network-device-repeat': 'NETWORK_DEVICE_INIT_REJECTED policy=halt',
    'storage-borrow': 'STORAGE_ACCESS_REJECTED policy=halt',
    'storage-repeat': 'STORAGE_MOUNT_REJECTED policy=halt',
}


def patch_fixture(root: Path, case: str) -> str:
    if case not in HALTS:
        raise ValueError('unknown coordinator access case')
    module = 'runtime' if case == 'vfs-borrow' else 'network' if case.startswith('network') else 'storage'
    names = ['kernel/src/main.rs', f'kernel/src/{module}.rs']
    before = {name: (root / name).read_text() for name in names}
    if any('COORDINATOR_ACCESS_WINDOW' in text or 'pub fn run_borrow_probe' in text for text in before.values()):
        raise ValueError('coordinator fixture already installed')
    after = before.copy()
    window = f'    serial::println("COORDINATOR_ACCESS_WINDOW case={case}");\n'
    returned = '    serial::println("COORDINATOR_ACCESS_RETURNED"); arch::halt_loop();\n'
    if module == 'runtime':
        anchor = '    network::init();'
        after[names[0]] = replace_once(after[names[0]], anchor,
            window + '    runtime::run_borrow_probe();\n' + returned + anchor)
        after[names[1]] += '''
pub fn run_borrow_probe() {
    let mut live = RamVfs::new();
    live.init_root();
    with_vfs_rollback(|snapshot| {
        snapshot.with_snapshot(&mut live, |live, saved| {
            with_vfs_rollback(|_| serial::println("COORDINATOR_ACCESS_RETURNED"));
            assert_eq!(live.count(), saved.count());
        });
    });
}
'''
    elif module == 'network':
        anchor = '    network::init();'
        call = ('    network::run_device_repeat_probe();\n' if case == 'network-device-repeat'
                else '    network::run_borrow_probe();\n' if case.endswith('borrow') else anchor + '\n')
        after[names[0]] = replace_once(after[names[0]], anchor, anchor + '\n' + window + call + returned)
        if case == 'network-device-repeat':
            after[names[1]] += '''
pub fn run_device_repeat_probe() {
    with_network(|stack| {
        let _ = stack.device.discover();
    });
}
'''
        elif case.endswith('borrow'):
            after[names[1]] += '''
pub fn run_borrow_probe() {
    with_network(|stack| {
        let prior = stack.available;
        let _ = config();
        assert_eq!(stack.available, prior);
    });
}
'''
    else:
        # Isolate software ownership before validation boot's intentional user
        # exceptions. No dedicated data disk is attached or required here.
        anchor = '    network::init();'
        setup = '    let mut probe_vfs = RamVfs::new(); probe_vfs.init_root();\n'
        first = '' if case.endswith('borrow') else '    let _ = storage::mount_or_create(&mut probe_vfs);\n'
        call = ('    storage::run_borrow_probe(&mut probe_vfs);\n' if case.endswith('borrow')
                else '    let _ = storage::mount_or_create(&mut probe_vfs);\n')
        after[names[0]] = replace_once(after[names[0]], anchor, setup + first + window + call + returned + anchor)
        if case.endswith('borrow'):
            after[names[1]] += '''
pub fn run_borrow_probe(vfs: &mut RamVfs) {
    with_storage(|state| {
        let prior = state.mount_started;
        let _ = mount_or_create(vfs);
        assert_eq!(state.mount_started, prior);
    });
}
'''
    for name in names:
        (root / name).write_text(after[name])
    return ''.join(''.join(difflib.unified_diff(before[name].splitlines(True), after[name].splitlines(True),
                   fromfile='a/' + name, tofile='b/' + name)) for name in names)


def validate_log(log: str, case: str) -> None:
    if case not in HALTS:
        raise ValueError('unknown coordinator access case')
    lines = log.replace('\r','').splitlines()
    window = f'COORDINATOR_ACCESS_WINDOW case={case}'
    proofs = ['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED',
              'PHYSICAL_ALIAS_POLICY_READY identity=only user_alias=sealed direct=nx',
              'IRQ_HARDWARE_ON', window, HALTS[case]]
    if any(lines.count(proof) != 1 for proof in proofs):
        raise ValueError('missing, embedded or duplicated coordinator proof')
    indexes = [lines.index(proof) for proof in proofs]
    if indexes != sorted(indexes) or lines[lines.index(window):] != [window, HALTS[case]]:
        raise ValueError('unordered coordinator refusal or unexpected continuation')
    if [line for line in lines if '_REJECTED' in line] != [HALTS[case]]:
        raise ValueError('wrong or additional refusal')
    if sum('COORDINATOR_ACCESS_WINDOW' in line for line in lines) != 1:
        raise ValueError('wrong or extra access window')
    if any(token in log for token in ['EXCEPTION_FRAME', 'EXCEPTION_TERMINAL', 'USER_FAULT',
        'KERNEL PANIC', 'BOOTLOADER_PANIC', 'COORDINATOR_ACCESS_RETURNED', 'GENOS_READY', 'USERMODE_READY']):
        raise ValueError('coordinator probe escaped or faulted')
    # No network device/data disk is attached to these ownership-only fixtures;
    # the existing bounded network/storage discovery may report unavailable.
    failures = [line for line in lines if '_FAILED' in line]
    if failures:
        raise ValueError('unexpected coordinator boot failure')


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
    campaign=source/'build/coordinator-access-evidence'/str(time.time_ns())
    failures=[]
    for case in [options.case] if options.case else HALTS:
        evidence=campaign/case;evidence.mkdir(parents=True,exist_ok=False)
        manifest={'status':'incomplete','case':case,'commit':commit,'source_clean':True,
                  'archive_sha256':hashlib.sha256(archive).hexdigest(),
                  'scope':'software coordinator borrow/initialization refusal; no device or CPU fault qualification'}
        try:
            manifest.update(environment(Path(firmware_path()),source))
            with tempfile.TemporaryDirectory(prefix='genos-coordinator-') as temporary:
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
        print(f'COORDINATOR_ACCESS_{"OK" if manifest["status"]=="passed" else "FAILED"} case={case} evidence={evidence}',flush=True)
    if failures:
        raise SystemExit(1)


if __name__=='__main__':
    main()
