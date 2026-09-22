//! A live validation run owns its process, pipes and unique retained evidence.
//! Every exit path, including preflight/spawn failure, finalizes the manifest.
use crate::{evidence_contract::Contract, reference_vm};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct RunPolicy {
    budget: Duration,
    minimum_after_ready: Duration,
    probe_uname: bool,
}
impl RunPolicy {
    pub fn boot(seconds: u64, hold_seconds: u64) -> Self {
        Self {
            budget: Duration::from_secs(seconds),
            minimum_after_ready: Duration::from_secs(hold_seconds),
            probe_uname: false,
        }
    }
    pub fn serial() -> Self {
        Self {
            probe_uname: true,
            ..Self::boot(30, 0)
        }
    }
}

pub struct Evidence {
    pub directory: PathBuf,
    run_id: String,
    report: String,
    legacy_serial: Option<PathBuf>,
    finalized: bool,
}
impl Evidence {
    pub fn new(label: &str, legacy_serial: Option<&Path>) -> Result<Self, String> {
        if label.is_empty()
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("invalid evidence label".into());
        }
        let run_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
            .to_string();
        let directory = PathBuf::from(format!("build/validation-evidence/{run_id}/{label}"));
        fs::create_dir_all(directory.parent().ok_or("missing evidence parent")?)
            .map_err(|e| e.to_string())?;
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let evidence = Self {
            directory,
            report: format!("run_id={run_id}\nlabel={label}\n"),
            run_id,
            legacy_serial: legacy_serial.map(Path::to_path_buf),
            finalized: false,
        };
        evidence.write_manifest("incomplete", None)?;
        // Both files exist before environment lookup or process launch. An
        // absent emulator must leave actionable evidence, not a stale old log.
        File::create(evidence.directory.join("serial.log")).map_err(|e| e.to_string())?;
        File::create(evidence.directory.join("qemu.log")).map_err(|e| e.to_string())?;
        Ok(evidence)
    }
    fn write_manifest(&self, status: &str, failure: Option<&str>) -> Result<(), String> {
        let text = format!("status={status}\n{}failure={failure:?}\n", self.report);
        fs::write(self.directory.join("manifest.txt"), text).map_err(|e| e.to_string())
    }
    pub fn finish(mut self, result: Result<(), String>) -> Result<(), String> {
        let result = result.and_then(|()| {
            if fs::metadata(self.directory.join("serial.log"))
                .map_err(|e| e.to_string())?
                .len()
                == 0
            {
                Err("cannot pass a run without serial evidence".into())
            } else {
                Ok(())
            }
        });
        for name in ["serial.log", "qemu.log"] {
            if let Ok(hash) = reference_vm::sha256(&self.directory.join(name)) {
                self.report
                    .push_str(&format!("{}_sha256={hash}\n", name.replace('.', "_")));
            }
        }
        self.write_manifest(
            if result.is_ok() { "passed" } else { "failed" },
            result.as_ref().err().map(String::as_str),
        )?;
        self.finalized = true;
        if let Some(path) = &self.legacy_serial {
            fs::copy(self.directory.join("serial.log"), path).map_err(|e| e.to_string())?;
        }
        result.map_err(|error| format!("{error}; evidence: {}", self.directory.display()))
    }
    pub fn run(
        &mut self,
        command: &mut Command,
        firmware: &Path,
        data: &Path,
        contract: Contract<'_>,
        policy: RunPolicy,
        on_line: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        self.report
            .push_str(&reference_vm::environment(firmware)?.report);
        self.report.push_str(&format!("commit={}\nworking_tree_status={:?}\nimage_sha256={}\ndata_image_initial_sha256={}\nbudget_ms={}\nminimum_after_ready_ms={}\n",
            reference_vm::output("git", &["rev-parse", "HEAD"])?,
            reference_vm::output("git", &["status", "--porcelain"] )?,
            reference_vm::sha256(Path::new(crate::IMAGE))?, reference_vm::sha256(data)?,
            policy.budget.as_millis(), policy.minimum_after_ready.as_millis()));
        self.run_live(command, contract, policy, on_line)
    }
    fn run_live(
        &mut self,
        command: &mut Command,
        contract: Contract<'_>,
        policy: RunPolicy,
        mut on_line: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let RunPolicy {
            budget,
            minimum_after_ready,
            probe_uname,
        } = policy;
        command.args([
            "-display",
            "none",
            "-monitor",
            "none",
            "-serial",
            "stdio",
            "-no-reboot",
        ]);
        self.report.push_str(&format!("command={command:?}\n"));
        self.write_manifest("incomplete", None)?;
        let stderr = File::create(self.directory.join("qemu.log")).map_err(|e| e.to_string())?;
        let mut log = File::create(self.directory.join("serial.log")).map_err(|e| e.to_string())?;
        let mut guest = Guest::spawn(command, stderr)?;
        let started = Instant::now();
        let deadline = started + budget;
        let mut output = String::new();
        let mut freshness = ValidationFreshness::new(&self.run_id)?;
        let mut issued = false;
        let mut ready_at = None;
        let mut settled = None;
        let mut uname = UnameProof::default();
        while Instant::now() < deadline {
            // Drain the available batch before evaluating completion; a success
            // followed by a duplicate/failure in the same batch cannot pass.
            let next = guest.receiver.recv_timeout(Duration::from_millis(50));
            let mut batch = next.ok().into_iter().chain(guest.receiver.try_iter());
            for line in &mut batch {
                let line = line?;
                writeln!(log, "{line}").map_err(|e| e.to_string())?;
                output.push_str(&line);
                output.push('\n');
                if output.len() > 32 * 1024 * 1024 {
                    return Err("serial evidence exceeded 32 MiB bound".into());
                }
                freshness.observe(&line)?;
                if probe_uname {
                    uname.observe(&line)?;
                }
                on_line(&line)?;
                if line == "GENOS_READY" && ready_at.is_none() {
                    ready_at = Some(Instant::now());
                    self.report.push_str(&format!(
                        "boot_ready_ms={}\n",
                        started.elapsed().as_millis()
                    ));
                }
            }
            log.flush().map_err(|e| e.to_string())?;
            if let Some(status) = guest.child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!("QEMU exited before stable completion: {status}"));
            }
            let proof = contract.assess(&output)?;
            if proof && !issued && ready_at.is_some_and(|t| t.elapsed() >= minimum_after_ready) {
                if probe_uname && !uname.complete() {
                    if !uname.issued {
                        uname.issued = true;
                        guest
                            .input
                            .write_all(b"uname\r")
                            .and_then(|()| guest.input.flush())
                            .map_err(|e| format!("uname command write failed: {e}"))?;
                    }
                    continue;
                }
                guest
                    .input
                    .write_all(freshness.issue()?.as_bytes())
                    .and_then(|()| guest.input.flush())
                    .map_err(|e| format!("serial challenge write failed: {e}"))?;
                issued = true;
            }
            if proof && freshness.complete() {
                let since = settled.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_millis(500) {
                    self.report.push_str(&format!(
                        "fresh_serial_challenge=true\ncompleted_ms={}\n",
                        started.elapsed().as_millis()
                    ));
                    return Ok(());
                }
            }
        }
        Err(format!(
            "validation deadline expired after {} ms (challenge_issued={issued}, response={})",
            budget.as_millis(),
            freshness.complete()
        ))
    }
}
impl Drop for Evidence {
    fn drop(&mut self) {
        if !self.finalized {
            let _ = self.write_manifest("failed", Some("run abandoned before finalization"));
        }
    }
}

