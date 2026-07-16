use core::str;

use genos_abi::BootInfo;
use kernel::{
    display::{DisplayManager, FixedText, LineKind},
    input::{InputEvent, KeyEvent},
    tasks::{TaskError, TaskRegistry, TaskState, MAX_TASKS},
    vfs::{NodeKind, RamVfs, VfsError},
};

use crate::{arch, input_hw, interrupts, memory, rtc, serial};

#[derive(Clone, Copy)]
pub struct TaskIds {
    pub desktop: u32,
    pub shell: u32,
    pub input: u32,
    pub vfs: u32,
    pub taskmgr: u32,
    pub idle: u32,
}

pub fn run(
    mut display: DisplayManager,
    mut vfs: RamVfs,
    boot_info: &'static BootInfo,
    mut tasks: TaskRegistry,
    ids: TaskIds,
) -> ! {
    let mut cwd = FixedText::from_str("/");
    let mut history = [FixedText::empty(); 8];
    let mut history_len = 0usize;
    let mut history_cursor = 0usize;
    let mut last_tick = interrupts::ticks();
    let mut last_clock_second = 255u8;
    let mut irq_tick_marker_sent = false;
    let mut display_idle_marker_sent = false;

    loop {
        input_hw::poll();
        let tick = interrupts::poll_fallback_tick();
        let mut handled_event = false;
        let irq_stats = interrupts::stats();

        display.sync_stats(
            input_hw::mouse_state(),
            input_hw::event_depth(),
            vfs.count(),
            irq_stats.ticks,
        );
        display.refresh_stats_if_due(tick);
        display.animate_if_due(tick);

        if tick.is_multiple_of(25) {
            let now = rtc::read();
            if now.second != last_clock_second {
                last_clock_second = now.second;
                display.set_clock(now.format_clock());
            }
        }

        if !irq_tick_marker_sent && tick >= 100 {
            serial::println("IRQ_TICK_OK");
            irq_tick_marker_sent = true;
        }
        if !display_idle_marker_sent && tick >= 140 {
            serial::println("DISPLAY_IDLE_OK");
            display_idle_marker_sent = true;
        }

        while let Some(event) = input_hw::pop_event() {
            handled_event = true;
            tasks.mark_running(ids.input, tick);
            match event {
                InputEvent::Key(KeyEvent::Enter) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    tasks.mark_running(ids.shell, tick);
                    let input = display.take_input();
                    let command = input.as_str();
                    if !command.is_empty() {
                        if history_len < history.len() {
                            history[history_len] = input;
                            history_len += 1;
                        } else {
                            let mut index = 1;
                            while index < history.len() {
                                history[index - 1] = history[index];
                                index += 1;
                            }
                            history[history.len() - 1] = input;
                        }
                        history_cursor = history_len;
                        let mut prompt = FixedText::empty();
                        prompt.push_str(cwd.as_str());
                        prompt.push_str("> ");
                        prompt.push_str(command);
                        display.push_fixed(LineKind::Prompt, prompt);
                    }
                    execute(
                        command,
                        &mut display,
                        &mut vfs,
                        &mut cwd,
                        boot_info,
                        &mut tasks,
                        ids,
                    );
                    display.sync_vfs(&vfs);
                }
                InputEvent::Key(KeyEvent::Backspace) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    tasks.mark_running(ids.shell, tick);
                    let _ = display.input_backspace();
                }
                InputEvent::Key(KeyEvent::Char(byte)) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    tasks.mark_running(ids.shell, tick);
                    let _ = display.input_push(byte);
                }
                InputEvent::Key(KeyEvent::ArrowUp) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    tasks.mark_running(ids.shell, tick);
                    if history_len > 0 {
                        history_cursor = history_cursor.saturating_sub(1);
                        display.set_input(history[history_cursor]);
                    }
                }
                InputEvent::Key(KeyEvent::ArrowDown) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    tasks.mark_running(ids.shell, tick);
                    if history_cursor + 1 < history_len {
                        history_cursor += 1;
                        display.set_input(history[history_cursor]);
                    } else {
                        history_cursor = history_len;
                        display.set_input(FixedText::empty());
                    }
                }
                InputEvent::Key(KeyEvent::Escape) => display.dismiss_focused(),
                InputEvent::Key(KeyEvent::Tab) => display.cycle_focus(),
                InputEvent::MouseMove { buttons, .. } => {
                    display.handle_mouse_move(input_hw::mouse_state().position, buttons.left);
                }
                InputEvent::MouseButton { buttons, .. } => {
                    if buttons.left {
                        tasks.mark_running(ids.desktop, tick);
                        display.handle_mouse_down(input_hw::mouse_state().position);
                    } else {
                        display.end_drag();
                    }
                }
            }
        }

        if tick != last_tick {
            tasks.scheduler_tick(tick);
            tasks.mark_running(ids.desktop, tick);
            tasks.set_state(ids.input, TaskState::Waiting, tick);
            last_tick = tick;
        }

        if handled_event {
            tasks.set_state(ids.shell, TaskState::Ready, tick);
            tasks.set_state(ids.input, TaskState::Waiting, tick);
            tasks.set_state(ids.idle, TaskState::Sleeping, tick);
        } else {
            tasks.set_state(ids.idle, TaskState::Running, tick);
            tasks.tick_idle(tick);
        }

        display.flush(&tasks);
        core::hint::spin_loop();
    }
}

