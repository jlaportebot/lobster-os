//! Login greeter (C5): a centered card listing users, shown at boot only when
//! more than one account exists. Runs its own small poll loop in the boot path
//! (before the desktop is composited), so no WM input hooks are needed.

use crate::display::fb::{self, FramebufferInfo};
use crate::display::graphics as g;
use crate::usb::hid::{self, HidEvent};
use crate::wm::theme;
use alloc::string::String;
use alloc::vec::Vec;

const CARD_W: u32 = 320;
const TITLE_H: u32 = 56;
const ROW_H: u32 = 36;
const ROW_PAD: u32 = 12;

/// True when the greeter should be shown (more than one login account).
pub fn needed() -> bool {
    crate::users::login_users().len() > 1
}

fn card_rect(info: &FramebufferInfo, rows: usize) -> (u32, u32, u32, u32) {
    let h = TITLE_H + rows as u32 * ROW_H + ROW_PAD * 2;
    (
        (info.width.saturating_sub(CARD_W)) / 2,
        (info.height.saturating_sub(h)) / 2,
        CARD_W,
        h,
    )
}

fn row_rect(card: (u32, u32, u32, u32), i: usize) -> (u32, u32, u32, u32) {
    (
        card.0 + ROW_PAD,
        card.1 + TITLE_H + i as u32 * ROW_H,
        card.2 - ROW_PAD * 2,
        ROW_H - 4,
    )
}

/// Draw the full-screen greeter with `sel` highlighted.
pub fn draw(info: &FramebufferInfo, users: &[String], sel: usize) {
    let t = theme::current();
    g::fill_rect(info, 0, 0, info.width, info.height, t.desktop_bg);
    let card = card_rect(info, users.len());
    g::fill_rounded_rect(info, card.0, card.1, card.2, card.3, 8, t.win_bg);
    g::draw_rect_outline(info, card.0, card.1, card.2, card.3, t.menu_border);
    g::draw_text_centered(
        info,
        card.0 + card.2 / 2,
        card.1 + 14,
        "LobsterOS",
        t.label_text,
        t.win_bg,
    );
    g::draw_text_centered(
        info,
        card.0 + card.2 / 2,
        card.1 + 34,
        "Choose a user to log in",
        t.label_text_dim,
        t.win_bg,
    );
    for (i, name) in users.iter().enumerate() {
        let r = row_rect(card, i);
        let bg = if i == sel {
            t.menu_item_hover
        } else {
            t.menu_bg
        };
        g::fill_rounded_rect(info, r.0, r.1, r.2, r.3, 6, bg);
        g::draw_text(info, r.0 + 14, r.1 + (r.3 - 8) / 2, name, t.label_text, bg);
    }
}

/// Map a click to a user row index.
pub fn handle_click(info: &FramebufferInfo, rows: usize, x: u32, y: u32) -> Option<usize> {
    let card = card_rect(info, rows);
    (0..rows).find(|&i| {
        let r = row_rect(card, i);
        x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
    })
}

/// Show the greeter and block until a user is chosen (click, or Up/Down +
/// Enter). Logs the user in via `users::login`.
pub fn run() {
    let info = fb::get_info();
    if info.width == 0 {
        return;
    }
    let users: Vec<String> = crate::users::login_users();
    let mut sel = 0usize;
    let (mut cx, mut cy) = crate::wm::input::cursor_pos();
    let mut left = false;
    crate::println!("[greeter] {} users: waiting for login", users.len());
    draw(&info, &users, sel);
    crate::wm::compositor::draw_software_cursor(&info);
    loop {
        let mut redraw = false;
        let mut moved = false;
        let mut chosen: Option<usize> = None;
        while let Some(ev) = hid::poll() {
            match ev {
                HidEvent::MouseMove { dx, dy } => {
                    cx = (cx as i32 + dx as i32).clamp(0, info.width as i32 - 1) as u32;
                    cy = (cy as i32 + dy as i32).clamp(0, info.height as i32 - 1) as u32;
                    moved = true;
                    if let Some(i) = handle_click(&info, users.len(), cx, cy) {
                        if i != sel {
                            sel = i;
                            redraw = true;
                        }
                    }
                }
                HidEvent::MouseButton { left: l, .. } => {
                    if l && !left {
                        chosen = handle_click(&info, users.len(), cx, cy);
                    }
                    left = l;
                }
                HidEvent::KeyPress { scancode, .. } => match scancode {
                    0x50 | 0x52 if sel > 0 => {
                        sel -= 1;
                        redraw = true;
                    }
                    0x4F | 0x51 if sel + 1 < users.len() => {
                        sel += 1;
                        redraw = true;
                    }
                    0x28 => chosen = Some(sel),
                    _ => {}
                },
                _ => {}
            }
        }
        if let Some(i) = chosen {
            crate::wm::compositor::restore_software_cursor(&info);
            // Hand the pointer position to the WM so its cursor stays in
            // sync with the device's absolute coordinates after login.
            crate::wm::input::set_cursor_pos(cx, cy);
            crate::users::login(&users[i]);
            return;
        }
        if moved || redraw {
            crate::wm::compositor::restore_software_cursor(&info);
            if redraw {
                draw(&info, &users, sel);
            }
            crate::wm::input::set_cursor_pos(cx, cy);
            crate::wm::compositor::draw_software_cursor(&info);
        }
        core::hint::spin_loop();
    }
}
