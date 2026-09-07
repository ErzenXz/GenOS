//! Export a self-contained application workspace. It has no path dependency
//! on this checkout and needs no registry packages or custom Rust toolchain.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const APP_MANIFEST: &str = r#"[package]
name = "genos-app"
version = "0.1.0"
edition = "2021"
license = "MIT"
build = "build.rs"

[workspace]
members = ["sdk/abi", "sdk/runtime"]

[dependencies]
genos-user-runtime = { path = "sdk/runtime" }

[profile.release]
panic = "abort"
opt-level = "s"
lto = false
codegen-units = 1
"#;

const APP_SOURCE: &str = r#"#![no_std]
#![no_main]
use core::panic::PanicInfo;
use genos_user_runtime as runtime;

// This header must remain first in the writable data page. The kernel owns
// its token and preemption fields; application data follows it.
#[used]
#[link_section = ".data.process"]
static mut PROCESS: runtime::UserProcessHeader = runtime::UserProcessHeader::empty();

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start() -> ! {
    if runtime::abi_version() != runtime::ABI_VERSION {
        runtime::exit(78);
    }
    let text = b"Hello from the standalone GenOS SDK";
    if runtime::write(text) != text.len() as u64 { runtime::exit(1); }
    runtime::exit(0)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! { runtime::exit(101) }
"#;

const CONFIG: &str = r#"[build]
target = "x86_64-unknown-none"

[target.x86_64-unknown-none]
rustflags = ["-C", "no-redzone=yes", "-C", "relocation-model=static", "-C", "code-model=large"]
"#;

pub fn new_app(destination: &Path) -> Result<(), String> {
    // Refuse every existing destination (including an empty directory or a
    // symlink). Export never merges into or overwrites somebody's project.
    fs::create_dir(destination).map_err(|e| {
        format!(
            "cannot create new application {}: {e}",
            destination.display()
        )
    })?;
    let result = export(destination);
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn export(destination: &Path) -> Result<(), String> {
    for directory in ["src", ".cargo", "sdk/abi/src", "sdk/runtime/src"] {
        fs::create_dir_all(destination.join(directory)).map_err(|e| e.to_string())?;
    }
    let abi_manifest = "[package]\nname = \"genos_abi\"\nversion = \"0.1.0\"\nedition = \"2021\"\nlicense = \"MIT\"\n";
    let runtime_manifest = "[package]\nname = \"genos-user-runtime\"\nversion = \"0.1.0\"\nedition = \"2021\"\nlicense = \"MIT\"\n\n[dependencies]\ngenos_abi = { path = \"../abi\" }\n";
    let build_script =
        include_str!("../../../userspace/init/build.rs").replace("genos-init", "genos-app");
    for (name, contents) in [
        ("Cargo.toml", APP_MANIFEST),
        ("src/main.rs", APP_SOURCE),
        (".cargo/config.toml", CONFIG),
        (
            "rust-toolchain.toml",
            include_str!("../../../rust-toolchain.toml"),
        ),
        ("build.rs", build_script.as_str()),
        (
            "linker.ld",
            include_str!("../../../userspace/init/linker.ld"),
        ),
        ("sdk/abi/Cargo.toml", abi_manifest),
        ("sdk/runtime/Cargo.toml", runtime_manifest),
        (
            "sdk/abi/src/lib.rs",
            include_str!("../../../crates/abi/src/lib.rs"),
        ),
        (
            "sdk/runtime/src/lib.rs",
            include_str!("../../../userspace/runtime/src/lib.rs"),
        ),
        ("LICENSE", include_str!("../../../LICENSE")),
        ("README.md", include_str!("../../../docs/SDK.md")),
        (".gitignore", "/target/\n"),
    ] {
        fs::write(destination.join(name), contents).map_err(|e| format!("export {name}: {e}"))?;
    }
    println!(
        "Created {} (ABI 18, image layout 2). Build there with cargo build --release --offline.",
        destination.display()
    );
    Ok(())
}

pub fn build_external_example() -> Result<(PathBuf, Vec<u8>), String> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("genos-sdk-{}-{unique}", std::process::id()));
    new_app(&directory)?;
    let status = Command::new("cargo")
        .current_dir(&directory)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .args(["build", "--release", "--offline"])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "standalone SDK build failed; retained {}",
            directory.display()
        ));
    }
    let elf = fs::read(directory.join("target/x86_64-unknown-none/release/genos-app"))
        .map_err(|e| e.to_string())?;
    // Ring 3 boot validation uses the kernel's real ELF parser and mapper,
    // capability boundary, output syscall, exit, and frame reclamation.
    Ok((directory, elf))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_does_not_overwrite_an_existing_project() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("genos-sdk-export-{}-{nonce}", std::process::id()));
        new_app(&path).unwrap();
        fs::write(path.join("important.txt"), "keep").unwrap();
        assert!(new_app(&path).is_err());
        assert_eq!(
            fs::read_to_string(path.join("important.txt")).unwrap(),
            "keep"
        );
        let manifest = fs::read_to_string(path.join("sdk/runtime/Cargo.toml")).unwrap();
        assert!(!manifest.contains("workspace"));
        assert!(!manifest.contains("/Users/"));
        fs::remove_dir_all(path).unwrap();
    }
}
