//! One versioned VM contract for developer runs and qualification evidence.
use std::{collections::BTreeMap, path::Path, process::Command};

const SOURCE: &str = include_str!("../../reference-vm.conf");
const KEYS: &[&str] = &[
    "profile",
    "machine",
    "cpu",
    "accel",
    "memory",
    "smp",
    "qemu_version",
    "rust_release",
    "firmware_sha256",
    "boot_drive",
    "storage_controller",
    "storage_device",
    "network_backend",
    "network_device",
];

pub struct Profile<'a>(BTreeMap<&'a str, &'a str>);

impl<'a> Profile<'a> {
    fn parse(source: &'a str) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        for line in source
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let (key, value) = line.split_once('=').ok_or("malformed VM profile field")?;
            if !KEYS.contains(&key)
                || value.is_empty()
                || value.trim() != value
                || values.insert(key, value).is_some()
            {
                return Err(format!(
                    "unknown, empty or duplicate VM profile field: {key}"
                ));
            }
        }
        if KEYS.iter().any(|key| !values.contains_key(key)) {
            return Err("missing VM profile field".into());
        }
        if !is_sha256(values["firmware_sha256"]) {
            return Err("invalid reference firmware SHA256".into());
        }
        Ok(Self(values))
    }

    pub fn get(&self, key: &str) -> &'a str {
        self.0[key]
    }

    fn mismatches(&self, rust: &str, qemu: &str, firmware: &str) -> Vec<&'static str> {
        let mut differences = Vec::new();
        if !rust
            .lines()
            .any(|line| line.strip_prefix("release: ") == Some(self.get("rust_release")))
        {
            differences.push("rust_release");
        }
        if qemu.lines().next() != Some(self.get("qemu_version")) {
            differences.push("qemu_version");
        }
        if firmware != self.get("firmware_sha256") {
            differences.push("firmware_sha256");
        }
        differences
    }
}

pub fn profile() -> Result<Profile<'static>, String> {
    Profile::parse(SOURCE)
}

pub fn command() -> Result<Command, String> {
    let profile = profile()?;
    let mut command = Command::new("qemu-system-x86_64");
    for (flag, key) in [
        ("-machine", "machine"),
        ("-cpu", "cpu"),
        ("-accel", "accel"),
        ("-m", "memory"),
        ("-smp", "smp"),
    ] {
        command.args([flag, profile.get(key)]);
    }
    Ok(command)
}

pub fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let result = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_string())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn sha256(path: &Path) -> Result<String, String> {
    let path = path.to_str().ok_or("hash path is not UTF-8")?;
    let hash = output(
        "python3",
        &[
            "-c",
            "import hashlib,sys; print(hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest())",
            path,
        ],
    )?;
    if !is_sha256(&hash) {
        return Err("hash command did not return a SHA256 digest".into());
    }
    Ok(hash)
}

pub struct Environment {
    pub report: String,
    pub matches: bool,
}

pub fn environment(firmware: &Path) -> Result<Environment, String> {
    let profile = profile()?;
    let rust = output("rustc", &["-Vv"])?;
    let qemu = output("qemu-system-x86_64", &["--version"])?;
    let firmware_hash = sha256(firmware)?;
    let differences = profile.mismatches(&rust, &qemu, &firmware_hash);
    let matches = differences.is_empty();
    Ok(Environment {
        report: format!("reference_profile={}\nreference_environment_match={matches}\nreference_mismatches={}\nfirmware_path={}\nfirmware_sha256={firmware_hash}\nprofile_sha256={}\nrust={rust:?}\nqemu={qemu:?}\n",
            profile.get("profile"), differences.join(","), firmware.display(),
            sha256(Path::new("tools/reference-vm.conf"))?),
        matches,
    })
}

pub fn clean_source() -> Result<String, String> {
    if !output(
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
    )?
    .is_empty()
    {
        return Err("reference acceptance requires committed source; use test-release for working-tree development".into());
    }
    output("git", &["rev-parse", "HEAD"])
}

pub fn check(firmware: &Path) -> Result<(), String> {
    let environment = environment(firmware)?;
    print!("{}", environment.report);
    if !environment.matches {
        return Err("reference environment mismatch; developer tests remain available, but this environment cannot supply reference qualification".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_rejects_omitted_duplicate_unknown_and_malformed_fields() {
        for invalid in [
            SOURCE.replacen("memory=512M\n", "", 1),
            format!("{SOURCE}memory=1G\n"),
            format!("{SOURCE}memroy=512M\n"),
            SOURCE.replace("memory=512M", "memory="),
            SOURCE.replace("firmware_sha256=3309", "firmware_sha256=xyz"),
        ] {
            assert!(Profile::parse(&invalid).is_err());
        }
    }

    #[test]
    fn each_environment_identity_is_required_for_reference_evidence() {
        let profile = profile().unwrap();
        let rust = format!(
            "rustc {}\nrelease: {}",
            profile.get("rust_release"),
            profile.get("rust_release")
        );
        let qemu = format!("{}\nCopyright", profile.get("qemu_version"));
        let hash = profile.get("firmware_sha256");
        assert!(profile.mismatches(&rust, &qemu, hash).is_empty());
        assert_eq!(
            profile.mismatches("unavailable", &qemu, hash),
            ["rust_release"]
        );
        assert_eq!(
            profile.mismatches(&rust, "QEMU emulator version 11.1.10", hash),
            ["qemu_version"]
        );
        assert_eq!(
            profile.mismatches(&rust, &qemu, &"0".repeat(64)),
            ["firmware_sha256"]
        );
    }

    #[test]
    fn launch_arguments_have_explicit_machine_cpu_accelerator_and_topology() {
        let command = command().unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "-machine",
                "pc-q35-8.2",
                "-cpu",
                "qemu64-v1,smep=on,smap=on",
                "-accel",
                "tcg,thread=single",
                "-m",
                "512M",
                "-smp",
                "1,sockets=1,cores=1,threads=1"
            ]
        );
    }
}