// Validation builds deliberately retain a diagnostic console record as well
// as the visible Ring 3 output. Require both streams, not a broad trace-strip.
struct ConsoleRoundTrip {
    command: String,
    response: String,
    pid: Option<u8>,
    prompt: bool,
    traced_response: bool,
    response_seen: bool,
}
impl ConsoleRoundTrip {
    fn new(command: String, response: String) -> Self {
        Self {
            command,
            response,
            pid: None,
            prompt: false,
            traced_response: false,
            response_seen: false,
        }
    }
    fn observe(&mut self, line: &str, issued: bool) -> Result<(), String> {
        if !issued {
            return Err("unsolicited validation console proof".into());
        }
        let prompt = format!("/> {}", self.command);
        if let Some(record) = line.strip_prefix("USER_CONSOLE_WRITE pid=") {
            let (pid, text) = record
                .split_once(" text=")
                .ok_or("malformed console diagnostic")?;
            if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
                return Err("invalid console owner encoding".into());
            }
            let pid = pid
                .parse::<u8>()
                .ok()
                .filter(|pid| *pid != 0)
                .ok_or("invalid console owner")?;
            if text == prompt {
                if self.pid.is_some() {
                    return Err("duplicate console command diagnostic".into());
                }
                self.pid = Some(pid);
            } else if text == self.response {
                if self.pid != Some(pid) || !self.prompt || self.traced_response {
                    return Err(
                        "wrong-owner, duplicate or premature console response diagnostic".into(),
                    );
                }
                self.traced_response = true;
            } else {
                return Err("stale or forged console diagnostic payload".into());
            }
        } else if line == prompt {
            if self.pid.is_none() || self.prompt {
                return Err("duplicate or premature visible command".into());
            }
            self.prompt = true;
        } else if line == self.response {
            if !self.prompt || !self.traced_response || self.response_seen {
                return Err("duplicate or premature visible response".into());
            }
            self.response_seen = true;
        } else {
            return Err("stale or embedded validation console proof".into());
        }
        Ok(())
    }
    fn complete(&self) -> bool {
        self.pid.is_some() && self.prompt && self.traced_response && self.response_seen
    }
}

