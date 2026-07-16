use crate::display::FixedText;

pub const MAX_TASKS: usize = 16;
pub const DEFAULT_QUANTUM_TICKS: u8 = 5;
const WORK_UNITS_PER_SLICE: u64 = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskClass {
    System,
    Worker,
    User,
}

impl TaskClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Worker => "worker",
            Self::User => "user",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping,
    Waiting,
    Exited,
    Faulted,
}

impl TaskState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Sleeping => "sleeping",
            Self::Waiting => "waiting",
            Self::Exited => "exited",
            Self::Faulted => "fault",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Faulted)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskError {
    TableFull,
    NotFound,
    Protected,
    InvalidState,
}

#[derive(Clone, Copy)]
pub struct TaskRecord {
    pub id: u32,
    pub name: FixedText,
    pub class: TaskClass,
    pub state: TaskState,
    pub ticks: u64,
    pub wake_count: u64,
    pub memory_kib: u32,
    pub last_activity: u64,
    pub wake_at: u64,
    pub context_switches: u64,
    pub work_units: u64,
    pub checksum: u64,
    pub exit_code: i32,
    pub runtime_pid: u8,
}

impl TaskRecord {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            name: FixedText::empty(),
            class: TaskClass::System,
            state: TaskState::Exited,
            ticks: 0,
            wake_count: 0,
            memory_kib: 0,
            last_activity: 0,
            wake_at: 0,
            context_switches: 0,
            work_units: 0,
            checksum: 0,
            exit_code: 0,
            runtime_pid: 0,
        }
    }

    pub fn is_live(&self) -> bool {
        self.id != 0 && !self.state.is_terminal()
    }
}

pub struct TaskRegistry {
    tasks: [TaskRecord; MAX_TASKS],
    len: usize,
    next_id: u32,
    scheduler_cursor: usize,
    current_worker: Option<usize>,
    quantum_ticks: u8,
    quantum_used: u8,
    total_switches: u64,
}

impl TaskRegistry {
    pub const fn new() -> Self {
        Self {
            tasks: [TaskRecord::empty(); MAX_TASKS],
            len: 0,
            next_id: 1,
            scheduler_cursor: 0,
            current_worker: None,
            quantum_ticks: DEFAULT_QUANTUM_TICKS,
            quantum_used: 0,
            total_switches: 0,
        }
    }

    pub fn register(&mut self, name: &str, state: TaskState, memory_kib: u32) -> u32 {
        self.insert(name, TaskClass::System, state, memory_kib, 0, false)
            .unwrap_or(0)
    }

