use genos_abi::BootInfo;

use crate::{
    input::MouseState,
    tasks::TaskRegistry,
    vfs::{NodeKind, RamVfs},
};

use super::{
    Color, FixedText, FramebufferDevice, LineKind, Point, Rect, ShellBuffer, ShellLine,
    TextRenderer, TextStyle,
};

const INPUT_CAP: usize = 128;
const DIRTY_CAP: usize = 16;
const STATS_REFRESH_TICKS: u64 = 25;
const FILE_VIEW_CAP: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowKind {
    Terminal,
    Files,
    TaskManager,
    About,
    Game,
    StartMenu,
}

#[derive(Clone, Copy)]
struct FileView {
    path: FixedText,
    kind: NodeKind,
    bytes: usize,
}

impl FileView {
    const fn empty() -> Self {
        Self {
            path: FixedText::empty(),
            kind: NodeKind::File,
            bytes: 0,
        }
    }
}

pub struct DisplayManager {
    fb: FramebufferDevice,
    shell: ShellBuffer,
    input: FixedText,
    status: FixedText,
    cursor: Point,
    previous_cursor: Point,
    cursor_dirty: bool,
    dirty: [Rect; DIRTY_CAP],
    dirty_len: usize,
    start_open: bool,
    terminal_open: bool,
    files_open: bool,
    task_manager_open: bool,
    about_open: bool,
    game_open: bool,
    focus: WindowKind,
    boot_abi: u32,
    memory_bytes: u64,
    initrd_files: usize,
    vfs_files: usize,
    event_depth: usize,
    uptime_ticks: u64,
    last_stats_refresh: u64,
    last_game_refresh: u64,
    game_frame: u64,
    clock: FixedText,
    files: [FileView; FILE_VIEW_CAP],
    files_len: usize,
    selected_file: usize,
    drag_window: Option<WindowKind>,
    drag_anchor: Point,
    terminal_offset: Point,
    files_offset: Point,
    task_manager_offset: Point,
    game_offset: Point,
    about_offset: Point,
}

impl DisplayManager {
    pub fn new(
        fb: FramebufferDevice,
        boot_info: &BootInfo,
        memory_bytes: u64,
        initrd_files: usize,
    ) -> Self {
        let mut manager = Self {
            fb,
            shell: ShellBuffer::new(),
            input: FixedText::empty(),
            status: FixedText::from_str("ready"),
            cursor: Point::new(0, 0),
            previous_cursor: Point::new(0, 0),
            cursor_dirty: true,
            dirty: [Rect::new(0, 0, 0, 0); DIRTY_CAP],
            dirty_len: 0,
            start_open: false,
            terminal_open: true,
            files_open: false,
            task_manager_open: false,
            about_open: false,
            game_open: false,
            focus: WindowKind::Terminal,
            boot_abi: boot_info.version,
            memory_bytes,
            initrd_files,
            vfs_files: initrd_files,
            event_depth: 0,
            uptime_ticks: 0,
            last_stats_refresh: 0,
            last_game_refresh: 0,
            game_frame: 0,
            clock: FixedText::from_str("--:--:--"),
            files: [FileView::empty(); FILE_VIEW_CAP],
            files_len: 0,
            selected_file: 0,
            drag_window: None,
            drag_anchor: Point::new(0, 0),
            terminal_offset: Point::new(0, 0),
            files_offset: Point::new(0, 0),
            task_manager_offset: Point::new(0, 0),
            game_offset: Point::new(0, 0),
            about_offset: Point::new(0, 0),
        };
        manager.push_line(LineKind::Status, "Display manager online");
        manager.push_line(LineKind::Output, "Type help to explore GenOS.");
        manager
    }

    pub fn push_line(&mut self, kind: LineKind, text: &str) {
        self.shell.push(ShellLine::new(kind, text));
        self.invalidate(self.terminal_window_rect());
    }

    pub fn push_fixed(&mut self, kind: LineKind, text: FixedText) {
        self.shell.push(ShellLine { kind, text });
        self.invalidate(self.terminal_window_rect());
    }

    pub fn clear_shell(&mut self) {
        self.shell.clear();
        self.invalidate(self.terminal_window_rect());
    }

    pub fn input_push(&mut self, byte: u8) -> bool {
        if self.input.len() >= INPUT_CAP || !(0x20..=0x7e).contains(&byte) {
            return false;
        }
        let mut buf = [0u8; 1];
        buf[0] = byte;
        let text = core::str::from_utf8(&buf).unwrap_or("");
        self.input.push_str(text);
        self.invalidate(self.input_rect());
        true
    }

    pub fn input_backspace(&mut self) -> bool {
        let text = self.input.as_str();
        if text.is_empty() {
            return false;
        }
        let mut next = FixedText::empty();
        let keep = text.len().saturating_sub(1);
        next.push_str(&text[..keep]);
        self.input = next;
        self.invalidate(self.input_rect());
        true
    }

    pub fn set_input(&mut self, text: FixedText) {
        self.input = text;
        self.invalidate(self.input_rect());
    }

    pub fn shell_input_active(&self) -> bool {
        self.terminal_open && self.focus == WindowKind::Terminal && !self.start_open
    }

    pub fn take_input(&mut self) -> FixedText {
        let current = self.input;
        self.input = FixedText::empty();
        self.invalidate(self.input_rect());
        current
    }

    pub fn set_status(&mut self, text: &str) {
        self.status = FixedText::from_str(text);
        self.invalidate(self.taskbar_rect());
    }

    pub fn set_clock(&mut self, text: FixedText) {
        if self.clock.as_str() == text.as_str() {
            return;
        }
        self.clock = text;
        self.invalidate(self.taskbar_rect());
    }

    pub fn sync_stats(
        &mut self,
        mouse: MouseState,
        event_depth: usize,
        vfs_files: usize,
        uptime_ticks: u64,
    ) {
        self.event_depth = event_depth;
        self.vfs_files = vfs_files;
        self.uptime_ticks = uptime_ticks;
        if mouse.position != self.cursor {
            self.previous_cursor = self.cursor;
            self.cursor = mouse.position;
            self.cursor_dirty = true;
        }
    }

    pub fn sync_vfs(&mut self, vfs: &RamVfs) {
        let previous_len = self.files_len;
        self.files_len = 0;
        for node in vfs.list_root().take(FILE_VIEW_CAP) {
            self.files[self.files_len] = FileView {
                path: FixedText::from_str(node.path()),
                kind: node.kind(),
                bytes: node.len(),
            };
            self.files_len += 1;
        }
        if self.files_len == 0 {
            self.selected_file = 0;
        } else {
            self.selected_file = self.selected_file.min(self.files_len - 1);
        }
        if self.files_open || previous_len != self.files_len {
            self.invalidate(self.files_window_rect());
            self.invalidate(self.taskbar_rect());
        }
    }

