#!/usr/bin/env python3
"""Exercise production memory phase/borrow refusal and allocation IRQ exclusion.

Immutable fixtures change only call sites or insert a bounded allocation window.
They preserve initialization, ContextSlot, admission, zeroing, scrubbing and IRQs.
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
from test_cpu_paging import boot as boot_denial
from test_exception_entry import firmware_path, replace_once, require_clean_source
from test_input_irq import boot as boot_irq

HALTS = {
    'before-init': 'MEMORY_PHASE_REJECTED reason=uninitialized policy=halt',
    'init-repeat': 'MEMORY_PHASE_REJECTED reason=already-initialized policy=halt',
    'reentrant': 'MEMORY_ACCESS_REJECTED policy=halt',
}
CASES = [*HALTS, 'irq-deferred']
ARMED = 'MEMORY_ALLOC_IRQ_WINDOW if=clear'
DEFERRED = 'MEMORY_ALLOC_IRQ_DEFERRED pending=keyboard irqs=unchanged queue=empty'
READY = 'MEMORY_ALLOC_IRQ_READY key=97 delivered=1 zeroed=true scrubbed=true reclaimed=true if=restored'
BITMAP = 'FRAME_ALLOCATOR_BITMAP_READY capacity_gib=8'
IRQ_FIXTURE = '''
fn memory_irq_window() {
    assert!(!crate::arch::interrupts_enabled());
    let before = crate::interrupts::stats().keyboard_irqs;
    assert_eq!(crate::input_hw::event_depth(), 0);
    crate::serial::println("MEMORY_ALLOC_IRQ_WINDOW if=clear");
    let deadline = crate::arch::timestamp_cycles().saturating_add(5_000_000_000);
    loop {
        if let Some(byte) = crate::serial::read_byte() {
            assert_eq!(byte, b'!');
            break;
        }
        assert!(crate::arch::timestamp_cycles() < deadline);
        core::hint::spin_loop();
    }
    // SAFETY: read only PS/2 status while IF is clear; the actual IRQ must
    // consume the pending data byte after the memory borrow ends.
    assert_eq!(unsafe { crate::arch::inb(0x64) } & 0x21, 1);
    assert_eq!(crate::interrupts::stats().keyboard_irqs, before);
    assert_eq!(crate::input_hw::event_depth(), 0);
    crate::serial::println("MEMORY_ALLOC_IRQ_DEFERRED pending=keyboard irqs=unchanged queue=empty");
}

pub fn run_memory_irq_probe() {
    assert!(crate::arch::interrupts_enabled());
    let before = crate::interrupts::stats().keyboard_irqs;
    let baseline = allocated_frames();
    let grant = alloc_frame(Owner::KERNEL, Kind::User).expect("probe allocation");
    assert!(crate::arch::interrupts_enabled());
    let deadline = crate::arch::timestamp_cycles().saturating_add(5_000_000_000);
    while crate::interrupts::stats().keyboard_irqs == before {
        assert!(crate::arch::timestamp_cycles() < deadline);
        core::hint::spin_loop();
    }
    assert_eq!(crate::interrupts::stats().keyboard_irqs, before + 1);
    assert_eq!(crate::input_hw::pop_event(), Some(kernel::input::InputEvent::Key(kernel::input::KeyEvent::Char(b'a'))));
    assert_eq!(crate::input_hw::event_depth(), 0);
    assert!(is_live(grant));
    assert_eq!(allocated_frames(), baseline + 1);
    // SAFETY: the fixture owns this fresh, unmapped grant exclusively. No user
    // references, mappings or DMA exist. Keep IF clear across release and the
    // deliberate free-page inspection, before any coordinator can reuse it.
    crate::arch::without_interrupts(|| unsafe {
        assert!(!crate::arch::interrupts_enabled());
        let pointer = grant.address() as *mut u8;
        assert!((0..PAGE_SIZE as usize).all(|i| pointer.add(i).read_volatile() == 0));
        core::ptr::write_bytes(pointer, 0x5a, PAGE_SIZE as usize);
        assert!(free_frame(grant));
        assert!((0..PAGE_SIZE as usize).all(|i| pointer.add(i).read_volatile() == 0));
        assert_eq!(allocated_frames(), baseline);
        assert!(!is_live(grant));
        assert!(!crate::arch::interrupts_enabled());
    });
    assert!(crate::arch::interrupts_enabled());
    crate::serial::println("MEMORY_ALLOC_IRQ_READY key=97 delivered=1 zeroed=true scrubbed=true reclaimed=true if=restored");
    crate::arch::disable_interrupts();
    crate::arch::halt_loop();
}
'''


def patch_fixture(root: Path, case: str) -> str:
    if case not in CASES:
        raise ValueError('unknown memory access case')
    names = ['kernel/src/main.rs', 'kernel/src/memory.rs']
    before = {name: (root / name).read_text() for name in names}
    if any(marker in text for text in before.values()
           for marker in ['MEMORY_ACCESS_WINDOW case=', 'MEMORY_ALLOC_IRQ_WINDOW if=clear']):
        raise ValueError('memory access fixture already installed')
    after = before.copy()
    window = f'    crate::serial::println("MEMORY_ACCESS_WINDOW case={case}");\n'
    returned = '    crate::serial::println("MEMORY_ACCESS_RETURNED"); crate::arch::halt_loop();\n'
    main = 'kernel/src/main.rs'
    memory = 'kernel/src/memory.rs'
    if case in {'before-init', 'init-repeat'}:
        anchor = '    memory::init(boot_info);'
        action = '    memory::allocated_frames();\n' if case == 'before-init' else anchor + '\n'
        prefix = '' if case == 'before-init' else anchor + '\n'
        after[main] = replace_once(after[main], anchor, prefix + window + action + returned)
    elif case == 'reentrant':
        anchor = 'pub fn alloc_frame(owner: Owner, kind: Kind) -> Option<Grant> {\n    with_ready_state(|state| {'
        injection = window + '        allocated_frames();\n' + returned
        after[memory] = replace_once(after[memory], anchor, anchor + '\n' + injection)
    else:
        after[main] = replace_once(after[main], '    interrupts::enable();',
                                  '    interrupts::enable();\n    memory::run_memory_irq_probe();')
        anchor = '        let frame = grant.address();\n        // SAFETY: the bitmap just granted'
        after[memory] = replace_once(after[memory], anchor,
            '        let frame = grant.address();\n        if kind == Kind::User { memory_irq_window(); }\n'
            '        // SAFETY: the bitmap just granted') + IRQ_FIXTURE
    for name in names:
        (root / name).write_text(after[name])
    return ''.join(''.join(difflib.unified_diff(before[name].splitlines(True), after[name].splitlines(True),
                   fromfile='a/' + name, tofile='b/' + name)) for name in names)


def validate_log(log: str, case: str) -> None:
    if case not in CASES:
        raise ValueError('unknown memory access case')
    lines = log.replace('\r', '').splitlines()
    proofs = ['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED', 'RECOVERY_BOUNDARY_OK']
    if case != 'before-init':
        proofs.append(BITMAP)
    tail = [ARMED, DEFERRED, READY] if case == 'irq-deferred' else [f'MEMORY_ACCESS_WINDOW case={case}', HALTS[case]]
    if case == 'irq-deferred':
        proofs.append('IRQ_HARDWARE_ON')
    proofs += tail
    if any(lines.count(proof) != 1 for proof in proofs):
        raise ValueError('missing, embedded or duplicate memory access proof')
    positions = [lines.index(proof) for proof in proofs]
    if positions != sorted(positions) or lines[lines.index(tail[0]):] != tail:
        raise ValueError('memory phases reordered or unexpected continuation')
    if case == 'before-init' and BITMAP in lines:
        raise ValueError('before-init case ran initialization')
    refusals = [line for line in lines if '_REJECTED' in line]
    if refusals != ([] if case == 'irq-deferred' else [HALTS[case]]):
        raise ValueError('wrong or extra memory refusal')
    if any(token in log for token in ['EXCEPTION_FRAME', 'EXCEPTION_TERMINAL', 'EXCEPTION_FATAL_HALT',
        'USER_FAULT', 'KERNEL PANIC', 'BOOTLOADER_PANIC', '_FAILED', 'MEMORY_ACCESS_RETURNED',
        'USERMODE_READY', 'GENOS_READY']):
        raise ValueError('memory probe faulted or continued')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=CASES)
    parser.add_argument('--timeout', type=int, default=60)
    options = parser.parse_args()
    if sys.version_info < (3, 12) or options.timeout <= 0:
        parser.error('Python >=3.12 and positive timeout required')
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.check_output(['git', 'archive', '--format=tar', commit], cwd=source, timeout=30)
    campaign = source / 'build/memory-access-evidence' / str(time.time_ns())
    failures = []
    for case in [options.case] if options.case else CASES:
        evidence = campaign / case
        evidence.mkdir(parents=True, exist_ok=False)
        manifest = {'status': 'incomplete', 'case': case, 'commit': commit, 'source_clean': True,
            'archive_sha256': hashlib.sha256(archive).hexdigest(),
            'scope': ('real keyboard IRQ delayed during granted frame allocation; zero/scrub/reclaim and IF restoration'
                      if case == 'irq-deferred' else 'software memory initialization/borrow refusal before nested access')}
        try:
            manifest.update(environment(Path(firmware_path()), source))
            with tempfile.TemporaryDirectory(prefix='genos-memory-access-') as temporary:
                root = Path(temporary) / 'source'; root.mkdir()
                with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as tree:
                    tree.extractall(root, filter='data')
                patch = patch_fixture(root, case)
                (evidence / 'fixture.patch').write_text(patch)
                manifest['fixture_sha256'] = hashlib.sha256(patch.encode()).hexdigest()
                with (evidence / 'build.log').open('w') as output:
                    subprocess.run(['cargo', 'xtask', 'build-test'], cwd=root, stdout=output,
                                   stderr=subprocess.STDOUT, check=True, timeout=600)
                manifest['image_sha256'] = hashlib.sha256((root / 'build/genos.img').read_bytes()).hexdigest()
                if case == 'irq-deferred':
                    boot_irq(root, evidence, options.timeout, armed_marker=ARMED, ready_marker=READY,
                             validate_serial=lambda log: validate_log(log, case))
                else:
                    boot_denial(root, evidence, options.timeout, halt_marker=HALTS[case],
                                validate_serial=lambda log: validate_log(log, case))
                manifest['status'] = 'passed'
        except BaseException as error:
            manifest.update(status='failed', failure=f'{type(error).__name__}: {error}')
            failures.append(case)
            if isinstance(error, (KeyboardInterrupt, SystemExit)):
                raise
        finally:
            manifest['files_sha256'] = {str(path.relative_to(evidence)): hashlib.sha256(path.read_bytes()).hexdigest()
                                        for path in sorted(evidence.rglob('*')) if path.is_file()}
            (evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        print(f'MEMORY_ACCESS_{"OK" if manifest["status"] == "passed" else "FAILED"} case={case} evidence={evidence}', flush=True)
    if failures:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
