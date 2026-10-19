//! Context menu — right-click popup menu for desktop and windows
//!
//! Shows a vertical list of menu items with icons/labels,
//! handles hover highlighting and click dispatch.

use crate::display::fb::get_info;
use crate::display::graphics;
use crate::sync::spinlock::Spinlock;

/// Maximum items in a context menu
const MAX_ITEMS: usize = 12;

/// Maximum text length for menu items
const MAX_TEXT_LEN: usize = 40;

/// Menu item — an action or a separator
#[derive(Clone, Copy, Debug)]
pub struct MenuItem {
    /// Label text
    text: [u8; MAX_TEXT_LEN],
    text_len: usize,
    /// Action ID (0 = separator)
    action_id: u32,
    /// Hover state
    hovered: bool,
}

impl MenuItem {
    const fn empty() -> Self {
        MenuItem {
            text: [0; MAX_TEXT_LEN],
            text_len: 0,
            action_id: 0,
            hovered: false,
        }
    }

    fn label(&self) -> &str {
        core::str::from_utf8(&self.text[..self.text_len]).unwrap_or("")
    }

    fn set_label(&mut self, s: &str) {
        let len = s.len().min(MAX_TEXT_LEN);
        self.text[..len].copy_from_slice(&s.as_bytes()[..len]);
        self.text_len = len;
    }
}

/// Menu state
struct MenuState {
    visible: bool,
    x: u32,
    y: u32,
    items: [MenuItem; MAX_ITEMS],
    item_count: usize,
    width: u32,
    item_height: u32,
}

const ITEM_H: u32 = 24;
const MENU_PAD: u32 = 4;
const MENU_W: u32 = 180;

static MENU: Spinlock<MenuState> = Spinlock::new(MenuState {
    visible: false,
    x: 0,
    y: 0,
    items: [MenuItem::empty(); MAX_ITEMS],
    item_count: 0,
    width: MENU_W,
    item_height: ITEM_H,
});

/// Show a context menu at (x, y) with the given items
pub fn show(x: u32, y: u32, items: &[(&str, u32)]) {
    let mut menu = MENU.lock();
    menu.visible = true;
    menu.x = x;
    menu.y = y;
    menu.item_count = items.len().min(MAX_ITEMS);
    menu.width = MENU_W;

    for (i, (label, action_id)) in items.iter().enumerate().take(MAX_ITEMS) {
        menu.items[i] = MenuItem::empty();
        menu.items[i].set_label(label);
        menu.items[i].action_id = *action_id;
    }
}

/// Show the desktop context menu
pub fn show_desktop_menu(x: u32, y: u32) {
    show(
        x,
        y,
        &[
            ("New File", 30),
            ("New Folder", 31),
            ("---", 0),
            ("Open Terminal", 1),
            ("File Manager", 2),
            ("---", 0),
            ("Display Settings", 32),
            ("Change Wallpaper", 10),
            ("Toggle Dark/Light", 33),
            ("---", 0),
            ("Refresh", 34),
            ("About LobsterOS", 20),
        ],
    );
}

/// Hide the context menu
pub fn hide() {
    MENU.lock().visible = false;
}

/// Check if menu is visible
pub fn is_visible() -> bool {
    MENU.lock().visible
}

/// Handle a mouse click — returns Some(action_id) if an item was clicked
pub fn handle_click(cx: u32, cy: u32) -> Option<u32> {
    let menu = MENU.lock();
    if !menu.visible {
        return None;
    }

    // Check if click is inside menu bounds
    let menu_h = MENU_PAD * 2 + menu.item_count as u32 * ITEM_H;
    if cx < menu.x || cx >= menu.x + menu.width || cy < menu.y || cy >= menu.y + menu_h {
        hide();
        return None;
    }

    // Find which item was clicked
    let rel_y = cy.saturating_sub(menu.y + MENU_PAD);
    let item_idx = (rel_y / ITEM_H) as usize;

    if item_idx < menu.item_count {
        let action = menu.items[item_idx].action_id;
        if action > 0 {
            hide();
            return Some(action);
        }
    }

    hide();
    None
}

/// Handle mouse move for hover highlighting
pub fn handle_hover(cx: u32, cy: u32) {
    let mut menu = MENU.lock();
    if !menu.visible {
        return;
    }

    let menu_h = MENU_PAD * 2 + menu.item_count as u32 * ITEM_H;
    if cx < menu.x || cx >= menu.x + menu.width || cy < menu.y || cy >= menu.y + menu_h {
        for i in 0..menu.item_count {
            menu.items[i].hovered = false;
        }
        return;
    }

    let rel_y = cy.saturating_sub(menu.y + MENU_PAD);
    let item_idx = (rel_y / ITEM_H) as usize;

    for i in 0..menu.item_count {
        menu.items[i].hovered = i == item_idx;
    }
}

/// Draw the context menu
pub fn draw() {
    let menu = MENU.lock();
    if !menu.visible {
        return;
    }

    let fb = get_info();
    if fb.width == 0 {
        return;
    }

    let theme = crate::wm::theme::current();
    let menu_h = MENU_PAD * 2 + menu.item_count as u32 * ITEM_H;
    let x = menu.x;
    let y = menu.y;
    let w = menu.width;

    // Draw menu background with shadow
    graphics::draw_rect(&fb, x + 2, y + 2, w, menu_h, 0x80000000); // Shadow
    graphics::draw_rect(&fb, x, y, w, menu_h, theme.menu_bg); // Background
    graphics::draw_rect_outline(&fb, x, y, w, menu_h, theme.menu_border); // Border

    // Draw items
    for i in 0..menu.item_count {
        let item_y = y + MENU_PAD + i as u32 * ITEM_H;

        if menu.items[i].action_id == 0 {
            // Separator
            let sep_y = item_y + ITEM_H / 2;
            graphics::draw_rect(&fb, x + 8, sep_y, w - 16, 1, theme.menu_separator);
            continue;
        }

        if menu.items[i].hovered {
            // Hover highlight
            graphics::draw_rect(&fb, x + 1, item_y, w - 2, ITEM_H, theme.menu_item_hover);
        }

        // Item text
        let label = menu.items[i].label();
        let text_y = item_y + (ITEM_H - 14) / 2;
        graphics::draw_text(
            &fb,
            x + 12,
            text_y,
            label,
            theme.menu_item_text,
            theme.menu_bg,
        );
    }
}
