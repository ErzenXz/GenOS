use kernel::{
    display::FixedText,
    input::{InputEvent, KeyEvent},
    socket::SocketProtocol,
    tasks::{TaskRegistry, TaskSnapshotSet, TaskState},
    vfs::{NodeKind, RamVfs},
};

use crate::{network, serial, storage, userspace};

const MAX_RUNTIME_EVENTS: usize = 4;
pub const SHELL_TASK_ID: u32 = 0x100;
static mut VFS_ROLLBACK: RamVfs = RamVfs::new();

enum SocketTransportProgress {
    Pending,
    Complete {
        bytes: [u8; genos_abi::USER_SOCKET_BUFFER_CAPACITY as usize],
        len: usize,
    },
    Failed,
}

#[derive(Clone, Copy)]
pub struct TaskIds {
    pub desktop: u32,
    pub shell: u32,
    pub input: u32,
    pub vfs: u32,
    pub idle: u32,
}

// This fixed-capacity no-heap event remains Copy; indirection would change that contract.
#[allow(dead_code, clippy::large_enum_variant)]
#[derive(Clone, Copy)]
pub enum RuntimeEvent {
    Process(userspace::ProcessUpdate),
    Error(userspace::LaunchError),
}

pub struct RuntimeBatch {
    events: [Option<RuntimeEvent>; MAX_RUNTIME_EVENTS],
    len: usize,
    pub vfs_changed: bool,
}

impl RuntimeBatch {
    const fn new() -> Self {
        Self {
            events: [None; MAX_RUNTIME_EVENTS],
            len: 0,
            vfs_changed: false,
        }
    }

    fn push(&mut self, event: RuntimeEvent) {
        if self.len < self.events.len() {
            self.events[self.len] = Some(event);
            self.len += 1;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = RuntimeEvent> + '_ {
        self.events[..self.len].iter().filter_map(|event| *event)
    }
}

pub struct RuntimeCoordinator {
    tasks: TaskRegistry,
    task_snapshot: TaskSnapshotSet,
    ids: TaskIds,
    processes: userspace::ProcessManager,
    vfs: RamVfs,
    persistent_fs: storage::PersistentFs,
    pending_vfs_request: Option<userspace::UserVfsRequest>,
    pending_lifecycle_request: Option<userspace::UserLifecycleRequest>,
    pending_socket_request: Option<userspace::UserSocketRequest>,
    socket_transport_started: bool,
    pending_tcp_listener: Option<userspace::UserTcpListener>,
    tcp_stream_listener: Option<userspace::UserTcpListener>,
    tcp_stream: Option<userspace::UserTcpStream>,
    pending_tcp_stream_send: Option<userspace::UserTcpStreamSend>,
    tcp_stream_close_started: bool,
    next_process_task_id: u32,
    completed_vfs_requests: u64,
    completed_lifecycle_launches: u64,
    last_completed_vfs_identity: Option<userspace::AsyncRequestIdentity>,
    last_completed_lifecycle_identity: Option<userspace::AsyncRequestIdentity>,
    last_completed_socket_identity: Option<userspace::AsyncRequestIdentity>,
}

impl RuntimeCoordinator {
    pub fn new(
        tasks: TaskRegistry,
        ids: TaskIds,
        processes: userspace::ProcessManager,
        vfs: RamVfs,
        persistent_fs: storage::PersistentFs,
    ) -> Self {
        let mut coordinator = Self {
            tasks,
            task_snapshot: TaskSnapshotSet::new(),
            ids,
            processes,
            vfs,
            persistent_fs,
            pending_vfs_request: None,
            pending_lifecycle_request: None,
            pending_socket_request: None,
            socket_transport_started: false,
            pending_tcp_listener: None,
            tcp_stream_listener: None,
            tcp_stream: None,
            pending_tcp_stream_send: None,
            tcp_stream_close_started: false,
            next_process_task_id: SHELL_TASK_ID + 1,
            completed_vfs_requests: 0,
            completed_lifecycle_launches: 0,
            last_completed_vfs_identity: None,
            last_completed_lifecycle_identity: None,
            last_completed_socket_identity: None,
        };
        coordinator.refresh_task_snapshot();
        coordinator
    }

