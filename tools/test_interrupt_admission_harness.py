from pathlib import Path
import tempfile
import unittest
from test_interrupt_admission import HALTS,patch_fixture,validate_log


def valid_log(case):
    proofs = ['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED']
    if case != 'before-arch':
        proofs += ['GDT/TSS initialized', 'IDT initialized']
    if case.startswith('repeat-'):
        proofs += ['EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated', 'IRQ_READY']
    if case == 'repeat-after-seal':
        proofs += ['IDT_READONLY_READY',
                   'PHYSICAL_ALIAS_POLICY_READY identity=only user_alias=sealed direct=nx',
                   'IRQ_HARDWARE_ON']
    return '\n'.join(proofs + [f'INTERRUPT_ADMISSION_WINDOW case={case}', HALTS[case], ''])


class InterruptAdmissionHarnessTests(unittest.TestCase):
    def test_all_exact_interrupt_refusals_pass(self):
        for case in HALTS:
            validate_log(valid_log(case),case)

    def test_missing_duplicate_embedded_reordered_or_continued_proof_fails(self):
        for case in HALTS:
            good=valid_log(case)
            for line in good.splitlines():
                for bad in [good.replace(line+'\n',''),good+line+'\n',good.replace(line,'forged '+line),
                            good.replace(line+'\n','')+line+'\n']:
                    if bad!=good:
                        with self.subTest(case=case,line=line),self.assertRaises(ValueError):
                            validate_log(bad,case)
            with self.assertRaises(ValueError):
                validate_log(good+'unexpected output\n',case)

    def test_wrong_refusal_faults_unknown_windows_and_returned_callbacks_fail(self):
        for case in HALTS:
            good=valid_log(case)
            variants=[good.replace(HALTS[case],HALTS[other]) for other in HALTS if HALTS[other]!=HALTS[case]]
            variants += [token+'\n'+good for token in ['EXCEPTION_FRAME','EXCEPTION_TERMINAL','USER_FAULT',
                'KERNEL PANIC','BOOTLOADER_PANIC','INTERRUPT_ADMISSION_RETURNED','GENOS_READY','USERMODE_READY',
                'OTHER_REJECTED','INTERRUPT_ADMISSION_WINDOW case=unknown','OTHER_FAILED']]
            for bad in variants:
                with self.subTest(case=case,log=bad),self.assertRaises(ValueError):
                    validate_log(bad,case)

    def test_fixture_preserves_hardware_modules_and_refuses_second_installation(self):
        source = Path(__file__).resolve().parents[1]
        name = 'kernel/src/main.rs'
        for case in HALTS:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                path = root / name
                path.parent.mkdir(parents=True)
                original = (source / name).read_text()
                path.write_text(original)
                patch = patch_fixture(root, case)
                changed = path.read_text()
                self.assertIn('INTERRUPT_ADMISSION_WINDOW', patch)
                # Fixture adds calls only: every original line stays in order.
                remaining = iter(changed.splitlines())
                self.assertTrue(all(any(candidate == line for candidate in remaining)
                                    for line in original.splitlines()))
                self.assertEqual(list(root.rglob('*.rs')), [path])
                self.assertEqual(changed.count('    interrupts::init();'),
                                 original.count('    interrupts::init();') + int(case != 'enable-before-init'))
                with self.assertRaises(ValueError):
                    patch_fixture(root, case)
                self.assertEqual(changed, path.read_text())

    def test_early_cases_reject_spurious_later_hardware_readiness(self):
        for case in HALTS:
            good = valid_log(case)
            for marker in ['GDT/TSS initialized', 'IDT initialized', 'IRQ_READY',
                           'EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated',
                           'IDT_READONLY_READY', 'IRQ_HARDWARE_ON']:
                if marker not in good.splitlines():
                    with self.subTest(case=case, marker=marker), self.assertRaises(ValueError):
                        validate_log(marker + '\n' + good, case)

    def test_unknown_case_refuses_before_reading_or_mutating_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                patch_fixture(Path(temporary),'unknown')
        with self.assertRaises(ValueError):
            validate_log('','unknown')


if __name__=='__main__':
    unittest.main()
