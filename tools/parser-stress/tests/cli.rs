use std::process::Command;

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_genos-parser-stress"))
}

#[test]
fn retained_corpus_and_truncations_pass() {
    let output = command().args(["--iterations", "0"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("corpus=37 truncations=85137 mutations=0")
    );
}

#[test]
fn replay_uses_production_acceptance_and_rejects_invalid_cli() {
    for (target, fixture, expectation) in [
        ("elf", "elf-valid.bin", "accept"),
        ("elf", "elf-overflow.bin", "reject"),
        ("native-elf", "native-elf-valid.bin", "accept"),
        ("native-elf", "native-elf-late-wx.bin", "reject"),
        ("tcp", "tcp-option-zero-length.bin", "reject"),
        ("tcp", "tcp-eol-nonzero-padding.bin", "reject"),
        ("tcp", "tcp-unknown-option-valid.bin", "accept"),
        ("tcp", "tcp-mss-unaligned-valid.bin", "accept"),
    ] {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("corpus")
            .join(fixture);
        let result = command()
            .args(["--target", target, "--expect", expectation, "--input"])
            .arg(fixture)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for args in [
        vec!["--target", "missing"],
        vec!["--seed", "-1"],
        vec!["--iterations", "10000001"],
        vec!["--expect", "accept"],
    ] {
        assert_eq!(
            command().args(args).output().unwrap().status.code(),
            Some(2)
        );
    }
}

#[test]
fn new_targets_reach_valid_and_rejected_production_paths() {
    use genos_parser_stress::exercise;

    let mut boot_map = [0u8; 44];
    boot_map[..4].copy_from_slice(&[40, 0, 1, 0]);
    boot_map[4..8].copy_from_slice(&7u32.to_le_bytes());
    boot_map[12..20].copy_from_slice(&4096u64.to_le_bytes());
    boot_map[28..36].copy_from_slice(&1u64.to_le_bytes());
    assert!(exercise("boot-map", &boot_map));
    assert!(!exercise("boot-map", &boot_map[..43]));

    assert!(exercise("paths", b"/USER/NOTE.TXT"));
    assert!(exercise("paths", b"/USER/../NOTE.TXT"));
    assert!(!exercise("paths", b"\xff"));

    let one_rx = [0, 0, 1, 0, 0, 0, 0, 0, 16, 0, 0, 0];
    assert!(exercise("descriptors", &one_rx));
    let bad_rx = [0, 0, 1, 0, 8, 0, 0, 0, 16, 0, 0, 0];
    assert!(exercise("descriptors", &bad_rx));
}
