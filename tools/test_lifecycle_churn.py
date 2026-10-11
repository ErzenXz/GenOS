#!/usr/bin/env python3
"""Qualify real process execution, exit/kill and exact reclamation in one boot.

A disposable source fixture only adds a call and probe. Production construction,
context switching, execution, handle cleanup, allocation/scrubbing and reaping
remain unchanged. Short runs are exploratory, never the 10,000-cycle gate.
"""
from __future__ import annotations
import argparse
import difflib
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import time

from reference_vm import environment
from test_cpu_paging import boot
from test_exception_entry import firmware_path, replace_once, require_clean_source

QUALIFICATION_CYCLES = 10_000
TASK = 0x1600
FIXTURE = r'''
pub fn run_lifecycle_churn_probe() {
    const CYCLES: u64 = __CYCLES__;
    const TASK: u32 = 0x1600;
    const POLLS: usize = 8;
    assert!(crate::arch::interrupts_enabled());
    assert_eq!(active_process_count(), 0);
    let baseline = memory::allocated_frames();
    let reclaimed_before = reclaimed_frame_count();
    let ticks_before = crate::interrupts::ticks();
    let mut manager = ProcessManager::new();
    let mut previous_incarnation = 0;
    let mut total_frames = 0;
    let mut seen = [false; 256];
    let mut pid_reused = false;
    crate::serial::print("LIFECYCLE_CHURN_WINDOW cycles=");
    crate::serial::print_u64(CYCLES);
    crate::serial::print(" baseline=");
    crate::serial::print_u64(baseline);
    crate::serial::println("");
    for cycle in 1..=CYCLES {
        let hold = cycle % 2 == 0;
        crate::serial::print("LIFECYCLE_CHURN_BEGIN cycle=");
        crate::serial::print_u64(cycle);
        crate::serial::print(" mode=");
        crate::serial::println(if hold { "kill" } else { "exit" });
        let pid = manager.spawn_init(TASK, hold).expect("churn spawn");
        pid_reused |= seen[pid as usize];
        seen[pid as usize] = true;
        let managed = manager.slots[0].as_ref().expect("churn slot");
        let incarnation = managed.key.incarnation;
        assert_eq!(managed.key.slot, 0);
        assert!(incarnation > previous_incarnation);
        assert!(managed.handle_table_is_consistent());
        previous_incarnation = incarnation;
        let allocated = memory::allocated_frames().checked_sub(baseline).expect("churn allocation");
        assert!(allocated > 0);
        assert_eq!(manager.live_count(), 1);
        assert_eq!(active_process_count(), 1);
        let mut output_seen = false;
        let mut exited = false;
        for _ in 0..POLLS {
            let update = manager.poll(crate::interrupts::ticks()).expect("churn dispatch");
            assert_eq!(update.pid, pid);
            assert!(update.vfs_request.is_none() && update.lifecycle_request.is_none()
                && update.socket_request.is_none());
            output_seen |= !update.output.is_empty();
            if hold {
                assert_eq!(update.state, ManagedState::Ready);
                if output_seen { break; }
            } else if update.state == ManagedState::Exited {
                assert_eq!(update.exit_code, 0);
                exited = true;
                break;
            } else {
                assert_eq!(update.state, ManagedState::Ready);
            }
        }
        assert!(output_seen, "churn must execute Ring 3 output");
        if hold {
            let killed = manager.kill(TASK).expect("churn kill");
            assert_eq!(killed.pid, pid);
            assert_eq!(killed.state, ManagedState::Killed);
            assert_eq!(killed.exit_code, 137);
        } else { assert!(exited, "churn exit deadline"); }
        let result = manager.wait(TASK).expect("churn reap");
        assert_eq!(result.pid, pid);
        assert_eq!(result.state, if hold { ManagedState::Killed } else { ManagedState::Exited });
        assert_eq!(result.exit_code, if hold { 137 } else { 0 });
        assert!(manager.wait(TASK).is_err());
        assert!(manager.kill(TASK).is_err());
        assert_eq!(manager.live_count(), 0);
        assert!(manager.slots.iter().all(Option::is_none));
        assert_eq!(active_process_count(), 0);
        assert_eq!(memory::allocated_frames(), baseline);
        total_frames += allocated;
        assert_eq!(reclaimed_frame_count() - reclaimed_before, total_frames);
        let report = memory::report().expect("churn memory report");
        assert!(report.as_bytes().windows(b"consistent=yes\n".len())
            .any(|window| window == b"consistent=yes\n"));
        assert!(crate::arch::interrupts_enabled());
        crate::serial::print("LIFECYCLE_CHURN_CYCLE cycle=");
        crate::serial::print_u64(cycle);
        crate::serial::print(" pid=");
        crate::serial::print_u64(pid as u64);
        crate::serial::print(" incarnation=");
        crate::serial::print_u64(incarnation);
        crate::serial::print(" live=");
        crate::serial::print_u64(memory::allocated_frames());
        crate::serial::print(" reclaimed=");
        crate::serial::print_u64(total_frames);
        crate::serial::println(" active=0 slots=0 output=1 consistent=yes if=restored");
    }
    assert!(CYCLES <= 255 || pid_reused);
    assert!(crate::interrupts::ticks() > ticks_before);
    crate::serial::print("LIFECYCLE_CHURN_COMPLETE cycles=");
    crate::serial::print_u64(CYCLES);
    crate::serial::print(" exits=");
    crate::serial::print_u64(CYCLES.div_ceil(2));
    crate::serial::print(" kills=");
    crate::serial::print_u64(CYCLES / 2);
    crate::serial::print(" reclaimed=");
    crate::serial::print_u64(total_frames);
    crate::serial::print(" baseline=");
    crate::serial::print_u64(baseline);
    crate::serial::print(" ticks=");
    crate::serial::print_u64(crate::interrupts::ticks() - ticks_before);
    crate::serial::println(" active=0 slots=0");
    crate::arch::disable_interrupts();
    crate::arch::halt_loop();
}
'''


