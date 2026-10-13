//! Deterministic host stress harness against the production kernel library.
//! This is bounded mutation testing, not a coverage-guided fuzzer or safety proof.
use genos_parser_stress::exercise;
use std::{env, fs, path::PathBuf, process};

const DEFAULT_SEED: u64 = 0x0047_454e_4f53;
const MAX_INPUT: usize = 65_536;
const TARGETS: &[&str] = &[
    "all",
    "elf",
    "native-elf",
    "ipv4",
    "ipv6",
    "udp",
    "tcp",
    "socket",
    "boot-map",
    "paths",
    "descriptors",
];

struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        // SplitMix64 has a defined wrapping sequence, including when seeded zero.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
    fn index(&mut self, len: usize) -> usize {
        (self.next() % len as u64) as usize
    }
}

fn run_case(target: &str, bytes: &[u8], description: &str, expected: Option<bool>) {
    let result = std::panic::catch_unwind(|| {
        let accepted = exercise(target, bytes);
        if let Some(expected) = expected {
            assert_eq!(accepted, expected, "corpus acceptance changed");
        }
    });
    if result.is_err() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/failures");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = directory.join(format!("parser-{target}-{}-{timestamp}.bin", process::id()));
        if let Err(error) = fs::create_dir_all(&directory).and_then(|()| fs::write(&path, bytes)) {
            eprintln!("Could not retain failure input: {error}");
        }
        eprintln!(
            "FAILED {description}; target={target}; input_len={}",
            bytes.len()
        );
        let expectation = expected
            .map(|value| {
                if value {
                    " --expect accept"
                } else {
                    " --expect reject"
                }
            })
            .unwrap_or("");
        let command = format!("cargo run --manifest-path tools/parser-stress/Cargo.toml --release --offline -- --target {target} --input {}{expectation}", path.display());
        let _ = fs::write(
            path.with_extension("txt"),
            format!("{description}\n{command}\n"),
        );
        eprintln!("Replay: {command}");
        process::exit(1);
    }
}

fn mutate(random: &mut Random, original: &[u8]) -> Vec<u8> {
    let mut bytes = original.to_vec();
    match random.index(7) {
        0 => bytes.truncate(random.index(bytes.len() + 1)),
        1 => {
            let length = random.index(2049);
            bytes.resize(length, 0);
            for byte in &mut bytes {
                *byte = random.next() as u8;
            }
        }
        2 => {
            if !bytes.is_empty() {
                let index = random.index(bytes.len());
                bytes[index] ^= 1 << random.index(8);
            }
        }
        3 => {
            if !bytes.is_empty() {
                let start = random.index(bytes.len());
                let end = (start + random.index(16) + 1).min(bytes.len());
                bytes[start..end].fill(if random.next() & 1 == 0 { 0 } else { 255 });
            }
        }
        4 => {
            for _ in 0..random.index(8) + 1 {
                let index = random.index(bytes.len() + 1);
                bytes.insert(index, random.next() as u8);
            }
        }
        5 => {
            for _ in 0..random.index(8) + 1 {
                if !bytes.is_empty() {
                    let index = random.index(bytes.len());
                    bytes[index] = random.next() as u8;
                }
            }
        }
        _ => bytes.resize(
            [
                0, 1, 7, 8, 19, 20, 33, 34, 53, 54, 63, 64, 119, 120, 128, 1518, 1519, MAX_INPUT,
            ][random.index(18)],
            255,
        ),
    }
    bytes
}