    pub fn refresh_stats_if_due(&mut self, tick: u64) {
        if tick.saturating_sub(self.last_stats_refresh) < STATS_REFRESH_TICKS {
            return;
        }
        self.last_stats_refresh = tick;
        self.invalidate(self.system_widget_rect());
        self.invalidate(self.taskbar_rect());
        if self.task_manager_open {
            self.invalidate(self.task_manager_rect());
        }
    }

    pub fn animate_if_due(&mut self, tick: u64) {
        if !self.game_open || tick == self.last_game_refresh {
            return;
        }
        self.last_game_refresh = tick;
        self.game_frame = self.game_frame.wrapping_add(1);
        self.invalidate(self.game_window_rect());
    }

    pub fn open_task_manager(&mut self) {
        self.task_manager_open = true;
        self.focus = WindowKind::TaskManager;
        self.invalidate(self.screen());
    }

    pub fn open_terminal(&mut self) {
        self.terminal_open = true;
        self.focus = WindowKind::Terminal;
        self.invalidate(self.screen());
    }

    pub fn open_files(&mut self) {
        self.files_open = true;
        self.focus = WindowKind::Files;
        self.invalidate(self.screen());
    }

    pub fn open_about(&mut self) {
        self.about_open = true;
        self.focus = WindowKind::About;
        self.invalidate(self.screen());
    }

    pub fn open_game(&mut self) {
        self.game_open = true;
        self.focus = WindowKind::Game;
        self.invalidate(self.screen());
    }

    pub fn refresh_task_manager(&mut self) {
        if self.task_manager_open {
            self.invalidate(self.task_manager_rect());
        }
    }

    pub fn handle_mouse_down(&mut self, point: Point) {
        if self.start_button_rect().contains(point) {
            let panel = self.start_panel_rect();
            self.start_open = !self.start_open;
            self.drag_window = None;
            self.invalidate(panel);
            self.invalidate(self.taskbar_rect());
            return;
        }

        if let Some(kind) = self.taskbar_button_at(point) {
            self.open_window(kind);
            return;
        }

        if self.start_open && self.start_panel_rect().contains(point) {
            let panel = self.start_panel_rect();
            let row = (point.y - (panel.y + 72)) / 46;
            match row {
                0 => self.open_terminal(),
                1 => self.open_files(),
                2 => self.open_task_manager(),
                3 => self.open_game(),
                4 => self.open_about(),
                _ => {}
            }
            self.start_open = false;
            self.invalidate(panel);
            return;
        }

        if self.window_is_open(self.focus) && self.window_rect(self.focus).contains(point) {
            self.activate_window_at(self.focus, point);
            return;
        }

        let windows = [
            WindowKind::About,
            WindowKind::Game,
            WindowKind::TaskManager,
            WindowKind::Files,
            WindowKind::Terminal,
        ];
        for kind in windows {
            if kind != self.focus
                && self.window_is_open(kind)
                && self.window_rect(kind).contains(point)
            {
                self.activate_window_at(kind, point);
                return;
            }
        }

        if let Some(kind) = self.desktop_icon_at(point) {
            self.open_window(kind);
            return;
        }

        if self.start_open {
            let panel = self.start_panel_rect();
            self.start_open = false;
            self.invalidate(panel);
        }
        self.drag_window = None;
    }

    pub fn handle_mouse_move(&mut self, point: Point, left_down: bool) {
        if !left_down {
            self.drag_window = None;
            return;
        }
        let Some(kind) = self.drag_window else {
            if self.window_is_open(self.focus) && self.titlebar_rect(self.focus).contains(point) {
                self.drag_window = Some(self.focus);
                self.drag_anchor = point;
            }
            return;
        };
        let dx = point.x - self.drag_anchor.x;
        let dy = point.y - self.drag_anchor.y;
        if dx == 0 && dy == 0 {
            return;
        }
        let old = self.window_rect(kind);
        self.move_window_by(kind, dx, dy);
        self.drag_anchor = point;
        self.invalidate(old);
        self.invalidate(self.window_rect(kind));
    }

    pub fn end_drag(&mut self) {
        self.drag_window = None;
    }

    pub fn dismiss_focused(&mut self) {
        if self.start_open {
            let panel = self.start_panel_rect();
            self.start_open = false;
            self.invalidate(panel);
            return;
        }
        if self.window_is_open(self.focus) {
            self.close_window(self.focus);
        }
    }

    pub fn cycle_focus(&mut self) {
        const WINDOWS: [WindowKind; 5] = [
            WindowKind::Terminal,
            WindowKind::Files,
            WindowKind::TaskManager,
            WindowKind::Game,
            WindowKind::About,
        ];
        let current = WINDOWS
            .iter()
            .position(|kind| *kind == self.focus)
            .unwrap_or(0);
        for step in 1..=WINDOWS.len() {
            let candidate = WINDOWS[(current + step) % WINDOWS.len()];
            if self.window_is_open(candidate) {
                self.focus = candidate;
                self.invalidate(self.screen());
                return;
            }
        }
    }

    fn activate_window_at(&mut self, kind: WindowKind, point: Point) {
        if self.close_button_rect(kind).contains(point) {
            self.close_window(kind);
            return;
        }
        self.focus = kind;
        if kind == WindowKind::Files {
            let body = self.files_body_rect();
            if body.contains(point) && point.y >= body.y + 70 {
                let index = ((point.y - (body.y + 70)) / 34) as usize;
                if index < self.files_len {
                    self.selected_file = index;
                    self.invalidate(self.files_window_rect());
                }
            }
        }
        if self.titlebar_rect(kind).contains(point) {
            self.drag_window = Some(kind);
            self.drag_anchor = point;
        }
        self.invalidate(self.screen());
    }

    fn open_window(&mut self, kind: WindowKind) {
        match kind {
            WindowKind::Terminal => self.open_terminal(),
            WindowKind::Files => self.open_files(),
            WindowKind::TaskManager => self.open_task_manager(),
            WindowKind::Game => self.open_game(),
            WindowKind::About => self.open_about(),
            WindowKind::StartMenu => {}
        }
        self.start_open = false;
        self.invalidate(self.screen());
    }

    fn close_window(&mut self, kind: WindowKind) {
        let old = self.window_rect(kind);
        match kind {
            WindowKind::Terminal => self.terminal_open = false,
            WindowKind::Files => self.files_open = false,
            WindowKind::TaskManager => self.task_manager_open = false,
            WindowKind::Game => self.game_open = false,
            WindowKind::About => self.about_open = false,
            WindowKind::StartMenu => self.start_open = false,
        }
        self.drag_window = None;
        self.focus = [
            WindowKind::Terminal,
            WindowKind::Files,
            WindowKind::TaskManager,
            WindowKind::Game,
            WindowKind::About,
        ]
        .into_iter()
        .find(|candidate| self.window_is_open(*candidate))
        .unwrap_or(WindowKind::Terminal);
        self.invalidate(old);
        self.invalidate(self.screen());
    }

