use core::ptr::addr_of_mut;

use genos_abi::UserNetworkConfig;
use kernel::net::{self, parse_ipv4_frame, parse_tcp, parse_udp};

use crate::{arch, serial};

const IO_BASE: u16 = 0x300;
const DATA_PORT: u16 = IO_BASE + 0x10;
const RESET_PORT: u16 = IO_BASE + 0x1f;
const TX_PAGE: u8 = 0x40;
const RX_START: u8 = 0x46;
const RX_STOP: u8 = 0x80;
const MAX_FRAME: usize = 1518;
const POLL_LIMIT: usize = 800_000;
const RETRIES: usize = 3;

const CR: u16 = IO_BASE;
const PSTART: u16 = IO_BASE + 0x01;
const PSTOP: u16 = IO_BASE + 0x02;
const BNRY: u16 = IO_BASE + 0x03;
const TPSR: u16 = IO_BASE + 0x04;
const TBCR0: u16 = IO_BASE + 0x05;
const TBCR1: u16 = IO_BASE + 0x06;
const ISR: u16 = IO_BASE + 0x07;
const RSAR0: u16 = IO_BASE + 0x08;
const RSAR1: u16 = IO_BASE + 0x09;
const RBCR0: u16 = IO_BASE + 0x0a;
const RBCR1: u16 = IO_BASE + 0x0b;
const RCR: u16 = IO_BASE + 0x0c;
const TCR: u16 = IO_BASE + 0x0d;
const DCR: u16 = IO_BASE + 0x0e;
const IMR: u16 = IO_BASE + 0x0f;

#[derive(Clone, Copy, Eq, PartialEq)]
enum PacketOwner {
    Free,
    Driver,
    Stack,
}

#[derive(Clone, Copy)]
struct PacketBuffer {
    bytes: [u8; MAX_FRAME],
    len: usize,
    owner: PacketOwner,
}

impl PacketBuffer {
    const fn empty() -> Self {
        Self {
            bytes: [0; MAX_FRAME],
            len: 0,
            owner: PacketOwner::Free,
        }
    }
}

struct Ne2000 {
    mac: [u8; 6],
    next_rx: u8,
}

impl Ne2000 {
    const fn new() -> Self {
        Self {
            mac: [0; 6],
            next_rx: RX_START + 1,
        }
    }

    fn init(&mut self) -> bool {
        let reset = unsafe { arch::inb(RESET_PORT) };
        if reset == 0xff {
            return false;
        }
        unsafe { arch::outb(RESET_PORT, reset) };
        if !wait_register(ISR, 0x80) {
            return false;
        }
        write(CR, 0x21);
        write(DCR, 0x49);
        write(RBCR0, 0);
        write(RBCR1, 0);
        write(RCR, 0x20);
        write(TCR, 0x02);
        write(PSTART, RX_START);
        write(BNRY, RX_START);
        write(PSTOP, RX_STOP);
        write(ISR, 0xff);
        write(IMR, 0);

        let mut prom = [0u8; 32];
        if !self.remote_read(0, &mut prom) {
            return false;
        }
        for (index, byte) in self.mac.iter_mut().enumerate() {
            *byte = prom[index * 2];
        }
        if self.mac == [0; 6] || self.mac == [0xff; 6] {
            return false;
        }

        write(CR, 0x61);
        for (index, byte) in self.mac.iter().copied().enumerate() {
            write(IO_BASE + 1 + index as u16, byte);
        }
        write(IO_BASE + 7, self.next_rx);
        for offset in 8..=15 {
            write(IO_BASE + offset, 0);
        }
        write(CR, 0x22);
        write(TCR, 0);
        write(RCR, 0x04);
        write(ISR, 0xff);
        true
    }

    fn transmit(&mut self, frame: &[u8]) -> bool {
        if frame.len() > MAX_FRAME {
            return false;
        }
        let wire_len = frame.len().max(60);
        let mut padded = [0u8; MAX_FRAME];
        padded[..frame.len()].copy_from_slice(frame);
        if !self.remote_write(u16::from(TX_PAGE) << 8, &padded[..wire_len]) {
            return false;
        }
        write(TPSR, TX_PAGE);
        write(TBCR0, wire_len as u8);
        write(TBCR1, (wire_len >> 8) as u8);
        write(ISR, 0x0a);
        write(CR, 0x26);
        for _ in 0..POLL_LIMIT {
            let status = read(ISR);
            if status & 0x02 != 0 {
                write(ISR, 0x02);
                return true;
            }
            if status & 0x08 != 0 {
                write(ISR, 0x08);
                return false;
            }
        }
        false
    }

    fn receive(&mut self, packet: &mut PacketBuffer) -> bool {
        write(CR, 0x62);
        let current = read(IO_BASE + 7);
        write(CR, 0x22);
        if self.next_rx == current {
            return false;
        }
        let mut header = [0u8; 4];
        if !self.ring_read(u16::from(self.next_rx) << 8, &mut header) {
            return false;
        }
        let next = header[1];
        let recorded = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if header[0] & 0x01 == 0
            || !(RX_START..RX_STOP).contains(&next)
            || !(4..=MAX_FRAME + 4).contains(&recorded)
        {
            self.reset_ring(current);
            return false;
        }
        let len = recorded - 4;
        packet.owner = PacketOwner::Driver;
        if !self.ring_read((u16::from(self.next_rx) << 8) + 4, &mut packet.bytes[..len]) {
            packet.owner = PacketOwner::Free;
            return false;
        }
        packet.len = len;
        packet.owner = PacketOwner::Stack;
        self.next_rx = next;
        let boundary = if next == RX_START {
            RX_STOP - 1
        } else {
            next - 1
        };
        write(BNRY, boundary);
        true
    }

