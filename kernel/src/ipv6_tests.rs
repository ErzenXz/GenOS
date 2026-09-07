use super::*;
use std::{vec, vec::Vec};

const MAC: [u8; 6] = [0x52, 0x54, 0, 0x12, 0x34, 0x56];
const ROUTER_MAC: [u8; 6] = [0x52, 0x55, 0, 0, 0, 2];
const ROUTER: Address = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];

fn router_message() -> Vec<u8> {
    let mut body = vec![0; 56];
    body[0] = 134;
    body[4] = 64;
    body[6..8].copy_from_slice(&1800u16.to_be_bytes());
    body[16..18].copy_from_slice(&[1, 1]);
    body[18..24].copy_from_slice(&ROUTER_MAC);
    body[24..28].copy_from_slice(&[3, 4, 64, 0xc0]);
    body[28..32].copy_from_slice(&3600u32.to_be_bytes());
    body[32..36].copy_from_slice(&1800u32.to_be_bytes());
    body[40..44].copy_from_slice(&[0xfd, 0x42, 0x12, 0x34]);
    body
}

fn wire(source: Address, destination: Address, hop: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; FRAME_CAPACITY];
    let target = if is_multicast(destination) {
        multicast_mac(destination)
    } else {
        MAC
    };
    let len = write_icmp(
        &mut bytes,
        ROUTER_MAC,
        target,
        source,
        destination,
        hop,
        body,
    )
    .unwrap();
    bytes.truncate(len);
    bytes
}

fn route(bytes: &[u8]) -> Option<Route> {
    router_advertisement(&parse_frame(bytes)?, link_local(MAC), 0)
}

fn configured() -> Host {
    let mut host = Host::new(MAC, 0);
    let mut output = [0; FRAME_CAPACITY];
    host.poll(0, &mut output).unwrap();
    host.poll(100, &mut output).unwrap();
    host.receive(
        &wire(ROUTER, ALL_NODES, 255, &router_message()),
        101,
        &mut output,
    );
    assert_eq!(host.state, State::GlobalDad);
    host.poll(101, &mut output).unwrap();
    host.poll(201, &mut output).unwrap();
    assert_eq!(host.state, State::Ready);
    host
}

#[test]
fn slaac_uses_advertised_prefix_and_probes_both_addresses_before_use() {
    let mut host = Host::new(MAC, 50);
    let mut output = [0; FRAME_CAPACITY];
    let len = host.poll(50, &mut output).unwrap();
    let packet = parse_frame(&output[..len]).unwrap();
    assert_eq!(packet.source, UNSPECIFIED);
    assert_eq!(packet.destination, solicited_node(host.link_local));
    assert_eq!(
        packet.payload.len(),
        24,
        "DAD must not include source link-layer option"
    );
    assert!(valid_transport(&packet));
    assert!(host.poll(149, &mut output).is_none());
    let len = host.poll(150, &mut output).unwrap();
    assert_eq!(parse_frame(&output[..len]).unwrap().payload[0], 133);
    host.receive(
        &wire(ROUTER, ALL_NODES, 255, &router_message()),
        151,
        &mut output,
    );
    assert_eq!(host.state, State::GlobalDad);
    assert_eq!(
        host.route.unwrap().address[..8],
        [0xfd, 0x42, 0x12, 0x34, 0, 0, 0, 0]
    );
    let len = host.poll(151, &mut output).unwrap();
    assert_eq!(parse_frame(&output[..len]).unwrap().payload[0], 135);
    assert!(host.poll(250, &mut output).is_none());
    let len = host.poll(251, &mut output).unwrap();
    let echo = parse_frame(&output[..len]).unwrap();
    assert_eq!(echo.payload[0], 128);
    assert_eq!(echo.source, host.route.unwrap().address);
    assert!(valid_transport(&echo));
}

#[test]
fn every_truncation_and_checksum_corruption_fails_closed() {
    let frame = wire(ROUTER, ALL_NODES, 255, &router_message());
    assert!(route(&frame).is_some());
    for len in 0..frame.len() {
        assert!(route(&frame[..len]).is_none(), "len={len}");
    }
    for pos in 54..frame.len() {
        let mut corrupt = frame.clone();
        corrupt[pos] ^= 1;
        assert!(route(&corrupt).is_none(), "byte={pos}");
    }
}

#[test]
fn advertisement_requires_link_local_router_exact_hop_limit_and_valid_lifetimes() {
    assert!(route(&wire(ROUTER, ALL_NODES, 254, &router_message())).is_none());
    let mut global = ROUTER;
    global[0] = 0x20;
    assert!(route(&wire(global, ALL_NODES, 255, &router_message())).is_none());
    let mut body = router_message();
    body[6..8].fill(0);
    assert!(route(&wire(ROUTER, ALL_NODES, 255, &body)).is_none());
    let mut body = router_message();
    body[32..36].copy_from_slice(&7200u32.to_be_bytes());
    assert!(route(&wire(ROUTER, ALL_NODES, 255, &body)).is_none());
    let mut body = router_message();
    body[25] = 0;
    assert!(route(&wire(ROUTER, ALL_NODES, 255, &body)).is_none());
    let mut body = router_message();
    body[18] ^= 2;
    assert!(route(&wire(ROUTER, ALL_NODES, 255, &body)).is_none());
}

