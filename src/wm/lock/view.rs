//! Drawing of the lock/login overlay: dimmed wallpaper, clock, avatar and
//! password field (or the account list in the greeter).

use super::form::Status;
use super::layout::{self, Rect};
use super::overlay::{Mode, Overlay, Stage};
use crate::display::fb::FramebufferInfo;
use crate::display::graphics::{
    blend_rect, blend_rounded_rect, draw_circle, fill_circle, fill_rounded_rect,
};
use crate::display::text::{self, Face, TextStyle};
use crate::users::auth::{AuthError, PasswordState};
use crate::users::throttle::Throttle;
use alloc::string::String;

/// Black laid over the wallpaper, 0..=255.
const DIM_ALPHA: u8 = 0xA0;
/// Primary text on the dimmed wallpaper.
const TEXT: u32 = 0xFFFF_FFFF;
/// Secondary text.
const TEXT_DIM: u32 = 0xFFC4_CAD4;
/// Error text.
const TEXT_ERROR: u32 = 0xFFFF_8A80;
/// Accent for the focus ring and the button.
const ACCENT: u32 = 0xFF3B_82F6;
/// Fill of the password field and list rows.
const PANEL: u32 = 0xFFFF_FFFF;
/// Panel opacity.
const PANEL_ALPHA: u8 = 0x30;
/// Opacity of the highlighted list row.
const PANEL_ALPHA_SELECTED: u8 = 0x58;
/// Corner radius of fields, buttons and rows.
const RADIUS: u32 = 8;
/// Radius of one password dot.
const DOT_RADIUS: u32 = 4;
/// Distance between password dot centres.
const DOT_STEP: u32 = 14;
/// Left padding inside the field.
const FIELD_PAD: u32 = 16;
/// Avatar palette; the account name picks one.
const AVATAR_COLORS: [u32; 6] = [
    0xFF2E_7D9A,
    0xFF8E_4FA8,
    0xFFC0_6A2B,
    0xFF3F_8F5B,
    0xFFB2_4A5E,
    0xFF55_6FB5,
];
/// Milliseconds per second, for the countdown.
const MS_PER_SECOND: u64 = 1000;

/// Clock text for the overlay: `(time, date)`.
#[must_use]
pub fn clock_lines(uptime_ms: u64) -> (String, String) {
    use crate::wm::panel_layout as pl;
    match crate::clock::local_at(uptime_ms) {
        Some(now) => (
            pl::time_text(now.hour, now.minute, crate::clock::use_24_hour()),
            pl::date_text(now.weekday_name(), now.day, now.month_name()),
        ),
        None => (String::from("--:--"), String::from("clock not set")),
    }
}

fn style(face: Face, size: u32, color: u32) -> TextStyle {
    TextStyle { face, size, color }
}

fn avatar_color(name: &str) -> u32 {
    let sum = name
        .bytes()
        .fold(0usize, |a, b| a.wrapping_add(usize::from(b)));
    AVATAR_COLORS[sum % AVATAR_COLORS.len()]
}

fn draw_avatar(fb: &FramebufferInfo, cx: u32, cy: u32, r: u32, name: &str, px: u32) {
    fill_circle(fb, cx, cy, r, avatar_color(name));
    draw_circle(fb, cx, cy, r, 0xFFFF_FFFF);
    let initial: String = name.chars().take(1).flat_map(char::to_uppercase).collect();
    let face = style(Face::UiBold, px, TEXT);
    let top = cy.saturating_sub(text::line_height(&face) / 2);
    text::draw_centered(fb, cx, top, &initial, &face);
}

fn draw_clock(fb: &FramebufferInfo, center_x: u32, time_y: u32, date_y: u32, uptime_ms: u64) {
    let (time, date) = clock_lines(uptime_ms);
    let big = style(Face::Mono, 26, TEXT);
    text::draw_centered(fb, center_x, time_y, &time, &big);
    text::draw_centered(fb, center_x, date_y, &date, &style(Face::Ui, 20, TEXT_DIM));
}

