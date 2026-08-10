pub const SOCKET_CAPACITY: usize = 4;
pub const SOCKET_BUFFER_CAPACITY: usize = 128;

pub const SOCKET_READY_READABLE: u64 = 1;
pub const SOCKET_READY_WRITABLE: u64 = 2;
pub const SOCKET_READY_CONNECTED: u64 = 4;
pub const SOCKET_READY_CLOSED: u64 = 8;
pub const SOCKET_READY_ERROR: u64 = 16;
pub const SOCKET_READY_ACCEPT: u64 = 32;
pub const SOCKET_LISTENER_BACKLOG_CAPACITY: usize = 2;
pub const SOCKET_LISTENER_PORT_MIN: u16 = 1024;

const SOCKET_HANDLE_TAG: u64 = 0xe7 << 56;
const SOCKET_HANDLE_TAG_MASK: u64 = 0xff << 56;
const SOCKET_GENERATION_MAX: u64 = u16::MAX as u64;
const SOCKET_OWNER_INCARNATION_MASK: u64 = (1u64 << 28) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum SocketProtocol {
    Udp = 1,
    TcpStream = 2,
}

impl SocketProtocol {
    pub const fn from_raw(value: u64) -> Option<Self> {
        match value {
            1 => Some(Self::Udp),
            2 => Some(Self::TcpStream),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum SocketState {
    Open = 1,
    Connecting = 2,
    Established = 3,
    ReadClosed = 4,
    WriteClosed = 5,
    Closed = 6,
    Failed = 7,
    Bound = 8,
    Listening = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketOwner {
    pub slot: u8,
    pub incarnation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketStatus {
    pub protocol: SocketProtocol,
    pub state: SocketState,
    pub readiness: u64,
    pub queued_send: usize,
    pub queued_receive: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketError {
    InvalidHandle,
    InvalidState,
    WouldBlock,
    Capacity,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcpServerPeer {
    pub target: u32,
    pub remote_port: u16,
    pub local_port: u16,
    pub remote_sequence: u32,
    pub local_sequence: u32,
    pub source_mac: [u8; 6],
}

#[derive(Clone, Copy)]
struct ByteQueue {
    bytes: [u8; SOCKET_BUFFER_CAPACITY],
    len: usize,
}

impl ByteQueue {
    const fn new() -> Self {
        Self {
            bytes: [0; SOCKET_BUFFER_CAPACITY],
            len: 0,
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<usize, SocketError> {
        if bytes.is_empty() || bytes.len() > self.bytes.len().saturating_sub(self.len) {
            return Err(SocketError::WouldBlock);
        }
        let start = self.len;
        self.bytes[start..start + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(bytes.len())
    }

    fn pop(&mut self, output: &mut [u8]) -> Result<usize, SocketError> {
        if self.len == 0 {
            return Err(SocketError::WouldBlock);
        }
        let len = self.len.min(output.len());
        output[..len].copy_from_slice(&self.bytes[..len]);
        self.bytes.copy_within(len..self.len, 0);
        self.bytes[self.len - len..self.len].fill(0);
        self.len -= len;
        Ok(len)
    }

    fn clear(&mut self) {
        self.bytes.fill(0);
        self.len = 0;
    }
}

#[derive(Clone, Copy)]
struct SocketEntry {
    handle: u64,
    generation: u64,
    owner: SocketOwner,
    protocol: SocketProtocol,
    state: SocketState,
    target: u32,
    port: u16,
    local_port: u16,
    backlog_limit: usize,
    backlog: [Option<TcpServerPeer>; SOCKET_LISTENER_BACKLOG_CAPACITY],
    backlog_len: usize,
    server_peer: Option<TcpServerPeer>,
    send: ByteQueue,
    in_flight_request: u64,
    in_flight_send: usize,
    receive: ByteQueue,
}

impl SocketEntry {
    fn status(&self) -> SocketStatus {
        let mut readiness = 0;
        if self.receive.len != 0 {
            readiness |= SOCKET_READY_READABLE;
        }
        if self.state == SocketState::Listening && self.backlog_len != 0 {
            readiness |= SOCKET_READY_READABLE | SOCKET_READY_ACCEPT;
        }
        let queued_send = self.send.len.saturating_add(self.in_flight_send);
        if queued_send < SOCKET_BUFFER_CAPACITY
            && !matches!(
                self.state,
                SocketState::WriteClosed
                    | SocketState::Closed
                    | SocketState::Failed
                    | SocketState::Bound
                    | SocketState::Listening
            )
        {
            readiness |= SOCKET_READY_WRITABLE;
        }
        if self.state == SocketState::Established {
            readiness |= SOCKET_READY_CONNECTED;
        }
        if matches!(
            self.state,
            SocketState::ReadClosed
                | SocketState::WriteClosed
                | SocketState::Closed
                | SocketState::Failed
        ) {
            readiness |= SOCKET_READY_CLOSED;
        }
        if self.state == SocketState::Failed {
            readiness |= SOCKET_READY_ERROR;
        }
        SocketStatus {
            protocol: self.protocol,
            state: self.state,
            readiness,
            queued_send,
            queued_receive: self.receive.len,
        }
    }
}

pub struct SocketSet {
    entries: [Option<SocketEntry>; SOCKET_CAPACITY],
    next_generation: u64,
}

impl SocketSet {
    pub const fn new() -> Self {
        Self {
            entries: [None; SOCKET_CAPACITY],
            next_generation: 1,
        }
    }

    pub fn open(
        &mut self,
        owner: SocketOwner,
        protocol: SocketProtocol,
    ) -> Result<u64, SocketError> {
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(SocketError::Capacity)?;
        let generation = self.next_generation;
        if generation == 0 || generation > SOCKET_GENERATION_MAX {
            return Err(SocketError::Capacity);
        }
        self.next_generation = generation + 1;
        let handle = socket_handle(owner, generation, slot);
        self.entries[slot] = Some(SocketEntry {
            handle,
            generation,
            owner,
            protocol,
            state: SocketState::Open,
            target: 0,
            port: 0,
            local_port: 0,
            backlog_limit: 0,
            backlog: [None; SOCKET_LISTENER_BACKLOG_CAPACITY],
            backlog_len: 0,
            server_peer: None,
            send: ByteQueue::new(),
            in_flight_request: 0,
            in_flight_send: 0,
            receive: ByteQueue::new(),
        });
        Ok(handle)
    }

    pub fn connect(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        target: u32,
        port: u16,
    ) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if target == 0 || port == 0 || entry.state != SocketState::Open {
            return Err(SocketError::InvalidState);
        }
        entry.target = target;
        entry.port = port;
        entry.state = match entry.protocol {
            SocketProtocol::Udp => SocketState::Established,
            SocketProtocol::TcpStream => SocketState::Connecting,
        };
        Ok(())
    }

    pub fn bind(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        local_port: u16,
    ) -> Result<(), SocketError> {
        self.validate_bind(owner, handle, local_port)?;
        let entry = self.entry_mut(owner, handle)?;
        entry.local_port = local_port;
        entry.state = SocketState::Bound;
        Ok(())
    }

    pub fn validate_bind(
        &self,
        owner: SocketOwner,
        handle: u64,
        local_port: u16,
    ) -> Result<(), SocketError> {
        let entry = self.entry(owner, handle)?;
        if entry.protocol != SocketProtocol::TcpStream
            || entry.state != SocketState::Open
            || local_port < SOCKET_LISTENER_PORT_MIN
        {
            return Err(SocketError::InvalidState);
        }
        Ok(())
    }

    pub fn listen(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        backlog: usize,
    ) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.protocol != SocketProtocol::TcpStream
            || entry.state != SocketState::Bound
            || backlog == 0
            || backlog > SOCKET_LISTENER_BACKLOG_CAPACITY
        {
            return Err(SocketError::InvalidState);
        }
        entry.backlog_limit = backlog;
        entry.state = SocketState::Listening;
        Ok(())
    }

    pub fn queue_incoming(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        peer: TcpServerPeer,
    ) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.state != SocketState::Listening
            || peer.target == 0
            || peer.remote_port == 0
            || peer.local_port != entry.local_port
            || entry.backlog_len >= entry.backlog_limit
        {
            return Err(SocketError::WouldBlock);
        }
        entry.backlog[entry.backlog_len] = Some(peer);
        entry.backlog_len += 1;
        Ok(())
    }

    pub fn drop_incoming(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        peer: TcpServerPeer,
    ) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.state != SocketState::Listening {
            return Err(SocketError::InvalidState);
        }
        let position = entry.backlog[..entry.backlog_len]
            .iter()
            .position(|candidate| *candidate == Some(peer))
            .ok_or(SocketError::InvalidState)?;
        entry
            .backlog
            .copy_within(position + 1..entry.backlog_len, position);
        entry.backlog_len -= 1;
        entry.backlog[entry.backlog_len] = None;
        Ok(())
    }

    pub fn accept(&mut self, owner: SocketOwner, handle: u64) -> Result<u64, SocketError> {
        if self.entries.iter().all(Option::is_some) {
            return Err(SocketError::Capacity);
        }
        let listener_slot = self.slot_for(owner, handle)?;
        let (connection, local_port) = {
            let listener = self.entries[listener_slot]
                .as_ref()
                .ok_or(SocketError::InvalidHandle)?;
            if listener.state != SocketState::Listening || listener.backlog_len == 0 {
                return Err(SocketError::WouldBlock);
            }
            (
                listener.backlog[0].ok_or(SocketError::WouldBlock)?,
                listener.local_port,
            )
        };
        // Allocate the child before consuming the queued connection. If the
        // generation space is exhausted, accept remains safely retryable.
        let accepted = self.open(owner, SocketProtocol::TcpStream)?;
        {
            let listener = self.entries[listener_slot]
                .as_mut()
                .ok_or(SocketError::InvalidHandle)?;
            listener.backlog.copy_within(1..listener.backlog_len, 0);
            listener.backlog_len -= 1;
            listener.backlog[listener.backlog_len] = None;
        }
        let entry = self.entry_mut(owner, accepted)?;
        entry.target = connection.target;
        entry.port = connection.remote_port;
        entry.local_port = local_port;
        entry.server_peer = Some(connection);
        entry.state = SocketState::Established;
        Ok(accepted)
    }

    pub fn owns_local_port(&self, local_port: u16) -> bool {
        self.entries.iter().flatten().any(|entry| {
            entry.protocol == SocketProtocol::TcpStream
                && entry.local_port == local_port
                && matches!(entry.state, SocketState::Bound | SocketState::Listening)
        })
    }

    pub fn listener_handle(&self, local_port: u16) -> Option<u64> {
        self.entries.iter().flatten().find_map(|entry| {
            (entry.protocol == SocketProtocol::TcpStream
                && entry.state == SocketState::Listening
                && entry.local_port == local_port)
                .then_some(entry.handle)
        })
    }

    pub fn mark_connected(&mut self, owner: SocketOwner, handle: u64) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.state != SocketState::Connecting {
            return Err(SocketError::InvalidState);
        }
        entry.state = SocketState::Established;
        Ok(())
    }

    pub fn send(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        bytes: &[u8],
    ) -> Result<usize, SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if !matches!(
            entry.state,
            SocketState::Connecting | SocketState::Established | SocketState::ReadClosed
        ) {
            return Err(SocketError::InvalidState);
        }
        if bytes.len()
            > SOCKET_BUFFER_CAPACITY.saturating_sub(entry.send.len + entry.in_flight_send)
        {
            return Err(SocketError::WouldBlock);
        }
        entry.send.push(bytes)
    }

    pub fn pending_udp_handle(&self, owner: SocketOwner) -> Option<u64> {
        self.entries.iter().flatten().find_map(|entry| {
            (entry.owner == owner
                && entry.protocol == SocketProtocol::Udp
                && entry.state == SocketState::Established
                && entry.send.len != 0
                && entry.in_flight_send == 0)
                .then_some(entry.handle)
        })
    }

    pub fn pending_transport(&self, owner: SocketOwner) -> Option<(u64, SocketProtocol)> {
        self.entries.iter().flatten().find_map(|entry| {
            (entry.owner == owner
                && entry.server_peer.is_none()
                && matches!(
                    entry.protocol,
                    SocketProtocol::Udp | SocketProtocol::TcpStream
                )
                && matches!(
                    entry.state,
                    SocketState::Connecting | SocketState::Established
                )
                && entry.send.len != 0
                && entry.in_flight_send == 0)
                .then_some((entry.handle, entry.protocol))
        })
    }

    pub fn begin_transport(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        protocol: SocketProtocol,
        request_id: u64,
        output: &mut [u8; SOCKET_BUFFER_CAPACITY],
    ) -> Result<(u32, u16, usize), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.server_peer.is_some()
            || entry.protocol != protocol
            || !matches!(
                entry.state,
                SocketState::Connecting | SocketState::Established
            )
            || request_id == 0
            || entry.in_flight_request != 0
            || entry.in_flight_send != 0
            || entry.send.len == 0
        {
            return Err(SocketError::InvalidState);
        }
        let len = entry.send.len;
        output[..len].copy_from_slice(&entry.send.bytes[..len]);
        entry.send.clear();
        entry.in_flight_request = request_id;
        entry.in_flight_send = len;
        Ok((entry.target, entry.port, len))
    }

    pub fn transport_request_active(
        &self,
        owner: SocketOwner,
        handle: u64,
        protocol: SocketProtocol,
        request_id: u64,
        length: usize,
    ) -> bool {
        self.entry(owner, handle).is_ok_and(|entry| {
            entry.server_peer.is_none()
                && entry.protocol == protocol
                && matches!(
                    entry.state,
                    SocketState::Connecting | SocketState::Established
                )
                && entry.in_flight_request == request_id
                && entry.in_flight_send == length
                && request_id != 0
                && length != 0
        })
    }

    pub fn complete_transport(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        protocol: SocketProtocol,
        request_id: u64,
        response: &[u8],
    ) -> Result<usize, SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.protocol != protocol
            || !matches!(
                entry.state,
                SocketState::Connecting | SocketState::Established
            )
            || entry.in_flight_request != request_id
            || entry.in_flight_send == 0
        {
            return Err(SocketError::InvalidState);
        }
        let result = entry.receive.push(response);
        entry.in_flight_request = 0;
        entry.in_flight_send = 0;
        if result.is_ok() && protocol == SocketProtocol::TcpStream {
            entry.state = SocketState::Established;
        } else if result.is_err() {
            entry.state = SocketState::Failed;
        }
        result
    }

    pub fn fail_transport(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        protocol: SocketProtocol,
        request_id: u64,
    ) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.protocol != protocol
            || entry.in_flight_request != request_id
            || entry.in_flight_send == 0
        {
            return Err(SocketError::InvalidState);
        }
        entry.in_flight_request = 0;
        entry.in_flight_send = 0;
        entry.state = SocketState::Failed;
        Ok(())
    }

    pub fn server_handle(&self, peer: TcpServerPeer) -> Option<u64> {
        self.entries.iter().flatten().find_map(|entry| {
            (entry.protocol == SocketProtocol::TcpStream
                && entry.server_peer == Some(peer)
                && matches!(
                    entry.state,
                    SocketState::Established
                        | SocketState::ReadClosed
                        | SocketState::WriteClosed
                        | SocketState::Closed
                ))
            .then_some(entry.handle)
        })
    }

    pub fn server_peer(
        &self,
        owner: SocketOwner,
        handle: u64,
    ) -> Result<TcpServerPeer, SocketError> {
        self.entry(owner, handle)?
            .server_peer
            .ok_or(SocketError::InvalidState)
    }

    pub fn begin_server_send(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        request_id: u64,
        output: &mut [u8; SOCKET_BUFFER_CAPACITY],
    ) -> Result<usize, SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.server_peer.is_none()
            || !matches!(
                entry.state,
                SocketState::Established | SocketState::ReadClosed
            )
            || request_id == 0
            || entry.in_flight_request != 0
            || entry.in_flight_send != 0
            || entry.send.len == 0
        {
            return Err(SocketError::InvalidState);
        }
        let len = entry.send.len;
        output[..len].copy_from_slice(&entry.send.bytes[..len]);
        entry.send.clear();
        entry.in_flight_request = request_id;
        entry.in_flight_send = len;
        Ok(len)
    }

