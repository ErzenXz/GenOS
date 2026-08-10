use genos_abi::BootInfo;
use kernel::{
    display::{DisplayManager, FixedText, LineKind},
    input::{InputEvent, KeyEvent},
    recovery::{self, RecoveryCommand},
};

use crate::{
    arch, input_hw, interrupts, memory, rtc,
    runtime::{RuntimeCoordinator, RuntimeEvent},
    serial, userspace,
};

pub fn run_terminal(boot_info: &'static BootInfo, mut runtime: RuntimeCoordinator) -> ! {
    let mut last_tick = interrupts::ticks();
    let mut irq_tick_marker_sent = false;
    let mut terminal_idle_marker_sent = false;
    let mut awaiting_command_completion = false;
    let mut last_was_cr = false;
    let mut serial_rx_marker_sent = false;
    serial::println("");
    serial::print("genos> ");

    loop {
        let tick = interrupts::poll_fallback_tick();
        let mut handled_event = false;

        if runtime.console_process_active() && runtime.console_input_ready() {
            for _ in 0..16 {
                let Some(byte) = serial::read_byte() else {
                    break;
                };
                if !serial_rx_marker_sent {
                    serial::println("SERIAL_RX_OK");
                    serial_rx_marker_sent = true;
                }
                if byte == b'\n' && last_was_cr {
                    last_was_cr = false;
                    continue;
                }
                last_was_cr = byte == b'\r';
                let event = match byte {
                    b'\r' | b'\n' => {
                        serial::println("");
                        awaiting_command_completion = true;
                        InputEvent::Key(KeyEvent::Enter)
                    }
                    8 | 0x7f => {
                        serial::print("\x08 \x08");
                        InputEvent::Key(KeyEvent::Backspace)
                    }
                    b'\t' => InputEvent::Key(KeyEvent::Tab),
                    0x1b => InputEvent::Key(KeyEvent::Escape),
                    byte if (0x20..=0x7e).contains(&byte) => {
                        serial::echo_byte(byte);
                        InputEvent::Key(KeyEvent::Char(byte))
                    }
                    _ => continue,
                };
                handled_event = true;
                runtime.record_input_activity(tick);
                match runtime.deliver_input(event) {
                    Ok(Some(update)) => write_terminal_update(update),
                    Ok(None) => {}
                    Err(_) => serial::println("terminal input delivery failed"),
                }
                break;
            }
        }

        if tick != last_tick {
            let batch = runtime.advance(tick);
            for event in batch.iter() {
                match event {
                    RuntimeEvent::Process(update) => write_terminal_update(update),
                    RuntimeEvent::Error(_) => serial::println("userspace lifecycle error"),
                }
            }
            last_tick = tick;
        }

        if awaiting_command_completion && runtime.console_input_ready() {
            serial::print("genos> ");
            awaiting_command_completion = false;
        }
        if !irq_tick_marker_sent && tick >= 100 {
            serial::println("IRQ_TICK_OK");
            irq_tick_marker_sent = true;
        }
        if !terminal_idle_marker_sent && tick >= 140 {
            serial::println("TERMINAL_IDLE_OK");
            terminal_idle_marker_sent = true;
        }
        if !runtime.console_process_active() {
            serial::println("RECOVERY_CONSOLE_READY");
            serial::println("recovery commands require the later serial recovery parser");
            let _ = boot_info;
            arch::halt_loop();
        }

        runtime.finish_iteration(handled_event, tick);
        core::hint::spin_loop();
    }
}

fn write_terminal_update(update: userspace::ProcessUpdate) {
    if let Some(userspace::ConsoleUpdate::Write { text, .. }) = update.console {
        serial::println(text.as_str());
    }
}

