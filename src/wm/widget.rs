//! Widget toolkit — reusable UI components for LobsterOS windows
//!
//! Widgets are the building blocks of window content:
//! - Button: clickable action trigger
//! - Label: static text display
//! - TextInput: editable single-line text field
//! - Checkbox: boolean toggle
//! - ProgressBar: completion indicator
//! - ScrollBar: vertical/horizontal scroll control
//! - Panel: container with optional border
//!
//! Each widget has a `draw()` method that renders it into a window's
//! content buffer at a given position, using the active theme.

use crate::display::fb::FramebufferInfo;
use crate::display::graphics;
use crate::sync::spinlock::Spinlock;
use crate::wm::theme;

// ─── Widget Types ──────────────────────────────────────────────────────

/// Widget identifier (window-local)
pub type WidgetId = u32;

/// Maximum widgets per window
const MAX_WIDGETS: usize = 64;

/// Maximum text length for labels and buttons
const MAX_TEXT: usize = 128;

/// Widget state flags
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WidgetState {
    Normal,
    Hovered,
    Pressed,
    Disabled,
    Focused,
}

// ─── Widget Kinds ──────────────────────────────────────────────────────

/// Button widget
#[derive(Clone, Copy, Debug)]
pub struct Button {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub text: [u8; MAX_TEXT],
    pub text_len: usize,
    pub state: WidgetState,
    pub clicked: bool,
}

impl Button {
    pub const ZERO: Button = Button {
        x: 0,
        y: 0,
        w: 100,
        h: 28,
        text: [0u8; MAX_TEXT],
        text_len: 0,
        state: WidgetState::Normal,
        clicked: false,
    };

    pub fn text_str(&self) -> &str {
        core::str::from_utf8(&self.text[..self.text_len]).unwrap_or("")
    }

    pub fn set_text(&mut self, s: &str) {
        let len = s.len().min(MAX_TEXT);
        self.text[..len].copy_from_slice(s.as_bytes());
        self.text_len = len;
    }

    /// Draw the button into the framebuffer
    pub fn draw(&self, fb: &FramebufferInfo, offset_x: u32, offset_y: u32) {
        let theme = theme::current();
        let bg = match self.state {
            WidgetState::Normal => theme.btn_bg,
            WidgetState::Hovered => theme.btn_bg_hover,
            WidgetState::Pressed => theme.btn_bg_pressed,
            _ => theme.btn_bg,
        };
        let px = offset_x + self.x;
        let py = offset_y + self.y;

        // Button background (filled rect)
        fill_rect(fb, px, py, self.w, self.h, bg);

        // Border
        draw_rect_outline(fb, px, py, self.w, self.h, theme.btn_border);

        // Text (centered)
        if self.text_len > 0 {
            let text = self.text_str();
            let char_w = 8u32; // 8px per character in our font
            let text_w = text.chars().count() as u32 * char_w;
            let text_x = px + (self.w.saturating_sub(text_w)) / 2;
            let text_y = py + (self.h.saturating_sub(theme.font_size)) / 2;
            graphics::draw_text(fb, text_x, text_y, text, theme.btn_text, bg);
        }
    }