    pub fn invalidate(&mut self, rect: Rect) {
        let rect = rect.intersect(self.fb.bounds());
        if rect.is_empty() {
            return;
        }
        for index in 0..self.dirty_len {
            if !self.dirty[index].intersect(rect).is_empty() {
                self.dirty[index] = self.dirty[index].union(rect);
                return;
            }
        }
        if self.dirty_len < DIRTY_CAP {
            self.dirty[self.dirty_len] = rect;
            self.dirty_len += 1;
        } else {
            self.dirty[0] = self.dirty[0].union(rect);
            self.dirty_len = 1;
        }
    }

    pub fn flush(&mut self, tasks: &TaskRegistry) {
        if self.dirty_len == 0 && !self.cursor_dirty {
            return;
        }
        let count = self.dirty_len;
        let dirty = self.dirty;
        let old_cursor = self.cursor_rect(self.previous_cursor);
        let cursor_dirty = self.cursor_dirty;
        self.dirty_len = 0;
        self.cursor_dirty = false;
        if cursor_dirty {
            self.fb.present_rect(old_cursor);
        }
        for rect in dirty.iter().take(count) {
            self.redraw_region(*rect, tasks);
        }
        for rect in dirty.iter().take(count) {
            self.fb.present_rect(*rect);
        }
        self.draw_cursor_overlay();
    }

    pub fn redraw(&mut self) {
        self.redraw_all(None);
    }

    pub fn redraw_with_tasks(&mut self, tasks: &TaskRegistry) {
        self.redraw_all(Some(tasks));
    }

    fn redraw_all(&mut self, tasks: Option<&TaskRegistry>) {
        self.fb.desktop_wallpaper();
        let screen = self.fb.bounds();
        if self.cursor == Point::new(0, 0) {
            self.cursor = Point::new(screen.width / 2, screen.height / 2);
        }

        self.draw_desktop_icons();
        self.draw_open_windows(tasks);
        if self.start_open {
            self.draw_start_panel(self.start_panel_rect());
        }
        self.draw_system_widget(self.system_widget_rect());
        self.draw_taskbar(self.taskbar_rect());
        self.fb.present_all();
        self.draw_cursor_overlay();
        self.dirty_len = 0;
        self.cursor_dirty = false;
    }

    fn redraw_region(&mut self, rect: Rect, tasks: &TaskRegistry) {
        if self.terminal_open
            && self.focus == WindowKind::Terminal
            && !self.input_rect().intersect(rect).is_empty()
            && rect.intersect(self.input_rect()) == rect
        {
            self.draw_shell_input(self.shell_body_rect());
            return;
        }
        self.fb.desktop_wallpaper_rect(rect);
        self.draw_desktop_icons_if_needed(rect);
        self.draw_windows_intersecting(rect, Some(tasks));
        if self.start_open && !self.start_panel_rect().intersect(rect).is_empty() {
            self.draw_start_panel(self.start_panel_rect());
        }
        if !self.system_widget_rect().intersect(rect).is_empty() {
            self.draw_system_widget(self.system_widget_rect());
        }
        if !self.taskbar_rect().intersect(rect).is_empty() {
            self.draw_taskbar(self.taskbar_rect());
        }
    }

    fn draw_open_windows(&mut self, tasks: Option<&TaskRegistry>) {
        const WINDOWS: [WindowKind; 5] = [
            WindowKind::Terminal,
            WindowKind::Files,
            WindowKind::TaskManager,
            WindowKind::Game,
            WindowKind::About,
        ];
        for kind in WINDOWS {
            if kind != self.focus && self.window_is_open(kind) {
                self.draw_window(kind, tasks);
            }
        }
        if self.window_is_open(self.focus) {
            self.draw_window(self.focus, tasks);
        }
    }

    fn draw_windows_intersecting(&mut self, clip: Rect, tasks: Option<&TaskRegistry>) {
        const WINDOWS: [WindowKind; 5] = [
            WindowKind::Terminal,
            WindowKind::Files,
            WindowKind::TaskManager,
            WindowKind::Game,
            WindowKind::About,
        ];
        for kind in WINDOWS {
            if kind != self.focus
                && self.window_is_open(kind)
                && !self.window_rect(kind).intersect(clip).is_empty()
            {
                self.draw_window(kind, tasks);
            }
        }
        if self.window_is_open(self.focus)
            && !self.window_rect(self.focus).intersect(clip).is_empty()
        {
            self.draw_window(self.focus, tasks);
        }
    }

    fn draw_window(&mut self, kind: WindowKind, tasks: Option<&TaskRegistry>) {
        match kind {
            WindowKind::Terminal => self.draw_terminal_window(self.terminal_window_rect()),
            WindowKind::Files => self.draw_files_window(self.files_window_rect()),
            WindowKind::TaskManager => {
                self.draw_task_manager_window(self.task_manager_rect(), tasks)
            }
            WindowKind::Game => self.draw_game_window(self.game_window_rect()),
            WindowKind::About => self.draw_about_window(self.about_window_rect()),
            WindowKind::StartMenu => {}
        }
    }

    fn screen(&self) -> Rect {
        self.fb.bounds()
    }

    fn taskbar_rect(&self) -> Rect {
        let screen = self.screen();
        Rect::new(0, screen.bottom() - 52, screen.width, 52)
    }

    fn start_button_rect(&self) -> Rect {
        let taskbar = self.taskbar_rect();
        Rect::new(12, taskbar.y + 8, 102, 36)
    }

    fn taskbar_terminal_rect(&self) -> Rect {
        let taskbar = self.taskbar_rect();
        Rect::new(128, taskbar.y + 8, 94, 36)
    }

    fn taskbar_files_rect(&self) -> Rect {
        let taskbar = self.taskbar_rect();
        Rect::new(230, taskbar.y + 8, 94, 36)
    }

    fn taskbar_tasks_rect(&self) -> Rect {
        let taskbar = self.taskbar_rect();
        Rect::new(332, taskbar.y + 8, 94, 36)
    }

    fn taskbar_game_rect(&self) -> Rect {
        let taskbar = self.taskbar_rect();
        Rect::new(434, taskbar.y + 8, 94, 36)
    }

    fn taskbar_about_rect(&self) -> Rect {
        let taskbar = self.taskbar_rect();
        Rect::new(536, taskbar.y + 8, 94, 36)
    }

    fn taskbar_button_at(&self, point: Point) -> Option<WindowKind> {
        [
            (self.taskbar_terminal_rect(), WindowKind::Terminal),
            (self.taskbar_files_rect(), WindowKind::Files),
            (self.taskbar_tasks_rect(), WindowKind::TaskManager),
            (self.taskbar_game_rect(), WindowKind::Game),
            (self.taskbar_about_rect(), WindowKind::About),
        ]
        .into_iter()
        .find_map(|(rect, kind)| rect.contains(point).then_some(kind))
    }

    fn start_panel_rect(&self) -> Rect {
        let screen = self.screen();
        Rect::new(12, screen.bottom() - 386, 314, 326)
    }

    fn system_widget_rect(&self) -> Rect {
        let screen = self.screen();
        Rect::new(screen.right() - 312, 18, 292, 42)
    }

