//! The immediate-mode drawing context shared by every control.
//!
//! An app builds a [`Ui`] once per frame over its window surface, draws
//! controls with the methods in this module tree and drops it. Each
//! interactive control registers a hit region (tag = the caller's `key`),
//! so presses reach the app through the normal hit-region router. Hover and
//! pressed looks come from the live pointer, focus from [`ControlState`].

use super::drag::Track;
use super::Rect;
use crate::display::fb::FramebufferInfo;
use crate::display::text::{self, Face, TextStyle};
use crate::wm::hit_region::{self, HitKind};
use crate::wm::theme;
use alloc::vec::Vec;

/// Key value meaning "no control".
pub const KEY_NONE: u32 = 0;

/// Interaction look of one control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Normal,
    Hover,
    Pressed,
    Disabled,
}

/// State that must outlive a frame: keyboard focus, the focus order and the
/// slider/scrollbar tracks laid out in the last frame.
#[derive(Default)]
pub struct ControlState {
    /// Key of the control holding keyboard focus ([`KEY_NONE`] for none).
    pub focus: u32,
    /// Whether the focus ring is shown (after keyboard use, not mouse use).
    pub focus_visible: bool,
    order: Vec<u32>,
    building: Vec<u32>,
    tracks: Vec<(u32, Track)>,
}

impl ControlState {
    /// Empty state with nothing focused.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            focus: KEY_NONE,
            focus_visible: false,
            order: Vec::new(),
            building: Vec::new(),
            tracks: Vec::new(),
        }
    }

    /// Move focus to the next (or previous) control in drawing order,
    /// wrapping around. Returns the newly focused key.
    pub fn move_focus(&mut self, backwards: bool) -> u32 {
        self.focus_visible = true;
        if self.order.is_empty() {
            return KEY_NONE;
        }
        let len = self.order.len();
        let at = self.order.iter().position(|k| *k == self.focus);
        let next = match (at, backwards) {
            (None, false) => 0,
            (None, true) => len - 1,
            (Some(i), false) => (i + 1) % len,
            (Some(i), true) => (i + len - 1) % len,
        };
        self.focus = self.order[next];
        self.focus
    }

    /// Slider or scrollbar track registered under `key` in the last frame.
    #[must_use]
    pub fn track(&self, key: u32) -> Option<Track> {
        self.tracks.iter().find(|(k, _)| *k == key).map(|(_, t)| *t)
    }
}

/// Per-frame drawing context; see the module docs.
pub struct Ui<'a> {
    pub(super) clip: FramebufferInfo,
    full: FramebufferInfo,
    pub(super) win_id: u32,
    origin: (i32, i32),
    viewport: Rect,
    shift: (i32, i32),
    pointer: Option<(i32, i32)>,
    pressed: bool,
    modal: bool,
    pub(super) popup_requested: bool,
    pub(super) state: &'a mut ControlState,
}

impl<'a> Ui<'a> {
    /// Start a frame for window `win_id` whose client area starts at screen
    /// position `client` and is `client_size` px. The view initially covers
    /// the whole client area with no scroll shift.
    pub fn new(
        target: &FramebufferInfo,
        win_id: u32,
        client: (u32, u32),
        client_size: (u32, u32),
        state: &'a mut ControlState,
    ) -> Self {
        let (cx, cy) = crate::wm::input::cursor_pos();
        let origin = (client.0 as i32, client.1 as i32);
        let pointer = (cx as i32 - origin.0, cy as i32 - origin.1);
        let viewport = Rect::new(0, 0, client_size.0, client_size.1);
        state.building.clear();
        state.tracks.clear();
        super::drag::watch_hover(win_id);
        let mut ui = Self {
            clip: *target,
            full: *target,
            win_id,
            origin,
            viewport,
            shift: (0, 0),
            pointer: viewport.contains(pointer.0, pointer.1).then_some(pointer),
            pressed: crate::wm::input::mouse_buttons().0,
            modal: false,
            popup_requested: false,
            state,
        };
        ui.set_view(viewport, (0, 0));
        ui
    }

