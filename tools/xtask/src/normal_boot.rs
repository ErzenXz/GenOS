//! Ordered evidence for the real shell in a normal (non-validation) image.
//! Each response must arrive after its command; boot markers are exact lines.

pub const FORBIDDEN: &[&str] = &[
    "BOOT_MODE validation",
    "USER_ELF_VALIDATED",
    "PROCESS_GENERATION_STRESS_READY",
    "RUNTIME_ROLLBACK_READY",
    "CONSOLE_TRANSCRIPT_READY",
    "USER_SOCKET_CAPABILITY_READY",
    "SDK_APPLICATION_READY",
    "SCHED_DISPATCH_BENCH",
    "SUPERVISOR_CLEANUP_READY",
];

const STEPS: &[(&str, &str)] = &[
    ("cat /USER/SHELL.TXT\r", "text=file unavailable"),
    ("cat /USER/APP.TXT\r", "text=file unavailable"),
    ("uname\r", "text=GenOS v0.56 ring3-shell x86_64 ABI 18"),
    ("run init hold\r", "USER_PROCESS_LAUNCHED owner=4 "),
    ("ps\r", "USER_PROCESS_STATUS owner=4 "),
    ("kill 1\r", "USER_PROCESS_KILLED owner=4 "),
    ("wait 1\r", "USER_PROCESS_REAPED owner=4 "),
    (
        "write /USER/NORMAL.TXT NORMAL_BOOT_OK\r",
        "text=file written",
    ),
    ("cat /USER/NORMAL.TXT\r", "text=NORMAL_BOOT_OK"),
];

#[derive(Default)]
pub struct Transcript {
    policy: bool,
    kernel_ready: bool,
    shell_ready: bool,
    step: usize,
}

impl Transcript {
    /// Return the next command only after the preceding response was observed.
    pub fn observe(&mut self, line: &str) -> Result<Option<&'static str>, String> {
        if FORBIDDEN.iter().any(|marker| line.contains(marker)) {
            return Err(format!("normal boot executed a validation proof: {line}"));
        }
        if [
            "_FAILED",
            "KERNEL PANIC",
            "EXCEPTION_FATAL_HALT",
            "RECOVERY_CONSOLE_READY",
        ]
        .iter()
        .any(|marker| line.contains(marker))
        {
            return Err(format!("normal boot reported a failure: {line}"));
        }
        match line {
            "BOOT_MODE normal" => {
                if self.policy {
                    return Err("duplicate boot policy".into());
                }
                self.policy = true;
            }
            "GENOS_READY" => {
                if !self.policy || self.kernel_ready {
                    return Err("invalid kernel readiness order".into());
                }
                self.kernel_ready = true;
            }
            "NORMAL_SHELL_READY" => {
                if !self.kernel_ready || self.shell_ready {
                    return Err("invalid shell readiness order".into());
                }
                self.shell_ready = true;
                return Ok(Some(STEPS[0].0));
            }
            _ => {}
        }
        if self.shell_ready && self.step < STEPS.len() {
            let response = STEPS[self.step].1;
            let matches = if response.starts_with("text=") {
                line.strip_prefix("USER_CONSOLE_WRITE pid=4 ") == Some(response)
            } else {
                line.starts_with(response)
            };
            if matches {
                self.step += 1;
                return Ok(STEPS.get(self.step).map(|step| step.0));
            }
        }
        Ok(None)
    }

    pub fn complete(&self) -> bool {
        self.shell_ready && self.step == STEPS.len()
    }
    pub fn step(&self) -> usize {
        self.step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Transcript {
        let mut proof = Transcript::default();
        proof.observe("BOOT_MODE normal").unwrap();
        proof.observe("GENOS_READY").unwrap();
        assert_eq!(
            proof.observe("NORMAL_SHELL_READY").unwrap(),
            Some(STEPS[0].0)
        );
        proof
    }

    #[test]
    fn real_console_responses_drive_commands_in_order() {
        let mut proof = ready();
        for (index, (_, response)) in STEPS.iter().enumerate() {
            let line = if response.starts_with("text=") {
                format!("USER_CONSOLE_WRITE pid=4 {response}")
            } else {
                format!("{response}pid=5")
            };
            assert_eq!(
                proof.observe(&line).unwrap(),
                STEPS.get(index + 1).map(|s| s.0)
            );
        }
        assert!(proof.complete());
    }

    #[test]
    fn missing_duplicate_and_embedded_readiness_cannot_pass() {
        let mut proof = Transcript::default();
        assert!(proof.observe("NORMAL_SHELL_READY").is_err());
        assert!(proof.observe("GENOS_READY").is_err());
        proof
            .observe("USER_CONSOLE_WRITE pid=4 text=NORMAL_SHELL_READY")
            .unwrap();
        assert!(!proof.complete());
        let mut proof = ready();
        assert!(proof.observe("BOOT_MODE normal").is_err());
        assert!(proof.observe("NORMAL_SHELL_READY").is_err());
    }

    #[test]
    fn echoed_stale_wrong_owner_and_partial_responses_do_not_advance() {
        let mut proof = ready();
        for line in [
            "file unavailable",
            "USER_CONSOLE_WRITE pid=5 text=file unavailable",
            "USER_CONSOLE_WRITE pid=4 text=file unavailable extra",
            "USER_PROCESS_REAPED owner=4 pid=5",
            "USER_CONSOLE_WRITE pid=4 text=NORMAL_BOOT_OK",
        ] {
            assert_eq!(proof.observe(line).unwrap(), None);
            assert_eq!(proof.step(), 0);
        }
    }

    #[test]
    fn validation_and_fault_markers_fail_even_after_readiness() {
        for marker in FORBIDDEN.iter().copied().chain([
            "KERNEL PANIC",
            "USER_PROCESS_FAILED",
            "EXCEPTION_FATAL_HALT",
            "RECOVERY_CONSOLE_READY",
        ]) {
            assert!(ready().observe(marker).is_err(), "{marker}");
        }
    }
}
