//! Deterministic host stress harness against the production kernel library.
//! This is bounded mutation testing, not a coverage-guided fuzzer or safety proof.
use kernel::{elf::ElfImage, ipv6, net, socket};
use std::{env, fs, path::PathBuf, process};

const DEFAULT_SEED: u64 = 0x0047_454e_4f53;
const MAX_INPUT: usize = 65_536;
const TARGETS: &[&str] = &["all", "elf", "ipv4", "ipv6", "udp", "tcp", "socket"];

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

fn subslice(parent: &[u8], child: &[u8]) {
    let start = parent.as_ptr() as usize;
    let end = start.checked_add(parent.len()).unwrap();
    let child_start = child.as_ptr() as usize;
    assert!(child_start >= start);
    assert!(child_start.checked_add(child.len()).unwrap() <= end);
}

fn check_elf(bytes: &[u8]) -> bool {
    let Ok(image) = ElfImage::parse(bytes) else {
        return false;
    };
    assert_eq!(image.byte_len(), bytes.len());
    assert_ne!(image.entry(), 0);
    let mut count = 0;
    for result in image.segments() {
        let segment = result.expect("parse accepted an invalid load segment");
        subslice(bytes, segment.file_data);
        assert!(segment.memory_size > 0);
        assert!(segment.memory_size >= segment.file_data.len() as u64);
        assert!(segment.align.is_power_of_two());
        assert!(segment.file_offset <= bytes.len() as u64);
        count += 1;
        assert!(count <= 16);
    }
    assert!(count > 0);
    true
}

fn check_udp(bytes: &[u8]) -> bool {
    let Some(packet) = net::parse_udp(bytes) else {
        return false;
    };
    subslice(bytes, packet.payload);
    assert!(packet.payload.len() + net::UDP_HEADER_BYTES <= bytes.len());
    true
}

fn check_tcp(bytes: &[u8]) -> bool {
    let Some(packet) = net::parse_tcp(bytes) else {
        return false;
    };
    subslice(bytes, packet.payload);
    assert!(packet.payload.len() + net::TCP_HEADER_BYTES <= bytes.len());
    if net::is_initial_tcp_syn(&packet) {
        assert!(packet.payload.is_empty());
        assert_ne!(packet.source_port, 0);
        assert_ne!(packet.destination_port, 0);
    }
    // Exercise both matching and mismatching sequences, including wraparound.
    for (remote, local) in [
        (0, 0),
        (u32::MAX, u32::MAX),
        (
            packet.sequence.wrapping_sub(1),
            packet.acknowledgment.wrapping_sub(1),
        ),
        (packet.sequence, packet.acknowledgment),
    ] {
        if net::is_tcp_handshake_ack(&packet, remote, local) {
            assert!(packet.payload.is_empty());
            assert_eq!(packet.sequence, remote.wrapping_add(1));
            assert_eq!(packet.acknowledgment, local.wrapping_add(1));
        }
    }
    true
}

fn check_ipv4(bytes: &[u8]) -> bool {
    let _ = net::parse_arp_reply(bytes, [10, 0, 2, 2], [10, 0, 2, 15]);
    let Some(packet) = net::parse_ipv4_frame(bytes) else {
        return false;
    };
    subslice(bytes, packet.payload);
    assert!(
        packet.payload.len() + net::ETHERNET_HEADER_BYTES + net::IPV4_HEADER_BYTES <= bytes.len()
    );
    let _ = net::transport_checksum_valid(
        packet.source,
        packet.destination,
        packet.protocol,
        packet.payload,
    );
    if packet.protocol == 17 {
        check_udp(packet.payload);
    }
    if packet.protocol == 6 {
        check_tcp(packet.payload);
    }
    true
}

fn check_ipv6(bytes: &[u8]) -> bool {
    let Some(packet) = ipv6::parse_frame(bytes) else {
        return false;
    };
    subslice(bytes, packet.payload);
    assert!(bytes.len() <= ipv6::FRAME_CAPACITY);
    assert_ne!(packet.hop_limit, 0);
    assert!(!ipv6::is_multicast(packet.source));
    assert_ne!(packet.destination, ipv6::UNSPECIFIED);
    let _ = ipv6::transport_checksum(
        packet.source,
        packet.destination,
        packet.next_header,
        packet.payload,
    );
    if packet.next_header == 17 {
        check_udp(packet.payload);
    }
    if packet.next_header == 6 {
        check_tcp(packet.payload);
    }
    true
}

