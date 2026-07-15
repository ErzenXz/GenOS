use crate::display::FixedText;

pub const MAX_TASKS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping,
    Waiting,
}

impl TaskState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Sleeping => "sleeping",
            Self::Waiting => "waiting",
        }
    }
}

#[derive(Clone, Copy)]
pub struct TaskRecord {
    pub id: u32,
    pub name: FixedText,
    pub state: TaskState,
    pub ticks: u64,
    pub wake_count: u64,
    pub memory_kib: u32,
    pub last_activity: u64,
}

impl TaskRecord {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            name: FixedText::empty(),
            state: TaskState::Sleeping,
            ticks: 0,
            wake_count: 0,
            memory_kib: 0,
            last_activity: 0,
        }
    }
}

pub struct TaskRegistry {
    tasks: [TaskRecord; MAX_TASKS],
    len: usize,
    next_id: u32,
}

impl TaskRegistry {
    pub const fn new() -> Self {
        Self {
            tasks: [TaskRecord::empty(); MAX_TASKS],
            len: 0,
            next_id: 1,
        }
    }

    pub fn register(&mut self, name: &str, state: TaskState, memory_kib: u32) -> u32 {
        if self.len >= MAX_TASKS {
            return 0;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tasks[self.len] = TaskRecord {
            id,
            name: FixedText::from_str(name),
            state,
            ticks: 0,
            wake_count: 0,
            memory_kib,
            last_activity: 0,
        };
        self.len += 1;
        id
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

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn task(&self, index: usize) -> Option<&TaskRecord> {
        self.tasks.get(index).filter(|_| index < self.len)
    }

    pub fn format_row(&self, index: usize) -> Option<FixedText> {
        let task = self.task(index)?;
        let mut text = FixedText::empty();
        text.push_u64(task.id as u64);
        text.push_str(" ");
        text.push_str(task.name.as_str());
        text.push_str(" ");
        text.push_str(task.state.as_str());
        text.push_str(" ticks=");
        text.push_u64(task.ticks);
        text.push_str(" mem=");
        text.push_u64(task.memory_kib as u64);
        text.push_str("K");
        Some(text)
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut TaskRecord> {
        self.tasks
            .iter_mut()
            .take(self.len)
            .find(|task| task.id == id)
    }
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
    fn task_registry_tracks_ticks() {
        let mut registry = TaskRegistry::new();
        let shell = registry.register("shell", TaskState::Ready, 32);
        registry.mark_running(shell, 10);
        registry.mark_running(shell, 11);
        let row = registry.format_row(0).unwrap();
        assert!(row.as_str().contains("ticks=2"));
    }
}
