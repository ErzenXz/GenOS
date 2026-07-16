use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const BUILD_DIR: &str = "build";
const IMAGE: &str = "build/genos.img";
const INITRD: &str = "build/INITRD.GRD";
const USER_INIT: &str = "target/x86_64-unknown-none/userspace/genos-init";

fn main() {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "build".to_string());
    let result = match command.as_str() {
        "build" => build(),
        "run" => run(),
        "test" => test(),
        "clean" => clean(),
        other => Err(format!("unknown xtask command: {other}")),
    };

    if let Err(error) = result {
        eprintln!("xtask: {error}");
        std::process::exit(1);
    }
}

fn build() -> Result<(), String> {
    fs::create_dir_all(BUILD_DIR).map_err(|e| e.to_string())?;
    cargo([
        "build",
        "-p",
        "bootloader",
        "--target",
        "x86_64-unknown-uefi",
    ])?;
    cargo([
        "build",
        "-p",
        "genos-init",
        "--profile",
        "userspace",
        "--target",
        "x86_64-unknown-none",
    ])?;
    cargo(["build", "-p", "kernel", "--target", "x86_64-unknown-none"])?;
    write_initrd(Path::new(INITRD))?;
    create_image()
}

fn run() -> Result<(), String> {
    build()?;
    let firmware = find_ovmf_code()?;
    let status = Command::new("qemu-system-x86_64")
        .arg("-machine")
        .arg("q35")
        .arg("-m")
        .arg("512M")
        .arg("-drive")
        .arg(format!(
            "if=pflash,format=raw,readonly=on,file={}",
            firmware.display()
        ))
        .arg("-drive")
        .arg(format!("format=raw,file={IMAGE}"))
        .arg("-vga")
        .arg("std")
        .arg("-serial")
        .arg("stdio")
        .arg("-no-reboot")
        .status()
        .map_err(|e| format!("failed to launch qemu: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("qemu exited with {status}"))
    }
}

fn test() -> Result<(), String> {
    cargo(["test", "-p", "genos_abi"])?;
    cargo(["test", "-p", "kernel", "--lib"])?;
    cargo(["test", "-p", "xtask"])?;
    build()?;
    smoke_qemu()
}

fn clean() -> Result<(), String> {
    if Path::new(BUILD_DIR).exists() {
        fs::remove_dir_all(BUILD_DIR).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn cargo<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new("cargo")
        .args(args)
        .status()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo exited with {status}"))
    }
}

fn write_initrd(path: &Path) -> Result<(), String> {
    let user_init =
        fs::read(USER_INIT).map_err(|error| format!("failed to read {USER_INIT}: {error}"))?;
    let files = vec![
        (
            "README.TXT",
            b"Welcome to GenOS.\nThis file lives in the V1 RAM disk.\n".to_vec(),
        ),
        ("USER.TXT", b"user.name=genos\nhome=/users/genos\n".to_vec()),
        (
            "NOTES.TXT",
            b"INIT.ELF is a separately built GenOS userspace executable.\n".to_vec(),
        ),
        ("INIT.ELF", user_init),
    ];

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GRD1");
    bytes.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, data) in files {
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&data);
    }
    fs::write(path, bytes).map_err(|e| e.to_string())
}

fn create_image() -> Result<(), String> {
    let bootloader = Path::new("target/x86_64-unknown-uefi/debug/bootloader.efi");
    let kernel = Path::new("target/x86_64-unknown-none/debug/kernel");
    if !bootloader.exists() {
        return Err(format!("missing {}", bootloader.display()));
    }
    if !kernel.exists() {
        return Err(format!("missing {}", kernel.display()));
    }

    let image = Path::new(IMAGE);
    let file = File::create(image).map_err(|e| e.to_string())?;
    file.set_len(64 * 1024 * 1024).map_err(|e| e.to_string())?;

    run_tool("mformat", ["-i", IMAGE, "-F", "::"])?;
    run_tool("mmd", ["-i", IMAGE, "::/EFI"])?;
    run_tool("mmd", ["-i", IMAGE, "::/EFI/BOOT"])?;
    run_tool("mmd", ["-i", IMAGE, "::/EFI/GENOS"])?;
    run_tool(
        "mcopy",
        [
            "-i",
            IMAGE,
            bootloader.to_str().ok_or("invalid bootloader path")?,
            "::/EFI/BOOT/BOOTX64.EFI",
        ],
    )?;
    run_tool(
        "mcopy",
        [
            "-i",
            IMAGE,
            kernel.to_str().ok_or("invalid kernel path")?,
            "::/EFI/GENOS/KERNEL.ELF",
        ],
    )?;
    run_tool("mcopy", ["-i", IMAGE, INITRD, "::/EFI/GENOS/INITRD.GRD"])?;
    Ok(())
}