fn execute(
    command: &str,
    display: &mut DisplayManager,
    vfs: &mut RamVfs,
    cwd: &mut FixedText,
    boot_info: &BootInfo,
    tasks: &mut TaskRegistry,
    ids: TaskIds,
) {
    serial::print("shell: ");
    serial::println(command);

    let trimmed = trim(command);
    if trimmed.is_empty() {
        display.set_status("idle");
        return;
    }

    let tick = interrupts::ticks();
    let (cmd, args) = split_once_space(trimmed);
    match cmd {
        "help" => {
            display.push_line(
                LineKind::Output,
                "help clear mem pwd cd ls cat touch write append rm mkdir stat ps run spawn kill sleep wake sched userabi taskmgr files game time apps echo uname about ui reboot shutdown",
            );
            display.set_status("help printed");
        }
        "clear" => {
            display.clear_shell();
            display.push_line(LineKind::Status, "Shell cleared");
            display.set_status("clear");
        }
        "mem" => {
            let mut line = FixedText::from_str("usable memory: ");
            line.push_u64(memory::usable_bytes());
            line.push_str(" bytes");
            display.push_fixed(LineKind::Output, line);
            if let Some(frame) = memory::alloc_frame() {
                let mut frame_line = FixedText::from_str("allocated frame: 0x");
                frame_line.push_hex(frame);
                display.push_fixed(LineKind::Status, frame_line);
            }
            display.set_status("memory sampled");
        }
        "pwd" => {
            display.push_fixed(LineKind::Output, *cwd);
            display.set_status("cwd");
        }
        "cd" => {
            let target = resolve_path(cwd.as_str(), trim(args));
            match vfs.find(target.as_str()) {
                Some(node) if node.kind() == NodeKind::Directory => {
                    *cwd = target;
                    display.set_status("directory changed");
                }
                Some(_) => {
                    display.push_line(LineKind::Error, "not a directory");
                    display.set_status("cd failed");
                }
                None => {
                    display.push_line(LineKind::Error, "directory not found");
                    display.set_status("cd failed");
                }
            }
        }
        "ls" => {
            let mut found = false;
            for node in vfs.list_root() {
                found = true;
                let mut line = FixedText::empty();
                line.push_str(match node.kind() {
                    NodeKind::File => "file ",
                    NodeKind::Directory => "dir  ",
                });
                line.push_str(node.path());
                line.push_str(" ");
                line.push_u64(node.len() as u64);
                line.push_str("b");
                display.push_fixed(LineKind::Output, line);
            }
            if !found {
                display.push_line(LineKind::Output, "(empty)");
            }
            display.set_status("vfs listed");
        }
        "cat" => {
            let path = resolve_path(cwd.as_str(), trim(args));
            if trim(args).is_empty() {
                display.push_line(LineKind::Error, "usage: cat FILE");
            } else {
                match vfs.read(path.as_str()) {
                    Ok(bytes) => match str::from_utf8(bytes) {
                        Ok(text) => push_multiline(display, LineKind::Output, text),
                        Err(_) => display.push_line(LineKind::Error, "binary file"),
                    },
                    Err(error) => push_vfs_error(display, error),
                }
                display.set_status("file opened");
            }
        }
        "touch" => {
            let path = resolve_path(cwd.as_str(), trim(args));
            if trim(args).is_empty() {
                display.push_line(LineKind::Error, "usage: touch FILE");
            } else {
                tasks.mark_running(ids.vfs, tick);
                report_vfs_result(display, vfs.touch(path.as_str()), "file touched");
            }
        }
        "write" => {
            let (path_arg, text) = split_once_space(args);
            let path = resolve_path(cwd.as_str(), path_arg);
            if path_arg.is_empty() {
                display.push_line(LineKind::Error, "usage: write FILE TEXT");
            } else {
                tasks.mark_running(ids.vfs, tick);
                report_vfs_result(
                    display,
                    vfs.write(path.as_str(), text.as_bytes()),
                    "file written",
                );
            }
        }
        "append" => {
            let (path_arg, text) = split_once_space(args);
            let path = resolve_path(cwd.as_str(), path_arg);
            if path_arg.is_empty() {
                display.push_line(LineKind::Error, "usage: append FILE TEXT");
            } else {
                tasks.mark_running(ids.vfs, tick);
                report_vfs_result(
                    display,
                    vfs.append(path.as_str(), text.as_bytes()),
                    "file appended",
                );
            }
        }
        "rm" => {
            let path = resolve_path(cwd.as_str(), trim(args));
            if trim(args).is_empty() {
                display.push_line(LineKind::Error, "usage: rm PATH");
            } else {
                tasks.mark_running(ids.vfs, tick);
                report_vfs_result(display, vfs.remove(path.as_str()), "path removed");
            }
        }
        "mkdir" => {
            let path = resolve_path(cwd.as_str(), trim(args));
            if trim(args).is_empty() {
                display.push_line(LineKind::Error, "usage: mkdir DIR");
            } else {
                tasks.mark_running(ids.vfs, tick);
                report_vfs_result(display, vfs.mkdir(path.as_str()), "directory created");
            }
        }
        "stat" => {
            let path = resolve_path(cwd.as_str(), trim(args));
            if trim(args).is_empty() {
                display.push_line(LineKind::Error, "usage: stat PATH");
            } else {
                match vfs.stat(path.as_str()) {
                    Ok(line) => display.push_fixed(LineKind::Output, line),
                    Err(error) => push_vfs_error(display, error),
                }
                display.set_status("stat");
            }
        }
        "tasks" | "ps" => {
            let mut index = 0;
            while index < tasks.len() {
                if let Some(row) = tasks.format_row(index) {
                    display.push_fixed(LineKind::Output, row);
                }
                index += 1;
            }
            display.set_status("processes listed");
        }
        "run" => {
            let program = trim(args);
            if program != "init" && program != "INIT.ELF" {
                display.push_line(LineKind::Error, "usage: run init");
                display.set_status("ELF launch failed");
            } else if tasks.len() >= MAX_TASKS {
                push_task_error(display, TaskError::TableFull);
            } else {
                match crate::userspace::launch_init() {
                    Ok(result) => {
                        match tasks.record_user_exit(
                            "init-elf",
                            result.exit_code,
                            interrupts::ticks(),
                        ) {
                            Ok(task_pid) => {
                                let mut line = FixedText::from_str("ELF launched ring-pid=");
                                line.push_u64(result.pid as u64);
                                line.push_str(" task-pid=");
                                line.push_u64(task_pid as u64);
                                line.push_str(" exit=");
                                line.push_u64(result.exit_code as u64);
                                line.push_str(" preempt=");
                                line.push_u64(result.preemptions as u64);
                                display.push_fixed(LineKind::Status, line);
                                display.set_status("INIT.ELF completed");
                            }
                            Err(error) => push_task_error(display, error),
                        }
                    }
                    Err(error) => {
                        let text = match error {
                            crate::userspace::LaunchError::ImageUnavailable => {
                                "INIT.ELF is unavailable"
                            }
                            crate::userspace::LaunchError::ProcessBuildFailed => {
                                "INIT.ELF failed validation or mapping"
                            }
                            crate::userspace::LaunchError::ProcessFaulted => {
                                "INIT.ELF terminated with a CPU fault"
                            }
                            crate::userspace::LaunchError::InvalidResult => {
                                "INIT.ELF returned an invalid result"
                            }
                        };
                        display.push_line(LineKind::Error, text);
                        display.set_status("ELF launch failed");
                    }
                }
            }
            display.refresh_task_manager();
        }
        "spawn" => {
            let name = trim(args);
            if name.is_empty() {
                display.push_line(LineKind::Error, "usage: spawn NAME");
            } else {
                match tasks.spawn_worker(name, 24, tick) {
                    Ok(pid) => {
                        let mut line = FixedText::from_str("worker started pid=");
                        line.push_u64(pid as u64);
                        line.push_str(" name=");
                        line.push_str(name);
                        display.push_fixed(LineKind::Status, line);
                        display.set_status("worker started");
                    }
                    Err(error) => push_task_error(display, error),
                }
            }
            display.refresh_task_manager();
        }
        "kill" => {
            match parse_u32(trim(args)) {
                Some(pid) => match tasks.terminate(pid, 0, tick) {
                    Ok(()) => {
                        display.push_line(LineKind::Status, "worker terminated");
                        display.set_status("worker terminated");
                    }
                    Err(error) => push_task_error(display, error),
                },
                None => display.push_line(LineKind::Error, "usage: kill PID"),
            }
            display.refresh_task_manager();
        }
        "sleep" => {
            let (pid_text, ticks_text) = split_once_space(args);
            match (parse_u32(pid_text), parse_u64(ticks_text)) {
                (Some(pid), Some(duration)) => match tasks.sleep(pid, duration, tick) {
                    Ok(()) => {
                        let mut line = FixedText::from_str("worker sleeping for ");
                        line.push_u64(duration);
                        line.push_str(" ticks");
                        display.push_fixed(LineKind::Status, line);
                        display.set_status("worker sleeping");
                    }
                    Err(error) => push_task_error(display, error),
                },
                _ => display.push_line(LineKind::Error, "usage: sleep PID TICKS"),
            }
            display.refresh_task_manager();
        }
        "wake" => {
            match parse_u32(trim(args)) {
                Some(pid) => match tasks.wake(pid, tick) {
                    Ok(()) => {
                        display.push_line(LineKind::Status, "worker ready");
                        display.set_status("worker woken");
                    }
                    Err(error) => push_task_error(display, error),
                },
                None => display.push_line(LineKind::Error, "usage: wake PID"),
            }
            display.refresh_task_manager();
        }
        "sched" => {
            let mut line = FixedText::from_str("workers=");
            line.push_u64(tasks.worker_len() as u64);
            line.push_str(" current=");
            match tasks.current_worker_id() {
                Some(pid) => line.push_u64(pid as u64),
                None => line.push_str("none"),
            }
            line.push_str(" quantum=");
            line.push_u64(tasks.quantum_ticks() as u64);
            line.push_str(" switches=");
            line.push_u64(tasks.total_switches());
            display.push_fixed(LineKind::Output, line);
            display.set_status("scheduler sampled");
        }
        "userabi" => {
            let mut line = FixedText::from_str("ring3=");
            line.push_str(if crate::userspace::probe_passed() {
                "passed"
            } else {
                "failed"
            });
            line.push_str(" abi=");
            line.push_u64(kernel::syscall::USER_ABI_VERSION);
            line.push_str(" elf=");
            line.push_str(if crate::userspace::elf_ready() {
                "ready"
            } else {
                "missing"
            });
            line.push_str(" proc=");
            line.push_u64(crate::userspace::process_count() as u64);
            line.push_str(" spaces=");
            line.push_u64(crate::userspace::address_space_count() as u64);
            line.push_str(" yields=");
            line.push_u64(crate::userspace::yield_count() as u64);
            line.push_str(" preempt=");
            line.push_u64(crate::userspace::preemption_count() as u64);
            line.push_str(" faults=");
            line.push_u64(crate::userspace::local_fault_count() as u64);
            display.push_fixed(LineKind::Output, line);
            display.set_status("userspace ABI sampled");
        }
        "taskmgr" => {
            tasks.mark_running(ids.taskmgr, tick);
            display.open_task_manager();
            display.set_status("task manager opened");
        }
        "game" | "demo" => {
            display.open_game();
            display.push_line(
                LineKind::Status,
                "Game surface opened: backbuffer blits + dirty app frames",
            );
            display.set_status("game opened");
        }
        "files" => {
            display.open_files();
            display.set_status("files opened");
        }
        "apps" => {
            display.open_files();
            display.open_task_manager();
            display.open_about();
            display.open_game();
            display.set_status("apps opened");
        }
        "echo" => {
            display.push_line(LineKind::Output, args);
            display.set_status("echo");
        }
        "uname" => {
            let mut line = FixedText::from_str("GenOS v0.9 desktop-kernel bootabi=");
            line.push_u64(boot_info.version as u64);
            line.push_str(" arch=x86_64");
            display.push_fixed(LineKind::Output, line);
            display.set_status("system identified");
        }
        "about" => {
            display.open_about();
            display.push_line(
                LineKind::Output,
                "GenOS 0.9 validates, maps, and launches separate ELF applications with an initial no-std userspace runtime.",
            );
            display.set_status("about");
        }
        "whoami" => {
            display.push_line(LineKind::Output, "genos");
            display.set_status("session user");
        }
        "time" => {
            let now = rtc::read();
            display.push_fixed(LineKind::Output, now.format_date_time());
            display.set_clock(now.format_clock());
            display.set_status("time read");
        }
        "ui" => {
            display.push_line(
                LineKind::Status,
                "display: backbuffered dirty-region desktop manager",
            );
            display.push_line(
                LineKind::Status,
                "input: ps/2 keyboard and mouse event queue",
            );
            display.push_line(LineKind::Status, "storage: writable session RAM VFS");
            let stats = interrupts::stats();
            let mut line = FixedText::from_str("irq: ticks=");
            line.push_u64(stats.ticks);
            line.push_str(" kbd=");
            line.push_u64(stats.keyboard_irqs);
            line.push_str(" mouse=");
            line.push_u64(stats.mouse_irqs);
            display.push_fixed(LineKind::Status, line);
            display.set_status("ui diagnostics");
        }
        "reboot" => arch::reboot(),
        "shutdown" => arch::shutdown(),
        _ => {
            display.push_line(LineKind::Error, "unknown command");
            display.set_status("command error");
        }
    }
}

