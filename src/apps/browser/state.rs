//! Browser state: the loaded page, navigation, history and bookmarks.

use super::bookmarks;
use super::dom::{self, Document};
use super::history::History;
use super::html::decode_bytes;
use super::layout::{self, Layout};
use super::pages::{self, LoadInfo};
use super::url;
use crate::net::fetch::FetchError;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Content width assumed before the first frame tells us the real one.
const DEFAULT_CONTENT_W: u32 = 960;
/// Vertical scroll step for arrow keys and the shell, in px.
pub const SCROLL_STEP: i32 = 48;

/// A page that is loaded and laid out.
pub struct Page {
    /// Parsed document, kept so a resize can re-layout without refetching.
    pub doc: Document,
    /// Layout at `layout_width`.
    pub layout: Layout,
    /// Width the layout was computed for.
    pub layout_width: u32,
    /// Load facts for the status line.
    pub info: LoadInfo,
}

/// A navigation waiting for the "Loading" frame to be drawn.
struct Queued {
    url: String,
    record: bool,
}

/// The browser application state (one window).
pub struct Browser {
    /// URL of the page on screen.
    pub url: String,
    /// Text of the address field.
    pub address: String,
    /// The address field has keyboard focus.
    pub address_focus: bool,
    /// The next typed character replaces the whole address.
    pub replace_on_type: bool,
    /// A load is in flight or queued.
    pub loading: bool,
    /// A page has been loaded at least once.
    pub loaded: bool,
    /// Page on screen.
    pub page: Option<Page>,
    /// Vertical scroll offset in px.
    pub scroll: i32,
    /// Transient message replacing the load summary.
    pub note: String,
    pub(super) history: History,
    pub(super) bookmarks: Vec<String>,
    /// Content width of the last drawn frame.
    pub(super) content_w: u32,
    /// Page viewport height of the last drawn frame.
    pub(super) view_h: u32,
    queued: Option<Queued>,
    /// The "Loading" frame of the queued navigation has been drawn.
    queue_shown: bool,
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        crate::arch::timer::Timer::uptime_ms()
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        0
    }
}

