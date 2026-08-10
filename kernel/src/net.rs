pub const ETHERNET_HEADER_BYTES: usize = 14;
pub const IPV4_HEADER_BYTES: usize = 20;
pub const UDP_HEADER_BYTES: usize = 8;
pub const TCP_HEADER_BYTES: usize = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4Packet<'a> {
    pub source_mac: [u8; 6],
    pub source: [u8; 4],
    pub destination: [u8; 4],
    pub protocol: u8,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UdpPacket<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcpPacket<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    pub sequence: u32,
    pub acknowledgment: u32,
    pub flags: u8,
    pub payload: &'a [u8],
}

pub fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut chunks = bytes.chunks_exact(2);
    for chunk in &mut chunks {
        sum = sum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]) as u32);
    }
    if let Some(byte) = chunks.remainder().first() {
        sum = sum.wrapping_add(u16::from_be_bytes([*byte, 0]) as u32);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn transport_checksum_valid(
    source: [u8; 4],
    destination: [u8; 4],
    protocol: u8,
    segment: &[u8],
) -> bool {
    if segment.len() > u16::MAX as usize {
        return false;
    }
    if protocol == 17 && segment.get(6..8) == Some(&[0, 0]) {
        return true;
    }
    let mut sum = 0u32;
    for bytes in [source.as_slice(), destination.as_slice()] {
        for chunk in bytes.chunks_exact(2) {
            sum = sum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]) as u32);
        }
    }
    sum = sum.wrapping_add(protocol as u32);
    sum = sum.wrapping_add(segment.len() as u32);
    let mut chunks = segment.chunks_exact(2);
    for chunk in &mut chunks {
        sum = sum.wrapping_add(u16::from_be_bytes([chunk[0], chunk[1]]) as u32);
    }
    if let Some(byte) = chunks.remainder().first() {
        sum = sum.wrapping_add(u16::from_be_bytes([*byte, 0]) as u32);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    sum as u16 == u16::MAX
}

pub fn parse_ipv4_frame(frame: &[u8]) -> Option<Ipv4Packet<'_>> {
    if frame.len() < ETHERNET_HEADER_BYTES + IPV4_HEADER_BYTES
        || u16::from_be_bytes([frame[12], frame[13]]) != 0x0800
    {
        return None;
    }
    let ip = &frame[ETHERNET_HEADER_BYTES..];
    if ip[0] >> 4 != 4 {
        return None;
    }
    let header_len = usize::from(ip[0] & 0x0f).checked_mul(4)?;
    let total_len = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
    if header_len < IPV4_HEADER_BYTES
        || header_len > ip.len()
        || total_len < header_len
        || total_len > ip.len()
        || checksum(&ip[..header_len]) != 0
    {
        return None;
    }
    let fragment = u16::from_be_bytes([ip[6], ip[7]]);
    if fragment & 0x3fff != 0 {
        return None;
    }
    Some(Ipv4Packet {
        source_mac: frame[6..12].try_into().ok()?,
        source: ip[12..16].try_into().ok()?,
        destination: ip[16..20].try_into().ok()?,
        protocol: ip[9],
        payload: &ip[header_len..total_len],
    })
}

pub fn parse_arp_reply(frame: &[u8], sender: [u8; 4], target: [u8; 4]) -> Option<[u8; 6]> {
    (frame.len() >= 42
        && frame[12..14] == 0x0806u16.to_be_bytes()
        && frame[14..16] == 1u16.to_be_bytes()
        && frame[16..18] == 0x0800u16.to_be_bytes()
        && frame[18] == 6
        && frame[19] == 4
        && frame[20..22] == 2u16.to_be_bytes()
        && frame[28..32] == sender
        && frame[38..42] == target)
        .then(|| frame[22..28].try_into().ok())
        .flatten()
}

pub fn parse_udp(payload: &[u8]) -> Option<UdpPacket<'_>> {
    if payload.len() < UDP_HEADER_BYTES {
        return None;
    }
    let len = usize::from(u16::from_be_bytes([payload[4], payload[5]]));
    if len < UDP_HEADER_BYTES || len > payload.len() {
        return None;
    }
    Some(UdpPacket {
        source_port: u16::from_be_bytes([payload[0], payload[1]]),
        destination_port: u16::from_be_bytes([payload[2], payload[3]]),
        payload: &payload[UDP_HEADER_BYTES..len],
    })
}

