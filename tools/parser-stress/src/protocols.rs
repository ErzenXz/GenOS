//! Production wire parsers: accepted extents and exact peer/transaction identity.
use kernel::{dhcp, net};

pub fn dhcp(bytes: &[u8]) -> bool {
    let Some(config) = dhcp::parse_dhcp(bytes, 0x4745_4e4f, 5) else {
        return false;
    };
    assert_eq!(bytes[0], 2);
    assert_eq!(&bytes[4..8], &0x4745_4e4fu32.to_be_bytes());
    assert_eq!(&bytes[16..20], &config.address);
    for address in [config.address, config.gateway, config.dns, config.server] {
        assert_ne!(address, [0; 4]);
    }
    assert!(dhcp::parse_dhcp(bytes, 0x4745_4e50, 5).is_none());
    let mut request = [0; 300];
    let length = dhcp::build_dhcp(
        &mut request,
        [1, 2, 3, 4, 5, 6],
        0x4745_4e4f,
        3,
        Some(config.address),
        Some(config.server),
    )
    .unwrap();
    assert!((241..=300).contains(&length));
    assert_eq!(request[length - 1], 255);
    assert!(dhcp::parse_dhcp(&request[..length], 0x4745_4e4f, 5).is_none());
    true
}

pub fn dns(bytes: &[u8]) -> bool {
    let Some(address) = genos_shell::dns::first_a(bytes) else {
        return false;
    };
    assert_eq!(&bytes[..2], &[0x47, 0x45]);
    assert_ne!(bytes[2] & 0x80, 0);
    assert_eq!(bytes[2] & 0x7a, 0);
    assert_eq!(bytes[3] & 0x0f, 0);
    assert_eq!(&bytes[4..6], &[0, 1]);
    assert_ne!(u16::from_be_bytes([bytes[6], bytes[7]]), 0);
    assert!(bytes
        .windows(4)
        .any(|window| window == address.to_be_bytes()));
    let mut wrong_id = bytes.to_vec();
    wrong_id[0] ^= 1;
    assert!(genos_shell::dns::first_a(&wrong_id).is_none());
    true
}

pub fn arp(bytes: &[u8]) -> bool {
    let Some(mac) = net::parse_arp_reply(bytes, [10, 0, 2, 2], [10, 0, 2, 15]) else {
        return false;
    };
    assert!(bytes.len() >= 42);
    assert_eq!(&bytes[22..28], &mac);
    assert_eq!(&bytes[28..32], &[10, 0, 2, 2]);
    assert_eq!(&bytes[38..42], &[10, 0, 2, 15]);
    assert!(net::parse_arp_reply(bytes, [10, 0, 2, 3], [10, 0, 2, 15]).is_none());
    assert!(net::parse_arp_reply(bytes, [10, 0, 2, 2], [10, 0, 2, 16]).is_none());
    true
}

pub fn icmp(bytes: &[u8]) -> bool {
    let accepted = net::is_icmp_echo_reply(bytes, [0x47, 0x45, 0, 1]);
    if accepted {
        assert!(bytes.len() >= 8);
        assert_eq!(bytes[0], 0);
        assert_eq!(&bytes[4..8], &[0x47, 0x45, 0, 1]);
        // Independent one's-complement sum, including odd-length padding.
        let mut sum: u64 = bytes
            .chunks(2)
            .map(|pair| (u64::from(pair[0]) << 8) | u64::from(*pair.get(1).unwrap_or(&0)))
            .sum();
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        assert_eq!(sum, 0xffff);
        assert!(!net::is_icmp_echo_reply(bytes, [0x47, 0x45, 0, 2]));
    }
    accepted
}
