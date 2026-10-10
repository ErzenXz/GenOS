"""Host tests of fault identity, stack lifetime, return sites and fixture scope."""
from pathlib import Path
import re
import shutil
from tempfile import TemporaryDirectory
import unittest

from test_exception_harness import valid_log as exception_log
from test_fault_containment import BAD_ADDRESS, CASES, NESTED, READY, RETURNS, SOFTWARE_KERNEL, patch_fixture, validate_log
from test_stack_guards import STACK_SIZES


def valid_log(case):
    lines = ['CPU_MACHINE_CHECK_READY available=1 policy=halt', READY,
             'KERNEL_STACK_GUARDS_READY stacks=7 guards=14 bytes=4096', 'IDT_READONLY_READY']
    regions = {}
    address = 0x100000
    for name, size in STACK_SIZES.items():
        lower, usable, top, end = address, address + 4096, address + 4096 + size, address + 8192 + size
        regions[name] = (lower, usable, top, end)
        lines += [f'STACK_GUARD_REGION name={name} lower=0x{lower:x} usable=0x{usable:x} top=0x{top:x} end=0x{end:x}']
        address = end
    if case in RETURNS:
        lines += ['ENTRY_RETURN_SITE rip=0x400000']
    lines += [f'ENTRY_CASE_ARMED case={case}']
    if case in NESTED or case == 'double-fault':
        if case in {'exception-pf', 'mce-gp', 'nmi-iret', 'nmi-reentry'}:
            parent = NESTED[case][1]
            outer = exception_log('user' if case == 'exception-pf' else 'kernel', vector=parent, error=6 if parent == 14 else 0, cr2=0x40000000b000 if parent == 14 else 0)
            lines += [line for line in outer.splitlines() if line.startswith('EXCEPTION_FRAME')]
        if case in NESTED:
            lines += [f'ENTRY_WINDOW case={case}']
        if case == 'nmi-reentry':
            lines += ['ENTRY_NMI_UNBLOCKED']
        identities = [NESTED[case][:3]] if case in NESTED else [(8, 255, 0)]
        if case == 'nmi-iret':
            identities += [(2, 2, 0)]
        for vector, parent, error in identities:
            pointer = regions['double-fault'][2] - 176
            source = NESTED[case][3] if case in NESTED else 'boot'
            rsp = regions[source][1] + 1024
            kind = 'double-fault' if case == 'double-fault' else 'nested'
            lines += [f'EXCEPTION_TERMINAL kind={kind} vector=0x{vector:x} parent=0x{parent:x} error=0x{error:x}'
                      f' rip=0x400000 cs=0x8 rsp=0x{rsp:x} cr2=0x{BAD_ADDRESS if vector == 14 else 0:x} frame=0x{pointer:x}',
                      'EXCEPTION_TERMINAL_HALT']
    elif case in RETURNS or case in {'nmi-idle', 'mce-idle'}:
        vector, error = RETURNS.get(case, (2 if case == 'nmi-idle' else 18, 0))
        frame_log = exception_log('kernel', vector, error)
        if case in {'syscall-cs', 'irq-cs'}:
            frame_log = frame_log.replace('ss=0x10', 'ss=0x0')
        lines += [line for line in frame_log.splitlines() if line.startswith('EXCEPTION_FRAME') or line == 'EXCEPTION_FATAL_HALT']
    elif case in SOFTWARE_KERNEL:
        lines += ['USER_RETURN_REJECTED origin=kernel policy=halt']
    else:
        lines += ['USER_RETURN_REJECTED origin=user policy=terminate']
        lines += [line for line in exception_log('user', 13).splitlines() if line != 'EXCEPTION_ENTRY_READY vectors=256 fatal_ist=dedicated' and not line.startswith('EXCEPTION_FRAME')]
    return '\n'.join(lines) + '\n'


class ContainmentHarnessTests(unittest.TestCase):
    def test_all_nineteen_fault_and_return_contracts(self):
        self.assertEqual(len(CASES), 19)
        self.assertEqual(len(set(CASES)), 19)
        for case in CASES:
            with self.subTest(case=case):
                validate_log(valid_log(case), case)

    def test_missing_duplicate_forged_and_reordered_records_fail(self):
        for case in CASES:
            good = valid_log(case)
            for line in good.splitlines():
                for bad in [good.replace(line + '\n', ''), good + line + '\n', good.replace(line, 'forged ' + line)]:
                    with self.subTest(case=case, line=line), self.assertRaises(ValueError):
                        validate_log(bad, case)
            with self.subTest(case=case), self.assertRaises(ValueError):
                validate_log('\n'.join(reversed(good.splitlines())), case)

    def test_terminal_frame_must_use_reserved_stack_and_exact_fault(self):
        for case in [*NESTED, 'double-fault']:
            good = valid_log(case)
            for bad in [good.replace('cs=0x8', 'cs=0x33'),
                        re.sub(r'frame=0x[0-9a-f]+', 'frame=0x1', good),
                        good.replace('parent=0x', 'parent=0x1'),
                        good.replace('error=0x', 'error=0x1'),
                        good + 'GENOS_READY\n', good + 'EXCEPTION_FATAL_HALT\n']:
                with self.subTest(case=case), self.assertRaises(ValueError):
                    validate_log(bad, case)

    def test_return_fault_must_be_at_the_iret_instruction(self):
        for case in RETURNS:
            good = valid_log(case)
            with self.subTest(case=case), self.assertRaises(ValueError):
                validate_log(good.replace('ENTRY_RETURN_SITE rip=0x400000', 'ENTRY_RETURN_SITE rip=0x400001'), case)

    def test_interrupt_return_fault_has_exact_null_kernel_ss(self):
        for case in ['syscall-cs', 'irq-cs']:
            with self.subTest(case=case), self.assertRaises(ValueError):
                validate_log(valid_log(case).replace('ss=0x0', 'ss=0x10'), case)

    def test_software_denial_never_counts_as_a_cpu_exception(self):
        for case in ['user-nt', *SOFTWARE_KERNEL]:
            with self.subTest(case=case), self.assertRaises(ValueError):
                validate_log(valid_log(case) + exception_log('kernel'), case)

    def test_fixture_keeps_production_guard_and_policy_intact(self):
        source = Path(__file__).resolve().parents[1]
        names = ['kernel/src/main.rs', 'kernel/src/userspace.rs', 'kernel/src/cpu_context.rs', 'kernel/src/interrupts.rs',
                 'kernel/src/interrupt_entry.S', 'userspace/init/src/main.rs']
        for case in CASES:
            with self.subTest(case=case), TemporaryDirectory() as temporary:
                root = Path(temporary)
                for name in names:
                    path = root / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(source / name, path)
                fixture = patch_fixture(root, case)
                self.assertTrue(fixture)
                self.assertNotIn('entry_guard.S', fixture)
                self.assertNotIn('fn valid_return_context', fixture)
                self.assertNotIn('pub const fn valid_return', fixture)
                self.assertNotIn('kernel/src/scheduler.rs', fixture)
                self.assertNotIn('fn terminate_process_fault', fixture)
                self.assertNotIn('fn init_machine_check', fixture)
                self.assertNotIn('genos_entry_release:', fixture)


if __name__ == '__main__':
    unittest.main()
