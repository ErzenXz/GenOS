use core::ptr::addr_of_mut;

use genos_abi::UserNetworkConfig;
use kernel::{
    ipv6,
    net::{self, parse_ipv4_frame, parse_tcp, parse_udp},
    socket::TcpServerPeer,
};

use crate::{
    network_device::{NetworkDevice, PacketBuffer, PacketOwner, MAX_FRAME},
    serial,
};

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;

const POLL_LIMIT: usize = 800_000;
const RETRIES: usize = 3;
const ASYNC_UDP_RETRY_TICKS: u64 = 25;
const ASYNC_UDP_RX_POLLS_PER_TICK: usize = 4_096;
const ASYNC_UDP_BUFFER_CAPACITY: usize = genos_abi::USER_SOCKET_BUFFER_CAPACITY as usize;
const ASYNC_TCP_RETRY_TICKS: u64 = 25;
const ASYNC_TCP_RX_POLLS_PER_TICK: usize = 4_096;
const ASYNC_TCP_BUFFER_CAPACITY: usize = genos_abi::USER_SOCKET_BUFFER_CAPACITY as usize;
const PASSIVE_TCP_RETRY_TICKS: u64 = 25;
const PASSIVE_TCP_RX_POLLS_PER_TICK: usize = 4_096;
const PASSIVE_TCP_STREAM_IDLE_TICKS: u64 = 200;
const PASSIVE_TCP_STREAM_BUFFER_CAPACITY: usize = genos_abi::USER_SOCKET_BUFFER_CAPACITY as usize;
const PASSIVE_TCP_MSS: usize = PASSIVE_TCP_STREAM_BUFFER_CAPACITY;
const PASSIVE_TCP_INITIAL_CWND: usize = PASSIVE_TCP_MSS * 2;
const PASSIVE_TCP_MAX_CWND: usize = PASSIVE_TCP_MSS * 8;
const PASSIVE_TCP_REORDER_SLOTS: usize = 1;
const PASSIVE_TCP_RECEIVE_CAPACITY: usize =
    PASSIVE_TCP_STREAM_BUFFER_CAPACITY * (1 + PASSIVE_TCP_REORDER_SLOTS);
pub const PASSIVE_TCP_HANDSHAKE_SLOTS: usize = 4;
pub const PASSIVE_TCP_STREAM_SLOTS: usize = 4;

#[derive(Clone, Copy)]
enum AsyncUdpPhase {
    Resolve,
    Response { mac: [u8; 6] },
}

#[derive(Clone, Copy)]
struct AsyncUdpOperation {
    target: [u8; 4],
    next_hop: [u8; 4],
    remote_port: u16,
    local_port: u16,
    request: [u8; ASYNC_UDP_BUFFER_CAPACITY],
    request_len: usize,
    phase: AsyncUdpPhase,
    attempts: usize,
    deadline: u64,
}

pub enum AsyncUdpProgress {
    Idle,
    Pending,
    Complete {
        bytes: [u8; ASYNC_UDP_BUFFER_CAPACITY],
        len: usize,
    },
    Failed,
}

#[derive(Clone, Copy)]
enum AsyncTcpPhase {
    Resolve,
    Syn {
        mac: [u8; 6],
    },
    Response {
        mac: [u8; 6],
        local_seq: u32,
        remote_seq: u32,
        bytes: [u8; ASYNC_TCP_BUFFER_CAPACITY],
        len: usize,
    },
}

#[derive(Clone, Copy)]
struct AsyncTcpOperation {
    target: [u8; 4],
    next_hop: [u8; 4],
    remote_port: u16,
    local_port: u16,
    initial_seq: u32,
    request: [u8; ASYNC_TCP_BUFFER_CAPACITY],
    request_len: usize,
    phase: AsyncTcpPhase,
    attempts: usize,
    deadline: u64,
}

