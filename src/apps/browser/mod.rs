//! Lobster Browser — a minimal graphical web browser
//!
//! Fetches a URL over lobster-net (host HTTP proxy), renders a
//! plain-text/HTML-stripped page into a scrollable window client area,
//! with URL bar, back/forward history, and a status line. Bookmarks and
//! history persist to /sd/etc/browser/.

use crate::display::fb::get_info;
use crate::display::graphics;
use crate::wm::theme;
use crate::wm::window;

use super::browser_text::{validate_url, History, Page, MAX_LINES};

pub mod history;
pub mod html;
pub mod url;

/// Visible rows in the page area
const VISIBLE_ROWS: usize = 28;

pub struct Browser {
    pub url: alloc::string::String,
    pub loaded: bool,
    pub loading: bool,
    page: Page,
    scroll: usize,
    history: History,
    status: alloc::string::String,
    /// Persisted bookmark list (one URL per line, /sd/etc/browser/bookmarks)
    bookmarks: alloc::vec::Vec<alloc::string::String>,
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl Browser {
    pub fn new() -> Self {
        Self {
            url: alloc::string::String::from("http://example.com"),
            loaded: false,
            loading: false,
            page: Page::new(),
            scroll: 0,
            history: History::new(),
            status: alloc::string::String::new(),
            bookmarks: alloc::vec::Vec::new(),
        }
    }

    /// Load bookmarks from /sd/etc/browser/bookmarks (best effort).
    pub fn load_bookmarks(&mut self) {
        self.bookmarks.clear();
        if let Ok(data) = crate::fs::vfs::read_file("/sd/etc/browser/bookmarks") {
            if let Ok(text) = core::str::from_utf8(&data) {
                for line in text.lines() {
                    let t = line.trim();
                    if !t.is_empty() {
                        self.bookmarks.push(alloc::string::String::from(t));
                    }
                }
            }
        }
    }

    /// Save bookmarks to /sd/etc/browser/bookmarks (best effort).
    pub fn save_bookmarks(&self) {
        // Ensure the parent directory exists (persisted via mkdir write-through)
        let _ = crate::fs::vfs::create_dir("/sd/etc/browser");
        let mut body = alloc::string::String::new();
        for b in &self.bookmarks {
            body.push_str(b);
            body.push('\n');
        }
        match crate::fs::vfs::write_file("/sd/etc/browser/bookmarks", body.as_bytes()) {
            Ok(_) => {}
            Err(e) => crate::println!("[browser] bookmark save failed: {:?}", e),
        }
    }

    /// Toggle a bookmark for the current URL.
    pub fn toggle_bookmark(&mut self) {
        let url = self.url.clone();
        if let Some(pos) = self.bookmarks.iter().position(|b| *b == url) {
            self.bookmarks.remove(pos);
            self.status = alloc::format!("Bookmark removed: {}", url);
        } else {
            self.bookmarks.push(url.clone());
            self.status = alloc::format!("Bookmarked: {}", url);
        }
        self.save_bookmarks();
    }

    pub fn bookmark_count(&self) -> usize {
        self.bookmarks.len()
    }

    /// Persist the browsing history to /sd/etc/browser/history (last 20).
    fn persist_history(&self) {
        let mut body = alloc::string::String::new();
        let entries = self.history.entries();
        let start = entries.len().saturating_sub(20);
        for h in &entries[start..] {
            body.push_str(h);
            body.push('\n');
        }
        let _ = crate::fs::vfs::write_file("/sd/etc/browser/history", body.as_bytes());
    }

    /// Fetch `url`, render it, and record it in the history.
    pub fn navigate(&mut self, url: &str) {
        if self.load(url) {
            self.history.visit(url);
            self.persist_history();
        }
    }

    /// Fetch and render `url` without touching the history.
    /// Returns true if the page was loaded. Never panics: every failure is
    /// reported to the user through the status line.
    fn load(&mut self, url: &str) -> bool {
        self.url = alloc::string::String::from(url);
        if let Err(e) = validate_url(url) {
            self.loading = false;
            self.status = alloc::format!("Cannot load: {}", e.message());
            return false;
        }
        self.loading = true;
        self.status = alloc::format!("Loading {}...", url);

        match crate::drivers::lobster_net::http_fetch(url) {
            Some(body) => {
                self.page.render_html(&body);
                self.loaded = true;
                self.loading = false;
                self.scroll = 0;
                self.status = if self.page.truncated {
                    alloc::format!(
                        "Done ({} bytes) - page truncated at {} lines",
                        body.len(),
                        MAX_LINES
                    )
                } else {
                    alloc::format!("Done ({} bytes)", body.len())
                };
                true
            }
            None => {
                self.loading = false;
                self.status = alloc::format!("Failed to load {}", url);
                false
            }
        }
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(3);
    }
    pub fn scroll_down(&mut self) {
        if self.scroll + VISIBLE_ROWS < self.page.line_count {
            self.scroll += 3;
        }
    }

    pub fn back(&mut self) -> bool {
        match self.history.back() {
            Some(url) => {
                self.load(&url);
                true
            }
            None => false,
        }
    }

    pub fn forward(&mut self) -> bool {
        match self.history.forward() {
            Some(url) => {
                self.load(&url);
                true
            }
            None => false,
        }
    }

    /// Render the browser UI into a window client area.
    pub fn render(&self, cx: u32, cy: u32, cw: u32, ch: u32, focused: bool) {
        let fb = match get_info_nonzero() {
            Some(fb) => fb,
            None => return,
        };
        let theme = theme::current();

        // Chrome band: URL bar + buttons + status
        let chrome_h = 56u32;
        graphics::draw_rect(&fb, cx, cy, cw, chrome_h, theme.win_bg);

        // URL bar
        let url_y = cy + 8;
        let url_h = 20u32;
        let url_w = cw.saturating_sub(200);
        graphics::draw_rect(&fb, cx + 8, url_y, url_w, url_h, theme.input_bg);
        graphics::draw_rect_outline(
            &fb,
            cx + 8,
            url_y,
            url_w,
            url_h,
            if focused {
                theme.input_border_focused
            } else {
                theme.input_border
            },
        );
        // URL text (clip)
        let url_display: alloc::string::String =
            self.url.chars().take((url_w / 8) as usize).collect();
        graphics::draw_text(
            &fb,
            cx + 12,
            url_y + 4,
            &url_display,
            theme.input_text,
            theme.input_bg,
        );

        // Buttons: Back | Fwd | Go
        let mut bx = cx + url_w + 16;
        for (label, w) in [("< Back", 56u32), ("Fwd >", 56u32), ("Go", 28u32)] {
            graphics::fill_rounded_rect(&fb, bx, url_y, w, url_h, 6, theme.btn_bg);
            graphics::draw_text(&fb, bx + 6, url_y + 4, label, theme.btn_text, theme.btn_bg);
            bx += w + 6;
        }

        // Status line + bookmark count
        graphics::draw_text(
            &fb,
            cx + 8,
            cy + 34,
            &self.status,
            theme.label_text_dim,
            theme.win_bg,
        );
        let bm = alloc::format!("\u{2605} {}", self.bookmarks.len());
        graphics::draw_text(
            &fb,
            cx + cw.saturating_sub(120),
            cy + 34,
            &bm,
            0xFFD08770,
            theme.win_bg,
        );
        graphics::draw_rect(&fb, cx, cy + chrome_h, cw, 1, theme.menu_border);

        // Page area
        let page_y = cy + chrome_h + 2;
        let page_h = ch.saturating_sub(chrome_h + 4);
        graphics::draw_rect(&fb, cx, page_y, cw, page_h, 0xFFFFFFFF);

        // Page lines
        let char_h = 16u32;
        let rows = ((page_h / char_h) as usize).min(VISIBLE_ROWS);
        for r in 0..rows {
            let idx = self.scroll + r;
            if idx >= self.page.line_count {
                break;
            }
            let text = self.page.line_str(idx);
            if !text.is_empty() {
                graphics::draw_text(
                    &fb,
                    cx + 10,
                    page_y + 2 + (r as u32) * char_h,
                    text,
                    0xFF202020,
                    0xFFFFFFFF,
                );
            }
        }

        // Scroll indicator (all arithmetic saturating: tiny windows must not wrap)
        let line_count = self.page.line_count;
        if line_count > rows && cw > 6 {
            let bar_x = cx + cw - 6;
            let track_h = page_h.saturating_sub(4);
            let thumb_h = (track_h.saturating_mul(rows as u32) / line_count as u32)
                .max(12)
                .min(track_h);
            let max_scroll = line_count - rows;
            let thumb_y = page_y
                + 2
                + (track_h - thumb_h).saturating_mul(self.scroll.min(max_scroll) as u32)
                    / max_scroll.max(1) as u32;
            graphics::draw_rect(&fb, bar_x, page_y + 2, 4, track_h, theme.menu_border);
            graphics::draw_rect(&fb, bar_x, thumb_y, 4, thumb_h, theme.label_text_dim);
        }
    }
}

fn get_info_nonzero() -> Option<crate::display::fb::FramebufferInfo> {
    let fb = get_info();
    if fb.width == 0 {
        None
    } else {
        Some(fb)
    }
}

// ─── App integration ─────────────────────────────────────────────────

pub static mut INSTANCE: Option<Browser> = None;

pub fn init() {
    unsafe {
        if INSTANCE.is_none() {
            let mut b = Browser::new();
            b.load_bookmarks();
            INSTANCE = Some(b);
        }
    }
    crate::println!("[browser] Browser application initialized");
}

/// Toggle a bookmark for the current URL.
pub fn toggle_bookmark() {
    unsafe {
        if let Some(b) = INSTANCE.as_mut() {
            b.toggle_bookmark();
        }
    }
}

/// Bookmark count (for the shell/tests).
pub fn bookmark_count() -> usize {
    unsafe { INSTANCE.as_ref().map(|b| b.bookmark_count()).unwrap_or(0) }
}

pub fn open(url: &str) -> u32 {
    init();
    // Focus-or-create the window
    let win_id = match window::find_by_title("Browser") {
        Some(id) => {
            window::focus(id);
            id
        }
        None => match window::create(200, 100, 900, 620, "Browser") {
            Some(id) => id,
            None => return 0,
        },
    };
    unsafe {
        if let Some(b) = INSTANCE.as_mut() {
            b.navigate(url);
        }
    }
    win_id
}

pub fn navigate(url: &str) {
    unsafe {
        if let Some(b) = INSTANCE.as_mut() {
            b.navigate(url);
        }
    }
}

pub fn scroll(down: bool) {
    unsafe {
        if let Some(b) = INSTANCE.as_mut() {
            if down {
                b.scroll_down();
            } else {
                b.scroll_up();
            }
        }
    }
}

/// Called from wm::tick: render into the Browser window's client area.
#[cfg(feature = "qemu_raspi5b")]
pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
    let Some(win_id) = window::find_by_title("Browser") else {
        return;
    };
    let Some((x, y, w, h)) = window::get_info(win_id) else {
        return;
    };
    let focused = window::focused_id() == win_id;
    let fb = &crate::wm::scene::window_target(win_id, fb);
    let (cx, cy, cw, ch) =
        crate::wm::window_frame::draw_frame(fb, win_id, x, y, w, h, "Browser", focused);
    unsafe {
        if let Some(b) = INSTANCE.as_ref() {
            b.render(cx, cy, cw, ch, focused);
        }
    }
}