    pub fn server_send_pending(
        &self,
        owner: SocketOwner,
        handle: u64,
        peer: TcpServerPeer,
    ) -> bool {
        self.entry(owner, handle).is_ok_and(|entry| {
            entry.server_peer == Some(peer)
                && matches!(
                    entry.state,
                    SocketState::Established | SocketState::ReadClosed
                )
                && entry.send.len != 0
                && entry.in_flight_send == 0
        })
    }

    pub fn server_send_active(
        &self,
        owner: SocketOwner,
        handle: u64,
        peer: TcpServerPeer,
        request_id: u64,
        length: usize,
    ) -> bool {
        self.entry(owner, handle).is_ok_and(|entry| {
            entry.server_peer == Some(peer)
                && matches!(
                    entry.state,
                    SocketState::Established | SocketState::ReadClosed
                )
                && entry.in_flight_request == request_id
                && entry.in_flight_send == length
                && request_id != 0
                && length != 0
        })
    }

    pub fn complete_server_send(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        peer: TcpServerPeer,
        request_id: u64,
    ) -> Result<usize, SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.server_peer != Some(peer)
            || entry.in_flight_request != request_id
            || entry.in_flight_send == 0
        {
            return Err(SocketError::InvalidState);
        }
        let sent = entry.in_flight_send;
        entry.in_flight_request = 0;
        entry.in_flight_send = 0;
        Ok(sent)
    }

    pub fn server_write_closed(
        &self,
        owner: SocketOwner,
        handle: u64,
        peer: TcpServerPeer,
    ) -> bool {
        self.entry(owner, handle).is_ok_and(|entry| {
            entry.server_peer == Some(peer)
                && matches!(entry.state, SocketState::WriteClosed | SocketState::Closed)
                && entry.send.len == 0
                && entry.in_flight_send == 0
        })
    }

    pub fn mark_server_read_closed(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        peer: TcpServerPeer,
    ) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.server_peer != Some(peer) {
            return Err(SocketError::InvalidState);
        }
        entry.state = match entry.state {
            SocketState::Established => SocketState::ReadClosed,
            SocketState::WriteClosed => SocketState::WriteClosed,
            SocketState::ReadClosed | SocketState::Closed => entry.state,
            _ => return Err(SocketError::InvalidState),
        };
        Ok(())
    }

    pub fn mark_server_closed(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        peer: TcpServerPeer,
    ) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.server_peer != Some(peer) {
            return Err(SocketError::InvalidState);
        }
        entry.send.clear();
        entry.in_flight_request = 0;
        entry.in_flight_send = 0;
        entry.state = SocketState::Closed;
        Ok(())
    }

    pub fn begin_udp_send(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        request_id: u64,
        output: &mut [u8; SOCKET_BUFFER_CAPACITY],
    ) -> Result<(u32, u16, usize), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.protocol != SocketProtocol::Udp
            || entry.state != SocketState::Established
            || request_id == 0
            || entry.in_flight_request != 0
            || entry.in_flight_send != 0
            || entry.send.len == 0
        {
            return Err(SocketError::InvalidState);
        }
        let len = entry.send.len;
        output[..len].copy_from_slice(&entry.send.bytes[..len]);
        entry.send.clear();
        entry.in_flight_request = request_id;
        entry.in_flight_send = len;
        Ok((entry.target, entry.port, len))
    }

    pub fn udp_request_active(
        &self,
        owner: SocketOwner,
        handle: u64,
        request_id: u64,
        length: usize,
    ) -> bool {
        self.entry(owner, handle).is_ok_and(|entry| {
            entry.protocol == SocketProtocol::Udp
                && entry.state == SocketState::Established
                && entry.in_flight_request == request_id
                && entry.in_flight_send == length
                && request_id != 0
                && length != 0
        })
    }

    pub fn complete_udp_send(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        request_id: u64,
        response: &[u8],
    ) -> Result<usize, SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.protocol != SocketProtocol::Udp
            || entry.state != SocketState::Established
            || entry.in_flight_request != request_id
            || entry.in_flight_send == 0
        {
            return Err(SocketError::InvalidState);
        }
        let result = entry.receive.push(response);
        entry.in_flight_request = 0;
        entry.in_flight_send = 0;
        if result.is_err() {
            entry.state = SocketState::Failed;
        }
        result
    }

    pub fn fail_udp_send(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        request_id: u64,
    ) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if entry.protocol != SocketProtocol::Udp
            || entry.in_flight_request != request_id
            || entry.in_flight_send == 0
        {
            return Err(SocketError::InvalidState);
        }
        entry.in_flight_request = 0;
        entry.in_flight_send = 0;
        entry.state = SocketState::Failed;
        Ok(())
    }

    pub fn take_send(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        output: &mut [u8],
    ) -> Result<usize, SocketError> {
        self.entry_mut(owner, handle)?.send.pop(output)
    }

    pub fn push_receive(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        bytes: &[u8],
    ) -> Result<usize, SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if !matches!(
            entry.state,
            SocketState::Established | SocketState::WriteClosed
        ) {
            return Err(SocketError::InvalidState);
        }
        entry.receive.push(bytes)
    }

    pub fn receive(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        output: &mut [u8],
    ) -> Result<usize, SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        if matches!(entry.state, SocketState::Open | SocketState::Connecting) {
            return Err(SocketError::WouldBlock);
        }
        entry.receive.pop(output)
    }

    pub fn shutdown(
        &mut self,
        owner: SocketOwner,
        handle: u64,
        read: bool,
        write: bool,
    ) -> Result<(), SocketError> {
        if !read && !write {
            return Err(SocketError::InvalidState);
        }
        let entry = self.entry_mut(owner, handle)?;
        if matches!(
            entry.state,
            SocketState::Closed | SocketState::Failed | SocketState::Bound | SocketState::Listening
        ) {
            return Err(SocketError::InvalidState);
        }
        if entry.server_peer.is_some()
            && write
            && (entry.send.len != 0 || entry.in_flight_send != 0)
        {
            return Err(SocketError::WouldBlock);
        }
        if entry.server_peer.is_some() && write {
            if read {
                entry.receive.clear();
            }
            entry.send.clear();
            entry.state = SocketState::WriteClosed;
            return Ok(());
        }
        if read {
            entry.receive.clear();
        }
        if write {
            entry.send.clear();
            entry.in_flight_request = 0;
            entry.in_flight_send = 0;
        }
        entry.state = match (read, write, entry.state) {
            (true, true, _)
            | (true, false, SocketState::WriteClosed)
            | (false, true, SocketState::ReadClosed) => SocketState::Closed,
            (true, false, _) => SocketState::ReadClosed,
            (false, true, _) => SocketState::WriteClosed,
            (false, false, _) => return Err(SocketError::InvalidState),
        };
        Ok(())
    }

    pub fn fail(&mut self, owner: SocketOwner, handle: u64) -> Result<(), SocketError> {
        let entry = self.entry_mut(owner, handle)?;
        entry.send.clear();
        entry.in_flight_request = 0;
        entry.in_flight_send = 0;
        entry.state = SocketState::Failed;
        Ok(())
    }

    pub fn close(&mut self, owner: SocketOwner, handle: u64) -> Result<(), SocketError> {
        let slot = self.slot_for(owner, handle)?;
        self.entries[slot] = None;
        Ok(())
    }

    pub fn close_owner(&mut self, owner: SocketOwner) -> usize {
        let mut closed = 0;
        for entry in &mut self.entries {
            if entry.is_some_and(|entry| entry.owner == owner) {
                *entry = None;
                closed += 1;
            }
        }
        closed
    }

    pub fn len_owner(&self, owner: SocketOwner) -> usize {
        self.entries
            .iter()
            .flatten()
            .filter(|entry| entry.owner == owner)
            .count()
    }

    pub fn handles(&self, owner: SocketOwner) -> impl Iterator<Item = u64> + '_ {
        self.entries
            .iter()
            .flatten()
            .filter(move |entry| entry.owner == owner)
            .map(|entry| entry.handle)
    }

    pub fn status(&self, owner: SocketOwner, handle: u64) -> Result<SocketStatus, SocketError> {
        Ok(self.entry(owner, handle)?.status())
    }

    pub fn remote(&self, owner: SocketOwner, handle: u64) -> Result<(u32, u16), SocketError> {
        let entry = self.entry(owner, handle)?;
        Ok((entry.target, entry.port))
    }

    fn entry(&self, owner: SocketOwner, handle: u64) -> Result<&SocketEntry, SocketError> {
        let slot = self.slot_for(owner, handle)?;
        self.entries[slot]
            .as_ref()
            .ok_or(SocketError::InvalidHandle)
    }

    fn entry_mut(
        &mut self,
        owner: SocketOwner,
        handle: u64,
    ) -> Result<&mut SocketEntry, SocketError> {
        let slot = self.slot_for(owner, handle)?;
        self.entries[slot]
            .as_mut()
            .ok_or(SocketError::InvalidHandle)
    }

    fn slot_for(&self, owner: SocketOwner, handle: u64) -> Result<usize, SocketError> {
        let slot = socket_slot(handle).ok_or(SocketError::InvalidHandle)?;
        let entry = self.entries[slot].ok_or(SocketError::InvalidHandle)?;
        if entry.handle != handle
            || entry.owner != owner
            || socket_handle(entry.owner, entry.generation, slot) != handle
        {
            return Err(SocketError::InvalidHandle);
        }
        Ok(slot)
    }
}