    /// Check if a point is inside this button
    pub fn hit_test(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Label widget (static text)
#[derive(Clone, Copy, Debug)]
pub struct Label {
    pub x: u32,
    pub y: u32,
    pub text: [u8; MAX_TEXT],
    pub text_len: usize,
    pub dim: bool,
}

impl Label {
    pub const ZERO: Label = Label {
        x: 0,
        y: 0,
        text: [0u8; MAX_TEXT],
        text_len: 0,
        dim: false,
    };

    pub fn text_str(&self) -> &str {
        core::str::from_utf8(&self.text[..self.text_len]).unwrap_or("")
    }

    pub fn set_text(&mut self, s: &str) {
        let len = s.len().min(MAX_TEXT);
        self.text[..len].copy_from_slice(s.as_bytes());
        self.text_len = len;
    }

    pub fn draw(&self, fb: &FramebufferInfo, offset_x: u32, offset_y: u32, bg_color: u32) {
        let theme = theme::current();
        let color = if self.dim {
            theme.label_text_dim
        } else {
            theme.label_text
        };
        let px = offset_x + self.x;
        let py = offset_y + self.y;
        if self.text_len > 0 {
            graphics::draw_text(fb, px, py, self.text_str(), color, bg_color);
        }
    }
}

/// Text input widget
#[derive(Clone, Copy, Debug)]
pub struct TextInput {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub text: [u8; MAX_TEXT],
    pub text_len: usize,
    pub cursor_pos: usize,
    pub state: WidgetState,
    pub scroll_offset: u32,
}

impl TextInput {
    pub const ZERO: TextInput = TextInput {
        x: 0,
        y: 0,
        w: 200,
        h: 28,
        text: [0u8; MAX_TEXT],
        text_len: 0,
        cursor_pos: 0,
        state: WidgetState::Normal,
        scroll_offset: 0,
    };

    pub fn text_str(&self) -> &str {
        core::str::from_utf8(&self.text[..self.text_len]).unwrap_or("")
    }

    pub fn set_text(&mut self, s: &str) {
        let len = s.len().min(MAX_TEXT);
        self.text[..len].copy_from_slice(s.as_bytes());
        self.text_len = len;
        self.cursor_pos = len;
    }

    /// Insert a character at cursor position
    pub fn insert_char(&mut self, c: u8) {
        if self.text_len >= MAX_TEXT - 1 {
            return;
        }
        // Shift characters right
        for i in (self.cursor_pos..self.text_len).rev() {
            self.text[i + 1] = self.text[i];
        }
        self.text[self.cursor_pos] = c;
        self.text_len += 1;
        self.cursor_pos += 1;
    }

    /// Delete character before cursor (backspace)
    pub fn backspace(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        for i in self.cursor_pos..self.text_len {
            self.text[i - 1] = self.text[i];
        }
        self.text_len -= 1;
        self.cursor_pos -= 1;
    }

    pub fn draw(&self, fb: &FramebufferInfo, offset_x: u32, offset_y: u32) {
        let theme = theme::current();
        let border = match self.state {
            WidgetState::Focused => theme.input_border_focused,
            _ => theme.input_border,
        };
        let px = offset_x + self.x;
        let py = offset_y + self.y;

        // Background
        fill_rect(fb, px, py, self.w, self.h, theme.input_bg);

        // Border
        draw_rect_outline(fb, px, py, self.w, self.h, border);

        // Text content (with scroll offset)
        if self.text_len > 0 {
            let visible = &self.text[self.scroll_offset as usize..self.text_len];
            let visible_str = core::str::from_utf8(visible).unwrap_or("");
            graphics::draw_text(
                fb,
                px + 4,
                py + 4,
                visible_str,
                theme.input_text,
                theme.input_bg,
            );
        }

        // Cursor (blinking would need timer, so just draw a static line)
        if self.state == WidgetState::Focused {
            let cursor_px = px + 4 + (self.cursor_pos as u32 - self.scroll_offset) * 8;
            // Draw 2px wide cursor line
            for dy in 2..(self.h - 2) {
                graphics::draw_pixel(fb, cursor_px, py + dy, theme.input_cursor);
                graphics::draw_pixel(fb, cursor_px + 1, py + dy, theme.input_cursor);
            }
        }
    }

    pub fn hit_test(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Checkbox widget
#[derive(Clone, Copy, Debug)]
pub struct Checkbox {
    pub x: u32,
    pub y: u32,
    pub size: u32,
    pub checked: bool,
    pub label: [u8; MAX_TEXT],
    pub label_len: usize,
}

impl Checkbox {
    pub const ZERO: Checkbox = Checkbox {
        x: 0,
        y: 0,
        size: 16,
        checked: false,
        label: [0u8; MAX_TEXT],
        label_len: 0,
    };

    pub fn label_str(&self) -> &str {
        core::str::from_utf8(&self.label[..self.label_len]).unwrap_or("")
    }

    pub fn draw(&self, fb: &FramebufferInfo, offset_x: u32, offset_y: u32, bg_color: u32) {
        let theme = theme::current();
        let px = offset_x + self.x;
        let py = offset_y + self.y;

        // Box background
        fill_rect(fb, px, py, self.size, self.size, theme.checkbox_bg);
        // Border
        draw_rect_outline(fb, px, py, self.size, self.size, theme.checkbox_border);

        // Checkmark if checked
        if self.checked {
            let _half = self.size / 2;
            // Draw an "X" inside the box
            for i in 0..self.size {
                // Diagonal 1
                let x1 = px + i;
                let y1 = py + i;
                graphics::draw_pixel(fb, x1, y1, theme.checkbox_check);
                // Diagonal 2
                let x2 = px + self.size - 1 - i;
                let y2 = py + i;
                graphics::draw_pixel(fb, x2, y2, theme.checkbox_check);
            }
        }

        // Label text to the right
        if self.label_len > 0 {
            graphics::draw_text(
                fb,
                px + self.size + 6,
                py,
                self.label_str(),
                theme.label_text,
                bg_color,
            );
        }
    }

    pub fn hit_test(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.size && py >= self.y && py < self.y + self.size
    }

    pub fn toggle(&mut self) {
        self.checked = !self.checked;
    }
}

/// Progress bar widget
#[derive(Clone, Copy, Debug)]
pub struct ProgressBar {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// 0..=100
    pub value: u32,
}

impl ProgressBar {
    pub const ZERO: ProgressBar = ProgressBar {
        x: 0,
        y: 0,
        w: 200,
        h: 12,
        value: 0,
    };

    pub fn draw(&self, fb: &FramebufferInfo, offset_x: u32, offset_y: u32) {
        let theme = theme::current();
        let px = offset_x + self.x;
        let py = offset_y + self.y;

        // Track
        fill_rect(fb, px, py, self.w, self.h, theme.progress_track);

        // Fill
        let fill_w = self.w * self.value.min(100) / 100;
        if fill_w > 0 {
            fill_rect(fb, px, py, fill_w, self.h, theme.progress_fill);
        }

        // Border
        draw_rect_outline(fb, px, py, self.w, self.h, theme.panel_border);
    }
}

/// Vertical scrollbar widget
#[derive(Clone, Copy, Debug)]
pub struct ScrollBar {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub thumb_pos: u32,  // 0..=100
    pub thumb_size: u32, // percentage of track height
    pub hovered: bool,
}

impl ScrollBar {
    pub const ZERO: ScrollBar = ScrollBar {
        x: 0,
        y: 0,
        w: 14,
        h: 100,
        thumb_pos: 0,
        thumb_size: 20,
        hovered: false,
    };

    pub fn draw(&self, fb: &FramebufferInfo, offset_x: u32, offset_y: u32) {
        let theme = theme::current();
        let px = offset_x + self.x;
        let py = offset_y + self.y;

        // Track
        fill_rect(fb, px, py, self.w, self.h, theme.scroll_track);

        // Thumb
        let thumb_h = self.h * self.thumb_size.min(100) / 100;
        let thumb_y = py + self.h * self.thumb_pos.min(100) / 100;
        let thumb_color = if self.hovered {
            theme.scroll_thumb_hover
        } else {
            theme.scroll_thumb
        };
        fill_rect(fb, px + 2, thumb_y, self.w - 4, thumb_h, thumb_color);
    }

    /// Update thumb position from scroll percentage
    pub fn set_scroll(&mut self, percent: u32) {
        self.thumb_pos = percent.min(100 - self.thumb_size.min(100));
    }
}

/// Panel widget (container with optional border and title)
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub title: [u8; MAX_TEXT],
    pub title_len: usize,
    pub has_border: bool,
}

impl Panel {
    pub const ZERO: Panel = Panel {
        x: 0,
        y: 0,
        w: 200,
        h: 150,
        title: [0u8; MAX_TEXT],
        title_len: 0,
        has_border: true,
    };

    pub fn title_str(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len]).unwrap_or("")
    }

    pub fn draw(&self, fb: &FramebufferInfo, offset_x: u32, offset_y: u32) {
        let theme = theme::current();
        let px = offset_x + self.x;
        let py = offset_y + self.y;

        // Background
        fill_rect(fb, px, py, self.w, self.h, theme.panel_bg);

        // Border
        if self.has_border {
            draw_rect_outline(fb, px, py, self.w, self.h, theme.panel_border);
        }

        // Title
        if self.title_len > 0 {
            graphics::draw_text(
                fb,
                px + 6,
                py + 4,
                self.title_str(),
                theme.label_text,
                theme.panel_bg,
            );
            // Separator line below title
            fill_rect(fb, px + 4, py + 20, self.w - 8, 1, theme.panel_border);
        }
    }
}

// ─── Widget Registry (per-window) ─────────────────────────────────────

/// Widget kind enumeration for the registry
#[derive(Clone, Copy, Debug)]
pub enum WidgetKind {
    Button(Button),
    Label(Label),
    TextInput(TextInput),
    Checkbox(Checkbox),
    ProgressBar(ProgressBar),
    ScrollBar(ScrollBar),
    Panel(Panel),
}

/// A registered widget with an ID and its kind
#[derive(Clone, Copy, Debug)]
pub struct WidgetEntry {
    pub id: WidgetId,
    pub kind: WidgetKind,
    pub active: bool,
}

/// Global widget table (shared across all windows)
/// Protected by Spinlock for thread-safe access
pub static WIDGET_TABLE: Spinlock<WidgetTable> = Spinlock::new(WidgetTable::new());

/// Per-window widget table
pub struct WidgetTable {
    entries: [WidgetEntry; MAX_WIDGETS],
    count: usize,
    next_id: WidgetId,
}

// WidgetTable is Send/Sync safe because all internal types are primitive or Copy
unsafe impl Send for WidgetTable {}
unsafe impl Sync for WidgetTable {}

impl Default for WidgetTable {
    fn default() -> Self {
        Self::new()
    }
}

impl WidgetTable {
    /// Create an empty widget table
    pub const fn new() -> Self {
        WidgetTable {
            entries: [WidgetEntry {
                id: 0,
                kind: WidgetKind::Label(Label::ZERO),
                active: false,
            }; MAX_WIDGETS],
            count: 0,
            next_id: 1,
        }
    }