    #[allow(dead_code)]
    // Retained for the deferred graphical task surface until ROADMAP F4 removes or isolates it.
    pub fn task_snapshot(&self) -> &TaskSnapshotSet {
        &self.task_snapshot
    }

    pub fn process_snapshot_is_authoritative(&self) -> bool {
        self.processes.task_snapshots_match(&self.task_snapshot)
    }

    pub fn unified_handle_table_is_authoritative(&self) -> bool {
        self.processes.unified_handle_table_is_authoritative()
    }

    pub fn async_request_identity_is_authoritative(&self) -> bool {
        let Some(vfs) = self.last_completed_vfs_identity else {
            return false;
        };
        let Some(lifecycle) = self.last_completed_lifecycle_identity else {
            return false;
        };
        let socket_identity_is_valid =
            match (network::config(), self.last_completed_socket_identity) {
                (Some(_), Some(socket)) => {
                    socket.request_id != 0
                        && vfs.owner_slot == socket.owner_slot
                        && vfs.owner_instance == socket.owner_instance
                        && vfs.request_id != socket.request_id
                        && lifecycle.request_id != socket.request_id
                }
                (None, None) => true,
                _ => false,
            };
        vfs.request_id != 0
            && lifecycle.request_id != 0
            && vfs.owner_slot == lifecycle.owner_slot
            && vfs.owner_instance == lifecycle.owner_instance
            && vfs.request_id != lifecycle.request_id
            && socket_identity_is_valid
            && self.pending_vfs_request.is_none()
            && self.pending_lifecycle_request.is_none()
            && self.pending_socket_request.is_none()
            && !self.socket_transport_started
    }

    #[allow(dead_code)]
    // Retained for the deferred graphical file surface until ROADMAP F4 removes or isolates it.
    pub fn vfs(&self) -> &RamVfs {
        &self.vfs
    }

    pub fn console_process_active(&self) -> bool {
        self.processes.console_process_active()
    }

    pub fn console_input_ready(&self) -> bool {
        self.processes.console_input_ready()
    }

    pub fn run_headless_boot_probe(&mut self, max_steps: u16) -> bool {
        let initial_vfs = self.completed_vfs_requests;
        let initial_lifecycle = self.completed_lifecycle_launches;
        for tick in 0..u64::from(max_steps) {
            let _ = self.advance(tick);
            self.finish_iteration(false, tick);
            if self.completed_vfs_requests > initial_vfs
                && self.completed_lifecycle_launches > initial_lifecycle
            {
                serial::print("HEADLESS_RUNTIME_OK vfs=");
                serial::print_u64(self.completed_vfs_requests - initial_vfs);
                serial::print(" lifecycle=");
                serial::print_u64(self.completed_lifecycle_launches - initial_lifecycle);
                serial::println("");
                return true;
            }
        }
        false
    }

    pub fn run_console_transcript_probe(&mut self) -> bool {
        for _ in 0..1024 {
            if self.console_input_ready() {
                break;
            }
            let _ = self.advance(0);
            self.finish_iteration(false, 0);
        }
        if !self.console_input_ready() {
            return false;
        }

        let mut saw_echo_prompt = false;
        let mut saw_echo_output = false;
        let mut saw_uname_prompt = false;
        let mut saw_uname_output = false;
        if !self.drive_console_command(
            b"echo qemu-console",
            &mut saw_echo_prompt,
            &mut saw_echo_output,
            b"/> echo qemu-console",
            b"qemu-console",
        ) || !self.drive_console_command(
            b"uname",
            &mut saw_uname_prompt,
            &mut saw_uname_output,
            b"/> uname",
            b"GenOS v0.49 ring3-shell x86_64 ABI 17",
        ) {
            return false;
        }
        if saw_echo_prompt && saw_echo_output && saw_uname_prompt && saw_uname_output {
            serial::println("USER_CONSOLE_TRANSCRIPT_OK commands=2");
            serial::println("USER_CONSOLE_HEADLESS_OK");
            true
        } else {
            false
        }
    }