    /// Restrict drawing and hit regions to `viewport` (client coordinates)
    /// and translate every following rectangle by `shift`.
    pub fn set_view(&mut self, viewport: Rect, shift: (i32, i32)) {
        self.viewport = viewport;
        self.shift = shift;
        let screen = viewport.offset(self.origin.0, self.origin.1);
        let (x0, y0) = (screen.x.max(0) as u32, screen.y.max(0) as u32);
        let (x1, y1) = (screen.right().max(0) as u32, screen.bottom().max(0) as u32);
        self.clip = self.full.clipped(x0, y0, x1, y1);
    }

    /// While set, controls ignore the pointer so only the open popup reacts.
    pub fn set_modal(&mut self, modal: bool) {
        self.modal = modal;
    }

    /// End the frame, committing the focus order.
    pub fn finish(self) {
        if !self.popup_requested {
            crate::wm::popup_list::on_window_destroyed(self.win_id);
        }
        self.state.order = core::mem::take(&mut self.state.building);
    }

    /// Whether the focus ring should be drawn on `key`.
    #[must_use]
    pub fn has_focus(&self, key: u32) -> bool {
        self.state.focus == key && self.state.focus_visible
    }

    /// Screen rectangle of a view rectangle (view -> client -> screen).
    #[must_use]
    pub fn to_screen(&self, r: Rect) -> Rect {
        r.offset(self.origin.0 + self.shift.0, self.origin.1 + self.shift.1)
    }

    /// The current viewport in client coordinates.
    #[must_use]
    pub fn viewport_rect(&self) -> Rect {
        self.viewport
    }

    /// The viewport in view coordinates (undoing the scroll shift), for
    /// controls that must cover everything currently visible.
    #[must_use]
    pub fn view_extent(&self) -> Rect {
        self.viewport.offset(-self.shift.0, -self.shift.1)
    }

    pub(super) fn to_client(&self, r: Rect) -> Rect {
        r.offset(self.shift.0, self.shift.1)
    }

    /// Part of `r` that is visible inside the viewport, in client coordinates.
    #[must_use]
    pub fn visible(&self, r: Rect) -> Option<Rect> {
        self.to_client(r).intersect(&self.viewport)
    }

    /// Interaction look for a control occupying `r`.
    #[must_use]
    pub fn look(&self, r: Rect, enabled: bool) -> Look {
        if !enabled {
            return Look::Disabled;
        }
        let over = self.pointer.is_some_and(|(px, py)| {
            self.to_client(r).contains(px, py) && self.viewport.contains(px, py)
        });
        match (over && !self.modal, self.pressed) {
            (true, true) => Look::Pressed,
            (true, false) => Look::Hover,
            _ => Look::Normal,
        }
    }

    /// Register `r` as a hit region with tag `key`, and optionally as a
    /// keyboard-focus stop. Off-screen controls are skipped.
    pub fn register(&mut self, key: u32, r: Rect, kind: HitKind, focusable: bool) {
        let Some(vis) = self.visible(r) else { return };
        let (x, y) = (vis.x as u32, vis.y as u32);
        hit_region::register(self.win_id, x, y, vis.w, vis.h, key, kind);
        if focusable {
            self.state.building.push(key);
        }
    }

    pub(super) fn add_track(&mut self, key: u32, track: Track) {
        self.state.tracks.push((key, track));
    }

    // ── primitives (colours are 0xAARRGGBB) ──────────────────────────

    pub(super) fn px(&self, r: Rect) -> (u32, u32, u32, u32) {
        let s = self.to_screen(r);
        (s.x.max(0) as u32, s.y.max(0) as u32, s.w, s.h)
    }

    /// Opaque rectangle.
    pub fn fill(&self, r: Rect, color: u32) {
        let (x, y, w, h) = self.px(r);
        crate::display::graphics::draw_rect(&self.clip, x, y, w, h, color);
    }