    /// Add a button and return its ID
    pub fn add_button(&mut self, x: u32, y: u32, w: u32, h: u32, text: &str) -> Option<WidgetId> {
        let id = self.next_id;
        self.next_id += 1;
        let mut btn = Button::ZERO;
        btn.x = x;
        btn.y = y;
        btn.w = w;
        btn.h = h;
        btn.set_text(text);
        self.insert(WidgetEntry {
            id,
            kind: WidgetKind::Button(btn),
            active: true,
        })
    }

    /// Add a label and return its ID
    pub fn add_label(&mut self, x: u32, y: u32, text: &str) -> Option<WidgetId> {
        let id = self.next_id;
        self.next_id += 1;
        let mut lbl = Label::ZERO;
        lbl.x = x;
        lbl.y = y;
        lbl.set_text(text);
        self.insert(WidgetEntry {
            id,
            kind: WidgetKind::Label(lbl),
            active: true,
        })
    }

    /// Add a text input and return its ID
    pub fn add_text_input(&mut self, x: u32, y: u32, w: u32, h: u32) -> Option<WidgetId> {
        let id = self.next_id;
        self.next_id += 1;
        let mut inp = TextInput::ZERO;
        inp.x = x;
        inp.y = y;
        inp.w = w;
        inp.h = h;
        self.insert(WidgetEntry {
            id,
            kind: WidgetKind::TextInput(inp),
            active: true,
        })
    }

