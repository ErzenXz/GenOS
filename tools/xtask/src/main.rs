use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

mod sdk;

const BUILD_DIR: &str = "build";
const IMAGE: &str = "build/genos.img";
const DATA_IMAGE: &str = "build/genos-data.img";
const TEST_DATA_IMAGE: &str = "build/genos-data-test.img";
const REPAIR_DATA_IMAGE: &str = "build/genos-data-repair-test.img";
const READ_ONLY_DATA_IMAGE: &str = "build/genos-data-read-only.img";
const FAILURE_DATA_IMAGE: &str = "build/genos-data-corrupt.img";
const INITRD: &str = "build/INITRD.GRD";
const USER_INIT: &str = "target/x86_64-unknown-none/userspace/genos-init";
const USER_SHELL: &str = "target/x86_64-unknown-none/userspace/genos-shell";
const DATA_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const PARTITION_START_LBA: usize = 64;
const PARTITION_TYPE_GENOS: u8 = 0x7f;
const PARTITION_TYPE_GENOS_READ_ONLY: u8 = 0x7e;
const SLOT_SECTORS: usize = 40;
const SLOT_BYTES: usize = SLOT_SECTORS * 512;
const SLOT_OFFSETS: [usize; 2] = [1, 1 + SLOT_SECTORS];
const SNAPSHOT_HEADER_BYTES: usize = 64;
const SNAPSHOT_CHECKSUM_OFFSET: usize = 20;
const MODERN_NETWORK_DEVICE: &str =
    "virtio-net-pci,disable-legacy=on,netdev=net0,mac=52:54:00:12:34:56";

fn main() {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "build".to_string());
    let result = match command.as_str() {
        "build" => build(),
        "run" => run(),
        "test" => test(),
        "test-network" => test_network(),
        "new-app" => match args.next() {
            Some(path) if args.next().is_none() => sdk::new_app(Path::new(&path)),
            _ => Err("usage: cargo xtask new-app PATH (a new directory)".to_string()),
        },
        "test-sdk" => test_sdk(),
        "test-memory" => test_memory(),
        "test-protections" => test_protections(),
        "bench" => benchmark(),
        "inspect-data" => inspect_data_command(),
        "repair-data" => repair_data_command(),
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
    cargo([
        "build",
        "-p",
        "genos-shell",
        "--profile",
        "userspace",
        "--target",
        "x86_64-unknown-none",
    ])?;
    cargo(["build", "-p", "kernel", "--target", "x86_64-unknown-none"])?;
    write_initrd(Path::new(INITRD))?;
    create_image()?;
    ensure_data_image()
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
        .arg("-device")
        .arg("piix3-ide,id=genos-storage")
        .arg("-drive")
        .arg(format!(
            "if=none,id=genos-data,format=raw,cache=writeback,file={DATA_IMAGE}"
        ))
        .arg("-device")
        .arg("ide-hd,drive=genos-data,bus=genos-storage.0,unit=0")
        .arg("-netdev")
        .arg("user,id=net0")
        .arg("-device")
        .arg(MODERN_NETWORK_DEVICE)
        .arg("-display")
        .arg("none")
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
    ensure_test_data_image(true)?;
    smoke_qemu()?;
    test_sdk()?;
    test_memory()?;
    test_protections()
}

fn test_protections() -> Result<(), String> {
    // Existing build-and-boot CI runs this entry point and retains serial*.log.
    // Expand the suite here without changing workflow definitions or grants.
    let mut manifests = String::new();
    for (mode, fault) in [
        ("user", "nx-data"),
        ("user", "nx-stack"),
        ("kernel", "wp"),
        ("kernel", "kernel-text"),
        ("kernel", "smep"),
        ("kernel", "smap"),
    ] {
        let run_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
            .to_string();
        let status = Command::new("python3")
            .args([
                "tools/test_exception_entry.py",
                "--mode",
                mode,
                "--fault",
                fault,
                "--run-id",
                &run_id,
            ])
            .status()
            .map_err(|e| format!("failed to run protection probe: {e}"))?;
        let evidence = PathBuf::from(format!("build/exception-evidence/{mode}-{fault}/{run_id}"));
        // Use the exact invocation's evidence, never a previous successful
        // run's files if this invocation failed before creating its directory.
        let copied = fs::copy(
            evidence.join("serial.log"),
            format!("build/serial-protection-{mode}-{fault}.log"),
        );
        if status.success() {
            copied.map_err(|e| format!("missing protection serial evidence: {e}"))?;
        }
        if let Ok(manifest) = fs::read_to_string(evidence.join("manifest.json")) {
            manifests.push_str(&manifest);
            manifests.push('\n');
        } else if status.success() {
            return Err("missing protection manifest".to_string());
        }
        fs::write("build/serial-protection-manifests.log", &manifests)
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("protection probe failed: {mode}/{fault}"));
        }
    }
    println!("all six exact-address CPU protection probes passed");
    Ok(())
}

fn test_memory() -> Result<(), String> {
    build()?;
    ensure_test_data_image(false)?;
    let result = (|| {
        cargo([
            "build",
            "-p",
            "kernel",
            "--features",
            "memory-test-faults",
            "--target",
            "x86_64-unknown-none",
        ])?;
        write_initrd(Path::new(INITRD))?;
        create_image()?;
        smoke_qemu_phase(
            Path::new("build/serial-memory.log"),
            Path::new(TEST_DATA_IMAGE),
            false,
            &[
                "FRAME_ALLOCATOR_BITMAP_READY",
                "MEMORY_ROLLBACK_READY allocation_points=10 leaked_frames=0",
                "USER_SHELL_READY",
                "GENOS_READY",
            ],
            false,
        )
    })();
    restore_production_after(result)?;
    println!("physical allocation rollback passed at every process construction allocation");
    Ok(())
}

fn test_sdk() -> Result<(), String> {
    let (workspace, elf) = sdk::build_external_example()?;
    build()?;
    ensure_test_data_image(false)?;
    let result = (|| {
        write_initrd_with_sdk(Path::new(INITRD), Some(elf))?;
        create_image()?;
        // This explicitly packaged test application runs only when SDK.ELF is
        // present. Keep a normal production artifact even when the gate fails.
        smoke_qemu_phase(
            Path::new("build/serial-sdk.log"),
            Path::new(TEST_DATA_IMAGE),
            false,
            &[
                "SDK_APPLICATION_READY abi=18 exit=0 reclaimed=true",
                "USER_SHELL_READY",
                "GENOS_READY",
            ],
            false,
        )
    })();
    restore_production_after(result)?;
    println!(
        "standalone SDK build and Ring 3 boot passed; source: {}",
        workspace.display()
    );
    Ok(())
}

fn benchmark() -> Result<(), String> {
    build()?;
    let mut samples = Vec::new();
    let data_image = Path::new("build/genos-data-benchmark.img");
    let mut evidence = String::new();
    for sample in 0..3 {
        write_partitioned_image(data_image, false)?;
        let log = PathBuf::from(format!("build/serial-benchmark-{sample}.log"));
        let started = Instant::now();
        smoke_qemu_phase(
            &log,
            data_image,
            false,
            &[
                "SERVER_TERMINAL_READY",
                "GENOS_READY",
                "SCHED_DISPATCH_BENCH_OK",
                "SCHED_CONTEXT_BENCH_OK",
            ],
            false,
        )?;
        samples.push(started.elapsed().as_millis());
        let serial = fs::read_to_string(log).map_err(|e| e.to_string())?;
        for line in serial.lines().filter(|line| {
            line.starts_with("SCHED_DISPATCH_BENCH ") || line.starts_with("SCHED_CONTEXT_BENCH ")
        }) {
            evidence.push_str(&format!("sample {sample}: {line}\n"));
        }
    }
    samples.sort_unstable();
    let version = |program: &str| -> String {
        Command::new(program)
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .unwrap_or("unknown")
                    .to_string()
            })
            .unwrap_or_else(|| "unknown".to_string())
    };
    let mut sizes = String::new();
    for path in [
        "target/x86_64-unknown-none/debug/kernel",
        "target/x86_64-unknown-uefi/debug/bootloader.efi",
        USER_INIT,
        USER_SHELL,
        IMAGE,
    ] {
        let bytes = fs::metadata(path).map_err(|e| e.to_string())?.len();
        sizes.push_str(&format!("| `{path}` | {bytes} |\n"));
    }
    let report = format!("# GenOS development benchmark\n\nHost: {} / {}. {}. {}.\n\nThree separate QEMU q35 boots, 512 MiB RAM, no NIC, fresh disposable data disk each run, headless serial terminal. The normal user disk is not used. Development kernel; optimized userspace.\n\n| Metric | Milliseconds |\n|---|---:|\n| Fastest boot | {} |\n| Median boot | {} |\n| Slowest boot | {} |\n\nTiming starts before QEMU launch and ends after its serial readiness markers and process teardown. It includes firmware, kernel acceptance probes, fresh-volume initialization, and up to 100 ms harness polling. These are end-to-end development-boot observations, not kernel-only latency or comparative speed claims.\n\n| Artifact | Bytes on disk |\n|---|---:|\n{}\nThe disk image is sparse; apparent size is shown, not resident memory or allocated disk blocks.\n\n```text\n{}```\n\nReproduce with `cargo xtask bench`. Raw logs: `build/serial-benchmark-0.log` through `build/serial-benchmark-2.log`.\n",
        std::env::consts::OS, std::env::consts::ARCH, version("qemu-system-x86_64"), version("rustc"),
        samples[0], samples[1], samples[2], sizes, evidence);
    fs::write("build/benchmarks.md", report).map_err(|e| e.to_string())?;
    println!(
        "benchmark report: build/benchmarks.md (median {} ms)",
        samples[1]
    );
    Ok(())
}