    pub fn spawn_worker(
        &mut self,
        name: &str,
        memory_kib: u32,
        tick: u64,
    ) -> Result<u32, TaskError> {
        if name.is_empty()
            || name.len() > 12
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(TaskError::InvalidState);
        }
        self.insert(
            name,
            TaskClass::Worker,
            TaskState::Ready,
            memory_kib,
            tick,
            true,
        )
    }

    pub fn record_user_exit(
        &mut self,
        name: &str,
        exit_code: u8,
        tick: u64,
    ) -> Result<u32, TaskError> {
        let pid = self.insert(name, TaskClass::User, TaskState::Exited, 20, tick, false)?;
        if let Some(task) = self.get_mut(pid) {
            task.exit_code = exit_code as i32;
        }
        Ok(pid)
    }

    pub fn record_user_fault(
        &mut self,
        name: &str,
        exit_code: u8,
        tick: u64,
    ) -> Result<u32, TaskError> {
        let pid = self.insert(name, TaskClass::User, TaskState::Faulted, 20, tick, false)?;
        if let Some(task) = self.get_mut(pid) {
            task.exit_code = exit_code as i32;
        }
        Ok(pid)
    }

    pub fn spawn_user(&mut self, name: &str, runtime_pid: u8, tick: u64) -> Result<u32, TaskError> {
        if runtime_pid == 0 {
            return Err(TaskError::InvalidState);
        }
        let pid = self.reserve_user(name, tick)?;
        self.bind_user_runtime(pid, runtime_pid)?;
        Ok(pid)
    }

    pub fn reserve_user(&mut self, name: &str, tick: u64) -> Result<u32, TaskError> {
        let reuse_terminal = self.len >= MAX_TASKS;
        self.insert(
            name,
            TaskClass::User,
            TaskState::Ready,
            40,
            tick,
            reuse_terminal,
        )
    }

    pub fn bind_user_runtime(&mut self, id: u32, runtime_pid: u8) -> Result<(), TaskError> {
        let task = self.get_mut(id).ok_or(TaskError::NotFound)?;
        if task.class != TaskClass::User || runtime_pid == 0 {
            return Err(TaskError::InvalidState);
        }
        task.runtime_pid = runtime_pid;
        Ok(())
    }

    pub fn update_user(
        &mut self,
        id: u32,
        state: TaskState,
        exit_code: u8,
        tick: u64,
    ) -> Result<(), TaskError> {
        let task = self.get_mut(id).ok_or(TaskError::NotFound)?;
        if task.class != TaskClass::User {
            return Err(TaskError::InvalidState);
        }
        task.state = state;
        task.exit_code = exit_code as i32;
        task.last_activity = tick;
        task.context_switches = task.context_switches.saturating_add(1);
        task.ticks = task.ticks.saturating_add(1);
        Ok(())
    }

    fn insert(
        &mut self,
        name: &str,
        class: TaskClass,
        state: TaskState,
        memory_kib: u32,
        tick: u64,
        reuse_exited: bool,
    ) -> Result<u32, TaskError> {
        let reusable = reuse_exited
            .then(|| {
                self.tasks
                    .iter()
                    .take(self.len)
                    .position(|task| task.state.is_terminal())
            })
            .flatten();
        let slot = reusable
            .or_else(|| (self.len < MAX_TASKS).then_some(self.len))
            .ok_or(TaskError::TableFull)?;

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.tasks[slot] = TaskRecord {
            id,
            name: FixedText::from_str(name),
            class,
            state,
            ticks: 0,
            wake_count: 0,
            memory_kib,
            last_activity: tick,
            wake_at: 0,
            context_switches: 0,
            work_units: 0,
            checksum: id as u64 ^ 0x4745_4e4f_5357_4f52,
            exit_code: 0,
            runtime_pid: 0,
        };
        if slot == self.len {
            self.len += 1;
        }
        Ok(id)
    }

    pub fn mark_running(&mut self, id: u32, tick: u64) {
        if let Some(task) = self.get_mut(id) {
            task.state = TaskState::Running;
            task.ticks += 1;
            task.wake_count += 1;
            task.last_activity = tick;
        }
    }

    pub fn set_state(&mut self, id: u32, state: TaskState, tick: u64) {
        if let Some(task) = self.get_mut(id) {
            task.state = state;
            task.last_activity = tick;
        }
    }

    pub fn tick_idle(&mut self, tick: u64) {
        for task in self.tasks.iter_mut().take(self.len) {
            if task.name.as_str() == "idle" {
                task.ticks += 1;
                task.last_activity = tick;
                break;
            }
        }
    }

    pub fn scheduler_tick(&mut self, tick: u64) {
        self.wake_due(tick);

        if let Some(index) = self.current_worker {
            if self.tasks[index].class == TaskClass::Worker
                && self.tasks[index].state == TaskState::Running
            {
                run_worker_slice(&mut self.tasks[index], tick);
                self.quantum_used = self.quantum_used.saturating_add(1);
                if self.quantum_used < self.quantum_ticks {
                    return;
                }
                self.tasks[index].state = TaskState::Ready;
                self.tasks[index].last_activity = tick;
                self.scheduler_cursor = index;
            }
            self.current_worker = None;
        }

        let Some(index) = self.next_ready_worker() else {
            self.quantum_used = 0;
            return;
        };
        let task = &mut self.tasks[index];
        task.state = TaskState::Running;
        task.context_switches += 1;
        task.last_activity = tick;
        self.current_worker = Some(index);
        self.scheduler_cursor = index;
        self.quantum_used = 0;
        self.total_switches += 1;
    }

    pub fn terminate(&mut self, id: u32, exit_code: i32, tick: u64) -> Result<(), TaskError> {
        let index = self.index_of(id).ok_or(TaskError::NotFound)?;
        if self.tasks[index].class == TaskClass::System {
            return Err(TaskError::Protected);
        }
        if self.tasks[index].class != TaskClass::Worker {
            return Err(TaskError::InvalidState);
        }
        if self.tasks[index].state.is_terminal() {
            return Err(TaskError::InvalidState);
        }
        self.tasks[index].state = TaskState::Exited;
        self.tasks[index].exit_code = exit_code;
        self.tasks[index].wake_at = 0;
        self.tasks[index].last_activity = tick;
        if self.current_worker == Some(index) {
            self.current_worker = None;
            self.quantum_used = 0;
        }
        Ok(())
    }

    pub fn sleep(&mut self, id: u32, duration: u64, tick: u64) -> Result<(), TaskError> {
        let index = self.index_of(id).ok_or(TaskError::NotFound)?;
        if self.tasks[index].class == TaskClass::System {
            return Err(TaskError::Protected);
        }
        if self.tasks[index].class != TaskClass::Worker {
            return Err(TaskError::InvalidState);
        }
        if self.tasks[index].state.is_terminal() || duration == 0 {
            return Err(TaskError::InvalidState);
        }
        self.tasks[index].state = TaskState::Sleeping;
        self.tasks[index].wake_at = tick.saturating_add(duration);
        self.tasks[index].last_activity = tick;
        if self.current_worker == Some(index) {
            self.current_worker = None;
            self.quantum_used = 0;
        }
        Ok(())
    }

    pub fn wake(&mut self, id: u32, tick: u64) -> Result<(), TaskError> {
        let task = self.get_mut(id).ok_or(TaskError::NotFound)?;
        if task.class == TaskClass::System {
            return Err(TaskError::Protected);
        }
        if task.class != TaskClass::Worker {
            return Err(TaskError::InvalidState);
        }
        if task.state != TaskState::Sleeping {
            return Err(TaskError::InvalidState);
        }
        task.state = TaskState::Ready;
        task.wake_at = 0;
        task.wake_count += 1;
        task.last_activity = tick;
        Ok(())
    }

    fn wake_due(&mut self, tick: u64) {
        for task in self.tasks.iter_mut().take(self.len) {
            if task.class == TaskClass::Worker
                && task.state == TaskState::Sleeping
                && task.wake_at != 0
                && tick >= task.wake_at
            {
                task.state = TaskState::Ready;
                task.wake_at = 0;
                task.wake_count += 1;
                task.last_activity = tick;
            }
        }
    }

    fn next_ready_worker(&self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        for offset in 1..=self.len {
            let index = (self.scheduler_cursor + offset) % self.len;
            let task = &self.tasks[index];
            if task.class == TaskClass::Worker && task.state == TaskState::Ready {
                return Some(index);
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn live_len(&self) -> usize {
        self.tasks
            .iter()
            .take(self.len)
            .filter(|task| task.is_live())
            .count()
    }

    pub fn worker_len(&self) -> usize {
        self.tasks
            .iter()
            .take(self.len)
            .filter(|task| task.class == TaskClass::Worker && task.is_live())
            .count()
    }

    pub fn current_worker_id(&self) -> Option<u32> {
        self.current_worker.map(|index| self.tasks[index].id)
    }

    pub const fn quantum_ticks(&self) -> u8 {
        self.quantum_ticks
    }

    pub const fn total_switches(&self) -> u64 {
        self.total_switches
    }

    pub fn task(&self, index: usize) -> Option<&TaskRecord> {
        self.tasks.get(index).filter(|_| index < self.len)
    }

    pub fn find(&self, id: u32) -> Option<&TaskRecord> {
        self.index_of(id).map(|index| &self.tasks[index])
    }

    pub fn runtime_pid(&self, id: u32) -> Option<u8> {
        let task = self.find(id)?;
        (task.class == TaskClass::User && task.runtime_pid != 0).then_some(task.runtime_pid)
    }

    pub fn format_row(&self, index: usize) -> Option<FixedText> {
        let task = self.task(index)?;
        let mut text = FixedText::empty();
        text.push_u64(task.id as u64);
        text.push_str(" ");
        text.push_str(task.name.as_str());
        text.push_str(" ");
        text.push_str(task.class.as_str());
        text.push_str(" ");
        text.push_str(task.state.as_str());
        text.push_str(" cpu=");
        text.push_u64(task.ticks);
        text.push_str(" mem=");
        text.push_u64(task.memory_kib as u64);
        text.push_str("K");
        Some(text)
    }

    fn index_of(&self, id: u32) -> Option<usize> {
        self.tasks
            .iter()
            .take(self.len)
            .position(|task| task.id == id)
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut TaskRecord> {
        self.index_of(id).map(|index| &mut self.tasks[index])
    }
}

fn run_worker_slice(task: &mut TaskRecord, tick: u64) {
    let mut value = task.checksum ^ tick.rotate_left((task.id % 31) + 1);
    for _ in 0..WORK_UNITS_PER_SLICE {
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
    }
    task.checksum = value;
    task.work_units = task.work_units.saturating_add(WORK_UNITS_PER_SLICE);
    task.ticks = task.ticks.saturating_add(1);
    task.last_activity = tick;
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_registry_tracks_system_ticks() {
        let mut registry = TaskRegistry::new();
        let shell = registry.register("shell", TaskState::Ready, 32);
        registry.mark_running(shell, 10);
        registry.mark_running(shell, 11);
        let row = registry.format_row(0).unwrap();
        assert!(row.as_str().contains("cpu=2"));
    }

    #[test]
    fn scheduler_rotates_workers_after_a_quantum() {
        let mut registry = TaskRegistry::new();
        let first = registry.spawn_worker("first", 16, 0).unwrap();
        let second = registry.spawn_worker("second", 16, 0).unwrap();

        registry.scheduler_tick(1);
        assert_eq!(registry.current_worker_id(), Some(second));
        for tick in 2..=6 {
            registry.scheduler_tick(tick);
        }
        assert_eq!(registry.current_worker_id(), Some(first));
        assert_eq!(registry.total_switches(), 2);
        assert_eq!(registry.find(second).unwrap().work_units, 320);
    }

    #[test]
    fn sleeping_worker_wakes_at_deadline() {
        let mut registry = TaskRegistry::new();
        let worker = registry.spawn_worker("sleeper", 16, 0).unwrap();
        registry.sleep(worker, 10, 5).unwrap();
        registry.scheduler_tick(14);
        assert_eq!(registry.find(worker).unwrap().state, TaskState::Sleeping);
        registry.scheduler_tick(15);
        assert_eq!(registry.find(worker).unwrap().state, TaskState::Running);
    }

    #[test]
    fn terminated_slots_are_reused_with_new_pids() {
        let mut registry = TaskRegistry::new();
        let old = registry.spawn_worker("old", 16, 0).unwrap();
        registry.terminate(old, 0, 1).unwrap();
        let new = registry.spawn_worker("new", 16, 2).unwrap();
        assert_ne!(old, new);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.find(new).unwrap().name.as_str(), "new");
    }

    #[test]
    fn system_tasks_are_protected_from_worker_controls() {
        let mut registry = TaskRegistry::new();
        let kernel = registry.register("desktop", TaskState::Ready, 32);
        assert_eq!(registry.terminate(kernel, 0, 1), Err(TaskError::Protected));
        assert_eq!(registry.sleep(kernel, 10, 1), Err(TaskError::Protected));
    }

    #[test]
    fn worker_names_are_bounded_for_system_surfaces() {
        let mut registry = TaskRegistry::new();
        assert_eq!(
            registry.spawn_worker("contains spaces", 16, 0),
            Err(TaskError::InvalidState)
        );
        assert_eq!(
            registry.spawn_worker("name-that-is-too-long", 16, 0),
            Err(TaskError::InvalidState)
        );
        assert!(registry.spawn_worker("render-1", 16, 0).is_ok());
    }

    #[test]
    fn completed_userspace_probe_keeps_exit_status() {
        let mut registry = TaskRegistry::new();
        registry.register("desktop", TaskState::Running, 32);
        let fault = registry.record_user_fault("user-crash", 142, 11).unwrap();
        let first = registry.record_user_exit("user-a", 7, 12).unwrap();
        let second = registry.record_user_exit("user-b", 0, 13).unwrap();
        let task = registry.find(first).unwrap();

        assert_eq!(registry.find(fault).unwrap().state, TaskState::Faulted);
        assert_eq!(registry.find(fault).unwrap().exit_code, 142);
        assert_eq!(task.class, TaskClass::User);
        assert_eq!(task.state, TaskState::Exited);
        assert_eq!(task.exit_code, 7);
        assert_eq!(registry.find(second).unwrap().name.as_str(), "user-b");
        assert_eq!(registry.len(), 4);
    }

    #[test]
    fn live_userspace_tasks_track_runtime_identity_and_exit() {
        let mut registry = TaskRegistry::new();
        let task = registry.spawn_user("init-elf", 9, 3).unwrap();
        assert_eq!(registry.runtime_pid(task), Some(9));
        assert_eq!(registry.find(task).unwrap().state, TaskState::Ready);

        registry
            .update_user(task, TaskState::Running, 0, 4)
            .unwrap();
        registry.update_user(task, TaskState::Exited, 7, 5).unwrap();
        let record = registry.find(task).unwrap();
        assert_eq!(record.exit_code, 7);
        assert_eq!(record.context_switches, 2);
    }

    #[test]
    fn worker_controls_cannot_desynchronize_userspace_tasks() {
        let mut registry = TaskRegistry::new();
        let task = registry.spawn_user("init-elf", 9, 3).unwrap();
        assert_eq!(registry.terminate(task, 0, 4), Err(TaskError::InvalidState));
        assert_eq!(registry.sleep(task, 10, 4), Err(TaskError::InvalidState));
        assert_eq!(registry.wake(task, 4), Err(TaskError::InvalidState));
        assert_eq!(registry.find(task).unwrap().state, TaskState::Ready);
    }
}
