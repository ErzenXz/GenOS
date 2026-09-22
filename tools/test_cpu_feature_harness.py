"""Host regressions for CPU feature fixture scope and evidence rejection."""
from pathlib import Path
from tempfile import TemporaryDirectory
from contextlib import redirect_stderr, redirect_stdout
import io
import json
import tarfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import test_cpu_features as harness
from test_cpu_features import CASES, FIRMWARE_CASES, ISA, LANES, MISSING, MIXED, POLICY, SAMPLES, cpu_variant, patch_fixture, validate_log
from test_exception_harness import valid_log as exception_log


def valid_log(case):
    if case.startswith(('missing-', 'sample-')):
        marker = ('CPU_PROTECTIONS_UNSUPPORTED required=nx' if case == 'missing-nx'
                  else 'CPU_XSTATE_UNSUPPORTED required=x87,mmx,fxsr,sse,sse2')
        sample = (f'CPU_CPUID_SAMPLE_READY feature={case[7:]} actual=1 admitted_sample=0\n'
                  if case.startswith('sample-') else '')
        return 'BOOT_MEMORY_MAP_VALIDATED\n' + sample + marker + '\n'
    if case.startswith('isa-'):
        name = case[4:]
        vector = 6 if ISA[name][0] == 'ud' else 13
        return (POLICY + '\n' + f'CPU_ISA_FIXTURE_READY instruction={name} advertised=1\n'
                + exception_log(vector=vector))
    smep, smap = MIXED[case[6:]]
    return (POLICY + '\n' + f'CPU_PROTECTIONS_READY nx=1 wp=1 smep={smep} smap={smap}\n'
            'KERNEL_STACK_GUARDS_READY stacks=7 guards=14 bytes=4096\n'
            'GENOS_READY\nNORMAL_SHELL_READY\n')


