#!/usr/bin/env python3
"""Defer a real PS/2 keyboard IRQ while production input state is borrowed.

The immutable fixture inserts a coordinator window, not a fake handler or queue.
QMP input events follow https://www.qemu.org/docs/master/interop/qemu-qmp-ref.html.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
from pathlib import Path
import socket
import subprocess
import sys
import tarfile
import tempfile
import time

from reference_vm import environment, load_profile, qemu_args
from test_cpu_paging import STATUS_ID, validate_qmp as validate_base_qmp
from test_exception_entry import firmware_path, replace_once, require_clean_source
from test_kernel_elf import START_ID, QmpMonitor, connect_qmp

ARMED = 'INPUT_IRQ_WINDOW_ARMED if=clear'
DEFERRED = 'INPUT_IRQ_DEFERRED pending=keyboard irqs=unchanged queue=empty'
READY = 'INPUT_IRQ_READY key=97 delivered=1 queue=empty if=restored'
KEY_ID = 'genos-input-key'
KEY_COMMAND = {'execute': 'input-send-event', 'arguments': {'events': [
    {'type': 'key', 'data': {'down': True, 'key': {'type': 'qcode', 'data': 'a'}}}]}, 'id': KEY_ID}
FIXTURE = '''
pub fn run_irq_probe() {
    assert!(arch::interrupts_enabled());
    assert_eq!(event_depth(), 0);
    let before = crate::interrupts::stats().keyboard_irqs;
    with_input(|state| {
        assert!(!arch::interrupts_enabled());
        crate::serial::println("INPUT_IRQ_WINDOW_ARMED if=clear");
        let deadline = arch::timestamp_cycles().saturating_add(5_000_000_000);
        loop {
            if let Some(byte) = crate::serial::read_byte() {
                assert_eq!(byte, b'!');
                break;
            }
            assert!(arch::timestamp_cycles() < deadline);
            core::hint::spin_loop();
        }
        // SAFETY: observe only the PS/2 status byte while the production masked
        // scope owns input. Leave the data byte for the actual IRQ handler.
        assert_eq!(unsafe { arch::inb(0x64) } & 0x21, 1);
        assert_eq!(crate::interrupts::stats().keyboard_irqs, before);
        assert_eq!(state.depth(), 0);
        crate::serial::println("INPUT_IRQ_DEFERRED pending=keyboard irqs=unchanged queue=empty");
    });
    assert!(arch::interrupts_enabled());
    let deadline = arch::timestamp_cycles().saturating_add(5_000_000_000);
    while crate::interrupts::stats().keyboard_irqs == before {
        assert!(arch::timestamp_cycles() < deadline);
        core::hint::spin_loop();
    }
    assert_eq!(crate::interrupts::stats().keyboard_irqs, before + 1);
    assert_eq!(peek_event(), Some(InputEvent::Key(kernel::input::KeyEvent::Char(b'a'))));
    assert_eq!(event_depth(), 1);
    assert_eq!(pop_event(), Some(InputEvent::Key(kernel::input::KeyEvent::Char(b'a'))));
    assert_eq!(event_depth(), 0);
    assert!(arch::interrupts_enabled());
    // Exercise a caller whose IF was already clear: nested adapters must not
    // restore it early, including observations and the bounded polling API.
    arch::without_interrupts(|| {
        assert_eq!(peek_event(), None);
        assert_eq!(pop_event(), None);
        assert_eq!(event_depth(), 0);
        let _ = mouse_state();
        poll();
        assert!(!arch::interrupts_enabled());
    });
    assert!(arch::interrupts_enabled());
    crate::serial::println("INPUT_IRQ_READY key=97 delivered=1 queue=empty if=restored");
    arch::halt_loop();
}
'''


def patch_fixture(root: Path) -> str:
    if ('input_hw::run_irq_probe();' in (root / 'kernel/src/main.rs').read_text()
            or 'pub fn run_irq_probe()' in (root / 'kernel/src/input_hw.rs').read_text()):
        raise ValueError('input IRQ fixture already installed')
    patches = []
    for name, anchor, replacement in [
        ('kernel/src/main.rs', '    interrupts::enable();',
         '    interrupts::enable();\n    input_hw::run_irq_probe();'),
        ('kernel/src/input_hw.rs', '\nfn require_masked()', FIXTURE + '\nfn require_masked()'),
    ]:
        path = root / name
        before = path.read_text()
        after = replace_once(before, anchor, replacement)
        path.write_text(after)
        patches.extend(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                                           fromfile='a/' + name, tofile='b/' + name))
    return ''.join(patches)


def validate_log(log: str) -> None:
    lines = log.replace('\r', '').splitlines()
    proofs = ['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED', 'IRQ_HARDWARE_ON', ARMED, DEFERRED, READY]
    if any(lines.count(proof) != 1 for proof in proofs):
        raise ValueError('missing, embedded or duplicated input IRQ proof')
    positions = [lines.index(proof) for proof in proofs]
    if positions != sorted(positions) or lines[positions[-3]:] != [ARMED, DEFERRED, READY]:
        raise ValueError('input IRQ phases reordered or unexpected continuation')
    if any(token in log for token in ['EXCEPTION_FRAME', 'EXCEPTION_TERMINAL', 'USER_FAULT',
        'KERNEL PANIC', 'BOOTLOADER_PANIC', '_FAILED', '_REJECTED', 'USERMODE_READY', 'GENOS_READY']):
        raise ValueError('input probe faulted or continued instead of completing')


def validate_qmp(messages: list[dict]) -> None:
    replies = [message for message in messages if message.get('id') == KEY_ID]
    if replies != [{'return': {}, 'id': KEY_ID}]:
        raise ValueError('missing, duplicate or rejected key event acknowledgement')
    validate_base_qmp([message for message in messages if message.get('id') != KEY_ID])
    indexes = [next(index for index, message in enumerate(messages) if message.get('id') == identity)
               for identity in [START_ID, KEY_ID, STATUS_ID]]
    if indexes != sorted(indexes):
        raise ValueError('key event outside running probe phases')


def request(monitor: QmpMonitor, command: dict, deadline: float) -> dict:
    wire = (json.dumps(command) + '\n').encode()
    monitor.sent.write(wire); monitor.sent.flush()
    monitor.connection.settimeout(min(1.0, max(.001, deadline - time.monotonic())))
    monitor.connection.sendall(wire)
    while True:
        reply = monitor.read(deadline)
        if reply is None:
            raise RuntimeError('QMP disconnected before reply')
        if 'event' in reply:
            continue
        if reply.get('id') != command['id'] or 'error' in reply:
            raise ValueError('QMP command failed or unexpected reply identity')
        return reply


def boot(root: Path, evidence: Path, timeout: int) -> None:
    with tempfile.TemporaryDirectory(prefix='genos-input-irq-') as temporary:
        qmp_path = Path(temporary) / 'qmp.sock'
        uart_path = Path(temporary) / 'uart.sock'
        args = qemu_args(root=root) + [
            '-drive', f'if=pflash,format=raw,readonly=on,file={firmware_path()}',
            '-drive', load_profile(root)['boot_drive'] + ',file=build/genos.img', '-net', 'none',
            '-display', 'none', '-monitor', 'none', '-S', '-no-reboot',
            '-serial', f'unix:{uart_path},server=on,wait=off',
            '-qmp', f'unix:{qmp_path},server=on,wait=off']
        (evidence / 'qemu-command.json').write_text(json.dumps(args, indent=2) + '\n')
        with (evidence / 'qemu.log').open('w') as output:
            process = subprocess.Popen(args, cwd=root, stdout=output, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + timeout
                with connect_qmp(qmp_path, process, deadline) as qmp, \
                     connect_qmp(uart_path, process, deadline) as uart, \
                     (evidence / 'qmp-received.jsonl').open('wb') as received, \
                     (evidence / 'qmp-sent.jsonl').open('wb') as sent, \
                     (evidence / 'serial.log').open('wb') as serial:
                    monitor = QmpMonitor(qmp, received, sent)
                    log = bytearray()
                    injected = False
                    try:
                        monitor.start(deadline)
                        while time.monotonic() < deadline:
                            if process.poll() is not None:
                                raise RuntimeError('QEMU exited/reset during input probe')
                            uart.settimeout(.05)
                            try:
                                data = uart.recv(4096)
                                if not data:
                                    raise RuntimeError('serial disconnected during input probe')
                                serial.write(data); serial.flush(); log.extend(data)
                                if len(log) > 1024 * 1024:
                                    raise ValueError('serial output exceeded byte budget')
                            except socket.timeout:
                                pass
                            text = log.decode(errors='replace')
                            if ARMED + '\n' in text and not injected:
                                if request(monitor, KEY_COMMAND, deadline) != {'return': {}, 'id': KEY_ID}:
                                    raise ValueError('key event was not accepted')
                                uart.sendall(b'!')
                                (evidence / 'serial-sent.bin').write_bytes(b'!')
                                injected = True
                            if READY + '\n' in text:
                                if not injected:
                                    raise ValueError('probe completed without key injection')
                                request(monitor, {'execute': 'query-status', 'id': STATUS_ID}, deadline)
                                while b'\n' in monitor.pending:
                                    monitor.read(deadline)
                                validate_qmp(monitor.messages)
                                validate_log(text)
                                if process.poll() is not None:
                                    raise RuntimeError('QEMU exited after completion')
                                return
                            if any(token in text for token in ['KERNEL PANIC', '_FAILED', '_REJECTED']):
                                raise RuntimeError('input IRQ probe failed in guest')
                        raise TimeoutError('input IRQ probe missed its declared deadline')
                    finally:
                        (evidence / 'qmp.json').write_text(json.dumps(monitor.messages, indent=2) + '\n')
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill(); process.wait()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--timeout', type=int, default=60)
    options = parser.parse_args()
    if sys.version_info < (3, 12) or options.timeout <= 0:
        parser.error('Python >=3.12 and positive timeout required')
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.check_output(['git', 'archive', '--format=tar', commit], cwd=source, timeout=30)
    evidence = source / 'build/input-irq-evidence' / str(time.time_ns())
    evidence.mkdir(parents=True, exist_ok=False)
    manifest = {'status': 'incomplete', 'commit': commit, 'source_clean': True,
                'archive_sha256': hashlib.sha256(archive).hexdigest(),
                'scope': 'real PS/2 IRQ deferred across production scoped input borrow and delivered after IF restoration'}
    try:
        manifest.update(environment(Path(firmware_path()), source))
        with tempfile.TemporaryDirectory(prefix='genos-input-source-') as temporary:
            root = Path(temporary) / 'source'; root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as tree:
                tree.extractall(root, filter='data')
            patch = patch_fixture(root)
            (evidence / 'fixture.patch').write_text(patch)
            manifest['fixture_sha256'] = hashlib.sha256(patch.encode()).hexdigest()
            with (evidence / 'build.log').open('w') as output:
                subprocess.run(['cargo', 'xtask', 'build-test'], cwd=root, stdout=output,
                               stderr=subprocess.STDOUT, check=True, timeout=600)
            manifest['image_sha256'] = hashlib.sha256((root / 'build/genos.img').read_bytes()).hexdigest()
            boot(root, evidence, options.timeout)
            manifest['status'] = 'passed'
    except BaseException as error:
        manifest.update(status='failed', failure=f'{type(error).__name__}: {error}')
        raise
    finally:
        manifest['files_sha256'] = {str(path.relative_to(evidence)): hashlib.sha256(path.read_bytes()).hexdigest()
                                    for path in sorted(evidence.rglob('*')) if path.is_file()}
        (evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'INPUT_IRQ_PROBE_OK evidence={evidence}', flush=True)


if __name__ == '__main__':
    main()
