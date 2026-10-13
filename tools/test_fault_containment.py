#!/usr/bin/env python3
"""Real entry/IRET faults and QMP NMI/MCE delivery on immutable source fixtures.

Only deliberate fault sites, return words and test barriers are patched. The
production entry guard, terminal reporter, classification and cleanup stay intact.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
from pathlib import Path
import re
import socket
import subprocess
import sys
import tarfile
import tempfile
import time

import cpu_backend
from reference_vm import environment, load_profile, qemu_args
from test_exception_entry import FRAME, USER_READY, firmware_path, replace_once, require_clean_source
from test_exception_entry import patch_fixture as exception_fixture
from test_stack_guards import REGION, STACK_SIZES

BAD_ADDRESS = 0x7ffffffff000
NESTED = {'irq-pf': (14, 32, 2, 'irq'), 'syscall-gp': (13, 128, 0x38, 'privilege'),
          'exception-pf': (14, 14, 2, 'irq'), 'irq-nmi': (2, 32, 0, 'irq'),
          'irq-mce': (18, 32, 0, 'irq'), 'mce-gp': (13, 18, 0x38, 'machine-check'),
          'nmi-iret': (13, 2, 0x38, 'nmi'), 'nmi-reentry': (2, 2, 0, 'nmi')}
RETURNS = {'initial-cs': (13, 0x38), 'initial-rip': (13, 0),
           'syscall-cs': (13, 0x38), 'irq-cs': (13, 0x38)}
SOFTWARE_KERNEL = {
    'initial-flags': ('rflags: 0x202,', 'rflags: 2,'),
    'initial-rsp-reject': ('rsp: stack_top,', 'rsp: 0x0000800000000000,'),
    'initial-ss-reject': ('ss: crate::segments::USER_DATA as u64,', 'ss: 0x10,'),
}
CASES = [*NESTED, *RETURNS, 'double-fault', *SOFTWARE_KERNEL, 'user-nt', 'nmi-idle', 'mce-idle', 'syscall-borrow']
BORROW_HALT = 'CPU_LOCAL_ACCESS_REJECTED policy=halt'
READY = 'EXCEPTION_NESTING_READY policy=halt terminal=double-fault ist_slots=5'
TERMINAL = re.compile(r'^EXCEPTION_TERMINAL kind=(nested|double-fault) vector=0x([0-9a-f]+)'
                     r' parent=0x([0-9a-f]+) error=0x([0-9a-f]+) rip=0x([0-9a-f]+)'
                     r' cs=0x([0-9a-f]+) rsp=0x([0-9a-f]+) cr2=0x([0-9a-f]+)'
                     r' frame=0x([0-9a-f]+)$', re.MULTILINE)
GP = 'core::arch::asm!("mov ax, 0x38", "mov ds, ax", out("rax") _, options(nostack));'
PF = 'core::arch::asm!("mov rax, 0x7ffffffff000", "mov [rax], rax", out("rax") _, options(nostack));'
WAIT = 'crate::arch::halt_loop();'


def patch_fixture(root: Path, case: str) -> str:
    paths = ['kernel/src/main.rs', 'kernel/src/userspace.rs', 'kernel/src/cpu_context.rs', 'kernel/src/interrupts.rs',
             'kernel/src/interrupt_entry.S', 'userspace/init/src/main.rs']
    before = {name: (root / name).read_text() for name in paths}

    def edit(name, old, new):
        path = root / name
        path.write_text(replace_once(path.read_text(), old, new))

    if case == 'user-nt':
        exception_fixture(root, 'user', 'gp')
        edit('userspace/init/src/main.rs', 'core::arch::asm!("cli", options(nostack));',
             'core::arch::asm!("pushfq", "or qword ptr [rsp], 0x4000", "popfq"); let _ = runtime::ping();')
    if case in SOFTWARE_KERNEL:
        edit('kernel/src/cpu_context.rs', *SOFTWARE_KERNEL[case])
    if case == 'syscall-borrow':
        edit('kernel/src/userspace.rs',
             'fn dispatch_syscall(frame: &mut UserContext, process: &mut UserProcess) -> u64 {',
             'fn dispatch_syscall(frame: &mut UserContext, process: &mut UserProcess) -> u64 {\n'
             '    crate::serial::println("ENTRY_WINDOW case=syscall-borrow");\n'
             '    with_current_process(|_| crate::serial::println("ENTRY_PROBE_RETURNED"));')
    if case in NESTED:
        if case.startswith('irq-'):
            action = PF if case == 'irq-pf' else WAIT
            anchor = '    TICKS.fetch_add(1, Ordering::Relaxed);'
        elif case == 'syscall-gp':
            action, anchor = GP, '    let frame = unsafe { &mut *frame };'
            # That anchor also occurs in other entries; restrict to the syscall body.
            path = root / 'kernel/src/userspace.rs'
            text = path.read_text()
            prefix, body = text.split('extern "C" fn genos_syscall_rust', 1)
            body = replace_once(body, anchor, f'    crate::serial::println("ENTRY_WINDOW case={case}");\n    unsafe {{ {action} }}\n' + anchor)
            path.write_text(prefix + 'extern "C" fn genos_syscall_rust' + body)
            action = None
        else:
            anchor = '    print_fault(frame, cr2);'
            action = PF if case == 'exception-pf' else GP
            if case in {'nmi-iret', 'nmi-reentry'}:
                action = ('core::arch::asm!("mov rax, rsp", "push 0x10", "push rax", "push 2", '
                          '"push 0x38", "lea rax, [rip + 2f]", "push rax", "iretq", "2:", out("rax") _);')
                if case == 'nmi-reentry':
                    action = action.replace('"push 0x38"', '"push 0x8"') + ' crate::serial::println("ENTRY_NMI_UNBLOCKED"); crate::arch::halt_loop();'
            action = f'if frame.vector == {14 if case == "exception-pf" else 2 if case in {"nmi-iret", "nmi-reentry"} else 18} {{ crate::serial::println("ENTRY_WINDOW case={case}"); unsafe {{ {action} }} crate::serial::println("ENTRY_PROBE_RETURNED"); crate::arch::halt_loop(); }}'
        if action is not None:
            injected = (f'\n    crate::serial::println("ENTRY_WINDOW case={case}");\n    unsafe {{ {action} }}\n'
                        if case.startswith('irq-') else '\n    ' + action + '\n')
            edit('kernel/src/interrupts.rs', anchor, anchor + injected)
    if case in RETURNS:
        path = 'kernel/src/userspace.rs'
        if case.startswith('initial-'):
            if case == 'initial-cs':
                edit(path, 'push qword ptr [rbx + {ctx_cs}]',
                     'push qword ptr [rbx + {ctx_cs}]\n    mov qword ptr [rsp], 0x3b')
            else:
                edit(path, 'push qword ptr [rbx + {ctx_rip}]',
                     'push qword ptr [rbx + {ctx_rip}]\n    movabs rax, 0x0000800000000000\n    mov [rsp], rax')
            edit(path, '    mov rbx, [rbx + {ctx_rbx}]\n    iretq',
                 '    mov rbx, [rbx + {ctx_rbx}]\n    .global genos_probe_iret\ngenos_probe_iret:\n    iretq')
        elif case == 'syscall-cs':
            edit(path, '    pop rax\n    iretq', '    pop rax\n    mov qword ptr [rsp + 8], 0x3b\n    .global genos_probe_iret\ngenos_probe_iret:\n    iretq')
        else:
            corruption = '    mov qword ptr [rsp + 8], 0x3b\n'
            edit('kernel/src/interrupt_entry.S', '    genos_pop_regs\n    iretq\n\n.global genos_irq1_stub',
                 '    genos_pop_regs\n' + corruption + '    .global genos_probe_iret\ngenos_probe_iret:\n    iretq\n\n.global genos_irq1_stub')

    main = root / 'kernel/src/main.rs'
    anchor = '    serial::println("IDT_READONLY_READY");'
    prelude = '''
    for (name, region) in arch::stack_regions() {
        serial::print("STACK_GUARD_REGION name="); serial::print(name);
        for (label, value) in [(" lower=0x", region.lower_guard), (" usable=0x", region.usable_start),
                              (" top=0x", region.top), (" end=0x", region.allocation_end)] {
            serial::print(label); serial::print_hex(value);
        }
        serial::println("");
    }
'''
    if case in RETURNS:
        prelude += '''    unsafe extern "C" { static genos_probe_iret: u8; }
    serial::print("ENTRY_RETURN_SITE rip=0x");
    serial::print_hex(core::ptr::addr_of!(genos_probe_iret) as u64); serial::println("");
'''
    prelude += f'    serial::println("ENTRY_CASE_ARMED case={case}");\n'
    if case in {'nmi-idle', 'mce-idle', 'nmi-iret', 'nmi-reentry', 'mce-gp'}:
        prelude += '    arch::disable_interrupts(); arch::halt_loop();\n'
    if case.startswith('irq-'):
        prelude += '    arch::enable_interrupts(); arch::halt_loop();\n'
    if case == 'double-fault':
        # #GP delivery sees a non-present gate: contributory #NP during #GP
        # becomes a real CPU #DF. Only this one descriptor is changed, before WP.
        edit('kernel/src/main.rs', '    interrupts::init();', '''    interrupts::init();
    unsafe {
        let present = (arch::idt_address() + 13 * 16 + 5) as *mut u8;
        core::ptr::write_volatile(present, core::ptr::read_volatile(present) & !0x80);
    }''')
        prelude += f'    unsafe {{ {GP} }}\n    serial::println("ENTRY_PROBE_RETURNED"); arch::halt_loop();\n'
    main.write_text(replace_once(main.read_text(), anchor, anchor + prelude))
    return ''.join(''.join(difflib.unified_diff(before[name].splitlines(True), (root / name).read_text().splitlines(True),
                   fromfile='a/' + name, tofile='b/' + name)) for name in paths)


def validate_log(log: str, case: str) -> None:
    log = log.replace('\r', '')
    lines = log.splitlines()
    required = ['CPU_MACHINE_CHECK_READY available=1 policy=halt', READY, 'KERNEL_STACK_GUARDS_READY stacks=7 guards=14 bytes=4096',
                'IDT_READONLY_READY', f'ENTRY_CASE_ARMED case={case}']
    if any(lines.count(line) != 1 for line in required) or [lines.index(line) for line in required] != sorted(lines.index(line) for line in required):
        raise ValueError('missing, duplicate or wrong-phase containment admission')
    if 'ENTRY_PROBE_RETURNED' in lines or any('KERNEL PANIC' in line or '_FAILED' in line for line in lines):
        raise ValueError('probe returned or failed')
    regions = {name: tuple(int(value, 16) for value in values) for name, *values in REGION.findall(log)}
    if len(REGION.findall(log)) != 7 or set(regions) != set(STACK_SIZES):
        raise ValueError('missing or duplicate stack geometry')
    ordered = sorted(regions.values())
    for name, (lower, usable, top, end) in regions.items():
        if not lower or any(value % 4096 for value in (lower, usable, top, end)) or usable != lower + 4096 or top - usable != STACK_SIZES[name] or end != top + 4096:
            raise ValueError('invalid stack geometry')
    if any(left[3] > right[0] for left, right in zip(ordered, ordered[1:])):
        raise ValueError('stack overlap')
    terminals = TERMINAL.findall(log)
    frames = FRAME.findall(log)
    armed = lines.index(required[-1])
    if case != 'user-nt' and any(marker in lines for marker in ['GENOS_READY', 'USERMODE_READY']):
        raise ValueError('fatal containment admitted applications')
    if any(not lines.index('IDT_READONLY_READY') < lines.index(match.group(0)) < armed for match in REGION.finditer(log)):
        raise ValueError('stack geometry in the wrong phase')
    if any(line.startswith('EXCEPTION_TERMINAL ') and TERMINAL.fullmatch(line) is None for line in lines):
        raise ValueError('malformed terminal record')
    if any(lines.index(match.group(0)) <= armed for match in [*TERMINAL.finditer(log), *FRAME.finditer(log)]):
        raise ValueError('fault before arming')
    if case == 'syscall-borrow':
        window = 'ENTRY_WINDOW case=syscall-borrow'
        if (lines.count(window) != 1 or lines.count(BORROW_HALT) != 1
                or sum(window in line for line in lines) != 1
                or sum('CPU_LOCAL_ACCESS_REJECTED' in line for line in lines) != 1
                or not armed < lines.index(window) < lines.index(BORROW_HALT)
                or frames or terminals
                or any(line.startswith(('EXCEPTION_FRAME', 'EXCEPTION_TERMINAL', 'EXCEPTION_FATAL_HALT')) for line in lines)):
            raise ValueError('missing exact scoped-borrow denial, wrong phase or fabricated CPU fault')
    elif case in NESTED or case == 'double-fault':
        window = f'ENTRY_WINDOW case={case}'
        first = TERMINAL.search(log)
        if case in NESTED and (first is None or lines.count(window) != 1 or lines.index(window) <= armed
                               or lines.index(window) >= lines.index(first.group(0))):
            raise ValueError('nested fault lacks its active outer-entry window')
        expected = [NESTED[case][:3]] if case != 'double-fault' else [(8, 255, 0)]
        if case == 'nmi-iret':
            expected += [(2, 2, 0)]
        if len(terminals) != len(expected) or lines.count('EXCEPTION_TERMINAL_HALT') != len(expected) or 'EXCEPTION_FATAL_HALT' in lines:
            raise ValueError('wrong terminal count or resumed Rust dispatch')
        for record, identity, match in zip(terminals, expected, TERMINAL.finditer(log)):
            kind, *fields = record
            vector, parent, error, rip, cs, rsp, cr2, pointer = map(lambda value: int(value, 16), fields)
            lower, usable, top, _ = regions['double-fault']
            if (vector, parent, error) != identity or kind != ('double-fault' if case == 'double-fault' else 'nested') or cs != 8 or not rip:
                raise ValueError('wrong terminal fault identity')
            if not usable <= pointer <= top - 176 or cr2 != (BAD_ADDRESS if vector == 14 else 0):
                raise ValueError('terminal frame outside reserved stack or wrong CR2')
            halt = lines.index('EXCEPTION_TERMINAL_HALT', lines.index(match.group(0)))
            if halt != lines.index(match.group(0)) + 1:
                raise ValueError('terminal halt must follow its complete record')
            if case in NESTED and not (case == 'nmi-iret' and vector == 2):
                source = regions[NESTED[case][3]]
                if not source[1] < rsp <= source[2]:
                    raise ValueError('interrupted RSP was not in the declared outer stack')
        expected_frames = 1 if case in {'exception-pf', 'mce-gp', 'nmi-iret', 'nmi-reentry'} else 0
        if len(frames) != expected_frames:
            raise ValueError('nested entry ran Rust or outer frame missing')
        if case == 'nmi-reentry':
            unblocked = 'ENTRY_NMI_UNBLOCKED'
            if lines.count(unblocked) != 1 or not lines.index(window) < lines.index(unblocked) < lines.index(first.group(0)):
                raise ValueError('second NMI lacks its successful IRET control')
        if frames:
            vector, cpl = map(int, frames[0][:2])
            error, rip, cs, flags, rsp, ss, cr2 = [int(value, 16) for value in frames[0][2:]]
            user = case == 'exception-pf'
            if (vector != NESTED[case][1] or cpl != (3 if user else 0) or error != (6 if user else 0)
                    or cs != (0x33 if user else 8) or ss != (0x2b if user else 0x10)
                    or not rip or not rsp or not flags & 2 or cr2 != (0x40000000b000 if user else 0)
                    or lines.index(FRAME.search(log).group(0)) >= lines.index(window)):
                raise ValueError('wrong outer exception or phase')
        if any(marker in lines for marker in ['GENOS_READY', 'USERMODE_READY']):
            raise ValueError('fatal containment continued boot')
    elif case in RETURNS or case in {'nmi-idle', 'mce-idle'}:
        identity = RETURNS.get(case, (2 if case == 'nmi-idle' else 18, 0))
        if len(frames) != 1 or terminals or lines.count('EXCEPTION_FATAL_HALT') != 1:
            raise ValueError('expected one first-level kernel fault and explicit halt')
        vector, cpl = map(int, frames[0][:2])
        error, rip, cs, flags, rsp, ss, cr2 = [int(value, 16) for value in frames[0][2:]]
        if (vector, error) != identity or cpl != 0 or cs != 8 or ss != (0 if case in {'syscall-cs', 'irq-cs'} else 0x10) or not rip or not rsp or cr2 != 0 or not flags & 2:
            raise ValueError('wrong return/event fault frame')
        if case in RETURNS:
            sites = re.findall(r'^ENTRY_RETURN_SITE rip=0x([0-9a-f]+)$', log, re.MULTILINE)
            if len(sites) != 1 or rip != int(sites[0], 16) or lines.index('ENTRY_RETURN_SITE rip=0x' + sites[0]) >= armed:
                raise ValueError('fault did not occur at the actual IRET site')
        if lines.index('EXCEPTION_FATAL_HALT') <= lines.index(FRAME.search(log).group(0)) or 'GENOS_READY' in lines:
            raise ValueError('wrong fatal phase or continued boot')
    else:
        marker = 'USER_RETURN_REJECTED origin=' + ('kernel policy=halt' if case in SOFTWARE_KERNEL else 'user policy=terminate')
        if lines.count(marker) != 1 or lines.index(marker) <= armed or frames or terminals:
            raise ValueError('missing software return-state denial or fabricated CPU fault')
        if case == 'user-nt':
            terminated = re.findall(r'^USER_FAULT_TERMINATED pid=1 vector=13 error=0x0 rip=0x[0-9a-f]+ cr2=0x0$', log, re.MULTILINE)
            if len(terminated) != 1 or any(lines.count(marker) != 1 for marker in USER_READY):
                raise ValueError('return-state denial lacks exact process cleanup/isolation')
            if sum(line.startswith('USER_FAULT_TERMINATED ') for line in lines) != 1:
                raise ValueError('duplicate/wrong process fault record')
            order = [marker, terminated[0], *USER_READY]
            if [lines.index(line) for line in order] != sorted(lines.index(line) for line in order):
                raise ValueError('denial and cleanup out of order')
        elif 'GENOS_READY' in lines:
            raise ValueError('invalid initial context admitted')
    if 'mce' in case and lines.count('CPU_MACHINE_CHECK_READY available=1 policy=halt') != 1:
        raise ValueError('MCE fixture lacks production enable/readback')


def boot(root: Path, evidence: Path, case: str, timeout: int, backend: str = 'tcg') -> None:
    profile = load_profile(root)
    serial = evidence / 'serial.log'
    # AF_UNIX paths have a small limit (104 bytes on macOS). Evidence paths can
    # be longer; keep only the live socket in an owned, short temporary directory.
    temporary_channel = tempfile.TemporaryDirectory(prefix='genos-qmp-', dir='/tmp')
    channel = Path(temporary_channel.name) / 'qmp.sock'
    args = qemu_args(root=root, backend=backend) + ['-drive', f'if=pflash,format=raw,readonly=on,file={firmware_path()}',
            '-drive', profile['boot_drive'] + ',file=build/genos.img', '-net', 'none', '-display', 'none',
            '-monitor', 'none', '-serial', f'file:{serial}', '-qmp', f'unix:{channel},server=on,wait=off', '-no-reboot']
    (evidence / 'qemu-command.json').write_text(json.dumps(args, indent=2) + '\n')
    transcript = []

    def command(request):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(5)
            connection.connect(str(channel))
            stream = connection.makefile('rwb')
            json.loads(stream.readline())
            for payload in [{'execute': 'qmp_capabilities'}, request]:
                transcript.append({'request': payload})
                stream.write((json.dumps(payload) + '\n').encode()); stream.flush()
                while True:
                    response = json.loads(stream.readline())
                    transcript.append({'response': response})
                    if 'return' in response or 'error' in response:
                        break
                if 'error' in response:
                    raise RuntimeError('QMP rejected request: ' + json.dumps(response['error']))
            return response['return']

    with (evidence / 'qemu.log').open('w') as output:
        process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline, injected = time.monotonic() + timeout, 0
            accelerator_verified = False
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError('QEMU exited/reset instead of deliberate containment')
                if not accelerator_verified:
                    if not channel.exists():
                        time.sleep(.05)
                        continue
                    cpu_backend.verify_accelerator(backend, command({'execute': 'query-kvm'}))
                    accelerator_verified = True
                log = serial.read_text(errors='replace') if serial.exists() else ''
                trigger = f'ENTRY_WINDOW case={case}' if case in {'irq-nmi', 'irq-mce'} else f'ENTRY_CASE_ARMED case={case}'
                if injected == 0 and trigger in log and case in {'nmi-idle', 'irq-nmi', 'nmi-iret', 'nmi-reentry', 'mce-idle', 'irq-mce', 'mce-gp'}:
                    request = {'execute': 'inject-nmi'} if 'nmi' in case else {'execute': 'human-monitor-command', 'arguments': {'command-line': 'mce 0 0 0xb200000000000000 0x5 0x0 0x0'}}
                    response = command(request)
                    if 'mce' in case and response != '':
                        raise RuntimeError('machine-check injection was not accepted: ' + str(response))
                    injected = 1
                if injected == 1 and ((case == 'nmi-iret' and 'EXCEPTION_TERMINAL_HALT' in log)
                                      or (case == 'nmi-reentry' and 'ENTRY_NMI_UNBLOCKED' in log)):
                    command({'execute': 'inject-nmi'}); injected = 2
                marker = (BORROW_HALT if case == 'syscall-borrow' else 'USERMODE_READY' if case == 'user-nt' else 'USER_RETURN_REJECTED origin=kernel policy=halt' if case in SOFTWARE_KERNEL
                          else 'EXCEPTION_TERMINAL_HALT' if case in NESTED or case == 'double-fault' else 'EXCEPTION_FATAL_HALT')
                complete = log.count(marker) >= (2 if case == 'nmi-iret' else 1)
                if complete:
                    time.sleep(.5)
                    if process.poll() is not None or not command({'execute': 'query-status'})['running']:
                        raise RuntimeError('guest reset/exited/stopped after apparent containment')
                    validate_log(serial.read_text(errors='replace'), case)
                    return
                if 'ENTRY_PROBE_RETURNED' in log or '_FAILED' in log or 'KERNEL PANIC' in log:
                    raise RuntimeError('guest reported failed containment')
                time.sleep(.05)
            raise TimeoutError('no complete containment result within deadline')
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()
            (evidence / 'qmp.json').write_text(json.dumps(transcript, indent=2) + '\n')
            temporary_channel.cleanup()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=CASES)
    parser.add_argument('--timeout', type=int, default=60)
    parser.add_argument('--cpu-backend', choices=('tcg', 'kvm'), default='tcg')
    options = parser.parse_args()
    if sys.version_info < (3, 12) or options.timeout <= 0:
        parser.error('Python >=3.12 and positive timeout required')
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.run(['git', 'archive', '--format=tar', commit], cwd=source, stdout=subprocess.PIPE, check=True).stdout
    campaign = source / 'build/fault-containment-evidence' / str(time.time_ns())
    failures = []
    for case in [options.case] if options.case else CASES:
        evidence = campaign / case
        evidence.mkdir(parents=True, exist_ok=False)
        manifest = {'case': case, 'commit': commit, 'source_clean': True, 'status': 'incomplete',
                    'cpu_backend': options.cpu_backend,
                    'scope': ('software scoped current-process borrow denial' if case == 'syscall-borrow'
                              else 'software return-state denial' if case in SOFTWARE_KERNEL or case == 'user-nt'
                              else 'real CPU faults; QMP NMI and HMP machine-check injection where named')}
        try:
            manifest['cpu_backend_preflight'] = cpu_backend.preflight(options.cpu_backend)
            manifest.update(environment(firmware_path(), source, backend=options.cpu_backend))
            with tempfile.TemporaryDirectory(prefix='genos-containment-') as temporary:
                root = Path(temporary) / 'source'; root.mkdir()
                with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as tree:
                    tree.extractall(root, filter='data')
                fixture = patch_fixture(root, case)
                (evidence / 'fixture.patch').write_text(fixture)
                manifest['fixture_sha256'] = hashlib.sha256(fixture.encode()).hexdigest()
                with (evidence / 'build.log').open('w') as output:
                    subprocess.run(['cargo', 'xtask', 'build-test'], cwd=root, stdout=output, stderr=subprocess.STDOUT, check=True, timeout=600)
                manifest['image_sha256'] = hashlib.sha256((root / 'build/genos.img').read_bytes()).hexdigest()
                boot(root, evidence, case, options.timeout, backend=options.cpu_backend)
                manifest['status'] = 'passed'
        except Exception as error:
            manifest.update(status='failed', failure=f'{type(error).__name__}: {error}')
            failures.append(case)
        finally:
            for name in ['serial.log', 'qemu.log', 'build.log', 'qmp.json', 'qemu-command.json']:
                path = evidence / name
                if path.exists():
                    manifest[name + '_sha256'] = hashlib.sha256(path.read_bytes()).hexdigest()
            (evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        print(f'FAULT_CONTAINMENT_{"OK" if manifest["status"] == "passed" else "FAILED"} case={case} evidence={evidence}' +
              (f' failure={manifest["failure"]}' if manifest['status'] != 'passed' else ''), flush=True)
    if failures:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