impl Browser {
    /// A browser showing nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            url: String::new(),
            address: String::new(),
            address_focus: false,
            replace_on_type: false,
            loading: false,
            loaded: false,
            page: None,
            scroll: 0,
            note: String::new(),
            history: History::new(),
            bookmarks: Vec::new(),
            content_w: DEFAULT_CONTENT_W,
            view_h: 0,
            queued: None,
            queue_shown: false,
        }
    }

    /// Load the persisted bookmarks.
    pub fn load_bookmarks(&mut self) {
        self.bookmarks = bookmarks::load();
    }

    /// Number of saved bookmarks.
    #[must_use]
    pub fn bookmark_count(&self) -> usize {
        self.bookmarks.len()
    }

    /// Whether the current page is bookmarked.
    #[must_use]
    pub fn is_bookmarked(&self) -> bool {
        !self.url.is_empty() && self.bookmarks.contains(&self.url)
    }

    /// Add or remove a bookmark for the current page.
    pub fn toggle_bookmark(&mut self) {
        if self.url.is_empty() {
            return;
        }
        if let Some(pos) = self.bookmarks.iter().position(|b| *b == self.url) {
            self.bookmarks.remove(pos);
            self.note = format!("Bookmark removed: {}", self.url);
        } else {
            self.bookmarks.push(self.url.clone());
            self.note = format!("Bookmarked: {}", self.url);
        }
        bookmarks::save(&self.bookmarks);
    }

    /// Go to `input` as typed in the address bar.
    pub fn navigate(&mut self, input: &str) {
        let target = url::normalize_input(input);
        self.queue(target, true);
    }

    /// Follow a link found on the current page.
    pub fn follow(&mut self, href: &str) {
        if href == pages::RETRY_HREF {
            self.reload();
            return;
        }
        let target = url::resolve(&self.url, href);
        let (new_base, fragment) = url::split_fragment(&target);
        let (old_base, _) = url::split_fragment(&self.url);
        if self.page.is_some() && new_base == old_base {
            self.jump_in_page(&target, fragment);
            return;
        }
        self.queue(target, true);
    }

    fn jump_in_page(&mut self, target: &str, fragment: Option<&str>) {
        self.url = String::from(target);
        self.address = self.url.clone();
        self.history.visit(target);
        bookmarks::save_history(&self.history);
        self.scroll_to_fragment(fragment);
    }

    fn scroll_to_fragment(&mut self, fragment: Option<&str>) {
        let y = fragment
            .filter(|f| !f.is_empty())
            .and_then(|f| self.page.as_ref()?.layout.anchor_y(f));
        self.scroll = y.map_or(0, |y| y as i32);
        self.clamp_scroll();
    }

    /// Reload the current page.
    pub fn reload(&mut self) {
        if !self.url.is_empty() {
            let url = self.url.clone();
            self.queue(url, false);
        }
    }

    /// Step back in history.
    pub fn back(&mut self) -> bool {
        match self.history.back() {
            Some(url) => {
                self.queue(url, false);
                true
            }
            None => false,
        }
    }

    /// Step forward in history.
    pub fn forward(&mut self) -> bool {
        match self.history.forward() {
            Some(url) => {
                self.queue(url, false);
                true
            }
            None => false,
        }
    }

    /// True when a Back step is possible.
    #[must_use]
    pub fn can_back(&self) -> bool {
        self.history.can_back()
    }

    /// True when a Forward step is possible.
    #[must_use]
    pub fn can_forward(&self) -> bool {
        self.history.can_forward()
    }

    fn queue(&mut self, url: String, record: bool) {
        self.address = url.clone();
        self.note = format!("Loading {url} ...");
        self.loading = true;
        self.queued = Some(Queued { url, record });
        self.queue_shown = false;
        crate::wm::dirty::mark();
    }

    /// Called after every frame: the first call after a navigation was queued
    /// only records that "Loading" is on screen, the next one runs the
    /// (blocking) fetch.
    pub fn tick_queue(&mut self) {
        if self.queued.is_none() {
            return;
        }
        if !self.queue_shown {
            self.queue_shown = true;
            crate::wm::dirty::mark();
            return;
        }
        self.run_queued();
    }

    fn run_queued(&mut self) {
        let Some(q) = self.queued.take() else { return };
        self.load(&q.url, q.record);
        self.loading = false;
        crate::wm::dirty::mark();
    }

    fn load(&mut self, target: &str, record: bool) {
        self.note.clear();
        if url::validate_url(target).is_err() || !url::is_fetchable(target) {
            self.install(
                target,
                &pages::unsupported_page_html(target),
                0,
                0,
                0,
                false,
            );
            return;
        }
        let (base, fragment) = url::split_fragment(target);
        let started = now_ms();
        let html_result = crate::net::fetch(base);
        let load_ms = now_ms().saturating_sub(started);
        match html_result {
            Ok(resp) => {
                let html = decode_bytes(&resp.body);
                self.install(
                    target,
                    &html,
                    resp.status,
                    resp.body.len(),
                    load_ms,
                    resp.truncated,
                );
            }
            Err(e) => self.install_error(target, e, load_ms),
        }
        if record {
            self.history.visit(target);
            bookmarks::save_history(&self.history);
        }
        self.scroll_to_fragment(fragment);
    }

    fn install_error(&mut self, target: &str, error: FetchError, load_ms: u64) {
        let html = pages::error_page_html(target, error);
        self.install(target, &html, 0, 0, load_ms, false);
    }

    fn install(
        &mut self,
        target: &str,
        html: &str,
        status: u16,
        bytes: usize,
        load_ms: u64,
        cut: bool,
    ) {
        let started = now_ms();
        let doc = dom::parse(html);
        let layout_width = self.content_w;
        let lay = layout::layout(&doc, layout_width);
        let layout_ms = now_ms().saturating_sub(started);
        let truncated = cut || lay.truncated;
        self.page = Some(Page {
            doc,
            layout: lay,
            layout_width,
            info: LoadInfo {
                status,
                bytes,
                load_ms,
                layout_ms,
                truncated,
            },
        });
        self.url = String::from(target);
        self.address = self.url.clone();
        self.loaded = true;
        self.scroll = 0;
    }

    /// Re-layout when the content width changed since the last layout.
    pub fn relayout_if_needed(&mut self) {
        let width = self.content_w;
        let Some(page) = self.page.as_mut() else {
            return;
        };
        if page.layout_width == width {
            return;
        }
        let started = now_ms();
        page.layout = layout::layout(&page.doc, width);
        page.layout_width = width;
        page.info.layout_ms = now_ms().saturating_sub(started);
        self.clamp_scroll();
    }

    /// Title of the page for the window caption.
    #[must_use]
    pub fn page_title(&self) -> &str {
        self.page.as_ref().map_or("", |p| p.doc.title.as_str())
    }

    /// Largest valid scroll offset.
    #[must_use]
    pub fn max_scroll(&self) -> i32 {
        let height = self.page.as_ref().map_or(0, |p| p.layout.height);
        crate::wm::controls::scroll::max_scroll(height, self.view_h)
    }

    /// Keep the scroll offset inside the page.
    pub fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(0, self.max_scroll());
    }

    /// Scroll by `delta` px.
    pub fn scroll_by(&mut self, delta: i32) {
        self.scroll = self.scroll.saturating_add(delta);
        self.clamp_scroll();
    }

    /// Status-line text when nothing is hovered.
    #[must_use]
    pub fn status_line(&self) -> String {
        if !self.note.is_empty() {
            return self.note.clone();
        }
        match &self.page {
            Some(p) => pages::status_text(&p.info),
            None => String::from("Type an address and press Enter"),
        }
    }
}
