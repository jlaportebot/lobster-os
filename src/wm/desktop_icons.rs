//! Desktop Icons — renders icons on the desktop background and handles clicks
//!
//! Icons are arranged in a grid on the left side of the screen. Each icon
//! has a colored glyph, a label, and an action (focus existing window or
//! create a new one).

use crate::display::fb;
use crate::display::graphics;
use crate::println;
use crate::wm::theme;
use crate::wm::window;

/// Maximum number of desktop icons
const MAX_ICONS: usize = 12;

/// Icon dimensions
const ICON_SIZE: u32 = 64;
const ICON_SPACING: u32 = 16;
const ICON_LABEL_H: u32 = 18;
const ICON_MARGIN_LEFT: u32 = 32;
const ICON_MARGIN_TOP: u32 = 40;

/// Desktop icon entry
#[derive(Clone, Copy)]
struct DesktopIcon {
    label: [u8; 24],
    label_len: usize,
    glyph: char,
    color: u32, // icon background color (ARGB)
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
        glyph: ' ',
        color: 0,
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
    *HOVERED.lock() = hit_test(cx, cy);
}

/// Initialize desktop icons
pub fn init() {
    let mut icons = ICONS.lock();
    let mut count = 0;

    let entries: &[(&str, char, u32, IconAction)] = &[
        ("Terminal", '>', 0xFF1A1A2E, IconAction::Terminal),
        ("Files", 'F', 0xFF2D5B8E, IconAction::FileManager),
        ("Calc", 'C', 0xFF8E2D5B, IconAction::Calculator),
        ("System", 'S', 0xFF5B8E2D, IconAction::SysMonitor),
        ("Procs", 'P', 0xFF8E5B2D, IconAction::ProcessManager),
        ("Settings", '*', 0xFF5B2D8E, IconAction::Settings),
    ];

    let cols = 1;
    let row_h = ICON_SIZE + ICON_LABEL_H + ICON_SPACING;

    for (i, (label, glyph, color, action)) in entries.iter().enumerate() {
        if count >= MAX_ICONS {
            break;
        }
        let icon = &mut icons[count];
        icon.label = [0u8; 24];
        let label_bytes = label.as_bytes();
        let len = label_bytes.len().min(24);
        icon.label[..len].copy_from_slice(&label_bytes[..len]);
        icon.label_len = len;
        icon.glyph = *glyph;
        icon.color = *color;
        icon.action = *action;
        icon.x = ICON_MARGIN_LEFT;
        icon.y = ICON_MARGIN_TOP + (i as u32) * row_h;
        let _ = cols;
        count += 1;
    }

    *ICON_COUNT.lock() = count;
    println!("[desktop] {} desktop icons initialized", count);
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

        // Selection highlight (behind icon + label)
        if sel == Some(i) {
            graphics::fill_rounded_rect(
                fb,
                x.saturating_sub(4),
                y.saturating_sub(4),
                ICON_SIZE + 8,
                ICON_SIZE + ICON_LABEL_H + 6,
                6,
                theme.input_border_focused,
            );
        } else if hov == Some(i) {
            // Subtle hover tint
            graphics::fill_rounded_rect(
                fb,
                x.saturating_sub(2),
                y.saturating_sub(2),
                ICON_SIZE + 4,
                ICON_SIZE + ICON_LABEL_H + 2,
                6,
                0x3388C0D0,
            );
        }

        // Icon background: rounded rectangle effect (solid for now)
        // Draw border (slightly lighter)
        graphics::draw_rect(fb, x, y, ICON_SIZE, ICON_SIZE, 0xFF333333);
        // Fill with icon color
        graphics::fill_rect(fb, x + 2, y + 2, ICON_SIZE - 4, ICON_SIZE - 4, icon.color);

        // Draw glyph (single character centered)
        let glyph_str: [u8; 1] = [icon.glyph as u8];
        let glyph_str = core::str::from_utf8(&glyph_str).unwrap_or("?");
        let text_x = x + ICON_SIZE / 2 - 4;
        let text_y = y + ICON_SIZE / 2 - 8;
        graphics::draw_text(fb, text_x, text_y, glyph_str, 0xFFFFFFFF, icon.color);

        // Draw label below icon
        let label = core::str::from_utf8(&icon.label[..icon.label_len]).unwrap_or("");
        let label_w = (label.len() as u32) * 8;
        let label_x = x + ICON_SIZE / 2 - label_w / 2;
        let label_y = y + ICON_SIZE + 2;
        graphics::draw_text(
            fb,
            label_x,
            label_y,
            label,
            theme.title_text,
            theme.desktop_bg,
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
            // Try to focus existing terminal window, or create one
            let id = window::create(100, 80, 720, 500, "Terminal");
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
