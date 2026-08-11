//! Boot logo / splash screen — LobsterOS branding drawn to framebuffer

use super::console;
use super::fb::FramebufferInfo;
use super::graphics;
use crate::println;

/// Color palette for the boot logo
const LOBSTER_RED: u32 = 0xFFCC3300;
const LOBSTER_DARK_RED: u32 = 0xFF8B1A00;
const SAND: u32 = 0xFFE8D5B7;
const WHITE: u32 = 0xFFFFFFFF;
const BLACK: u32 = 0xFF000000;
const GRAY: u32 = 0xFF404040;

/// Draw the LobsterOS boot splash screen
pub fn draw_splash(info: FramebufferInfo) {
    if info.base_addr == 0 {
        println!("[logo] No framebuffer, skipping splash");
        return;
    }

    let cx = info.width / 2;
    let cy = info.height / 2;

    // Dark blue gradient background — top half
    for y in 0..info.height / 2 {
        let half_h = info.height / 2;
        let r = 0x10 + (y * 0x20) / half_h;
        let g = 0x20 + (y * 0x1A) / half_h;
        let shade = 0xFF000000 | (r as u32) << 16 | (g as u32) << 8;
        for x in 0..info.width {
            graphics::draw_pixel(&info, x, y, shade);
        }
    }
    // Bottom half — darker
    for y in info.height / 2..info.height {
        let half_h = info.height / 2;
        let t = ((y - half_h) * 0x20) / half_h;
        let shade = 0xFF000000 | ((0x10 + t) as u32) << 16 | ((0x20 + t) as u32) << 8;
        for x in 0..info.width {
            graphics::draw_pixel(&info, x, y, shade);
        }
    }

    // Draw a large circle as the "shell" background
    let shell_radius = if info.width < info.height {
        info.width / 4
    } else {
        info.height / 4
    };
    graphics::draw_circle(&info, cx, cy, shell_radius, LOBSTER_DARK_RED);
    graphics::draw_circle(&info, cx, cy, shell_radius + 2, LOBSTER_RED);
    graphics::draw_circle(&info, cx, cy, shell_radius + 4, LOBSTER_RED);

    // Draw "LobsterOS" title centered
    let title = "LobsterOS";
    let title_width = title.len() as u32 * console::CHAR_WIDTH;
    let title_x = cx.saturating_sub(title_width / 2);
    let title_y = cy.saturating_sub(shell_radius / 2);

    graphics::draw_text(&info, title_x, title_y, title, WHITE, 0);

    // Draw version below
    let ver = "v0.2.0";
    let ver_width = ver.len() as u32 * console::CHAR_WIDTH;
    let ver_x = cx.saturating_sub(ver_width / 2);
    graphics::draw_text(
        &info,
        ver_x,
        title_y + console::CHAR_HEIGHT * 2,
        ver,
        SAND,
        0,
    );

    // Draw tagline
    let tag = "Bare-metal Rust on BCM2712";
    let tag_width = tag.len() as u32 * console::CHAR_WIDTH;
    let tag_x = cx.saturating_sub(tag_width / 2);
    graphics::draw_text(
        &info,
        tag_x,
        title_y + console::CHAR_HEIGHT * 4,
        tag,
        GRAY,
        0,
    );

    // Draw decorative lines
    let line_y = title_y + console::CHAR_HEIGHT * 6;
    let line_start = cx.saturating_sub(shell_radius);
    let line_end = cx + shell_radius;
    graphics::draw_line(&info, line_start, line_y, line_end, line_y, LOBSTER_RED);

    // Bottom info bar
    let bar_y = info.height - console::CHAR_HEIGHT * 3;
    graphics::draw_rect(
        &info,
        0,
        bar_y,
        info.width,
        console::CHAR_HEIGHT * 3,
        0xFF0A1520,
    );

    let info1 = "Cortex-A76 | 4 Cores | 2.4 GHz | 8GB RAM";
    let info1_x = cx.saturating_sub((info1.len() as u32 * console::CHAR_WIDTH) / 2);
    graphics::draw_text(&info, info1_x, bar_y + 4, info1, 0xFF608090, 0);

    let info2 = "MMU | GIC-400 | Virtual Memory | Heap Allocator";
    let info2_x = cx.saturating_sub((info2.len() as u32 * console::CHAR_WIDTH) / 2);
    graphics::draw_text(
        &info,
        info2_x,
        bar_y + console::CHAR_HEIGHT + 4,
        info2,
        0xFF405060,
        0,
    );

    println!("[logo] Boot splash drawn ({}x{})", info.width, info.height);
}

/// Draw a simple ASCII-art lobster on the UART console
pub fn draw_ascii_logo() {
    println!();
    println!(r"     ___                       ");
    println!(r"    / _ \___  ____  ___  _____ ");
    println!(r"   / , _/ _ \/ __/ / _ \/ ___/");
    println!(r"  /_/|_|\___/_/   / , __/ /    ");
    println!(r"                   |_|  /_/     ");
    println!();
    println!("  === LobsterOS v0.2.0 ===");
    println!("  Bare-metal Rust on Raspberry Pi 5");
    println!();
}

/// Draw a progress bar at the bottom of the screen
pub fn draw_progress_bar(info: FramebufferInfo, progress: u32, label: &str) {
    if info.base_addr == 0 {
        return;
    }

    let bar_height = 20u32;
    let bar_margin = 80u32;
    let bar_y = info.height - 60;
    let bar_width = info.width - bar_margin * 2;

    // Background
    graphics::draw_rect(&info, bar_margin, bar_y, bar_width, bar_height, GRAY);

    // Fill
    let fill_width = (bar_width * progress.min(100)) / 100;
    graphics::draw_rect(
        &info,
        bar_margin,
        bar_y,
        fill_width,
        bar_height,
        LOBSTER_RED,
    );

    // Border
    graphics::draw_rect_outline(&info, bar_margin, bar_y, bar_width, bar_height, WHITE);

    // Label
    let label_x = bar_margin;
    graphics::draw_text(
        &info,
        label_x,
        bar_y - console::CHAR_HEIGHT - 2,
        label,
        SAND,
        0,
    );

    // Percentage text
    let tens = (progress / 10) as u8;
    let ones = (progress % 10) as u8;
    let pct_bytes = [b'0' + tens, b'0' + ones, b'%'];
    let pct_str = core::str::from_utf8(&pct_bytes).unwrap_or("%");
    let pct_x = bar_margin + bar_width + 8;
    graphics::draw_text(&info, pct_x, bar_y + 2, pct_str, WHITE, 0);
}

/// Clear splash and prepare for console output
pub fn clear_splash(info: FramebufferInfo) {
    if info.base_addr == 0 {
        return;
    }
    graphics::draw_rect(&info, 0, 0, info.width, info.height, BLACK);
}