pub fn run(
    mut display: DisplayManager,
    boot_info: &'static BootInfo,
    mut runtime: RuntimeCoordinator,
) -> ! {
    let mut last_tick = interrupts::ticks();
    let mut last_clock_second = 255u8;
    let mut irq_tick_marker_sent = false;
    let mut display_idle_marker_sent = false;
    let mut recovery_announced = false;

    loop {
        input_hw::poll();
        let tick = interrupts::poll_fallback_tick();
        let mut handled_event = false;
        let irq_stats = interrupts::stats();

        display.sync_stats(
            input_hw::mouse_state(),
            input_hw::event_depth(),
            runtime.vfs().count(),
            irq_stats.ticks,
        );
        display.refresh_stats_if_due(tick);
        display.animate_if_due(tick);

        if !runtime.console_process_active() && !recovery_announced {
            display.push_line(
                LineKind::Error,
                "SHELL.ELF is not live; emergency recovery console active",
            );
            display.push_line(
                LineKind::Status,
                "Recovery is limited to help, status, mem, reboot, and shutdown",
            );
            display.set_status("emergency recovery console");
            serial::println("RECOVERY_CONSOLE_READY");
            recovery_announced = true;
        }

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

        while let Some(next_event) = input_hw::peek_event() {
            let console_busy = runtime.console_process_active() && !runtime.console_input_ready();
            if console_busy
                && matches!(
                    next_event,
                    InputEvent::Key(
                        KeyEvent::Char(_)
                            | KeyEvent::Enter
                            | KeyEvent::Backspace
                            | KeyEvent::ArrowUp
                            | KeyEvent::ArrowDown
                    )
                )
            {
                break;
            }
            let Some(event) = input_hw::pop_event() else {
                break;
            };
            handled_event = true;
            runtime.record_input_activity(tick);
            match event {
                InputEvent::Key(KeyEvent::Escape) => {
                    display.dismiss_focused();
                    continue;
                }
                InputEvent::Key(KeyEvent::Tab) => {
                    display.cycle_focus();
                    continue;
                }
                _ => {}
            }
            match runtime.deliver_input(event) {
                Ok(Some(update)) => {
                    apply_process_update(&mut display, update);
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    push_launch_error(&mut display, error);
                    continue;
                }
            }
            match event {
                InputEvent::Key(KeyEvent::Enter) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    runtime.record_shell_activity(tick);
                    let input = display.take_input();
                    let command = input.as_str();
                    if !command.is_empty() {
                        let mut prompt = FixedText::from_str("recovery> ");
                        prompt.push_str(command);
                        display.push_fixed(LineKind::Prompt, prompt);
                    }
                    execute_recovery(command, &mut display, boot_info);
                }
                InputEvent::Key(KeyEvent::Backspace) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    runtime.record_shell_activity(tick);
                    let _ = display.input_backspace();
                }
                InputEvent::Key(KeyEvent::Char(byte)) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    runtime.record_shell_activity(tick);
                    let _ = display.input_push(byte);
                }
                InputEvent::Key(KeyEvent::ArrowUp) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    runtime.record_shell_activity(tick);
                }
                InputEvent::Key(KeyEvent::ArrowDown) => {
                    if !display.shell_input_active() {
                        continue;
                    }
                    runtime.record_shell_activity(tick);
                }
                InputEvent::Key(KeyEvent::Escape) => display.dismiss_focused(),
                InputEvent::Key(KeyEvent::Tab) => display.cycle_focus(),
                InputEvent::MouseMove { buttons, .. } => {
                    display.handle_mouse_move(input_hw::mouse_state().position, buttons.left);
                }
                InputEvent::MouseButton { buttons, .. } => {
                    if buttons.left {
                        runtime.record_desktop_activity(tick);
                        display.handle_mouse_down(input_hw::mouse_state().position);
                    } else {
                        display.end_drag();
                    }
                }
            }
        }

        if tick != last_tick {
            let batch = runtime.advance(tick);
            if batch.vfs_changed {
                display.sync_vfs(runtime.vfs());
            }
            for event in batch.iter() {
                match event {
                    RuntimeEvent::Process(update) => apply_process_update(&mut display, update),
                    RuntimeEvent::Error(error) => push_launch_error(&mut display, error),
                }
            }
            last_tick = tick;
        }

        runtime.finish_iteration(handled_event, tick);
        display.flush(runtime.task_snapshot());
        core::hint::spin_loop();
    }
}