fn report_vfs_result(display: &mut DisplayManager, result: Result<(), VfsError>, ok: &str) {
    match result {
        Ok(()) => {
            display.push_line(LineKind::Status, ok);
            display.set_status(ok);
        }
        Err(error) => {
            push_vfs_error(display, error);
            display.set_status("vfs error");
        }
    }
}

fn push_vfs_error(display: &mut DisplayManager, error: VfsError) {
    let text = match error {
        VfsError::Exists => "path already exists",
        VfsError::NotFound => "path not found",
        VfsError::NoSpace => "vfs has no space",
        VfsError::IsDirectory => "path is a directory",
        VfsError::NotDirectory => "path is not a directory",
        VfsError::InvalidPath => "invalid path",
    };
    display.push_line(LineKind::Error, text);
}

fn push_task_error(display: &mut DisplayManager, error: TaskError) {
    let text = match error {
        TaskError::TableFull => "process table is full",
        TaskError::NotFound => "pid not found",
        TaskError::Protected => "system task is protected",
        TaskError::InvalidState => "invalid task state or worker name",
    };
    display.push_line(LineKind::Error, text);
    display.set_status("task error");
}

fn parse_u32(text: &str) -> Option<u32> {
    let value = parse_u64(text)?;
    (value <= u32::MAX as u64).then_some(value as u32)
}

