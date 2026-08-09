use kernel::{
    display::FixedText,
    input::{InputEvent, KeyEvent},
    tasks::{TaskRegistry, TaskSnapshotSet, TaskState},
    vfs::{NodeKind, RamVfs},
};

use crate::{serial, storage, userspace};

const MAX_RUNTIME_EVENTS: usize = 3;
pub const SHELL_TASK_ID: u32 = 0x100;
static mut VFS_ROLLBACK: RamVfs = RamVfs::new();

#[derive(Clone, Copy)]
pub struct TaskIds {
    pub desktop: u32,
    pub shell: u32,
    pub input: u32,
    pub vfs: u32,
    pub idle: u32,
}

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
    next_process_task_id: u32,
    completed_vfs_requests: u64,
    completed_lifecycle_launches: u64,
    last_completed_vfs_identity: Option<userspace::AsyncRequestIdentity>,
    last_completed_lifecycle_identity: Option<userspace::AsyncRequestIdentity>,
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
            next_process_task_id: SHELL_TASK_ID + 1,
            completed_vfs_requests: 0,
            completed_lifecycle_launches: 0,
            last_completed_vfs_identity: None,
            last_completed_lifecycle_identity: None,
        };
        coordinator.refresh_task_snapshot();
        coordinator
    }

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
        vfs.request_id != 0
            && lifecycle.request_id != 0
            && vfs.owner_slot == lifecycle.owner_slot
            && vfs.owner_instance == lifecycle.owner_instance
            && vfs.request_id != lifecycle.request_id
            && self.pending_vfs_request.is_none()
            && self.pending_lifecycle_request.is_none()
    }

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
        for _ in 0..max_steps {
            let _ = self.advance(0);
            self.finish_iteration(false, 0);
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
            b"GenOS v0.42 ring3-shell x86_64 ABI 15",
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

    pub fn record_shell_activity(&mut self, tick: u64) {
        self.tasks.mark_running(self.ids.shell, tick);
    }

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

        if let Some(update) = self.processes.poll(tick) {
            if let Some(request) = update.vfs_request {
                self.pending_vfs_request = Some(request);
            }
            if let Some(request) = update.lifecycle_request {
                self.pending_lifecycle_request = Some(request);
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