fn check_socket(bytes: &[u8]) -> bool {
    use socket::{SocketOwner, SocketProtocol, SocketSet};
    let owners = [
        SocketOwner {
            slot: 0,
            incarnation: 1,
        },
        SocketOwner {
            slot: 1,
            incarnation: 2,
        },
    ];
    let mut sockets = SocketSet::new();
    let mut remembered = [0u64; 16];
    let mut next = 0;
    // Socket commands are four-byte records; partial trailing commands are ignored.
    for command in bytes.chunks_exact(4).take(64) {
        let owner_index = usize::from(command[1] & 1);
        let owner = owners[owner_index];
        let index = usize::from(command[2] & 15);
        let handle = if command[1] & 2 == 0 {
            remembered[index]
        } else {
            u64::from_le_bytes([
                command[0], command[1], command[2], command[3], 0, 0, 0, 0xe7,
            ])
        };
        let size = usize::from(command[3]);
        let data = [command[2]; 255];
        let mut output = [0u8; 255];
        let action = command[0] % 13;
        let rejected_handle = sockets.status(owner, handle).is_err();
        let before = socket_snapshot(&sockets, &owners);
        match action {
            0 => {
                let protocol = if command[3] & 1 == 0 {
                    SocketProtocol::Udp
                } else {
                    SocketProtocol::TcpStream
                };
                if let Ok(opened) = sockets.open(owner, protocol) {
                    remembered[next % remembered.len()] = opened;
                    next += 1;
                    assert!(sockets.status(owners[owner_index ^ 1], opened).is_err());
                }
            }
            1 => {
                let _ =
                    sockets.connect(owner, handle, u32::from(command[2]), u16::from(command[3]));
            }
            2 => {
                let _ = sockets.mark_connected(owner, handle);
            }
            3 => {
                let _ = sockets.bind(owner, handle, u16::from_be_bytes([command[2], command[3]]));
            }
            4 => {
                let _ = sockets.listen(owner, handle, size);
            }
            5 => {
                if let Ok(count) = sockets.send(owner, handle, &data[..size]) {
                    assert!(count <= size);
                }
            }
            6 => {
                if let Ok(count) = sockets.receive(owner, handle, &mut output[..size]) {
                    assert!(count <= size);
                }
            }
            7 => {
                if let Ok(count) = sockets.push_receive(owner, handle, &data[..size]) {
                    assert!(count <= size);
                }
            }
            8 => {
                let _ = sockets.shutdown(owner, handle, command[3] & 1 != 0, command[3] & 2 != 0);
            }
            9 => {
                if sockets.close(owner, handle).is_ok() {
                    assert!(sockets.status(owner, handle).is_err());
                }
            }
            10 => {
                let previous = sockets.len_owner(owner);
                assert_eq!(sockets.close_owner(owner), previous);
                assert_eq!(sockets.len_owner(owner), 0);
            }
            11 => {
                if let Ok(count) = sockets.take_send(owner, handle, &mut output[..size]) {
                    assert!(count <= size);
                }
            }
            _ => {
                let _ = SocketProtocol::from_raw(u64::from_be_bytes([
                    0, 0, 0, 0, command[0], command[1], command[2], command[3],
                ]));
            }
        }
        if rejected_handle && !matches!(action, 0 | 10 | 12) {
            assert_eq!(
                socket_snapshot(&sockets, &owners),
                before,
                "rejected handle changed socket state"
            );
        }
        assert!(
            sockets.len_owner(owners[0]) + sockets.len_owner(owners[1]) <= socket::SOCKET_CAPACITY
        );
        for (owner_index, owner) in owners.iter().copied().enumerate() {
            for live in sockets.handles(owner) {
                let status = sockets.status(owner, live).unwrap();
                assert!(status.queued_send <= socket::SOCKET_BUFFER_CAPACITY);
                assert!(status.queued_receive <= socket::SOCKET_BUFFER_CAPACITY);
                assert!(sockets.status(owners[owner_index ^ 1], live).is_err());
                assert!(sockets
                    .status(
                        SocketOwner {
                            incarnation: owner.incarnation + 1,
                            ..owner
                        },
                        live
                    )
                    .is_err());
            }
        }
    }
    for owner in owners {
        let expected = sockets.len_owner(owner);
        assert_eq!(sockets.close_owner(owner), expected);
        assert_eq!(sockets.len_owner(owner), 0);
        for handle in remembered {
            assert!(sockets.status(owner, handle).is_err());
        }
    }
    true
}

fn socket_snapshot(
    sockets: &socket::SocketSet,
    owners: &[socket::SocketOwner; 2],
) -> Vec<(u64, socket::SocketStatus)> {
    owners
        .iter()
        .flat_map(|&owner| {
            sockets
                .handles(owner)
                .map(move |handle| (handle, sockets.status(owner, handle).unwrap()))
        })
        .collect()
}

fn exercise(target: &str, bytes: &[u8]) -> bool {
    match target {
        "elf" => check_elf(bytes),
        "ipv4" => check_ipv4(bytes),
        "ipv6" => check_ipv6(bytes),
        "udp" => check_udp(bytes),
        "tcp" => check_tcp(bytes),
        "socket" => check_socket(bytes),
        "all" => {
            check_elf(bytes);
            check_ipv4(bytes);
            check_ipv6(bytes);
            check_udp(bytes);
            check_tcp(bytes);
            check_socket(bytes);
            true
        }
        _ => unreachable!(),
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
    if expected.is_some() && (input.is_none() || target == "all" || target == "socket") {
        return Err(
            "--expect requires --input and a parser target (elf, ipv4, ipv6, udp, tcp)".into(),
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