    fn drive_console_command(
        &mut self,
        command: &[u8],
        saw_prompt: &mut bool,
        saw_output: &mut bool,
        expected_prompt: &[u8],
        expected_output: &[u8],
    ) -> bool {
        for event in command
            .iter()
            .copied()
            .map(|byte| InputEvent::Key(KeyEvent::Char(byte)))
            .chain(core::iter::once(InputEvent::Key(KeyEvent::Enter)))
        {
            if self.deliver_input(event).ok().flatten().is_none() {
                return false;
            }
            let mut rearmed = false;
            for _ in 0..512 {
                let batch = self.advance(0);
                for runtime_event in batch.iter() {
                    if let RuntimeEvent::Process(update) = runtime_event {
                        if let Some(userspace::ConsoleUpdate::Write { text, .. }) = update.console {
                            *saw_prompt |= text.as_str().as_bytes() == expected_prompt;
                            *saw_output |= text.as_str().as_bytes() == expected_output;
                        }
                    }
                }
                self.finish_iteration(false, 0);
                if self.console_input_ready() {
                    rearmed = true;
                    break;
                }
            }
            if !rearmed {
                return false;
            }
        }
        true
    }

    pub fn record_input_activity(&mut self, tick: u64) {
        self.tasks.mark_running(self.ids.input, tick);
    }

    #[allow(dead_code)]
    // Retained for the legacy graphical shell accounting until ROADMAP F4 removes or isolates it.
    pub fn record_shell_activity(&mut self, tick: u64) {
        self.tasks.mark_running(self.ids.shell, tick);
    }

    #[allow(dead_code)]
    // Retained for the legacy desktop accounting until ROADMAP F4 removes or isolates it.
    pub fn record_desktop_activity(&mut self, tick: u64) {
        self.tasks.mark_running(self.ids.desktop, tick);
    }

    pub fn deliver_input(
        &mut self,
        event: InputEvent,
    ) -> Result<Option<userspace::ProcessUpdate>, userspace::LaunchError> {
        self.processes.deliver_input(event)
    }

    pub fn advance(&mut self, tick: u64) -> RuntimeBatch {
        let mut batch = RuntimeBatch::new();
        self.complete_lifecycle_request(&mut batch);
        self.complete_vfs_request(tick, &mut batch);
        self.complete_socket_request(tick, &mut batch);
        if !self.socket_transport_started {
            self.complete_passive_tcp_stream(tick);
            self.complete_passive_tcp(tick);
        }

        if let Some(update) = self.processes.poll(tick) {
            if let Some(request) = update.vfs_request {
                self.pending_vfs_request = Some(request);
            }
            if let Some(request) = update.lifecycle_request {
                self.pending_lifecycle_request = Some(request);
            }
            if let Some(request) = update.socket_request {
                if self.pending_socket_request.is_none() {
                    self.pending_socket_request = Some(request);
                } else if let Ok(owner) = self.processes.complete_socket_request(request, None) {
                    serial::println("USER_SOCKET_TRANSPORT_QUEUE_FULL");
                    batch.push(RuntimeEvent::Process(owner));
                }
            }
            batch.push(RuntimeEvent::Process(update));
        }

        self.tasks.scheduler_tick(tick);
        self.tasks.mark_running(self.ids.desktop, tick);
        self.tasks
            .set_state(self.ids.input, TaskState::Waiting, tick);
        batch
    }

    pub fn finish_iteration(&mut self, handled_event: bool, tick: u64) {
        if handled_event {
            self.tasks.set_state(self.ids.shell, TaskState::Ready, tick);
            self.tasks
                .set_state(self.ids.input, TaskState::Waiting, tick);
            self.tasks
                .set_state(self.ids.idle, TaskState::Sleeping, tick);
        } else {
            self.tasks
                .set_state(self.ids.idle, TaskState::Running, tick);
            self.tasks.tick_idle(tick);
        }
        self.refresh_task_snapshot();
    }