pub fn parse_tcp(payload: &[u8]) -> Option<TcpPacket<'_>> {
    if payload.len() < TCP_HEADER_BYTES {
        return None;
    }
    let header_len = usize::from(payload[12] >> 4).checked_mul(4)?;
    if header_len < TCP_HEADER_BYTES || header_len > payload.len() {
        return None;
    }
    Some(TcpPacket {
        source_port: u16::from_be_bytes([payload[0], payload[1]]),
        destination_port: u16::from_be_bytes([payload[2], payload[3]]),
        sequence: u32::from_be_bytes(payload[4..8].try_into().ok()?),
        acknowledgment: u32::from_be_bytes(payload[8..12].try_into().ok()?),
        flags: payload[13],
        payload: &payload[header_len..],
    })
}

pub fn is_initial_tcp_syn(packet: &TcpPacket<'_>) -> bool {
    packet.flags & 0x3f == 0x02
        && packet.source_port != 0
        && packet.destination_port != 0
        && packet.payload.is_empty()
}

pub fn is_tcp_handshake_ack(
    packet: &TcpPacket<'_>,
    remote_sequence: u32,
    local_sequence: u32,
) -> bool {
    packet.flags & 0x17 == 0x10
        && packet.sequence == remote_sequence.wrapping_add(1)
        && packet.acknowledgment == local_sequence.wrapping_add(1)
        && packet.payload.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_frame() -> [u8; ETHERNET_HEADER_BYTES + IPV4_HEADER_BYTES + UDP_HEADER_BYTES + 3] {
        let mut frame = [0u8; ETHERNET_HEADER_BYTES + IPV4_HEADER_BYTES + UDP_HEADER_BYTES + 3];
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        let ip = &mut frame[ETHERNET_HEADER_BYTES..];
        ip[0] = 0x45;
        ip[2..4]
            .copy_from_slice(&((IPV4_HEADER_BYTES + UDP_HEADER_BYTES + 3) as u16).to_be_bytes());
        ip[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
        ip[8] = 64;
        ip[9] = 17;
        ip[12..16].copy_from_slice(&[10, 0, 2, 15]);
        ip[16..20].copy_from_slice(&[10, 0, 2, 3]);
        let sum = checksum(&ip[..IPV4_HEADER_BYTES]);
        ip[10..12].copy_from_slice(&sum.to_be_bytes());
        let udp = &mut ip[IPV4_HEADER_BYTES..];
        udp[0..2].copy_from_slice(&49152u16.to_be_bytes());
        udp[2..4].copy_from_slice(&53u16.to_be_bytes());
        udp[4..6].copy_from_slice(&11u16.to_be_bytes());
        udp[8..].copy_from_slice(b"dns");
        frame
    }

    fn valid_tcp_syn_frame() -> [u8; ETHERNET_HEADER_BYTES + IPV4_HEADER_BYTES + TCP_HEADER_BYTES] {
        const SOURCE: [u8; 4] = [10, 0, 2, 2];
        const DESTINATION: [u8; 4] = [10, 0, 2, 15];
        let mut frame = [0u8; ETHERNET_HEADER_BYTES + IPV4_HEADER_BYTES + TCP_HEADER_BYTES];
        frame[6..12].copy_from_slice(&[0x52, 0x54, 0, 0x12, 0x34, 0x56]);
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        let ip = &mut frame[ETHERNET_HEADER_BYTES..];
        ip[0] = 0x45;
        ip[2..4].copy_from_slice(&((IPV4_HEADER_BYTES + TCP_HEADER_BYTES) as u16).to_be_bytes());
        ip[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
        ip[8] = 64;
        ip[9] = 6;
        ip[12..16].copy_from_slice(&SOURCE);
        ip[16..20].copy_from_slice(&DESTINATION);
        let sum = checksum(&ip[..IPV4_HEADER_BYTES]);
        ip[10..12].copy_from_slice(&sum.to_be_bytes());
        let tcp = &mut ip[IPV4_HEADER_BYTES..];
        tcp[0..2].copy_from_slice(&49152u16.to_be_bytes());
        tcp[2..4].copy_from_slice(&18081u16.to_be_bytes());
        tcp[4..8].copy_from_slice(&0x1020_3040u32.to_be_bytes());
        tcp[12] = 5 << 4;
        tcp[13] = 0x02;
        tcp[14..16].copy_from_slice(&4096u16.to_be_bytes());
        let mut pseudo = [0u8; 12 + TCP_HEADER_BYTES];
        pseudo[..4].copy_from_slice(&SOURCE);
        pseudo[4..8].copy_from_slice(&DESTINATION);
        pseudo[9] = 6;
        pseudo[10..12].copy_from_slice(&(TCP_HEADER_BYTES as u16).to_be_bytes());
        pseudo[12..].copy_from_slice(tcp);
        tcp[16..18].copy_from_slice(&checksum(&pseudo).to_be_bytes());
        frame
    }

    #[test]
    fn ipv4_udp_parser_accepts_one_bounded_packet() {
        let frame = valid_frame();
        let ip = parse_ipv4_frame(&frame).unwrap();
        let udp = parse_udp(ip.payload).unwrap();
        assert_eq!(udp.destination_port, 53);
        assert_eq!(udp.payload, b"dns");
    }

    #[test]
    fn malformed_packets_are_rejected_without_indexing_past_bounds() {
        let valid = valid_frame();
        for len in 0..valid.len() {
            let _ = parse_ipv4_frame(&valid[..len]);
        }
        let mut bad = valid;
        bad[ETHERNET_HEADER_BYTES] = 0x4f;
        assert!(parse_ipv4_frame(&bad).is_none());
        let mut bad = valid;
        bad[ETHERNET_HEADER_BYTES + 2..ETHERNET_HEADER_BYTES + 4]
            .copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(parse_ipv4_frame(&bad).is_none());
        assert!(parse_udp(&[]).is_none());
        assert!(parse_tcp(&[0; 19]).is_none());
        let mut tcp = [0u8; 20];
        tcp[12] = 0xf0;
        assert!(parse_tcp(&tcp).is_none());
        assert!(transport_checksum_valid(
            [10, 0, 2, 15],
            [10, 0, 2, 3],
            17,
            &valid[ETHERNET_HEADER_BYTES + IPV4_HEADER_BYTES..]
        ));
        assert!(!transport_checksum_valid(
            [10, 0, 2, 15],
            [10, 0, 2, 3],
            6,
            &[0; 20]
        ));
    }

    #[test]
    fn passive_tcp_handshake_requires_exact_bounded_packets() {
        let frame = valid_tcp_syn_frame();
        let ip = parse_ipv4_frame(&frame).unwrap();
        assert_eq!(ip.protocol, 6);
        assert!(transport_checksum_valid(
            ip.source,
            ip.destination,
            ip.protocol,
            ip.payload
        ));
        let syn = parse_tcp(ip.payload).unwrap();
        assert!(is_initial_tcp_syn(&syn));
        assert_eq!(syn.destination_port, 18081);
        for len in 0..frame.len() {
            assert!(parse_ipv4_frame(&frame[..len]).is_none());
        }

        let mut bad_checksum = frame;
        bad_checksum[ETHERNET_HEADER_BYTES + IPV4_HEADER_BYTES + 16] ^= 1;
        let ip = parse_ipv4_frame(&bad_checksum).unwrap();
        assert!(!transport_checksum_valid(
            ip.source,
            ip.destination,
            ip.protocol,
            ip.payload
        ));

        let mut ack = [0u8; TCP_HEADER_BYTES];
        ack[0..2].copy_from_slice(&49152u16.to_be_bytes());
        ack[2..4].copy_from_slice(&18081u16.to_be_bytes());
        ack[4..8].copy_from_slice(&0x1020_3041u32.to_be_bytes());
        ack[8..12].copy_from_slice(&0x5060_7081u32.to_be_bytes());
        ack[12] = 5 << 4;
        ack[13] = 0x10;
        let ack = parse_tcp(&ack).unwrap();
        assert!(is_tcp_handshake_ack(&ack, 0x1020_3040, 0x5060_7080));
        assert!(!is_initial_tcp_syn(&ack));
        assert!(!is_tcp_handshake_ack(&ack, 0x1020_3041, 0x5060_7080));
    }

    #[test]
    fn arp_reply_requires_the_exact_peer_and_rejects_every_truncation() {
        let sender = [10, 0, 2, 2];
        let target = [10, 0, 2, 15];
        let mac = [0x52, 0x54, 0, 0x12, 0x34, 0x56];
        let mut frame = [0u8; 42];
        frame[12..14].copy_from_slice(&0x0806u16.to_be_bytes());
        frame[14..16].copy_from_slice(&1u16.to_be_bytes());
        frame[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
        frame[18] = 6;
        frame[19] = 4;
        frame[20..22].copy_from_slice(&2u16.to_be_bytes());
        frame[22..28].copy_from_slice(&mac);
        frame[28..32].copy_from_slice(&sender);
        frame[38..42].copy_from_slice(&target);

        assert_eq!(parse_arp_reply(&frame, sender, target), Some(mac));
        for len in 0..frame.len() {
            assert!(parse_arp_reply(&frame[..len], sender, target).is_none());
        }
        assert!(parse_arp_reply(&frame, [10, 0, 2, 3], target).is_none());
        assert!(parse_arp_reply(&frame, sender, [10, 0, 2, 16]).is_none());

        frame[20..22].copy_from_slice(&1u16.to_be_bytes());
        assert!(parse_arp_reply(&frame, sender, target).is_none());
    }
}
