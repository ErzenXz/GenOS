//! Shared production invariant checks for deterministic and coverage-guided input.
use kernel::{elf::ElfImage, ipv6, native_image, net, socket};

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

fn check_native_elf(bytes: &[u8]) -> bool {
    let Ok(plan) = native_image::Plan::parse(bytes) else {
        return false;
    };
    let mut addresses = std::collections::BTreeSet::new();
    let mut executable_pages = 0;
    let mut data_pages = 0;
    let mut entry_mapped = false;
    for page in plan.pages() {
        subslice(bytes, page.file_bytes());
        assert!(page.file_bytes().len() <= 4096);
        assert!(page.address().is_multiple_of(4096));
        assert!((native_image::USER_CODE..native_image::USER_STACK_GUARD).contains(&page.address()));
        assert!(addresses.insert(page.address()));
        assert!(!(page.writable() && page.executable()));
        if page.executable() {
            executable_pages += 1;
            entry_mapped |= (page.address()..page.address() + 4096).contains(&plan.entry());
        }
        if page.process_data() {
            data_pages += 1;
            assert_eq!(page.address(), native_image::USER_DATA);
            assert!(page.writable());
        }
    }
    assert_eq!(addresses.len(), usize::from(plan.page_count()));
    assert!((1..=10).contains(&addresses.len()));
    assert!((1..=16).contains(&plan.segment_count()));
    assert!((1..=8).contains(&executable_pages));
    assert_eq!(data_pages, 1);
    assert!(entry_mapped);
    assert!((plan.executable_start()..plan.executable_end()).contains(&plan.entry()));
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

pub fn exercise(target: &str, bytes: &[u8]) -> bool {
    match target {
        "elf" => check_elf(bytes),
        "native-elf" => check_native_elf(bytes),
        "ipv4" => check_ipv4(bytes),
        "ipv6" => check_ipv6(bytes),
        "udp" => check_udp(bytes),
        "tcp" => check_tcp(bytes),
        "socket" => check_socket(bytes),
        "boot-map" => check_boot_map(bytes),
        "paths" => check_paths(bytes),
        "descriptors" => check_descriptors(bytes),
        "all" => {
            check_elf(bytes);
            check_native_elf(bytes);
            check_ipv4(bytes);
            check_ipv6(bytes);
            check_udp(bytes);
            check_tcp(bytes);
            check_socket(bytes);
            check_boot_map(bytes);
            check_paths(bytes);
            check_descriptors(bytes);
            true
        }
        _ => unreachable!(),
    }
}

fn check_boot_map(bytes: &[u8]) -> bool {
    use genos_abi::{boot_memory, MemoryRegionKind, MAX_MEMORY_REGIONS};
    let Some(header) = bytes.get(..4) else {
        return false;
    };
    let stride = usize::from(u16::from_le_bytes([header[0], header[1]]));
    let version = u32::from(u16::from_le_bytes([header[2], header[3]]));
    let raw = &bytes[4..];
    let Ok(map) = boot_memory::decode_uefi_map(raw, stride, version) else {
        return false;
    };
    let regions = boot_memory::validate_map(&map).expect("accepted boot map must validate");
    assert!((1..=MAX_MEMORY_REGIONS).contains(&regions.len()));
    assert_eq!(regions.len(), raw.len() / stride);
    for (i, region) in regions.iter().enumerate() {
        assert_ne!(region.size, 0);
        assert_eq!(region.start % 4096, 0);
        assert_eq!(region.size % 4096, 0);
        let end = region.start.checked_add(region.size).unwrap();
        for other in &regions[..i] {
            assert!(
                end <= other.start || other.start.checked_add(other.size).unwrap() <= region.start
            );
        }
        let descriptor = &raw[i * stride..][..40];
        let attributes = u64::from_le_bytes(descriptor[32..40].try_into().unwrap());
        if attributes & (1 << 63) != 0 {
            assert_eq!(region.kind, MemoryRegionKind::Reserved);
        }
        if region.kind == MemoryRegionKind::Usable {
            assert_eq!(u32::from_le_bytes(descriptor[..4].try_into().unwrap()), 7);
            assert_eq!(attributes & (1 << 63), 0);
        }
    }
    true
}

fn check_paths(bytes: &[u8]) -> bool {
    use genos_abi::USER_PATH_MAX;
    use kernel::{console_text::FixedText, path_policy as paths, vfs::RamVfs};
    let Ok(path) = std::str::from_utf8(bytes) else {
        return false;
    };
    let valid = paths::valid_absolute_path(path);
    if valid {
        assert!(path.len() <= USER_PATH_MAX && path.starts_with('/'));
        if path != "/" {
            for part in path[1..].split('/') {
                assert!(!part.is_empty() && !matches!(part, "." | ".."));
                assert!(part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)));
            }
        }
        if let Some((parent, name)) = path.rsplit_once('/') {
            if !name.is_empty() {
                let parent = if parent.is_empty() { "/" } else { parent };
                let joined =
                    paths::join_child_path(FixedText::from_str(parent), FixedText::from_str(name));
                assert_eq!(joined.unwrap().as_str(), path);
            }
        }
    } else {
        assert!(!paths::is_user_writable_path(path));
        let mut vfs = RamVfs::new();
        vfs.init_root();
        let before = vfs.count();
        assert!(vfs.touch(path).is_err());
        assert_eq!(vfs.count(), before);
    }
    if paths::is_user_writable_path(path) {
        assert!(valid && path.len() > 6);
        assert!(path.as_bytes()[..6].eq_ignore_ascii_case(b"/USER/"));
    }
    true
}

