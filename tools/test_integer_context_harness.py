import unittest
from pathlib import Path
import tempfile

from test_integer_context import ADMISSION, MARKERS, validate_log, validate_rejection, patch_fixture


class IntegerContextEvidenceTests(unittest.TestCase):
    def records(self):
        round_records = [f'USER_YIELD pid={pid}' for pid in (130, 131, 132)] + [
            'USER_EXIT pid=130 code=0', 'USER_EXIT pid=131 code=0',
            'USER_FAULT_TERMINATED pid=132 vector=6 error=0x0 rip=0x400000001800 cr2=0x0',
        ]
        return ADMISSION + round_records * 2 + MARKERS

    def test_exact_positive_and_every_missing_embedded_duplicate_proof(self):
        records = self.records()
        validate_log('\n'.join(records))
        for marker in ADMISSION + MARKERS:
            with self.assertRaises(ValueError):
                validate_log('\n'.join(line for line in records if line != marker))
            with self.assertRaises(ValueError):
                validate_log('\n'.join(records + [marker]))
            with self.assertRaises(ValueError):
                validate_log('\n'.join(records).replace(marker, 'forged ' + marker))

    def test_order_fault_identity_peer_completion_and_flag_proof(self):
        good = '\n'.join(self.records())
        for before, after in [
            ('vector=6', 'vector=14'), ('error=0x0', 'error=0x1'),
            ('cr2=0x0', 'cr2=0x1'), ('rip=0x400000001800', 'rip=0x400000002000'),
            ('USER_EXIT pid=130 code=0', 'USER_EXIT pid=130 code=1'),
            ('USER_YIELD pid=132', 'USER_YIELD pid=133'),
            ('captures=12', 'captures=11'), ('flags=exact', 'flags=masked'),
        ]:
            with self.assertRaises(ValueError):
                validate_log(good.replace(before, after))
        for failure in ('USER_INTEGER_MISMATCH', 'KERNEL PANIC', 'USER_XSTATE_FAILED',
                        'USER_EXIT pid=132 code=0'):
            with self.assertRaises(ValueError):
                validate_log(good + '\n' + failure)
        for index in range(len(MARKERS) - 1):
            records = self.records()
            first = records.index(MARKERS[index])
            records[first], records[first + 1] = records[first + 1], records[first]
            with self.assertRaises(ValueError):
                validate_log('\n'.join(records))

    def test_later_or_early_faults_cannot_supply_phase_evidence(self):
        records = self.records()
        fault = records.pop(len(ADMISSION) + 5)
        for position in (0, len(records)):
            wrong = records.copy()
            wrong.insert(position, fault)
            with self.assertRaises(ValueError):
                validate_log('\n'.join(wrong))

    def test_rejection_needs_cleanup_exact_mismatches_and_no_success(self):
        records = ADMISSION + [f'USER_INTEGER_SAMPLE_MISMATCH pid={pid} sample=2'
                               for pid in (130, 131, 132)] + [
            'USER_INTEGER_MISMATCH', 'USER_INTEGER_REJECTED reclaimed=true', 'USER_XSTATE_FAILED',
        ]
        good = '\n'.join(records)
        for case in ('gpr-corrupt', 'flags-corrupt', 'timer-skipped'):
            validate_rejection(good, case)
            for marker in ADMISSION + records[-3:]:
                with self.assertRaises(ValueError):
                    validate_rejection(good.replace(marker, 'missing'), case)
            for marker in MARKERS + ['KERNEL PANIC']:
                with self.assertRaises(ValueError):
                    validate_rejection(good + '\n' + marker, case)
        with self.assertRaises(ValueError):
            validate_rejection(good.replace('sample=2', 'sample=1'), 'gpr-corrupt')

    def test_private_mutations_touch_only_selected_user_observations_or_loop(self):
        assembly = (Path(__file__).resolve().parents[1] / 'kernel/src/integer_probe.S').read_text()
        for case in ('intact', 'gpr-corrupt', 'flags-corrupt', 'timer-skipped'):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                target = root / 'kernel/src/integer_probe.S'
                target.parent.mkdir(parents=True)
                target.write_text(assembly)
                diff = patch_fixture(root, case)
                self.assertEqual(bool(diff), case != 'intact')
                if case != 'intact':
                    self.assertEqual(diff.count('+++ b/'), 1)
                    self.assertIn('+++ b/kernel/src/integer_probe.S', diff)
                    with self.assertRaises(ValueError):
                        patch_fixture(root, case)