fn run_tool<I, S>(program: &str, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("failed to run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

fn smoke_qemu() -> Result<(), String> {
    let firmware = find_ovmf_code()?;
    let serial_log = Path::new("build/serial.log");
    let _ = fs::remove_file(serial_log);

    let mut child = Command::new("qemu-system-x86_64")
        .arg("-machine")
        .arg("q35")
        .arg("-m")
        .arg("512M")
        .arg("-drive")
        .arg(format!(
            "if=pflash,format=raw,readonly=on,file={}",
            firmware.display()
        ))
        .arg("-drive")
        .arg(format!("format=raw,file={IMAGE}"))
        .arg("-vga")
        .arg("std")
        .arg("-display")
        .arg("none")
        .arg("-serial")
        .arg(format!("file:{}", serial_log.display()))
        .arg("-no-reboot")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to launch qemu smoke test: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(25);
    let mut output = String::new();
    let mut ready_at = None;
    while Instant::now() < deadline {
        output.clear();
        if let Ok(mut file) = File::open(serial_log) {
            let _ = file.read_to_string(&mut output);
            if output.contains("GENOS_READY") && ready_at.is_none() {
                ready_at = Some(Instant::now());
            }
            if smoke_markers_ready(&output)
                && ready_at
                    .map(|instant| instant.elapsed() >= Duration::from_secs(12))
                    .unwrap_or(false)
            {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(());
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "qemu exited early with {status}; serial:\n{output}"
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }

    let _ = child.kill();
    let _ = child.wait();
    Err(format!(
        "timed out waiting for long-lived GenOS smoke markers; serial:\n{output}"
    ))
}

fn smoke_markers_ready(output: &str) -> bool {
    [
        "IRQ_READY",
        "VFS_READY",
        "TASKS_READY",
        "SCHED_READY",
        "PAGING_READY",
        "ADDRESS_SPACES_READY",
        "USER_ELF_VALIDATED",
        "USER_ELF_LOADED",
        "USER_ELF_LAUNCH_OK",
        "USER_CONTEXT_OK",
        "USER_CONTEXT_RESUME_OK",
        "USER_PREEMPT_OK",
        "USER_FAULT_TERMINATED",
        "USER_FAULT_ISOLATED",
        "USER_SYSCALL_OK",
        "USER_COPY_OK",
        "USER_ISOLATION_OK",
        "USERMODE_READY",
        "BACKBUFFER_READY",
        "GENOS_READY",
        "IRQ_HARDWARE_ON",
        "IRQ_TICK_OK",
        "DISPLAY_IDLE_OK",
    ]
    .iter()
    .all(|marker| output.contains(marker))
}

fn find_ovmf_code() -> Result<PathBuf, String> {
    let candidates = [
        "/opt/homebrew/share/qemu/edk2-x86_64-code.fd",
        "/opt/homebrew/Cellar/qemu/10.2.2/share/qemu/edk2-x86_64-code.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/edk2/x64/OVMF_CODE.fd",
        "/usr/share/qemu/OVMF.fd",
    ];
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Ok(path);
        }
    }
    Err("could not find OVMF/EDK2 x86_64 firmware".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ovmf_search_has_known_names() {
        assert!(find_ovmf_code().is_ok() || cfg!(not(target_os = "macos")));
    }

    #[test]
    fn smoke_requires_elf_preemption_and_fault_markers() {
        assert!(smoke_markers_ready(
            "IRQ_READY\nVFS_READY\nTASKS_READY\nSCHED_READY\nPAGING_READY\nADDRESS_SPACES_READY\nUSER_ELF_VALIDATED\nUSER_ELF_LOADED\nUSER_ELF_LAUNCH_OK\nUSER_CONTEXT_OK\nUSER_CONTEXT_RESUME_OK\nUSER_PREEMPT_OK\nUSER_FAULT_TERMINATED\nUSER_FAULT_ISOLATED\nUSER_SYSCALL_OK\nUSER_COPY_OK\nUSER_ISOLATION_OK\nUSERMODE_READY\nBACKBUFFER_READY\nGENOS_READY\nIRQ_HARDWARE_ON\nIRQ_TICK_OK\nDISPLAY_IDLE_OK\n"
        ));
        assert!(!smoke_markers_ready("GENOS_READY\n"));
    }
}
