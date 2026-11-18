//! Desktop Icons — renders icons on the desktop background and handles clicks
//!
//! Icons are arranged in a grid on the left side of the screen. Each icon
//! has an app tile, a label, and an action (focus existing window or
//! create a new one).

use crate::display::fb;
use crate::display::graphics;
use crate::display::icons::{self, Icon};
use crate::println;
use crate::wm::theme;
use crate::wm::window;

/// Maximum number of desktop icons
const MAX_ICONS: usize = 12;

/// Icon dimensions: the clickable cell is 64 px wide, the tile inside is 48.
const ICON_SIZE: u32 = 64;
/// Edge of the app tile drawn inside the cell.
const TILE_SIZE: u32 = 48;
/// Tile inset from the cell's left edge (centres it).
const TILE_INSET_X: u32 = (ICON_SIZE - TILE_SIZE) / 2;
/// Tile inset from the cell's top edge.
const TILE_INSET_Y: u32 = 6;
/// Corner radius of the selection and hover highlights.
const HIGHLIGHT_RADIUS: u32 = 8;
/// Highlight padding around the cell for selection and hover.
const SELECT_PAD: u32 = 4;
const HOVER_PAD: u32 = 2;
/// Highlight opacity for the selected and hovered states.
const SELECT_ALPHA: u8 = 0x70;
const HOVER_ALPHA: u8 = 0x38;
/// Highlight tint (selection uses the theme's focus colour instead).
const HOVER_TINT: u32 = 0xFFFF_FFFF;
/// Background argument that makes `draw_text` leave the pixels under glyphs alone,
/// so the highlight shows through the label.
const LABEL_TRANSPARENT_BG: u32 = 0;
/// Width in pixels of one label character in the built-in font.
const LABEL_CHAR_W: u32 = 8;
const ICON_SPACING: u32 = 16;
const ICON_LABEL_H: u32 = 18;
const ICON_MARGIN_LEFT: u32 = 32;
const ICON_MARGIN_TOP: u32 = 40;

/// Desktop icon entry
#[derive(Clone, Copy)]
struct DesktopIcon {
    label: [u8; 24],
    label_len: usize,
    icon: Icon,
    action: IconAction,
    x: u32,
    y: u32,
}

/// What happens when an icon is clicked
#[derive(Clone, Copy)]
enum IconAction {
    /// Focus or create a terminal window
    Terminal,
    /// Focus or create a file manager window
    FileManager,
    /// Focus or create a calculator window
    Calculator,
    /// Focus or create a system monitor window
    SysMonitor,
    /// Focus or create a process manager window
    ProcessManager,
    /// Open the settings panel
    Settings,
}

impl DesktopIcon {
    const ZERO: DesktopIcon = DesktopIcon {
        label: [0u8; 24],
        label_len: 0,
        icon: Icon::AppGeneric,
        action: IconAction::Terminal,
        x: 0,
        y: 0,
    };
}

/// Global icon list
use crate::sync::spinlock::Spinlock;
static ICONS: Spinlock<[DesktopIcon; MAX_ICONS]> = Spinlock::new([DesktopIcon::ZERO; MAX_ICONS]);
static ICON_COUNT: Spinlock<usize> = Spinlock::new(0);
/// Currently selected icon (single selection; None = none)
static SELECTED: Spinlock<Option<usize>> = Spinlock::new(None);
/// Icon under the cursor (hover)
static HOVERED: Spinlock<Option<usize>> = Spinlock::new(None);

/// Select an icon (single selection).
pub fn select(idx: Option<usize>) {
    *SELECTED.lock() = idx;
}

/// Currently selected icon index.
pub fn selected() -> Option<usize> {
    *SELECTED.lock()
}

/// Update hover state from cursor movement.
pub fn update_hover(cx: u32, cy: u32) {
    let hit = hit_test(cx, cy);
    let mut hovered = HOVERED.lock();
    if *hovered != hit {
        *hovered = hit;
        drop(hovered);
        crate::wm::compositor::mark_full_redraw();
    }
}

/// Initialize desktop icons
pub fn init() {
    let mut icons = ICONS.lock();
    let mut count = 0;

    let entries: &[(&str, IconAction)] = &[
        ("Terminal", IconAction::Terminal),
        ("Files", IconAction::FileManager),
        ("Calc", IconAction::Calculator),
        ("System", IconAction::SysMonitor),
        ("Procs", IconAction::ProcessManager),
        ("Settings", IconAction::Settings),
    ];

    let cols = 1;
    let row_h = ICON_SIZE + ICON_LABEL_H + ICON_SPACING;

    for (i, (label, action)) in entries.iter().enumerate() {
        if count >= MAX_ICONS {
            break;
        }
        let icon = &mut icons[count];
        icon.label = [0u8; 24];
        let label_bytes = label.as_bytes();
        let len = label_bytes.len().min(24);
        icon.label[..len].copy_from_slice(&label_bytes[..len]);
        icon.label_len = len;
        icon.icon = icons::for_app(label);
        icon.action = *action;
        icon.x = ICON_MARGIN_LEFT;
        icon.y = ICON_MARGIN_TOP + (i as u32) * row_h;
        let _ = cols;
        count += 1;
    }

    *ICON_COUNT.lock() = count;
    println!("[desktop] {} desktop icons initialized", count);
}

