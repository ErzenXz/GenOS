import tempfile
from pathlib import Path
import unittest

from test_stack_guards import CASES, READY, STACK_SIZES, patch_fixture, validate_log


def valid_log(case):
    records = []
    regions = {}
    base = 0x200_0000
    for name, size in STACK_SIZES.items():
        usable = base + 4096
        top = usable + size
        end = top + 4096
        regions[name] = (base, usable, top, end)
        records.append(f"STACK_GUARD_REGION name={name} lower=0x{base:x} usable=0x{usable:x} top=0x{top:x} end=0x{end:x}")
        base = end
    _, name, side = CASES[case]
    common = ['KERNEL_IMAGE_PROTECTED text=rx rodata=r data=rw-nx', READY,
              'EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated', 'IDT_READONLY_READY', *records]
    if case == 'usage':
        samples = []
        for name, capacity in STACK_SIZES.items():
            touched = capacity - 8192 if name == 'boot' else 8192
            samples.append(f'KERNEL_STACK_USAGE name={name} capacity={capacity} touched={touched} remaining={capacity-touched}')
        return '\n'.join([*common, *samples, 'STACK_USAGE_PROBE_OK stacks=7', ''])
    _, usable, top, _ = regions[name]
    address = usable - 8 if side == "lower" else regions[name][0] if side == "probe" else top
    cr2 = usable - 16 if side == "probe" else address
    rsp = usable if side in ("lower", "probe") else regions['boot'][2] - 256
    return '\n'.join([
        *common,
        f'STACK_GUARD_PROBE_TARGET stack={name} side={side} address=0x{address:x}',
        f'EXCEPTION_FRAME vector=14 cpl=0 error=0x2 rip=0x2001234 cs=0x8 rflags=0x46 rsp=0x{rsp:x} ss=0x10 cr2=0x{cr2:x}',
        'EXCEPTION_FATAL_HALT', '',
    ])


class StackGuardHarnessTests(unittest.TestCase):
    def test_all_scoped_cases_accept_the_exact_fault(self):
        for case in CASES:
            validate_log(valid_log(case), case)

    def test_missing_duplicate_wrong_phase_or_wrong_fault_is_rejected(self):
        for case in CASES:
            if case == 'usage':
                continue
            good = valid_log(case)
            variants = [good.replace(READY, ''), good + good, 'prefix ' + good,
                        good.replace('error=0x2', 'error=0x3'),
                        good.replace('vector=14', 'vector=13'),
                        good.replace('cr2=0x', 'cr2=0x1'),
                        good.replace('rflags=0x46', 'rflags=0x246'),
                        good.replace('cpl=0', 'cpl=3'),
                        good.replace('EXCEPTION_FATAL_HALT\n', '') + 'GENOS_READY\n',
                        good + 'STACK_GUARD_PROBE_RETURNED\n', good + 'EXCEPTION_FATAL_HALT\n',
                        good.replace('IDT_READONLY_READY\n', '') + 'IDT_READONLY_READY\n',
                        good.replace('name=debug ', 'name=boot '),
                        good.replace('lower=0x2000000', 'lower=0x2000001')]
            for malformed in variants:
                with self.subTest(case=case, log=malformed), self.assertRaises(ValueError):
                    validate_log(malformed, case)

    def test_all_guard_metadata_is_required_even_for_one_access(self):
        good = valid_log('boot-lower')
        without_nmi = '\n'.join(line for line in good.splitlines() if 'name=nmi ' not in line)
        with self.assertRaises(ValueError):
            validate_log(without_nmi, 'boot-lower')
        with self.assertRaises(ValueError):
            validate_log(good, 'irq-lower')

    def test_patch_changes_only_the_deliberate_main_fault_site(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / 'kernel/src/main.rs'
            path.parent.mkdir(parents=True)
            source = 'fn main() {\n    serial::println("IDT_READONLY_READY");\n}\n#[panic_handler]\n'
            for case in CASES:
                path.write_text(source)
                patch = patch_fixture(root, case)
                self.assertIn('a/kernel/src/main.rs', patch)
                self.assertIn('STACK_USAGE_PROBE_OK' if case == 'usage' else 'STACK_GUARD_PROBE_RETURNED', path.read_text())
                self.assertNotIn('guard_kernel_page(', path.read_text())
                self.assertNotIn('set_idt_handler(', path.read_text())
                self.assertEqual('push 0' in path.read_text(), CASES[case][2] == 'lower')
            path.write_text(source.replace('IDT_READONLY_READY', 'changed-anchor'))
            with self.assertRaises(ValueError):
                patch_fixture(root, 'boot-lower')

    def test_usage_cannot_be_forged_or_report_a_missing_margin(self):
        good = valid_log('usage')
        for malformed in [good + good, good.replace('name=nmi ', 'name=debug '),
                          good.replace('capacity=2097152', 'capacity=2097151'),
                          good.replace('remaining=8192', 'remaining=0'),
                          good.replace('touched=8192', 'touched=0'),
                          good.replace('STACK_USAGE_PROBE_OK stacks=7', ''),
                          good + 'GENOS_READY\n']:
            with self.assertRaises(ValueError):
                validate_log(malformed, 'usage')

    def test_compiler_probe_cannot_skip_below_the_guard(self):
        good = valid_log('compiler-probe')
        with self.assertRaises(ValueError):
            validate_log(good.replace('rsp=0x2001000', 'rsp=0x1fff000'), 'compiler-probe')


if __name__ == '__main__':
    unittest.main()