    /// Opaque rounded rectangle.
    pub fn rounded(&self, r: Rect, radius: u32, color: u32) {
        let (x, y, w, h) = self.px(r);
        crate::display::graphics::fill_rounded_rect(&self.clip, x, y, w, h, radius, color);
    }

    /// Translucent rounded rectangle.
    pub fn blend(&self, r: Rect, radius: u32, color: u32, alpha: u8) {
        crate::display::graphics::blend_rounded_rect(&self.clip, self.px(r), radius, color, alpha);
    }

    /// One-pixel outline.
    pub fn outline(&self, r: Rect, color: u32) {
        let (x, y, w, h) = self.px(r);
        crate::display::graphics::draw_rect_outline(&self.clip, x, y, w, h, color);
    }

    /// Rounded outline `width` px thick: an outer rounded fill with the
    /// inner area refilled in `inner`.
    pub fn ring(&self, r: Rect, radius: u32, width: u32, color: u32, inner: u32) {
        self.rounded(r, radius, color);
        self.rounded(r.inset(width), radius.saturating_sub(width), inner);
    }

    /// Filled disc centred in `r`.
    pub fn disc(&self, r: Rect, color: u32) {
        let (x, y, w, h) = self.px(r);
        let radius = w.min(h) / 2;
        crate::display::graphics::fill_circle(&self.clip, x + w / 2, y + h / 2, radius, color);
    }

    /// Draw `s` with its line box at (`x`, `y`); returns the advance.
    pub fn text(&self, x: i32, y: i32, s: &str, style: &TextStyle) -> u32 {
        let r = self.to_screen(Rect::new(x, y, 1, 1));
        text::draw(&self.clip, r.x.max(0) as u32, r.y.max(0) as u32, s, style)
    }

    /// Draw `s` truncated with an ellipsis to `max_w` px.
    pub fn text_fitted(&self, x: i32, y: i32, max_w: u32, s: &str, style: &TextStyle) {
        let r = self.to_screen(Rect::new(x, y, 1, 1));
        let (px, py) = (r.x.max(0) as u32, r.y.max(0) as u32);
        text::draw_fitted(&self.clip, px, py, max_w, s, style);
    }

    /// Draw `s` centred horizontally and vertically in `r`.
    pub fn text_centered(&self, r: Rect, s: &str, style: &TextStyle) {
        let top = r.y + (r.h.saturating_sub(text::line_height(style)) / 2) as i32;
        let x = r.x + (r.w.saturating_sub(text::width(s, style)) / 2) as i32;
        self.text_fitted(x, top, r.w, s, style);
    }

    /// Draw a symbolic glyph centred in `r`.
    pub fn glyph(&self, glyph: crate::display::icons::Glyph, size: u32, r: Rect, color: u32) {
        let x = r.x + (r.w.saturating_sub(size) / 2) as i32;
        let y = r.y + (r.h.saturating_sub(size) / 2) as i32;
        let s = self.to_screen(Rect::new(x, y, size, size));
        let (px, py) = (s.x.max(0) as u32, s.y.max(0) as u32);
        crate::display::icons::draw_glyph(&self.clip, glyph, size, px, py, color);
    }
}

/// Style for control text: `px` is the size at the Medium text setting.
#[must_use]
pub fn style(face: Face, px: u32, color: u32) -> TextStyle {
    theme::chrome_text(face, px, color, MAX_LINE_H)
}

/// Tallest line box any control text may use.
const MAX_LINE_H: u32 = 30;

/// Text colour that stays readable on `background`.
#[must_use]
pub fn on_color(background: u32) -> u32 {
    let r = (background >> 16) & 0xFF;
    let g = (background >> 8) & 0xFF;
    let b = background & 0xFF;
    let luma = (r * 299 + g * 587 + b * 114) / 1000;
    if luma > LIGHT_BACKGROUND_LUMA {
        0xFF1B1F27
    } else {
        0xFFFFFFFF
    }
}

/// Backgrounds brighter than this get dark text.
const LIGHT_BACKGROUND_LUMA: u32 = 150;
