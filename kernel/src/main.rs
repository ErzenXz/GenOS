#![no_std]
#![no_main]

mod arch;
mod input_hw;
mod interrupts;
mod memory;
mod ramfs;
mod rtc;
mod serial;
mod shell;

use core::panic::PanicInfo;
use genos_abi::{BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION};
use kernel::display::{DisplayManager, FramebufferDevice};
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
    memory::init(boot_info);

    interrupts::init();

    let initrd = ramfs::RamFs::from_initrd(boot_info.initrd.base, boot_info.initrd.size);
    let mut vfs = RamVfs::new();
    vfs.init_root();
    for file in initrd.iter() {
        vfs.seed_file(file.name, file.data);
    }
    serial::println("VFS_READY");

    let mut tasks = TaskRegistry::new();
    let task_ids = shell::TaskIds {
        desktop: tasks.register("desktop", TaskState::Running, 96),
        shell: tasks.register("shell", TaskState::Ready, 48),
        input: tasks.register("input", TaskState::Waiting, 24),
        vfs: tasks.register("vfs", TaskState::Ready, 40),
        taskmgr: tasks.register("taskmgr", TaskState::Ready, 32),
        idle: tasks.register("idle", TaskState::Sleeping, 8),
    };
    serial::println("TASKS_READY");
    serial::println("SCHED_READY");

    let display = FramebufferDevice::new(&boot_info.framebuffer);
    if display.is_backbuffered() {
        serial::print("BACKBUFFER_READY bytes=");
        serial::print_u64(display.draw_bytes_len() as u64);
        serial::println("");
    } else {
        serial::println("DIRECT_FRAMEBUFFER");
    }
    input_hw::init(display.width(), display.height());

    let mut manager =
        DisplayManager::new(display, boot_info, memory::usable_bytes(), initrd.count());
    manager.sync_stats(
        input_hw::mouse_state(),
        input_hw::event_depth(),
        vfs.count(),
        interrupts::ticks(),
    );
    manager.sync_vfs(&vfs);
    manager.redraw_with_tasks(&tasks);
    serial::println("GENOS_READY");

    interrupts::enable();
    shell::run(manager, vfs, boot_info, tasks, task_ids);
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