class CpuFeatureHarnessTests(unittest.TestCase):
    def test_all_twenty_variant_contracts_are_explicit(self):
        self.assertEqual(len(CASES), 20)
        self.assertEqual(len(set(CASES)), 20)
        self.assertEqual(len(LANES['kernel']), 18)
        self.assertEqual(set(FIRMWARE_CASES), {'missing-fxsr', 'missing-sse'})
        self.assertEqual(set(LANES['kernel']) & set(LANES['firmware-limit']), set())
        self.assertEqual(set(LANES['kernel']) | set(LANES['firmware-limit']), set(CASES))
        profile = {'cpu': 'qemu64-v1,smep=on,smap=on'}
        for name in MISSING:
            self.assertEqual(cpu_variant('missing-' + name, profile), profile['cpu'] + ',' + name + '=off')
        for name, (smep, smap) in MIXED.items():
            variant = cpu_variant('mixed-' + name, profile)
            self.assertIn('smep=' + ('on' if smep else 'off'), variant)
            self.assertIn('smap=' + ('on' if smap else 'off'), variant)
        for name in ISA:
            self.assertEqual(cpu_variant('isa-' + name, profile), 'max')
        for name in SAMPLES:
            self.assertEqual(cpu_variant('sample-' + name, profile), profile['cpu'])
        for case in CASES:
            validate_log(valid_log(case), case)

    def test_missing_duplicate_reordered_and_forged_lines_are_rejected(self):
        for case in CASES:
            good = valid_log(case)
            for line in good.splitlines():
                for malformed in (good.replace(line + '\n', ''), good + line + '\n',
                                  good.replace(line + '\n', 'forged ' + line + '\n')):
                    with self.subTest(case=case, line=line), self.assertRaises(ValueError):
                        validate_log(malformed, case)
            with self.subTest(case=case), self.assertRaises(ValueError):
                validate_log('\n'.join(reversed(good.splitlines())), case)

    def test_rejection_cannot_count_a_crash_or_admit_applications(self):
        for name in MISSING:
            case = 'missing-' + name
            forbidden = ['EXCEPTION_FRAME', 'KERNEL PANIC', 'USERMODE_READY',
                         'GENOS_READY', 'NORMAL_SHELL_READY']
            forbidden += ['KERNEL_IMAGE_PROTECTED'] if name == 'nx' else [POLICY, 'GDT/TSS initialized']
            for extra in forbidden:
                with self.subTest(case=case, extra=extra), self.assertRaises(ValueError):
                    validate_log(valid_log(case) + extra + '\n', case)

    def test_mixed_readiness_requires_matching_bits_and_no_fault(self):
        for name in MIXED:
            case = 'mixed-' + name
            good = valid_log(case)
            for malformed in (good.replace('nx=1', 'nx=0'), good.replace('wp=1', 'wp=0'),
                              good.replace('smep=1', 'smep=0') if MIXED[name][0] else good.replace('smep=0', 'smep=1'),
                              good + 'EXCEPTION_FRAME\n', good + 'SOMETHING_FAILED\n'):
                with self.subTest(case=case), self.assertRaises(ValueError):
                    validate_log(malformed, case)

    def test_unsupported_isa_requires_advertised_hardware_exact_fault_and_cleanup(self):
        for name in ISA:
            case = 'isa-' + name
            good = valid_log(case)
            for malformed in (good.replace('advertised=1', 'advertised=0'),
                              good.replace('instruction=' + name, 'instruction=other'),
                              good.replace('cpl=3', 'cpl=0'),
                              good.replace('vector=6', 'vector=13') if ISA[name][0] == 'ud' else good.replace('vector=13', 'vector=6'),
                              good.replace('USER_RECLAIM_OK\n', ''), good + exception_log(),
                              good + 'CPU_ISA_HARDWARE_MISSING\n', good + 'SOMETHING_FAILED\n'):
                with self.subTest(case=case), self.assertRaises(ValueError):
                    validate_log(malformed, case)

    def test_only_deliberate_instruction_and_admission_observation_are_patched(self):
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            init = root / 'userspace/init/src/main.rs'
            userspace = root / 'kernel/src/userspace.rs'
            main = root / 'kernel/src/main.rs'
            init.parent.mkdir(parents=True)
            userspace.parent.mkdir(parents=True)
            for name, (_, statement, _) in ISA.items():
                init.write_text('write_volatile(runtime::STACK_GUARD as *mut u64, token);\n')
                userspace.write_text('if !xstate_probe::run() {\n}\nconst FAULT_EXIT_CODE: u8 = 128 + 14;\n'
                                     'faulting.fault_vector == 14\nfaulting.fault_error == 0x6\n'
                                     'faulting.fault_address == paging::USER_STACK_GUARD\n')
                main.write_text('    serial::println("IDT_READONLY_READY");\n')
                fixture = patch_fixture(root, 'isa-' + name)
                self.assertIn(statement, init.read_text())
                self.assertIn('CPU_ISA_HARDWARE_MISSING', main.read_text())
                self.assertIn('advertised=1', main.read_text())
                self.assertNotIn('interrupts.rs', fixture)
                self.assertNotIn('arch.rs', fixture)
                self.assertNotIn('terminate_process_fault', fixture)
                self.assertNotIn('map_user_page', fixture)
                self.assertIn('isolated exception fixture', userspace.read_text())
            self.assertEqual(patch_fixture(root, 'missing-nx'), '')
            self.assertEqual(patch_fixture(root, 'mixed-base'), '')

    def test_samples_require_real_present_bit_and_preserve_the_admission_policy(self):
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            arch = root / 'kernel/src/arch.rs'
            arch.parent.mkdir(parents=True)
            source = '    if __cpuid(0).eax < 1 || !xstate::supported(__cpuid(1).edx) {\n        return false;\n    }\n'
            for name, mask in SAMPLES.items():
                arch.write_text(source)
                fixture = patch_fixture(root, 'sample-' + name)
                self.assertIn(f'xstate::supported(sample_actual & !{mask})', arch.read_text())
                self.assertIn('return false;', arch.read_text())
                self.assertIn('actual=1 admitted_sample=0', arch.read_text())
                self.assertNotIn('xstate.rs', fixture)
                self.assertNotIn('userspace.rs', fixture)
                good = valid_log('sample-' + name)
                for malformed in (good.replace('actual=1', 'actual=0'),
                                  good.replace('admitted_sample=0', 'admitted_sample=1'),
                                  good + 'CPU_CPUID_SAMPLE_HARDWARE_MISSING\n',
                                  good + 'GDT/TSS initialized\n'):
                    with self.assertRaises(ValueError):
                        validate_log(malformed, 'sample-' + name)

    def test_all_lane_retains_failures_and_continues_without_relabeling(self):
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode='w'):
            pass
        attempted = []

        def fixture(root, case):
            image = root / 'build/genos.img'
            image.parent.mkdir()
            image.write_bytes(b'host-runner-fixture')
            return ''

        def boot(root, evidence, case, timeout):
            attempted.append(case)
            (evidence / 'serial.log').write_text('host-runner-fixture')
            if case in FIRMWARE_CASES:
                raise RuntimeError('retained actual firmware failure')

        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            with (patch.object(harness, '__file__', str(root / 'tools/test_cpu_features.py')),
                  patch.object(harness, 'require_clean_source', return_value='a' * 40),
                  patch.object(harness, 'load_profile', return_value={'cpu': 'qemu64-v1,smep=on,smap=on'}),
                  patch.object(harness, 'environment', return_value={}),
                  patch.object(harness, 'firmware_path', return_value=Path('/host-test-firmware')),
                  patch.object(harness, 'patch_fixture', side_effect=fixture),
                  patch.object(harness, 'boot', side_effect=boot),
                  patch.object(harness.subprocess, 'run', return_value=SimpleNamespace(stdout=archive.getvalue())),
                  patch.object(harness.sys, 'argv', ['test_cpu_features.py', '--lane', 'all']),
                  redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()),
                  self.assertRaises(SystemExit) as stopped):
                harness.main()
            self.assertEqual(stopped.exception.code, 1)
            self.assertEqual(attempted, CASES)
            records = [json.loads(path.read_text()) for path in (root / 'build').glob('cpu-feature-evidence/*/*/manifest.json')]
            self.assertEqual(len(records), 20)
            self.assertEqual(sum(record['status'] == 'passed' for record in records), 18)
            self.assertEqual({record['case'] for record in records if record['status'] == 'failed'}, set(FIRMWARE_CASES))
            self.assertTrue(all(record['commit'] == 'a' * 40 and record['serial_sha256'] for record in records))


if __name__ == '__main__':
    unittest.main()
