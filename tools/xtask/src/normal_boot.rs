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

#[derive(Clone, Copy)]
enum Reply {
    Exact(&'static str),
    Prefix(&'static str),
    Network,
}
impl Reply {
    fn text(self, network: bool) -> &'static str {
        match self {
            Self::Exact(text) | Self::Prefix(text) => text,
            Self::Network if network => "network online - DHCP configuration available",
            Self::Network => "network unavailable",
        }
    }
    fn matches(self, line: &str, network: bool) -> bool {
        let text = self.text(network);
        match self {
            Self::Prefix(_) => line.starts_with(text),
            _ => line == text,
        }
    }
}

const STEPS: &[(&str, Reply)] = &[
    (
        "help\r",
        Reply::Exact("Use /USER/... for files; JOB is the number shown by run or ps."),
    ),
    ("cat /USER/SHELL.TXT\r", Reply::Exact("file unavailable")),
    ("cat /USER/APP.TXT\r", Reply::Exact("file unavailable")),
    (
        "uname\r",
        Reply::Exact("GenOS v0.56 ring3-shell x86_64 ABI 18"),
    ),
    ("net\r", Reply::Network),
    ("mem\r", Reply::Exact("consistent=yes")),
    (
        "write /MEMORY.STATUS invalid\r",
        Reply::Exact("file change denied; use /USER/FILE"),
    ),
    (
        "touch /USER/..\r",
        Reply::Exact("file change denied; use /USER/FILE"),
    ),
    ("run init hold\r", Reply::Prefix("job 1 started task=")),
    ("mem\r", Reply::Exact("consistent=yes")),
    ("ps\r", Reply::Prefix("job 1 task=")),
    ("kill 1\r", Reply::Exact("killed job 1")),
    ("wait 1\r", Reply::Exact("reaped job 1")),
    ("mem\r", Reply::Exact("consistent=yes")),
    (
        "write /USER/NORMAL.TXT NORMAL_BOOT_OK\r",
        Reply::Exact("file written"),
    ),
    ("cat /USER/NORMAL.TXT\r", Reply::Exact("NORMAL_BOOT_OK")),
    ("clear\r", Reply::Exact("\x1b[2J\x1b[H")),
    ("echo terminal ready\r", Reply::Exact("terminal ready")),
];

pub fn repetition_count(value: Option<&str>) -> Result<usize, String> {
    let count = match value {
        None => 10,
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "repeat count must be an integer from 1 through 1000")?,
    };
    if !(1..=1000).contains(&count) {
        return Err("repeat count must be an integer from 1 through 1000".into());
    }
    Ok(count)
}

#[derive(Default)]
pub struct Transcript {
    policy: bool,
    kernel_ready: bool,
    shell_ready: bool,
    step: usize,
    command_seen: bool,
    network: bool,
    memory_baseline: Option<(u64, u64)>,
    memory_samples: usize,
    memory_seen: bool,
}

