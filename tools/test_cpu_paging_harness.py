import copy
from pathlib import Path
import tempfile
import unittest

from test_cpu_paging import (ARMED, CAPABILITIES_ID, START_ID, STATUS_ID, OBSERVATION,
                             FIXTURE, HALT, patch_fixture, validate_log, validate_qmp)


def valid_log():
    return '\n'.join([
        'BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED',
        'EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated',
        'PHYSICAL_ALIAS_POLICY_READY identity=only user_alias=sealed direct=nx',
        'CPU_ROOT_OBSERVATION_ARMED target=0x5000 actual=0x5000 observed=0x4000 previous=0x2000',
        HALT, '',
    ])


def valid_qmp():
    return [{'QMP': {'version': {}, 'capabilities': []}},
            {'return': {}, 'id': CAPABILITIES_ID}, {'event': 'RESUME'},
            {'return': {}, 'id': START_ID},
            {'return': {'running': True, 'status': 'running'}, 'id': STATUS_ID}]


class CpuPagingHarnessTests(unittest.TestCase):
    def test_exact_denial_requires_distinct_real_root_and_falsified_observation(self):
        validate_log(valid_log())
        validate_qmp(valid_qmp())

    def test_missing_duplicate_embedded_and_reordered_proofs_fail(self):
        good = valid_log()
        for line in good.splitlines():
            if line.startswith('EXCEPTION_ENTRY_READY'):
                continue
            for bad in [good.replace(line + '\n', ''), good + line + '\n',
                        good.replace(line, 'prefix ' + line),
                        good.replace(line + '\n', '') + line + '\n']:
                if bad != good:
                    with self.subTest(line=line, log=bad), self.assertRaises(ValueError):
                        validate_log(bad)

    def test_wrong_roots_faults_refusal_or_continuation_fail(self):
        good = valid_log()
        for bad in [good.replace('actual=0x5000', 'actual=0x6000'),
                    good.replace('observed=0x4000', 'observed=0x5000'),
                    good.replace('previous=0x2000', 'previous=0x5000'),
                    good.replace('previous=0x2000', 'previous=0x0'),
                    good.replace('previous=0x2000', 'previous=0x2001'),
                    good.replace('previous=0x2000', 'previous=0x10000000000000'),
                    good.replace('reason=hardware-mismatch', 'reason=wrong-phase'),
                    good + 'CPU_LOCAL_ROOT_REJECTED malformed\n',
                    *[good + line + '\n' for line in ['EXCEPTION_FRAME malformed', 'EXCEPTION_FATAL_HALT',
                        'USER_FAULT_TERMINATED', 'KERNEL PANIC', 'BOOTLOADER_PANIC', 'USERMODE_READY',
                        'USER_SHELL_READY', 'GENOS_READY', 'CPU_ROOT_PROBE_RETURNED', 'PAGING_FAILED']]]:
            with self.subTest(log=bad), self.assertRaises(ValueError):
                validate_log(bad)

    def test_partial_dispatch_or_unknown_output_after_arming_is_rejected(self):
        good = valid_log()
        for line in ['USER_PREEMPT pid=2', 'USER_SYSCALL_OK', 'USER_EXIT pid=2 code=0',
                     'USER_ELF_LOADED pid=4 root=0x6000', 'unexpected diagnostic']:
            for bad in [good + line + '\n', good.replace(HALT, line + '\n' + HALT)]:
                with self.subTest(log=bad), self.assertRaises(ValueError):
                    validate_log(bad)

    def test_qmp_rejects_stopped_reset_missing_duplicate_malformed_and_unordered_evidence(self):
        good = valid_qmp()
        cases = [good[1:], good + [good[-1]], good + [good[0]], good + [good[2]],
                 [good[0], good[3], good[2], good[1], good[4]],
                 good + [{'return': {}, 'id': 'wrong'}],
                 *[good + [{'event': event}] for event in ['RESET', 'STOP', 'SHUTDOWN', 'WATCHDOG', 'GUEST_PANICKED']]]
        for value in [None, [], {}, {'running': False, 'status': 'running'},
                      {'running': True, 'status': 'paused'}, {'running': 1, 'status': 'running'}]:
            altered = copy.deepcopy(good); altered[-1]['return'] = value; cases.append(altered)
        for index in range(len(good)):
            altered = copy.deepcopy(good); del altered[index]; cases.append(altered)
        altered = copy.deepcopy(good); altered[-1]['error'] = {}; cases.append(altered)
        for messages in cases:
            with self.subTest(messages=messages), self.assertRaises(ValueError):
                validate_qmp(messages)

    def test_fixture_changes_only_observation_not_validator_or_mmu_operation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / 'kernel/src/paging.rs'; path.parent.mkdir(parents=True)
            before = 'write_cr3(target);\n' + OBSERVATION + '\nCPU_PAGING.record_switch(target, observed);\n'
            path.write_text(before)
            patch = patch_fixture(root)
            self.assertEqual(path.read_text(), before.replace(OBSERVATION, FIXTURE))
            self.assertIn('a/kernel/src/paging.rs', patch)
            self.assertNotIn('cpu_paging.rs', patch)
            with self.assertRaises(ValueError):
                patch_fixture(root)
            path.write_text(before + OBSERVATION)
            with self.assertRaises(ValueError):
                patch_fixture(root)


if __name__ == '__main__':
    unittest.main()