fn parse_u64(text: &str) -> Option<u64> {
    let text = trim(text);
    if text.is_empty() {
        return None;
    }
    let mut value = 0u64;
    for byte in text.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((byte - b'0') as u64)?;
    }
    Some(value)
}

fn resolve_path(cwd: &str, arg: &str) -> FixedText {
    let arg = trim(arg);
    if arg.is_empty() {
        return FixedText::from_str(cwd);
    }
    if arg.starts_with('/') {
        return FixedText::from_str(arg);
    }
    let mut path = FixedText::from_str(cwd);
    if path.as_str() != "/" {
        path.push_str("/");
    }
    path.push_str(arg);
    path
}

fn push_multiline(display: &mut DisplayManager, kind: LineKind, text: &str) {
    let bytes = text.as_bytes();
    let mut start = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let line = str::from_utf8(&bytes[start..index]).unwrap_or("");
            display.push_line(kind, line);
            start = index + 1;
        }
    }
    if start < bytes.len() {
        let line = str::from_utf8(&bytes[start..]).unwrap_or("");
        display.push_line(kind, line);
    }
}

fn split_once_space(text: &str) -> (&str, &str) {
    if let Some(index) = text.find(' ') {
        (&text[..index], trim(&text[index + 1..]))
    } else {
        (text, "")
    }
}

fn trim(mut text: &str) -> &str {
    while text.as_bytes().first() == Some(&b' ') {
        text = &text[1..];
    }
    while text.as_bytes().last() == Some(&b' ') {
        text = &text[..text.len() - 1];
    }
    text
}