    fn complete_lifecycle_request(&mut self, batch: &mut RuntimeBatch) {
        let Some(pending) = self.pending_lifecycle_request.take() else {
            return;
        };
        if !self.processes.lifecycle_request_active(pending) {
            serial::println("USER_LIFECYCLE_STALE_REQUEST_DROPPED");
            return;
        }
        let userspace::UserLifecycleRequest::Launch(request) = pending;
        let identity = pending.identity();

        let task_id = self.allocate_process_task_id();
        let live_before = self.processes.live_count();
        match self
            .processes
            .complete_process_launch(request, Some(task_id))
        {
            Ok(completion) => {
                if self.processes.live_count() > live_before {
                    self.completed_lifecycle_launches =
                        self.completed_lifecycle_launches.saturating_add(1);
                    self.last_completed_lifecycle_identity = Some(identity);
                }
                batch.push(RuntimeEvent::Process(completion.owner));
            }
            Err(error) => batch.push(RuntimeEvent::Error(error)),
        }
    }

    fn complete_vfs_request(&mut self, tick: u64, batch: &mut RuntimeBatch) {
        let Some(request) = self.pending_vfs_request.take() else {
            return;
        };
        if !self.processes.vfs_request_active(request) {
            serial::println("USER_VFS_STALE_REQUEST_DROPPED");
            return;
        }
        let identity = request.identity();

        self.tasks.mark_running(self.ids.vfs, tick);
        let completion = match request {
            userspace::UserVfsRequest::Open(request) => {
                let writable = request.rights & genos_abi::USER_FILE_RIGHT_WRITE != 0;
                let manageable = request.rights & genos_abi::USER_FILE_RIGHT_MANAGE != 0;
                let mutable_path = request.path.as_str().eq_ignore_ascii_case("/USER")
                    || userspace::is_user_writable_path(request.path.as_str());
                let mut allowed = (!writable && !manageable)
                    || (writable && userspace::is_user_writable_path(request.path.as_str()))
                    || (manageable && !writable && mutable_path);
                if self.persistent_write_denied(request.path.as_str()) && (writable || manageable) {
                    allowed = false;
                    serial::println("PERSISTENT_READ_ONLY_MUTATION_DENIED");
                }
                if allowed
                    && writable
                    && !manageable
                    && self.vfs.find(request.path.as_str()).is_none()
                {
                    capture_vfs(&self.vfs);
                    let created = self.vfs.touch(request.path.as_str()).is_ok();
                    if created && self.persist_change() {
                        batch.vfs_changed = true;
                    } else {
                        allowed = false;
                    }
                }
                let info = allowed
                    .then(|| self.vfs.find(request.path.as_str()))
                    .flatten()
                    .and_then(|node| match node.kind() {
                        NodeKind::File if !manageable => Some(userspace::FileOpenInfo {
                            size: node.len() as u64,
                            kind: genos_abi::USER_FILE_KIND_REGULAR,
                        }),
                        NodeKind::File => None,
                        NodeKind::Directory => Some(userspace::FileOpenInfo {
                            size: 0,
                            kind: genos_abi::USER_FILE_KIND_DIRECTORY,
                        }),
                    });
                self.processes.complete_file_open(request, info)
            }
            userspace::UserVfsRequest::Read(request) => {
                let bytes = self.vfs.read(request.path.as_str()).ok().map(|data| {
                    let start = (request.offset as usize).min(data.len());
                    &data[start..]
                });
                self.processes.complete_file_read(request, bytes)
            }
            userspace::UserVfsRequest::Write(request) => {
                let written = if self.persistent_write_denied(request.path.as_str()) {
                    serial::println("PERSISTENT_READ_ONLY_MUTATION_DENIED");
                    None
                } else {
                    capture_vfs(&self.vfs);
                    self.vfs
                        .write_at(
                            request.path.as_str(),
                            request.offset as usize,
                            request.data.as_slice(),
                        )
                        .ok()
                        .map(|count| count as u64)
                        .filter(|_| self.persist_change())
                };
                batch.vfs_changed |= written.is_some();
                self.processes.complete_file_write(request, written)
            }
            userspace::UserVfsRequest::Truncate(request) => {
                let truncated = if self.persistent_write_denied(request.path.as_str()) {
                    serial::println("PERSISTENT_READ_ONLY_MUTATION_DENIED");
                    false
                } else {
                    capture_vfs(&self.vfs);
                    self.vfs.truncate(request.path.as_str()).is_ok() && self.persist_change()
                };
                batch.vfs_changed |= truncated;
                self.processes.complete_file_truncate(request, truncated)
            }
            userspace::UserVfsRequest::ReadDirectory(request) => {
                let result = match self.vfs.read_dir_at(
                    request.path.as_str(),
                    request.cursor.min(usize::MAX as u64) as usize,
                ) {
                    Ok(Some(node)) => {
                        let name = node.path().rsplit('/').next().unwrap_or("");
                        let kind = match node.kind() {
                            NodeKind::File => genos_abi::USER_FILE_KIND_REGULAR,
                            NodeKind::Directory => genos_abi::USER_FILE_KIND_DIRECTORY,
                        };
                        userspace::DirectoryReadResult::Entry(userspace::DirectoryEntryInfo {
                            name: FixedText::from_str(name),
                            kind,
                            size: node.len() as u64,
                        })
                    }
                    Ok(None) => userspace::DirectoryReadResult::End,
                    Err(_) => userspace::DirectoryReadResult::Unavailable,
                };
                self.processes.complete_directory_read(request, result)
            }
            userspace::UserVfsRequest::CreateDirectory(request) => {
                let created = if self.persistent_write_denied(request.target.as_str()) {
                    serial::println("PERSISTENT_READ_ONLY_MUTATION_DENIED");
                    false
                } else {
                    capture_vfs(&self.vfs);
                    self.vfs.mkdir(request.target.as_str()).is_ok() && self.persist_change()
                };
                batch.vfs_changed |= created;
                self.processes.complete_directory_create(request, created)
            }
            userspace::UserVfsRequest::RemovePath(request) => {
                let removed = if self.persistent_write_denied(request.target.as_str()) {
                    serial::println("PERSISTENT_READ_ONLY_MUTATION_DENIED");
                    false
                } else {
                    capture_vfs(&self.vfs);
                    self.vfs.remove(request.target.as_str()).is_ok() && self.persist_change()
                };
                batch.vfs_changed |= removed;
                self.processes.complete_path_remove(request, removed)
            }
        };

        match completion {
            Ok(update) => {
                self.completed_vfs_requests = self.completed_vfs_requests.saturating_add(1);
                self.last_completed_vfs_identity = Some(identity);
                batch.push(RuntimeEvent::Process(update));
            }
            Err(error) => batch.push(RuntimeEvent::Error(error)),
        }
    }

