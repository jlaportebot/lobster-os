//! Start Menu — the panel launcher's app menu
//!
//! A polished start-menu surface: grid of application tiles with icons,
//! opens on launcher click, closes on outside click or Esc, launches the
//! selected app's window (focusing an existing instance if one is open).

use crate::display::fb::get_info;
use crate::display::graphics::{draw_rect, draw_rect_outline, draw_text, fill_rounded_rect};
use crate::wm::theme;
use crate::wm::window;

/// One launchable application entry
pub struct AppEntry {
    pub name: &'static str,
    /// Emoji/box-drawing icon glyph drawn on the tile
    pub icon: &'static str,
    /// Tile accent color (ARGB)
    pub color: u32,
    /// Launch behavior
    pub launch: fn(),
}

/// The full application catalog
pub static APPS: &[AppEntry] = &[
    AppEntry {
        name: "Terminal",
        icon: ">_",
        color: 0xFF2E3440,
        launch: || {
            focus_or_create("Terminal", 720, 500, || {
                let _ = window::create(100, 80, 720, 500, "Terminal");
            });
        },
    },
    AppEntry {
        name: "Files",
        icon: "\u{1F4C1}",
        color: 0xFFD08770,
        launch: || {
            focus_or_create("File Manager", 500, 400, || {
                let _ = window::create(120, 100, 500, 400, "File Manager");
            });
        },
    },
    AppEntry {
        name: "Editor",
        icon: "\u{1F4DD}",
        color: 0xFFEBCB8B,
        launch: || {
            focus_or_create("Editor", 520, 400, || {
                let _ = window::create(140, 120, 520, 400, "Editor");
            });
        },
    },
    AppEntry {
        name: "Calculator",
        icon: "\u{1F5A9}",
        color: 0xFF88C0D0,
        launch: || {
            focus_or_create("Calculator", 300, 400, || {
                let _ = window::create(160, 140, 300, 400, "Calculator");
            });
        },
    },
    AppEntry {
        name: "Task Manager",
        icon: "\u{1F4CA}",
        color: 0xFFA3BE8C,
        launch: || {
            crate::apps::task_manager::open();
        },
    },
    AppEntry {
        name: "Settings",
        icon: "\u{2699}",
        color: 0xFF81A1C1,
        launch: || {
            focus_or_create("Settings", 400, 300, || {
                let _ = window::create(220, 200, 400, 300, "Settings");
            });
        },
    },
    AppEntry {
        name: "Network",
        icon: "\u{1F310}",
        color: 0xFF8FBCBB,
        launch: || {
            focus_or_create("Network", 420, 300, || {
                let _ = window::create(240, 220, 420, 300, "Network");
            });
        },
    },
    AppEntry {
        name: "App Store",
        icon: "\u{1F381}",
        color: 0xFFBF616A,
        launch: || {
            focus_or_create("App Store", 460, 320, || {
                let _ = window::create(260, 240, 460, 320, "App Store");
            });
        },
    },
    AppEntry {
        name: "Images",
        icon: "\u{1F5BC}",
        color: 0xFFD8DEE9,
        launch: || {
            focus_or_create("Image Viewer", 460, 340, || {
                let _ = window::create(280, 260, 460, 340, "Image Viewer");
            });
        },
    },
    AppEntry {
        name: "Browser",
        icon: "\u{1F310}",
        color: 0xFF5E81AC,
        launch: || {
            focus_or_create("Browser", 900, 620, || {
                crate::apps::browser::open("http://example.com");
            });
        },
    },
    AppEntry {
        name: "Notes",
        icon: "\u{1F4AC}",
        color: 0xFFEBCB8B,
        launch: || {
            focus_or_create("Notes", 700, 480, || {
                crate::apps::notes::init();
                let _ = window::create(260, 140, 700, 480, "Notes");
            });
        },
    },
    AppEntry {
        name: "About",
        icon: "\u{1F99E}",
        color: 0xFFFF6B6B,
        launch: || {
            focus_or_create("About", 360, 260, || {
                let _ = window::create(300, 280, 360, 260, "About");
            });
        },
    },
];

// ─── Menu state ───────────────────────────────────────────────────────

const MENU_WIDTH: u32 = 360;
const MENU_HEIGHT: u32 = 420;
const TILE_H: u32 = 56;
const TILE_MARGIN: u32 = 8;
const MENU_TOP_PAD: u32 = 12;
/// Number of rows visible without scrolling
const VISIBLE_ROWS: usize = 7;

static mut OPEN: bool = false;
static mut HOVER_ROW: i32 = -1;
static mut SCROLL: usize = 0;

