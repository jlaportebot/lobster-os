//! Process Manager app — view and manage running processes
//!
//! Lists all tasks with PID, state, runtime, priority.
//! Can kill processes, change priority.

use crate::display::fb;
use crate::display::graphics;
use crate::sched;
use crate::sched::TaskState;
use alloc::format;
use alloc::vec::Vec;

const APP_NAME: &str = "Process Manager";
const ROW_HEIGHT: u32 = 20;
const MAX_DISPLAY_ROWS: usize = 20;

/// Process manager state
pub struct ProcessManager {
    selected_idx: usize,
    scroll_offset: usize,
    running: bool,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            selected_idx: 0,
            scroll_offset: 0,
            running: true,
        }
    }

    pub fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset += 1;
    }

    pub fn select_up(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn select_down(&mut self) {
        self.selected_idx += 1;
    }

    pub fn kill_selected(&mut self) {
        crate::println!("[proc_mgr] Kill requested for index {}", self.selected_idx);
    }

    fn state_str(state: TaskState) -> &'static str {
        match state {
            TaskState::Created => "Created",
            TaskState::Ready => "Ready",
            TaskState::Running => "Running",
            TaskState::Blocked => "Blocked",
            TaskState::Zombie => "Zombie",
        }
    }

    fn state_color(state: TaskState) -> u32 {
        match state {
            TaskState::Running => 0x4EC9B0,
            TaskState::Ready => 0x88CC88,
            TaskState::Blocked => 0xFFBD2E,
            TaskState::Zombie => 0xF44747,
            TaskState::Created => 0x666688,
        }
    }

    /// Collect current task info from scheduler
    fn collect_tasks() -> Vec<(u64, TaskState, u64, u8)> {
        let mut result = Vec::new();
        for i in 0..crate::sched::MAX_TASKS {
            unsafe {
                if let Some(ref task) = crate::sched::TASK_TABLE[i] {
                    let priority = match task.priority {
                        crate::sched::Priority::Low => 0,
                        crate::sched::Priority::Normal => 1,
                        crate::sched::Priority::High => 2,
                    };
                    result.push((task.pid, task.state, task.runtime_ticks, priority));
                }
            }
        }
        result
    }

    /// Render the process manager
    pub fn render(&self, x: u32, y: u32, width: u32, height: u32) {
        let info = fb::get_info();

        // Title bar (legacy standalone chrome)
        graphics::draw_rect(&info, x, y, width, 28, 0x2D2D30);
        graphics::draw_text(&info, x + 8, y + 6, APP_NAME, 0xFFFFFF, 0x2D2D30);
        self.render_pane(&info, x, y + 28, width, height.saturating_sub(28));
    }

    /// Render the process list into a client region (no chrome) — used by
    /// the merged Task Manager.
    pub fn render_pane(
        &self,
        info: &crate::display::fb::FramebufferInfo,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) {

        // Column headers
        let header_y = y + 2;
        graphics::draw_rect(info, x, header_y, width, 20, 0x1A1A2A);
        graphics::draw_text(info, x + 8, header_y + 4, "PID", 0x8888CC, 0x1A1A2A);
        graphics::draw_text(info, x + 70, header_y + 4, "State", 0x8888CC, 0x1A1A2A);
        graphics::draw_text(info, x + 180, header_y + 4, "Runtime", 0x8888CC, 0x1A1A2A);
        graphics::draw_text(info, x + 300, header_y + 4, "Priority", 0x8888CC, 0x1A1A2A);

        // Collect tasks
        let tasks = Self::collect_tasks();
        let count = tasks.len();

        // Process rows
        let row_start = header_y + 20;
        let visible_rows = ((height.saturating_sub(50)) / ROW_HEIGHT) as usize;

        for row_idx in 0..visible_rows.min(MAX_DISPLAY_ROWS) {
            let proc_idx = row_idx + self.scroll_offset;
            if proc_idx >= count {
                break;
            }

            let row_y = row_start + row_idx as u32 * ROW_HEIGHT;
            let is_selected = row_idx == self.selected_idx;
            let (pid, state, runtime, priority) = tasks[proc_idx];

            let bg = if is_selected {
                0x3A4A6A
            } else if row_idx % 2 == 0 {
                0x1E1E2E
            } else {
                0x1A1A2A
            };
            graphics::draw_rect(info, x, row_y, width, ROW_HEIGHT, bg);

            let pid_str = format!("{}", pid);
            graphics::draw_text(info, x + 8, row_y + 4, &pid_str, 0xCCCCCC, bg);
            graphics::draw_text(
                info,
                x + 70,
                row_y + 4,
                Self::state_str(state),
                Self::state_color(state),
                bg,
            );
            let rt_str = format!("{} ticks", runtime);
            graphics::draw_text(info, x + 180, row_y + 4, &rt_str, 0xCCCCCC, bg);
            let pri_str = match priority {
                0 => "Low",
                2 => "High",
                _ => "Normal",
            };
            graphics::draw_text(info, x + 300, row_y + 4, pri_str, 0xCCCCCC, bg);
        }

        // Status bar
        let status_y = y + height.saturating_sub(20);
        let status = format!("{} processes | row {} selected", count, self.selected_idx);
        graphics::draw_rect(info, x, status_y, width, 20, 0x1A1A2A);
        graphics::draw_text(info, x + 8, status_y + 4, &status, 0x666688, 0x1A1A2A);
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn close(&mut self) {
        self.running = false;
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init() {
    crate::println!("[apps] Process Manager initialized");
}

/// Global Process Manager instance
static mut PROCESS_MANAGER: Option<ProcessManager> = None;

/// Get the global Process Manager instance
fn get_process_manager() -> &'static mut ProcessManager {
    unsafe {
        if PROCESS_MANAGER.is_none() {
            PROCESS_MANAGER = Some(ProcessManager::new());
        }
        PROCESS_MANAGER
            .as_mut()
            .expect("ProcessManager not initialized")
    }
}

/// Render Process Manager to its window (QEMU direct-to-framebuffer mode)
#[cfg(feature = "qemu_raspi5b")]
pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
    if let Some(win_id) = crate::wm::window::find_by_title("Process Manager") {
        if let Some(win) = crate::wm::window::get(win_id) {
            if win.active {
                let focused = crate::wm::window::focused_id() == win_id;
                let (cx, cy, cw, ch) = crate::wm::window_frame::draw_frame(
                    fb, win_id, win.x, win.y, win.w, win.h, "Process Manager", focused,
                );
                let pm = get_process_manager();
                pm.render(cx, cy, cw, ch);
            }
        }
    }
}