    fn complete_socket_request(&mut self, tick: u64, batch: &mut RuntimeBatch) {
        let Some(request) = self.pending_socket_request else {
            return;
        };
        if !self.processes.socket_request_active(request) {
            network::cancel_socket_async();
            self.pending_socket_request = None;
            self.socket_transport_started = false;
            serial::println(match request.protocol {
                SocketProtocol::Udp => "USER_SOCKET_STALE_REQUEST_DROPPED protocol=udp",
                SocketProtocol::TcpStream => "USER_SOCKET_STALE_REQUEST_DROPPED protocol=tcp",
            });
            return;
        }
        if !self.socket_transport_started {
            if self.pending_tcp_listener.is_some()
                || network::tcp_passive_active()
                || network::tcp_passive_stream_peer().is_some()
            {
                return;
            }
            let started = match request.protocol {
                SocketProtocol::Udp => network::start_udp_async(
                    request.target,
                    request.port,
                    request.data.as_slice(),
                    tick,
                ),
                SocketProtocol::TcpStream => network::start_tcp_async(
                    request.target,
                    request.port,
                    request.data.as_slice(),
                    tick,
                ),
            };
            if !started {
                let completion = self.processes.complete_socket_request(request, None);
                self.pending_socket_request = None;
                self.socket_transport_started = false;
                if let Ok(update) = completion {
                    batch.push(RuntimeEvent::Process(update));
                }
                return;
            }
            self.socket_transport_started = true;
            serial::println(match request.protocol {
                SocketProtocol::Udp => "USER_SOCKET_TRANSPORT_STARTED protocol=udp",
                SocketProtocol::TcpStream => "USER_SOCKET_TRANSPORT_STARTED protocol=tcp",
            });
        }
        let progress = match request.protocol {
            SocketProtocol::Udp => match network::poll_udp_async(tick) {
                network::AsyncUdpProgress::Idle | network::AsyncUdpProgress::Failed => {
                    SocketTransportProgress::Failed
                }
                network::AsyncUdpProgress::Pending => SocketTransportProgress::Pending,
                network::AsyncUdpProgress::Complete { bytes, len } => {
                    SocketTransportProgress::Complete { bytes, len }
                }
            },
            SocketProtocol::TcpStream => match network::poll_tcp_async(tick) {
                network::AsyncTcpProgress::Idle | network::AsyncTcpProgress::Failed => {
                    SocketTransportProgress::Failed
                }
                network::AsyncTcpProgress::Pending => SocketTransportProgress::Pending,
                network::AsyncTcpProgress::Complete { bytes, len } => {
                    SocketTransportProgress::Complete { bytes, len }
                }
            },
        };
        match progress {
            SocketTransportProgress::Pending => {}
            SocketTransportProgress::Complete { bytes, len } => {
                let identity = request.identity();
                let completion = self
                    .processes
                    .complete_socket_request(request, Some(&bytes[..len]));
                self.pending_socket_request = None;
                self.socket_transport_started = false;
                if let Ok(update) = completion {
                    self.last_completed_socket_identity = Some(identity);
                    serial::println(match request.protocol {
                        SocketProtocol::Udp => "USER_SOCKET_TRANSPORT_COMPLETE protocol=udp",
                        SocketProtocol::TcpStream => "USER_SOCKET_TRANSPORT_COMPLETE protocol=tcp",
                    });
                    batch.push(RuntimeEvent::Process(update));
                }
            }
            SocketTransportProgress::Failed => {
                let completion = self.processes.complete_socket_request(request, None);
                self.pending_socket_request = None;
                self.socket_transport_started = false;
                if let Ok(update) = completion {
                    batch.push(RuntimeEvent::Process(update));
                }
            }
        }
    }

