use core::ptr::addr_of_mut;

use genos_abi::UserNetworkConfig;
use kernel::{
    net::{self, parse_ipv4_frame, parse_tcp, parse_udp},
    socket::TcpServerPeer,
};

use crate::{
    network_device::{NetworkDevice, PacketBuffer, PacketOwner, MAX_FRAME},
    serial,
};

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
    attempts: usize,
    deadline: u64,
}

#[derive(Clone, Copy)]
struct PassiveTcpStreamOperation {
    peer: TcpServerPeer,
    remote_sequence: u32,
    local_sequence: u32,
    receive: [u8; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
    receive_len: usize,
    send: [u8; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
    send_len: usize,
    send_completed: bool,
    fin_sent: bool,
    fin_acked: bool,
    peer_fin: bool,
    peer_fin_pending: bool,
    attempts: usize,
    deadline: u64,
}

pub enum PassiveTcpProgress {
    Idle,
    Syn(PassiveTcpSyn),
    Pending,
    Established(TcpServerPeer),
    Failed,
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
    async_udp: Option<AsyncUdpOperation>,
    async_tcp: Option<AsyncTcpOperation>,
    passive_tcp: Option<PassiveTcpOperation>,
    passive_stream: Option<PassiveTcpStreamOperation>,
}

impl NetworkStack {
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
            async_udp: None,
            async_tcp: None,
            passive_tcp: None,
            passive_stream: None,
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
            if !self.device.receive(&mut self.rx) {
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
            if !self.device.receive(&mut self.rx) {
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
                if !self.device.receive(&mut self.rx) {
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
        if !self.device.receive(&mut self.rx) {
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

    fn start_udp_async(&mut self, target: [u8; 4], port: u16, request: &[u8], tick: u64) -> bool {
        if !self.available
            || self.async_udp.is_some()
            || self.async_tcp.is_some()
            || self.passive_tcp.is_some()
            || self.passive_stream.is_some()
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
            if self.device.receive(&mut self.rx) {
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
            || self.passive_tcp.is_some()
            || self.passive_stream.is_some()
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
            if self.device.receive(&mut self.rx) {
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

    fn poll_tcp_passive(&mut self, tick: u64) -> PassiveTcpProgress {
        if !self.available || self.passive_stream.is_some() {
            return PassiveTcpProgress::Idle;
        }
        let Some(mut operation) = self.passive_tcp.take() else {
            let mut received = false;
            for _ in 0..PASSIVE_TCP_RX_POLLS_PER_TICK {
                if self.device.receive(&mut self.rx) {
                    received = true;
                    break;
                }
                core::hint::spin_loop();
            }
            if !received {
                return PassiveTcpProgress::Idle;
            }
            let syn = self.decode_passive_syn();
            self.rx.owner = PacketOwner::Free;
            return syn.map_or(PassiveTcpProgress::Idle, PassiveTcpProgress::Syn);
        };

        let mut received = false;
        for _ in 0..PASSIVE_TCP_RX_POLLS_PER_TICK {
            if self.device.receive(&mut self.rx) {
                received = true;
                break;
            }
            core::hint::spin_loop();
        }
        if received {
            let packet = self.decode_tcp_reply(
                operation.target,
                operation.remote_port,
                operation.local_port,
            );
            self.rx.owner = PacketOwner::Free;
            if let Some(packet) = packet {
                if packet.flags & 0x04 != 0 {
                    return PassiveTcpProgress::Failed;
                }
                if packet.flags & 0x3f == 0x02
                    && packet.sequence == operation.remote_sequence
                    && packet.len == 0
                {
                    if self.send_passive_syn_ack(operation).is_none() {
                        return PassiveTcpProgress::Failed;
                    }
                    operation.deadline = tick.saturating_add(PASSIVE_TCP_RETRY_TICKS);
                } else if packet.flags & 0x17 == 0x10
                    && packet.sequence == operation.remote_sequence.wrapping_add(1)
                    && packet.acknowledgment == operation.local_sequence.wrapping_add(1)
                    && packet.len == 0
                {
                    return PassiveTcpProgress::Established(TcpServerPeer {
                        target: u32::from_be_bytes(operation.target),
                        remote_port: operation.remote_port,
                        local_port: operation.local_port,
                        remote_sequence: operation.remote_sequence.wrapping_add(1),
                        local_sequence: operation.local_sequence.wrapping_add(1),
                        source_mac: operation.source_mac,
                    });
                }
            }
        }

        if tick >= operation.deadline {
            if operation.attempts >= RETRIES || self.send_passive_syn_ack(operation).is_none() {
                return PassiveTcpProgress::Failed;
            }
            operation.attempts += 1;
            operation.deadline = tick.saturating_add(PASSIVE_TCP_RETRY_TICKS);
        }
        self.passive_tcp = Some(operation);
        PassiveTcpProgress::Pending
    }

    fn start_tcp_passive(&mut self, syn: PassiveTcpSyn, tick: u64) -> bool {
        if !self.available
            || self.async_udp.is_some()
            || self.async_tcp.is_some()
            || self.passive_tcp.is_some()
            || self.passive_stream.is_some()
            || syn.target == 0
            || syn.remote_port == 0
            || syn.local_port == 0
        {
            return false;
        }
        let target = syn.target.to_be_bytes();
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
            attempts: 1,
            deadline: tick.saturating_add(PASSIVE_TCP_RETRY_TICKS),
        };
        if self.send_passive_syn_ack(operation).is_none() {
            return false;
        }
        self.passive_tcp = Some(operation);
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

    fn cancel_tcp_passive(&mut self) {
        if let Some(operation) = self.passive_tcp.take() {
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

    fn start_tcp_passive_stream(&mut self, peer: TcpServerPeer, tick: u64) -> bool {
        if !self.available
            || self.async_udp.is_some()
            || self.async_tcp.is_some()
            || self.passive_tcp.is_some()
            || self.passive_stream.is_some()
        {
            return false;
        }
        self.passive_stream = Some(PassiveTcpStreamOperation {
            peer,
            remote_sequence: peer.remote_sequence,
            local_sequence: peer.local_sequence,
            receive: [0; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
            receive_len: 0,
            send: [0; PASSIVE_TCP_STREAM_BUFFER_CAPACITY],
            send_len: 0,
            send_completed: false,
            fin_sent: false,
            fin_acked: false,
            peer_fin: false,
            peer_fin_pending: false,
            attempts: 0,
            deadline: tick.saturating_add(PASSIVE_TCP_STREAM_IDLE_TICKS),
        });
        true
    }

    fn poll_tcp_passive_stream(&mut self, tick: u64) -> PassiveTcpStreamProgress {
        let Some(mut operation) = self.passive_stream.take() else {
            return PassiveTcpStreamProgress::Idle;
        };
        if tick >= operation.deadline
            && operation.send_len == 0
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
            return PassiveTcpStreamProgress::Failed(operation.peer);
        }
        if operation.receive_len != 0 {
            let progress = PassiveTcpStreamProgress::Received {
                peer: operation.peer,
                bytes: operation.receive,
                len: operation.receive_len,
            };
            self.passive_stream = Some(operation);
            return progress;
        }
        if operation.send_completed {
            let peer = operation.peer;
            self.passive_stream = Some(operation);
            return PassiveTcpStreamProgress::SendComplete(peer);
        }
        if operation.peer_fin && operation.fin_sent && operation.fin_acked {
            let peer = operation.peer;
            self.passive_stream = Some(operation);
            return PassiveTcpStreamProgress::Closed(peer);
        }
        if operation.peer_fin_pending {
            let peer = operation.peer;
            self.passive_stream = Some(operation);
            return PassiveTcpStreamProgress::PeerClosed(peer);
        }

        let mut received = false;
        for _ in 0..PASSIVE_TCP_RX_POLLS_PER_TICK {
            if self.device.receive(&mut self.rx) {
                received = true;
                break;
            }
            core::hint::spin_loop();
        }
        if received {
            let packet = self.decode_tcp_reply(
                operation.peer.target.to_be_bytes(),
                operation.peer.remote_port,
                operation.peer.local_port,
            );
            self.rx.owner = PacketOwner::Free;
            if let Some(packet) = packet {
                if packet.flags & 0x04 != 0 {
                    return PassiveTcpStreamProgress::Reset(operation.peer);
                }
                let mut acknowledgment_valid = false;
                if packet.flags & 0x10 != 0 {
                    if packet.acknowledgment == operation.local_sequence {
                        acknowledgment_valid = true;
                    } else if operation.send_len != 0
                        && packet.acknowledgment
                            == operation
                                .local_sequence
                                .wrapping_add(operation.send_len as u32)
                    {
                        operation.local_sequence = packet.acknowledgment;
                        operation.send_len = 0;
                        operation.send_completed = true;
                        operation.attempts = 0;
                        operation.deadline = tick.saturating_add(PASSIVE_TCP_STREAM_IDLE_TICKS);
                        acknowledgment_valid = true;
                    } else if operation.fin_sent
                        && !operation.fin_acked
                        && packet.acknowledgment == operation.local_sequence.wrapping_add(1)
                    {
                        operation.local_sequence = packet.acknowledgment;
                        operation.fin_acked = true;
                        operation.attempts = 0;
                        operation.deadline = tick.saturating_add(PASSIVE_TCP_STREAM_IDLE_TICKS);
                        acknowledgment_valid = true;
                    }
                }

                let mut acknowledge = false;
                if (packet.len != 0 || packet.flags & 0x01 != 0) && !acknowledgment_valid {
                    self.passive_stream = Some(operation);
                    return PassiveTcpStreamProgress::Pending;
                }
                if packet.len > operation.receive.len() {
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
                    return PassiveTcpStreamProgress::Failed(operation.peer);
                }
                if packet.len != 0 {
                    if packet.sequence == operation.remote_sequence
                        && packet.len <= operation.receive.len()
                    {
                        operation.receive[..packet.len].copy_from_slice(packet.payload());
                        operation.receive_len = packet.len;
                        operation.remote_sequence =
                            operation.remote_sequence.wrapping_add(packet.len as u32);
                        operation.deadline = tick.saturating_add(PASSIVE_TCP_STREAM_IDLE_TICKS);
                    }
                    acknowledge = true;
                }
                if packet.flags & 0x01 != 0 {
                    let fin_sequence = packet.sequence.wrapping_add(packet.len as u32);
                    if fin_sequence == operation.remote_sequence {
                        operation.remote_sequence = operation.remote_sequence.wrapping_add(1);
                        operation.peer_fin = true;
                        operation.peer_fin_pending = true;
                        operation.deadline = tick.saturating_add(PASSIVE_TCP_STREAM_IDLE_TICKS);
                    }
                    acknowledge = true;
                }
                if acknowledge
                    && self
                        .send_tcp(
                            operation.peer.source_mac,
                            operation.peer.target.to_be_bytes(),
                            operation.peer.local_port,
                            operation.peer.remote_port,
                            operation.local_sequence,
                            operation.remote_sequence,
                            0x10,
                            &[],
                        )
                        .is_none()
                {
                    return PassiveTcpStreamProgress::Failed(operation.peer);
                }
            }
        }

        if tick >= operation.deadline {
            let retry = if operation.send_len != 0 {
                Some((0x18, &operation.send[..operation.send_len]))
            } else if operation.fin_sent && !operation.fin_acked {
                Some((0x11, &operation.send[..0]))
            } else {
                None
            };
            if let Some((flags, payload)) = retry {
                if operation.attempts >= RETRIES
                    || self
                        .send_tcp(
                            operation.peer.source_mac,
                            operation.peer.target.to_be_bytes(),
                            operation.peer.local_port,
                            operation.peer.remote_port,
                            operation.local_sequence,
                            operation.remote_sequence,
                            flags,
                            payload,
                        )
                        .is_none()
                {
                    return PassiveTcpStreamProgress::Failed(operation.peer);
                }
                operation.attempts += 1;
                operation.deadline = tick.saturating_add(PASSIVE_TCP_RETRY_TICKS);
            }
        }

        self.passive_stream = Some(operation);
        PassiveTcpStreamProgress::Pending
    }

    fn start_tcp_passive_stream_send(
        &mut self,
        peer: TcpServerPeer,
        bytes: &[u8],
        tick: u64,
    ) -> bool {
        let Some(mut operation) = self.passive_stream.take() else {
            return false;
        };
        if operation.peer != peer
            || bytes.is_empty()
            || bytes.len() > operation.send.len()
            || operation.send_len != 0
            || operation.send_completed
            || operation.fin_sent
        {
            self.passive_stream = Some(operation);
            return false;
        }
        operation.send[..bytes.len()].copy_from_slice(bytes);
        operation.send_len = bytes.len();
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
                0x18,
                bytes,
            )
            .is_some();
        self.passive_stream = Some(operation);
        sent
    }

    fn consume_tcp_passive_stream_receive(&mut self, peer: TcpServerPeer) -> bool {
        self.passive_stream.as_mut().is_some_and(|operation| {
            if operation.peer != peer || operation.receive_len == 0 {
                return false;
            }
            operation.receive[..operation.receive_len].fill(0);
            operation.receive_len = 0;
            true
        })
    }

    fn consume_tcp_passive_stream_send(&mut self, peer: TcpServerPeer) -> bool {
        self.passive_stream.as_mut().is_some_and(|operation| {
            if operation.peer != peer || !operation.send_completed {
                return false;
            }
            operation.send.fill(0);
            operation.send_completed = false;
            true
        })
    }

    fn consume_tcp_passive_peer_close(&mut self, peer: TcpServerPeer) -> bool {
        self.passive_stream.as_mut().is_some_and(|operation| {
            if operation.peer != peer || !operation.peer_fin_pending {
                return false;
            }
            operation.peer_fin_pending = false;
            true
        })
    }

    fn start_tcp_passive_stream_close(&mut self, peer: TcpServerPeer, tick: u64) -> bool {
        let Some(mut operation) = self.passive_stream.take() else {
            return false;
        };
        if operation.peer != peer
            || operation.send_len != 0
            || operation.send_completed
            || operation.fin_sent
        {
            self.passive_stream = Some(operation);
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
        self.passive_stream = Some(operation);
        sent
    }

    fn finish_tcp_passive_stream(&mut self, peer: TcpServerPeer) -> bool {
        if self
            .passive_stream
            .is_some_and(|operation| operation.peer == peer)
        {
            self.passive_stream = None;
            true
        } else {
            false
        }
    }

    fn cancel_tcp_passive_stream(&mut self) {
        if let Some(operation) = self.passive_stream.take() {
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

    fn decode_passive_syn(&self) -> Option<PassiveTcpSyn> {
        let ip = parse_ipv4_frame(&self.rx.bytes[..self.rx.len]).filter(|ip| {
            ip.protocol == 6
                && ip.destination == self.address
                && net::transport_checksum_valid(ip.source, ip.destination, ip.protocol, ip.payload)
        })?;
        let tcp = parse_tcp(ip.payload)?;
        net::is_initial_tcp_syn(&tcp).then_some(PassiveTcpSyn {
            target: u32::from_be_bytes(ip.source),
            remote_port: tcp.source_port,
            local_port: tcp.destination_port,
            remote_sequence: tcp.sequence,
            source_mac: ip.source_mac,
        })
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
                if !self.device.receive(&mut self.rx) {
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
        segment[14..16].copy_from_slice(&4096u16.to_be_bytes());
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

pub fn cancel_tcp_passive() {
    unsafe { &mut *addr_of_mut!(NETWORK) }.cancel_tcp_passive();
}

pub fn tcp_passive_active() -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.passive_tcp.is_some()
}

pub fn start_tcp_passive_stream(peer: TcpServerPeer, tick: u64) -> bool {
    unsafe { &mut *addr_of_mut!(NETWORK) }.start_tcp_passive_stream(peer, tick)
}

pub fn poll_tcp_passive_stream(tick: u64) -> PassiveTcpStreamProgress {
    unsafe { &mut *addr_of_mut!(NETWORK) }.poll_tcp_passive_stream(tick)
}

pub fn tcp_passive_stream_peer() -> Option<TcpServerPeer> {
    unsafe { &mut *addr_of_mut!(NETWORK) }
        .passive_stream
        .map(|operation| operation.peer)
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

pub fn cancel_tcp_passive_stream() {
    unsafe { &mut *addr_of_mut!(NETWORK) }.cancel_tcp_passive_stream();
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
