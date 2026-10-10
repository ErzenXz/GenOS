from pathlib import Path
import tempfile
import unittest

from test_memory_access import (CASES, HALTS, ARMED, DEFERRED, READY, BITMAP,
                                patch_fixture, validate_log)


def valid_log(case):
    lines = ['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED', 'RECOVERY_BOUNDARY_OK']
    if case != 'before-init':
        lines.append(BITMAP)
    if case in {'irq-deferred', 'init-if-enabled'}:
        lines.append('IRQ_HARDWARE_ON')
    if case == 'irq-deferred':
        lines += [ARMED, DEFERRED, READY]
    else:
        lines += [f'MEMORY_ACCESS_WINDOW case={case}', HALTS[case]]
    return '\n'.join(lines) + '\n'


class MemoryAccessHarnessTests(unittest.TestCase):
    def test_all_exact_phase_borrow_and_delayed_allocation_proofs_pass(self):
        for case in CASES:
            validate_log(valid_log(case), case)

    def test_missing_duplicate_embedded_and_reordered_proofs_fail(self):
        for case in CASES:
            good = valid_log(case)
            for line in good.splitlines():
                for bad in [good.replace(line + '\n', ''), good + line + '\n',
                            good.replace(line, 'forged ' + line),
                            good.replace(line + '\n', '') + line + '\n']:
                    if bad != good:
                        with self.subTest(case=case, log=bad), self.assertRaises(ValueError):
                            validate_log(bad, case)

    def test_wrong_phase_reason_early_irq_unrestored_if_faults_and_continuation_fail(self):
        for case in CASES:
            good = valid_log(case)
            variants = [good + 'unexpected continuation\n', *[token + '\n' + good for token in [
                'EXCEPTION_FRAME', 'EXCEPTION_TERMINAL', 'EXCEPTION_FATAL_HALT', 'USER_FAULT',
                'KERNEL PANIC', 'BOOTLOADER_PANIC', 'MEMORY_ACCESS_RETURNED', 'MEMORY_OTHER_REJECTED',
                'MEMORY_TEST_FAILED', 'USERMODE_READY', 'GENOS_READY']]]
            if case == 'before-init':
                variants.append(BITMAP + '\n' + good)
            if case in HALTS:
                variants += [good.replace(HALTS[case], HALTS[other]) for other in HALTS if other != case]
            else:
                variants += [good.replace(old, new) for old, new in [
                    ('irqs=unchanged','irqs=advanced'), ('key=97','key=98'), ('delivered=1','delivered=0'),
                    ('zeroed=true','zeroed=false'), ('scrubbed=true','scrubbed=false'),
                    ('reclaimed=true','reclaimed=false'), ('if=restored','if=clear')]]
            for bad in variants:
                with self.subTest(case=case, log=bad), self.assertRaises(ValueError):
                    validate_log(bad, case)

    def test_fixture_preserves_owner_adapter_and_places_delay_inside_real_allocation(self):
        source = Path(__file__).resolve().parents[1]
        for case in CASES:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for name in ['main.rs','memory.rs']:
                    destination = root / 'kernel/src' / name
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_text((source / 'kernel/src' / name).read_text())
                before = (root / 'kernel/src/memory.rs').read_text()
                patch = patch_fixture(root, case)
                updated = (root / 'kernel/src/memory.rs').read_text()
                self.assertIn(before[:before.index('pub fn alloc_frame(')], updated)
                self.assertIn('ACCESS.run(pointer, || ACCESS.with_current(operation))', updated)
                self.assertIn('.allocate(owner, kind, || state.allocator.alloc_frame())?;', updated)
                if case == 'irq-deferred':
                    grant = updated.index('let frame = grant.address();')
                    window = updated.index('if kind == Kind::User { memory_irq_window(); }')
                    zero = updated.index('core::ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE as usize);')
                    self.assertLess(grant,window)
                    self.assertLess(window,zero)
                    self.assertIn('assert!(free_frame(grant));',updated)
                self.assertIn('MEMORY_',patch)
                snapshot = [(root / 'kernel/src' / name).read_text() for name in ['main.rs','memory.rs']]
                with self.assertRaises(ValueError):
                    patch_fixture(root,case)
                self.assertEqual(snapshot,[(root / 'kernel/src' / name).read_text() for name in ['main.rs','memory.rs']])

    def test_unknown_case_is_rejected_before_fixture_mutation(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                patch_fixture(Path(temporary),'unknown')
        with self.assertRaises(ValueError):
            validate_log('','unknown')


if __name__ == '__main__':
    unittest.main()
