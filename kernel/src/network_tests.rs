use super::*;

fn peer(port: u16) -> TcpServerPeer {
    TcpServerPeer {
        target: u32::from_be_bytes([10, 0, 0, 2]),
        remote_port: port,
        local_port: 18081,
        source_mac: [2, 0, 0, 0, 0, 2],
        remote_sequence: 1000,
        local_sequence: 5000,
    }
}

fn stack() -> NetworkStack {
    let mut stack = NetworkStack::new();
    stack.available = true;
    stack.address = [10, 0, 0, 1];
    stack
}

fn connected() -> (NetworkStack, TcpServerPeer) {
    let mut stack = stack();
    let peer = peer(40000);
    assert!(stack.start_tcp_passive_stream(peer, EarlyStreamData::empty(), 0));
    (stack, peer)
}

#[allow(clippy::too_many_arguments)]
fn packet(
    stack: &mut NetworkStack,
    peer: TcpServerPeer,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    payload: &[u8],
    tick: u64,
) {
    let mut sender = self::stack();
    sender.address = peer.target.to_be_bytes();
    sender
        .send_tcp_with_window(
            stack.device.mac(),
            stack.address,
            peer.remote_port,
            peer.local_port,
            seq,
            ack,
            flags,
            payload,
            window,
        )
        .unwrap();
    stack
        .device
        .incoming
        .push_back(sender.device.outgoing.pop().unwrap());
    stack.pump_passive_rx(tick);
}

fn op(stack: &NetworkStack) -> PassiveTcpStreamOperation {
    stack.passive_streams[0].unwrap()
}

fn last_tcp(stack: &NetworkStack) -> net::TcpPacket<'_> {
    let ip = parse_ipv4_frame(stack.device.outgoing.last().unwrap()).unwrap();
    assert!(net::transport_checksum_valid(
        ip.source,
        ip.destination,
        6,
        ip.payload
    ));
    parse_tcp(ip.payload).unwrap()
}

#[test]
fn duplicates_are_acknowledged_without_duplicate_delivery() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 1000, 5000, 0x18, 256, b"hello", 1);
    packet(&mut stack, peer, 1000, 5000, 0x18, 256, b"hello", 2);
    assert_eq!(op(&stack).receive_len, 5);
    assert!(op(&stack).deferred.is_none());
    assert_eq!(last_tcp(&stack).acknowledgment, 1005);
    assert!(stack.consume_tcp_passive_stream_receive(peer));
    assert_eq!(op(&stack).receive_len, 0);
}

#[test]
fn deferred_data_and_fin_wait_until_every_gap_closes() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 1010, 5000, 0x19, 256, b"third", 1);
    packet(&mut stack, peer, 1000, 5000, 0x18, 256, b"first", 2);
    assert_eq!(op(&stack).remote_sequence, 1005);
    assert!(stack.consume_tcp_passive_stream_receive(peer));
    assert_eq!(op(&stack).receive_len, 0, "a gap cannot reach Ring 3");
    assert!(!op(&stack).peer_fin);
    packet(&mut stack, peer, 1005, 5000, 0x18, 256, b"2nd!!", 3);
    assert_eq!(op(&stack).remote_sequence, 1016);
    assert!(stack.consume_tcp_passive_stream_receive(peer));
    assert_eq!(&op(&stack).receive[..5], b"third");
    assert!(op(&stack).peer_fin);
}

#[test]
fn reset_requires_exact_receive_sequence_and_challenges_in_window_guess() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 999, 0, 0x04, 0, &[], 1);
    assert!(!op(&stack).reset, "old reset must not kill a connection");
    let sent = stack.device.outgoing.len();
    packet(&mut stack, peer, 1001, 0, 0x04, 0, &[], 2);
    assert!(!op(&stack).reset);
    assert_eq!(stack.device.outgoing.len(), sent + 1);
    assert_eq!(last_tcp(&stack).flags, 0x10);
    packet(&mut stack, peer, 1000, 0, 0x04, 0, &[], 3);
    assert!(
        matches!(stack.poll_tcp_passive_stream(3), PassiveTcpStreamProgress::Reset(p) if p == peer)
    );
}

#[test]
fn forged_ack_cannot_change_peer_window_or_complete_send() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 1000, 5000, 0x10, 64, &[], 1);
    assert!(stack.start_tcp_passive_stream_send(peer, b"hello", 2));
    packet(&mut stack, peer, 1000, 99999, 0x10, 0, &[], 3);
    assert_eq!(op(&stack).peer_window, 64);
    assert_eq!(op(&stack).send_len, 5);
    packet(&mut stack, peer, 99999, 5005, 0x10, 0, &[], 4);
    assert_eq!(op(&stack).send_len, 5, "out-of-window ACK has no authority");
}