struct ValidationFreshness {
    issued: bool,
    command: String,
    console: ConsoleRoundTrip,
}
impl ValidationFreshness {
    fn new(run_id: &str) -> Result<Self, String> {
        if run_id.is_empty() || run_id.len() > 32 || !run_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err("run identity must contain 1–32 ASCII digits".into());
        }
        let token = format!("GENOS_RUN_{run_id}");
        let command = format!("echo {token}");
        Ok(Self {
            issued: false,
            command: format!("{command}\r"),
            console: ConsoleRoundTrip::new(command, token),
        })
    }
    fn issue(&mut self) -> Result<&str, String> {
        if self.issued {
            return Err("validation challenge issued twice".into());
        }
        self.issued = true;
        Ok(&self.command)
    }
    fn observe(&mut self, line: &str) -> Result<(), String> {
        if !line.contains("GENOS_RUN_") {
            return Ok(());
        }
        self.console.observe(line, self.issued)
    }
    fn complete(&self) -> bool {
        self.console.complete()
    }
}

struct UnameProof {
    issued: bool,
    received: bool,
    console: ConsoleRoundTrip,
}
impl Default for UnameProof {
    fn default() -> Self {
        Self {
            issued: false,
            received: false,
            console: ConsoleRoundTrip::new(
                "uname".into(),
                "GenOS v0.56 ring3-shell x86_64 ABI 18".into(),
            ),
        }
    }
}
impl UnameProof {
    fn observe(&mut self, line: &str) -> Result<(), String> {
        if matches!(line, "SERIAL_RX_OK" | "genos> SERIAL_RX_OK") {
            if !self.issued || self.received {
                return Err("stale/duplicate serial receive proof".into());
            }
            self.received = true;
        } else if self.issued
            && (line.contains("/> uname") || line.contains(&self.console.response))
        {
            self.console.observe(line, true)?;
        }
        Ok(())
    }
    fn complete(&self) -> bool {
        self.issued && self.received && self.console.complete()
    }
}

