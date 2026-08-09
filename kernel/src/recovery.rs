#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryCommand {
    Help,
    Status,
    Memory,
    Reboot,
    Shutdown,
    Unknown,
}

pub fn parse(command: &str) -> RecoveryCommand {
    match command.trim() {
        "help" => RecoveryCommand::Help,
        "status" => RecoveryCommand::Status,
        "mem" => RecoveryCommand::Memory,
        "reboot" => RecoveryCommand::Reboot,
        "shutdown" => RecoveryCommand::Shutdown,
        _ => RecoveryCommand::Unknown,
    }
}

pub fn boundary_is_minimal() -> bool {
    matches!(parse("help"), RecoveryCommand::Help)
        && matches!(parse("status"), RecoveryCommand::Status)
        && matches!(parse("mem"), RecoveryCommand::Memory)
        && matches!(parse("reboot"), RecoveryCommand::Reboot)
        && matches!(parse("shutdown"), RecoveryCommand::Shutdown)
        && [
            "clear", "ls", "cat", "stat", "touch", "write", "append", "mkdir", "rm", "run", "ps",
            "kill", "wait",
        ]
        .iter()
        .all(|command| matches!(parse(command), RecoveryCommand::Unknown))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_surface_contains_only_boot_diagnostics_and_power_controls() {
        assert!(boundary_is_minimal());
        assert_eq!(parse("  status  "), RecoveryCommand::Status);
        assert_eq!(parse("status now"), RecoveryCommand::Unknown);
    }

    #[test]
    fn normal_shell_commands_are_not_kernel_commands() {
        for command in [
            "clear", "ls", "cat", "stat", "touch", "write", "append", "mkdir", "rm", "run", "ps",
            "kill", "wait", "echo", "uname",
        ] {
            assert_eq!(parse(command), RecoveryCommand::Unknown, "{command}");
        }
    }
}
