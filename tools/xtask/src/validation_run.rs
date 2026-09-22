//! A live validation run owns its process, pipes and unique retained evidence.
//! Every exit path, including preflight/spawn failure, finalizes the manifest.
use crate::{evidence_contract::Contract, normal_boot::Freshness, reference_vm};
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
        let mut freshness = Freshness::new(&self.run_id)?;
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

#[derive(Default)]
struct UnameProof {
    issued: bool,
    echoed: bool,
    received: bool,
    replied: bool,
}
impl UnameProof {
    fn observe(&mut self, line: &str) -> Result<(), String> {
        match line {
            "SERIAL_RX_OK" | "genos> SERIAL_RX_OK" => {
                if !self.issued || self.received {
                    return Err("stale/duplicate serial receive proof".into());
                }
                self.received = true;
            }
            "genos> uname" => {
                if !self.issued || self.echoed {
                    return Err("stale/duplicate uname echo".into());
                }
                self.echoed = true;
            }
            "GenOS v0.56 ring3-shell x86_64 ABI 18" if self.issued => {
                if !self.echoed || self.replied {
                    return Err("wrong-phase/duplicate uname result".into());
                }
                self.replied = true;
            }
            _ => {}
        }
        Ok(())
    }
    fn complete(&self) -> bool {
        self.issued && self.echoed && self.received && self.replied
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
print('genos> ' + text, flush=True)
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
    fn serial_input_requires_the_issued_command_and_its_own_reply() {
        let mut proof = UnameProof::default();
        proof
            .observe("GenOS v0.56 ring3-shell x86_64 ABI 18")
            .unwrap();
        assert!(!proof.complete());
        assert!(proof.observe("genos> uname").is_err());
        proof.issued = true;
        assert!(proof
            .observe("GenOS v0.56 ring3-shell x86_64 ABI 18")
            .is_err());
        for line in [
            "genos> SERIAL_RX_OK",
            "genos> uname",
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
