from pathlib import Path
import tempfile
import unittest
from test_coordinator_access import HALTS,patch_fixture,validate_log


def valid_log(case):
    return '\n'.join(['BOOT_MODE validation','BOOT_MEMORY_MAP_VALIDATED',
        'PHYSICAL_ALIAS_POLICY_READY identity=only user_alias=sealed direct=nx','IRQ_HARDWARE_ON',
        f'COORDINATOR_ACCESS_WINDOW case={case}',HALTS[case],''])


class CoordinatorAccessHarnessTests(unittest.TestCase):
    def test_all_exact_coordinator_refusals_pass(self):
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
            variants=[good.replace(HALTS[case],HALTS[other]) for other in HALTS if other!=case]
            variants += [token+'\n'+good for token in ['EXCEPTION_FRAME','EXCEPTION_TERMINAL','USER_FAULT',
                'KERNEL PANIC','BOOTLOADER_PANIC','COORDINATOR_ACCESS_RETURNED','GENOS_READY','USERMODE_READY',
                'OTHER_REJECTED','COORDINATOR_ACCESS_WINDOW case=unknown','OTHER_FAILED']]
            for bad in variants:
                with self.subTest(case=case,log=bad),self.assertRaises(ValueError):
                    validate_log(bad,case)

    def test_fixture_preserves_production_modules_and_refuses_second_installation(self):
        source=Path(__file__).resolve().parents[1]
        for case in HALTS:
            module='runtime' if case == 'vfs-borrow' else 'network' if case.startswith('network') else 'storage'
            names=['kernel/src/main.rs',f'kernel/src/{module}.rs']
            with tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary)
                for name in names:
                    p=root/name;p.parent.mkdir(parents=True,exist_ok=True)
                    p.write_text((source/name).read_text())
                original=(root/names[1]).read_text()
                patch=patch_fixture(root,case)
                self.assertTrue((root/names[1]).read_text().startswith(original))
                self.assertIn('COORDINATOR_ACCESS_WINDOW',patch)
                snapshot=[(root/name).read_text() for name in names]
                with self.assertRaises(ValueError):
                    patch_fixture(root,case)
                self.assertEqual(snapshot,[(root/name).read_text() for name in names])

    def test_unknown_case_refuses_before_reading_or_mutating_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                patch_fixture(Path(temporary),'unknown')
        with self.assertRaises(ValueError):
            validate_log('','unknown')


if __name__=='__main__':
    unittest.main()