    fn complete_passive_tcp(&mut self, tick: u64) {
        if self
            .pending_tcp_listener
            .is_some_and(|listener| !self.processes.tcp_listener_active(listener))
        {
            network::cancel_tcp_passive();
            self.pending_tcp_listener = None;
            serial::println("TCP_PASSIVE_STALE_LISTENER_DROPPED");
        }
        match network::poll_tcp_passive(tick) {
            network::PassiveTcpProgress::Idle | network::PassiveTcpProgress::Pending => {}
            network::PassiveTcpProgress::Syn(syn) => {
                let Some(listener) = self.processes.tcp_listener(syn.local_port) else {
                    network::reject_tcp_syn(syn);
                    return;
                };
                if network::start_tcp_passive(syn, tick) {
                    self.pending_tcp_listener = Some(listener);
                    serial::println("TCP_PASSIVE_SYN_ACCEPTED");
                } else {
                    network::reject_tcp_syn(syn);
                }
            }
            network::PassiveTcpProgress::Established(peer) => {
                let listener = self.pending_tcp_listener.take();
                if listener.is_some_and(|listener| {
                    if !network::start_tcp_passive_stream(peer, tick) {
                        return false;
                    }
                    if self.processes.queue_tcp_peer(listener, peer).is_ok() {
                        self.tcp_stream_listener = Some(listener);
                        true
                    } else {
                        network::cancel_tcp_passive_stream();
                        false
                    }
                }) {
                    serial::println("TCP_PASSIVE_HANDSHAKE_OK");
                } else {
                    network::reject_tcp_peer(peer);
                    serial::println("TCP_PASSIVE_BACKLOG_REFUSED");
                }
            }
            network::PassiveTcpProgress::Failed => {
                self.pending_tcp_listener = None;
                serial::println("TCP_PASSIVE_TIMEOUT");
            }
        }
    }