    fn terminal_window_rect(&self) -> Rect {
        let screen = self.screen();
        let width = if self.task_manager_open {
            screen.width - 586
        } else {
            screen.width - 224
        };
        Self::offset_rect(
            Rect::new(152, 72, width.max(470), screen.height - 150),
            self.terminal_offset,
        )
    }

    fn task_manager_rect(&self) -> Rect {
        let screen = self.screen();
        Self::offset_rect(
            Rect::new(screen.right() - 402, 72, 372, screen.height - 150),
            self.task_manager_offset,
        )
    }

    fn files_window_rect(&self) -> Rect {
        Self::offset_rect(Rect::new(206, 112, 530, 394), self.files_offset)
    }

    fn game_window_rect(&self) -> Rect {
        let screen = self.screen();
        let y = (screen.bottom() - 330).clamp(130, 460);
        Self::offset_rect(Rect::new(246, y, 560, 270), self.game_offset)
    }

    fn about_window_rect(&self) -> Rect {
        let screen = self.screen();
        Self::offset_rect(
            Rect::new(screen.right() - 506, screen.bottom() - 350, 456, 240),
            self.about_offset,
        )
    }

    const fn offset_rect(rect: Rect, offset: Point) -> Rect {
        Rect::new(
            rect.x + offset.x,
            rect.y + offset.y,
            rect.width,
            rect.height,
        )
    }

    fn window_rect(&self, kind: WindowKind) -> Rect {
        match kind {
            WindowKind::Terminal => self.terminal_window_rect(),
            WindowKind::Files => self.files_window_rect(),
            WindowKind::TaskManager => self.task_manager_rect(),
            WindowKind::Game => self.game_window_rect(),
            WindowKind::About => self.about_window_rect(),
            WindowKind::StartMenu => self.start_panel_rect(),
        }
    }

    fn window_is_open(&self, kind: WindowKind) -> bool {
        match kind {
            WindowKind::Terminal => self.terminal_open,
            WindowKind::Files => self.files_open,
            WindowKind::TaskManager => self.task_manager_open,
            WindowKind::Game => self.game_open,
            WindowKind::About => self.about_open,
            WindowKind::StartMenu => self.start_open,
        }
    }

    fn titlebar_rect(&self, kind: WindowKind) -> Rect {
        let rect = self.window_rect(kind);
        Rect::new(rect.x, rect.y, rect.width, 38)
    }

    fn close_button_rect(&self, kind: WindowKind) -> Rect {
        let rect = self.window_rect(kind);
        Rect::new(rect.right() - 34, rect.y + 8, 22, 22)
    }

    fn files_body_rect(&self) -> Rect {
        let rect = self.files_window_rect();
        Rect::new(rect.x + 8, rect.y + 45, rect.width - 16, rect.height - 53)
    }

    fn move_window_by(&mut self, kind: WindowKind, dx: i32, dy: i32) {
        let apply = |offset: &mut Point| {
            offset.x += dx;
            offset.y += dy;
        };
        match kind {
            WindowKind::Terminal => apply(&mut self.terminal_offset),
            WindowKind::Files => apply(&mut self.files_offset),
            WindowKind::TaskManager => apply(&mut self.task_manager_offset),
            WindowKind::Game => apply(&mut self.game_offset),
            WindowKind::About => apply(&mut self.about_offset),
            WindowKind::StartMenu => return,
        }

        let rect = self.window_rect(kind);
        let screen = self.screen();
        let correction = Point::new(
            if rect.x < 8 {
                8 - rect.x
            } else if rect.right() > screen.right() - 8 {
                screen.right() - 8 - rect.right()
            } else {
                0
            },
            if rect.y < 8 {
                8 - rect.y
            } else if rect.y > self.taskbar_rect().y - 38 {
                self.taskbar_rect().y - 38 - rect.y
            } else {
                0
            },
        );
        let correct = |offset: &mut Point| {
            offset.x += correction.x;
            offset.y += correction.y;
        };
        match kind {
            WindowKind::Terminal => correct(&mut self.terminal_offset),
            WindowKind::Files => correct(&mut self.files_offset),
            WindowKind::TaskManager => correct(&mut self.task_manager_offset),
            WindowKind::Game => correct(&mut self.game_offset),
            WindowKind::About => correct(&mut self.about_offset),
            WindowKind::StartMenu => {}
        }
    }

    fn input_rect(&self) -> Rect {
        let window = self.terminal_window_rect();
        Rect::new(window.x + 20, window.bottom() - 62, window.width - 40, 42)
    }

    fn shell_body_rect(&self) -> Rect {
        let rect = self.terminal_window_rect();
        Rect::new(rect.x + 8, rect.y + 45, rect.width - 16, rect.height - 53)
    }

    fn cursor_rect(&self, point: Point) -> Rect {
        Rect::new(point.x - 4, point.y - 4, 24, 26)
    }

    fn desktop_icon_at(&self, point: Point) -> Option<WindowKind> {
        let icons = [
            (24, 28, WindowKind::Terminal),
            (24, 112, WindowKind::Files),
            (24, 196, WindowKind::TaskManager),
            (24, 280, WindowKind::Game),
            (24, 364, WindowKind::About),
        ];
        for (x, y, kind) in icons {
            if Rect::new(x - 4, y, 100, 72).contains(point) {
                return Some(kind);
            }
        }
        None
    }

    fn draw_desktop_icons(&mut self) {
        let icons = [
            (24, 28, "TERMINAL", Color::ACCENT),
            (24, 112, "FILES", Color::TEXT_INVERTED),
            (24, 196, "TASKS", Color::SUCCESS),
            (24, 280, "GRAPHICS", Color::WARNING),
            (24, 364, "ABOUT", Color::TEXT_MUTED),
        ];
        for (x, y, label, color) in icons {
            self.draw_desktop_icon(Point::new(x, y), label, color);
        }
    }

    fn draw_desktop_icons_if_needed(&mut self, clip: Rect) {
        let icons = [
            (24, 28, "TERMINAL", Color::ACCENT),
            (24, 112, "FILES", Color::TEXT_INVERTED),
            (24, 196, "TASKS", Color::SUCCESS),
            (24, 280, "GRAPHICS", Color::WARNING),
            (24, 364, "ABOUT", Color::TEXT_MUTED),
        ];
        for (x, y, label, color) in icons {
            let bounds = Rect::new(x - 4, y, 100, 72);
            if !bounds.intersect(clip).is_empty() {
                self.draw_desktop_icon(Point::new(x, y), label, color);
            }
        }
    }

    fn draw_desktop_icon(&mut self, origin: Point, label: &str, color: Color) {
        let icon = Rect::new(origin.x + 18, origin.y, 36, 36);
        self.fb.fill_rect(icon, Color::rgb(24, 25, 24));
        self.fb.stroke_rect(icon, Color::rgb(91, 92, 87));
        self.fb.fill_rect(icon.inset(8), color);
        TextRenderer::draw_text(
            &mut self.fb,
            Rect::new(origin.x - 4, origin.y, 100, 72),
            Point::new(origin.x, origin.y + 48),
            label,
            TextStyle::regular(11, Color::TEXT_INVERTED),
        );
    }