pub enum AsyncTcpProgress {
    Idle,
    Pending,
    Complete {
        bytes: [u8; ASYNC_TCP_BUFFER_CAPACITY],
        len: usize,
    },
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PassiveTcpSyn {
    pub target: u32,
    pub remote_port: u16,
    pub local_port: u16,
    remote_sequence: u32,
    source_mac: [u8; 6],
}

#[derive(Clone, Copy)]
struct PassiveTcpOperation {
    target: [u8; 4],
    remote_port: u16,
    local_port: u16,
    remote_sequence: u32,
    local_sequence: u32,
    source_mac: [u8; 6],
    established: bool,
    failed: bool,
    early: EarlyStreamData,
    attempts: usize,
    deadline: u64,
}

/// At most one bounded data segment plus one FIN that a peer sends between
/// its final handshake acknowledgment and the moment the coordinator attaches
/// the established stream slot. The handshake slot acknowledges these bytes
/// immediately so a fast peer is not left retransmitting into the gap, and
/// the stream slot is seeded with them when it starts.
#[derive(Clone, Copy)]
pub struct EarlyStreamData {
    bytes: [u8; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
    len: usize,
    fin: bool,
    peer_window: u16,
}

impl EarlyStreamData {
    const fn empty() -> Self {
        Self {
            bytes: [0; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
            len: 0,
            fin: false,
            peer_window: u16::MAX,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PassiveTcpFailure {
    pub target: u32,
    pub remote_port: u16,
    pub local_port: u16,
}

/// One bounded, owned copy of a validated inbound TCP segment addressed to
/// this host. The shared passive receive pump decodes each frame exactly once
/// and dispatches it to the owning stream slot, handshake slot, or the
/// pending-SYN cell, so concurrent passive operations never consume each
/// other's frames.
#[derive(Clone, Copy)]
struct PassiveSegment {
    source: [u8; 4],
    source_mac: [u8; 6],
    remote_port: u16,
    local_port: u16,
    sequence: u32,
    acknowledgment: u32,
    flags: u8,
    window: u16,
    payload: [u8; 1400],
    len: usize,
}

#[derive(Clone, Copy)]
struct PassiveTcpStreamOperation {
    peer: TcpServerPeer,
    remote_sequence: u32,
    local_sequence: u32,
    receive: [u8; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
    receive_len: usize,
    deferred: Option<DeferredTcpSegment>,
    send: [u8; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
    send_len: usize,
    send_flight: usize,
    send_completed: bool,
    fin_sent: bool,
    fin_acked: bool,
    peer_fin: bool,
    peer_fin_pending: bool,
    reset: bool,
    failed: bool,
    attempts: usize,
    deadline: u64,
    sent_at: u64,
    rto_ticks: u64,
    peer_window: u16,
    window_sequence: u32,
    window_acknowledgment: u32,
    persist_deadline: u64,
    persist_attempts: usize,
    challenge_ack_tick: Option<u64>,
    congestion_window: usize,
    slow_start_threshold: usize,
    stream_received_bytes: u64,
    stream_sent_bytes: u64,
}

#[derive(Clone, Copy)]
struct DeferredTcpSegment {
    sequence: u32,
    bytes: [u8; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
    len: usize,
    fin: bool,
    acknowledged: bool,
}

pub enum PassiveTcpProgress {
    Idle,
    Syn(PassiveTcpSyn),
    Pending,
    Established(TcpServerPeer, EarlyStreamData),
    Failed(PassiveTcpFailure),
}

pub enum PassiveTcpStreamProgress {
    Idle,
    Pending,
    Received {
        peer: TcpServerPeer,
        bytes: [u8; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
        len: usize,
    },
    SendComplete(TcpServerPeer),
    PeerClosed(TcpServerPeer),
    Closed(TcpServerPeer),
    Reset(TcpServerPeer),
    Failed(TcpServerPeer),
}

struct NetworkStack {
    device: NetworkDevice,
    available: bool,
    address: [u8; 4],
    subnet: [u8; 4],
    gateway: [u8; 4],
    dns: [u8; 4],
    next_port: u16,
    ip_id: u16,
    rx: PacketBuffer,
    ipv6: Option<ipv6::Host>,
    control_tick: u64,
    ipv6_reported: bool,
    async_udp: Option<AsyncUdpOperation>,
    async_tcp: Option<AsyncTcpOperation>,
    passive_tcp: [Option<PassiveTcpOperation>; PASSIVE_TCP_HANDSHAKE_SLOTS],
    passive_streams: [Option<PassiveTcpStreamOperation>; PASSIVE_TCP_STREAM_SLOTS],
    passive_stream_cursor: usize,
    pending_syns: [Option<PassiveTcpSyn>; PASSIVE_TCP_HANDSHAKE_SLOTS],
    tcp_retransmissions: u64,
    tcp_reordered_segments: u64,
    tcp_payload_bytes: u64,
    tcp_max_buffered: usize,
    tcp_max_ack_latency: u64,
    tcp_max_stream_bytes: u64,
    tcp_congestion_events: u64,
    tcp_min_cwnd: usize,
    tcp_max_cwnd: usize,
    tcp_service_started: Option<u64>,
    tcp_service_finished: u64,
    #[cfg(feature = "network-test-faults")]
    regression_reported: bool,
    #[cfg(feature = "network-test-faults")]
    regression_sample_reported: bool,
}

impl NetworkStack {
    fn advance_control(&mut self, tick: u64) {
        self.control_tick = tick;
        let Some(host) = self.ipv6.as_mut() else {
            return;
        };
        let mut frame = [0u8; ipv6::FRAME_CAPACITY];
        if let Some(len) = host.poll(tick, &mut frame) {
            if !self.device.transmit(&frame[..len]) {
                host.state = ipv6::State::Unavailable;
                host.route = None;
            }
        }
        if !self.ipv6_reported && host.state == ipv6::State::Ready && host.echo_reply {
            self.ipv6_reported = true;
            serial::println("IPV6_SLAAC_READY prefix=ra dad=passed");
            serial::println("IPV6_ICMP_ECHO_OK");
        }
    }

    fn receive_ipv4(&mut self) -> bool {
        if !self.device.receive(&mut self.rx) {
            return false;
        }
        if self.rx.len >= 14 && self.rx.bytes[12..14] == [0x86, 0xdd] {
            if let Some(host) = self.ipv6.as_mut() {
                let mut reply = [0u8; ipv6::FRAME_CAPACITY];
                if let Some(len) =
                    host.receive(&self.rx.bytes[..self.rx.len], self.control_tick, &mut reply)
                {
                    let _ = self.device.transmit(&reply[..len]);
                }
            }
            self.rx.owner = PacketOwner::Free;
            return false;
        }
        true
    }

    const fn new() -> Self {
        Self {
            device: NetworkDevice::new(),
            available: false,
            address: [0; 4],
            subnet: [0; 4],
            gateway: [0; 4],
            dns: [0; 4],
            next_port: 49152,
            ip_id: 1,
            rx: PacketBuffer::empty(),
            ipv6: None,
            control_tick: 0,
            ipv6_reported: false,
            async_udp: None,
            async_tcp: None,
            passive_tcp: [None; PASSIVE_TCP_HANDSHAKE_SLOTS],
            passive_streams: [None; PASSIVE_TCP_STREAM_SLOTS],
            passive_stream_cursor: 0,
            pending_syns: [None; PASSIVE_TCP_HANDSHAKE_SLOTS],
            tcp_retransmissions: 0,
            tcp_reordered_segments: 0,
            tcp_payload_bytes: 0,
            tcp_max_buffered: 0,
            tcp_max_ack_latency: 0,
            tcp_max_stream_bytes: 0,
            tcp_congestion_events: 0,
            tcp_min_cwnd: PASSIVE_TCP_INITIAL_CWND,
            tcp_max_cwnd: PASSIVE_TCP_INITIAL_CWND,
            tcp_service_started: None,
            tcp_service_finished: 0,
            #[cfg(feature = "network-test-faults")]
            regression_reported: false,
            #[cfg(feature = "network-test-faults")]
            regression_sample_reported: false,
        }
    }

    fn configure(&mut self) -> bool {
        for _ in 0..RETRIES {
            if let Some(config) = self.dhcp_attempt() {
                self.address = config.address;
                self.subnet = config.subnet;
                self.gateway = config.gateway;
                self.dns = config.dns;
                self.available = true;
                return true;
            }
        }
        false
    }

    fn dhcp_attempt(&mut self) -> Option<DhcpConfig> {
        let xid = 0x4745_4e4f;
        let mut payload = [0u8; 300];
        let discover_len = build_dhcp(&mut payload, self.device.mac(), xid, 1, None, None)?;
        self.send_udp_raw(
            [0xff; 6],
            [0, 0, 0, 0],
            [255, 255, 255, 255],
            68,
            67,
            &payload[..discover_len],
        )?;
        let offer = self.wait_dhcp(xid, 2)?;
        let request_len = build_dhcp(
            &mut payload,
            self.device.mac(),
            xid,
            3,
            Some(offer.address),
            Some(offer.server),
        )?;
        self.send_udp_raw(
            [0xff; 6],
            [0, 0, 0, 0],
            [255, 255, 255, 255],
            68,
            67,
            &payload[..request_len],
        )?;
        self.wait_dhcp(xid, 5)
    }

    fn wait_dhcp(&mut self, xid: u32, expected_type: u8) -> Option<DhcpConfig> {
        for _ in 0..POLL_LIMIT {
            if !self.receive_ipv4() {
                continue;
            }
            let result = parse_ipv4_frame(&self.rx.bytes[..self.rx.len])
                .filter(|ip| {
                    ip.protocol == 17
                        && net::transport_checksum_valid(
                            ip.source,
                            ip.destination,
                            ip.protocol,
                            ip.payload,
                        )
                })
                .and_then(|ip| parse_udp(ip.payload))
                .filter(|udp| udp.source_port == 67 && udp.destination_port == 68)
                .and_then(|udp| parse_dhcp(udp.payload, xid, expected_type));
            self.rx.owner = PacketOwner::Free;
            if result.is_some() {
                return result;
            }
        }
        None
    }

    fn ping(&mut self, target: [u8; 4]) -> bool {
        let Some(mac) = self.resolve_route(target) else {
            return false;
        };
        let mut icmp = [0u8; 16];
        icmp[0] = 8;
        icmp[4..6].copy_from_slice(&0x4745u16.to_be_bytes());
        icmp[6..8].copy_from_slice(&1u16.to_be_bytes());
        icmp[8..].copy_from_slice(b"GENOSNET");
        let checksum = net::checksum(&icmp);
        icmp[2..4].copy_from_slice(&checksum.to_be_bytes());
        if !self.send_ipv4(mac, target, 1, &icmp) {
            return false;
        }
        for _ in 0..POLL_LIMIT {
            if !self.receive_ipv4() {
                continue;
            }
            let matched = parse_ipv4_frame(&self.rx.bytes[..self.rx.len]).is_some_and(|ip| {
                ip.protocol == 1
                    && ip.source == target
                    && ip.destination == self.address
                    && ip.payload.len() >= 8
                    && ip.payload[0] == 0
                    && ip.payload[4..8] == icmp[4..8]
                    && net::checksum(ip.payload) == 0
            });
            self.rx.owner = PacketOwner::Free;
            if matched {
                return true;
            }
        }
        false
    }

    fn udp_exchange(
        &mut self,
        target: [u8; 4],
        port: u16,
        request: &[u8],
        response: &mut [u8],
    ) -> Option<usize> {
        if !self.available || request.is_empty() || request.len() > 1200 {
            return None;
        }
        let source_port = self.allocate_port();
        let mac = self.resolve_route(target)?;
        for _ in 0..RETRIES {
            self.send_udp_raw(mac, self.address, target, source_port, port, request)?;
            for _ in 0..POLL_LIMIT / RETRIES {
                if !self.receive_ipv4() {
                    continue;
                }
                let result = parse_ipv4_frame(&self.rx.bytes[..self.rx.len])
                    .filter(|ip| {
                        ip.protocol == 17
                            && ip.source == target
                            && net::transport_checksum_valid(
                                ip.source,
                                ip.destination,
                                ip.protocol,
                                ip.payload,
                            )
                    })
                    .and_then(|ip| parse_udp(ip.payload))
                    .filter(|udp| udp.source_port == port && udp.destination_port == source_port)
                    .map(|udp| {
                        let len = udp.payload.len().min(response.len());
                        response[..len].copy_from_slice(&udp.payload[..len]);
                        len
                    });
                self.rx.owner = PacketOwner::Free;
                if result.is_some() {
                    return result;
                }
            }
        }
        None
    }

    fn tcp_exchange(
        &mut self,
        target: [u8; 4],
        port: u16,
        request: &[u8],
        response: &mut [u8],
    ) -> Option<usize> {
        if !self.available || request.is_empty() || request.len() > 1000 {
            return None;
        }
        let source_port = self.allocate_port();
        let mac = self.resolve_route(target)?;
        let initial = 0x1020_3040u32 ^ u32::from(source_port);
        let mut syn_ack = None;
        for _ in 0..RETRIES {
            self.send_tcp(mac, target, source_port, port, initial, 0, 0x02, &[])?;
            if let Some(reply) = self.wait_tcp(target, port, source_port, POLL_LIMIT / RETRIES) {
                syn_ack = Some(reply);
                break;
            }
        }
        let syn_ack = syn_ack?;
        if syn_ack.flags & 0x12 != 0x12 || syn_ack.acknowledgment != initial.wrapping_add(1) {
            return None;
        }
        serial::println("TCP_CONNECT_READY");
        let mut local_seq = initial.wrapping_add(1);
        let mut remote_seq = syn_ack.sequence.wrapping_add(1);
        self.send_tcp(
            mac,
            target,
            source_port,
            port,
            local_seq,
            remote_seq,
            0x10,
            &[],
        )?;
        self.send_tcp(
            mac,
            target,
            source_port,
            port,
            local_seq,
            remote_seq,
            0x18,
            request,
        )?;
        local_seq = local_seq.wrapping_add(request.len() as u32);

        let mut received = 0usize;
        for attempt in 0..RETRIES {
            if attempt > 0 && received == 0 {
                self.send_tcp(
                    mac,
                    target,
                    source_port,
                    port,
                    initial.wrapping_add(1),
                    remote_seq,
                    0x18,
                    request,
                )?;
            }
            for _ in 0..POLL_LIMIT {
                let Some(packet) = self.poll_tcp(target, port, source_port) else {
                    continue;
                };
                if packet.flags & 0x04 != 0 {
                    return None;
                }
                if !packet.payload().is_empty() && packet.sequence == remote_seq {
                    let len = packet
                        .payload()
                        .len()
                        .min(response.len().saturating_sub(received));
                    response[received..received + len].copy_from_slice(&packet.payload()[..len]);
                    received += len;
                    remote_seq = remote_seq.wrapping_add(packet.payload().len() as u32);
                    serial::println("TCP_DATA_READY");
                    self.send_tcp(
                        mac,
                        target,
                        source_port,
                        port,
                        local_seq,
                        remote_seq,
                        0x10,
                        &[],
                    )?;
                }
                if packet.flags & 0x01 != 0 {
                    remote_seq = remote_seq.wrapping_add(1);
                    self.send_tcp(
                        mac,
                        target,
                        source_port,
                        port,
                        local_seq,
                        remote_seq,
                        0x10,
                        &[],
                    )?;
                    serial::println("TCP_CLOSE_READY");
                    return (received > 0).then_some(received);
                }
            }
        }
        None
    }

    fn wait_tcp(
        &mut self,
        target: [u8; 4],
        remote_port: u16,
        local_port: u16,
        limit: usize,
    ) -> Option<TcpOwned> {
        for _ in 0..limit {
            if let Some(packet) = self.poll_tcp(target, remote_port, local_port) {
                return Some(packet);
            }
        }
        None
    }

    fn poll_tcp(&mut self, target: [u8; 4], remote_port: u16, local_port: u16) -> Option<TcpOwned> {
        if !self.receive_ipv4() {
            return None;
        }
        let result = parse_ipv4_frame(&self.rx.bytes[..self.rx.len])
            .filter(|ip| {
                ip.protocol == 6
                    && ip.source == target
                    && ip.destination == self.address
                    && net::transport_checksum_valid(
                        ip.source,
                        ip.destination,
                        ip.protocol,
                        ip.payload,
                    )
            })
            .and_then(|ip| parse_tcp(ip.payload))
            .filter(|tcp| tcp.source_port == remote_port && tcp.destination_port == local_port)
            .map(|tcp| TcpOwned::from_packet(tcp));
        self.rx.owner = PacketOwner::Free;
        result
    }

    fn resolve_route(&mut self, destination: [u8; 4]) -> Option<[u8; 6]> {
        let same_subnet = (0..4).all(|index| {
            destination[index] & self.subnet[index] == self.address[index] & self.subnet[index]
        });
        self.resolve_arp(if same_subnet {
            destination
        } else {
            self.gateway
        })
    }

    fn route_next_hop(&self, destination: [u8; 4]) -> [u8; 4] {
        let same_subnet = (0..4).all(|index| {
            destination[index] & self.subnet[index] == self.address[index] & self.subnet[index]
        });
        if same_subnet {
            destination
        } else {
            self.gateway
        }
    }

    fn passive_service_active(&self) -> bool {
        self.passive_tcp.iter().any(Option::is_some)
            || self.passive_streams.iter().any(Option::is_some)
    }

    fn start_udp_async(&mut self, target: [u8; 4], port: u16, request: &[u8], tick: u64) -> bool {
        if !self.available
            || self.async_udp.is_some()
            || self.async_tcp.is_some()
            || self.passive_service_active()
            || target == [0; 4]
            || port == 0
            || request.is_empty()
            || request.len() > ASYNC_UDP_BUFFER_CAPACITY
        {
            return false;
        }
        let mut payload = [0u8; ASYNC_UDP_BUFFER_CAPACITY];
        payload[..request.len()].copy_from_slice(request);
        self.async_udp = Some(AsyncUdpOperation {
            target,
            next_hop: self.route_next_hop(target),
            remote_port: port,
            local_port: self.allocate_port(),
            request: payload,
            request_len: request.len(),
            phase: AsyncUdpPhase::Resolve,
            attempts: 0,
            deadline: tick,
        });
        true
    }

    fn poll_udp_async(&mut self, tick: u64) -> AsyncUdpProgress {
        let Some(mut operation) = self.async_udp.take() else {
            return AsyncUdpProgress::Idle;
        };

        let mut received = false;
        for _ in 0..ASYNC_UDP_RX_POLLS_PER_TICK {
            if self.receive_ipv4() {
                received = true;
                break;
            }
            core::hint::spin_loop();
        }
        if received {
            let frame = &self.rx.bytes[..self.rx.len];
            match operation.phase {
                AsyncUdpPhase::Resolve => {
                    if let Some(mac) = net::parse_arp_reply(frame, operation.next_hop, self.address)
                    {
                        if self
                            .send_udp_raw(
                                mac,
                                self.address,
                                operation.target,
                                operation.local_port,
                                operation.remote_port,
                                &operation.request[..operation.request_len],
                            )
                            .is_none()
                        {
                            self.rx.owner = PacketOwner::Free;
                            return AsyncUdpProgress::Failed;
                        }
                        operation.phase = AsyncUdpPhase::Response { mac };
                        operation.attempts = 1;
                        operation.deadline = tick.saturating_add(ASYNC_UDP_RETRY_TICKS);
                    }
                }
                AsyncUdpPhase::Response { .. } => {
                    let result = parse_ipv4_frame(frame)
                        .filter(|ip| {
                            ip.protocol == 17
                                && ip.source == operation.target
                                && ip.destination == self.address
                                && net::transport_checksum_valid(
                                    ip.source,
                                    ip.destination,
                                    ip.protocol,
                                    ip.payload,
                                )
                        })
                        .and_then(|ip| parse_udp(ip.payload))
                        .filter(|udp| {
                            udp.source_port == operation.remote_port
                                && udp.destination_port == operation.local_port
                        });
                    if let Some(udp) = result {
                        let mut bytes = [0u8; ASYNC_UDP_BUFFER_CAPACITY];
                        let len = udp.payload.len().min(bytes.len());
                        bytes[..len].copy_from_slice(&udp.payload[..len]);
                        self.rx.owner = PacketOwner::Free;
                        return AsyncUdpProgress::Complete { bytes, len };
                    }
                }
            }
            self.rx.owner = PacketOwner::Free;
        }

        if tick >= operation.deadline {
            if operation.attempts >= RETRIES {
                return AsyncUdpProgress::Failed;
            }
            let sent = match operation.phase {
                AsyncUdpPhase::Resolve => self.send_arp_request(operation.next_hop),
                AsyncUdpPhase::Response { mac } => self
                    .send_udp_raw(
                        mac,
                        self.address,
                        operation.target,
                        operation.local_port,
                        operation.remote_port,
                        &operation.request[..operation.request_len],
                    )
                    .is_some(),
            };
            if !sent {
                return AsyncUdpProgress::Failed;
            }
            operation.attempts += 1;
            operation.deadline = tick.saturating_add(ASYNC_UDP_RETRY_TICKS);
        }
        self.async_udp = Some(operation);
        AsyncUdpProgress::Pending
    }

    fn start_tcp_async(&mut self, target: [u8; 4], port: u16, request: &[u8], tick: u64) -> bool {
        if !self.available
            || self.async_udp.is_some()
            || self.async_tcp.is_some()
            || self.passive_service_active()
            || target == [0; 4]
            || port == 0
            || request.is_empty()
            || request.len() > ASYNC_TCP_BUFFER_CAPACITY
        {
            return false;
        }
        let local_port = self.allocate_port();
        let mut payload = [0u8; ASYNC_TCP_BUFFER_CAPACITY];
        payload[..request.len()].copy_from_slice(request);
        self.async_tcp = Some(AsyncTcpOperation {
            target,
            next_hop: self.route_next_hop(target),
            remote_port: port,
            local_port,
            initial_seq: 0x5060_7080u32
                ^ u32::from(local_port)
                ^ (request.len() as u32).rotate_left(16),
            request: payload,
            request_len: request.len(),
            phase: AsyncTcpPhase::Resolve,
            attempts: 0,
            deadline: tick,
        });
        true
    }

    fn poll_tcp_async(&mut self, tick: u64) -> AsyncTcpProgress {
        let Some(mut operation) = self.async_tcp.take() else {
            return AsyncTcpProgress::Idle;
        };

        let mut received = false;
        for _ in 0..ASYNC_TCP_RX_POLLS_PER_TICK {
            if self.receive_ipv4() {
                received = true;
                break;
            }
            core::hint::spin_loop();
        }
        if received {
            match operation.phase {
                AsyncTcpPhase::Resolve => {
                    let mac = net::parse_arp_reply(
                        &self.rx.bytes[..self.rx.len],
                        operation.next_hop,
                        self.address,
                    );
                    self.rx.owner = PacketOwner::Free;
                    if let Some(mac) = mac {
                        if self
                            .send_tcp(
                                mac,
                                operation.target,
                                operation.local_port,
                                operation.remote_port,
                                operation.initial_seq,
                                0,
                                0x02,
                                &[],
                            )
                            .is_none()
                        {
                            return AsyncTcpProgress::Failed;
                        }
                        operation.phase = AsyncTcpPhase::Syn { mac };
                        operation.attempts = 1;
                        operation.deadline = tick.saturating_add(ASYNC_TCP_RETRY_TICKS);
                    }
                }
                AsyncTcpPhase::Syn { mac } => {
                    let packet = self.decode_tcp_reply(
                        operation.target,
                        operation.remote_port,
                        operation.local_port,
                    );
                    self.rx.owner = PacketOwner::Free;
                    if let Some(packet) = packet {
                        if packet.flags & 0x04 != 0 {
                            serial::println("TCP_ASYNC_RESET");
                            return AsyncTcpProgress::Failed;
                        }
                        if packet.flags & 0x12 == 0x12
                            && packet.acknowledgment == operation.initial_seq.wrapping_add(1)
                        {
                            let request_seq = operation.initial_seq.wrapping_add(1);
                            let remote_seq = packet.sequence.wrapping_add(1);
                            if self
                                .send_tcp(
                                    mac,
                                    operation.target,
                                    operation.local_port,
                                    operation.remote_port,
                                    request_seq,
                                    remote_seq,
                                    0x10,
                                    &[],
                                )
                                .is_none()
                                || self
                                    .send_tcp(
                                        mac,
                                        operation.target,
                                        operation.local_port,
                                        operation.remote_port,
                                        request_seq,
                                        remote_seq,
                                        0x18,
                                        &operation.request[..operation.request_len],
                                    )
                                    .is_none()
                            {
                                return AsyncTcpProgress::Failed;
                            }
                            operation.phase = AsyncTcpPhase::Response {
                                mac,
                                local_seq: request_seq.wrapping_add(operation.request_len as u32),
                                remote_seq,
                                bytes: [0; ASYNC_TCP_BUFFER_CAPACITY],
                                len: 0,
                            };
                            operation.attempts = 1;
                            operation.deadline = tick.saturating_add(ASYNC_TCP_RETRY_TICKS);
                        }
                    }
                }
                AsyncTcpPhase::Response {
                    mac,
                    local_seq,
                    mut remote_seq,
                    mut bytes,
                    mut len,
                } => {
                    let packet = self.decode_tcp_reply(
                        operation.target,
                        operation.remote_port,
                        operation.local_port,
                    );
                    self.rx.owner = PacketOwner::Free;
                    if let Some(packet) = packet {
                        if packet.flags & 0x04 != 0 {
                            serial::println("TCP_ASYNC_RESET");
                            return AsyncTcpProgress::Failed;
                        }
                        if packet.flags & 0x10 != 0 && packet.acknowledgment != local_seq {
                            // Ignore an ACK for another send sequence. The exact
                            // request remains in flight and its deadline governs retry.
                        } else if packet.sequence != remote_seq {
                            if self
                                .send_tcp(
                                    mac,
                                    operation.target,
                                    operation.local_port,
                                    operation.remote_port,
                                    local_seq,
                                    remote_seq,
                                    0x10,
                                    &[],
                                )
                                .is_none()
                            {
                                return AsyncTcpProgress::Failed;
                            }
                        } else {
                            if len.saturating_add(packet.len) > bytes.len() {
                                return AsyncTcpProgress::Failed;
                            }
                            bytes[len..len + packet.len]
                                .copy_from_slice(&packet.payload[..packet.len]);
                            len += packet.len;
                            remote_seq = remote_seq.wrapping_add(packet.len as u32);
                            let fin = packet.flags & 0x01 != 0;
                            if fin {
                                remote_seq = remote_seq.wrapping_add(1);
                            }
                            if packet.len != 0 || fin {
                                if self
                                    .send_tcp(
                                        mac,
                                        operation.target,
                                        operation.local_port,
                                        operation.remote_port,
                                        local_seq,
                                        remote_seq,
                                        0x10,
                                        &[],
                                    )
                                    .is_none()
                                {
                                    return AsyncTcpProgress::Failed;
                                }
                                operation.deadline = tick.saturating_add(ASYNC_TCP_RETRY_TICKS);
                            }
                            if fin {
                                if len == 0
                                    || self
                                        .send_tcp(
                                            mac,
                                            operation.target,
                                            operation.local_port,
                                            operation.remote_port,
                                            local_seq,
                                            remote_seq,
                                            0x11,
                                            &[],
                                        )
                                        .is_none()
                                {
                                    return AsyncTcpProgress::Failed;
                                }
                                return AsyncTcpProgress::Complete { bytes, len };
                            }
                        }
                    }
                    operation.phase = AsyncTcpPhase::Response {
                        mac,
                        local_seq,
                        remote_seq,
                        bytes,
                        len,
                    };
                }
            }
        }

        if tick >= operation.deadline {
            if operation.attempts >= RETRIES {
                serial::println("TCP_ASYNC_TIMEOUT");
                return AsyncTcpProgress::Failed;
            }
            let sent = match operation.phase {
                AsyncTcpPhase::Resolve => self.send_arp_request(operation.next_hop),
                AsyncTcpPhase::Syn { mac } => self
                    .send_tcp(
                        mac,
                        operation.target,
                        operation.local_port,
                        operation.remote_port,
                        operation.initial_seq,
                        0,
                        0x02,
                        &[],
                    )
                    .is_some(),
                AsyncTcpPhase::Response {
                    mac,
                    local_seq,
                    remote_seq,
                    len,
                    ..
                } => {
                    if len == 0 {
                        self.send_tcp(
                            mac,
                            operation.target,
                            operation.local_port,
                            operation.remote_port,
                            local_seq.wrapping_sub(operation.request_len as u32),
                            remote_seq,
                            0x18,
                            &operation.request[..operation.request_len],
                        )
                        .is_some()
                    } else {
                        self.send_tcp(
                            mac,
                            operation.target,
                            operation.local_port,
                            operation.remote_port,
                            local_seq,
                            remote_seq,
                            0x10,
                            &[],
                        )
                        .is_some()
                    }
                }
            };
            if !sent {
                return AsyncTcpProgress::Failed;
            }
            operation.attempts += 1;
            operation.deadline = tick.saturating_add(ASYNC_TCP_RETRY_TICKS);
        }
        self.async_tcp = Some(operation);
        AsyncTcpProgress::Pending
    }

    fn stream_slot(&self, peer: TcpServerPeer) -> Option<usize> {
        self.passive_streams
            .iter()
            .position(|slot| slot.is_some_and(|operation| operation.peer == peer))
    }

    fn stream_slot_for_tuple(
        &self,
        target: [u8; 4],
        remote_port: u16,
        local_port: u16,
    ) -> Option<usize> {
        self.passive_streams.iter().position(|slot| {
            slot.is_some_and(|operation| {
                operation.peer.target.to_be_bytes() == target
                    && operation.peer.remote_port == remote_port
                    && operation.peer.local_port == local_port
            })
        })
    }

    fn handshake_slot_for_tuple(
        &self,
        target: [u8; 4],
        remote_port: u16,
        local_port: u16,
    ) -> Option<usize> {
        self.passive_tcp.iter().position(|slot| {
            slot.is_some_and(|operation| {
                operation.target == target
                    && operation.remote_port == remote_port
                    && operation.local_port == local_port
            })
        })
    }

    /// Shared passive receive step: pull at most one frame within a bounded
    /// poll budget, decode it exactly once, and route it to the stream slot,
    /// handshake slot, or pending-SYN cell that owns its exact peer tuple.
    fn pump_passive_rx(&mut self, tick: u64) {
        if !self.available {
            return;
        }
        let mut received = false;
        for _ in 0..PASSIVE_TCP_RX_POLLS_PER_TICK {
            if self.receive_ipv4() {
                received = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !received {
            return;
        }
        let segment = self.decode_passive_segment();
        self.rx.owner = PacketOwner::Free;
        let Some(segment) = segment else {
            return;
        };
        #[cfg(feature = "network-test-faults")]
        {
            let attached_stream = self
                .stream_slot_for_tuple(segment.source, segment.remote_port, segment.local_port)
                .is_some();
            if attached_stream
                && segment.payload[..segment.len].starts_with(b"GENOS_PING_REORDER_A")
                && segment.len > PASSIVE_TCP_STREAM_BUFFER_CAPACITY
                && segment.payload[PASSIVE_TCP_STREAM_BUFFER_CAPACITY..segment.len]
                    .starts_with(b"GENOS_PING_REORDER_B")
            {
                let mut first = segment;
                first.len = PASSIVE_TCP_STREAM_BUFFER_CAPACITY;
                first.flags &= !0x01;
                let mut second = segment;
                second.sequence = second
                    .sequence
                    .wrapping_add(PASSIVE_TCP_STREAM_BUFFER_CAPACITY as u32);
                second.len = segment.len - PASSIVE_TCP_STREAM_BUFFER_CAPACITY;
                second
                    .payload
                    .copy_within(PASSIVE_TCP_STREAM_BUFFER_CAPACITY..segment.len, 0);
                serial::println("TCP_FAULT_REORDER_HELD");
                self.route_passive_segment(second, tick);
                self.route_passive_segment(first, tick);
                serial::println("TCP_FAULT_REORDER_RELEASED");
                return;
            }
        }
        self.route_passive_segment(segment, tick);
    }

    fn route_passive_segment(&mut self, segment: PassiveSegment, tick: u64) {
        if let Some(index) =
            self.stream_slot_for_tuple(segment.source, segment.remote_port, segment.local_port)
        {
            self.handle_stream_segment(index, &segment, tick);
            return;
        }
        if let Some(index) =
            self.handshake_slot_for_tuple(segment.source, segment.remote_port, segment.local_port)
        {
            self.handle_handshake_segment(index, &segment, tick);
            return;
        }
        if segment.flags & 0x3f == 0x02
            && segment.len == 0
            && segment.remote_port != 0
            && segment.local_port != 0
        {
            let syn = PassiveTcpSyn {
                target: u32::from_be_bytes(segment.source),
                remote_port: segment.remote_port,
                local_port: segment.local_port,
                remote_sequence: segment.sequence,
                source_mac: segment.source_mac,
            };
            let duplicate = self
                .pending_syns
                .iter()
                .flatten()
                .any(|pending| pending == &syn);
            if !duplicate {
                if let Some(free) = self.pending_syns.iter().position(Option::is_none) {
                    self.pending_syns[free] = Some(syn);
                }
            }
        }
    }

    fn handle_handshake_segment(&mut self, index: usize, segment: &PassiveSegment, tick: u64) {
        let Some(mut operation) = self.passive_tcp[index] else {
            return;
        };
        if segment.flags & 0x04 != 0 {
            // SYN-RECEIVED reset authority is tied to the expected peer
            // sequence; a matching four-tuple alone cannot abort the slot.
            let expected = operation
                .remote_sequence
                .wrapping_add(1)
                .wrapping_add(operation.early.len as u32)
                .wrapping_add(u32::from(operation.early.fin));
            operation.failed |= segment.sequence == expected;
            self.passive_tcp[index] = Some(operation);
            return;
        }
        if segment.flags & 0x3f == 0x02
            && segment.sequence == operation.remote_sequence
            && segment.len == 0
        {
            if self.send_passive_syn_ack(operation).is_none() {
                operation.failed = true;
            } else {
                operation.deadline = tick.saturating_add(PASSIVE_TCP_RETRY_TICKS);
            }
            self.passive_tcp[index] = Some(operation);
            return;
        }
        // The final handshake acknowledgment, optionally carrying the peer's
        // first bounded data segment and FIN. A fast peer sends these before
        // the coordinator can attach the stream slot, so the handshake slot
        // admits and acknowledges at most one early segment.
        let expected_sequence = operation
            .remote_sequence
            .wrapping_add(1)
            .wrapping_add(operation.early.len as u32);
        let acknowledgment_valid = segment.flags & 0x12 == 0x10
            && segment.acknowledgment == operation.local_sequence.wrapping_add(1);
        if !acknowledgment_valid || segment.sequence != expected_sequence {
            self.passive_tcp[index] = Some(operation);
            return;
        }
        operation.early.peer_window = segment.window;
        if !operation.established && segment.len == 0 && segment.flags & 0x01 == 0 {
            operation.established = true;
            self.passive_tcp[index] = Some(operation);
            return;
        }
        let mut admitted = false;
        if segment.len != 0
            && operation.early.len == 0
            && !operation.early.fin
            && segment.len <= operation.early.bytes.len()
        {
            operation.early.bytes[..segment.len].copy_from_slice(&segment.payload[..segment.len]);
            operation.early.len = segment.len;
            admitted = true;
        }
        if segment.flags & 0x01 != 0 && !operation.early.fin && (segment.len == 0 || admitted) {
            operation.early.fin = true;
            admitted = true;
        }
        if admitted {
            operation.established = true;
            operation.deadline = tick.saturating_add(PASSIVE_TCP_RETRY_TICKS);
            let acknowledgment = operation
                .remote_sequence
                .wrapping_add(1)
                .wrapping_add(operation.early.len as u32)
                .wrapping_add(u32::from(operation.early.fin));
            if self
                .send_tcp(
                    operation.source_mac,
                    operation.target,
                    operation.local_port,
                    operation.remote_port,
                    operation.local_sequence.wrapping_add(1),
                    acknowledgment,
                    0x10,
                    &[],
                )
                .is_none()
            {
                operation.failed = true;
            }
        }
        self.passive_tcp[index] = Some(operation);
    }

    fn poll_tcp_passive(&mut self, tick: u64) -> PassiveTcpProgress {
        if !self.available {
            return PassiveTcpProgress::Idle;
        }
        self.pump_passive_rx(tick);
        let mut any_active = false;
        for index in 0..PASSIVE_TCP_HANDSHAKE_SLOTS {
            let Some(mut operation) = self.passive_tcp[index] else {
                continue;
            };
            any_active = true;
            if operation.failed {
                self.passive_tcp[index] = None;
                return PassiveTcpProgress::Failed(handshake_failure(&operation));
            }
            if operation.established {
                self.passive_tcp[index] = None;
                return PassiveTcpProgress::Established(
                    TcpServerPeer {
                        target: u32::from_be_bytes(operation.target),
                        remote_port: operation.remote_port,
                        local_port: operation.local_port,
                        remote_sequence: operation.remote_sequence.wrapping_add(1),
                        local_sequence: operation.local_sequence.wrapping_add(1),
                        source_mac: operation.source_mac,
                    },
                    operation.early,
                );
            }
            if tick >= operation.deadline {
                if operation.attempts >= RETRIES || self.send_passive_syn_ack(operation).is_none() {
                    self.passive_tcp[index] = None;
                    return PassiveTcpProgress::Failed(handshake_failure(&operation));
                }
                operation.attempts += 1;
                operation.deadline = tick.saturating_add(PASSIVE_TCP_RETRY_TICKS);
                self.passive_tcp[index] = Some(operation);
            }
        }
        for slot in &mut self.pending_syns {
            if let Some(syn) = slot.take() {
                return PassiveTcpProgress::Syn(syn);
            }
        }
        if any_active {
            PassiveTcpProgress::Pending
        } else {
            PassiveTcpProgress::Idle
        }
    }

    fn start_tcp_passive(&mut self, syn: PassiveTcpSyn, tick: u64) -> bool {
        if !self.available
            || self.async_udp.is_some()
            || self.async_tcp.is_some()
            || syn.target == 0
            || syn.remote_port == 0
            || syn.local_port == 0
        {
            return false;
        }
        let target = syn.target.to_be_bytes();
        if self
            .stream_slot_for_tuple(target, syn.remote_port, syn.local_port)
            .is_some()
            || self
                .handshake_slot_for_tuple(target, syn.remote_port, syn.local_port)
                .is_some()
        {
            return false;
        }
        let Some(index) = self.passive_tcp.iter().position(Option::is_none) else {
            return false;
        };
        let operation = PassiveTcpOperation {
            target,
            remote_port: syn.remote_port,
            local_port: syn.local_port,
            remote_sequence: syn.remote_sequence,
            local_sequence: 0x90a0_b0c0u32
                ^ syn.target
                ^ (u32::from(syn.local_port) << 16 | u32::from(syn.remote_port))
                ^ syn.remote_sequence.rotate_left(13),
            source_mac: syn.source_mac,
            established: false,
            failed: false,
            early: EarlyStreamData::empty(),
            attempts: 1,
            deadline: tick.saturating_add(PASSIVE_TCP_RETRY_TICKS),
        };
        if self.send_passive_syn_ack(operation).is_none() {
            return false;
        }
        self.passive_tcp[index] = Some(operation);
        true
    }

    fn reject_tcp_syn(&mut self, syn: PassiveTcpSyn) {
        let _ = self.send_tcp(
            syn.source_mac,
            syn.target.to_be_bytes(),
            syn.local_port,
            syn.remote_port,
            0,
            syn.remote_sequence.wrapping_add(1),
            0x14,
            &[],
        );
    }

    fn reject_tcp_peer(&mut self, peer: TcpServerPeer) {
        let _ = self.send_tcp(
            peer.source_mac,
            peer.target.to_be_bytes(),
            peer.local_port,
            peer.remote_port,
            peer.local_sequence,
            peer.remote_sequence,
            0x14,
            &[],
        );
    }

    fn cancel_tcp_passive(&mut self, failure: PassiveTcpFailure) {
        let Some(index) = self.handshake_slot_for_tuple(
            failure.target.to_be_bytes(),
            failure.remote_port,
            failure.local_port,
        ) else {
            return;
        };
        if let Some(operation) = self.passive_tcp[index].take() {
            let _ = self.send_tcp(
                operation.source_mac,
                operation.target,
                operation.local_port,
                operation.remote_port,
                operation.local_sequence.wrapping_add(1),
                operation.remote_sequence.wrapping_add(1),
                0x14,
                &[],
            );
        }
    }

    fn start_tcp_passive_stream(
        &mut self,
        peer: TcpServerPeer,
        early: EarlyStreamData,
        tick: u64,
    ) -> bool {
        if !self.available
            || self.async_udp.is_some()
            || self.async_tcp.is_some()
            || early.len > PASSIVE_TCP_STREAM_BUFFER_CAPACITY
            || self
                .stream_slot_for_tuple(peer.target.to_be_bytes(), peer.remote_port, peer.local_port)
                .is_some()
        {
            return false;
        }
        let Some(index) = self.passive_streams.iter().position(Option::is_none) else {
            return false;
        };
        // Bytes and a FIN acknowledged by the handshake slot seed the stream
        // exactly once, so the peer's already-acknowledged early segment is
        // delivered instead of silently discarded.
        let mut receive = [0; PASSIVE_TCP_STREAM_BUFFER_CAPACITY];
        receive[..early.len].copy_from_slice(&early.bytes[..early.len]);
        self.passive_streams[index] = Some(PassiveTcpStreamOperation {
            peer,
            remote_sequence: peer
                .remote_sequence
                .wrapping_add(early.len as u32)
                .wrapping_add(u32::from(early.fin)),
            local_sequence: peer.local_sequence,
            receive,
            receive_len: early.len,
            deferred: None,
            send: [0; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
            send_len: 0,
            send_flight: 0,
            send_completed: false,
            fin_sent: false,
            fin_acked: false,
            peer_fin: early.fin,
            peer_fin_pending: early.fin,
            reset: false,
            failed: false,
            attempts: 0,
            deadline: tick.saturating_add(PASSIVE_TCP_STREAM_IDLE_TICKS),
            sent_at: tick,
            rto_ticks: PASSIVE_TCP_RETRY_TICKS,
            peer_window: early.peer_window,
            window_sequence: peer.remote_sequence,
            window_acknowledgment: peer.local_sequence,
            persist_deadline: tick.saturating_add(PASSIVE_TCP_RETRY_TICKS),
            persist_attempts: 0,
            challenge_ack_tick: None,
            congestion_window: PASSIVE_TCP_INITIAL_CWND,
            slow_start_threshold: PASSIVE_TCP_MAX_CWND,
            stream_received_bytes: early.len as u64,
            stream_sent_bytes: 0,
        });
        self.tcp_service_started.get_or_insert(tick);
        self.tcp_payload_bytes = self.tcp_payload_bytes.saturating_add(early.len as u64);
        self.tcp_max_buffered = self.tcp_max_buffered.max(early.len);
        true
    }

    fn handle_stream_segment(&mut self, index: usize, segment: &PassiveSegment, tick: u64) {
        let Some(mut operation) = self.passive_streams[index] else {
            return;
        };
        if segment.flags & 0x04 != 0 {
            if segment.sequence == operation.remote_sequence {
                operation.reset = true;
            } else if sequence_in_window(
                segment.sequence,
                operation.remote_sequence,
                receive_window(&operation),
            ) && operation.challenge_ack_tick != Some(tick)
            {
                // RFC 9293 / RFC 5961: challenge an in-window reset, ignore
                // an out-of-window reset. At most one challenge per tick.
                let _ = self.send_stream_ack(&operation);
                operation.challenge_ack_tick = Some(tick);
            }
            self.passive_streams[index] = Some(operation);
            return;
        }
        if segment.flags & 0x02 != 0 {
            if operation.challenge_ack_tick != Some(tick) {
                let _ = self.send_stream_ack(&operation);
                operation.challenge_ack_tick = Some(tick);
                self.passive_streams[index] = Some(operation);
            }
            return;
        }
        let sequence_valid = sequence_in_window(
            segment.sequence,
            operation.remote_sequence,
            receive_window(&operation),
        );
        let mut acknowledgment_valid = false;
        if segment.flags & 0x10 != 0 && sequence_valid {
            if segment.acknowledgment == operation.local_sequence {
                acknowledgment_valid = true;
            } else if operation.send_flight != 0
                && segment.acknowledgment
                    == operation
                        .local_sequence
                        .wrapping_add(operation.send_flight as u32)
            {
                let sample = tick.saturating_sub(operation.sent_at).max(1);
                self.tcp_max_ack_latency = self.tcp_max_ack_latency.max(sample);
                // Karn's algorithm: an ACK after retransmission cannot tell
                // which transmission supplied the measured round-trip time.
                if operation.attempts == 1 {
                    operation.rto_ticks = ((operation.rto_ticks.saturating_mul(3))
                        .saturating_add(sample.saturating_mul(2))
                        / 4)
                    .clamp(4, PASSIVE_TCP_RETRY_TICKS * 2);
                }
                operation.local_sequence = segment.acknowledgment;
                operation.stream_sent_bytes = operation
                    .stream_sent_bytes
                    .saturating_add(operation.send_flight as u64);
                let additive_step = (PASSIVE_TCP_MSS * PASSIVE_TCP_MSS)
                    .checked_div(operation.congestion_window.max(1))
                    .unwrap_or(1)
                    .max(1);
                operation.congestion_window =
                    if operation.congestion_window < operation.slow_start_threshold {
                        operation.congestion_window.saturating_add(PASSIVE_TCP_MSS)
                    } else {
                        operation.congestion_window.saturating_add(additive_step)
                    }
                    .min(PASSIVE_TCP_MAX_CWND);
                self.tcp_max_cwnd = self.tcp_max_cwnd.max(operation.congestion_window);
                self.tcp_max_stream_bytes = self.tcp_max_stream_bytes.max(
                    operation
                        .stream_received_bytes
                        .max(operation.stream_sent_bytes),
                );
                operation
                    .send
                    .copy_within(operation.send_flight..operation.send_len, 0);
                operation.send_len -= operation.send_flight;
                operation.send[operation.send_len..].fill(0);
                operation.send_flight = 0;
                operation.send_completed = operation.send_len == 0;
                operation.attempts = 0;
                refresh_stream_idle(&mut operation, tick);
                acknowledgment_valid = true;
            } else if operation.fin_sent
                && !operation.fin_acked
                && segment.acknowledgment == operation.local_sequence.wrapping_add(1)
            {
                operation.local_sequence = segment.acknowledgment;
                operation.fin_acked = true;
                operation.attempts = 0;
                refresh_stream_idle(&mut operation, tick);
                acknowledgment_valid = true;
            }
            if acknowledgment_valid
                && (sequence_after(segment.sequence, operation.window_sequence)
                    || (segment.sequence == operation.window_sequence
                        && !sequence_after(
                            operation.window_acknowledgment,
                            segment.acknowledgment,
                        )))
            {
                operation.peer_window = segment.window;
                operation.window_sequence = segment.sequence;
                operation.window_acknowledgment = segment.acknowledgment;
            }
        }

        let mut acknowledge = false;
        if (segment.len != 0 || segment.flags & 0x01 != 0) && !acknowledgment_valid {
            // Duplicate / out-of-window bytes never mutate state, but the
            // current ACK lets a peer recover from a lost acknowledgment.
            let _ = self.send_stream_ack(&operation);
            self.passive_streams[index] = Some(operation);
            return;
        }
        if (operation.peer_fin || operation.deferred.is_some_and(|d| d.acknowledged && d.fin))
            && (segment.len != 0 || segment.flags & 0x01 != 0)
        {
            let _ = self.send_stream_ack(&operation);
            self.passive_streams[index] = Some(operation);
            return;
        }
        if segment.len > operation.receive.len() {
            let _ = self.send_tcp(
                operation.peer.source_mac,
                operation.peer.target.to_be_bytes(),
                operation.peer.local_port,
                operation.peer.remote_port,
                operation.local_sequence,
                operation.remote_sequence,
                0x14,
                &[],
            );
            operation.failed = true;
            self.passive_streams[index] = Some(operation);
            return;
        }
        let mut deferred_fin = false;
        if receive_window(&operation) == 0 && (segment.len != 0 || segment.flags & 0x01 != 0) {
            let _ = self.send_stream_ack(&operation);
            self.passive_streams[index] = Some(operation);
            return;
        }
        if segment.len != 0
            && operation.deferred.is_some_and(|d| {
                !d.acknowledged
                    && segment.sequence == operation.remote_sequence
                    && segment.len as u32 > d.sequence.wrapping_sub(operation.remote_sequence)
            })
        {
            // Keep the first accepted bytes; partial overlap/merging requires
            // a larger reassembly contract than this single deferred slot.
            let _ = self.send_stream_ack(&operation);
            self.passive_streams[index] = Some(operation);
            return;
        }
        if segment.len != 0 {
            if segment.sequence == operation.remote_sequence && operation.receive_len == 0 {
                operation.receive[..segment.len].copy_from_slice(&segment.payload[..segment.len]);
                operation.receive_len = segment.len;
                operation.stream_received_bytes = operation
                    .stream_received_bytes
                    .saturating_add(segment.len as u64);
                operation.remote_sequence =
                    operation.remote_sequence.wrapping_add(segment.len as u32);
                self.tcp_payload_bytes = self.tcp_payload_bytes.saturating_add(segment.len as u64);
                refresh_stream_idle(&mut operation, tick);
                if let Some(mut deferred) = operation.deferred {
                    if deferred.sequence == operation.remote_sequence {
                        operation.remote_sequence = operation
                            .remote_sequence
                            .wrapping_add(deferred.len as u32)
                            .wrapping_add(u32::from(deferred.fin));
                        self.tcp_reordered_segments = self.tcp_reordered_segments.saturating_add(1);
                        deferred.acknowledged = true;
                        operation.deferred = Some(deferred);
                    }
                }
            } else if operation.deferred.is_none()
                && segment
                    .sequence
                    .wrapping_sub(operation.remote_sequence)
                    .saturating_add(segment.len as u32)
                    .saturating_add(u32::from(segment.flags & 0x01 != 0))
                    <= u32::from(receive_window(&operation))
            {
                let mut bytes = [0; PASSIVE_TCP_STREAM_BUFFER_CAPACITY];
                bytes[..segment.len].copy_from_slice(&segment.payload[..segment.len]);
                deferred_fin = segment.flags & 0x01 != 0;
                operation.deferred = Some(DeferredTcpSegment {
                    sequence: segment.sequence,
                    bytes,
                    len: segment.len,
                    fin: deferred_fin,
                    acknowledged: segment.sequence == operation.remote_sequence,
                });
                operation.stream_received_bytes = operation
                    .stream_received_bytes
                    .saturating_add(segment.len as u64);
                // A contiguous deferred segment is safely buffered and can be
                // cumulatively acknowledged even while Ring 3 owns the first
                // receive buffer. A segment beyond a gap is held but not ACKed
                // past the missing sequence.
                if segment.sequence == operation.remote_sequence {
                    operation.remote_sequence = operation
                        .remote_sequence
                        .wrapping_add(segment.len as u32)
                        .wrapping_add(u32::from(deferred_fin));
                }
                self.tcp_payload_bytes = self.tcp_payload_bytes.saturating_add(segment.len as u64);
                refresh_stream_idle(&mut operation, tick);
            }
            self.tcp_max_buffered = self
                .tcp_max_buffered
                .max(operation.receive_len + operation.deferred.map_or(0, |deferred| deferred.len));
            self.tcp_max_stream_bytes = self
                .tcp_max_stream_bytes
                .max(operation.stream_received_bytes);
            acknowledge = true;
        }
        if segment.flags & 0x01 != 0 && !deferred_fin {
            let fin_sequence = segment.sequence.wrapping_add(segment.len as u32);
            if fin_sequence == operation.remote_sequence {
                operation.remote_sequence = operation.remote_sequence.wrapping_add(1);
                operation.peer_fin = true;
                operation.peer_fin_pending = true;
                refresh_stream_idle(&mut operation, tick);
            }
            acknowledge = true;
        }
        if acknowledge
            && self
                .send_tcp_with_window(
                    operation.peer.source_mac,
                    operation.peer.target.to_be_bytes(),
                    operation.peer.local_port,
                    operation.peer.remote_port,
                    operation.local_sequence,
                    operation.remote_sequence,
                    0x10,
                    &[],
                    receive_window(&operation),
                )
                .is_none()
        {
            operation.failed = true;
        }
        self.passive_streams[index] = Some(operation);
    }

    fn poll_tcp_passive_stream(&mut self, tick: u64) -> PassiveTcpStreamProgress {
        self.maybe_report_regression_budgets(tick);
        self.pump_passive_rx(tick);
        let mut any_active = false;
        for offset in 0..PASSIVE_TCP_STREAM_SLOTS {
            let index = (self.passive_stream_cursor + offset) % PASSIVE_TCP_STREAM_SLOTS;
            let Some(mut operation) = self.passive_streams[index] else {
                continue;
            };
            any_active = true;
            if operation.reset {
                self.passive_streams[index] = None;
                self.passive_stream_cursor = (index + 1) % PASSIVE_TCP_STREAM_SLOTS;
                return PassiveTcpStreamProgress::Reset(operation.peer);
            }
            if operation.failed {
                self.passive_streams[index] = None;
                self.passive_stream_cursor = (index + 1) % PASSIVE_TCP_STREAM_SLOTS;
                return PassiveTcpStreamProgress::Failed(operation.peer);
            }
            if tick >= operation.deadline
                && operation.send_flight == 0
                && !(operation.fin_sent && !operation.fin_acked)
            {
                let _ = self.send_tcp(
                    operation.peer.source_mac,
                    operation.peer.target.to_be_bytes(),
                    operation.peer.local_port,
                    operation.peer.remote_port,
                    operation.local_sequence,
                    operation.remote_sequence,
                    0x14,
                    &[],
                );
                self.passive_streams[index] = None;
                self.passive_stream_cursor = (index + 1) % PASSIVE_TCP_STREAM_SLOTS;
                return PassiveTcpStreamProgress::Failed(operation.peer);
            }
            if operation.send_len != 0 && operation.send_flight == 0 {
                // Queue ownership remains with this exact stream while the
                // peer window is zero. Window updates resume at most one
                // bounded flight per scheduler pass, including small windows.
                if operation.peer_window != 0 {
                    if !self.transmit_stream_flight(&mut operation, tick) {
                        operation.failed = true;
                    }
                } else if tick >= operation.persist_deadline {
                    // Probe one already-consumed sequence number; this cannot
                    // advance either stream sequence or complete user data.
                    // Exponential probes share the hard idle lifetime, but
                    // never count as congestion loss or normal data retries.
                    if self
                        .send_tcp_with_window(
                            operation.peer.source_mac,
                            operation.peer.target.to_be_bytes(),
                            operation.peer.local_port,
                            operation.peer.remote_port,
                            operation.local_sequence.wrapping_sub(1),
                            operation.remote_sequence,
                            0x10,
                            &[0],
                            receive_window(&operation),
                        )
                        .is_none()
                    {
                        operation.failed = true;
                    }
                    operation.persist_attempts = (operation.persist_attempts + 1).min(RETRIES);
                    operation.persist_deadline = tick.saturating_add(
                        operation
                            .rto_ticks
                            .saturating_mul(1 << operation.persist_attempts)
                            .min(PASSIVE_TCP_STREAM_IDLE_TICKS),
                    );
                }
                self.passive_streams[index] = Some(operation);
            }
            if tick >= operation.deadline {
                let retry = if operation.send_flight != 0 {
                    Some((0x18, operation.send_flight))
                } else if operation.fin_sent && !operation.fin_acked {
                    Some((0x11, 0))
                } else {
                    None
                };
                if let Some((flags, payload_len)) = retry {
                    let payload = operation.send;
                    if operation.attempts >= RETRIES
                        || self
                            .send_tcp_with_window(
                                operation.peer.source_mac,
                                operation.peer.target.to_be_bytes(),
                                operation.peer.local_port,
                                operation.peer.remote_port,
                                operation.local_sequence,
                                operation.remote_sequence,
                                flags,
                                &payload[..payload_len],
                                receive_window(&operation),
                            )
                            .is_none()
                    {
                        self.passive_streams[index] = None;
                        self.passive_stream_cursor = (index + 1) % PASSIVE_TCP_STREAM_SLOTS;
                        return PassiveTcpStreamProgress::Failed(operation.peer);
                    }
                    self.tcp_retransmissions = self.tcp_retransmissions.saturating_add(1);
                    operation.slow_start_threshold = (operation.congestion_window / 2)
                        .clamp(PASSIVE_TCP_MSS, PASSIVE_TCP_MAX_CWND);
                    operation.congestion_window = PASSIVE_TCP_MSS;
                    self.tcp_min_cwnd = self.tcp_min_cwnd.min(operation.congestion_window);
                    self.tcp_congestion_events = self.tcp_congestion_events.saturating_add(1);
                    serial::println("TCP_CONGESTION_BACKOFF");
                    operation.attempts += 1;
                    operation.sent_at = tick;
                    operation.rto_ticks = operation
                        .rto_ticks
                        .saturating_mul(2)
                        .min(PASSIVE_TCP_RETRY_TICKS * 4);
                    operation.deadline = tick.saturating_add(operation.rto_ticks);
                    self.passive_streams[index] = Some(operation);
                }
            }
            if operation.receive_len != 0 {
                self.passive_stream_cursor = (index + 1) % PASSIVE_TCP_STREAM_SLOTS;
                return PassiveTcpStreamProgress::Received {
                    peer: operation.peer,
                    bytes: operation.receive,
                    len: operation.receive_len,
                };
            }
            if operation.send_completed {
                self.passive_stream_cursor = (index + 1) % PASSIVE_TCP_STREAM_SLOTS;
                return PassiveTcpStreamProgress::SendComplete(operation.peer);
            }
            if operation.peer_fin && operation.fin_sent && operation.fin_acked {
                self.tcp_service_finished = tick;
                self.passive_stream_cursor = (index + 1) % PASSIVE_TCP_STREAM_SLOTS;
                return PassiveTcpStreamProgress::Closed(operation.peer);
            }
            if operation.peer_fin_pending {
                self.passive_stream_cursor = (index + 1) % PASSIVE_TCP_STREAM_SLOTS;
                return PassiveTcpStreamProgress::PeerClosed(operation.peer);
            }
        }
        if any_active {
            PassiveTcpStreamProgress::Pending
        } else {
            PassiveTcpStreamProgress::Idle
        }
    }

    fn maybe_report_regression_budgets(&mut self, tick: u64) {
        #[cfg(feature = "network-test-faults")]
        {
            if self.regression_reported || self.tcp_service_finished == 0 {
                return;
            }
            let Some(started) = self.tcp_service_started else {
                return;
            };
            let elapsed = self.tcp_service_finished.saturating_sub(started).max(1);
            let device = crate::network_device::metrics();
            let device_frames = device.rx_frames.saturating_add(device.tx_frames);
            let interrupt_completions = device
                .rx_interrupt_completions
                .saturating_add(device.tx_interrupt_completions);
            let throughput = self.tcp_payload_bytes.saturating_mul(1_000) / elapsed;
            if !self.regression_sample_reported
                && self.tcp_max_stream_bytes >= PASSIVE_TCP_MAX_CWND as u64
                && self.passive_streams.iter().all(Option::is_none)
            {
                self.regression_sample_reported = true;
                serial::print("NETWORK_REGRESSION_SAMPLE bytes=");
                serial::print_u64(self.tcp_payload_bytes);
                serial::print(" elapsed=");
                serial::print_u64(elapsed);
                serial::print(" retrans=");
                serial::print_u64(self.tcp_retransmissions);
                serial::print(" reorder=");
                serial::print_u64(self.tcp_reordered_segments);
                serial::print(" max_stream=");
                serial::print_u64(self.tcp_max_stream_bytes);
                serial::print(" congestion=");
                serial::print_u64(self.tcp_congestion_events);
                serial::print(" cwnd_min=");
                serial::print_u64(self.tcp_min_cwnd as u64);
                serial::print(" cwnd_max=");
                serial::print_u64(self.tcp_max_cwnd as u64);
                serial::print(" ack=");
                serial::print_u64(self.tcp_max_ack_latency);
                serial::print(" irqs=");
                serial::print_u64(device.interrupts);
                serial::print(" rx_irq=");
                serial::print_u64(device.rx_interrupt_completions);
                serial::print(" tx_irq=");
                serial::print_u64(device.tx_interrupt_completions);
                serial::print(" recovery=");
                serial::print_u64(device.recovery_completions);
                serial::print(" polls=");
                serial::print_u64(device.recovery_polls);
                serial::print(" frames=");
                serial::print_u64(device_frames);
                serial::print(" notifications=");
                serial::print_u64(device.queue_notifications);
                serial::println("");
            }
            let within_budget = self.tcp_retransmissions >= 1
                && self.tcp_retransmissions <= 12
                && self.tcp_reordered_segments >= 1
                && self.tcp_max_buffered <= PASSIVE_TCP_RECEIVE_CAPACITY
                && self.tcp_max_stream_bytes >= PASSIVE_TCP_MAX_CWND as u64
                && self.tcp_congestion_events >= 3
                && self.tcp_min_cwnd == PASSIVE_TCP_MSS
                && self.tcp_max_cwnd > PASSIVE_TCP_MSS
                && self.tcp_max_cwnd <= PASSIVE_TCP_MAX_CWND
                && self.tcp_max_ack_latency <= PASSIVE_TCP_RETRY_TICKS * 4
                && elapsed <= PASSIVE_TCP_STREAM_IDLE_TICKS * 4
                && throughput > 0
                && device.interrupts > 0
                && device.rx_interrupt_completions > 0
                && device.tx_interrupt_completions > 0
                && device.recovery_completions <= device_frames / 16 + 1
                && interrupt_completions > device.recovery_completions.saturating_mul(8)
                && device_frames > 0
                && device.queue_notifications <= device_frames.saturating_mul(3)
                && device.recovery_polls <= device_frames.saturating_mul(128);
            if !within_budget {
                return;
            }
            self.regression_reported = true;
            serial::print("NETWORK_REGRESSION_BUDGET_OK bytes=");
            serial::print_u64(self.tcp_payload_bytes);
            serial::print(" elapsed_ticks=");
            serial::print_u64(elapsed);
            serial::print(" throughput_milli_bytes_per_tick=");
            serial::print_u64(throughput);
            serial::print(" max_ack_ticks=");
            serial::print_u64(self.tcp_max_ack_latency);
            serial::print(" retransmissions=");
            serial::print_u64(self.tcp_retransmissions);
            serial::print(" reordered=");
            serial::print_u64(self.tcp_reordered_segments);
            serial::print(" max_buffered=");
            serial::print_u64(self.tcp_max_buffered as u64);
            serial::print(" max_stream_bytes=");
            serial::print_u64(self.tcp_max_stream_bytes);
            serial::print(" congestion_events=");
            serial::print_u64(self.tcp_congestion_events);
            serial::print(" cwnd_min=");
            serial::print_u64(self.tcp_min_cwnd as u64);
            serial::print(" cwnd_max=");
            serial::print_u64(self.tcp_max_cwnd as u64);
            serial::print(" irq_completions=");
            serial::print_u64(interrupt_completions);
            serial::print(" recovery_completions=");
            serial::print_u64(device.recovery_completions);
            serial::print(" recovery_polls=");
            serial::print_u64(device.recovery_polls);
            serial::print(" frames=");
            serial::print_u64(device_frames);
            serial::print(" notifications=");
            serial::print_u64(device.queue_notifications);
            serial::print(" queue_capacity=");
            serial::print_u64(crate::network_device::VIRTIO_QUEUE_CAPACITY as u64);
            serial::print(" observed_tick=");
            serial::print_u64(tick);
            serial::println("");
            serial::println("TCP_STREAM_LARGE_READY bytes=1024 bounded_window=256");
            serial::println("TCP_CONGESTION_CONTROL_READY algorithm=aimd max_cwnd=1024");
        }
        #[cfg(not(feature = "network-test-faults"))]
        let _ = tick;
    }

    fn start_tcp_passive_stream_send(
        &mut self,
        peer: TcpServerPeer,
        bytes: &[u8],
        tick: u64,
    ) -> bool {
        let Some(index) = self.stream_slot(peer) else {
            return false;
        };
        let Some(mut operation) = self.passive_streams[index] else {
            return false;
        };
        if bytes.is_empty()
            || bytes.len() > operation.send.len()
            || bytes.len() > operation.congestion_window
            || operation.send_len != 0
            || operation.send_completed
            || operation.fin_sent
        {
            return false;
        }
        operation.send[..bytes.len()].copy_from_slice(bytes);
        operation.send_len = bytes.len();
        operation.deadline = tick.saturating_add(PASSIVE_TCP_STREAM_IDLE_TICKS);
        operation.persist_deadline = tick.saturating_add(operation.rto_ticks);
        operation.persist_attempts = 0;
        let sent = operation.peer_window == 0 || self.transmit_stream_flight(&mut operation, tick);
        // A failed device publication cannot leave a live phantom flight.
        if !sent {
            operation.failed = true;
        }
        self.passive_streams[index] = Some(operation);
        sent
    }

    fn transmit_stream_flight(
        &mut self,
        operation: &mut PassiveTcpStreamOperation,
        tick: u64,
    ) -> bool {
        let len = operation
            .send_len
            .min(usize::from(operation.peer_window))
            .min(operation.congestion_window);
        if len == 0 || operation.send_flight != 0 {
            return false;
        }
        operation.send_flight = len;
        operation.attempts = 1;
        operation.sent_at = tick;
        operation.deadline = tick.saturating_add(operation.rto_ticks);
        #[cfg(feature = "network-test-faults")]
        let inject_loss = operation.send[..len].starts_with(b"GENOS_PONG_LOSS");
        #[cfg(not(feature = "network-test-faults"))]
        let inject_loss = false;
        if inject_loss {
            serial::println("TCP_FAULT_DATA_DROP_INJECTED");
            true
        } else {
            self.send_tcp_with_window(
                operation.peer.source_mac,
                operation.peer.target.to_be_bytes(),
                operation.peer.local_port,
                operation.peer.remote_port,
                operation.local_sequence,
                operation.remote_sequence,
                0x18,
                &operation.send[..len],
                receive_window(operation),
            )
            .is_some()
        }
    }

    fn send_stream_ack(&mut self, operation: &PassiveTcpStreamOperation) -> Option<()> {
        self.send_tcp_with_window(
            operation.peer.source_mac,
            operation.peer.target.to_be_bytes(),
            operation.peer.local_port,
            operation.peer.remote_port,
            operation.local_sequence,
            operation.remote_sequence,
            0x10,
            &[],
            receive_window(operation),
        )
    }

    fn consume_tcp_passive_stream_receive(&mut self, peer: TcpServerPeer) -> bool {
        let Some(index) = self.stream_slot(peer) else {
            return false;
        };
        let Some(mut operation) = self.passive_streams[index] else {
            return false;
        };
        if operation.receive_len == 0 {
            return false;
        }
        operation.receive[..operation.receive_len].fill(0);
        operation.receive_len = 0;
        if let Some(deferred) = operation.deferred.filter(|segment| segment.acknowledged) {
            operation.deferred = None;
            operation.receive[..deferred.len].copy_from_slice(&deferred.bytes[..deferred.len]);
            operation.receive_len = deferred.len;
            if deferred.fin {
                operation.peer_fin = true;
                operation.peer_fin_pending = true;
            }
        }
        let window = receive_window(&operation);
        let _ = self.send_tcp_with_window(
            operation.peer.source_mac,
            operation.peer.target.to_be_bytes(),
            operation.peer.local_port,
            operation.peer.remote_port,
            operation.local_sequence,
            operation.remote_sequence,
            0x10,
            &[],
            window,
        );
        self.passive_streams[index] = Some(operation);
        true
    }

    fn consume_tcp_passive_stream_send(&mut self, peer: TcpServerPeer) -> bool {
        let Some(index) = self.stream_slot(peer) else {
            return false;
        };
        self.passive_streams[index]
            .as_mut()
            .is_some_and(|operation| {
                if !operation.send_completed {
                    return false;
                }
                operation.send.fill(0);
                operation.send_completed = false;
                true
            })
    }

    fn consume_tcp_passive_peer_close(&mut self, peer: TcpServerPeer) -> bool {
        let Some(index) = self.stream_slot(peer) else {
            return false;
        };
        self.passive_streams[index]
            .as_mut()
            .is_some_and(|operation| {
                if !operation.peer_fin_pending {
                    return false;
                }
                operation.peer_fin_pending = false;
                true
            })
    }

    fn start_tcp_passive_stream_close(&mut self, peer: TcpServerPeer, tick: u64) -> bool {
        let Some(index) = self.stream_slot(peer) else {
            return false;
        };
        let Some(mut operation) = self.passive_streams[index] else {
            return false;
        };
        if operation.send_len != 0 || operation.send_completed || operation.fin_sent {
            return false;
        }
        operation.fin_sent = true;
        operation.attempts = 1;
        operation.deadline = tick.saturating_add(PASSIVE_TCP_RETRY_TICKS);
        let sent = self
            .send_tcp(
                peer.source_mac,
                peer.target.to_be_bytes(),
                peer.local_port,
                peer.remote_port,
                operation.local_sequence,
                operation.remote_sequence,
                0x11,
                &[],
            )
            .is_some();
        self.passive_streams[index] = Some(operation);
        sent
    }

    fn finish_tcp_passive_stream(&mut self, peer: TcpServerPeer) -> bool {
        let Some(index) = self.stream_slot(peer) else {
            return false;
        };
        self.passive_streams[index] = None;
        true
    }

    fn cancel_tcp_passive_stream(&mut self, peer: TcpServerPeer) {
        let Some(index) = self.stream_slot(peer) else {
            return;
        };
        if let Some(operation) = self.passive_streams[index].take() {
            let _ = self.send_tcp(
                operation.peer.source_mac,
                operation.peer.target.to_be_bytes(),
                operation.peer.local_port,
                operation.peer.remote_port,
                operation.local_sequence,
                operation.remote_sequence,
                0x14,
                &[],
            );
        }
    }

    fn decode_passive_segment(&self) -> Option<PassiveSegment> {
        let ip = parse_ipv4_frame(&self.rx.bytes[..self.rx.len]).filter(|ip| {
            ip.protocol == 6
                && ip.destination == self.address
                && net::transport_checksum_valid(ip.source, ip.destination, ip.protocol, ip.payload)
        })?;
        let tcp = parse_tcp(ip.payload)?;
        let mut segment = PassiveSegment {
            source: ip.source,
            source_mac: ip.source_mac,
            remote_port: tcp.source_port,
            local_port: tcp.destination_port,
            sequence: tcp.sequence,
            acknowledgment: tcp.acknowledgment,
            flags: tcp.flags,
            window: tcp.window,
            payload: [0; 1400],
            len: tcp.payload.len().min(1400),
        };
        segment.payload[..segment.len].copy_from_slice(&tcp.payload[..segment.len]);
        Some(segment)
    }

    fn send_passive_syn_ack(&mut self, operation: PassiveTcpOperation) -> Option<()> {
        self.send_tcp(
            operation.source_mac,
            operation.target,
            operation.local_port,
            operation.remote_port,
            operation.local_sequence,
            operation.remote_sequence.wrapping_add(1),
            0x12,
            &[],
        )
    }

    fn decode_tcp_reply(
        &self,
        target: [u8; 4],
        remote_port: u16,
        local_port: u16,
    ) -> Option<TcpOwned> {
        parse_ipv4_frame(&self.rx.bytes[..self.rx.len])
            .filter(|ip| {
                ip.protocol == 6
                    && ip.source == target
                    && ip.destination == self.address
                    && net::transport_checksum_valid(
                        ip.source,
                        ip.destination,
                        ip.protocol,
                        ip.payload,
                    )
            })
            .and_then(|ip| parse_tcp(ip.payload))
            .filter(|tcp| tcp.source_port == remote_port && tcp.destination_port == local_port)
            .map(TcpOwned::from_packet)
    }

    fn send_arp_request(&mut self, target: [u8; 4]) -> bool {
        let mut frame = [0u8; 42];
        frame[..6].fill(0xff);
        frame[6..12].copy_from_slice(&self.device.mac());
        frame[12..14].copy_from_slice(&0x0806u16.to_be_bytes());
        frame[14..16].copy_from_slice(&1u16.to_be_bytes());
        frame[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
        frame[18] = 6;
        frame[19] = 4;
        frame[20..22].copy_from_slice(&1u16.to_be_bytes());
        frame[22..28].copy_from_slice(&self.device.mac());
        frame[28..32].copy_from_slice(&self.address);
        frame[38..42].copy_from_slice(&target);
        self.device.transmit(&frame)
    }

    fn resolve_arp(&mut self, target: [u8; 4]) -> Option<[u8; 6]> {
        for _ in 0..RETRIES {
            if !self.send_arp_request(target) {
                return None;
            }
            for _ in 0..POLL_LIMIT / RETRIES {
                if !self.receive_ipv4() {
                    continue;
                }
                let result =
                    net::parse_arp_reply(&self.rx.bytes[..self.rx.len], target, self.address);
                self.rx.owner = PacketOwner::Free;
                if result.is_some() {
                    return result;
                }
            }
        }
        None
    }

    fn send_udp_raw(
        &mut self,
        mac: [u8; 6],
        source: [u8; 4],
        destination: [u8; 4],
        source_port: u16,
        destination_port: u16,
        payload: &[u8],
    ) -> Option<()> {
        let mut udp = [0u8; 1400];
        let len = 8usize.checked_add(payload.len())?;
        if len > udp.len() {
            return None;
        }
        udp[0..2].copy_from_slice(&source_port.to_be_bytes());
        udp[2..4].copy_from_slice(&destination_port.to_be_bytes());
        udp[4..6].copy_from_slice(&(len as u16).to_be_bytes());
        udp[8..len].copy_from_slice(payload);
        self.send_ipv4_from(mac, source, destination, 17, &udp[..len])
            .then_some(())
    }

    fn send_ipv4(
        &mut self,
        mac: [u8; 6],
        destination: [u8; 4],
        protocol: u8,
        payload: &[u8],
    ) -> bool {
        self.send_ipv4_from(mac, self.address, destination, protocol, payload)
    }

    fn send_ipv4_from(
        &mut self,
        mac: [u8; 6],
        source: [u8; 4],
        destination: [u8; 4],
        protocol: u8,
        payload: &[u8],
    ) -> bool {
        let total = 20 + payload.len();
        if 14 + total > MAX_FRAME || total > u16::MAX as usize {
            return false;
        }
        let mut frame = [0u8; MAX_FRAME];
        frame[..6].copy_from_slice(&mac);
        frame[6..12].copy_from_slice(&self.device.mac());
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        let ip = &mut frame[14..14 + total];
        ip[0] = 0x45;
        ip[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        ip[4..6].copy_from_slice(&self.ip_id.to_be_bytes());
        self.ip_id = self.ip_id.wrapping_add(1);
        ip[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
        ip[8] = 64;
        ip[9] = protocol;
        ip[12..16].copy_from_slice(&source);
        ip[16..20].copy_from_slice(&destination);
        let sum = net::checksum(&ip[..20]);
        ip[10..12].copy_from_slice(&sum.to_be_bytes());
        ip[20..].copy_from_slice(payload);
        self.device.transmit(&frame[..14 + total])
    }

    #[allow(clippy::too_many_arguments)]
    fn send_tcp(
        &mut self,
        mac: [u8; 6],
        destination: [u8; 4],
        source_port: u16,
        destination_port: u16,
        sequence: u32,
        acknowledgment: u32,
        flags: u8,
        payload: &[u8],
    ) -> Option<()> {
        self.send_tcp_with_window(
            mac,
            destination,
            source_port,
            destination_port,
            sequence,
            acknowledgment,
            flags,
            payload,
            PASSIVE_TCP_RECEIVE_CAPACITY as u16,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn send_tcp_with_window(
        &mut self,
        mac: [u8; 6],
        destination: [u8; 4],
        source_port: u16,
        destination_port: u16,
        sequence: u32,
        acknowledgment: u32,
        flags: u8,
        payload: &[u8],
        receive_window: u16,
    ) -> Option<()> {
        let len = 20usize.checked_add(payload.len())?;
        let mut segment = [0u8; 1400];
        if len > segment.len() {
            return None;
        }
        segment[0..2].copy_from_slice(&source_port.to_be_bytes());
        segment[2..4].copy_from_slice(&destination_port.to_be_bytes());
        segment[4..8].copy_from_slice(&sequence.to_be_bytes());
        segment[8..12].copy_from_slice(&acknowledgment.to_be_bytes());
        segment[12] = 5 << 4;
        segment[13] = flags;
        segment[14..16].copy_from_slice(&receive_window.to_be_bytes());
        segment[20..len].copy_from_slice(payload);
        let checksum = transport_checksum(self.address, destination, 6, &segment[..len]);
        segment[16..18].copy_from_slice(&checksum.to_be_bytes());
        self.send_ipv4(mac, destination, 6, &segment[..len])
            .then_some(())
    }

    fn allocate_port(&mut self) -> u16 {
        let port = self.next_port;
        self.next_port = self.next_port.wrapping_add(1).max(49152);
        port
    }
}

fn receive_window(operation: &PassiveTcpStreamOperation) -> u16 {
    let buffered = operation
        .receive_len
        .saturating_add(operation.deferred.map_or(0, |deferred| deferred.len));
    PASSIVE_TCP_RECEIVE_CAPACITY
        .saturating_sub(buffered)
        .min(u16::MAX as usize) as u16
}

fn sequence_after(value: u32, reference: u32) -> bool {
    (value.wrapping_sub(reference) as i32) > 0
}

fn refresh_stream_idle(operation: &mut PassiveTcpStreamOperation, tick: u64) {
    // Receiving bytes must never postpone a pending data/FIN retransmission.
    if operation.send_flight == 0 && !(operation.fin_sent && !operation.fin_acked) {
        operation.deadline = tick.saturating_add(PASSIVE_TCP_STREAM_IDLE_TICKS);
    }
}

fn sequence_in_window(sequence: u32, next: u32, window: u16) -> bool {
    // Zero-window control packets are acceptable only at RCV.NXT. All
    // comparisons stay valid across u32 sequence wrap with bounded windows.
    sequence.wrapping_sub(next) < u32::from(window).max(1)
}

fn handshake_failure(operation: &PassiveTcpOperation) -> PassiveTcpFailure {
    PassiveTcpFailure {
        target: u32::from_be_bytes(operation.target),
        remote_port: operation.remote_port,
        local_port: operation.local_port,
    }
}

#[derive(Clone, Copy)]
struct DhcpConfig {
    address: [u8; 4],
    subnet: [u8; 4],
    gateway: [u8; 4],
    dns: [u8; 4],
    server: [u8; 4],
}

#[derive(Clone, Copy)]
struct TcpOwned {
    sequence: u32,
    acknowledgment: u32,
    flags: u8,
    payload: [u8; 1400],
    len: usize,
}

impl TcpOwned {
    fn from_packet(packet: net::TcpPacket<'_>) -> Self {
        let mut owned = Self {
            sequence: packet.sequence,
            acknowledgment: packet.acknowledgment,
            flags: packet.flags,
            payload: [0; 1400],
            len: packet.payload.len().min(1400),
        };
        owned.payload[..owned.len].copy_from_slice(&packet.payload[..owned.len]);
        owned
    }

    fn payload(&self) -> &[u8] {
        &self.payload[..self.len]
    }
}

static mut NETWORK: NetworkStack = NetworkStack::new();

pub fn init() {
    let stack = unsafe { &mut *addr_of_mut!(NETWORK) };
    if !stack.device.discover() {
        serial::println("NETWORK_DEVICE_UNAVAILABLE");
        return;
    }
    serial::print("NETWORK_DEVICE_READY driver=");
    serial::print(stack.device.driver_name());
    serial::print(" transport=");
    serial::println(stack.device.transport_name());
    serial::print("PACKET_OWNERSHIP_READY buffers=");
    serial::print_u64(stack.device.receive_buffer_count() as u64);
    serial::println(" states=free,driver,stack");
    if !stack.configure() {
        serial::println("NETWORK_DHCP_FAILED");
        return;
    }
    serial::print("NETWORK_DHCP_READY address=");
    print_ip(stack.address);
    serial::print(" gateway=");
    print_ip(stack.gateway);
    serial::print(" dns=");
    print_ip(stack.dns);
    serial::println("");
    serial::println("ETHERNET_ARP_IPV4_UDP_READY");
    if stack.ping(stack.gateway) {
        serial::println("NETWORK_ICMP_ECHO_OK");
    } else {
        serial::println("NETWORK_ICMP_ECHO_FAILED");
        stack.available = false;
    }
    serial::println("NETWORK_TIMEOUT_POLICY_READY retries=3 bounded_poll=true");
    if stack.available {
        stack.ipv6 = Some(ipv6::Host::new(stack.device.mac(), 0));
        serial::println("IPV6_AUTOCONFIG_STARTED");
    }
}

pub fn advance_control(tick: u64) {
    // RuntimeCoordinator is the sole network owner; interrupt handlers only
    // publish device readiness and never access this protocol state.
    unsafe { &mut *addr_of_mut!(NETWORK) }.advance_control(tick);
}

pub fn config() -> Option<UserNetworkConfig> {
    let stack = unsafe { &mut *addr_of_mut!(NETWORK) };
    stack.available.then_some(UserNetworkConfig {
        address: u32::from_be_bytes(stack.address),
        subnet: u32::from_be_bytes(stack.subnet),
        gateway: u32::from_be_bytes(stack.gateway),
        dns: u32::from_be_bytes(stack.dns),
        mac: stack.device.mac(),
        reserved: [0; 2],
    })
}

pub fn udp_exchange(target: u32, port: u16, request: &[u8], response: &mut [u8]) -> Option<usize> {
    unsafe { &mut *addr_of_mut!(NETWORK) }.udp_exchange(
        target.to_be_bytes(),
        port,
        request,
        response,
    )
}

pub fn tcp_exchange(target: u32, port: u16, request: &[u8], response: &mut [u8]) -> Option<usize> {
    unsafe { &mut *addr_of_mut!(NETWORK) }.tcp_exchange(
        target.to_be_bytes(),
        port,
        request,
        response,
    )
}

pub fn start_udp_async(target: u32, port: u16, request: &[u8], tick: u64) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.start_udp_async(
        target.to_be_bytes(),
        port,
        request,
        tick,
    )
}

pub fn poll_udp_async(tick: u64) -> AsyncUdpProgress {
    unsafe { &mut *addr_of_mut!(NETWORK) }.poll_udp_async(tick)
}

pub fn start_tcp_async(target: u32, port: u16, request: &[u8], tick: u64) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.start_tcp_async(
        target.to_be_bytes(),
        port,
        request,
        tick,
    )
}

pub fn poll_tcp_async(tick: u64) -> AsyncTcpProgress {
    unsafe { &mut *addr_of_mut!(NETWORK) }.poll_tcp_async(tick)
}

pub fn poll_tcp_passive(tick: u64) -> PassiveTcpProgress {
    unsafe { &mut *addr_of_mut!(NETWORK) }.poll_tcp_passive(tick)
}

pub fn start_tcp_passive(syn: PassiveTcpSyn, tick: u64) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.start_tcp_passive(syn, tick)
}

pub fn reject_tcp_syn(syn: PassiveTcpSyn) {
    unsafe { &mut *addr_of_mut!(NETWORK) }.reject_tcp_syn(syn);
}

pub fn reject_tcp_peer(peer: TcpServerPeer) {
    unsafe { &mut *addr_of_mut!(NETWORK) }.reject_tcp_peer(peer);
}

pub fn cancel_tcp_passive(failure: PassiveTcpFailure) {
    unsafe { &mut *addr_of_mut!(NETWORK) }.cancel_tcp_passive(failure);
}

pub fn tcp_passive_active() -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }
        .passive_tcp
        .iter()
        .any(Option::is_some)
}

pub fn start_tcp_passive_stream(peer: TcpServerPeer, early: EarlyStreamData, tick: u64) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.start_tcp_passive_stream(peer, early, tick)
}

pub fn poll_tcp_passive_stream(tick: u64) -> PassiveTcpStreamProgress {
    unsafe { &mut *addr_of_mut!(NETWORK) }.poll_tcp_passive_stream(tick)
}

pub fn tcp_passive_stream_peers() -> [Option<TcpServerPeer>; PASSIVE_TCP_STREAM_SLOTS] {
    let stack = unsafe { &mut *addr_of_mut!(NETWORK) };
    let mut peers = [None; PASSIVE_TCP_STREAM_SLOTS];
    for (peer, slot) in peers.iter_mut().zip(stack.passive_streams.iter()) {
        *peer = slot.map(|operation| operation.peer);
    }
    peers
}

pub fn start_tcp_passive_stream_send(peer: TcpServerPeer, bytes: &[u8], tick: u64) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.start_tcp_passive_stream_send(peer, bytes, tick)
}

pub fn consume_tcp_passive_stream_receive(peer: TcpServerPeer) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.consume_tcp_passive_stream_receive(peer)
}

pub fn consume_tcp_passive_stream_send(peer: TcpServerPeer) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.consume_tcp_passive_stream_send(peer)
}

pub fn consume_tcp_passive_peer_close(peer: TcpServerPeer) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.consume_tcp_passive_peer_close(peer)
}

pub fn start_tcp_passive_stream_close(peer: TcpServerPeer, tick: u64) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.start_tcp_passive_stream_close(peer, tick)
}

pub fn finish_tcp_passive_stream(peer: TcpServerPeer) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.finish_tcp_passive_stream(peer)
}

pub fn cancel_tcp_passive_stream(peer: TcpServerPeer) {
    unsafe { &mut *addr_of_mut!(NETWORK) }.cancel_tcp_passive_stream(peer);
}

pub fn cancel_socket_async() {
    let stack = unsafe { &mut *addr_of_mut!(NETWORK) };
    stack.async_udp = None;
    stack.async_tcp = None;
}

fn build_dhcp(
    output: &mut [u8; 300],
    mac: [u8; 6],
    xid: u32,
    message_type: u8,
    requested: Option<[u8; 4]>,
    server: Option<[u8; 4]>,
) -> Option<usize> {
    output.fill(0);
    output[0] = 1;
    output[1] = 1;
    output[2] = 6;
    output[4..8].copy_from_slice(&xid.to_be_bytes());
    output[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    output[28..34].copy_from_slice(&mac);
    output[236..240].copy_from_slice(&[99, 130, 83, 99]);
    let mut cursor = 240;
    push_option(output, &mut cursor, 53, &[message_type])?;
    if let Some(address) = requested {
        push_option(output, &mut cursor, 50, &address)?;
    }
    if let Some(address) = server {
        push_option(output, &mut cursor, 54, &address)?;
    }
    push_option(output, &mut cursor, 55, &[1, 3, 6])?;
    *output.get_mut(cursor)? = 255;
    Some(cursor + 1)
}

fn push_option(output: &mut [u8], cursor: &mut usize, kind: u8, value: &[u8]) -> Option<()> {
    let end = cursor.checked_add(2 + value.len())?;
    if end > output.len() || value.len() > u8::MAX as usize {
        return None;
    }
    output[*cursor] = kind;
    output[*cursor + 1] = value.len() as u8;
    output[*cursor + 2..end].copy_from_slice(value);
    *cursor = end;
    Some(())
}

fn parse_dhcp(payload: &[u8], xid: u32, expected_type: u8) -> Option<DhcpConfig> {
    if payload.len() < 241
        || payload[0] != 2
        || payload[4..8] != xid.to_be_bytes()
        || payload[236..240] != [99, 130, 83, 99]
    {
        return None;
    }
    let address = payload[16..20].try_into().ok()?;
    let mut config = DhcpConfig {
        address,
        subnet: [255, 255, 255, 0],
        gateway: [0; 4],
        dns: [0; 4],
        server: [0; 4],
    };
    let mut message_type = 0;
    let mut cursor = 240;
    while cursor < payload.len() {
        let kind = payload[cursor];
        cursor += 1;
        if kind == 255 {
            break;
        }
        if kind == 0 {
            continue;
        }
        let len = *payload.get(cursor)? as usize;
        cursor += 1;
        let end = cursor
            .checked_add(len)
            .filter(|end| *end <= payload.len())?;
        let value = &payload[cursor..end];
        match (kind, value) {
            (53, [value]) => message_type = *value,
            (1, [a, b, c, d]) => config.subnet = [*a, *b, *c, *d],
            (3, [a, b, c, d, ..]) => config.gateway = [*a, *b, *c, *d],
            (6, [a, b, c, d, ..]) => config.dns = [*a, *b, *c, *d],
            (54, [a, b, c, d]) => config.server = [*a, *b, *c, *d],
            _ => {}
        }
        cursor = end;
    }
    (message_type == expected_type
        && config.address != [0; 4]
        && config.gateway != [0; 4]
        && config.dns != [0; 4]
        && config.server != [0; 4])
        .then_some(config)
}

fn transport_checksum(source: [u8; 4], destination: [u8; 4], protocol: u8, bytes: &[u8]) -> u16 {
    let mut pseudo = [0u8; 1420];
    let len = 12 + bytes.len();
    pseudo[..4].copy_from_slice(&source);
    pseudo[4..8].copy_from_slice(&destination);
    pseudo[9] = protocol;
    pseudo[10..12].copy_from_slice(&(bytes.len() as u16).to_be_bytes());
    pseudo[12..len].copy_from_slice(bytes);
    net::checksum(&pseudo[..len])
}

fn print_ip(address: [u8; 4]) {
    for (index, byte) in address.iter().copied().enumerate() {
        if index > 0 {
            serial::print(".");
        }
        serial::print_u64(u64::from(byte));
    }
}