    /// Add a checkbox and return its ID
    pub fn add_checkbox(&mut self, x: u32, y: u32, label: &str) -> Option<WidgetId> {
        let id = self.next_id;
        self.next_id += 1;
        let mut cb = Checkbox::ZERO;
        cb.x = x;
        cb.y = y;
        let len = label.len().min(MAX_TEXT);
        cb.label[..len].copy_from_slice(label.as_bytes());
        cb.label_len = len;
        self.insert(WidgetEntry {
            id,
            kind: WidgetKind::Checkbox(cb),
            active: true,
        })
    }

    /// Add a progress bar and return its ID
    pub fn add_progress(&mut self, x: u32, y: u32, w: u32, h: u32) -> Option<WidgetId> {
        let id = self.next_id;
        self.next_id += 1;
        let mut pb = ProgressBar::ZERO;
        pb.x = x;
        pb.y = y;
        pb.w = w;
        pb.h = h;
        self.insert(WidgetEntry {
            id,
            kind: WidgetKind::ProgressBar(pb),
            active: true,
        })
    }

    /// Add a scrollbar and return its ID
    pub fn add_scrollbar(&mut self, x: u32, y: u32, h: u32) -> Option<WidgetId> {
        let id = self.next_id;
        self.next_id += 1;
        let mut sb = ScrollBar::ZERO;
        sb.x = x;
        sb.y = y;
        sb.h = h;
        self.insert(WidgetEntry {
            id,
            kind: WidgetKind::ScrollBar(sb),
            active: true,
        })
    }

    /// Add a panel and return its ID
    pub fn add_panel(&mut self, x: u32, y: u32, w: u32, h: u32, title: &str) -> Option<WidgetId> {
        let id = self.next_id;
        self.next_id += 1;
        let mut pnl = Panel::ZERO;
        pnl.x = x;
        pnl.y = y;
        pnl.w = w;
        pnl.h = h;
        let len = title.len().min(MAX_TEXT);
        pnl.title[..len].copy_from_slice(title.as_bytes());
        pnl.title_len = len;
        self.insert(WidgetEntry {
            id,
            kind: WidgetKind::Panel(pnl),
            active: true,
        })
    }