    fn draw_system_widget(&mut self, rect: Rect) {
        self.fb.fill_rect(rect, Color::rgb(28, 29, 28));
        self.fb.stroke_rect(rect, Color::rgb(78, 79, 75));
        let mut mem = FixedText::from_str("RAM ");
        mem.push_u64(self.memory_bytes / 1024 / 1024);
        mem.push_str(" MB");
        TextRenderer::draw_text(
            &mut self.fb,
            rect,
            Point::new(rect.x + 14, rect.y + 8),
            "GENOS",
            TextStyle::bold(14, Color::TEXT_INVERTED),
        );
        TextRenderer::draw_text(
            &mut self.fb,
            rect,
            Point::new(rect.x + 152, rect.y + 10),
            mem.as_str(),
            TextStyle::regular(12, Color::TEXT_MUTED),
        );
        let mut boot = FixedText::from_str("ABI ");
        boot.push_u64(self.boot_abi as u64);
        boot.push_str(" /");
        boot.push_u64(self.initrd_files as u64);
        TextRenderer::draw_text(
            &mut self.fb,
            rect,
            Point::new(rect.x + 92, rect.y + 10),
            boot.as_str(),
            TextStyle::regular(12, Color::TEXT_MUTED),
        );
    }

    fn draw_start_panel(&mut self, rect: Rect) {
        self.fb.fill_rect(rect, Color::WINDOW_DARK);
        self.fb.stroke_rect(rect, Color::rgb(83, 84, 80));
        let header = Rect::new(rect.x, rect.y, rect.width, 58);
        self.fb.fill_rect(header, Color::rgb(34, 35, 34));
        self.fb
            .fill_rect(Rect::new(rect.x, rect.y, 4, 58), Color::ACCENT);
        TextRenderer::draw_text(
            &mut self.fb,
            header,
            Point::new(rect.x + 20, rect.y + 17),
            "GENOS",
            TextStyle::bold(18, Color::TEXT_INVERTED),
        );

        let rows = [
            ("Terminal", "Shell"),
            ("Files", "RAM VFS"),
            ("TaskMgr", "Live tasks"),
            ("Game", "Frame demo"),
            ("About", "System"),
        ];
        let mut y = rect.y + 72;
        for (title, subtitle) in rows {
            let row = Rect::new(rect.x + 10, y, rect.width - 20, 40);
            self.fb.fill_rect(row, Color::rgb(28, 29, 28));
            self.fb
                .fill_rect(Rect::new(row.x, row.y, 2, row.height), Color::BORDER);
            TextRenderer::draw_text(
                &mut self.fb,
                rect,
                Point::new(rect.x + 22, y + 10),
                title,
                TextStyle::bold(13, Color::TEXT_INVERTED),
            );
            TextRenderer::draw_text(
                &mut self.fb,
                rect,
                Point::new(rect.x + 154, y + 11),
                subtitle,
                TextStyle::regular(11, Color::TEXT_MUTED),
            );
            y += 46;
        }
        TextRenderer::draw_text(
            &mut self.fb,
            rect,
            Point::new(rect.x + 16, rect.bottom() - 24),
            "ESC CLOSE   TAB SWITCH",
            TextStyle::regular(12, Color::TEXT_MUTED),
        );
    }

    fn draw_terminal_window(&mut self, rect: Rect) {
        self.draw_window_frame(rect, "Terminal - GenOS Kernel Shell");
        self.draw_shell(self.shell_body_rect());
    }

    fn draw_files_window(&mut self, rect: Rect) {
        self.draw_window_frame(rect, "Files / session storage");
        let body = self.files_body_rect();
        self.fb.fill_rect(body, Color::rgb(232, 230, 222));
        self.fb.stroke_rect(body, Color::rgb(102, 102, 97));
        let sidebar = Rect::new(body.x, body.y, 122, body.height);
        self.fb.fill_rect(sidebar, Color::rgb(205, 202, 193));
        self.fb.fill_rect(
            Rect::new(sidebar.right(), body.y, 1, body.height),
            Color::BORDER,
        );
        TextRenderer::draw_text(
            &mut self.fb,
            sidebar,
            Point::new(sidebar.x + 14, sidebar.y + 16),
            "PLACES",
            TextStyle::bold(12, Color::TEXT),
        );
        let places = ["HOME", "SESSION", "SYSTEM"];
        let mut place_y = sidebar.y + 52;
        for (index, place) in places.into_iter().enumerate() {
            if index == 1 {
                self.fb.fill_rect(
                    Rect::new(sidebar.x + 6, place_y - 8, sidebar.width - 12, 30),
                    Color::rgb(184, 180, 169),
                );
            }
            TextRenderer::draw_text(
                &mut self.fb,
                sidebar,
                Point::new(sidebar.x + 16, place_y),
                place,
                TextStyle::regular(12, Color::TEXT),
            );
            place_y += 38;
        }

        let list = Rect::new(
            sidebar.right() + 1,
            body.y,
            body.width - sidebar.width - 1,
            body.height,
        );
        let mut heading = FixedText::from_str("SESSION  /  ");
        heading.push_u64(self.files_len as u64);
        heading.push_str(" ITEMS");
        TextRenderer::draw_text(
            &mut self.fb,
            list,
            Point::new(list.x + 16, list.y + 15),
            heading.as_str(),
            TextStyle::bold(12, Color::TEXT),
        );
        self.fb
            .fill_rect(Rect::new(list.x, list.y + 42, list.width, 1), Color::BORDER);
        TextRenderer::draw_text(
            &mut self.fb,
            list,
            Point::new(list.x + 16, list.y + 51),
            "NAME",
            TextStyle::bold(11, Color::TEXT_MUTED),
        );
        TextRenderer::draw_text(
            &mut self.fb,
            list,
            Point::new(list.x + 226, list.y + 51),
            "TYPE      SIZE",
            TextStyle::bold(11, Color::TEXT_MUTED),
        );

        let mut y = list.y + 78;
        if self.files_len == 0 {
            TextRenderer::draw_text(
                &mut self.fb,
                list,
                Point::new(list.x + 16, y),
                "NO FILES IN THIS SESSION",
                TextStyle::regular(12, Color::TEXT_MUTED),
            );
        }
        for index in 0..self.files_len {
            if y + 30 >= list.bottom() - 42 {
                break;
            }
            let file = self.files[index];
            let row = Rect::new(list.x + 6, y - 8, list.width - 12, 31);
            if index == self.selected_file {
                self.fb.fill_rect(row, Color::rgb(207, 194, 165));
                self.fb
                    .fill_rect(Rect::new(row.x, row.y, 3, row.height), Color::ACCENT_DARK);
            }
            TextRenderer::draw_text(
                &mut self.fb,
                row,
                Point::new(row.x + 10, y),
                file.path.as_str(),
                TextStyle::regular(12, Color::TEXT),
            );
            TextRenderer::draw_text(
                &mut self.fb,
                row,
                Point::new(row.x + 220, y),
                match file.kind {
                    NodeKind::File => "FILE",
                    NodeKind::Directory => "DIR",
                },
                TextStyle::regular(11, Color::TEXT_MUTED),
            );
            let mut bytes = FixedText::empty();
            bytes.push_u64(file.bytes as u64);
            bytes.push_str(" B");
            TextRenderer::draw_text(
                &mut self.fb,
                row,
                Point::new(row.x + 292, y),
                bytes.as_str(),
                TextStyle::regular(11, Color::TEXT_MUTED),
            );
            y += 34;
        }
        self.fb.fill_rect(
            Rect::new(list.x, list.bottom() - 36, list.width, 36),
            Color::rgb(217, 214, 205),
        );
        if self.files_len > 0 {
            let file = self.files[self.selected_file];
            let mut detail = FixedText::from_str("SELECTED ");
            detail.push_str(file.path.as_str());
            TextRenderer::draw_text(
                &mut self.fb,
                list,
                Point::new(list.x + 16, list.bottom() - 25),
                detail.as_str(),
                TextStyle::regular(11, Color::TEXT_MUTED),
            );
        }
    }