fn number(value: &str) -> Result<u64, String> {
    if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse()
    }
    .map_err(|_| format!("invalid unsigned number: {value}"))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("parser-stress: {error}");
        process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut seed = DEFAULT_SEED;
    let mut iterations = 25_000u64;
    let mut target = "all".to_owned();
    let mut input = None;
    let mut expected = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" {
            println!(
                "parser-stress [--seed N|0xHEX] [--iterations N] [--target {}] [--input RAW_FILE] [--expect accept|reject]",
                TARGETS.join("|")
            );
            return Ok(());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--seed" => seed = number(&value)?,
            "--iterations" => iterations = number(&value)?,
            "--target" => target = value,
            "--input" => input = Some(PathBuf::from(value)),
            "--expect" => {
                expected = Some(match value.as_str() {
                    "accept" => true,
                    "reject" => false,
                    _ => return Err("--expect must be accept or reject".into()),
                })
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    if !TARGETS.contains(&target.as_str()) {
        return Err(format!("unknown target: {target}"));
    }
    if iterations > 10_000_000 {
        return Err("--iterations must be at most 10000000".into());
    }
    if expected.is_some()
        && (input.is_none()
            || target == "all"
            || matches!(target.as_str(), "socket" | "descriptors"))
    {
        return Err(
            "--expect requires --input and a parser target (elf, native-elf, ipv4, ipv6, udp, tcp)"
                .into(),
        );
    }
    if let Some(input) = input {
        let metadata = fs::metadata(&input).map_err(|error| error.to_string())?;
        if metadata.len() > MAX_INPUT as u64 {
            return Err("replay input exceeds 65536 bytes".into());
        }
        let bytes = fs::read(&input).map_err(|error| error.to_string())?;
        run_case(&target, &bytes, "replay input", expected);
        println!("PASS replay target={target} bytes={}", bytes.len());
        return Ok(());
    }
    let corpus: &[(&str, &str, bool, &[u8])] = &[
        (
            "native-elf-valid",
            "native-elf",
            true,
            include_bytes!("../corpus/native-elf-valid.bin"),
        ),
        (
            "native-elf-late-wx",
            "native-elf",
            false,
            include_bytes!("../corpus/native-elf-late-wx.bin"),
        ),
        (
            "elf-valid",
            "elf",
            true,
            include_bytes!("../corpus/elf-valid.bin"),
        ),
        (
            "elf-overflow",
            "elf",
            false,
            include_bytes!("../corpus/elf-overflow.bin"),
        ),
        (
            "elf-truncated",
            "elf",
            false,
            include_bytes!("../corpus/elf-truncated.bin"),
        ),
        (
            "ipv4-udp-valid",
            "ipv4",
            true,
            include_bytes!("../corpus/ipv4-udp-valid.bin"),
        ),
        (
            "ipv4-fragment",
            "ipv4",
            false,
            include_bytes!("../corpus/ipv4-fragment.bin"),
        ),
        (
            "ipv6-udp-valid",
            "ipv6",
            true,
            include_bytes!("../corpus/ipv6-udp-valid.bin"),
        ),
        (
            "ipv6-options-truncated",
            "ipv6",
            false,
            include_bytes!("../corpus/ipv6-options-truncated.bin"),
        ),
        (
            "udp-valid",
            "udp",
            true,
            include_bytes!("../corpus/udp-valid.bin"),
        ),
        (
            "tcp-syn-valid",
            "tcp",
            true,
            include_bytes!("../corpus/tcp-syn-valid.bin"),
        ),
        (
            "tcp-offset-invalid",
            "tcp",
            false,
            include_bytes!("../corpus/tcp-offset-invalid.bin"),
        ),
        (
            "socket-zero-slot",
            "socket",
            true,
            include_bytes!("../corpus/socket-zero-slot.bin"),
        ),
        (
            "socket-lifecycle",
            "socket",
            true,
            include_bytes!("../corpus/socket-lifecycle.bin"),
        ),
    ];
    let mut truncations = 0;
    for (name, corpus_target, expected, bytes) in corpus {
        run_case(corpus_target, bytes, name, Some(*expected));
        // Every prefix is exercised, including headers cut on every byte boundary.
        for length in 0..bytes.len() {
            run_case(corpus_target, &bytes[..length], name, None);
            truncations += 1;
        }
    }
    let mut random = Random(seed);
    for iteration in 0..iterations {
        let (_, _, _, original) = corpus[random.index(corpus.len())];
        let bytes = mutate(&mut random, original);
        run_case(
            &target,
            &bytes,
            &format!("seed={seed:#x} iteration={iteration}"),
            None,
        );
    }
    println!("PASS parser stress: seed={seed:#x} target={target} corpus={} truncations={truncations} mutations={iterations}", corpus.len());
    Ok(())
}