/// Translucent rounded highlight behind a tile and its label.
fn draw_highlight(
    fb: &fb::FramebufferInfo,
    x: u32,
    y: u32,
    selected: bool,
    hovered: bool,
    focus: u32,
) {
    let (pad, alpha, tint) = if selected {
        (SELECT_PAD, SELECT_ALPHA, focus)
    } else if hovered {
        (HOVER_PAD, HOVER_ALPHA, HOVER_TINT)
    } else {
        return;
    };
    let rect = (
        x.saturating_sub(pad),
        y.saturating_sub(pad),
        ICON_SIZE + 2 * pad,
        ICON_SIZE + ICON_LABEL_H + 2 * pad,
    );
    graphics::blend_rounded_rect(fb, rect, HIGHLIGHT_RADIUS, tint, alpha);
}

/// Draw all desktop icons to the framebuffer
pub fn draw(fb: &fb::FramebufferInfo) {
    let icons = ICONS.lock();
    let count = *ICON_COUNT.lock();
    let theme = theme::current();

    let sel = *SELECTED.lock();
    let hov = *HOVERED.lock();

    for i in 0..count {
        let icon = &icons[i];
        let x = icon.x;
        let y = icon.y;

        draw_highlight(
            fb,
            x,
            y,
            sel == Some(i),
            hov == Some(i),
            theme.input_border_focused,
        );
        icons::draw(fb, icon.icon, TILE_SIZE, x + TILE_INSET_X, y + TILE_INSET_Y);

        // Draw label below icon
        let label = core::str::from_utf8(&icon.label[..icon.label_len]).unwrap_or("");
        let label_w = (label.len() as u32) * LABEL_CHAR_W;
        let label_x = x + ICON_SIZE / 2 - label_w / 2;
        let label_y = y + ICON_SIZE + 2;
        graphics::draw_text(
            fb,
            label_x,
            label_y,
            label,
            theme.title_text,
            LABEL_TRANSPARENT_BG,
        );
    }
}

/// Check if a point is on an icon. Returns the icon index if hit.
pub fn hit_test(cx: u32, cy: u32) -> Option<usize> {
    let icons = ICONS.lock();
    let count = *ICON_COUNT.lock();

    for i in 0..count {
        let icon = &icons[i];
        if cx >= icon.x
            && cx < icon.x + ICON_SIZE
            && cy >= icon.y
            && cy < icon.y + ICON_SIZE + ICON_LABEL_H
        {
            return Some(i);
        }
    }
    None
}

/// Handle a click on a desktop icon
pub fn handle_click(icon_idx: usize) {
    let icons = ICONS.lock();
    if icon_idx >= *ICON_COUNT.lock() {
        return;
    }
    let action = icons[icon_idx].action;

    // Selection semantics: first click selects; clicking the already-selected
    // icon opens it (single-click select + click-again to open).
    let was_selected = *SELECTED.lock() == Some(icon_idx);
    *SELECTED.lock() = Some(icon_idx);
    if !was_selected {
        return; // select only on first click
    }

    match action {
        IconAction::Terminal => {
            // Focus the existing terminal window, or create a real one
            let id = crate::apps::terminal::open_terminal_window(100, 80, 720, 500);
            if let Some(id) = id {
                window::focus(id);
            }
        }
        IconAction::FileManager => {
            let id = window::create(100, 80, 500, 400, "File Manager");
            if let Some(id) = id {
                window::focus(id);
            }
        }
        IconAction::Calculator => {
            let id = window::create(100, 80, 300, 400, "Calculator");
            if let Some(id) = id {
                window::focus(id);
            }
        }
        IconAction::SysMonitor => {
            let id = window::create(100, 80, 420, 280, "System Monitor");
            if let Some(id) = id {
                window::focus(id);
            }
        }
        IconAction::ProcessManager => {
            let id = window::create(100, 80, 440, 280, "Process Manager");
            if let Some(id) = id {
                window::focus(id);
            }
        }
        IconAction::Settings => {
            let id = window::create(100, 80, 400, 300, "Settings");
            if let Some(id) = id {
                window::focus(id);
            }
        }
    }
}