fn execute_recovery(command: &str, display: &mut DisplayManager, boot_info: &BootInfo) {
    serial::print("recovery: ");
    serial::println(command);

    match recovery::parse(command) {
        RecoveryCommand::Help => {
            display.push_line(
                LineKind::Output,
                "recovery commands: help status mem reboot shutdown",
            );
            display.push_line(
                LineKind::Status,
                "Normal commands require the isolated SHELL.ELF process",
            );
            display.set_status("recovery help printed");
        }
        RecoveryCommand::Status => {
            let mut line = FixedText::from_str("GenOS v0.49 recovery bootabi=");
            line.push_u64(boot_info.version as u64);
            line.push_str(" userabi=");
            line.push_u64(kernel::syscall::USER_ABI_VERSION);
            display.push_fixed(LineKind::Output, line);

            let mut state = FixedText::from_str("ring3-probe=");
            state.push_str(if userspace::probe_passed() {
                "passed"
            } else {
                "failed"
            });
            state.push_str(" live-processes=");
            state.push_u64(userspace::active_process_count() as u64);
            display.push_fixed(LineKind::Status, state);

            let irq = interrupts::stats();
            let mut irq_line = FixedText::from_str("irq-ticks=");
            irq_line.push_u64(irq.ticks);
            irq_line.push_str(" keyboard=");
            irq_line.push_u64(irq.keyboard_irqs);
            irq_line.push_str(" mouse=");
            irq_line.push_u64(irq.mouse_irqs);
            display.push_fixed(LineKind::Status, irq_line);
            display.push_fixed(LineKind::Status, rtc::read().format_date_time());
            display.set_status("recovery status sampled");
        }
        RecoveryCommand::Memory => {
            let mut line = FixedText::from_str("usable-bytes=");
            line.push_u64(memory::usable_bytes());
            line.push_str(" allocated-frames=");
            line.push_u64(memory::allocated_frames());
            line.push_str(" recycled-frames=");
            line.push_u64(memory::recycled_frames() as u64);
            display.push_fixed(LineKind::Output, line);
            display.set_status("recovery memory sampled");
        }
        RecoveryCommand::Reboot => arch::reboot(),
        RecoveryCommand::Shutdown => arch::shutdown(),
        RecoveryCommand::Unknown => {
            display.push_line(
                LineKind::Error,
                "Unavailable in Ring 0; normal commands require SHELL.ELF",
            );
            display.set_status("recovery command rejected");
        }
    }
}
fn apply_process_update(display: &mut DisplayManager, update: userspace::ProcessUpdate) {
    if !update.output.is_empty() {
        let mut line = FixedText::from_str("app[");
        line.push_u64(update.pid as u64);
        line.push_str("]: ");
        line.push_str(update.output.as_str());
        display.push_fixed(LineKind::Output, line);
    }
    if let Some(console) = update.console {
        match console {
            userspace::ConsoleUpdate::Write { kind, text } => display.push_fixed(kind, text),
            userspace::ConsoleUpdate::SetInput(text) => display.set_input(text),
            userspace::ConsoleUpdate::Clear => display.clear_shell(),
        }
    }
    let terminal_state = matches!(
        update.state,
        userspace::ManagedState::Exited
            | userspace::ManagedState::Faulted
            | userspace::ManagedState::Killed
    );
    if update.state != userspace::ManagedState::Ready && (!update.console_process || terminal_state)
    {
        let mut line = FixedText::from_str("ELF task-pid=");
        line.push_u64(update.task_id as u64);
        line.push_str(" state=");
        line.push_str(managed_state_text(update.state));
        line.push_str(" exit=");
        line.push_u64(update.exit_code as u64);
        line.push_str(" preempt=");
        line.push_u64(update.preemptions);
        display.push_fixed(LineKind::Status, line);
        display.refresh_task_manager();
    }
}

fn managed_state_text(state: userspace::ManagedState) -> &'static str {
    match state {
        userspace::ManagedState::Ready => "ready",
        userspace::ManagedState::Sleeping => "sleeping",
        userspace::ManagedState::Waiting => "waiting",
        userspace::ManagedState::Exited => "exited",
        userspace::ManagedState::Faulted => "fault",
        userspace::ManagedState::Killed => "killed",
    }
}

fn push_launch_error(display: &mut DisplayManager, error: userspace::LaunchError) {
    let text = match error {
        userspace::LaunchError::ImageUnavailable => "userspace process not found",
        userspace::LaunchError::ProcessBuildFailed => "INIT.ELF failed validation or mapping",
        userspace::LaunchError::ProcessFaulted => "INIT.ELF terminated with a CPU fault",
        userspace::LaunchError::InvalidResult => "process is not in the required state",
        userspace::LaunchError::ProcessTableFull => "userspace process table is full; wait first",
    };
    display.push_line(LineKind::Error, text);
    display.set_status("userspace lifecycle error");
}