fn restore_production_after(result: Result<(), String>) -> Result<(), String> {
    let restored = build().and_then(|()| verify_production_fault_hooks_absent());
    match (result, restored) {
        (Err(test), Err(restore)) => Err(format!(
            "{test}\nRestoring the production image also failed: {restore}"
        )),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn test_network() -> Result<(), String> {
    let result = (|| {
        build_network_test_image()?;
        ensure_test_data_image(false)?;
        smoke_network_qemu()
    })();
    restore_production_after(result)?;
    smoke_network_without_http_server()
}

fn build_network_test_image() -> Result<(), String> {
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
    cargo([
        "build",
        "-p",
        "genos-shell",
        "--profile",
        "userspace",
        "--target",
        "x86_64-unknown-none",
    ])?;
    cargo([
        "build",
        "-p",
        "kernel",
        "--features",
        "network-test-faults",
        "--target",
        "x86_64-unknown-none",
    ])?;
    write_initrd(Path::new(INITRD))?;
    create_image()?;
    ensure_data_image()
}

fn verify_production_fault_hooks_absent() -> Result<(), String> {
    let kernel = fs::read("target/x86_64-unknown-none/debug/kernel")
        .map_err(|error| format!("failed to inspect production kernel: {error}"))?;
    for forbidden in [
        b"TCP_FAULT_DATA_DROP_INJECTED".as_slice(),
        b"MEMORY_ROLLBACK_READY".as_slice(),
        b"TCP_FAULT_REORDER_HELD".as_slice(),
        b"TCP_FAULT_REORDER_RELEASED".as_slice(),
    ] {
        if kernel
            .windows(forbidden.len())
            .any(|window| window == forbidden)
        {
            return Err("production kernel contains a network fault-injection hook".to_string());
        }
    }
    println!("production kernel excludes network fault-injection hooks");
    Ok(())
}

fn inspect_data_command() -> Result<(), String> {
    let report = inspect_filesystem_image(Path::new(DATA_IMAGE))?;
    println!(
        "GenOS data image: {} valid slot(s), {} invalid slot(s)",
        report.valid_slots.len(),
        report.invalid_slots.len()
    );
    for slot in report.valid_slots {
        println!(
            "slot={} generation={} files={}",
            slot.slot,
            slot.generation,
            slot.entries.len()
        );
        for entry in slot.entries {
            println!(
                "  {} kind={} bytes={}",
                entry.path,
                if entry.directory { "dir" } else { "file" },
                entry.data.len()
            );
        }
    }
    Ok(())
}

fn repair_data_command() -> Result<(), String> {
    match repair_filesystem_image(Path::new(DATA_IMAGE))? {
        RepairOutcome::Repaired {
            source_slot,
            target_slot,
            generation,
        } => println!(
            "GenOS data image repaired: source_slot={source_slot} target_slot={target_slot} generation={generation}"
        ),
        RepairOutcome::Healthy => {
            println!("GenOS data image is healthy: both snapshots are valid")
        }
    }
    Ok(())
}

fn clean() -> Result<(), String> {
    clean_generated_files(Path::new(BUILD_DIR))
}

fn clean_generated_files(directory: &Path) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name() == "genos-data.img" {
            continue;
        }
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        if file_type.is_dir() {
            fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
        } else {
            fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
        }
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
    write_initrd_with_sdk(path, None)
}

fn write_initrd_with_sdk(path: &Path, sdk_elf: Option<Vec<u8>>) -> Result<(), String> {
    let user_init =
        fs::read(USER_INIT).map_err(|error| format!("failed to read {USER_INIT}: {error}"))?;
    let user_shell =
        fs::read(USER_SHELL).map_err(|error| format!("failed to read {USER_SHELL}: {error}"))?;
    let mut files = vec![
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
        ("SHELL.ELF", user_shell),
    ];
    if let Some(elf) = sdk_elf {
        files.push(("SDK.ELF", elf));
    }

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

fn ensure_data_image() -> Result<(), String> {
    ensure_partitioned_image(Path::new(DATA_IMAGE))
}

fn ensure_test_data_image(reset: bool) -> Result<(), String> {
    let path = Path::new(TEST_DATA_IMAGE);
    if reset {
        write_partitioned_image(path, false)?;
    }
    ensure_partitioned_image(path)
}

fn ensure_partitioned_image(path: &Path) -> Result<(), String> {
    match fs::read(path) {
        Ok(bytes) => valid_genos_partition(&bytes).map(|_| ()).map_err(|error| {
            format!(
                "{}: {error}; preserving existing data image for explicit recovery",
                path.display()
            )
        }),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && fs::symlink_metadata(path).is_err() =>
        {
            write_partitioned_image(path, false)
        }
        Err(error) => Err(format!(
            "cannot read {}: {error}; existing image was not replaced",
            path.display()
        )),
    }
}

fn write_partitioned_image(path: &Path, corrupt_slots: bool) -> Result<(), String> {
    let mut bytes = vec![0u8; DATA_IMAGE_BYTES];
    let sectors = DATA_IMAGE_BYTES / 512 - PARTITION_START_LBA;
    let entry = 446;
    bytes[entry + 4] = PARTITION_TYPE_GENOS;
    bytes[entry + 8..entry + 12].copy_from_slice(&(PARTITION_START_LBA as u32).to_le_bytes());
    bytes[entry + 12..entry + 16].copy_from_slice(&(sectors as u32).to_le_bytes());
    bytes[510] = 0x55;
    bytes[511] = 0xaa;
    if corrupt_slots {
        let partition = PARTITION_START_LBA * 512;
        for offset in SLOT_OFFSETS {
            bytes[partition + offset * 512..partition + offset * 512 + SLOT_BYTES].fill(0x5a);
        }
    }
    fs::write(path, bytes).map_err(|error| error.to_string())
}

fn valid_genos_partition(bytes: &[u8]) -> Result<(usize, usize), String> {
    if bytes.len() < 512 || bytes[510..512] != [0x55, 0xaa] {
        return Err("missing MBR signature".to_string());
    }
    for index in 0..4 {
        let offset = 446 + index * 16;
        if !matches!(
            bytes[offset + 4],
            PARTITION_TYPE_GENOS | PARTITION_TYPE_GENOS_READ_ONLY
        ) {
            continue;
        }
        let start = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
        let sectors =
            u32::from_le_bytes(bytes[offset + 12..offset + 16].try_into().unwrap()) as usize;
        if start > 0
            && sectors > SLOT_SECTORS * 2
            && start
                .checked_add(sectors)
                .is_some_and(|end| end <= bytes.len() / 512)
        {
            return Ok((start, sectors));
        }
    }
    Err("GenOS MBR partition not found or out of bounds".to_string())
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
    smoke_qemu_phase(
        Path::new("build/serial-persistence-create.log"),
        Path::new(TEST_DATA_IMAGE),
        false,
        &[
            "PARTITION_DISCOVERED",
            "BLOCK_CACHE_READY",
            "BLOCK_CACHE_HIT_OK",
            "PERSISTENT_STORAGE_CREATED",
            "USER_DURABLE_WRITE_OK",
            "USER_SHELL_READY",
        ],
        false,
    )?;
    inspect_persistent_image(false)?;
    smoke_qemu_phase(
        Path::new("build/serial.log"),
        Path::new(TEST_DATA_IMAGE),
        false,
        &["PERSISTENT_STORAGE_RESTORED", "USER_DURABLE_RESTORE_OK"],
        true,
    )?;
    smoke_serial_terminal_input()?;

    create_read_only_recovery_image()?;
    smoke_qemu_phase(
        Path::new("build/serial-storage-read-only.log"),
        Path::new(READ_ONLY_DATA_IMAGE),
        false,
        &[
            "PARTITION_DISCOVERED scheme=mbr type=0x7e",
            "PERSISTENT_STORAGE_READ_ONLY",
            "USER_STORAGE_READ_ONLY_OK",
            "USER_READ_ONLY_MUTATION_DENIED_OK",
            "USER_DURABLE_RESTORE_OK",
            "USER_RAMFS_TEMP_APP_OK",
            "USER_SHELL_READY",
        ],
        false,
    )?;

    simulate_torn_write(Path::new(TEST_DATA_IMAGE))?;
    inspect_persistent_image(true)?;
    fs::copy(TEST_DATA_IMAGE, REPAIR_DATA_IMAGE)
        .map_err(|error| format!("failed to create repair test image: {error}"))?;
    match repair_filesystem_image(Path::new(REPAIR_DATA_IMAGE))? {
        RepairOutcome::Repaired { .. } => {}
        RepairOutcome::Healthy => {
            return Err("torn repair test unexpectedly found a healthy image".to_string());
        }
    }
    inspect_persistent_image_at(Path::new(REPAIR_DATA_IMAGE), false)?;
    smoke_qemu_phase(
        Path::new("build/serial-storage-recovery.log"),
        Path::new(TEST_DATA_IMAGE),
        false,
        &[
            "PERSISTENT_STORAGE_RECOVERED_TORN_WRITE",
            "CRASH_SAFE_STORAGE_READY",
            "USER_STORAGE_STATUS_VISIBLE_OK",
            "USER_RAMFS_TEMP_APP_OK",
            "USER_DURABLE_RESTORE_OK",
            "USER_SHELL_READY",
        ],
        false,
    )?;
    inspect_persistent_image(false)?;

    ensure_failure_image()?;
    smoke_qemu_phase(
        Path::new("build/serial-storage-failure.log"),
        Path::new(FAILURE_DATA_IMAGE),
        false,
        &[
            "PERSISTENT_STORAGE_UNAVAILABLE",
            "USER_STORAGE_FAILURE_VISIBLE_OK",
            "STORAGE_FAILURE_SURFACE_READY",
            "USER_RAMFS_TEMP_APP_OK",
            "USER_SESSION_WRITE_OK",
            "USER_SHELL_READY",
        ],
        false,
    )?;
    test_network()
}

fn smoke_qemu_phase(
    serial_log: &Path,
    data_image: &Path,
    read_only: bool,
    required_markers: &[&str],
    require_full_smoke: bool,
) -> Result<(), String> {
    let firmware = find_ovmf_code()?;
    let _ = fs::remove_file(serial_log);

    let mut data_drive = format!(
        "if=none,id=genos-data,format=raw,cache=writeback,file={}",
        data_image.display()
    );
    if read_only {
        data_drive.push_str(",readonly=on");
    }

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
        .arg("-device")
        .arg("piix3-ide,id=genos-storage")
        .arg("-drive")
        .arg(data_drive)
        .arg("-device")
        .arg("ide-hd,drive=genos-data,bus=genos-storage.0,unit=0")
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

    let deadline = Instant::now() + Duration::from_secs(if require_full_smoke { 60 } else { 20 });
    let mut output = String::new();
    let mut ready_at = None;
    while Instant::now() < deadline {
        output.clear();
        if let Ok(mut file) = File::open(serial_log) {
            let _ = file.read_to_string(&mut output);
            if output.contains("GENOS_READY") && ready_at.is_none() {
                ready_at = Some(Instant::now());
            }
            if output.contains("KERNEL PANIC") || output.contains("_FAILED") {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("GenOS reported a boot failure; serial:\n{output}"));
            }
            let phase_ready = required_markers
                .iter()
                .all(|marker| output.contains(marker));
            if phase_ready && !require_full_smoke {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(());
            }
            if phase_ready
                && smoke_markers_ready(&output)
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

fn smoke_serial_terminal_input() -> Result<(), String> {
    let firmware = find_ovmf_code()?;
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
        .arg("-device")
        .arg("piix3-ide,id=genos-storage")
        .arg("-drive")
        .arg(format!(
            "if=none,id=genos-data,format=raw,cache=writeback,file={TEST_DATA_IMAGE}"
        ))
        .arg("-device")
        .arg("ide-hd,drive=genos-data,bus=genos-storage.0,unit=0")
        .arg("-display")
        .arg("none")
        .arg("-monitor")
        .arg("none")
        .arg("-serial")
        .arg("stdio")
        .arg("-no-reboot")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to launch serial input smoke test: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("serial stdout pipe unavailable")?;
    let mut stdin = child.stdin.take().ok_or("serial stdin pipe unavailable")?;
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut output = String::new();
    let mut command_sent = false;
    let mut passed = false;
    while Instant::now() < deadline {
        if let Ok(line) = receiver.recv_timeout(Duration::from_millis(200)) {
            output.push_str(&line);
            output.push('\n');
            if (output.contains("KERNEL PANIC") || output.contains("_FAILED")) && !passed {
                break;
            }
            if output.contains("GENOS_READY") && !command_sent {
                stdin
                    .write_all(b"uname\r")
                    .and_then(|_| stdin.flush())
                    .map_err(|error| format!("failed to write serial command: {error}"))?;
                command_sent = true;
                output.clear();
            }
            if command_sent
                && output.contains("SERIAL_RX_OK")
                && output.contains("GenOS v0.56 ring3-shell x86_64 ABI 18")
            {
                passed = true;
                break;
            }
        }
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    drop(stdin);
    let _ = reader.join();
    if passed {
        println!("serial terminal input smoke passed: host command reached Ring 3");
        Ok(())
    } else {
        Err(format!(
            "serial terminal input smoke failed; transcript:\n{output}"
        ))
    }
}

fn smoke_network_qemu() -> Result<(), String> {
    let inbound_reservation = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to reserve inbound TCP probe port: {error}"))?;
    let inbound_host_port = inbound_reservation
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    drop(inbound_reservation);
    let listener = TcpListener::bind("0.0.0.0:18080")
        .map_err(|error| format!("failed to bind network smoke server: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let (server_sender, server_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut accepted = 0usize;
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut request = [0u8; 512];
                    let mut read = 0usize;
                    while read < request.len() {
                        match stream.read(&mut request[read..]) {
                            Ok(0) => break,
                            Ok(bytes) => {
                                read += bytes;
                                if request[..read].windows(4).any(|end| end == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nGENOS_OK";
                    let _ = stream.write_all(response);
                    let _ = stream.flush();
                    accepted += 1;
                    if accepted == 2 {
                        // The guest's independent HTTP markers prove that both
                        // responses arrived. Do not fail this host helper when
                        // the bounded guest closes immediately after reading
                        // and the host observes that close during flush.
                        let _ = server_sender.send(true);
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
        let _ = server_sender.send(false);
    });

    let firmware = find_ovmf_code()?;
    let serial_log = Path::new("build/serial-network.log");
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
        .arg("-device")
        .arg("piix3-ide,id=genos-storage")
        .arg("-drive")
        .arg(format!(
            "if=none,id=genos-data,format=raw,cache=writeback,file={TEST_DATA_IMAGE}"
        ))
        .arg("-device")
        .arg("ide-hd,drive=genos-data,bus=genos-storage.0,unit=0")
        .arg("-netdev")
        .arg(format!(
            "user,id=net0,hostfwd=tcp:127.0.0.1:{inbound_host_port}-:18081"
        ))
        .arg("-device")
        .arg(MODERN_NETWORK_DEVICE)
        .arg("-display")
        .arg("none")
        .arg("-serial")
        .arg(format!("file:{}", serial_log.display()))
        .arg("-no-reboot")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to launch network QEMU smoke test: {error}"))?;
    let (inbound_trigger_sender, inbound_trigger_receiver) = mpsc::channel();
    let (inbound_sender, inbound_receiver) = mpsc::channel();
    let inbound_client = thread::spawn(move || {
        if inbound_trigger_receiver
            .recv_timeout(Duration::from_secs(45))
            .is_err()
        {
            let _ = inbound_sender.send(false);
            return;
        }
        let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, inbound_host_port));
        // Two concurrent clients: both must be connected before either sends,
        // so the guest serves two established passive streams at once.
        let connect_barrier = Arc::new(Barrier::new(2));
        let send_barrier = Arc::new(Barrier::new(2));
        let mut workers = Vec::new();
        for scenario in 0..2 {
            let connect_barrier = Arc::clone(&connect_barrier);
            let send_barrier = Arc::clone(&send_barrier);
            workers.push(thread::spawn(move || -> bool {
                let exchanges = if scenario == 0 {
                    (0..3)
                        .map(|index| {
                            (
                                format!("GENOS_PING_LOSS_{index}").into_bytes(),
                                format!("GENOS_PONG_LOSS_{index}").into_bytes(),
                            )
                        })
                        .collect::<Vec<_>>()
                } else {
                    let mut first = vec![b'a'; 128];
                    let mut second = vec![b'b'; 128];
                    first[..b"GENOS_PING_REORDER_A".len()].copy_from_slice(b"GENOS_PING_REORDER_A");
                    second[..b"GENOS_PING_REORDER_B".len()]
                        .copy_from_slice(b"GENOS_PING_REORDER_B");
                    let mut expected_first = first.clone();
                    let mut expected_second = second.clone();
                    expected_first[..b"GENOS_PONG_REORDER_A".len()]
                        .copy_from_slice(b"GENOS_PONG_REORDER_A");
                    expected_second[..b"GENOS_PONG_REORDER_B".len()]
                        .copy_from_slice(b"GENOS_PONG_REORDER_B");
                    first.extend_from_slice(&second);
                    expected_first.extend_from_slice(&expected_second);
                    let mut exchanges = vec![(first, expected_first)];
                    for index in 2..8 {
                        let mut request = vec![b'a' + index as u8; 128];
                        let prefix = format!("GENOS_PING_STREAM_{index}");
                        request[..prefix.len()].copy_from_slice(prefix.as_bytes());
                        let mut response = request.clone();
                        let prefix = format!("GENOS_PONG_STREAM_{index}");
                        response[..prefix.len()].copy_from_slice(prefix.as_bytes());
                        exchanges.push((request, response));
                    }
                    exchanges
                };
                connect_barrier.wait();
                let deadline = Instant::now() + Duration::from_secs(20);
                let mut first_attempt = true;
                while Instant::now() < deadline {
                    let connection =
                        TcpStream::connect_timeout(&address, Duration::from_millis(200));
                    let Ok(mut stream) = connection else {
                        if first_attempt {
                            send_barrier.wait();
                            first_attempt = false;
                        }
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.set_nodelay(true);
                    if first_attempt {
                        send_barrier.wait();
                        first_attempt = false;
                    }
                    // Give Ring 3 time to claim both completed handshakes. This
                    // keeps the fault proof on accepted streams rather than the
                    // separate one-segment early-handshake buffer.
                    thread::sleep(Duration::from_millis(250));
                    // Half-close only after the response arrives: slirp aborts
                    // a forwarded connection whose host side reaches EOF while
                    // the guest-side connection is still being established.
                    let mut exchanged = true;
                    for (request, expected) in &exchanges {
                        if stream.write_all(request).is_err() || stream.flush().is_err() {
                            exchanged = false;
                            break;
                        }
                        let mut response = vec![0u8; expected.len()];
                        if stream.read_exact(&mut response).is_err() || response != *expected {
                            exchanged = false;
                            break;
                        }
                    }
                    if !exchanged {
                        thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                    let half_closed = stream.shutdown(Shutdown::Write).is_ok();
                    let mut trailing = [0u8; 1];
                    let closed = matches!(stream.read(&mut trailing), Ok(0));
                    if half_closed && closed {
                        return true;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                false
            }));
        }
        let all_ok = workers
            .into_iter()
            .all(|worker| worker.join().unwrap_or(false));
        let _ = inbound_sender.send(all_ok);
    });
    let required = [
        "NETWORK_DEVICE_READY driver=virtio-net-pci transport=modern-pci",
        "VIRTIO_NET_MSIX_READY vector=48 queues=rx,tx",
        "PACKET_OWNERSHIP_READY",
        "NETWORK_DHCP_READY",
        "ETHERNET_ARP_IPV4_UDP_READY",
        "NETWORK_ICMP_ECHO_OK",
        "IPV6_SLAAC_READY prefix=ra dad=passed",
        "IPV6_ICMP_ECHO_OK",
        "USER_DNS_RESOLVE_OK",
        "USER_HTTP_REQUEST_OK",
        "USER_SOCKET_API_READY",
        "USER_SOCKET_CAPABILITY_READY abi=18",
        "USER_SOCKET_LISTENER_CAPABILITY_READY abi=18",
        "USER_SOCKET_PASSIVE_LISTEN_READY port=18081",
        "USER_SOCKET_PASSIVE_LISTENER_READY",
        "TCP_PASSIVE_SYN_ACCEPTED",
        "TCP_PASSIVE_HANDSHAKE_OK",
        "USER_SOCKET_PASSIVE_ACCEPT_READY",
        "TCP_PASSIVE_STREAM_RX_OK",
        "TCP_PASSIVE_STREAM_TX_OK",
        "TCP_PASSIVE_STREAM_PEER_FIN_OK",
        "TCP_PASSIVE_STREAM_FIN_OK",
        "USER_SOCKET_PASSIVE_STREAM_READY",
        "USER_SOCKET_PASSIVE_CONCURRENT_READY streams=2",
        "TCP_FAULT_DATA_DROP_INJECTED",
        "TCP_FAULT_REORDER_HELD",
        "TCP_FAULT_REORDER_RELEASED",
        "TCP_STREAM_LARGE_READY bytes=1024 bounded_window=256",
        "TCP_CONGESTION_CONTROL_READY algorithm=aimd max_cwnd=1024",
        "USER_SOCKET_READINESS_WAIT_READY abi=18 wake_budget=2",
        "USER_SOCKET_WAIT_BLOCK",
        "USER_SOCKET_WAIT_WAKE",
        "USER_SOCKET_WAIT_IMMEDIATE",
        "USER_SOCKET_WAIT_TIMEOUT",
        "NETWORK_REGRESSION_BUDGET_OK",
        "USER_SOCKET_TRANSPORT_STARTED protocol=udp",
        "USER_SOCKET_TRANSPORT_COMPLETE protocol=udp",
        "USER_SOCKET_UDP_ASYNC_READY",
        "USER_SOCKET_UDP_TIMEOUT",
        "USER_SOCKET_STALE_REQUEST_DROPPED",
        "USER_SOCKET_TRANSPORT_STARTED protocol=tcp",
        "USER_SOCKET_TRANSPORT_COMPLETE protocol=tcp",
        "USER_SOCKET_TCP_ASYNC_READY",
        "TCP_ASYNC_RESET",
        "USER_SOCKET_STALE_REQUEST_DROPPED protocol=tcp",
        "USER_NETWORK_TIMEOUT_OK",
        "USER_NETWORK_DIAGNOSTICS_READY",
        "USER_SHELL_READY",
        "GENOS_READY",
    ];
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut output = String::new();
    let mut passed = false;
    let mut inbound_triggered = false;
    while Instant::now() < deadline {
        output.clear();
        if let Ok(mut file) = File::open(serial_log) {
            let _ = file.read_to_string(&mut output);
            if output.contains("KERNEL PANIC") || output.contains("_FAILED") {
                break;
            }
            if !inbound_triggered && output.contains("USER_SOCKET_PASSIVE_LISTENER_READY") {
                let _ = inbound_trigger_sender.send(());
                inbound_triggered = true;
            }
            if required.iter().all(|marker| output.contains(marker)) {
                passed = true;
                break;
            }
        }
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    let server_ok = server_receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or(false);
    let inbound_ok = inbound_receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or(false);
    let _ = server.join();
    let _ = inbound_client.join();
    if passed && server_ok && inbound_ok {
        println!(
            "network smoke passed: DHCP, ICMP, DNS, TCP/HTTP, concurrent passive streams, and timeout policy"
        );
        Ok(())
    } else {
        Err(format!(
            "network smoke failed (markers={passed} http_server={server_ok} inbound_clients={inbound_ok}); serial:\n{output}"
        ))
    }
}

fn smoke_network_without_http_server() -> Result<(), String> {
    let firmware = find_ovmf_code()?;
    let serial_log = Path::new("build/serial-network-normal-run.log");
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
        .arg("-device")
        .arg("piix3-ide,id=genos-storage")
        .arg("-drive")
        .arg(format!(
            "if=none,id=genos-data,format=raw,cache=writeback,file={TEST_DATA_IMAGE}"
        ))
        .arg("-device")
        .arg("ide-hd,drive=genos-data,bus=genos-storage.0,unit=0")
        .arg("-netdev")
        .arg("user,id=net0")
        .arg("-device")
        .arg(MODERN_NETWORK_DEVICE)
        .arg("-display")
        .arg("none")
        .arg("-serial")
        .arg(format!("file:{}", serial_log.display()))
        .arg("-no-reboot")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to launch normal network boot test: {error}"))?;
    let required = [
        "NETWORK_DEVICE_READY driver=virtio-net-pci transport=modern-pci",
        "NETWORK_DHCP_READY",
        "IPV6_SLAAC_READY prefix=ra dad=passed",
        "IPV6_ICMP_ECHO_OK",
        "USER_DNS_RESOLVE_OK",
        "USER_SOCKET_CAPABILITY_READY abi=18",
        "USER_SOCKET_LISTENER_CAPABILITY_READY abi=18",
        "USER_SOCKET_TRANSPORT_STARTED protocol=udp",
        "USER_SOCKET_TRANSPORT_COMPLETE protocol=udp",
        "USER_SOCKET_UDP_ASYNC_READY",
        "USER_SOCKET_UDP_TIMEOUT",
        "USER_SOCKET_STALE_REQUEST_DROPPED",
        "USER_SOCKET_TRANSPORT_STARTED protocol=tcp",
        "TCP_ASYNC_RESET",
        "USER_SOCKET_TCP_ERROR",
        "USER_SOCKET_STALE_REQUEST_DROPPED protocol=tcp",
        "USER_SHELL_READY",
        "GENOS_READY",
    ];
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut output = String::new();
    let mut passed = false;
    while Instant::now() < deadline {
        output.clear();
        if let Ok(mut file) = File::open(serial_log) {
            let _ = file.read_to_string(&mut output);
            if output.contains("KERNEL PANIC") || output.contains("_FAILED") {
                break;
            }
            if required.iter().all(|marker| output.contains(marker)) {
                passed = true;
                break;
            }
        }
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    if passed
        && !output.contains("USER_HTTP_REQUEST_OK")
        && !output.contains("USER_SOCKET_TCP_ASYNC_READY")
        && !output.contains("USER_SOCKET_PASSIVE_ACCEPT_READY")
        && !output.contains("USER_SOCKET_PASSIVE_STREAM_READY")
        && !output.contains("USER_SOCKET_PASSIVE_CONCURRENT_READY")
    {
        println!("normal network boot passed without the test-only HTTP server");
        Ok(())
    } else {
        Err(format!(
            "normal network boot did not degrade safely; serial:\n{output}"
        ))
    }
}

#[derive(Debug, Eq, PartialEq)]
struct InspectedEntry {
    path: String,
    directory: bool,
    data: Vec<u8>,
}

#[derive(Debug, Eq, PartialEq)]
struct InspectedSlot {
    slot: usize,
    generation: u64,
    entries: Vec<InspectedEntry>,
}

#[derive(Debug, Eq, PartialEq)]
struct ImageReport {
    valid_slots: Vec<InspectedSlot>,
    invalid_slots: Vec<usize>,
}

fn inspect_persistent_image(expect_torn_slot: bool) -> Result<(), String> {
    inspect_persistent_image_at(Path::new(TEST_DATA_IMAGE), expect_torn_slot)
}

fn inspect_persistent_image_at(path: &Path, expect_torn_slot: bool) -> Result<(), String> {
    let report = inspect_filesystem_image(path)?;
    if report.valid_slots.is_empty()
        || (expect_torn_slot && report.valid_slots.len() != 1)
        || report.invalid_slots.len() != usize::from(expect_torn_slot)
    {
        return Err(format!("unexpected GenOS slot state: {report:?}"));
    }
    let slot = report
        .valid_slots
        .iter()
        .max_by_key(|slot| slot.generation)
        .ok_or("GenOS image has no valid snapshot")?;
    let persistent = slot
        .entries
        .iter()
        .find(|entry| entry.path == "/USER/PERSIST.TXT")
        .map(|entry| entry.data.as_slice());
    let unrelated = slot
        .entries
        .iter()
        .find(|entry| entry.path == "/USER/KEEP.TXT")
        .map(|entry| entry.data.as_slice());
    let shell = slot
        .entries
        .iter()
        .find(|entry| entry.path == "/USER/SHELL.TXT")
        .map(|entry| entry.data.as_slice());
    if persistent != Some(b"GenOS persistent storage survived a reboot.")
        || unrelated != Some(b"unrelated file remains intact")
        || shell != Some(b"Ring 3 shell file mutation is ready.")
        || slot
            .entries
            .iter()
            .any(|entry| entry.path.starts_with("/TMP/"))
    {
        return Err(
            "persistent image contents or RAM-filesystem separation are invalid".to_string(),
        );
    }
    Ok(())
}

fn inspect_filesystem_image(path: &Path) -> Result<ImageReport, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let (partition_start, _) = valid_genos_partition(&bytes)?;
    let mut report = ImageReport {
        valid_slots: Vec::new(),
        invalid_slots: Vec::new(),
    };
    for (slot, slot_offset) in SLOT_OFFSETS.iter().enumerate() {
        let start = (partition_start + slot_offset) * 512;
        let snapshot = &bytes[start..start + SLOT_BYTES];
        if snapshot.iter().all(|byte| *byte == 0) {
            continue;
        }
        match decode_filesystem_slot(slot, snapshot) {
            Ok(valid) => report.valid_slots.push(valid),
            Err(_) => report.invalid_slots.push(slot),
        }
    }
    Ok(report)
}

fn decode_filesystem_slot(slot: usize, snapshot: &[u8]) -> Result<InspectedSlot, String> {
    if snapshot.len() != SLOT_BYTES
        || &snapshot[..4] != b"GFS2"
        || u16::from_le_bytes([snapshot[4], snapshot[5]]) != 3
        || snapshot[6] == 0
        || snapshot[7] != 0xa5
    {
        return Err("invalid GenOS filesystem slot header".to_string());
    }
    let used = u32::from_le_bytes(snapshot[16..20].try_into().unwrap()) as usize;
    if !(SNAPSHOT_HEADER_BYTES..=SLOT_BYTES).contains(&used) {
        return Err("invalid GenOS filesystem used length".to_string());
    }
    let expected = u32::from_le_bytes(
        snapshot[SNAPSHOT_CHECKSUM_OFFSET..SNAPSHOT_CHECKSUM_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    if snapshot_checksum(snapshot) != expected {
        return Err("invalid GenOS filesystem slot checksum".to_string());
    }
    let generation = u64::from_le_bytes(snapshot[8..16].try_into().unwrap());
    let mut cursor = SNAPSHOT_HEADER_BYTES;
    let mut entries = Vec::new();
    for _ in 0..snapshot[6] {
        if cursor + 4 > used {
            return Err("truncated GenOS filesystem entry".to_string());
        }
        let path_len = snapshot[cursor] as usize;
        let directory = match snapshot[cursor + 1] {
            1 => false,
            2 => true,
            _ => return Err("invalid GenOS filesystem entry kind".to_string()),
        };
        let data_len = u16::from_le_bytes([snapshot[cursor + 2], snapshot[cursor + 3]]) as usize;
        if path_len == 0 || (directory && data_len != 0) {
            return Err("invalid GenOS filesystem entry header".to_string());
        }
        let path_start = cursor + 4;
        let data_start = path_start
            .checked_add(path_len)
            .ok_or("filesystem path overflow")?;
        let end = data_start
            .checked_add(data_len)
            .filter(|end| *end <= used)
            .ok_or("filesystem data overflow")?;
        let path = String::from_utf8(snapshot[path_start..data_start].to_vec())
            .map_err(|_| "filesystem path is not UTF-8")?;
        if !path.starts_with("/USER/")
            || entries
                .iter()
                .any(|entry: &InspectedEntry| entry.path.eq_ignore_ascii_case(&path))
        {
            return Err("filesystem path is invalid or duplicated".to_string());
        }
        entries.push(InspectedEntry {
            path,
            directory,
            data: snapshot[data_start..end].to_vec(),
        });
        cursor = end;
    }
    if cursor != used {
        return Err("filesystem entry table does not match used length".to_string());
    }
    Ok(InspectedSlot {
        slot,
        generation,
        entries,
    })
}

fn simulate_torn_write(path: &Path) -> Result<(), String> {
    let mut bytes = fs::read(path).map_err(|error| error.to_string())?;
    let (partition_start, _) = valid_genos_partition(&bytes)?;
    let report = inspect_filesystem_image(path)?;
    let newest = report
        .valid_slots
        .iter()
        .max_by_key(|slot| slot.generation)
        .ok_or("cannot inject a torn write without a valid generation")?;
    let target = newest.slot ^ 1;
    let source_start = (partition_start + SLOT_OFFSETS[newest.slot]) * 512;
    let target_start = (partition_start + SLOT_OFFSETS[target]) * 512;
    let source = bytes[source_start..source_start + SLOT_BYTES].to_vec();
    bytes[target_start..target_start + SLOT_BYTES].copy_from_slice(&source);
    let target_snapshot = &mut bytes[target_start..target_start + SLOT_BYTES];
    target_snapshot[8..16].copy_from_slice(&(newest.generation + 1).to_le_bytes());
    target_snapshot[SNAPSHOT_CHECKSUM_OFFSET..SNAPSHOT_CHECKSUM_OFFSET + 4].fill(0);
    let checksum = snapshot_checksum(target_snapshot);
    target_snapshot[SNAPSHOT_CHECKSUM_OFFSET..SNAPSHOT_CHECKSUM_OFFSET + 4]
        .copy_from_slice(&checksum.to_le_bytes());
    target_snapshot[SNAPSHOT_HEADER_BYTES + 8] ^= 0x80;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

#[derive(Debug, Eq, PartialEq)]
enum RepairOutcome {
    Healthy,
    Repaired {
        source_slot: usize,
        target_slot: usize,
        generation: u64,
    },
}

fn repair_filesystem_image(path: &Path) -> Result<RepairOutcome, String> {
    let mut bytes = fs::read(path).map_err(|error| error.to_string())?;
    let (partition_start, _) = valid_genos_partition(&bytes)?;
    let report = inspect_filesystem_image(path)?;
    match report.valid_slots.len() {
        2 => return Ok(RepairOutcome::Healthy),
        1 => {}
        _ => {
            return Err(
                "refusing repair: no valid GenOS snapshot exists to use as a trusted source"
                    .to_string(),
            );
        }
    }

    let source = &report.valid_slots[0];
    let target_slot = source.slot ^ 1;
    let source_start = (partition_start + SLOT_OFFSETS[source.slot]) * 512;
    let target_start = (partition_start + SLOT_OFFSETS[target_slot]) * 512;
    let source_snapshot = bytes[source_start..source_start + SLOT_BYTES].to_vec();
    bytes[target_start..target_start + SLOT_BYTES].copy_from_slice(&source_snapshot);
    let repaired = &mut bytes[target_start..target_start + SLOT_BYTES];
    let generation = source
        .generation
        .checked_add(1)
        .ok_or("refusing repair: snapshot generation is exhausted")?;
    repaired[8..16].copy_from_slice(&generation.to_le_bytes());
    repaired[SNAPSHOT_CHECKSUM_OFFSET..SNAPSHOT_CHECKSUM_OFFSET + 4].fill(0);
    let checksum = snapshot_checksum(repaired);
    repaired[SNAPSHOT_CHECKSUM_OFFSET..SNAPSHOT_CHECKSUM_OFFSET + 4]
        .copy_from_slice(&checksum.to_le_bytes());

    fs::write(path, bytes).map_err(|error| error.to_string())?;
    let verified = inspect_filesystem_image(path)?;
    if verified.valid_slots.len() != 2 || !verified.invalid_slots.is_empty() {
        return Err("repair verification failed; image was not reported healthy".to_string());
    }
    Ok(RepairOutcome::Repaired {
        source_slot: source.slot,
        target_slot,
        generation,
    })
}

fn ensure_failure_image() -> Result<(), String> {
    write_partitioned_image(Path::new(FAILURE_DATA_IMAGE), true)
}

fn create_read_only_recovery_image() -> Result<(), String> {
    fs::copy(TEST_DATA_IMAGE, READ_ONLY_DATA_IMAGE)
        .map_err(|error| format!("failed to create read-only recovery image: {error}"))?;
    let mut bytes = fs::read(READ_ONLY_DATA_IMAGE).map_err(|error| error.to_string())?;
    if bytes.len() < 512 || bytes[510..512] != [0x55, 0xaa] {
        return Err("cannot mark read-only recovery image: invalid MBR".to_string());
    }
    let entry = (0..4)
        .map(|index| 446 + index * 16)
        .find(|offset| bytes[*offset + 4] == PARTITION_TYPE_GENOS)
        .ok_or("cannot mark read-only recovery image: GenOS partition not found")?;
    bytes[entry + 4] = PARTITION_TYPE_GENOS_READ_ONLY;
    fs::write(READ_ONLY_DATA_IMAGE, bytes).map_err(|error| error.to_string())
}

fn snapshot_checksum(bytes: &[u8]) -> u32 {
    let mut checksum = 0x811c_9dc5u32;
    for (index, byte) in bytes.iter().enumerate() {
        checksum ^= if (SNAPSHOT_CHECKSUM_OFFSET..SNAPSHOT_CHECKSUM_OFFSET + 4).contains(&index) {
            0
        } else {
            *byte as u32
        };
        checksum = checksum.wrapping_mul(0x0100_0193);
    }
    checksum
}

fn smoke_markers_ready(output: &str) -> bool {
    let required = [
        "IRQ_READY",
        "RECOVERY_BOUNDARY_OK",
        "VFS_READY",
        "RAMFS_TEMP_CLEAN_OK",
        "RAMFS_TEMP_READY",
        "RAMFS_TEMPORARY_READY",
        "PARTITION_DISCOVERED",
        "PCI_STORAGE_CONTROLLER_READY",
        "BLOCK_CACHE_READY",
        "BLOCK_CACHE_HIT_OK",
        "PERSISTENT_STORAGE_RESTORED",
        "PERSISTENT_STORAGE_READY",
        "TASKS_READY",
        "SCHED_READY",
        "RUNTIME_COORDINATOR_READY",
        "HEADLESS_RUNTIME_READY",
        "PROCESS_SNAPSHOT_READY",
        "UNIFIED_HANDLE_TABLE_READY",
        "ASYNC_REQUEST_IDENTITY_READY",
        "USER_ASYNC_REQUEST_ID_OK",
        "USER_ASYNC_CANCELLATION_OK",
        "USER_ASYNC_ONE_SHOT_OK",
        "USER_SUPERVISOR_CLEANUP_OK mode=exit",
        "USER_SUPERVISOR_CLEANUP_OK mode=fault",
        "USER_SUPERVISOR_CLEANUP_OK mode=kill",
        "USER_SUPERVISOR_NO_STALE_TASKS_OK",
        "USER_SUPERVISOR_NO_STALE_HANDLES_OK",
        "USER_SUPERVISOR_PENDING_CANCEL_OK",
        "SUPERVISOR_CLEANUP_READY",
        "USER_ROLLBACK_FULL_TABLE_OK",
        "USER_ROLLBACK_LAUNCH_REFUSED_OK",
        "USER_ROLLBACK_COPYOUT_OK",
        "USER_ROLLBACK_CANCELLATION_OK",
        "RUNTIME_ROLLBACK_READY",
        "USER_PROCESS_GENERATION_STRESS_OK launches=257",
        "USER_PID_REUSE_SAFE_OK",
        "USER_STALE_PROCESS_HANDLE_REJECTED_OK",
        "PROCESS_GENERATION_STRESS_READY",
        "SCHED_DISPATCH_BENCH_OK",
        "SCHED_CONTEXT_BENCH_OK",
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
        "USER_OUTPUT_OK",
        "USER_RECLAIM_OK",
        "USER_ASYNC_EXIT_OK",
        "USER_OUTPUT_ASYNC_OK",
        "USER_KILL_OK",
        "USER_WAIT_OK",
        "USER_SLEEP_OK",
        "USER_CHILD_WAIT_OK",
        "USER_MESSAGE_OK",
        "USER_COORDINATION_OK",
        "USER_ENDPOINT_CAPABILITY_OK",
        "USER_CHANNEL_FAIRNESS_OK",
        "USER_ENDPOINT_WAKE_OK",
        "USER_FANIN_OK",
        "USER_COPY_OUT_OK",
        "USER_STRUCT_COPY_OK",
        "USER_VFS_BLOCKING_OK",
        "USER_FILE_CAPABILITY_OK",
        "USER_FILE_OFFSET_OK",
        "USER_FILE_CLOSE_OK",
        "USER_FILE_WRITE_OK",
        "USER_FILE_WRITE_POLICY_OK",
        "USER_FILE_WRITE_READBACK_OK",
        "USER_HANDLE_TRUNCATE_OK",
        "USER_INPUT_BLOCK_OK",
        "USER_INPUT_FILTER_OK",
        "USER_INPUT_OWNERSHIP_OK",
        "USER_INPUT_WAKE_OK",
        "USER_ASYNC_LIFECYCLE_OK",
        "USER_PROCESS_LAUNCHED",
        "USER_PROCESS_STATUS",
        "USER_PROCESS_KILLED",
        "USER_PROCESS_REAPED",
        "USER_SHELL_PROCESS_CONTROL_OK",
        "USER_SHELL_NAMESPACE_OK",
        "USER_SHELL_HISTORY_OK",
        "USER_SOCKET_CAPABILITY_READY abi=18",
        "USER_SOCKET_LISTENER_CAPABILITY_READY abi=18",
        "USER_SHELL_READY",
        "USER_STORAGE_STATUS_VISIBLE_OK",
        "USER_RAMFS_TEMP_APP_OK",
        "USER_DURABLE_RESTORE_OK",
        "USER_CONSOLE_TRANSCRIPT_OK commands=2",
        "USER_CONSOLE_HEADLESS_OK",
        "CONSOLE_TRANSCRIPT_READY",
        "PERSISTENT_STORAGE_RESTORED",
        "PERSISTENT_STORAGE_READY",
        "USER_DIRECTORY_READ_OK",
        "USER_ISOLATION_OK",
        "USERMODE_READY",
        "SERVER_TERMINAL_READY",
        "SERIAL_TERMINAL_READY",
        "GENOS_READY",
        "IRQ_HARDWARE_ON",
        "IRQ_TICK_OK",
        "TERMINAL_IDLE_OK",
    ]
    .iter()
    .all(|marker| output.contains(marker));
    required
        && markers_in_order(
            output,
            &[
                "USER_SOCKET_LISTENER_CAPABILITY_READY abi=18",
                "USER_SOCKET_CAPABILITY_READY abi=18",
                "USER_PROCESS_LAUNCHED",
                "USER_PROCESS_STATUS",
                "USER_PROCESS_KILLED",
                "USER_PROCESS_REAPED",
                "USER_SHELL_PROCESS_CONTROL_OK",
                "USER_SHELL_NAMESPACE_OK",
                "USER_SHELL_HISTORY_OK",
                "USER_SHELL_READY",
            ],
        )
}

fn markers_in_order(output: &str, markers: &[&str]) -> bool {
    let mut cursor = 0usize;
    for marker in markers {
        let Some(offset) = output[cursor..].find(marker) else {
            return false;
        };
        cursor += offset + marker.len();
    }
    true
}

fn find_ovmf_code() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("GENOS_OVMF_CODE") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path).ok_or_else(|| {
            "GENOS_OVMF_CODE must name a readable x86_64 OVMF/EDK2 firmware file".to_string()
        });
    }
    select_ovmf_code(OVMF_CANDIDATES.iter().map(PathBuf::from)).ok_or_else(|| {
        "could not find OVMF/EDK2 x86_64 firmware; install QEMU/OVMF or set GENOS_OVMF_CODE"
            .to_string()
    })
}

const OVMF_CANDIDATES: &[&str] = &[
    "/opt/homebrew/share/qemu/edk2-x86_64-code.fd",
    "/usr/local/share/qemu/edk2-x86_64-code.fd",
    "/usr/share/OVMF/OVMF_CODE.fd",
    "/usr/share/OVMF/OVMF_CODE_4M.fd",
    "/usr/share/edk2/x64/OVMF_CODE.fd",
    "/usr/share/qemu/OVMF.fd",
];

fn select_ovmf_code(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_user_volume_is_never_silently_reformatted() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "genos-corrupt-volume-{}-{nonce}.img",
            std::process::id()
        ));
        fs::write(&path, b"irreplaceable user data").unwrap();
        assert!(ensure_partitioned_image(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"irreplaceable user data");
        fs::remove_file(path).unwrap();
        assert_ne!(DATA_IMAGE, TEST_DATA_IMAGE);
    }

    #[test]
    fn clean_preserves_the_user_volume() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!("genos-clean-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("genos-data.img"), b"keep my files").unwrap();
        fs::write(directory.join("genos.img"), b"generated boot image").unwrap();
        fs::create_dir(directory.join("generated")).unwrap();
        clean_generated_files(&directory).unwrap();
        assert_eq!(
            fs::read(directory.join("genos-data.img")).unwrap(),
            b"keep my files"
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ovmf_search_has_known_names() {
        assert!(OVMF_CANDIDATES.contains(&"/opt/homebrew/share/qemu/edk2-x86_64-code.fd"));
        assert!(OVMF_CANDIDATES.contains(&"/usr/share/OVMF/OVMF_CODE_4M.fd"));
        // Host tests must not depend on an emulator being installed. Exercise
        // selection with a known repository file and reject directories.
        let existing = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert_eq!(
            select_ovmf_code([existing.parent().unwrap().to_path_buf(), existing.clone()]),
            Some(existing)
        );
        assert_eq!(select_ovmf_code([]), None);
    }

    #[test]
    fn smoke_requires_async_lifecycle_and_reclaim_markers() {
        assert!(smoke_markers_ready(concat!(
            "RAMFS_TEMP_CLEAN_OK\nRAMFS_TEMP_READY\nRAMFS_TEMPORARY_READY\nPCI_STORAGE_CONTROLLER_READY\nPARTITION_DISCOVERED\nBLOCK_CACHE_READY\nBLOCK_CACHE_HIT_OK\nUSER_STORAGE_STATUS_VISIBLE_OK\nUSER_RAMFS_TEMP_APP_OK\nUSER_DURABLE_RESTORE_OK\n",
            "IRQ_READY\nRECOVERY_BOUNDARY_OK\nVFS_READY\nTASKS_READY\nSCHED_READY\nRUNTIME_COORDINATOR_READY\nHEADLESS_RUNTIME_READY\nPROCESS_SNAPSHOT_READY\nUNIFIED_HANDLE_TABLE_READY\nASYNC_REQUEST_IDENTITY_READY\nSCHED_DISPATCH_BENCH_OK\nSCHED_CONTEXT_BENCH_OK\nPAGING_READY\nADDRESS_SPACES_READY\nUSER_ELF_VALIDATED\nUSER_ELF_LOADED\nUSER_ELF_LAUNCH_OK\nUSER_CONTEXT_OK\nUSER_CONTEXT_RESUME_OK\nUSER_PREEMPT_OK\nUSER_FAULT_TERMINATED\nUSER_FAULT_ISOLATED\nUSER_SYSCALL_OK\nUSER_COPY_OK\nUSER_OUTPUT_OK\nUSER_RECLAIM_OK\nUSER_ASYNC_EXIT_OK\nUSER_OUTPUT_ASYNC_OK\nUSER_KILL_OK\nUSER_WAIT_OK\nUSER_SLEEP_OK\nUSER_CHILD_WAIT_OK\nUSER_MESSAGE_OK\nUSER_COORDINATION_OK\nUSER_ENDPOINT_CAPABILITY_OK\nUSER_CHANNEL_FAIRNESS_OK\nUSER_ENDPOINT_WAKE_OK\nUSER_FANIN_OK\nUSER_COPY_OUT_OK\nUSER_STRUCT_COPY_OK\nUSER_VFS_BLOCKING_OK\nUSER_FILE_CAPABILITY_OK\nUSER_FILE_OFFSET_OK\nUSER_FILE_CLOSE_OK\nUSER_ASYNC_REQUEST_ID_OK\nUSER_ASYNC_CANCELLATION_OK\nUSER_ASYNC_ONE_SHOT_OK\nUSER_SUPERVISOR_CLEANUP_OK mode=exit\nUSER_SUPERVISOR_CLEANUP_OK mode=fault\nUSER_SUPERVISOR_CLEANUP_OK mode=kill\nUSER_SUPERVISOR_NO_STALE_TASKS_OK\nUSER_SUPERVISOR_NO_STALE_HANDLES_OK\nUSER_SUPERVISOR_PENDING_CANCEL_OK\nSUPERVISOR_CLEANUP_READY\nUSER_ROLLBACK_FULL_TABLE_OK\nUSER_ROLLBACK_LAUNCH_REFUSED_OK\nUSER_ROLLBACK_COPYOUT_OK\nUSER_ROLLBACK_CANCELLATION_OK\nRUNTIME_ROLLBACK_READY\nUSER_PROCESS_GENERATION_STRESS_OK launches=257\nUSER_PID_REUSE_SAFE_OK\nUSER_STALE_PROCESS_HANDLE_REJECTED_OK\nPROCESS_GENERATION_STRESS_READY\nUSER_FILE_WRITE_OK\nUSER_FILE_WRITE_POLICY_OK\nUSER_FILE_WRITE_READBACK_OK\nUSER_HANDLE_TRUNCATE_OK\nUSER_INPUT_BLOCK_OK\nUSER_INPUT_FILTER_OK\nUSER_INPUT_OWNERSHIP_OK\nUSER_INPUT_WAKE_OK\nUSER_ASYNC_LIFECYCLE_OK\nUSER_SOCKET_LISTENER_CAPABILITY_READY abi=18\nUSER_SOCKET_CAPABILITY_READY abi=18\nUSER_PROCESS_LAUNCHED\nUSER_PROCESS_STATUS\nUSER_PROCESS_KILLED\nUSER_PROCESS_REAPED\nUSER_SHELL_PROCESS_CONTROL_OK\nUSER_SHELL_NAMESPACE_OK\nUSER_SHELL_HISTORY_OK\nUSER_DIRECTORY_READ_OK\nUSER_SHELL_READY\nUSER_CONSOLE_TRANSCRIPT_OK commands=2\nUSER_CONSOLE_HEADLESS_OK\nCONSOLE_TRANSCRIPT_READY\nPERSISTENT_STORAGE_RESTORED\nPERSISTENT_STORAGE_READY\nUSER_ISOLATION_OK\nUSERMODE_READY\nSERVER_TERMINAL_READY\nSERIAL_TERMINAL_READY\nGENOS_READY\nIRQ_HARDWARE_ON\nIRQ_TICK_OK\nTERMINAL_IDLE_OK\n"
        )));
        assert!(!smoke_markers_ready("GENOS_READY\n"));
        assert!(!smoke_markers_ready(
            "USER_SHELL_READY\nUSER_SHELL_PROCESS_CONTROL_OK\nUSER_PROCESS_REAPED\nUSER_PROCESS_KILLED\nUSER_PROCESS_STATUS\nUSER_PROCESS_LAUNCHED\n"
        ));
    }

    #[test]
    fn desktop_shell_does_not_complete_runtime_requests() {
        let shell = include_str!("../../../kernel/src/shell.rs");
        let display = include_str!("../../../kernel/src/display/manager.rs");
        let runtime = include_str!("../../../kernel/src/runtime.rs");
        for operation in [
            "complete_process_launch",
            "complete_file_open",
            "complete_file_read",
            "complete_file_write",
            "complete_file_truncate",
            "complete_directory_read",
            "complete_directory_create",
            "complete_path_remove",
            "vfs_request_active",
        ] {
            assert!(
                !shell.contains(operation),
                "{operation} leaked into shell.rs"
            );
            assert!(
                !display.contains(operation),
                "{operation} leaked into DisplayManager"
            );
            assert!(
                runtime.contains(operation),
                "{operation} missing from runtime.rs"
            );
        }
    }

    #[test]
    fn canceled_async_work_is_gated_before_external_mutation() {
        let runtime = include_str!("../../../kernel/src/runtime.rs");
        let userspace = include_str!("../../../kernel/src/userspace.rs");
        let vfs_start = runtime
            .find("fn complete_vfs_request")
            .expect("runtime VFS completion exists");
        let vfs_end = runtime[vfs_start..]
            .find("fn allocate_process_task_id")
            .map(|offset| vfs_start + offset)
            .expect("runtime VFS completion is bounded");
        let vfs_completion = &runtime[vfs_start..vfs_end];
        let active_gate = vfs_completion
            .find("if !self.processes.vfs_request_active(request)")
            .expect("stale VFS gate exists");
        let mutation_dispatch = vfs_completion
            .find("let completion = match request")
            .expect("VFS mutation dispatch exists");
        assert!(active_gate < mutation_dispatch);

        let lifecycle_start = runtime
            .find("fn complete_lifecycle_request")
            .expect("runtime lifecycle completion exists");
        let lifecycle_end = runtime[lifecycle_start..]
            .find("fn complete_vfs_request")
            .map(|offset| lifecycle_start + offset)
            .expect("runtime lifecycle completion is bounded");
        let lifecycle_completion = &runtime[lifecycle_start..lifecycle_end];
        assert!(
            lifecycle_completion
                .find("lifecycle_request_active")
                .expect("stale lifecycle gate exists")
                < lifecycle_completion
                    .find("allocate_process_task_id")
                    .expect("process allocation exists")
        );

        for request in [
            "NamespaceMutationRequest",
            "DirectoryReadRequest",
            "FileOpenRequest",
            "FileReadRequest",
            "FileWriteRequest",
            "FileTruncateRequest",
        ] {
            let declaration = userspace
                .find(&format!("pub struct {request}"))
                .unwrap_or_else(|| panic!("{request} declaration exists"));
            assert!(userspace[declaration..].starts_with(&format!(
                "pub struct {request} {{\n    pub request_id: u64,"
            )));
        }
        assert!(userspace.contains("pending.request_id == request.request_id"));
    }

    #[test]
    fn every_supervisor_terminal_path_uses_the_same_cleanup() {
        let userspace = include_str!("../../../kernel/src/userspace.rs");
        let terminal = userspace
            .find("fn complete_terminal")
            .expect("terminal completion exists");
        let cleanup = userspace
            .find("fn terminate_process_at")
            .expect("central termination exists");
        let external_kill = userspace.find("pub fn kill").expect("external kill exists");
        assert!(userspace[terminal..cleanup].contains("terminate_process_at"));
        assert!(userspace[external_kill..].contains("terminate_process_at"));
        assert!(userspace[cleanup..].contains("cleanup_supervised_children"));
        assert!(userspace.contains("self.process.console_handle = 0"));
        assert!(userspace.contains("self.process.lifecycle_handle = 0"));
        assert!(userspace.contains("USER_SUPERVISOR_NO_STALE_TASKS_OK"));
        assert!(userspace.contains("USER_SUPERVISOR_PENDING_CANCEL_OK"));
    }

    #[test]
    fn runtime_failure_and_generation_proofs_are_required_at_boot() {
        let main = include_str!("../../../kernel/src/main.rs");
        let userspace = include_str!("../../../kernel/src/userspace.rs");
        assert!(main.contains("run_transactional_rollback_probe"));
        assert!(main.contains("run_process_generation_stress_probe"));
        assert!(userspace.contains("const LAUNCHES: usize = 257"));
        assert!(userspace.contains("USER_ROLLBACK_COPYOUT_OK"));
        assert!(userspace.contains("USER_STALE_PROCESS_HANDLE_REJECTED_OK"));
    }

    #[test]
    fn console_transcript_uses_real_ring3_input_and_output() {
        let runtime = include_str!("../../../kernel/src/runtime.rs");
        assert!(runtime.contains("InputEvent::Key(KeyEvent::Char(byte))"));
        assert!(runtime.contains("InputEvent::Key(KeyEvent::Enter)"));
        assert!(runtime.contains("ConsoleUpdate::Write"));
        assert!(runtime.contains("echo qemu-console"));
        assert!(runtime.contains("USER_CONSOLE_TRANSCRIPT_OK commands=2"));
    }

    #[test]
    fn persistent_image_decoder_checks_payload_and_checksum() {
        let payload = b"GenOS persistent storage survived a reboot.";
        let mut bytes = vec![0u8; SLOT_BYTES];
        bytes[..4].copy_from_slice(b"GFS2");
        bytes[4..6].copy_from_slice(&3u16.to_le_bytes());
        bytes[6] = 1;
        bytes[7] = 0xa5;
        bytes[8..16].copy_from_slice(&1u64.to_le_bytes());
        let path = b"/USER/PERSIST.TXT";
        let cursor = SNAPSHOT_HEADER_BYTES;
        bytes[cursor] = path.len() as u8;
        bytes[cursor + 1] = 1;
        bytes[cursor + 2..cursor + 4].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        bytes[cursor + 4..cursor + 4 + path.len()].copy_from_slice(path);
        bytes[cursor + 4 + path.len()..cursor + 4 + path.len() + payload.len()]
            .copy_from_slice(payload);
        let used = cursor + 4 + path.len() + payload.len();
        bytes[16..20].copy_from_slice(&(used as u32).to_le_bytes());
        let checksum = snapshot_checksum(&bytes);
        bytes[SNAPSHOT_CHECKSUM_OFFSET..SNAPSHOT_CHECKSUM_OFFSET + 4]
            .copy_from_slice(&checksum.to_le_bytes());
        let decoded = decode_filesystem_slot(0, &bytes).unwrap();
        assert_eq!(decoded.generation, 1);
        assert_eq!(decoded.entries[0].path, "/USER/PERSIST.TXT");
        assert_eq!(decoded.entries[0].data, payload);
        bytes[SNAPSHOT_HEADER_BYTES + 8] ^= 1;
        assert!(decode_filesystem_slot(0, &bytes).is_err());
    }

    #[test]
    fn repair_writer_restores_redundancy_without_inventing_data() {
        let path = env::temp_dir().join(format!("genos-repair-test-{}.img", std::process::id()));
        let mut image = vec![0u8; DATA_IMAGE_BYTES];
        let sectors = DATA_IMAGE_BYTES / 512 - PARTITION_START_LBA;
        image[446 + 4] = PARTITION_TYPE_GENOS;
        image[446 + 8..446 + 12].copy_from_slice(&(PARTITION_START_LBA as u32).to_le_bytes());
        image[446 + 12..446 + 16].copy_from_slice(&(sectors as u32).to_le_bytes());
        image[510..512].copy_from_slice(&[0x55, 0xaa]);

        let mut snapshot = vec![0u8; SLOT_BYTES];
        snapshot[..4].copy_from_slice(b"GFS2");
        snapshot[4..6].copy_from_slice(&3u16.to_le_bytes());
        snapshot[6] = 1;
        snapshot[7] = 0xa5;
        snapshot[8..16].copy_from_slice(&41u64.to_le_bytes());
        let path_bytes = b"/USER/REPAIR.TXT";
        let payload = b"trusted payload";
        let cursor = SNAPSHOT_HEADER_BYTES;
        snapshot[cursor] = path_bytes.len() as u8;
        snapshot[cursor + 1] = 1;
        snapshot[cursor + 2..cursor + 4].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        snapshot[cursor + 4..cursor + 4 + path_bytes.len()].copy_from_slice(path_bytes);
        snapshot[cursor + 4 + path_bytes.len()..cursor + 4 + path_bytes.len() + payload.len()]
            .copy_from_slice(payload);
        let used = cursor + 4 + path_bytes.len() + payload.len();
        snapshot[16..20].copy_from_slice(&(used as u32).to_le_bytes());
        let checksum = snapshot_checksum(&snapshot);
        snapshot[SNAPSHOT_CHECKSUM_OFFSET..SNAPSHOT_CHECKSUM_OFFSET + 4]
            .copy_from_slice(&checksum.to_le_bytes());
        let slot_start = (PARTITION_START_LBA + SLOT_OFFSETS[0]) * 512;
        image[slot_start..slot_start + SLOT_BYTES].copy_from_slice(&snapshot);
        fs::write(&path, &image).unwrap();

        assert_eq!(
            repair_filesystem_image(&path).unwrap(),
            RepairOutcome::Repaired {
                source_slot: 0,
                target_slot: 1,
                generation: 42,
            }
        );
        let report = inspect_filesystem_image(&path).unwrap();
        assert_eq!(report.valid_slots.len(), 2);
        assert_eq!(report.valid_slots[1].entries[0].data, payload);
        assert_eq!(repair_filesystem_image(&path), Ok(RepairOutcome::Healthy));

        image[slot_start..slot_start + SLOT_BYTES].fill(0x5a);
        let other_start = (PARTITION_START_LBA + SLOT_OFFSETS[1]) * 512;
        image[other_start..other_start + SLOT_BYTES].fill(0x5a);
        fs::write(&path, &image).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(repair_filesystem_image(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn mbr_partition_decoder_rejects_missing_and_out_of_bounds_ranges() {
        let mut bytes = vec![0u8; DATA_IMAGE_BYTES];
        assert!(valid_genos_partition(&bytes).is_err());
        bytes[510..512].copy_from_slice(&[0x55, 0xaa]);
        bytes[446 + 4] = PARTITION_TYPE_GENOS;
        bytes[446 + 8..446 + 12].copy_from_slice(&(PARTITION_START_LBA as u32).to_le_bytes());
        bytes[446 + 12..446 + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(valid_genos_partition(&bytes).is_err());
        let sectors = DATA_IMAGE_BYTES / 512 - PARTITION_START_LBA;
        bytes[446 + 12..446 + 16].copy_from_slice(&(sectors as u32).to_le_bytes());
        assert_eq!(
            valid_genos_partition(&bytes),
            Ok((PARTITION_START_LBA, sectors))
        );
    }

    #[test]
    fn storage_harness_covers_recovery_failure_and_ramfs_separation() {
        let xtask = include_str!("main.rs");
        let storage = include_str!("../../../kernel/src/storage.rs");
        assert!(xtask.contains("simulate_torn_write"));
        assert!(xtask.contains("inspect_filesystem_image"));
        assert!(xtask.contains("genos-data-corrupt.img"));
        assert!(storage.contains("newest_valid_slot"));
        assert!(storage.contains("PERSISTENT_STORAGE_RECOVERED_TORN_WRITE"));
        assert!(storage.contains("PERSISTENT_STORAGE_UNAVAILABLE"));
        assert!(storage.contains("RAMFS_TEMP_CLEAN_OK"));
        assert!(storage.contains("PARTITION_DISCOVERED"));
        assert!(storage.contains("BLOCK_CACHE_READY"));
        assert!(storage.contains("PERSISTENT_COMMIT_OK"));
        assert!(storage.contains("discover_pci_ide_controller"));
    }

    #[test]
    fn network_harness_covers_real_protocols_and_bounded_failure() {
        let xtask = include_str!("main.rs");
        let driver = include_str!("../../../kernel/src/network.rs");
        let device = include_str!("../../../kernel/src/network_device.rs");
        let protocol = include_str!("../../../kernel/src/net.rs");
        let runtime = include_str!("../../../kernel/src/runtime.rs");
        let userspace = include_str!("../../../kernel/src/userspace.rs");
        let abi = include_str!("../../../crates/abi/src/lib.rs");
        let shell = include_str!("../../../userspace/shell/src/main.rs");
        assert!(xtask.contains("virtio-net-pci"));
        assert!(xtask.contains("disable-legacy=on"));
        assert!(!MODERN_NETWORK_DEVICE.contains("ne2k"));
        assert!(xtask.contains("TcpListener::bind"));
        assert!(xtask.contains("hostfwd=tcp:127.0.0.1"));
        assert!(xtask.contains("TcpStream::connect_timeout"));
        assert!(xtask.contains("TCP_PASSIVE_HANDSHAKE_OK"));
        assert!(xtask.contains("USER_SOCKET_PASSIVE_ACCEPT_READY"));
        assert!(xtask.contains("GENOS_PING"));
        assert!(xtask.contains("GENOS_PONG"));
        assert!(xtask.contains("Shutdown::Write"));
        assert!(xtask.contains("TCP_PASSIVE_STREAM_RX_OK"));
        assert!(xtask.contains("TCP_PASSIVE_STREAM_TX_OK"));
        assert!(xtask.contains("TCP_PASSIVE_STREAM_FIN_OK"));
        assert!(xtask.contains("USER_SOCKET_PASSIVE_STREAM_READY"));
        assert!(xtask.contains("USER_SOCKET_PASSIVE_CONCURRENT_READY streams=2"));
        assert!(xtask.contains("network-test-faults"));
        assert!(xtask.contains("verify_production_fault_hooks_absent"));
        assert!(xtask.contains("TCP_FAULT_DATA_DROP_INJECTED"));
        assert!(xtask.contains("TCP_FAULT_REORDER_RELEASED"));
        assert!(xtask.contains("NETWORK_REGRESSION_BUDGET_OK"));
        assert!(xtask.contains("TCP_STREAM_LARGE_READY"));
        assert!(xtask.contains("TCP_CONGESTION_CONTROL_READY"));
        assert!(xtask.contains("GENOS_PING_STREAM_7"));
        assert!(xtask.contains("GENOS_PING_A"));
        assert!(xtask.contains("GENOS_PING_B"));
        assert!(xtask.contains("Barrier::new(2)"));
        assert!(xtask.contains("USER_HTTP_REQUEST_OK"));
        assert!(driver.contains("PacketOwner"));
        assert!(driver.contains("NetworkDevice"));
        assert!(driver.contains("const RETRIES: usize = 3"));
        assert!(driver.contains("dhcp_attempt"));
        assert!(driver.contains("resolve_arp"));
        assert!(driver.contains("start_udp_async"));
        assert!(driver.contains("poll_udp_async"));
        assert!(driver.contains("start_tcp_async"));
        assert!(driver.contains("poll_tcp_async"));
        assert!(driver.contains("poll_tcp_passive"));
        assert!(driver.contains("poll_tcp_passive_stream"));
        assert!(driver.contains("start_tcp_passive_stream_send"));
        assert!(driver.contains("PASSIVE_TCP_STREAM_IDLE_TICKS"));
        assert!(driver.contains("PASSIVE_TCP_HANDSHAKE_SLOTS: usize = 4"));
        assert!(driver.contains("PASSIVE_TCP_STREAM_SLOTS: usize = 4"));
        assert!(driver.contains("PASSIVE_TCP_MAX_CWND: usize = PASSIVE_TCP_MSS * 8"));
        assert!(driver.contains("tcp_congestion_events"));
        assert!(driver.contains("TCP_CONGESTION_BACKOFF"));
        assert!(driver.contains("pump_passive_rx"));
        assert!(driver.contains("DeferredTcpSegment"));
        assert!(driver.contains("receive_window"));
        assert!(driver.contains("tcp_retransmissions"));
        assert!(driver.contains("ASYNC_UDP_RX_POLLS_PER_TICK"));
        assert!(driver.contains("ASYNC_TCP_RX_POLLS_PER_TICK"));
        assert!(driver.contains("NETWORK_ICMP_ECHO_OK"));
        assert!(device.contains("VIRTIO_F_VERSION_1"));
        assert!(device.contains("VIRTIO_PCI_CAP_COMMON_CFG"));
        assert!(device.contains("VIRTIO_RX_QUEUE_MEMORY"));
        assert!(device.contains("VIRTIO_NET_MSIX_READY"));
        assert!(device.contains("record_virtio_interrupt"));
        assert!(device.contains("VIRTIO_RECOVERY_POLL_INTERVAL"));
        assert!(device.contains("ne2000-pio-legacy-fallback"));
        assert!(protocol.contains("transport_checksum_valid"));
        assert!(protocol.contains("parse_arp_reply"));
        assert!(protocol.contains("passive_tcp_handshake_requires_exact_bounded_packets"));
        assert!(protocol.contains("malformed_packets_are_rejected_without_indexing_past_bounds"));
        assert!(runtime.contains("PASSIVE_TCP_PER_PROCESS_BUDGET: usize = 2"));
        assert!(runtime.contains("passive_owner_count"));
        assert!(runtime.contains("TCP_PASSIVE_OWNER_BUDGET_REFUSED"));
        assert!(userspace.contains("SOCKET_WAIT_WAKE_BUDGET: usize = 2"));
        assert!(userspace.contains("socket_wait_cursor"));
        assert!(userspace.contains("BlockReason::Socket"));
        assert!(abi.contains("USER_ABI_VERSION: u64 = 18"));
        assert!(abi.contains("USER_SYSCALL_SOCKET_WAIT: u64 = 48"));
        assert!(abi.contains("USER_SOCKET_WAIT_MAX_TICKS: u64 = 10_000"));
        assert!(shell.contains("runtime::udp_exchange"));
        assert!(shell.contains("runtime::tcp_exchange"));
        assert!(shell.contains("runtime::socket_receive"));
        assert!(shell.contains("asynchronous UDP socket ready"));
        assert!(shell.contains("asynchronous TCP socket ready"));
        assert!(shell.contains("passive TCP accept ready"));
        assert!(shell.contains("passive TCP stream ready"));
        assert!(shell.contains("concurrent passive TCP streams ready"));
        assert!(shell.contains("runtime::socket_wait"));
        assert!(shell.contains("socket readiness wait ready"));
        assert!(shell.contains("network diagnostics ready"));
    }

    #[test]
    fn process_state_and_headless_boot_have_single_owners() {
        let tasks = include_str!("../../../kernel/src/tasks.rs");
        let userspace = include_str!("../../../kernel/src/userspace.rs");
        let display = include_str!("../../../kernel/src/display/manager.rs");
        let main = include_str!("../../../kernel/src/main.rs");

        for removed in [
            "pub fn reserve_user",
            "pub fn bind_user_runtime",
            "pub fn update_user",
        ] {
            assert!(
                !tasks.contains(removed),
                "duplicate lifecycle API remains: {removed}"
            );
        }
        assert!(userspace.contains("pub fn append_task_snapshots"));
        assert!(userspace.contains("pub fn task_snapshots_match"));
        assert!(display.contains("TaskSnapshotSet"));
        assert!(!display.contains("TaskRegistry"));

        let headless = main.find("run_headless_boot_probe").unwrap();
        let terminal = main.find("SERVER_TERMINAL_READY").unwrap();
        assert!(headless < terminal);
        assert!(!main.contains("FramebufferDevice::new"));
    }
}