#[test]
fn zero_window_defers_payload_and_reopening_resumes_exact_bytes() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 1000, 5000, 0x10, 0, &[], 1);
    let sent = stack.device.outgoing.len();
    assert!(stack.start_tcp_passive_stream_send(peer, b"hello", 2));
    assert_eq!(
        stack.device.outgoing.len(),
        sent,
        "zero window forbids payload flight"
    );
    packet(&mut stack, peer, 1000, 5000, 0x10, 3, &[], 3);
    stack.poll_tcp_passive_stream(3);
    assert_eq!(last_tcp(&stack).payload, b"hel");
    packet(&mut stack, peer, 1000, 5003, 0x10, 3, &[], 4);
    stack.poll_tcp_passive_stream(4);
    assert_eq!(last_tcp(&stack).payload, b"lo");
    packet(&mut stack, peer, 1000, 5005, 0x10, 3, &[], 5);
    assert!(
        matches!(stack.poll_tcp_passive_stream(5), PassiveTcpStreamProgress::SendComplete(p) if p == peer)
    );
}

#[test]
fn retransmitted_ack_does_not_supply_an_ambiguous_rtt_sample() {
    let (mut stack, peer) = connected();
    assert!(stack.start_tcp_passive_stream_send(peer, b"hello", 1));
    stack.poll_tcp_passive_stream(26);
    let backed_off = op(&stack).rto_ticks;
    packet(&mut stack, peer, 1000, 5005, 0x10, 256, &[], 27);
    assert_eq!(op(&stack).rto_ticks, backed_off);
}

#[test]
fn full_receive_window_reopens_after_consumption_without_losing_bytes() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 1000, 5000, 0x18, 256, &[b'a'; 128], 1);
    packet(&mut stack, peer, 1128, 5000, 0x18, 256, &[b'b'; 128], 2);
    assert_eq!(last_tcp(&stack).window, 0);
    packet(&mut stack, peer, 1256, 5000, 0x18, 256, b"retry", 3);
    assert_eq!(op(&stack).remote_sequence, 1256);
    assert!(stack.consume_tcp_passive_stream_receive(peer));
    assert_eq!(last_tcp(&stack).window, 128);
    assert_eq!(&op(&stack).receive[..128], &[b'b'; 128]);
    assert!(stack.consume_tcp_passive_stream_receive(peer));
    packet(&mut stack, peer, 1256, 5000, 0x18, 256, b"retry", 4);
    assert_eq!(&op(&stack).receive[..5], b"retry");
}

#[test]
fn slow_reader_does_not_stop_outbound_retransmissions() {
    let (mut stack, peer) = connected();
    assert!(stack.start_tcp_passive_stream_send(peer, b"reply", 1));
    packet(&mut stack, peer, 1000, 5000, 0x18, 256, b"unread", 2);
    let transmissions = stack.device.outgoing.len();
    stack.poll_tcp_passive_stream(26);
    assert_eq!(stack.device.outgoing.len(), transmissions + 1);
    assert_eq!(last_tcp(&stack).payload, b"reply");
}

#[test]
fn cancellation_and_reset_leave_sibling_stream_usable() {
    let (mut stack, first) = connected();
    let second = peer(40001);
    assert!(stack.start_tcp_passive_stream(second, EarlyStreamData::empty(), 0));
    assert!(stack.start_tcp_passive_stream_send(first, b"cancel", 1));
    stack.cancel_tcp_passive_stream(first);
    assert_eq!(last_tcp(&stack).flags, 0x14);
    packet(&mut stack, first, 1000, 5006, 0x10, 256, &[], 2);
    assert!(stack.stream_slot(first).is_none());
    packet(&mut stack, second, 1000, 5000, 0x18, 256, b"healthy", 3);
    assert!(
        matches!(stack.poll_tcp_passive_stream(3), PassiveTcpStreamProgress::Received { peer, len: 7, .. } if peer == second)
    );
}

#[test]
fn zero_window_persists_with_bounded_probes_and_releases_its_slot() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 1000, 5000, 0x10, 0, &[], 1);
    assert!(stack.start_tcp_passive_stream_send(peer, b"pending", 2));
    for tick in 3..202 {
        stack.poll_tcp_passive_stream(tick);
    }
    assert!(stack.device.outgoing.len() <= 3);
    assert_eq!(last_tcp(&stack).sequence, 4999);
    assert_eq!(op(&stack).send_len, 7);
    assert_eq!(stack.tcp_congestion_events, 0);
    assert!(
        matches!(stack.poll_tcp_passive_stream(202), PassiveTcpStreamProgress::Failed(p) if p == peer)
    );
    assert!(stack.stream_slot(peer).is_none());
}

