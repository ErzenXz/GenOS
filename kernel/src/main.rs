#![no_std]
#![no_main]

mod arch;
mod input_hw;
mod interrupts;
mod memory;
mod network;
mod paging;
mod ramfs;
mod rtc;
mod runtime;
mod serial;
mod shell;
mod storage;
mod userspace;

use core::panic::PanicInfo;
use genos_abi::{BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION};
use kernel::tasks::{TaskRegistry, TaskState};
use kernel::vfs::RamVfs;

#[no_mangle]
pub extern "sysv64" fn _start(boot_info: &'static BootInfo) -> ! {
    serial::init();
    serial::println("GenOS kernel entered");

    if boot_info.magic != BOOT_INFO_MAGIC || boot_info.version != BOOT_INFO_VERSION {
        serial::println("Invalid BootInfo; halting");
        arch::halt_loop();
    }

    arch::init();
    if !kernel::recovery::boundary_is_minimal() {
        serial::println("RECOVERY_BOUNDARY_FAILED");
        arch::halt_loop();
    }
    serial::println("RECOVERY_BOUNDARY_OK");
    memory::init(boot_info);
    if paging::init_protected_address_space().is_err() {
        serial::println("PAGING_FAILED");
        arch::halt_loop();
    }

    interrupts::init();
    network::init();
    let initrd = ramfs::RamFs::from_initrd(boot_info.initrd.base, boot_info.initrd.size);
    let Some(init_program) = initrd.find("INIT.ELF") else {
        serial::println("USER_ELF_MISSING");
        arch::halt_loop();
    };
    let Some(shell_program) = initrd.find("SHELL.ELF") else {
        serial::println("USER_SHELL_MISSING");
        arch::halt_loop();
    };
    userspace::register_shell_elf(shell_program.data);
    userspace::run_probe(init_program.data);
    let dynamic_probe = match userspace::launch_init() {
        Ok(result) => result,
        Err(_) => {
            serial::println("USER_ELF_BOOT_LAUNCH_FAILED");
            arch::halt_loop();
        }
    };
    serial::print("USER_BOOT_INIT pid=");
    serial::print_u64(dynamic_probe.pid as u64);
    serial::println("");
    let mut vfs = RamVfs::new();
    vfs.init_root();
    let _ = vfs.mkdir("/USER");
    for file in initrd.iter() {
        if file.name != "INIT.ELF" && file.name != "SHELL.ELF" {
            vfs.seed_file(file.name, file.data);
        }
    }
    if !storage::init_session_ramfs(&mut vfs) {
        serial::println("RAMFS_TEMP_FAILED");
        arch::halt_loop();
    }
    serial::println("RAMFS_TEMPORARY_READY");
    let (_, persistent_fs) = storage::mount_or_create(&mut vfs);
    serial::println("VFS_READY");
    userspace::run_lifecycle_probe(&mut vfs);
    // The lifecycle probe deliberately exercises a write through a temporary
    // manager. Do not let its fixture become part of the mounted user volume.
    let _ = vfs.remove("/USER/APP.TXT");
    userspace::run_supervisor_cleanup_probe();
    serial::println("SUPERVISOR_CLEANUP_READY");
    userspace::run_transactional_rollback_probe();
    serial::println("RUNTIME_ROLLBACK_READY");
    userspace::run_process_generation_stress_probe();
    serial::println("PROCESS_GENERATION_STRESS_READY");

    let scheduler_benchmark = kernel::tasks::benchmark_scheduler_policy();
    if scheduler_benchmark.dispatches == 0 || scheduler_benchmark.max_dispatch_latency_ticks == 0 {
        serial::println("SCHED_DISPATCH_BENCH_FAILED");
        arch::halt_loop();
    }
    serial::print("SCHED_DISPATCH_BENCH dispatches=");
    serial::print_u64(scheduler_benchmark.dispatches);
    serial::print(" max_latency_ticks=");
    serial::print_u64(scheduler_benchmark.max_dispatch_latency_ticks);
    serial::print(" avg_latency_milliticks=");
    serial::print_u64(scheduler_benchmark.average_latency_milliticks());
    serial::println("");
    serial::println("SCHED_DISPATCH_BENCH_OK");

    let mut tasks = TaskRegistry::new();
    let task_ids = runtime::TaskIds {
        desktop: tasks.register("desktop", TaskState::Running, 96),
        shell: tasks.register("shell", TaskState::Ready, 48),
        input: tasks.register("input", TaskState::Waiting, 24),
        vfs: tasks.register("vfs", TaskState::Ready, 40),
        idle: tasks.register("idle", TaskState::Sleeping, 8),
    };
    let _ = tasks.register("taskmgr", TaskState::Ready, 32);
    let mut processes = userspace::ProcessManager::new();
    let shell_task = runtime::SHELL_TASK_ID;
    processes.spawn_shell(shell_task).unwrap_or_else(|_| {
        serial::println("USER_SHELL_LAUNCH_FAILED");
        arch::halt_loop();
    });
    let mut runtime =
        runtime::RuntimeCoordinator::new(tasks, task_ids, processes, vfs, persistent_fs);
    if !runtime.run_headless_boot_probe(512) {
        serial::println("HEADLESS_RUNTIME_FAILED");
        arch::halt_loop();
    }
    if !runtime.run_console_transcript_probe() {
        serial::println("CONSOLE_TRANSCRIPT_FAILED");
        arch::halt_loop();
    }
    if !runtime.process_snapshot_is_authoritative() {
        serial::println("PROCESS_SNAPSHOT_FAILED");
        arch::halt_loop();
    }
    if !runtime.unified_handle_table_is_authoritative() {
        serial::println("UNIFIED_HANDLE_TABLE_FAILED");
        arch::halt_loop();
    }
    if !runtime.async_request_identity_is_authoritative() {
        serial::println("ASYNC_REQUEST_IDENTITY_FAILED");
        arch::halt_loop();
    }
    serial::println("TASKS_READY");
    serial::println("SCHED_READY");
    serial::println("RUNTIME_COORDINATOR_READY");
    serial::println("HEADLESS_RUNTIME_READY");
    serial::println("PROCESS_SNAPSHOT_READY");
    serial::println("UNIFIED_HANDLE_TABLE_READY");
    serial::println("ASYNC_REQUEST_IDENTITY_READY");
    serial::println("CONSOLE_TRANSCRIPT_READY");

    serial::println("SERVER_TERMINAL_READY mode=serial ui=off");
    serial::println("SERIAL_TERMINAL_READY port=com1");
    serial::println("GENOS_READY");

    interrupts::enable();
    shell::run_terminal(boot_info, runtime);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial::println("KERNEL PANIC");
    if let Some(location) = info.location() {
        serial::print("at ");
        serial::print(location.file());
        serial::print(":");
        serial::print_u64(location.line() as u64);
        serial::println("");
    }
    arch::halt_loop();
}