fn draw_button(fb: &FramebufferInfo, r: Rect, label: &str) {
    fill_rounded_rect(fb, r.x, r.y, r.w, r.h, RADIUS, ACCENT);
    let face = style(Face::UiBold, 14, TEXT);
    let top = r.y + r.h.saturating_sub(text::line_height(&face)) / 2;
    text::draw_centered(fb, r.x + r.w / 2, top, label, &face);
}

fn draw_field(fb: &FramebufferInfo, r: Rect, o: &Overlay) {
    blend_rounded_rect(fb, (r.x, r.y, r.w, r.h), RADIUS, PANEL, PANEL_ALPHA);
    outline(fb, r, ACCENT);
    let face = style(Face::Ui, 14, TEXT_DIM);
    let mid = r.y + r.h / 2;
    if o.form().is_empty() {
        let hint = if o.needs_typing() {
            "Password"
        } else {
            "Press Enter to unlock"
        };
        let top = r.y + r.h.saturating_sub(text::line_height(&face)) / 2;
        text::draw(fb, r.x + FIELD_PAD, top, hint, &face);
        return;
    }
    let fit = ((r.w - 2 * FIELD_PAD) / DOT_STEP) as usize;
    for i in 0..o.form().len().min(fit) {
        let cx = r.x + FIELD_PAD + DOT_RADIUS + i as u32 * DOT_STEP;
        fill_circle(fb, cx, mid, DOT_RADIUS, TEXT);
    }
}

fn outline(fb: &FramebufferInfo, r: Rect, color: u32) {
    crate::display::graphics::draw_rect_outline(fb, r.x, r.y, r.w, r.h, color);
}

fn refusal_text(why: AuthError) -> &'static str {
    match why {
        AuthError::AccountLocked => "This account is locked",
        AuthError::AccountExpired | AuthError::PasswordExpired => "This account has expired",
        _ => "Cannot log in",
    }
}

/// Message shown under the field, if any, as `(text, is_error)`.
#[must_use]
pub fn message(o: &Overlay, cooldown_ms: u64) -> Option<(String, bool)> {
    if cooldown_ms > 0 {
        let secs = cooldown_ms.div_ceil(MS_PER_SECOND);
        return Some((
            alloc::format!("Too many attempts. Try again in {secs} s"),
            true,
        ));
    }
    match o.form().status() {
        Status::Wrong(left) => Some((
            alloc::format!("Incorrect password. {left} {} left", plural_try(left)),
            true,
        )),
        Status::Refused(why) => Some((String::from(refusal_text(why)), true)),
        Status::Idle => None,
    }
}

fn plural_try(n: u32) -> &'static str {
    if n == 1 {
        "try"
    } else {
        "tries"
    }
}

fn draw_password(fb: &FramebufferInfo, o: &Overlay, cooldown_ms: u64, uptime_ms: u64) {
    let screen = crate::display::fb::device_info();
    let p = layout::password(screen.width, screen.height);
    draw_clock(fb, p.center_x, p.time_y, p.date_y, uptime_ms);
    let name = o.current().map_or("", |a| a.name.as_str());
    draw_avatar(fb, p.center_x, p.avatar_cy, layout::AVATAR_RADIUS, name, 24);
    text::draw_centered(
        fb,
        p.center_x,
        p.name_y,
        name,
        &style(Face::UiBold, 20, TEXT),
    );
    draw_field(fb, p.field, o);
    let label = if o.mode() == Mode::Lock {
        "Unlock"
    } else {
        "Log in"
    };
    draw_button(fb, p.button, label);
    if let Some((line, is_error)) = message(o, cooldown_ms) {
        let color = if is_error { TEXT_ERROR } else { TEXT_DIM };
        text::draw_centered(
            fb,
            p.center_x,
            p.message_y,
            &line,
            &style(Face::Ui, 14, color),
        );
    }
    if o.form().caps() {
        let face = style(Face::Ui, 13, TEXT_DIM);
        text::draw_centered(fb, p.center_x, p.caps_y, "Caps Lock is on", &face);
    }
}