pub fn is_open() -> bool {
    unsafe { OPEN }
}

pub fn toggle() {
    unsafe {
        OPEN = !OPEN;
        if OPEN {
            HOVER_ROW = -1;
            SCROLL = 0;
        }
    }
    // Force a full recomposition so the menu appears/disappears immediately.
    crate::wm::compositor::mark_full_redraw();
    let state = if is_open() { "open" } else { "closed" };
    crate::println!("[wm] Start menu {}", state);
}

pub fn close() {
    unsafe {
        if OPEN {
            OPEN = false;
        }
    }
}

/// Mouse wheel/arrow scroll within the menu
pub fn scroll(delta: i32) {
    unsafe {
        if !OPEN {
            return;
        }
        let max_scroll = APPS.len().saturating_sub(VISIBLE_ROWS);
        if delta > 0 && SCROLL < max_scroll {
            SCROLL += 1;
        } else if delta < 0 && SCROLL > 0 {
            SCROLL -= 1;
        }
    }
}

/// Handle a click inside the menu region. Returns true if the click was
/// consumed (so the WM does not also dispatch it to windows).
pub fn handle_click(px: u32, py: u32) -> bool {
    if !is_open() {
        return false;
    }
    let (x0, y0) = menu_origin();
    let w = MENU_WIDTH;
    let h = MENU_HEIGHT;

    // Outside → close, swallow nothing (let the click act on the desktop)
    if px < x0 || px >= x0 + w || py < y0 || py >= y0 + h {
        close();
        return false;
    }

    // Header area → close
    if py < y0 + MENU_TOP_PAD + 28 {
        close();
        return true;
    }

    // Tile row?
    let inner_y = py - (y0 + MENU_TOP_PAD + 28);
    if inner_y < VISIBLE_ROWS as u32 * (TILE_H + TILE_MARGIN) {
        let row = (inner_y / (TILE_H + TILE_MARGIN)) as usize;
        let idx = unsafe { SCROLL } + row;
        if idx < APPS.len() {
            close();
            (APPS[idx].launch)();
        }
        return true;
    }
    true
}

/// Track hover for highlight + keyboard selection
pub fn handle_move(px: u32, py: u32) {
    if !is_open() {
        return;
    }
    let (x0, y0) = menu_origin();
    if px < x0 || px >= x0 + MENU_WIDTH || py < y0 || py >= y0 + MENU_HEIGHT {
        unsafe { HOVER_ROW = -1; }
        return;
    }
    let inner_y = py.saturating_sub(y0 + MENU_TOP_PAD + 28);
    if inner_y < VISIBLE_ROWS as u32 * (TILE_H + TILE_MARGIN) {
        unsafe { HOVER_ROW = (inner_y / (TILE_H + TILE_MARGIN)) as i32; }
    } else {
        unsafe { HOVER_ROW = -1; }
    }
}

/// Activate the hovered row (Enter key)
pub fn activate_hovered() {
    let row = unsafe { HOVER_ROW };
    if row >= 0 {
        let idx = unsafe { SCROLL } + row as usize;
        if idx < APPS.len() {
            close();
            (APPS[idx].launch)();
        }
    }
}

/// Handle a key while the menu is open (Esc closes, arrows move, Enter opens)
pub fn handle_key(scancode: u8) -> bool {
    if !is_open() {
        return false;
    }
    match scancode {
        0x29 if !crate::usb::hid::alt_held() => {
            // Esc — close the menu
            close();
            true
        }
        0x50 | 0x52 => {
            // Left/Up arrows — move hover up, scrolling as needed
            unsafe {
                if HOVER_ROW <= 0 && SCROLL > 0 {
                    SCROLL -= 1;
                }
                if HOVER_ROW > 0 {
                    HOVER_ROW -= 1;
                } else {
                    HOVER_ROW = 0;
                }
            }
            true
        }
        0x4F | 0x51 => {
            // Right/Down arrows — move hover down, scrolling as needed
            unsafe {
                if HOVER_ROW < 0 {
                    HOVER_ROW = 0;
                } else if (HOVER_ROW as usize) < VISIBLE_ROWS - 1 {
                    HOVER_ROW += 1;
                } else if SCROLL + VISIBLE_ROWS < APPS.len() {
                    SCROLL += 1;
                }
            }
            true
        }
        0x28 => {
            // Enter — launch hovered
            activate_hovered();
            true
        }
        _ => false,
    }
}

