import copy
from pathlib import Path
import tempfile
import unittest

from test_cpu_paging_harness import valid_qmp as base_qmp
from test_input_irq import (ARMED, DEFERRED, READY, KEY_ID, KEY_COMMAND, STATUS_ID,
                            patch_fixture, validate_log, validate_qmp)


def valid_log():
    return '\n'.join(['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED',
                      'IRQ_HARDWARE_ON', ARMED, DEFERRED, READY, ''])


def valid_qmp():
    messages = base_qmp()
    messages.insert(-1, {'return': {}, 'id': KEY_ID})
    return messages


class InputIrqHarnessTests(unittest.TestCase):
    def test_exact_deferred_delivery_and_qmp_key_ack_pass(self):
        validate_log(valid_log())
        validate_qmp(valid_qmp())
        self.assertEqual(KEY_COMMAND['arguments']['events'], [
            {'type': 'key', 'data': {'down': True, 'key': {'type': 'qcode', 'data': 'a'}}}])

    def test_missing_duplicate_embedded_reordered_and_continued_proof_fail(self):
        good = valid_log()
        for line in good.splitlines():
            for bad in [good.replace(line + '\n', ''), good + line + '\n',
                        good.replace(line, 'forged ' + line),
                        good.replace(line + '\n', '') + line + '\n']:
                if bad != good:
                    with self.subTest(log=bad), self.assertRaises(ValueError):
                        validate_log(bad)
        for bad in [good + 'unexpected continuation\n', good.replace(DEFERRED, 'diagnostic\n' + DEFERRED)]:
            with self.assertRaises(ValueError):
                validate_log(bad)

    def test_faults_wrong_key_early_irq_or_missing_if_restoration_fail(self):
        good = valid_log()
        for bad in [good.replace('key=97', 'key=98'), good.replace('delivered=1', 'delivered=0'),
                    good.replace('irqs=unchanged', 'irqs=advanced'),
                    good.replace('if=restored', 'if=clear'), good.replace('queue=empty', 'queue=nonempty'),
                    *[token + '\n' + good for token in ['EXCEPTION_FRAME', 'EXCEPTION_TERMINAL',
                      'KERNEL PANIC', 'BOOTLOADER_PANIC', 'INPUT_ACCESS_REJECTED', 'IRQ_FAILED', 'USER_FAULT']]]:
            with self.subTest(log=bad), self.assertRaises(ValueError):
                validate_log(bad)

    def test_qmp_missing_duplicate_rejected_unordered_or_stopped_guest_fail(self):
        good = valid_qmp()
        variants = [good[:-1], [m for m in good if m.get('id') != KEY_ID],
                    good + [{'return': {}, 'id': KEY_ID}], good + [{'event': 'RESET'}],
                    good + [{'event': 'STOP'}], good + [{'event': 'GUEST_PANICKED'}]]
        for identity in [KEY_ID, STATUS_ID]:
            bad = copy.deepcopy(good)
            next(m for m in bad if m.get('id') == identity)['error'] = {'class': 'GenericError'}
            variants.append(bad)
        bad = copy.deepcopy(good)
        bad.insert(1, bad.pop(-2))
        variants.append(bad)
        for bad in variants:
            with self.subTest(messages=bad), self.assertRaises(ValueError):
                validate_qmp(bad)

    def test_fixture_inserts_only_window_and_call_without_replacing_hardware_adapter(self):
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as temporary:
            tree = Path(temporary)
            for name in ['main.rs', 'input_hw.rs']:
                destination = tree / 'kernel/src' / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_text((root / 'kernel/src' / name).read_text())
            original = (tree / 'kernel/src/input_hw.rs').read_text()
            patch = patch_fixture(tree)
            updated = (tree / 'kernel/src/input_hw.rs').read_text()
            self.assertIn(original[original.index('fn require_masked()'):], updated)
            self.assertIn('with_input(|state|', updated)
            self.assertIn('assert_eq!(unsafe { arch::inb(0x64) } & 0x21, 1);', updated)
            self.assertIn('input_hw::run_irq_probe();', patch)
            with self.assertRaises(ValueError):
                patch_fixture(tree)


if __name__ == '__main__':
    unittest.main()