struct Guest {
    child: Child,
    input: ChildStdin,
    receiver: mpsc::Receiver<Result<String, String>>,
    reader: Option<thread::JoinHandle<()>>,
}
impl Guest {
    fn spawn(command: &mut Command, stderr: File) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|e| format!("QEMU launch failed: {e}"))?;
        let input = child.stdin.take().expect("requested child stdin pipe");
        let stdout = child.stdout.take().expect("requested child stdout pipe");
        let (sender, receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender
                    .send(line.map_err(|e| format!("serial read failed: {e}")))
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            receiver,
            reader: Some(reader),
        })
    }
}
impl Drop for Guest {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_child_io_rejects_stale_incomplete_duplicate_and_early_exit_evidence() {
        let script = r#"
import pathlib, sys, time
log = pathlib.Path(sys.argv[1]).read_text()
mode = sys.argv[2]
if mode == 'missing': log = log.replace('USERMODE_READY\n', '')
sys.stdout.write(log); sys.stdout.flush()
if mode == 'exit':
    sys.stderr.write('fixture launch failed before challenge\n'); sys.exit(7)
command = bytearray()
while not command.endswith(b'\r'):
    value = sys.stdin.buffer.read(1)
    if not value: sys.exit(8)
    command.extend(value)
text = command.decode().strip()
if mode == 'stale': text = 'echo GENOS_RUN_0'
print('USER_CONSOLE_WRITE pid=33 text=/> ' + text, flush=True)
print('/> ' + text, flush=True)
print('USER_CONSOLE_WRITE pid=33 text=' + text.removeprefix('echo '), flush=True)
print(text.removeprefix('echo '), flush=True)
if mode == 'duplicate':
    time.sleep(0.1); print('GENOS_READY', flush=True)
time.sleep(2)
"#;
        for mode in ["good", "stale", "missing", "duplicate", "exit"] {
            let mut evidence = Evidence::new("host-child", None).unwrap();
            let path = evidence.directory.clone();
            let mut child = Command::new("python3");
            child
                .args(["-u", "-c", script])
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/validation.log"))
                .arg(mode);
            let policy = RunPolicy {
                budget: Duration::from_millis(if mode == "missing" { 150 } else { 2000 }),
                ..RunPolicy::boot(2, 0)
            };
            let result =
                evidence.run_live(&mut child, Contract::phase(&[], true), policy, |_| Ok(()));
            assert_eq!(result.is_ok(), mode == "good", "{mode}: {result:?}");
            let result = evidence.finish(result);
            assert_eq!(result.is_ok(), mode == "good");
            let manifest = fs::read_to_string(path.join("manifest.txt")).unwrap();
            assert!(manifest.starts_with(if mode == "good" {
                "status=passed\n"
            } else {
                "status=failed\n"
            }));
            if mode == "exit" {
                assert!(fs::read_to_string(path.join("qemu.log"))
                    .unwrap()
                    .contains("fixture launch failed"));
            }
            fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn validation_trace_and_visible_output_must_both_prove_the_current_round_trip() {
        let records = [
            "USER_CONSOLE_WRITE pid=33 text=/> echo GENOS_RUN_1234",
            "/> echo GENOS_RUN_1234",
            "USER_CONSOLE_WRITE pid=33 text=GENOS_RUN_1234",
            "GENOS_RUN_1234",
        ];
        for omitted in 0..records.len() {
            let mut proof = ValidationFreshness::new("1234").unwrap();
            proof.issue().unwrap();
            for (index, record) in records.iter().enumerate() {
                if index != omitted && proof.observe(record).is_err() {
                    break;
                }
            }
            assert!(!proof.complete());
        }
        for malformed in [
            "USER_CONSOLE_WRITE pid=0 text=/> echo GENOS_RUN_1234",
            "USER_CONSOLE_WRITE pid=33 text=/> echo GENOS_RUN_9999",
            "prefix GENOS_RUN_1234",
            "genos> echo GENOS_RUN_1234",
            "GENOS_RUN_1234",
        ] {
            let mut proof = ValidationFreshness::new("1234").unwrap();
            proof.issue().unwrap();
            assert!(proof.observe(malformed).is_err(), "{malformed}");
        }
        let mut reordered = ValidationFreshness::new("1234").unwrap();
        reordered.issue().unwrap();
        reordered.observe(records[0]).unwrap();
        assert!(reordered.observe(records[2]).is_err());
        for duplicate in 0..records.len() {
            let mut proof = ValidationFreshness::new("1234").unwrap();
            proof.issue().unwrap();
            for record in records.iter().take(duplicate + 1) {
                proof.observe(record).unwrap();
            }
            assert!(proof.observe(records[duplicate]).is_err());
        }
        let mut proof = ValidationFreshness::new("1234").unwrap();
        assert!(proof.observe(records[0]).is_err());
        proof.issue().unwrap();
        proof.observe(records[0]).unwrap();
        assert!(proof
            .observe("USER_CONSOLE_WRITE pid=34 text=GENOS_RUN_1234")
            .is_err());
    }

    #[test]
    fn recorded_validation_input_trace_is_not_mistaken_for_embedded_success() {
        let mut proof = ValidationFreshness::new("1790077679394907000").unwrap();
        proof.issue().unwrap();
        for line in include_str!("../tests/fixtures/validation-challenge-prefix.txt").lines() {
            proof.observe(line).unwrap();
        }
        assert!(!proof.complete());
        for line in [
            "/> echo GENOS_RUN_1790077679394907000",
            "USER_CONSOLE_WRITE pid=33 text=GENOS_RUN_1790077679394907000",
            "GENOS_RUN_1790077679394907000",
        ] {
            proof.observe(line).unwrap();
        }
        assert!(proof.complete());
    }

    #[test]
    fn serial_input_requires_the_issued_command_and_its_own_visible_reply() {
        let mut proof = UnameProof::default();
        proof
            .observe("USER_CONSOLE_WRITE pid=33 text=/> uname")
            .unwrap();
        proof
            .observe("USER_CONSOLE_WRITE pid=33 text=GenOS v0.56 ring3-shell x86_64 ABI 18")
            .unwrap();
        assert!(!proof.complete());
        proof.issued = true;
        assert!(proof
            .observe("GenOS v0.56 ring3-shell x86_64 ABI 18")
            .is_err());
        for line in [
            "genos> SERIAL_RX_OK",
            "USER_CONSOLE_WRITE pid=33 text=/> uname",
            "/> uname",
            "USER_CONSOLE_WRITE pid=33 text=GenOS v0.56 ring3-shell x86_64 ABI 18",
            "GenOS v0.56 ring3-shell x86_64 ABI 18",
        ] {
            proof.observe(line).unwrap();
        }
        assert!(proof.complete());
        assert!(proof
            .observe("GenOS v0.56 ring3-shell x86_64 ABI 18")
            .is_err());
    }
    #[test]
    fn failed_and_abandoned_runs_retain_distinct_manifests() {
        let first = Evidence::new("host-failure", None).unwrap();
        let first_path = first.directory.clone();
        assert!(first.finish(Err("fixture spawn failed".into())).is_err());
        let failed = fs::read_to_string(first_path.join("manifest.txt")).unwrap();
        assert!(failed.starts_with("status=failed\n"));
        assert!(failed.contains("fixture spawn failed"));
        let second = Evidence::new("host-failure", None).unwrap();
        let second_path = second.directory.clone();
        assert_ne!(first_path, second_path);
        drop(second);
        assert!(fs::read_to_string(second_path.join("manifest.txt"))
            .unwrap()
            .contains("abandoned"));
        assert_eq!(fs::metadata(first_path.join("qemu.log")).unwrap().len(), 0);
        fs::remove_dir_all(first_path).unwrap();
        fs::remove_dir_all(second_path).unwrap();
    }
    #[test]
    fn an_empty_evidence_directory_cannot_be_promoted() {
        let evidence = Evidence::new("host-empty", None).unwrap();
        let path = evidence.directory.clone();
        assert!(evidence.finish(Ok(())).is_err());
        assert!(fs::read_to_string(path.join("manifest.txt"))
            .unwrap()
            .starts_with("status=failed\n"));
        fs::remove_dir_all(path).unwrap();
    }
}