    fn draw_task_manager_window(&mut self, rect: Rect, tasks: Option<&TaskRegistry>) {
        self.draw_window_frame(rect, "Task Manager / scheduler");
        let body = Rect::new(rect.x + 8, rect.y + 45, rect.width - 16, rect.height - 53);
        self.fb.fill_rect(body, Color::rgb(232, 230, 222));
        self.fb.stroke_rect(body, Color::rgb(102, 102, 97));
        let clip = body.inset(12);
        let heading = TextStyle::bold(13, Color::TEXT);
        let text = TextStyle::regular(12, Color::TEXT_MUTED);
        let accent = TextStyle::regular(12, Color::ACCENT_DARK);
        let mut y = clip.y;
        TextRenderer::draw_text(
            &mut self.fb,
            clip,
            Point::new(clip.x, y),
            "SYSTEM HEALTH",
            heading,
        );
        y += 28;
        let mut stats = FixedText::from_str("UP ");
        stats.push_u64(self.uptime_ticks);
        stats.push_str(" TICKS    QUEUE ");
        stats.push_u64(self.event_depth as u64);
        TextRenderer::draw_text(
            &mut self.fb,
            clip,
            Point::new(clip.x, y),
            stats.as_str(),
            text,
        );
        y += 22;
        let mut stats2 = FixedText::from_str("FILES ");
        stats2.push_u64(self.vfs_files as u64);
        stats2.push_str("    POINTER ");
        stats2.push_u64(self.cursor.x as u64);
        stats2.push_str(",");
        stats2.push_u64(self.cursor.y as u64);
        TextRenderer::draw_text(
            &mut self.fb,
            clip,
            Point::new(clip.x, y),
            stats2.as_str(),
            text,
        );
        y += 22;
        if let Some(tasks) = tasks {
            let mut scheduler = FixedText::from_str("WORKERS ");
            scheduler.push_u64(tasks.worker_len() as u64);
            scheduler.push_str("  RUN ");
            match tasks.current_worker_id() {
                Some(pid) => scheduler.push_u64(pid as u64),
                None => scheduler.push_str("--"),
            }
            scheduler.push_str("  SW ");
            scheduler.push_u64(tasks.total_switches());
            TextRenderer::draw_text(
                &mut self.fb,
                clip,
                Point::new(clip.x, y),
                scheduler.as_str(),
                text,
            );
        }
        y += 28;
        self.fb
            .fill_rect(Rect::new(clip.x, y, clip.width, 1), Color::BORDER);
        y += 18;
        TextRenderer::draw_text(
            &mut self.fb,
            clip,
            Point::new(clip.x, y),
            "NAME       TYPE      STATE    MEM",
            heading,
        );
        y += 26;
        if let Some(tasks) = tasks {
            let mut index = 0;
            while index < tasks.len() && y < clip.bottom() - 22 {
                if let Some(task) = tasks.task(index) {
                    if index % 2 == 1 {
                        self.fb.fill_rect(
                            Rect::new(clip.x - 4, y - 7, clip.width + 8, 27),
                            Color::rgb(222, 220, 212),
                        );
                    }
                    TextRenderer::draw_text(
                        &mut self.fb,
                        clip,
                        Point::new(clip.x, y),
                        task.name.as_str(),
                        TextStyle::bold(12, Color::TEXT),
                    );
                    TextRenderer::draw_text(
                        &mut self.fb,
                        clip,
                        Point::new(clip.x + 108, y),
                        task.class.as_str(),
                        text,
                    );
                    TextRenderer::draw_text(
                        &mut self.fb,
                        clip,
                        Point::new(clip.x + 178, y),
                        task.state.as_str(),
                        accent,
                    );
                    let mut memory = FixedText::empty();
                    memory.push_u64(task.memory_kib as u64);
                    memory.push_str(" K");
                    TextRenderer::draw_text(
                        &mut self.fb,
                        clip,
                        Point::new(clip.x + 266, y),
                        memory.as_str(),
                        text,
                    );
                }
                y += 25;
                index += 1;
            }
        }
    }

