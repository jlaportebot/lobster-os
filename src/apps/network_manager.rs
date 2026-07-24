//! Network Manager app — WiFi scanning, connection, and status display
//!
//! Shows available networks, signal strength, security type.
//! Can connect/disconnect, view IP configuration.

use crate::display::fb;
use crate::display::graphics;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const APP_NAME: &str = "Network Manager";

/// Network info entry
#[derive(Clone)]
struct NetworkInfo {
    ssid: String,
    bssid: [u8; 6],
    rssi: i16,
    channel: u8,
    secure: bool,
}

/// Network manager state
pub struct NetworkManager {
    networks: Vec<NetworkInfo>,
    selected_idx: usize,
    scroll_offset: usize,
    connected_ssid: Option<String>,
    ip_address: [u8; 4],
    scanning: bool,
    running: bool,
}

impl NetworkManager {
    pub fn new() -> Self {
        Self {
            networks: Vec::new(),
            selected_idx: 0,
            scroll_offset: 0,
            connected_ssid: None,
            ip_address: [0; 4],
            scanning: false,
            running: true,
        }
    }

    /// Trigger a WiFi scan
    pub fn scan(&mut self) {
        self.scanning = true;
        // In production, call WiFi driver scan
        // For now, add placeholder networks
        self.networks.clear();
        self.networks.push(NetworkInfo {
            ssid: String::from("LobsterNet"),
            bssid: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
            rssi: -45,
            channel: 6,
            secure: true,
        });
        self.networks.push(NetworkInfo {
            ssid: String::from("Pi-5-Hotspot"),
            bssid: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            rssi: -62,
            channel: 11,
            secure: true,
        });
        self.networks.push(NetworkInfo {
            ssid: String::from("OpenWiFi"),
            bssid: [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
            rssi: -78,
            channel: 1,
            secure: false,
        });
        self.scanning = false;
    }

    /// Connect to selected network
    pub fn connect_selected(&mut self) {
        if let Some(net) = self.networks.get(self.selected_idx) {
            self.connected_ssid = Some(net.ssid.clone());
            // Simulate DHCP
            self.ip_address = [192, 168, 1, 100];
            crate::println!("[net_mgr] Connected to {} ({} dBm)", net.ssid, net.rssi);
        }
    }

    /// Disconnect from current network
    pub fn disconnect(&mut self) {
        self.connected_ssid = None;
        self.ip_address = [0; 4];
        crate::println!("[net_mgr] Disconnected");
    }

    pub fn select_up(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    pub fn select_down(&mut self) {
        if self.selected_idx + 1 < self.networks.len() {
            self.selected_idx += 1;
        }
    }

    /// Get signal strength bars (0-4)
    fn rssi_to_bars(rssi: i16) -> u8 {
        if rssi >= -50 {
            4
        } else if rssi >= -65 {
            3
        } else if rssi >= -75 {
            2
        } else if rssi >= -85 {
            1
        } else {
            0
        }
    }

    /// Render the network manager
    pub fn render(&self, x: u32, y: u32, width: u32, height: u32) {
        let info = fb::get_info();

        // Title bar
        graphics::draw_rect(&info, x, y, width, 28, 0x2D2D30);
        graphics::draw_text(&info, x + 8, y + 6, APP_NAME, 0xFFFFFF, 0x2D2D30);

        // Connection status
        let status_y = y + 30;
        if let Some(ref ssid) = self.connected_ssid {
            let ip = format!(
                "{}.{}.{}.{}",
                self.ip_address[0], self.ip_address[1], self.ip_address[2], self.ip_address[3]
            );
            let status = format!("Connected: {} | IP: {}", ssid, ip);
            graphics::draw_text(&info, x + 8, status_y, &status, 0x4EC9B0, 0x1E1E1E);
        } else {
            graphics::draw_text(&info, x + 8, status_y, "Not connected", 0xF44747, 0x1E1E1E);
        }

        // Scan status
        if self.scanning {
            graphics::draw_text(
                &info,
                x + width - 80,
                status_y,
                "Scanning...",
                0xFFBD2E,
                0x1E1E1E,
            );
        }

        // Network list header
        let header_y = status_y + 20;
        graphics::draw_rect(&info, x, header_y, width, 20, 0x1A1A2A);
        graphics::draw_text(&info, x + 8, header_y + 4, "Network", 0x8888CC, 0x1A1A2A);
        graphics::draw_text(&info, x + 200, header_y + 4, "Signal", 0x8888CC, 0x1A1A2A);
        graphics::draw_text(&info, x + 280, header_y + 4, "Security", 0x8888CC, 0x1A1A2A);

        // Network list
        let row_y = header_y + 22;
        let row_h = 24u32;

        for (i, net) in self.networks.iter().enumerate() {
            let ry = row_y + i as u32 * row_h;
            if ry + row_h > y + height - 20 {
                break;
            }

            let is_selected = i == self.selected_idx;
            let is_connected = self.connected_ssid.as_deref() == Some(net.ssid.as_str());

            let bg = if is_selected {
                0x3A4A6A
            } else if i % 2 == 0 {
                0x1E1E2E
            } else {
                0x1A1A2A
            };
            graphics::draw_rect(&info, x, ry, width, row_h, bg);

            // SSID
            let ssid_display = if is_connected {
                format!("● {}", net.ssid)
            } else {
                net.ssid.clone()
            };
            graphics::draw_text(&info, x + 8, ry + 6, &ssid_display, 0xCCCCCC, bg);

            // Signal bars
            let bars = Self::rssi_to_bars(net.rssi);
            let bar_x = x + 200;
            for bar_idx in 0..4u32 {
                let bar_h = 4 + bar_idx * 3;
                let color = if bar_idx < bars as u32 {
                    0x4EC9B0
                } else {
                    0x3D3D40
                };
                graphics::draw_rect(&info, bar_x + bar_idx * 6, ry + 20 - bar_h, 4, bar_h, color);
            }

            // Security
            let sec_str = if net.secure { "WPA2" } else { "Open" };
            let sec_color = if net.secure { 0x4EC9B0 } else { 0xFFBD2E };
            graphics::draw_text(&info, x + 280, ry + 6, sec_str, sec_color, bg);

            // Channel
            let ch_str = format!("Ch {}", net.channel);
            graphics::draw_text(&info, x + 350, ry + 6, &ch_str, 0x666688, bg);
        }

        // Status bar
        let sb_y = y + height - 20;
        let status = format!(
            "{} networks found{}",
            self.networks.len(),
            if self.scanning { " (scanning...)" } else { "" }
        );
        graphics::draw_rect(&info, x, sb_y, width, 20, 0x1A1A2A);
        graphics::draw_text(&info, x + 8, sb_y + 4, &status, 0x666688, 0x1A1A2A);
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn close(&mut self) {
        self.running = false;
    }
}

impl Default for NetworkManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init() {
    crate::println!("[apps] Network Manager initialized");
}