fn menu_origin() -> (u32, u32) {
    let fb = get_info();
    // Anchor bottom-left, above the panel
    let panel_h = crate::wm::panel::panel_height();
    let x0 = 4;
    let y0 = fb.height.saturating_sub(panel_h + MENU_HEIGHT + 4);
    (x0, y0)
}

// ─── Rendering ────────────────────────────────────────────────────────

pub fn draw(fb: &crate::display::fb::FramebufferInfo) {
    if !is_open() {
        return;
    }
    let theme = theme::current();
    let (x0, y0) = menu_origin();

    // Shadow (simple offset dark rect)
    draw_rect(fb, x0 + 6, y0 + 8, MENU_WIDTH, MENU_HEIGHT, 0x66000000);

    // Menu background + border
    fill_rounded_rect(fb, x0, y0, MENU_WIDTH, MENU_HEIGHT, 10, theme.menu_bg);
    draw_rect_outline(fb, x0, y0, MENU_WIDTH, MENU_HEIGHT, theme.menu_border);

    // Header: brand
    let header_h = 28u32;
    draw_text(
        fb,
        x0 + 14,
        y0 + MENU_TOP_PAD + (header_h - theme.font_size) / 2,
        "\u{1F99E} LobsterOS",
        theme.label_text,
        theme.menu_bg,
    );
    // Divider
    draw_rect(
        fb,
        x0 + 10,
        y0 + MENU_TOP_PAD + header_h + 4,
        MENU_WIDTH - 20,
        1,
        theme.menu_border,
    );

    // Tiles
    let inner_x = x0 + 10;
    let tiles_top = y0 + MENU_TOP_PAD + header_y_offset();
    let tile_w = MENU_WIDTH - 20;
    let scroll = unsafe { SCROLL };
    let hover = unsafe { HOVER_ROW };

    for row in 0..VISIBLE_ROWS {
        let idx = scroll + row;
        if idx >= APPS.len() {
            break;
        }
        let app = &APPS[idx];
        let ty = tiles_top + row as u32 * (TILE_H + TILE_MARGIN);

        // Hover/selection highlight
        let tile_bg = if hover == row as i32 {
            theme.menu_item_hover
        } else {
            blend(theme.menu_bg, app.color, 28)
        };
        fill_rounded_rect(fb, inner_x, ty, tile_w, TILE_H, 8, tile_bg);

        // Icon square
        let icon_sz = TILE_H - 16;
        fill_rounded_rect(fb, inner_x + 8, ty + 8, icon_sz, icon_sz, 8, app.color);
        draw_text(
            fb,
            inner_x + 8 + (icon_sz - 24) / 2,
            ty + 8 + (icon_sz - theme.font_size) / 2,
            app.icon,
            0xFFFFFFFF,
            app.color,
        );

        // App name
        draw_text(
            fb,
            inner_x + icon_sz + 24,
            ty + (TILE_H - theme.font_size) / 2,
            app.name,
            theme.label_text,
            tile_bg,
        );
    }

    // Scroll indicator
    if APPS.len() > VISIBLE_ROWS {
        let bar_h = 4;
        let track_y = tiles_top + VISIBLE_ROWS as u32 * (TILE_H + TILE_MARGIN) + 6;
        draw_rect(fb, inner_x, track_y, tile_w, bar_h, theme.menu_border);
        let thumb_w = (tile_w * VISIBLE_ROWS as u32 / APPS.len() as u32).max(40);
        let thumb_x = inner_x + (tile_w - thumb_w) * scroll as u32
            / (APPS.len() - VISIBLE_ROWS).max(1) as u32;
        draw_rect(fb, thumb_x, track_y, thumb_w, bar_h, theme.label_text_dim);
    }
}

fn header_y_offset() -> u32 {
    28 + 4 + 4
}

fn blend(bg: u32, accent: u32, alpha: u32) -> u32 {
    theme::blend_colors(accent, bg, alpha)
}

/// Launch an app by name (used by the shell `app <name>` command and the
/// desktop's own automation paths).
pub fn launch_by_name(name: &str) -> bool {
    for app in APPS {
        if app.name.eq_ignore_ascii_case(name) {
            (app.launch)();
            return true;
        }
    }
    false
}

// ─── Launch helpers ───────────────────────────────────────────────────

/// Focus an existing window with this exact title, else run `create`.
fn focus_or_create(title: &str, _w: u32, _h: u32, create: impl FnOnce()) {
    let count = window::count();
    for id in 1..=count as u32 {
        if window::exists(id) && window::get_title(id) == title {
            window::focus(id);
            return;
        }
    }
    create();
}
