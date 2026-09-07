//! Bounded IPv6 / ICMPv6 host bootstrap. No allocation, hardware access, or
//! implicit IPv4 conversion. See docs/IPV6.md for the supported wire contract.

pub type Address = [u8; 16];
pub const UNSPECIFIED: Address = [0; 16];
pub const ALL_NODES: Address = [0xff, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
pub const ALL_ROUTERS: Address = [0xff, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
pub const FRAME_CAPACITY: usize = 1518;
const HEADER: usize = 54;
const TICKS_PER_SECOND: u64 = 100;
const DAD_TICKS: u64 = TICKS_PER_SECOND;
const RS_TICKS: u64 = 4 * TICKS_PER_SECOND;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Packet<'a> {
    pub source_mac: [u8; 6],
    pub source: Address,
    pub destination: Address,
    pub next_header: u8,
    pub hop_limit: u8,
    pub payload: &'a [u8],
}

pub fn is_link_local(address: Address) -> bool {
    address[0] == 0xfe && address[1] & 0xc0 == 0x80
}
pub fn is_multicast(address: Address) -> bool {
    address[0] == 0xff
}

pub fn link_local(mac: [u8; 6]) -> Address {
    [
        0xfe,
        0x80,
        0,
        0,
        0,
        0,
        0,
        0,
        mac[0] ^ 2,
        mac[1],
        mac[2],
        0xff,
        0xfe,
        mac[3],
        mac[4],
        mac[5],
    ]
}

pub fn solicited_node(address: Address) -> Address {
    [
        0xff,
        2,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        1,
        0xff,
        address[13],
        address[14],
        address[15],
    ]
}

pub fn multicast_mac(address: Address) -> [u8; 6] {
    [
        0x33,
        0x33,
        address[12],
        address[13],
        address[14],
        address[15],
    ]
}

pub fn parse_frame(frame: &[u8]) -> Option<Packet<'_>> {
    if frame.len() < HEADER || frame.len() > FRAME_CAPACITY || frame[12..14] != [0x86, 0xdd] {
        return None;
    }
    let ip = &frame[14..];
    let length = usize::from(u16::from_be_bytes([ip[4], ip[5]]));
    if ip[0] >> 4 != 6 || ip[7] == 0 || length == 0 || length > ip.len() - 40 {
        return None;
    }
    let source: Address = ip[8..24].try_into().ok()?;
    let destination: Address = ip[24..40].try_into().ok()?;
    if is_multicast(source) || destination == UNSPECIFIED {
        return None;
    }
    let mut payload = &ip[40..40 + length];
    let mut next = ip[6];
    let mut seen_hop = false;
    let mut seen_destination = false;
    // At most one Hop-by-Hop header followed by one Destination Options
    // header. Only padding is supported; fragments, routing, AH/ESP, jumbo
    // payloads, unknown headers/options, and duplicate headers fail closed.
    while next == 0 || next == 60 {
        if payload.len() < 8 {
            return None;
        }
        if next == 0 {
            if seen_hop || seen_destination {
                return None;
            }
            seen_hop = true;
        } else {
            if seen_destination {
                return None;
            }
            seen_destination = true;
        }
        let len = (usize::from(payload[1]) + 1) * 8;
        let header = payload.get(..len)?;
        let mut offset = 2;
        while offset < len {
            match header[offset] {
                0 => offset += 1,
                1 => {
                    let count = usize::from(*header.get(offset + 1)?);
                    offset = offset.checked_add(count + 2)?;
                    if offset > len {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        next = payload[0];
        payload = &payload[len..];
    }
    if !matches!(next, 6 | 17 | 58) {
        return None;
    }
    Some(Packet {
        source_mac: frame[6..12].try_into().ok()?,
        source,
        destination,
        next_header: next,
        hop_limit: ip[7],
        payload,
    })
}

/// Internet checksum including the IPv6 pseudo-header. A valid complete
/// ICMPv6/TCP/UDP packet produces zero. Unlike IPv4, UDP zero is never optional.
pub fn transport_checksum(
    source: Address,
    destination: Address,
    protocol: u8,
    bytes: &[u8],
) -> u16 {
    let mut sum = 0u64;
    for slice in [source.as_slice(), destination.as_slice(), bytes] {
        let mut words = slice.chunks_exact(2);
        for word in &mut words {
            sum += u64::from(u16::from_be_bytes([word[0], word[1]]));
        }
        if let Some(&last) = words.remainder().first() {
            sum += u64::from(last) << 8;
        }
    }
    let len = bytes.len() as u64;
    sum += (len & 0xffff) + ((len >> 16) & 0xffff) + u64::from(protocol);
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn valid_transport(packet: &Packet<'_>) -> bool {
    let minimum = match packet.next_header {
        6 => 20,
        17 => 8,
        58 => 4,
        _ => return false,
    };
    if packet.payload.len() < minimum {
        return false;
    }
    if packet.next_header == 17
        && (packet.payload[6..8] == [0, 0]
            || usize::from(u16::from_be_bytes([packet.payload[4], packet.payload[5]]))
                != packet.payload.len())
    {
        return false;
    }
    transport_checksum(
        packet.source,
        packet.destination,
        packet.next_header,
        packet.payload,
    ) == 0
}

#[allow(clippy::too_many_arguments)]
pub fn write_icmp(
    frame: &mut [u8],
    source_mac: [u8; 6],
    destination_mac: [u8; 6],
    source: Address,
    destination: Address,
    hop_limit: u8,
    message: &[u8],
) -> Option<usize> {
    let len = HEADER.checked_add(message.len())?;
    if len > FRAME_CAPACITY || message.len() < 4 || hop_limit == 0 {
        return None;
    }
    let frame = frame.get_mut(..len)?;
    frame.fill(0);
    frame[..6].copy_from_slice(&destination_mac);
    frame[6..12].copy_from_slice(&source_mac);
    frame[12..14].copy_from_slice(&[0x86, 0xdd]);
    frame[14] = 0x60;
    frame[18..20].copy_from_slice(&(message.len() as u16).to_be_bytes());
    frame[20] = 58;
    frame[21] = hop_limit;
    frame[22..38].copy_from_slice(&source);
    frame[38..54].copy_from_slice(&destination);
    frame[54..].copy_from_slice(message);
    frame[56..58].fill(0);
    let checksum = transport_checksum(source, destination, 58, &frame[54..]);
    frame[56..58].copy_from_slice(&checksum.to_be_bytes());
    Some(len)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Route {
    pub address: Address,
    pub router: Address,
    pub router_mac: [u8; 6],
    pub mtu: u16,
    pub preferred_until: u64,
    pub valid_until: u64,
    pub router_until: u64,
}

fn deadline(tick: u64, seconds: u32) -> u64 {
    tick.saturating_add(u64::from(seconds) * TICKS_PER_SECOND)
}

fn nd_options(mut bytes: &[u8], mut visit: impl FnMut(u8, &[u8]) -> Option<()>) -> Option<()> {
    while !bytes.is_empty() {
        let len = usize::from(*bytes.get(1)?) * 8;
        if len == 0 {
            return None;
        }
        let option = bytes.get(..len)?;
        visit(option[0], option)?;
        bytes = &bytes[len..];
    }
    Some(())
}

pub fn router_advertisement(packet: &Packet<'_>, local: Address, tick: u64) -> Option<Route> {
    let body = packet.payload;
    if packet.next_header != 58
        || packet.hop_limit != 255
        || !is_link_local(packet.source)
        || !matches!(packet.destination, a if a == ALL_NODES || a == local)
        || body.len() < 16
        || body[..2] != [134, 0]
        || !valid_transport(packet)
    {
        return None;
    }
    let lifetime = u16::from_be_bytes([body[6], body[7]]);
    if lifetime == 0 {
        return None;
    }
    let mut prefix = None;
    let mut mac = None;
    let mut mtu = 1280u16;
    nd_options(&body[16..], |kind, option| {
        match kind {
            1 => {
                if option.len() != 8 || mac.is_some() {
                    return None;
                }
                let address: [u8; 6] = option[2..8].try_into().ok()?;
                if address != packet.source_mac || address[0] & 1 != 0 {
                    return None;
                }
                mac = Some(address);
            }
            3 => {
                if option.len() != 32 {
                    return None;
                }
                let valid = u32::from_be_bytes(option[4..8].try_into().ok()?);
                let preferred = u32::from_be_bytes(option[8..12].try_into().ok()?);
                let prefix_address: Address = option[16..32].try_into().ok()?;
                if preferred > valid {
                    return None;
                }
                if option[2] == 64
                    && option[3] & 0xc0 == 0xc0
                    && valid != 0
                    && preferred != 0
                    && !is_link_local(prefix_address)
                    && !is_multicast(prefix_address)
                    && prefix_address[..8] != [0; 8]
                    && prefix.is_none()
                {
                    prefix = Some((prefix_address, preferred, valid));
                }
            }
            5 => {
                if option.len() != 8 {
                    return None;
                }
                let advertised = u32::from_be_bytes(option[4..8].try_into().ok()?);
                if advertised >= 1280 {
                    mtu = advertised.min(1500) as u16;
                }
            }
            _ => {}
        }
        Some(())
    })?;
    let (mut address, preferred, valid) = prefix?;
    address[8..].copy_from_slice(&local[8..]);
    Some(Route {
        address,
        router: packet.source,
        router_mac: mac?,
        mtu,
        preferred_until: deadline(tick, preferred),
        valid_until: deadline(tick, valid),
        router_until: deadline(tick, u32::from(lifetime)),
    })
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum State {
    LinkLocalDad,
    RouterSolicitation,
    GlobalDad,
    Ready,
    Duplicate,
    Unavailable,
}

pub struct Host {
    pub state: State,
    pub link_local: Address,
    pub route: Option<Route>,
    pub echo_reply: bool,
    mac: [u8; 6],
    deadline: u64,
    attempts: u8,
    pending: bool,
    echo_sent: bool,
}

impl Host {
    pub fn new(mac: [u8; 6], tick: u64) -> Self {
        Self {
            state: State::LinkLocalDad,
            link_local: link_local(mac),
            route: None,
            echo_reply: false,
            mac,
            deadline: tick.saturating_add(DAD_TICKS),
            attempts: 0,
            pending: true,
            echo_sent: false,
        }
    }

    /// Produce at most one control frame per call. Time is supplied by the
    /// kernel's monotonic 100 Hz clock; loops never manufacture elapsed time.
    pub fn poll(&mut self, tick: u64, output: &mut [u8]) -> Option<usize> {
        // Do not consume a pending DAD/solicitation if the caller cannot hold
        // a complete frame. The production device always supplies this size.
        if output.len() < FRAME_CAPACITY {
            return None;
        }
        if !self.pending && tick >= self.deadline {
            match self.state {
                State::LinkLocalDad => {
                    self.state = State::RouterSolicitation;
                    self.pending = true;
                }
                State::GlobalDad => {
                    self.state = State::Ready;
                }
                State::RouterSolicitation => {
                    if self.attempts >= 3 {
                        self.state = State::Unavailable;
                    } else {
                        self.pending = true;
                    }
                }
                _ => {}
            }
        }
        if self.state == State::Ready
            && self
                .route
                .is_some_and(|r| tick >= r.valid_until || tick >= r.router_until)
        {
            self.route = None;
            self.state = State::RouterSolicitation;
            self.pending = true;
            self.attempts = 0;
            self.echo_reply = false;
            self.echo_sent = false;
        }
        if self.pending {
            self.pending = false;
            match self.state {
                State::LinkLocalDad | State::GlobalDad => {
                    let target = if self.state == State::LinkLocalDad {
                        self.link_local
                    } else {
                        self.route?.address
                    };
                    let destination = solicited_node(target);
                    let mut ns = [0u8; 24];
                    ns[0] = 135;
                    ns[8..].copy_from_slice(&target);
                    self.deadline = tick.saturating_add(DAD_TICKS);
                    return write_icmp(
                        output,
                        self.mac,
                        multicast_mac(destination),
                        UNSPECIFIED,
                        destination,
                        255,
                        &ns,
                    );
                }
                State::RouterSolicitation => {
                    let mut rs = [0u8; 16];
                    rs[0] = 133;
                    rs[8] = 1;
                    rs[9] = 1;
                    rs[10..].copy_from_slice(&self.mac);
                    self.attempts += 1;
                    self.deadline = tick.saturating_add(RS_TICKS);
                    return write_icmp(
                        output,
                        self.mac,
                        multicast_mac(ALL_ROUTERS),
                        self.link_local,
                        ALL_ROUTERS,
                        255,
                        &rs,
                    );
                }
                _ => {}
            }
        }
        if self.state == State::Ready && !self.echo_sent {
            let route = self.route?;
            self.echo_sent = true;
            return write_icmp(
                output,
                self.mac,
                route.router_mac,
                route.address,
                route.router,
                64,
                &[
                    128, 0, 0, 0, 0x47, 0x36, 0, 1, b'G', b'e', b'n', b'O', b'S', b'6',
                ],
            );
        }
        None
    }

    /// Validate before any state change. Neighbor packets use a strict
    /// one-interface policy; unauthenticated advertisements never grant a
    /// userspace handle or alter IPv4 state.
    pub fn receive(&mut self, frame: &[u8], tick: u64, output: &mut [u8]) -> Option<usize> {
        let packet = parse_frame(frame)?;
        let ethernet_destination: [u8; 6] = frame[..6].try_into().ok()?;
        if packet.source_mac[0] & 1 != 0
            || (if is_multicast(packet.destination) {
                ethernet_destination != multicast_mac(packet.destination)
            } else {
                ethernet_destination != self.mac
            })
        {
            return None;
        }
        if packet.source_mac == self.mac || packet.next_header != 58 || !valid_transport(&packet) {
            return None;
        }
        let body = packet.payload;
        if self.state == State::RouterSolicitation {
            if let Some(route) = router_advertisement(&packet, self.link_local, tick) {
                self.route = Some(route);
                self.state = State::GlobalDad;
                self.pending = true;
                return None;
            }
        }
        if matches!(body[0], 135 | 136) {
            if packet.hop_limit != 255 || body[1] != 0 || body.len() < 24 {
                return None;
            }
            let target: Address = body[8..24].try_into().ok()?;
            if is_multicast(target) || target == UNSPECIFIED {
                return None;
            }
            if packet.destination != self.link_local
                && packet.destination != ALL_NODES
                && packet.destination != solicited_node(target)
                && !self.route.is_some_and(|r| r.address == packet.destination)
            {
                return None;
            }
            let mut source_option = false;
            nd_options(&body[24..], |kind, option| {
                if matches!(kind, 1 | 2) && (option.len() != 8 || option[2..8] != packet.source_mac)
                {
                    return None;
                }
                source_option |= kind == 1;
                Some(())
            })?;
            if body[0] == 135
                && packet.source == UNSPECIFIED
                && (packet.destination != solicited_node(target) || source_option)
            {
                return None;
            }
            if body[0] == 136
                && (packet.source == UNSPECIFIED
                    || (is_multicast(packet.destination) && body[4] & 0x40 != 0))
            {
                return None;
            }
            let tentative = match self.state {
                State::LinkLocalDad => Some(self.link_local),
                State::GlobalDad => self.route.map(|r| r.address),
                _ => None,
            };
            if tentative == Some(target) && (body[0] == 136 || packet.source == UNSPECIFIED) {
                self.state = State::Duplicate;
                self.route = None;
                return None;
            }
            let owns_target = self.state != State::LinkLocalDad
                && self.state != State::Duplicate
                && (target == self.link_local
                    || (self.state == State::Ready
                        && self.route.is_some_and(|r| r.address == target)));
            if body[0] == 135
                && owns_target
                && (packet.destination == target || packet.destination == solicited_node(target))
            {
                let destination = if packet.source == UNSPECIFIED {
                    ALL_NODES
                } else {
                    packet.source
                };
                let destination_mac = if is_multicast(destination) {
                    multicast_mac(destination)
                } else {
                    packet.source_mac
                };
                let mut na = [0u8; 32];
                na[0] = 136;
                na[4] = if packet.source == UNSPECIFIED {
                    0x20
                } else {
                    0x60
                };
                na[8..24].copy_from_slice(&target);
                na[24] = 2;
                na[25] = 1;
                na[26..].copy_from_slice(&self.mac);
                return write_icmp(
                    output,
                    self.mac,
                    destination_mac,
                    target,
                    destination,
                    255,
                    &na,
                );
            }
        }
        if self.state == State::Ready {
            let route = self.route?;
            if body
                == [
                    129, 0, body[2], body[3], 0x47, 0x36, 0, 1, b'G', b'e', b'n', b'O', b'S', b'6',
                ]
                && self.echo_sent
                && packet.source == route.router
                && packet.destination == route.address
            {
                self.echo_reply = true;
            }
            if body[0] == 128
                && body[1] == 0
                && (8..=128).contains(&body.len())
                && packet.source != UNSPECIFIED
                && !is_multicast(packet.source)
                && (packet.destination == route.address || packet.destination == self.link_local)
            {
                let mut reply = [0u8; 128];
                reply[..body.len()].copy_from_slice(body);
                reply[0] = 129;
                return write_icmp(
                    output,
                    self.mac,
                    packet.source_mac,
                    packet.destination,
                    packet.source,
                    64,
                    &reply[..body.len()],
                );
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "ipv6_tests.rs"]
mod tests;