    fn draw_game_window(&mut self, rect: Rect) {
        self.draw_window_frame(rect, "Graphics Lab / live backbuffer");
        let body = Rect::new(rect.x + 8, rect.y + 45, rect.width - 16, rect.height - 53);
        self.fb.fill_rect(body, Color::rgb(18, 19, 18));
        self.fb.stroke_rect(body, Color::rgb(83, 84, 80));

        let clip = body.inset(10);
        self.fb
            .fill_rect_checker_blocks(clip, Color::rgb(23, 24, 23), Color::rgb(27, 28, 27), 20);

        let frame = self.game_frame as i32;
        let center = Point::new(clip.x + clip.width / 2, clip.y + clip.height / 2 + 12);
        let phase = frame % 160;
        let orbit_x = if phase < 80 { phase - 40 } else { 120 - phase };
        let orbit_y = ((frame % 96) - 48).abs() - 24;

        for index in 0..8 {
            let x = clip.x + 28 + ((index * 57 + frame * 3) % (clip.width - 56).max(1));
            let y = clip.y + 48 + ((index * 31 + frame * 2) % (clip.height - 76).max(1));
            self.fb
                .fill_rect(Rect::new(x, y, 3, 3), Color::rgb(172, 169, 157));
        }

        let cube = 54 + (frame % 24);
        let front = [
            Point::new(center.x - cube, center.y - cube / 2),
            Point::new(center.x + cube, center.y - cube / 2),
            Point::new(center.x + cube, center.y + cube / 2),
            Point::new(center.x - cube, center.y + cube / 2),
        ];
        let back_offset = Point::new(orbit_x / 2 + 32, orbit_y - 24);
        let back = [
            Point::new(front[0].x + back_offset.x, front[0].y + back_offset.y),
            Point::new(front[1].x + back_offset.x, front[1].y + back_offset.y),
            Point::new(front[2].x + back_offset.x, front[2].y + back_offset.y),
            Point::new(front[3].x + back_offset.x, front[3].y + back_offset.y),
        ];

        for index in 0..4 {
            let next = (index + 1) % 4;
            self.fb
                .line(front[index], front[next], 2, Color::ACCENT, clip);
            self.fb
                .line(back[index], back[next], 2, Color::SUCCESS, clip);
            self.fb.line(
                front[index],
                back[index],
                1,
                Color::rgb(162, 151, 126),
                clip,
            );
        }

        let sprite = Rect::new(
            center.x + orbit_x * 2 - 16,
            center.y + orbit_y * 2 - 16,
            32,
            32,
        );
        self.fb.blit_solid(sprite.intersect(clip), Color::WARNING);
        self.fb.fill_circle(
            Point::new(sprite.x + 16, sprite.y + 16),
            11,
            Color::TEXT_INVERTED,
        );

        let mut frame_text = FixedText::from_str("FRAME ");
        frame_text.push_u64(self.game_frame);
        frame_text.push_str("  DIRTY REGION BLIT");
        TextRenderer::draw_text(
            &mut self.fb,
            clip,
            Point::new(clip.x + 12, clip.y + 12),
            "LIVE FRAMEBUFFER",
            TextStyle::bold(14, Color::TEXT_INVERTED),
        );
        TextRenderer::draw_text(
            &mut self.fb,
            clip,
            Point::new(clip.x + 12, clip.y + 34),
            frame_text.as_str(),
            TextStyle::regular(12, Color::TEXT_MUTED),
        );
        TextRenderer::draw_text(
            &mut self.fb,
            clip,
            Point::new(clip.x + 12, clip.bottom() - 24),
            "ONLY CHANGED REGIONS ARE PRESENTED TO THE DISPLAY.",
            TextStyle::regular(11, Color::TEXT_MUTED),
        );
    }

    fn draw_about_window(&mut self, rect: Rect) {
        self.draw_window_frame(rect, "About GenOS");
        let body = Rect::new(rect.x + 8, rect.y + 45, rect.width - 16, rect.height - 53);
        self.fb.fill_rect(body, Color::rgb(29, 30, 29));
        self.fb.stroke_rect(body, Color::rgb(83, 84, 80));
        self.fb.fill_rect(
            Rect::new(body.x + 18, body.y + 20, 54, 54),
            Color::rgb(18, 19, 18),
        );
        self.fb
            .fill_rect(Rect::new(body.x + 30, body.y + 32, 30, 30), Color::ACCENT);
        TextRenderer::draw_text(
            &mut self.fb,
            body,
            Point::new(body.x + 92, body.y + 23),
            "GenOS 0.6",
            TextStyle::bold(16, Color::TEXT_INVERTED),
        );
        TextRenderer::draw_text(
            &mut self.fb,
            body,
            Point::new(body.x + 92, body.y + 52),
            "x86_64 experimental desktop kernel",
            TextStyle::regular(12, Color::TEXT_MUTED),
        );
        TextRenderer::draw_text(
            &mut self.fb,
            body,
            Point::new(body.x + 22, body.y + 104),
            "RING 3  /  SYSCALL ABI  /  SCHEDULER",
            TextStyle::regular(12, Color::TEXT_MUTED),
        );
        TextRenderer::draw_text(
            &mut self.fb,
            body,
            Point::new(body.x + 22, body.y + 136),
            "DRAG WINDOWS. TAB SWITCHES. ESC CLOSES.",
            TextStyle::regular(12, Color::TEXT_INVERTED),
        );
    }

    fn draw_window_frame(&mut self, rect: Rect, title: &str) {
        let active = self.window_is_open(self.focus) && rect == self.window_rect(self.focus);
        if active {
            self.fb.fill_rect(
                Rect::new(rect.x + 6, rect.y + 6, rect.width, rect.height),
                Color::rgb(22, 23, 22),
            );
        }
        self.fb.fill_rect(rect, Color::PANEL);
        self.fb
            .stroke_rect(rect, if active { Color::ACCENT } else { Color::BORDER });
        let titlebar = Rect::new(rect.x, rect.y, rect.width, 38);
        self.fb.fill_rect(
            titlebar,
            if active {
                Color::rgb(32, 33, 32)
            } else {
                Color::rgb(55, 56, 53)
            },
        );
        self.fb.fill_rect(
            Rect::new(rect.x, rect.y + 38, rect.width, 1),
            if active {
                Color::ACCENT_DARK
            } else {
                Color::BORDER
            },
        );
        TextRenderer::draw_text(
            &mut self.fb,
            titlebar,
            Point::new(rect.x + 14, rect.y + 11),
            title,
            TextStyle::regular(13, Color::TEXT_INVERTED),
        );
        let close = Rect::new(rect.right() - 34, rect.y + 8, 22, 22);
        self.fb.fill_rect(close, Color::rgb(50, 51, 49));
        self.fb.stroke_rect(close, Color::rgb(104, 104, 98));
        self.fb.line(
            Point::new(close.x + 6, close.y + 6),
            Point::new(close.right() - 6, close.bottom() - 6),
            1,
            Color::TEXT_INVERTED,
            close,
        );
        self.fb.line(
            Point::new(close.right() - 6, close.y + 6),
            Point::new(close.x + 6, close.bottom() - 6),
            1,
            Color::TEXT_INVERTED,
            close,
        );
    }

    fn draw_shell(&mut self, rect: Rect) {
        self.fb.fill_rect(rect, Color::WINDOW_DARK);
        self.fb.stroke_rect(rect, Color::rgb(74, 75, 71));
        let inner = rect.inset(14);
        let output_style = TextStyle::regular(15, Color::TEXT_INVERTED);
        let metrics = TextRenderer::metrics(output_style);
        let prompt_y = inner.bottom() - metrics.line_height;
        let columns = (inner.width / metrics.cell_width).max(1) as usize;
        let visible_rows =
            ((inner.height - metrics.line_height - 10) / metrics.line_height).max(1) as usize;
        let start = self.shell.visible_start(visible_rows);
        let mut y = inner.y;

        TextRenderer::draw_text(
            &mut self.fb,
            inner,
            Point::new(inner.x, y),
            "GENOS SHELL  /  HELP FOR COMMANDS  /  UP-DOWN FOR HISTORY",
            TextStyle::regular(14, Color::TEXT_MUTED),
        );
        y += metrics.line_height + 4;

        let mut index = start;
        while index < self.shell.len() && y < prompt_y {
            if let Some(line) = self.shell.line(index).copied() {
                y = self.draw_wrapped_line(inner, Point::new(inner.x, y), &line, columns);
            }
            index += 1;
        }

        self.draw_shell_input(rect);
    }

