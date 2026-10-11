from pathlib import Path
import tempfile
import unittest
from test_lifecycle_churn import QUALIFICATION_CYCLES, TASK, check_cycles, patch_fixture, validate_log


def valid_log(cycles=4):
    lines = ['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED', 'IRQ_HARDWARE_ON',
             'USER_PROCESS_GENERATION_STRESS_OK launches=257', 'USER_PID_REUSE_SAFE_OK',
             'USER_STALE_PROCESS_HANDLE_REJECTED_OK', 'PROCESS_GENERATION_STRESS_READY',
             f'LIFECYCLE_CHURN_WINDOW cycles={cycles} baseline=5']
    for cycle in range(1, cycles + 1):
        pid = (3 + cycle - 1) % 255 + 1
        hold = cycle % 2 == 0
        lines += [f'LIFECYCLE_CHURN_BEGIN cycle={cycle} mode={"kill" if hold else "exit"}',
                  f'USER_ELF_LOADED pid={pid} root=0x550000',
                  f'USER_ASYNC_SPAWN pid={pid} task={TASK} mode={"hold" if hold else "normal"}',
                  f'USER_PREEMPT pid={pid}',
                  f'USER_OUTPUT pid={pid} text=hello from INIT.ELF in ring 3']
        if not hold: lines.append(f'USER_EXIT pid={pid} code=0')
        lines.append(f'USER_FRAMES_RECLAIMED pid={pid} frames=10')
        if hold: lines.append(f'USER_KILLED pid={pid} task={TASK}')
        lines += [f'USER_WAIT_REAPED pid={pid}',
                  f'LIFECYCLE_CHURN_CYCLE cycle={cycle} pid={pid} incarnation={cycle} live=5 reclaimed={cycle*10} active=0 slots=0 output=1 consistent=yes if=restored']
    lines += [f'LIFECYCLE_CHURN_COMPLETE cycles={cycles} exits={(cycles+1)//2} kills={cycles//2} reclaimed={cycles*10} baseline=5 ticks=100 active=0 slots=0']
    return '\n'.join(lines) + '\n'


class LifecycleChurnHarnessTests(unittest.TestCase):
    def test_exact_scopes_and_pid_reuse_pass(self):
        for cycles in [2, 3, 255, 256, QUALIFICATION_CYCLES]:
            result = validate_log(valid_log(cycles), cycles)
            self.assertEqual(result['cycles'], cycles)
            self.assertEqual(result['normal_exits'], (cycles + 1) // 2)
            self.assertEqual(result['kills'], cycles // 2)
            self.assertEqual(result['reclaimed_frames'], cycles * 10)
        # A short process may exit before a timer tick; actual Ring 3 output
        # and its terminal transition remain mandatory for every cycle.
        no_preempt = '\n'.join(line for line in valid_log().splitlines()
                               if not line.startswith('USER_PREEMPT')) + '\n'
        validate_log(no_preempt, 4)

    def test_omitted_duplicate_embedded_and_reordered_records_fail(self):
        good = valid_log()
        for line in good.splitlines():
            for bad in [good.replace(line+'\n', ''), good+line+'\n',
                        good.replace(line, 'forged '+line), good.replace(line+'\n', '')+line+'\n']:
                if bad != good and not (line.startswith('USER_PREEMPT')
                                        and bad == good.replace(line+'\n', '')):
                    with self.subTest(line=line), self.assertRaises(ValueError):
                        validate_log(bad, 4)

    def test_resource_identity_execution_and_completion_corruption_fail(self):
        good = valid_log()
        for before, after in [('live=5', 'live=6'), ('incarnation=2', 'incarnation=1'),
                              ('reclaimed=20', 'reclaimed=19'), ('active=0', 'active=1'),
                              ('slots=0', 'slots=1'), ('output=1', 'output=0'),
                              ('consistent=yes', 'consistent=NO'), ('if=restored', 'if=clear'),
                              ('ticks=100', 'ticks=0'), ('cycles=4 exits=2', 'cycles=3 exits=2'),
                              ('kills=2', 'kills=1'), ('mode=kill', 'mode=exit'),
                              ('code=0', 'code=1'), ('frames=10', 'frames=9'),
                              ('USER_PREEMPT pid=4', 'USER_PREEMPT pid=5'),
                              ('pid=4', 'pid=3'), ('USER_OUTPUT', 'USER_FAKE_OUTPUT')]:
            with self.subTest(change=(before, after)), self.assertRaises(ValueError):
                validate_log(good.replace(before, after, 1), 4)
        with self.assertRaises(ValueError): validate_log(good, 3)

    def test_faults_escaped_dispatch_and_extra_records_fail(self):
        good = valid_log()
        marker = 'LIFECYCLE_CHURN_WINDOW cycles=4 baseline=5\n'
        for token in ['KERNEL PANIC', 'BOOTLOADER_PANIC', 'USER_FAULT', 'EXCEPTION_FRAME',
                      'OTHER_FAILED', 'OTHER_REJECTED', 'GENOS_READY', 'USERMODE_READY',
                      'LIFECYCLE_CHURN_UNKNOWN', 'unexpected production trace']:
            bad = good.replace(marker, marker+token+'\n')
            with self.subTest(token=token), self.assertRaises(ValueError): validate_log(bad, 4)
        with self.assertRaises(ValueError): validate_log(good+'unexpected continuation\n', 4)
        for token in ['KERNEL PANIC', 'BOOTLOADER_PANIC', 'OTHER_FAILED']:
            with self.assertRaises(ValueError): validate_log(token+'\n'+good, 4)

    def test_fixture_preserves_all_production_functions(self):
        source = Path(__file__).resolve().parents[1]
        names = ['kernel/src/main.rs', 'kernel/src/userspace.rs']
        originals = {name: (source/name).read_text() for name in names}
        for cycles in [2, QUALIFICATION_CYCLES]:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for name in names:
                    p = root/name; p.parent.mkdir(parents=True, exist_ok=True); p.write_text(originals[name])
                patch = patch_fixture(root, cycles)
                self.assertTrue((root/names[1]).read_text().startswith(originals[names[1]]))
                self.assertEqual((root/names[0]).read_text().replace('\n        userspace::run_lifecycle_churn_probe();', ''), originals[names[0]])
                self.assertIn(f'const CYCLES: u64 = {cycles};', patch)
                self.assertNotIn('unsafe', patch)
                before = [(root/name).read_text() for name in names]
                with self.assertRaises(ValueError): patch_fixture(root, cycles)
                self.assertEqual(before, [(root/name).read_text() for name in names])

    def test_invalid_scope_refuses_before_source_access(self):
        for cycles in [-1, 0, 1, 10_001, True, '10000']:
            with self.subTest(cycles=cycles), self.assertRaises(ValueError): check_cycles(cycles)
            with tempfile.TemporaryDirectory() as temporary, self.assertRaises(ValueError):
                patch_fixture(Path(temporary), cycles)


if __name__ == '__main__':
    unittest.main()