    fn reset_ring(&mut self, current: u8) {
        self.next_rx = if (RX_START..RX_STOP).contains(&current) {
            current
        } else {
            RX_START + 1
        };
        let boundary = if self.next_rx == RX_START {
            RX_STOP - 1
        } else {
            self.next_rx - 1
        };
        write(BNRY, boundary);
    }

    fn ring_read(&mut self, address: u16, output: &mut [u8]) -> bool {
        let ring_end = u16::from(RX_STOP) << 8;
        if address + output.len() as u16 <= ring_end {
            return self.remote_read(address, output);
        }
        let first = usize::from(ring_end - address);
        self.remote_read(address, &mut output[..first])
            && self.remote_read(u16::from(RX_START) << 8, &mut output[first..])
    }

    fn remote_read(&mut self, address: u16, output: &mut [u8]) -> bool {
        let count = (output.len() + 1) & !1;
        write(CR, 0x22);
        write(RBCR0, count as u8);
        write(RBCR1, (count >> 8) as u8);
        write(RSAR0, address as u8);
        write(RSAR1, (address >> 8) as u8);
        write(ISR, 0x40);
        write(CR, 0x0a);
        for index in (0..count).step_by(2) {
            let word = unsafe { arch::inw(DATA_PORT) }.to_le_bytes();
            if index < output.len() {
                output[index] = word[0];
            }
            if index + 1 < output.len() {
                output[index + 1] = word[1];
            }
        }
        wait_register(ISR, 0x40)
    }

    fn remote_write(&mut self, address: u16, input: &[u8]) -> bool {
        let count = (input.len() + 1) & !1;
        write(CR, 0x22);
        write(RBCR0, count as u8);
        write(RBCR1, (count >> 8) as u8);
        write(RSAR0, address as u8);
        write(RSAR1, (address >> 8) as u8);
        write(ISR, 0x40);
        write(CR, 0x12);
        for index in (0..count).step_by(2) {
            let low = input.get(index).copied().unwrap_or(0);
            let high = input.get(index + 1).copied().unwrap_or(0);
            unsafe { arch::outw(DATA_PORT, u16::from_le_bytes([low, high])) };
        }
        wait_register(ISR, 0x40)
    }
}

struct NetworkStack {
    device: Ne2000,
    available: bool,
    address: [u8; 4],
    subnet: [u8; 4],
    gateway: [u8; 4],
    dns: [u8; 4],
    next_port: u16,
    ip_id: u16,
    rx: PacketBuffer,
}

impl NetworkStack {
    const fn new() -> Self {
        Self {
            device: Ne2000::new(),
            available: false,
            address: [0; 4],
            subnet: [0; 4],
            gateway: [0; 4],
            dns: [0; 4],
            next_port: 49152,
            ip_id: 1,
            rx: PacketBuffer::empty(),
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
        let discover_len = build_dhcp(&mut payload, self.device.mac, xid, 1, None, None)?;
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
            self.device.mac,
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

    fn resolve_arp(&mut self, target: [u8; 4]) -> Option<[u8; 6]> {
        let mut frame = [0u8; 42];
        frame[..6].fill(0xff);
        frame[6..12].copy_from_slice(&self.device.mac);
        frame[12..14].copy_from_slice(&0x0806u16.to_be_bytes());
        frame[14..16].copy_from_slice(&1u16.to_be_bytes());
        frame[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
        frame[18] = 6;
        frame[19] = 4;
        frame[20..22].copy_from_slice(&1u16.to_be_bytes());
        frame[22..28].copy_from_slice(&self.device.mac);
        frame[28..32].copy_from_slice(&self.address);
        frame[38..42].copy_from_slice(&target);
        for _ in 0..RETRIES {
            if !self.device.transmit(&frame) {
                return None;
            }
            for _ in 0..POLL_LIMIT / RETRIES {
                if !self.device.receive(&mut self.rx) {
                    continue;
                }
                let bytes = &self.rx.bytes[..self.rx.len];
                let result = (bytes.len() >= 42
                    && bytes[12..14] == 0x0806u16.to_be_bytes()
                    && bytes[20..22] == 2u16.to_be_bytes()
                    && bytes[28..32] == target
                    && bytes[38..42] == self.address)
                    .then(|| bytes[22..28].try_into().ok())
                    .flatten();
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
        frame[6..12].copy_from_slice(&self.device.mac);
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
    if !stack.device.init() {
        serial::println("NETWORK_DEVICE_UNAVAILABLE");
        return;
    }
    serial::println("NETWORK_DEVICE_READY driver=ne2000-pio io=0x300");
    serial::println("PACKET_OWNERSHIP_READY buffers=1 states=free,driver,stack");
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
        mac: stack.device.mac,
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

fn wait_register(port: u16, mask: u8) -> bool {
    for _ in 0..POLL_LIMIT {
        if read(port) & mask != 0 {
            return true;
        }
    }
    false
}

fn read(port: u16) -> u8 {
    unsafe { arch::inb(port) }
}

fn write(port: u16, value: u8) {
    unsafe { arch::outb(port, value) }
}