pub fn local_port_is_available<'a>(
    mut socket_sets: impl Iterator<Item = &'a SocketSet>,
    local_port: u16,
) -> bool {
    !socket_sets.any(|sockets| sockets.owns_local_port(local_port))
}

impl Default for SocketSet {
    fn default() -> Self {
        Self::new()
    }
}

const fn socket_handle(owner: SocketOwner, generation: u64, slot: usize) -> u64 {
    SOCKET_HANDLE_TAG
        | ((owner.slot as u64 & 0x0f) << 52)
        | ((owner.incarnation & SOCKET_OWNER_INCARNATION_MASK) << 24)
        | (generation << 8)
        | (slot as u64 + 1)
}

fn socket_slot(handle: u64) -> Option<usize> {
    if handle & SOCKET_HANDLE_TAG_MASK != SOCKET_HANDLE_TAG {
        return None;
    }
    let slot = (handle & 0xff) as usize;
    (1..=SOCKET_CAPACITY).contains(&slot).then_some(slot - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: SocketOwner = SocketOwner {
        slot: 2,
        incarnation: 9,
    };
    const OTHER: SocketOwner = SocketOwner {
        slot: 3,
        incarnation: 9,
    };

    fn server_peer(target: u32, remote_port: u16) -> TcpServerPeer {
        TcpServerPeer {
            target,
            remote_port,
            local_port: SOCKET_LISTENER_PORT_MIN,
            remote_sequence: 100,
            local_sequence: 200,
            source_mac: [1, 2, 3, 4, 5, 6],
        }
    }

    #[test]
    fn handles_are_generation_safe_and_owner_scoped() {
        let mut sockets = SocketSet::new();
        let first = sockets.open(OWNER, SocketProtocol::Udp).unwrap();
        let other_handle = sockets.open(OTHER, SocketProtocol::Udp).unwrap();
        assert_ne!(first, other_handle);
        assert!(sockets.status(OTHER, first).is_err());
        assert!(sockets.status(OWNER, first ^ (1 << 8)).is_err());
        sockets.close(OWNER, first).unwrap();
        let second = sockets.open(OWNER, SocketProtocol::Udp).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            sockets.status(OWNER, first),
            Err(SocketError::InvalidHandle)
        );
    }

    #[test]
    fn bounded_queues_report_readiness_and_backpressure() {
        let mut sockets = SocketSet::new();
        let handle = sockets.open(OWNER, SocketProtocol::TcpStream).unwrap();
        sockets.connect(OWNER, handle, 0x0a00_0202, 443).unwrap();
        assert_eq!(
            sockets.receive(OWNER, handle, &mut [0; 8]),
            Err(SocketError::WouldBlock)
        );
        sockets.mark_connected(OWNER, handle).unwrap();
        let payload = [7u8; SOCKET_BUFFER_CAPACITY];
        assert_eq!(sockets.send(OWNER, handle, &payload), Ok(payload.len()));
        assert_eq!(
            sockets.send(OWNER, handle, b"x"),
            Err(SocketError::WouldBlock)
        );
        assert_eq!(
            sockets.status(OWNER, handle).unwrap().readiness & SOCKET_READY_WRITABLE,
            0
        );
        let mut drained = [0u8; SOCKET_BUFFER_CAPACITY];
        assert_eq!(
            sockets.take_send(OWNER, handle, &mut drained),
            Ok(payload.len())
        );
        assert_eq!(drained, payload);
        assert_ne!(
            sockets.status(OWNER, handle).unwrap().readiness & SOCKET_READY_WRITABLE,
            0
        );
    }

    #[test]
    fn receive_is_partial_bounded_and_lossless() {
        let mut sockets = SocketSet::new();
        let handle = sockets.open(OWNER, SocketProtocol::Udp).unwrap();
        sockets.connect(OWNER, handle, 0x0a00_0203, 53).unwrap();
        sockets.push_receive(OWNER, handle, b"abcdefgh").unwrap();
        let mut first = [0u8; 3];
        let mut second = [0u8; 5];
        assert_eq!(sockets.receive(OWNER, handle, &mut first), Ok(3));
        assert_eq!(&first, b"abc");
        assert_eq!(sockets.receive(OWNER, handle, &mut second), Ok(5));
        assert_eq!(&second, b"defgh");
        assert_eq!(
            sockets.receive(OWNER, handle, &mut second),
            Err(SocketError::WouldBlock)
        );
    }

    #[test]
    fn udp_completion_is_bound_to_the_exact_in_flight_request() {
        let mut sockets = SocketSet::new();
        let handle = sockets.open(OWNER, SocketProtocol::Udp).unwrap();
        sockets.connect(OWNER, handle, 0x0a00_0203, 53).unwrap();
        sockets.send(OWNER, handle, b"query").unwrap();
        let mut request = [0u8; SOCKET_BUFFER_CAPACITY];
        assert_eq!(
            sockets.begin_udp_send(OWNER, handle, 41, &mut request),
            Ok((0x0a00_0203, 53, 5))
        );
        assert_eq!(&request[..5], b"query");
        assert_eq!(sockets.status(OWNER, handle).unwrap().queued_send, 5);
        assert!(sockets.udp_request_active(OWNER, handle, 41, 5));
        assert!(!sockets.udp_request_active(OWNER, handle, 42, 5));
        assert_eq!(
            sockets.complete_udp_send(OWNER, handle, 42, b"reply"),
            Err(SocketError::InvalidState)
        );
        assert_eq!(
            sockets.complete_udp_send(OWNER, handle, 41, b"reply"),
            Ok(5)
        );
        assert!(!sockets.udp_request_active(OWNER, handle, 41, 5));
        let mut response = [0u8; 8];
        assert_eq!(sockets.receive(OWNER, handle, &mut response), Ok(5));
        assert_eq!(&response[..5], b"reply");
    }

    #[test]
    fn tcp_transport_preserves_connecting_state_until_exact_completion() {
        let mut sockets = SocketSet::new();
        let handle = sockets.open(OWNER, SocketProtocol::TcpStream).unwrap();
        sockets.connect(OWNER, handle, 0x0a00_0202, 18080).unwrap();
        sockets.send(OWNER, handle, b"GET /").unwrap();
        assert_eq!(
            sockets.pending_transport(OWNER),
            Some((handle, SocketProtocol::TcpStream))
        );
        let mut request = [0u8; SOCKET_BUFFER_CAPACITY];
        assert_eq!(
            sockets.begin_transport(OWNER, handle, SocketProtocol::TcpStream, 77, &mut request,),
            Ok((0x0a00_0202, 18080, 5))
        );
        assert_eq!(&request[..5], b"GET /");
        assert!(sockets.transport_request_active(OWNER, handle, SocketProtocol::TcpStream, 77, 5,));
        assert_eq!(
            sockets.complete_transport(OWNER, handle, SocketProtocol::Udp, 77, b"HTTP",),
            Err(SocketError::InvalidState)
        );
        assert_eq!(
            sockets.complete_transport(OWNER, handle, SocketProtocol::TcpStream, 78, b"HTTP",),
            Err(SocketError::InvalidState)
        );
        assert_eq!(
            sockets.complete_transport(OWNER, handle, SocketProtocol::TcpStream, 77, b"HTTP",),
            Ok(4)
        );
        assert_eq!(
            sockets.status(OWNER, handle).unwrap().state,
            SocketState::Established
        );
    }

    #[test]
    fn listener_backlog_and_accepted_children_are_bounded_capabilities() {
        let mut sockets = SocketSet::new();
        let listener = sockets.open(OWNER, SocketProtocol::TcpStream).unwrap();
        assert_eq!(
            sockets.bind(OWNER, listener, SOCKET_LISTENER_PORT_MIN - 1),
            Err(SocketError::InvalidState)
        );
        sockets
            .bind(OWNER, listener, SOCKET_LISTENER_PORT_MIN)
            .unwrap();
        assert!(sockets.owns_local_port(SOCKET_LISTENER_PORT_MIN));
        sockets.listen(OWNER, listener, 2).unwrap();
        assert_eq!(
            sockets.accept(OWNER, listener),
            Err(SocketError::WouldBlock)
        );
        sockets
            .queue_incoming(OWNER, listener, server_peer(0x0a00_0202, 50000))
            .unwrap();
        sockets
            .queue_incoming(OWNER, listener, server_peer(0x0a00_0203, 50001))
            .unwrap();
        assert_eq!(
            sockets.queue_incoming(OWNER, listener, server_peer(0x0a00_0204, 50002)),
            Err(SocketError::WouldBlock)
        );
        let status = sockets.status(OWNER, listener).unwrap();
        assert_eq!(status.state, SocketState::Listening);
        assert_ne!(status.readiness & SOCKET_READY_ACCEPT, 0);
        assert_eq!(status.readiness & SOCKET_READY_WRITABLE, 0);
        let first = sockets.accept(OWNER, listener).unwrap();
        assert_eq!(
            sockets.status(OWNER, first).unwrap().state,
            SocketState::Established
        );
        assert_eq!(sockets.remote(OWNER, first), Ok((0x0a00_0202, 50000)));
        assert_ne!(
            sockets.status(OWNER, first).unwrap().readiness & SOCKET_READY_WRITABLE,
            0
        );
        let first_peer = server_peer(0x0a00_0202, 50000);
        assert_eq!(sockets.server_peer(OWNER, first), Ok(first_peer));
        assert_eq!(sockets.send(OWNER, first, b"pong"), Ok(4));
        assert_eq!(sockets.pending_transport(OWNER), None);
        assert!(sockets.server_send_pending(OWNER, first, first_peer));
        let mut output = [0; SOCKET_BUFFER_CAPACITY];
        assert_eq!(
            sockets.begin_server_send(OWNER, first, 88, &mut output),
            Ok(4)
        );
        assert_eq!(&output[..4], b"pong");
        assert!(sockets.server_send_active(OWNER, first, first_peer, 88, 4));
        assert_eq!(
            sockets.shutdown(OWNER, first, false, true),
            Err(SocketError::WouldBlock)
        );
        assert_eq!(
            sockets.complete_server_send(OWNER, first, server_peer(0x0a00_0202, 50001), 88),
            Err(SocketError::InvalidState)
        );
        assert_eq!(
            sockets.complete_server_send(OWNER, first, first_peer, 88),
            Ok(4)
        );
        assert_eq!(sockets.push_receive(OWNER, first, b"ping"), Ok(4));
        let mut input = [0; 4];
        assert_eq!(sockets.receive(OWNER, first, &mut input), Ok(4));
        assert_eq!(&input, b"ping");
        sockets
            .mark_server_read_closed(OWNER, first, first_peer)
            .unwrap();
        assert_eq!(
            sockets.status(OWNER, first).unwrap().state,
            SocketState::ReadClosed
        );
        sockets.shutdown(OWNER, first, false, true).unwrap();
        assert_eq!(
            sockets.status(OWNER, first).unwrap().state,
            SocketState::WriteClosed
        );
        sockets
            .mark_server_closed(OWNER, first, first_peer)
            .unwrap();
        assert_eq!(
            sockets.status(OWNER, first).unwrap().state,
            SocketState::Closed
        );
        assert_eq!(
            sockets.entry(OWNER, first).unwrap().local_port,
            SOCKET_LISTENER_PORT_MIN
        );
        assert!(sockets.status(OTHER, first).is_err());
        let second = sockets.accept(OWNER, listener).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            sockets.accept(OWNER, listener),
            Err(SocketError::WouldBlock)
        );
        sockets.close(OWNER, listener).unwrap();
        assert!(!sockets.owns_local_port(SOCKET_LISTENER_PORT_MIN));
    }

    #[test]
    fn local_ports_are_exclusive_across_process_socket_sets_and_reusable_after_close() {
        let mut first = SocketSet::new();
        let second = SocketSet::new();
        let listener = first.open(OWNER, SocketProtocol::TcpStream).unwrap();
        assert!(local_port_is_available(
            [&first, &second].into_iter(),
            SOCKET_LISTENER_PORT_MIN
        ));
        first
            .bind(OWNER, listener, SOCKET_LISTENER_PORT_MIN)
            .unwrap();
        assert!(!local_port_is_available(
            [&first, &second].into_iter(),
            SOCKET_LISTENER_PORT_MIN
        ));
        first.close(OWNER, listener).unwrap();
        assert!(local_port_is_available(
            [&first, &second].into_iter(),
            SOCKET_LISTENER_PORT_MIN
        ));
    }

    #[test]
    fn failed_wire_peer_is_removed_from_the_listener_backlog() {
        let mut sockets = SocketSet::new();
        let listener = sockets.open(OWNER, SocketProtocol::TcpStream).unwrap();
        sockets
            .bind(OWNER, listener, SOCKET_LISTENER_PORT_MIN)
            .unwrap();
        sockets.listen(OWNER, listener, 1).unwrap();
        let peer = server_peer(0x0a00_0202, 50000);
        sockets.queue_incoming(OWNER, listener, peer).unwrap();
        sockets.drop_incoming(OWNER, listener, peer).unwrap();
        assert_eq!(
            sockets.accept(OWNER, listener),
            Err(SocketError::WouldBlock)
        );
        assert_eq!(
            sockets.drop_incoming(OWNER, listener, peer),
            Err(SocketError::InvalidState)
        );
    }

    #[test]
    fn accept_capacity_failure_preserves_the_oldest_pending_peer() {
        let mut sockets = SocketSet::new();
        let listener = sockets.open(OWNER, SocketProtocol::TcpStream).unwrap();
        sockets
            .bind(OWNER, listener, SOCKET_LISTENER_PORT_MIN)
            .unwrap();
        sockets.listen(OWNER, listener, 1).unwrap();
        sockets
            .queue_incoming(OWNER, listener, server_peer(0x0a00_0202, 50000))
            .unwrap();
        let first = sockets.open(OWNER, SocketProtocol::Udp).unwrap();
        let second = sockets.open(OWNER, SocketProtocol::Udp).unwrap();
        let third = sockets.open(OWNER, SocketProtocol::Udp).unwrap();
        assert_eq!(sockets.accept(OWNER, listener), Err(SocketError::Capacity));
        assert_ne!(
            sockets.status(OWNER, listener).unwrap().readiness & SOCKET_READY_ACCEPT,
            0
        );
        sockets.close(OWNER, first).unwrap();
        let accepted = sockets.accept(OWNER, listener).unwrap();
        assert_eq!(sockets.remote(OWNER, accepted), Ok((0x0a00_0202, 50000)));
        sockets.close(OWNER, second).unwrap();
        sockets.close(OWNER, third).unwrap();
    }

    #[test]
    fn shutdown_cancels_queued_work_and_owner_exit_reclaims_everything() {
        let mut sockets = SocketSet::new();
        let a = sockets.open(OWNER, SocketProtocol::Udp).unwrap();
        let b = sockets.open(OWNER, SocketProtocol::TcpStream).unwrap();
        sockets.connect(OWNER, a, 1, 1).unwrap();
        sockets.send(OWNER, a, b"queued").unwrap();
        sockets.shutdown(OWNER, a, false, true).unwrap();
        assert_eq!(sockets.status(OWNER, a).unwrap().queued_send, 0);
        assert_eq!(sockets.close_owner(OWNER), 2);
        assert!(sockets.status(OWNER, a).is_err());
        assert!(sockets.status(OWNER, b).is_err());
    }

    #[test]
    fn capacity_is_hard_bounded_and_failed_sockets_are_observable() {
        let mut sockets = SocketSet::new();
        let mut handles = [0; SOCKET_CAPACITY];
        for handle in &mut handles {
            *handle = sockets.open(OWNER, SocketProtocol::TcpStream).unwrap();
        }
        assert_eq!(
            sockets.open(OWNER, SocketProtocol::Udp),
            Err(SocketError::Capacity)
        );
        sockets.fail(OWNER, handles[0]).unwrap();
        let status = sockets.status(OWNER, handles[0]).unwrap();
        assert_eq!(status.state, SocketState::Failed);
        assert_ne!(status.readiness & SOCKET_READY_ERROR, 0);
        assert_ne!(status.readiness & SOCKET_READY_CLOSED, 0);
    }
}