#[test]
fn sequence_wrap_preserves_order_and_exact_send_completion() {
    let mut stack = stack();
    let mut peer = peer(40000);
    peer.remote_sequence = u32::MAX - 2;
    peer.local_sequence = u32::MAX - 1;
    assert!(stack.start_tcp_passive_stream(peer, EarlyStreamData::empty(), 0));
    packet(
        &mut stack,
        peer,
        u32::MAX - 2,
        u32::MAX - 1,
        0x18,
        256,
        b"wrap",
        1,
    );
    assert_eq!(op(&stack).remote_sequence, 1);
    assert!(stack.consume_tcp_passive_stream_receive(peer));
    assert!(stack.start_tcp_passive_stream_send(peer, b"wrap", 2));
    packet(&mut stack, peer, 1, 2, 0x10, 256, &[], 3);
    assert!(matches!(
        stack.poll_tcp_passive_stream(3),
        PassiveTcpStreamProgress::SendComplete(_)
    ));
}

#[test]
fn overlap_cannot_deliver_conflicting_deferred_bytes() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 1005, 5000, 0x18, 256, b"first", 1);
    packet(&mut stack, peer, 1000, 5000, 0x18, 256, b"0123456789", 2);
    // This bounded receiver declines partial overlaps, leaving the gap and
    // its first accepted bytes unchanged for an exact retransmission.
    assert_eq!(op(&stack).remote_sequence, 1000);
    assert_eq!(op(&stack).receive_len, 0);
    packet(&mut stack, peer, 1000, 5000, 0x18, 256, b"01234", 3);
    assert_eq!(op(&stack).remote_sequence, 1010);
    assert!(stack.consume_tcp_passive_stream_receive(peer));
    assert_eq!(&op(&stack).receive[..5], b"first");
}

#[test]
fn no_data_or_second_fin_is_admitted_after_peer_eof() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 1000, 5000, 0x11, 256, &[], 1);
    packet(&mut stack, peer, 1001, 5000, 0x19, 256, b"invalid", 2);
    assert_eq!(op(&stack).remote_sequence, 1001);
    assert_eq!(op(&stack).receive_len, 0);
}

#[test]
fn fin_waits_for_an_open_receive_window() {
    let (mut stack, peer) = connected();
    packet(&mut stack, peer, 1000, 5000, 0x18, 256, &[0; 128], 1);
    packet(&mut stack, peer, 1128, 5000, 0x18, 256, &[1; 128], 2);
    packet(&mut stack, peer, 1256, 5000, 0x11, 256, &[], 3);
    assert!(!op(&stack).peer_fin);
    assert_eq!(op(&stack).remote_sequence, 1256);
    stack.consume_tcp_passive_stream_receive(peer);
    packet(&mut stack, peer, 1256, 5000, 0x11, 256, &[], 4);
    assert!(op(&stack).peer_fin);
}

#[test]
fn stream_capacity_exhaustion_is_bounded_and_reusable() {
    let mut stack = stack();
    for offset in 0..PASSIVE_TCP_STREAM_SLOTS {
        assert!(stack.start_tcp_passive_stream(
            peer(40000 + offset as u16),
            EarlyStreamData::empty(),
            0
        ));
    }
    assert!(!stack.start_tcp_passive_stream(peer(50000), EarlyStreamData::empty(), 0));
    stack.cancel_tcp_passive_stream(peer(40002));
    assert!(stack.start_tcp_passive_stream(peer(50000), EarlyStreamData::empty(), 1));
}

#[test]
fn checksum_corruption_and_foreign_tuple_never_change_stream_state() {
    let (mut stack, peer) = connected();
    let mut sender = self::stack();
    sender.address = peer.target.to_be_bytes();
    sender
        .send_tcp_with_window(
            stack.device.mac(),
            stack.address,
            peer.remote_port,
            peer.local_port,
            1000,
            5000,
            0x18,
            b"safe",
            256,
        )
        .unwrap();
    let frame = sender.device.outgoing.pop().unwrap();
    for length in 0..frame.len() {
        stack.device.incoming.push_back(frame[..length].to_vec());
        stack.pump_passive_rx(1);
        assert_eq!(op(&stack).remote_sequence, 1000);
    }
    let mut corrupt = frame;
    *corrupt.last_mut().unwrap() ^= 1;
    stack.device.incoming.push_back(corrupt);
    stack.pump_passive_rx(2);
    let mut foreign = peer;
    foreign.remote_port += 1;
    packet(&mut stack, foreign, 1000, 5000, 0x18, 256, b"wrong", 3);
    assert_eq!(op(&stack).receive_len, 0);
}