fn password_note(state: PasswordState) -> &'static str {
    match state {
        PasswordState::None => "No password",
        PasswordState::Set => "Password required",
        PasswordState::Locked => "Account locked",
    }
}

fn draw_list(fb: &FramebufferInfo, o: &Overlay, uptime_ms: u64) {
    let screen = crate::display::fb::device_info();
    let (w, h) = (screen.width, screen.height);
    let p = layout::password(w, h);
    draw_clock(fb, p.center_x, p.time_y, p.date_y, uptime_ms);
    let shown = layout::visible_range(o.accounts().len(), o.selected(), layout::max_rows(h));
    let title_y = layout::list_title_y(h, shown.len());
    let heading = style(Face::UiBold, 20, TEXT);
    text::draw_centered(fb, w / 2, title_y, "Choose a user", &heading);
    for (slot, index) in shown.enumerate() {
        let r = layout::row_rect(w, h, o.accounts().len().min(layout::max_rows(h)), slot);
        let alpha = if index == o.selected() {
            PANEL_ALPHA_SELECTED
        } else {
            PANEL_ALPHA
        };
        blend_rounded_rect(fb, (r.x, r.y, r.w, r.h), RADIUS, PANEL, alpha);
        let account = &o.accounts()[index];
        let cy = r.y + r.h / 2;
        draw_avatar(
            fb,
            r.x + 16 + layout::ROW_AVATAR_RADIUS,
            cy,
            layout::ROW_AVATAR_RADIUS,
            &account.name,
            16,
        );
        let text_x = r.x + 16 + 2 * layout::ROW_AVATAR_RADIUS + 14;
        text::draw(
            fb,
            text_x,
            cy - 20,
            &account.name,
            &style(Face::UiBold, 16, TEXT),
        );
        let note = password_note(account.password);
        text::draw(fb, text_x, cy + 2, note, &style(Face::Ui, 12, TEXT_DIM));
    }
}

/// Paint the whole overlay into `target`: wallpaper, dim, then the stage.
pub fn draw(target: &FramebufferInfo, o: &Overlay, throttle: &Throttle, uptime_ms: u64) {
    crate::wm::wallpaper::paint_into(target);
    let screen = crate::display::fb::device_info();
    blend_rect(
        target,
        0,
        0,
        screen.width,
        screen.height,
        0xFF00_0000,
        DIM_ALPHA,
    );
    match o.stage() {
        Stage::Choose => draw_list(target, o, uptime_ms),
        Stage::Password => draw_password(target, o, throttle.remaining_ms(uptime_ms), uptime_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wm::lock::overlay::Account;

    fn lock_overlay() -> Overlay {
        Overlay::lock(Account {
            name: String::from("alice"),
            password: PasswordState::Set,
        })
    }

    #[test]
    fn no_message_before_any_attempt() {
        assert_eq!(message(&lock_overlay(), 0), None);
    }

    #[test]
    fn cooldown_message_counts_whole_seconds_up() {
        let (text, is_error) = message(&lock_overlay(), 29_001).expect("cooldown message");
        assert!(is_error);
        assert_eq!(text, "Too many attempts. Try again in 30 s");
    }

    #[test]
    fn wrong_password_message_names_the_tries_left() {
        let mut o = lock_overlay();
        let mut t = Throttle::new();
        o.submit(&mut t, 0, |_, _| Err(AuthError::InvalidPassword));
        let (text, _) = message(&o, 0).expect("wrong message");
        assert_eq!(text, "Incorrect password. 4 tries left");
    }

    #[test]
    fn avatar_colour_is_stable_per_name() {
        assert_eq!(avatar_color("alice"), avatar_color("alice"));
    }
}