    fn insert(&mut self, entry: WidgetEntry) -> Option<WidgetId> {
        for i in 0..MAX_WIDGETS {
            if !self.entries[i].active {
                self.entries[i] = entry;
                self.count += 1;
                return Some(self.entries[i].id);
            }
        }
        None
    }

    /// Remove a widget by ID
    pub fn remove(&mut self, id: WidgetId) {
        for i in 0..MAX_WIDGETS {
            if self.entries[i].active && self.entries[i].id == id {
                self.entries[i].active = false;
                self.count -= 1;
                return;
            }
        }
    }

    /// Draw all widgets in this table
    pub fn draw_all(&self, fb: &FramebufferInfo, offset_x: u32, offset_y: u32, bg_color: u32) {
        for i in 0..MAX_WIDGETS {
            if self.entries[i].active {
                match &self.entries[i].kind {
                    WidgetKind::Button(b) => b.draw(fb, offset_x, offset_y),
                    WidgetKind::Label(l) => l.draw(fb, offset_x, offset_y, bg_color),
                    WidgetKind::TextInput(t) => t.draw(fb, offset_x, offset_y),
                    WidgetKind::Checkbox(c) => c.draw(fb, offset_x, offset_y, bg_color),
                    WidgetKind::ProgressBar(p) => p.draw(fb, offset_x, offset_y),
                    WidgetKind::ScrollBar(s) => s.draw(fb, offset_x, offset_y),
                    WidgetKind::Panel(p) => p.draw(fb, offset_x, offset_y),
                }
            }
        }
    }

    /// Handle a click at (px, py) relative to the window content area
    pub fn handle_click(&mut self, px: u32, py: u32) -> Option<WidgetId> {
        for i in 0..MAX_WIDGETS {
            if !self.entries[i].active {
                continue;
            }
            match &mut self.entries[i].kind {
                WidgetKind::Button(b) => {
                    if b.hit_test(px, py) {
                        b.clicked = true;
                        return Some(self.entries[i].id);
                    }
                }
                WidgetKind::Checkbox(c) => {
                    if c.hit_test(px, py) {
                        c.toggle();
                        return Some(self.entries[i].id);
                    }
                }
                WidgetKind::TextInput(t) => {
                    if t.hit_test(px, py) {
                        t.state = WidgetState::Focused;
                        return Some(self.entries[i].id);
                    } else {
                        t.state = WidgetState::Normal;
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Get a mutable reference to a widget by ID
    pub fn get_mut(&mut self, id: WidgetId) -> Option<&mut WidgetKind> {
        for i in 0..MAX_WIDGETS {
            if self.entries[i].active && self.entries[i].id == id {
                return Some(&mut self.entries[i].kind);
            }
        }
        None
    }

    /// Get an immutable reference to a widget by ID
    pub fn get(&self, id: WidgetId) -> Option<&WidgetKind> {
        for i in 0..MAX_WIDGETS {
            if self.entries[i].active && self.entries[i].id == id {
                return Some(&self.entries[i].kind);
            }
        }
        None
    }

    /// Check if a button was clicked and consume the flag
    pub fn was_clicked(&mut self, id: WidgetId) -> bool {
        for i in 0..MAX_WIDGETS {
            if self.entries[i].active && self.entries[i].id == id {
                if let WidgetKind::Button(b) = &mut self.entries[i].kind {
                    let clicked = b.clicked;
                    b.clicked = false;
                    return clicked;
                }
            }
        }
        false
    }

    /// Get the count of active widgets
    pub fn count(&self) -> usize {
        self.count
    }
}

// ─── Drawing Helpers ───────────────────────────────────────────────────

/// Fill a rectangle with a solid color
pub fn fill_rect(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    for dy in 0..h {
        let py = y + dy;
        if py >= fb.height {
            break;
        }
        for dx in 0..w {
            let px = x + dx;
            if px >= fb.width {
                break;
            }
            graphics::draw_pixel(fb, px, py, color);
        }
    }
}

/// Draw a rectangle outline
fn draw_rect_outline(fb: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32, color: u32) {
    // Top and bottom
    for dx in 0..w {
        let px = x + dx;
        if px >= fb.width {
            break;
        }
        graphics::draw_pixel(fb, px, y, color);
        if h > 0 {
            graphics::draw_pixel(fb, px, y + h - 1, color);
        }
    }
    // Left and right
    for dy in 0..h {
        let py = y + dy;
        if py >= fb.height {
            break;
        }
        graphics::draw_pixel(fb, x, py, color);
        if w > 0 {
            graphics::draw_pixel(fb, x + w - 1, py, color);
        }
    }
}
