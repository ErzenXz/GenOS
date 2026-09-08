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
        String::from_utf8_lossy(&output.stdout).contains("corpus=12 truncations=693 mutations=0")
    );
}

#[test]
fn replay_uses_production_acceptance_and_rejects_invalid_cli() {
    for (fixture, expectation) in [("elf-valid.bin", "accept"), ("elf-overflow.bin", "reject")] {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("corpus")
            .join(fixture);
        let result = command()
            .args(["--target", "elf", "--expect", expectation, "--input"])
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