    fn draw_shell_input(&mut self, rect: Rect) {
        let inner = rect.inset(14);
        let output_style = TextStyle::regular(15, Color::TEXT_INVERTED);
        let prompt_style = TextStyle::bold(15, Color::ACCENT);
        let metrics = TextRenderer::metrics(output_style);
        let prompt_y = inner.bottom() - metrics.line_height;
        let strip = Rect::new(inner.x, prompt_y - 7, inner.width, metrics.line_height + 12);
        self.fb.fill_rect(strip, Color::WINDOW_DARK);
        self.fb.fill_rect(
            Rect::new(inner.x, prompt_y - 4, inner.width, 1),
            Color::rgb(61, 62, 58),
        );
        TextRenderer::draw_text(
            &mut self.fb,
            inner,
            Point::new(inner.x, prompt_y),
            "genos$",
            prompt_style,
        );
        let input_x = inner.x + metrics.cell_width * 8;
        TextRenderer::draw_text(
            &mut self.fb,
            inner,
            Point::new(input_x, prompt_y),
            self.input.as_str(),
            output_style,
        );
        TextRenderer::draw_cursor(
            &mut self.fb,
            inner,
            Point::new(
                input_x + self.input.len() as i32 * metrics.cell_width + 4,
                prompt_y,
            ),
            output_style,
        );
    }

    fn draw_wrapped_line(
        &mut self,
        clip: Rect,
        origin: Point,
        line: &ShellLine,
        columns: usize,
    ) -> i32 {
        let style = match line.kind {
            LineKind::Prompt => TextStyle::bold(16, Color::ACCENT),
            LineKind::Output => TextStyle::regular(16, Color::TEXT_INVERTED),
            LineKind::Error => TextStyle::regular(16, Color::ERROR),
            LineKind::Status => TextStyle::regular(16, Color::WARNING),
        };
        let metrics = TextRenderer::metrics(style);
        let text = line.text.as_str();
        let mut y = origin.y;
        if text.is_empty() {
            return y + metrics.line_height;
        }
        let bytes = text.as_bytes();
        let mut start = 0usize;
        while start < bytes.len() && y < clip.bottom() - metrics.line_height {
            let end = (start + columns).min(bytes.len());
            let part = core::str::from_utf8(&bytes[start..end]).unwrap_or("");
            TextRenderer::draw_text(&mut self.fb, clip, Point::new(origin.x, y), part, style);
            y += metrics.line_height;
            start = end;
        }
        y
    }

    fn draw_taskbar(&mut self, rect: Rect) {
        self.fb.fill_rect(rect, Color::rgb(24, 25, 24));
        self.fb
            .fill_rect(Rect::new(0, rect.y, rect.width, 1), Color::rgb(84, 85, 80));
        let start = self.start_button_rect();
        self.fb.fill_rect(
            start,
            if self.start_open {
                Color::ACCENT
            } else {
                Color::rgb(47, 48, 46)
            },
        );
        self.fb.stroke_rect(
            start,
            if self.start_open {
                Color::ACCENT
            } else {
                Color::BORDER
            },
        );
        TextRenderer::draw_text(
            &mut self.fb,
            start,
            Point::new(start.x + 14, start.y + 11),
            "GENOS",
            TextStyle::bold(
                13,
                if self.start_open {
                    Color::TEXT
                } else {
                    Color::TEXT_INVERTED
                },
            ),
        );

        self.draw_taskbar_button(self.taskbar_terminal_rect(), WindowKind::Terminal, "TERM");
        self.draw_taskbar_button(self.taskbar_files_rect(), WindowKind::Files, "FILES");
        self.draw_taskbar_button(self.taskbar_tasks_rect(), WindowKind::TaskManager, "TASKS");
        self.draw_taskbar_button(self.taskbar_game_rect(), WindowKind::Game, "LAB");
        self.draw_taskbar_button(self.taskbar_about_rect(), WindowKind::About, "ABOUT");

        let tray = Rect::new(rect.right() - 274, rect.y + 8, 260, 36);
        self.fb.fill_rect(tray, Color::rgb(31, 32, 31));
        self.fb.stroke_rect(tray, Color::rgb(68, 69, 65));
        let mut state = FixedText::from_str("F ");
        state.push_u64(self.vfs_files as u64);
        state.push_str("   Q ");
        state.push_u64(self.event_depth as u64);
        TextRenderer::draw_text(
            &mut self.fb,
            tray,
            Point::new(tray.x + 12, tray.y + 11),
            state.as_str(),
            TextStyle::regular(12, Color::TEXT_MUTED),
        );
        TextRenderer::draw_text(
            &mut self.fb,
            tray,
            Point::new(tray.right() - 88, tray.y + 11),
            self.clock.as_str(),
            TextStyle::bold(12, Color::TEXT_INVERTED),
        );
    }

    fn draw_taskbar_button(&mut self, rect: Rect, kind: WindowKind, label: &str) {
        let open = self.window_is_open(kind);
        let active = open && self.focus == kind;
        self.fb.fill_rect(
            rect,
            if active {
                Color::rgb(66, 62, 52)
            } else {
                Color::rgb(34, 35, 34)
            },
        );
        self.fb.stroke_rect(
            rect,
            if active {
                Color::ACCENT_DARK
            } else {
                Color::rgb(65, 66, 62)
            },
        );
        if open {
            self.fb.fill_rect(
                Rect::new(rect.x + 8, rect.bottom() - 3, rect.width - 16, 2),
                if active {
                    Color::ACCENT
                } else {
                    Color::TEXT_MUTED
                },
            );
        }
        TextRenderer::draw_text(
            &mut self.fb,
            rect,
            Point::new(rect.x + 12, rect.y + 11),
            label,
            TextStyle::regular(
                12,
                if active {
                    Color::TEXT_INVERTED
                } else {
                    Color::TEXT_MUTED
                },
            ),
        );
    }

    fn draw_cursor_overlay(&mut self) {
        let c = self.cursor;
        self.fb.overlay_line(
            c,
            Point::new(c.x + 12, c.y + 5),
            4,
            Color::WINDOW_DARK,
            self.fb.bounds(),
        );
        self.fb.overlay_line(
            c,
            Point::new(c.x + 5, c.y + 14),
            4,
            Color::WINDOW_DARK,
            self.fb.bounds(),
        );
        self.fb.overlay_line(
            Point::new(c.x + 12, c.y + 5),
            Point::new(c.x + 5, c.y + 14),
            4,
            Color::WINDOW_DARK,
            self.fb.bounds(),
        );
        self.fb.overlay_line(
            c,
            Point::new(c.x + 12, c.y + 5),
            2,
            Color::ACCENT,
            self.fb.bounds(),
        );
        self.fb.overlay_line(
            c,
            Point::new(c.x + 5, c.y + 14),
            2,
            Color::ACCENT,
            self.fb.bounds(),
        );
        self.fb.overlay_line(
            Point::new(c.x + 12, c.y + 5),
            Point::new(c.x + 5, c.y + 14),
            2,
            Color::ACCENT,
            self.fb.bounds(),
        );
    }
}