    fn complete_passive_tcp_stream(&mut self, tick: u64) {
        let Some(peer) = network::tcp_passive_stream_peer() else {
            self.tcp_stream_listener = None;
            self.tcp_stream = None;
            self.pending_tcp_stream_send = None;
            self.tcp_stream_close_started = false;
            return;
        };

        if self
            .tcp_stream
            .is_some_and(|stream| !self.processes.tcp_stream_active(stream))
        {
            network::cancel_tcp_passive_stream();
            self.tcp_stream = None;
            self.pending_tcp_stream_send = None;
            self.tcp_stream_listener = None;
            self.tcp_stream_close_started = false;
            serial::println("TCP_PASSIVE_STREAM_STALE_CAPABILITY");
            return;
        }
        if self.tcp_stream.is_none() {
            self.tcp_stream = self.processes.tcp_stream(peer);
            if self.tcp_stream.is_some() {
                self.tcp_stream_listener = None;
            } else if self
                .tcp_stream_listener
                .is_none_or(|listener| !self.processes.tcp_listener_active(listener))
            {
                network::cancel_tcp_passive_stream();
                self.tcp_stream_listener = None;
                serial::println("TCP_PASSIVE_STREAM_UNCLAIMED");
                return;
            }
        }
        if self
            .pending_tcp_stream_send
            .is_some_and(|request| !self.processes.tcp_stream_send_active(request))
        {
            if let Some(stream) = self.tcp_stream {
                let _ = self.processes.fail_tcp_stream(stream);
            }
            network::cancel_tcp_passive_stream();
            self.pending_tcp_stream_send = None;
            self.tcp_stream = None;
            self.tcp_stream_close_started = false;
            serial::println("TCP_PASSIVE_STREAM_STALE_SEND");
            return;
        }

        match network::poll_tcp_passive_stream(tick) {
            network::PassiveTcpStreamProgress::Idle
            | network::PassiveTcpStreamProgress::Pending => {}
            network::PassiveTcpStreamProgress::Received {
                peer: received_peer,
                bytes,
                len,
            } => {
                if received_peer == peer
                    && self.tcp_stream.is_some_and(|stream| {
                        self.processes
                            .queue_tcp_stream_receive(stream, &bytes[..len])
                            .is_ok()
                    })
                    && network::consume_tcp_passive_stream_receive(peer)
                {
                    serial::println("TCP_PASSIVE_STREAM_RX_OK");
                }
            }
            network::PassiveTcpStreamProgress::SendComplete(completed_peer) => {
                if completed_peer == peer
                    && self.pending_tcp_stream_send.is_some_and(|request| {
                        self.processes.complete_tcp_stream_send(request).is_ok()
                    })
                    && network::consume_tcp_passive_stream_send(peer)
                {
                    self.pending_tcp_stream_send = None;
                    serial::println("TCP_PASSIVE_STREAM_TX_OK");
                }
            }
            network::PassiveTcpStreamProgress::PeerClosed(closed_peer) => {
                if closed_peer == peer
                    && self.tcp_stream.is_some_and(|stream| {
                        self.processes.mark_tcp_stream_read_closed(stream).is_ok()
                    })
                    && network::consume_tcp_passive_peer_close(peer)
                {
                    serial::println("TCP_PASSIVE_STREAM_PEER_FIN_OK");
                }
            }
            network::PassiveTcpStreamProgress::Closed(closed_peer) => {
                if closed_peer == peer
                    && self
                        .tcp_stream
                        .is_some_and(|stream| self.processes.mark_tcp_stream_closed(stream).is_ok())
                    && network::finish_tcp_passive_stream(peer)
                {
                    serial::println("TCP_PASSIVE_STREAM_FIN_OK");
                }
                self.pending_tcp_stream_send = None;
                self.tcp_stream_listener = None;
                self.tcp_stream = None;
                self.tcp_stream_close_started = false;
                return;
            }
            network::PassiveTcpStreamProgress::Reset(reset_peer)
            | network::PassiveTcpStreamProgress::Failed(reset_peer) => {
                if reset_peer == peer {
                    if let Some(stream) = self.tcp_stream {
                        let _ = self.processes.fail_tcp_stream(stream);
                    } else if let Some(listener) = self.tcp_stream_listener {
                        let _ = self.processes.drop_tcp_peer(listener, peer);
                    }
                    let _ = network::finish_tcp_passive_stream(peer);
                }
                self.pending_tcp_stream_send = None;
                self.tcp_stream_listener = None;
                self.tcp_stream = None;
                self.tcp_stream_close_started = false;
                serial::println("TCP_PASSIVE_STREAM_FAILED");
                return;
            }
        }

        let Some(stream) = self.tcp_stream else {
            return;
        };
        if self.pending_tcp_stream_send.is_none() {
            if let Some(request) = self.processes.begin_tcp_stream_send(stream) {
                if network::start_tcp_passive_stream_send(peer, request.data.as_slice(), tick) {
                    self.pending_tcp_stream_send = Some(request);
                } else {
                    let _ = self.processes.fail_tcp_stream(stream);
                    network::cancel_tcp_passive_stream();
                    self.tcp_stream = None;
                    serial::println("TCP_PASSIVE_STREAM_FAILED");
                    return;
                }
            }
        }
        if self.pending_tcp_stream_send.is_none()
            && !self.tcp_stream_close_started
            && self.processes.tcp_stream_write_closed(stream)
        {
            if network::start_tcp_passive_stream_close(peer, tick) {
                self.tcp_stream_close_started = true;
                serial::println("TCP_PASSIVE_STREAM_FIN_SENT");
            } else {
                let _ = self.processes.fail_tcp_stream(stream);
                network::cancel_tcp_passive_stream();
                self.tcp_stream = None;
                serial::println("TCP_PASSIVE_STREAM_FAILED");
            }
        }
    }