fn check_descriptors(bytes: &[u8]) -> bool {
    use kernel::rx_queue::{RxBatch, RxCompletion, RX_BUFFER_BYTES, RX_HEADER_BYTES};
    use kernel::tx_queue::{TxError, TxQueue, TX_RING_SIZE};
    if bytes.len() >= 4 {
        let used = u16::from_le_bytes(bytes[..2].try_into().unwrap());
        let end = u16::from_le_bytes(bytes[2..4].try_into().unwrap());
        let entries: Vec<_> = bytes[4..]
            .chunks_exact(8)
            .take(8)
            .map(|raw| RxCompletion {
                descriptor: u32::from_le_bytes(raw[..4].try_into().unwrap()),
                length: u32::from_le_bytes(raw[4..].try_into().unwrap()),
            })
            .collect();
        let mut batch = RxBatch::<8>::new(used);
        if batch.admit(end, &entries).is_ok() {
            assert_eq!(usize::from(end.wrapping_sub(used)), entries.len());
            let mut seen = std::collections::BTreeSet::new();
            for (index, &entry) in entries.iter().enumerate() {
                assert!(entry.descriptor < 8 && seen.insert(entry.descriptor));
                assert!((RX_HEADER_BYTES..=RX_BUFFER_BYTES).contains(&(entry.length as usize)));
                assert_eq!(batch.take(), Some(entry));
                assert_eq!(batch.used_index(), used.wrapping_add(index as u16 + 1));
            }
            assert_eq!(batch.used_index(), end);
            assert!(!batch.pending());
            assert_eq!(batch.take(), None);
        } else {
            assert!(batch.is_quarantined());
            assert_eq!(batch.used_index(), used);
            assert_eq!(batch.take(), None);
            assert!(batch.admit(used, &[]).is_err());
        }
    }
    // A separate scalar model follows the single-buffer TX ownership contract.
    // At most 64 commands; no I/O, DMA memory or unbounded queue is constructed.
    let mut tx = TxQueue::new();
    let (mut device, mut quarantined, mut available, mut used) = (false, false, 0u16, 0u16);
    for raw in bytes.chunks_exact(5).take(64) {
        match raw[0] % 3 {
            0 => {
                let result = tx.begin();
                if quarantined {
                    assert!(matches!(result, Err(TxError::Quarantined)));
                } else if device {
                    assert!(matches!(result, Err(TxError::Busy)));
                } else {
                    let publication = result.unwrap();
                    assert_eq!(publication.ring_slot, usize::from(available) % TX_RING_SIZE);
                    available = available.wrapping_add(1);
                    assert_eq!(publication.available_index, available);
                    device = true;
                }
            }
            1 => {
                let reported = u16::from_le_bytes([raw[1], raw[2]]);
                let descriptor = u32::from(u16::from_le_bytes([raw[3], raw[4]]));
                let expected =
                    !quarantined && device && reported.wrapping_sub(used) == 1 && descriptor == 0;
                assert_eq!(tx.complete(reported, descriptor).is_ok(), expected);
                if expected {
                    used = reported;
                    device = false;
                } else {
                    quarantined = true;
                }
            }
            _ => {
                tx.quarantine();
                quarantined = true;
            }
        }
        assert_eq!(tx.used_index(), used);
        assert_eq!(tx.is_quarantined(), quarantined);
    }
    true
}