def check_cycles(cycles: int) -> None:
    if type(cycles) is not int or not 2 <= cycles <= QUALIFICATION_CYCLES:
        raise ValueError('cycles must be between 2 and 10000')


def patch_fixture(root: Path, cycles: int) -> str:
    check_cycles(cycles)
    names = ['kernel/src/main.rs', 'kernel/src/userspace.rs']
    before = {name: (root / name).read_text() for name in names}
    if any('run_lifecycle_churn_probe' in value for value in before.values()):
        raise ValueError('churn fixture already installed')
    after = before.copy()
    anchor = '        serial::println("PROCESS_GENERATION_STRESS_READY");'
    after[names[0]] = replace_once(after[names[0]], anchor,
        anchor + '\n        userspace::run_lifecycle_churn_probe();')
    after[names[1]] += FIXTURE.replace('__CYCLES__', str(cycles))
    for name in names:
        (root / name).write_text(after[name])
    return ''.join(''.join(difflib.unified_diff(before[name].splitlines(True), after[name].splitlines(True),
                   fromfile='a/' + name, tofile='b/' + name)) for name in names)


def validate_log(log: str, cycles: int) -> dict:
    check_cycles(cycles)
    lines = log.replace('\r', '').splitlines()
    if any(token in log for token in ['KERNEL PANIC', 'BOOTLOADER_PANIC', '_FAILED']):
        raise ValueError('boot or churn failed')
    windows = [(i, re.fullmatch(r'LIFECYCLE_CHURN_WINDOW cycles=(\d+) baseline=(\d+)', line))
               for i, line in enumerate(lines) if 'LIFECYCLE_CHURN_WINDOW' in line]
    if len(windows) != 1 or windows[0][1] is None:
        raise ValueError('missing, duplicate or embedded churn window')
    start, window = windows[0]
    expected, baseline = map(int, window.groups())
    if expected != cycles:
        raise ValueError('wrong cycle scope')
    proofs = ['BOOT_MODE validation', 'BOOT_MEMORY_MAP_VALIDATED', 'IRQ_HARDWARE_ON',
              'USER_PROCESS_GENERATION_STRESS_OK launches=257', 'USER_PID_REUSE_SAFE_OK',
              'USER_STALE_PROCESS_HANDLE_REJECTED_OK', 'PROCESS_GENERATION_STRESS_READY']
    if any(lines.count(proof) != 1 for proof in proofs):
        raise ValueError('missing or duplicate production setup proof')
    indexes = [lines.index(proof) for proof in proofs] + [start]
    if indexes != sorted(indexes):
        raise ValueError('unordered setup')
    tail = lines[start:]
    if any(token in '\n'.join(tail) for token in ['_FAILED', '_REJECTED', 'EXCEPTION_', 'USER_FAULT',
        'KERNEL PANIC', 'BOOTLOADER_PANIC', 'GENOS_READY', 'USERMODE_READY']):
        raise ValueError('churn faulted or escaped')
    cursor = 1
    reclaimed = 0
    pids = set()
    previous_pid = None
    for cycle in range(1, cycles + 1):
        mode = 'kill' if cycle % 2 == 0 else 'exit'
        if cursor >= len(tail) or tail[cursor] != f'LIFECYCLE_CHURN_BEGIN cycle={cycle} mode={mode}':
            raise ValueError('missing, reordered or wrong-mode cycle begin')
        begin = cursor
        cursor += 1
        while cursor < len(tail) and not tail[cursor].startswith('LIFECYCLE_CHURN_'):
            cursor += 1
        if cursor == len(tail):
            raise ValueError('incomplete cycle')
        match = re.fullmatch(r'LIFECYCLE_CHURN_CYCLE cycle=(\d+) pid=(\d+) incarnation=(\d+) live=(\d+) reclaimed=(\d+) active=0 slots=0 output=1 consistent=yes if=restored', tail[cursor])
        if match is None:
            raise ValueError('malformed cycle result')
        ordinal, pid, incarnation, live, total = map(int, match.groups())
        if ordinal != cycle or incarnation != cycle or live != baseline or not 1 <= pid <= 255 or total <= reclaimed:
            raise ValueError('stale identity, leaked frames or wrong cycle')
        if previous_pid is not None and pid != previous_pid % 255 + 1:
            raise ValueError('unexpected nonzero PID succession')
        previous_pid = pid
        records = tail[begin + 1:cursor]
        required = [f'USER_ASYNC_SPAWN pid={pid} task={TASK} mode={"hold" if mode == "kill" else "normal"}',
                    f'USER_OUTPUT pid={pid} text=hello from INIT.ELF in ring 3',
                    f'USER_FRAMES_RECLAIMED pid={pid} frames={total - reclaimed}',
                    f'USER_WAIT_REAPED pid={pid}']
        required.insert(2 if mode == 'exit' else 3,
                        f'USER_EXIT pid={pid} code=0' if mode == 'exit' else f'USER_KILLED pid={pid} task={TASK}')
        if any(records.count(proof) != 1 for proof in required):
            raise ValueError('missing, duplicate or wrong real production lifecycle proof')
        positions = [records.index(proof) for proof in required]
        if positions != sorted(positions):
            raise ValueError('production lifecycle out of order')
        allowed = required + [f'USER_PREEMPT pid={pid}']
        extras = [line for line in records if line not in allowed]
        if (len(extras) != 1 or re.fullmatch(rf'USER_ELF_LOADED pid={pid} root=0x[0-9a-f]+', extras[0]) is None
                or records[0] != extras[0]):
            raise ValueError('unexpected production trace inside cycle')
        pids.add(pid)
        reclaimed = total
        cursor += 1
    if cursor != len(tail) - 1:
        raise ValueError('missing final proof or unexpected continuation')
    final = re.fullmatch(r'LIFECYCLE_CHURN_COMPLETE cycles=(\d+) exits=(\d+) kills=(\d+) reclaimed=(\d+) baseline=(\d+) ticks=(\d+) active=0 slots=0', tail[cursor])
    if final is None:
        raise ValueError('malformed completion')
    count, exits, kills, frames, base, ticks = map(int, final.groups())
    if (count, exits, kills, frames, base) != (cycles, (cycles + 1) // 2, cycles // 2, reclaimed, baseline) or ticks == 0:
        raise ValueError('wrong completion totals or missing hardware ticks')
    if cycles >= 256 and len(pids) >= cycles:
        raise ValueError('missing PID reuse')
    if sum('LIFECYCLE_CHURN_' in line for line in lines) != 2 * cycles + 2:
        raise ValueError('extra, embedded or stale churn record')
    return {'cycles': cycles, 'normal_exits': exits, 'kills': kills, 'reclaimed_frames': reclaimed,
            'live_frames_baseline': baseline, 'unique_pids': len(pids), 'hardware_ticks': ticks}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cycles', type=int, default=QUALIFICATION_CYCLES)
    parser.add_argument('--timeout', type=int, default=1800)
    options = parser.parse_args()
    try: check_cycles(options.cycles)
    except ValueError as error: parser.error(str(error))
    if sys.version_info < (3, 12) or not 1 <= options.timeout <= 1800:
        parser.error('Python >=3.12 and timeout from 1 through 1800 seconds required')
    source = Path(__file__).resolve().parents[1]
    commit = require_clean_source(source)
    archive = subprocess.check_output(['git', 'archive', '--format=tar', commit], cwd=source, timeout=30)
    evidence = source / 'build/lifecycle-churn-evidence' / str(time.time_ns())
    evidence.mkdir(parents=True, exist_ok=False)
    manifest = {'status': 'incomplete', 'commit': commit, 'source_clean': True,
                'archive_sha256': hashlib.sha256(archive).hexdigest(), 'cycles': options.cycles,
                'timeout_seconds': options.timeout, 'qualifies_10000_cycles': False,
                'scope': 'single-boot real Ring 3 process execution, exit/kill, slot reuse and exact reclamation; excludes 24-hour mixed pressure, sustained IPC/I/O and C5'}
    started = time.monotonic()
    try:
        manifest.update(environment(Path(firmware_path()), source))
        with tempfile.TemporaryDirectory(prefix='genos-churn-') as temporary:
            root = Path(temporary) / 'source'; root.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as tree:
                tree.extractall(root, filter='data')
            patch = patch_fixture(root, options.cycles)
            (evidence / 'fixture.patch').write_text(patch)
            manifest['fixture_sha256'] = hashlib.sha256(patch.encode()).hexdigest()
            with (evidence / 'build.log').open('w') as output:
                subprocess.run(['cargo', 'xtask', 'build-test'], cwd=root, stdout=output,
                               stderr=subprocess.STDOUT, check=True, timeout=600)
            manifest['image_sha256'] = hashlib.sha256((root / 'build/genos.img').read_bytes()).hexdigest()
            boot(root, evidence, options.timeout, halt_marker=f'LIFECYCLE_CHURN_COMPLETE cycles={options.cycles} ',
                 validate_serial=lambda log: validate_log(log, options.cycles))
            manifest['result'] = validate_log((evidence / 'serial.log').read_text(), options.cycles)
            manifest.update(status='passed', qualifies_10000_cycles=options.cycles == QUALIFICATION_CYCLES)
    except BaseException as error:
        manifest.update(status='failed', failure=f'{type(error).__name__}: {error}')
        raise
    finally:
        manifest['elapsed_seconds'] = time.monotonic() - started
        manifest['files_sha256'] = {str(p.relative_to(evidence)): hashlib.sha256(p.read_bytes()).hexdigest()
                                  for p in sorted(evidence.rglob('*')) if p.is_file()}
        (evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'LIFECYCLE_CHURN_OK cycles={options.cycles} qualifies={manifest["qualifies_10000_cycles"]} evidence={evidence}', flush=True)


if __name__ == '__main__':
    main()