    fn persist_change(&mut self) -> bool {
        if !self.persistent_fs.available() || self.persistent_fs.sync(&self.vfs) {
            return true;
        }
        restore_vfs(&mut self.vfs);
        serial::println("PERSISTENT_WRITE_FAILED");
        false
    }

    fn persistent_write_denied(&self, path: &str) -> bool {
        self.persistent_fs.read_only()
            && (path.eq_ignore_ascii_case("/USER") || userspace::is_user_writable_path(path))
    }

    fn allocate_process_task_id(&mut self) -> u32 {
        let task_id = self.next_process_task_id;
        self.next_process_task_id = self
            .next_process_task_id
            .wrapping_add(1)
            .max(SHELL_TASK_ID + 1);
        task_id
    }

    fn refresh_task_snapshot(&mut self) {
        let mut snapshot = self.tasks.snapshot();
        self.processes.append_task_snapshots(&mut snapshot);
        self.task_snapshot = snapshot;
    }
}

fn capture_vfs(vfs: &RamVfs) {
    unsafe {
        core::ptr::copy_nonoverlapping(vfs, core::ptr::addr_of_mut!(VFS_ROLLBACK), 1);
    }
}

fn restore_vfs(vfs: &mut RamVfs) {
    unsafe {
        core::ptr::copy_nonoverlapping(core::ptr::addr_of!(VFS_ROLLBACK), vfs, 1);
    }
}
