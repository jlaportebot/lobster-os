//! System Monitor app — CPU, memory, network, temperature
//!
//! Real-time system resource display in a window manager app.

use crate::display::fb;
use crate::display::graphics;
use alloc::format;

const APP_NAME: &str = "System Monitor";

/// System monitor app state
pub struct SystemMonitor {
    cpu_history: [[u32; 4]; 60],
    mem_history: [u64; 60],
    net_rx_history: [u64; 60],
    net_tx_history: [u64; 60],
    sample_idx: usize,
    running: bool,
}

impl SystemMonitor {
    pub fn new() -> Self {
        Self {
            cpu_history: [[0; 4]; 60],
            mem_history: [0; 60],
            net_rx_history: [0; 60],
            net_tx_history: [0; 60],
            sample_idx: 0,
            running: true,
        }
    }

    /// Take a sample of current system state
    pub fn sample(&mut self) {
        let (total, allocated, _first_free) = crate::memory::frame_alloc::stats();
        self.mem_history[self.sample_idx] = (allocated * 4096) as u64;
        let _ = total;
        self.sample_idx = (self.sample_idx + 1) % 60;
    }

    /// Render the system monitor
    pub fn render(&self, x: u32, y: u32, width: u32, _height: u32) {
        let info = fb::get_info();

        // Title bar
        graphics::draw_rect(&info, x, y, width, 24, 0x2D2D30);
        graphics::draw_text(&info, x + 8, y + 6, APP_NAME, 0xFFFFFF, 0x2D2D30);

        // CPU usage section
        let cpu_y = y + 30;
        graphics::draw_text(&info, x + 8, cpu_y, "CPU Usage", 0xAAAACC, 0x1E1E1E);
        for core in 0..4u32 {
            let core_y = cpu_y + 20 + core * 30;
            let label = match core {
                0 => "Core 0",
                1 => "Core 1",
                2 => "Core 2",
                _ => "Core 3",
            };
            graphics::draw_text(&info, x + 8, core_y, label, 0x8888AA, 0x1E1E1E);

            let current_load = self.cpu_history[(self.sample_idx + 59) % 60][core as usize];
            let bar_width = (width.saturating_sub(120)) * current_load / 100;

            // Background bar
            graphics::draw_rect(&info, x + 80, core_y, width - 90, 16, 0x3D3D40);
            // Load bar
            let color = if current_load < 50 {
                0x4EC9B0
            } else if current_load < 80 {
                0xDCDCAA
            } else {
                0xF44747
            };
            graphics::draw_rect(&info, x + 80, core_y, bar_width, 16, color);

            let pct_str = format!("{}%", current_load);
            graphics::draw_text(&info, x + width - 45, core_y, &pct_str, 0xCCCCCC, 0x1E1E1E);
        }

        // Memory section
        let mem_y = cpu_y + 150;
        graphics::draw_text(&info, x + 8, mem_y, "Memory", 0xAAAACC, 0x1E1E1E);

        let current_mem = self.mem_history[(self.sample_idx + 59) % 60];
        let mem_mb = current_mem / (1024 * 1024);
        let mem_str = format!("{} MB used", mem_mb);
        graphics::draw_text(&info, x + 8, mem_y + 20, &mem_str, 0x8888AA, 0x1E1E1E);

        // Network section
        let net_y = mem_y + 50;
        graphics::draw_text(&info, x + 8, net_y, "Network", 0xAAAACC, 0x1E1E1E);
        let current_rx = self.net_rx_history[(self.sample_idx + 59) % 60];
        let current_tx = self.net_tx_history[(self.sample_idx + 59) % 60];
        let rx_str = format!("RX: {} KB", current_rx / 1024);
        let tx_str = format!("TX: {} KB", current_tx / 1024);
        graphics::draw_text(&info, x + 8, net_y + 20, &rx_str, 0x8888AA, 0x1E1E1E);
        graphics::draw_text(&info, x + 140, net_y + 20, &tx_str, 0x8888AA, 0x1E1E1E);

        // Temperature
        let temp_y = net_y + 50;
        graphics::draw_text(&info, x + 8, temp_y, "Temperature", 0xAAAACC, 0x1E1E1E);
        graphics::draw_text(&info, x + 8, temp_y + 20, "N/A", 0x8888AA, 0x1E1E1E);
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn close(&mut self) {
        self.running = false;
    }
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init() {
    crate::println!("[apps] System Monitor initialized");
}