impl Transcript {
    pub fn new(network: bool) -> Self {
        Self {
            network,
            ..Self::default()
        }
    }

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
        if [
            "USER_INPUT_",
            "USER_CONSOLE_WRITE",
            "USER_FILE_OPEN_",
            "USER_HANDLE_",
        ]
        .iter()
        .any(|marker| line.contains(marker))
        {
            return Err(format!(
                "normal terminal leaked development tracing: {line}"
            ));
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
            if line.strip_prefix("genos> ") == Some(STEPS[self.step].0.trim_end_matches('\r')) {
                self.command_seen = true;
                return Ok(None);
            }
            if self.command_seen && STEPS[self.step].0 == "mem\r" {
                if line.starts_with("frames_total=") {
                    if self.memory_seen {
                        return Err("duplicate memory counters".into());
                    }
                    let fields: Vec<_> = line.split_whitespace().collect();
                    let count = |index: usize, prefix: &str| -> Option<u64> {
                        fields.get(index)?.strip_prefix(prefix)?.parse().ok()
                    };
                    let (Some(total), Some(live), Some(free)) = (
                        count(0, "frames_total="),
                        count(1, "live="),
                        count(2, "free="),
                    ) else {
                        return Err("malformed memory counters".into());
                    };
                    if fields.len() != 3 || live.checked_add(free) != Some(total) {
                        return Err("incoherent memory counters".into());
                    }
                    match (self.memory_samples, self.memory_baseline) {
                        (0, None) => self.memory_baseline = Some((total, live)),
                        (1, Some((expected_total, baseline)))
                            if total == expected_total && live > baseline => {}
                        (2, Some((expected_total, baseline)))
                            if total == expected_total && live == baseline => {}
                        _ => {
                            return Err(
                                "process lifecycle did not preserve allocator baseline".into()
                            )
                        }
                    }
                    self.memory_seen = true;
                }
                if STEPS[self.step].1.matches(line, self.network) && !self.memory_seen {
                    return Err("missing memory counters before consistency result".into());
                }
            }
            if self.command_seen && STEPS[self.step].1.matches(line, self.network) {
                if STEPS[self.step].0 == "mem\r" {
                    self.memory_samples += 1;
                    self.memory_seen = false;
                }
                self.command_seen = false;
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
    fn ready(network: bool) -> Transcript {
        let mut proof = Transcript::new(network);
        proof.observe("BOOT_MODE normal").unwrap();
        proof.observe("GENOS_READY").unwrap();
        assert_eq!(
            proof.observe("NORMAL_SHELL_READY").unwrap(),
            Some(STEPS[0].0)
        );
        proof
    }

    fn feed_memory(proof: &mut Transcript, command: &str) {
        if command == "mem\r" {
            let live = if proof.memory_samples == 1 { 110 } else { 100 };
            proof
                .observe(&format!(
                    "frames_total=1000 live={live} free={}",
                    1000 - live
                ))
                .unwrap();
        }
    }

    #[test]
    fn stale_missing_or_leaked_memory_samples_are_rejected() {
        for (samples, record) in [
            (0, "frames_total=1000 live=100 free=901"),
            (1, "frames_total=1000 live=100 free=900"),
            (2, "frames_total=1000 live=101 free=899"),
            (2, "consistent=yes"),
        ] {
            let mut proof = ready(false);
            proof.step = STEPS.iter().position(|step| step.0 == "mem\r").unwrap();
            proof.command_seen = true;
            proof.memory_samples = samples;
            proof.memory_baseline = (samples > 0).then_some((1000, 100));
            assert!(proof.observe(record).is_err(), "{record}");
        }
    }

    #[test]
    fn repetition_budget_is_explicit_and_bounded() {
        assert_eq!(repetition_count(None), Ok(10));
        for count in [1, 10, 1000] {
            assert_eq!(repetition_count(Some(&count.to_string())), Ok(count));
        }
        for count in ["0", "1001", "-1", "hello", "1.5", "184467440737095516160"] {
            assert!(repetition_count(Some(count)).is_err());
        }
    }

    #[test]
    fn visible_command_responses_drive_both_network_configurations() {
        for network in [false, true] {
            let mut proof = ready(network);
            for (index, (command, response)) in STEPS.iter().enumerate() {
                let echo = format!("genos> {}", command.trim_end_matches('\r'));
                assert_eq!(proof.observe(&echo).unwrap(), None);
                feed_memory(&mut proof, command);
                assert_eq!(
                    proof.observe(response.text(network)).unwrap(),
                    STEPS.get(index + 1).map(|s| s.0)
                );
            }
            assert!(proof.complete());
        }
    }

    #[test]
    fn responses_require_their_own_echo_and_the_correct_result() {
        let mut proof = ready(false);
        for (index, (command, response)) in STEPS.iter().enumerate() {
            assert_eq!(proof.observe(response.text(false)).unwrap(), None);
            assert_eq!(proof.step(), index);
            let echo = format!("genos> {}", command.trim_end_matches('\r'));
            proof.observe(&echo).unwrap();
            feed_memory(&mut proof, command);
            assert_eq!(proof.observe("unrelated or stale response").unwrap(), None);
            if matches!(response, Reply::Network) {
                assert_eq!(proof.observe(response.text(true)).unwrap(), None);
            }
            proof.observe(response.text(false)).unwrap();
            assert_eq!(proof.step(), index + 1);
        }
        assert!(proof.complete());
    }

    #[test]
    fn missing_duplicate_or_embedded_readiness_never_passes() {
        let mut proof = Transcript::default();
        assert!(proof.observe("NORMAL_SHELL_READY").is_err());
        assert!(proof.observe("GENOS_READY").is_err());
        proof.observe("echo NORMAL_SHELL_READY").unwrap();
        assert!(!proof.complete());
        let mut proof = ready(false);
        assert!(proof.observe("NORMAL_SHELL_READY").is_err());
        assert!(proof.observe("BOOT_MODE normal").is_err());
    }

    #[test]
    fn failure_proof_and_debug_trace_output_are_rejected() {
        for marker in FORBIDDEN.iter().copied().chain([
            "KERNEL PANIC",
            "EXCEPTION_FATAL_HALT",
            "RECOVERY_CONSOLE_READY",
            "USER_INPUT_BLOCK pid=4",
            "USER_CONSOLE_WRITE pid=4 text=hello",
            "USER_HANDLE_READ_OK",
        ]) {
            assert!(ready(false).observe(marker).is_err(), "{marker}");
        }
    }
}