#[test]
fn malformed_extensions_fragments_routing_and_jumbograms_are_rejected() {
    let frame = wire(ROUTER, ALL_NODES, 255, &router_message());
    for next in [43, 44, 50, 51, 59, 253] {
        let mut bad = frame.clone();
        bad[20] = next;
        assert!(parse_frame(&bad).is_none());
    }
    let mut bad = frame.clone();
    bad[18..20].fill(0);
    assert!(parse_frame(&bad).is_none());
    let mut extended = frame[..54].to_vec();
    extended[20] = 0;
    extended[18..20].copy_from_slice(&((frame.len() - 54 + 8) as u16).to_be_bytes());
    extended.extend_from_slice(&[58, 0, 1, 4, 0, 0, 0, 0]);
    extended.extend_from_slice(&frame[54..]);
    assert!(route(&extended).is_some());
    extended[57] = 255;
    assert!(parse_frame(&extended).is_none());
}

#[test]
fn duplicate_address_never_becomes_usable() {
    let mut host = Host::new(MAC, 0);
    let mut output = [0; FRAME_CAPACITY];
    host.poll(0, &mut output);
    let mut na = [0; 24];
    na[0] = 136;
    na[8..].copy_from_slice(&host.link_local);
    host.receive(&wire(ROUTER, ALL_NODES, 255, &na), 1, &mut output);
    assert_eq!(host.state, State::Duplicate);
    assert!(host.poll(10000, &mut output).is_none());
    assert!(host.route.is_none());
}

#[test]
fn malformed_neighbor_packets_cannot_create_a_false_duplicate() {
    let mut host = Host::new(MAC, 0);
    let mut output = [0; FRAME_CAPACITY];
    let mut na = [0; 24];
    na[0] = 136;
    na[4] = 0x40;
    na[8..].copy_from_slice(&host.link_local);
    host.receive(&wire(ROUTER, ALL_NODES, 255, &na), 1, &mut output);
    assert_eq!(
        host.state,
        State::LinkLocalDad,
        "solicited multicast NA is invalid"
    );
    na[4] = 0;
    host.receive(&wire(ROUTER, ALL_NODES, 254, &na), 2, &mut output);
    assert_eq!(host.state, State::LinkLocalDad);
}

#[test]
fn neighbor_solicitation_gets_exact_checked_advertisement() {
    let mut host = configured();
    let target = host.route.unwrap().address;
    let mut ns = [0; 32];
    ns[0] = 135;
    ns[8..24].copy_from_slice(&target);
    ns[24] = 1;
    ns[25] = 1;
    ns[26..].copy_from_slice(&ROUTER_MAC);
    let mut output = [0; FRAME_CAPACITY];
    let len = host
        .receive(
            &wire(ROUTER, solicited_node(target), 255, &ns),
            202,
            &mut output,
        )
        .unwrap();
    let reply = parse_frame(&output[..len]).unwrap();
    assert!(valid_transport(&reply));
    assert_eq!(reply.source, target);
    assert_eq!(reply.destination, ROUTER);
    assert_eq!(reply.payload[4], 0x60);
    assert_eq!(&reply.payload[26..], &MAC);
}

#[test]
fn echo_reply_must_match_source_destination_identity_and_payload() {
    let mut host = configured();
    let target = host.route.unwrap().address;
    let mut output = [0; FRAME_CAPACITY];
    let mut body = [
        129, 0, 0, 0, 0x47, 0x36, 0, 1, b'G', b'e', b'n', b'O', b'S', b'6',
    ];
    body[7] = 2;
    host.receive(&wire(ROUTER, target, 64, &body), 202, &mut output);
    assert!(!host.echo_reply);
    body[7] = 1;
    host.receive(&wire(ROUTER, target, 64, &body), 203, &mut output);
    assert!(host.echo_reply);
}

#[test]
fn router_discovery_and_route_expiry_have_bounded_lifetimes() {
    let mut output = [0; FRAME_CAPACITY];
    let mut host = Host::new(MAC, 0);
    for tick in [0, 100, 500, 900, 1300] {
        host.poll(tick, &mut output);
    }
    assert_eq!(host.state, State::Unavailable);
    assert!(host.route.is_none());
    let mut host = configured();
    let expires = host.route.unwrap().router_until;
    let len = host.poll(expires, &mut output).unwrap();
    assert_eq!(host.state, State::RouterSolicitation);
    assert!(host.route.is_none());
    assert_eq!(parse_frame(&output[..len]).unwrap().payload[0], 133);
}

#[test]
fn udp_checksum_is_mandatory_on_ipv6() {
    let mut frame = wire(ROUTER, ALL_NODES, 64, &[0; 8]);
    frame[20] = 17;
    frame[54..].copy_from_slice(&[0, 1, 0, 2, 0, 8, 0, 0]);
    assert!(!valid_transport(&parse_frame(&frame).unwrap()));
    let checksum = transport_checksum(ROUTER, ALL_NODES, 17, &frame[54..]);
    frame[60..62].copy_from_slice(&checksum.to_be_bytes());
    assert!(valid_transport(&parse_frame(&frame).unwrap()));
}

#[test]
fn tiny_output_buffers_never_panic_or_write_past_the_slice() {
    for len in 0..62 {
        let mut output = vec![0x5a; len];
        assert!(write_icmp(
            &mut output,
            MAC,
            ROUTER_MAC,
            link_local(MAC),
            ROUTER,
            64,
            &[128; 8]
        )
        .is_none());
        assert!(output.iter().all(|&b| b == 0x5a));
    }
    let mut host = Host::new(MAC, 0);
    assert!(host.poll(0, &mut [0; 16]).is_none());
    assert!(host.poll(1000, &mut [0; 16]).is_none());
    assert_eq!(host.state, State::LinkLocalDad);
    let mut frame = [0; FRAME_CAPACITY];
    host.poll(1000, &mut frame).unwrap();
    assert!(host.poll(1099, &mut frame).is_none());
}
