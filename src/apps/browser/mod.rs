//! Lobster Browser: fetches pages over the status-aware network API, parses
//! them with a tolerant HTML tokenizer, lays them out with proportional text
//! and draws them with clickable links, scrolling, history and bookmarks.
//!
//! Modules: `html` (tokenizer), `dom` (document), `layout`, `url`,
//! `history`, `bookmarks`, `pages` (generated pages), `state` (navigation),
//! `view` (drawing) and `input` (keys and pointer).

pub mod bookmarks;
pub mod dom;
pub mod history;
pub mod html;
pub mod input;
pub mod layout;
pub mod pages;
pub mod state;
pub mod url;
pub mod view;

pub use state::Browser;

use crate::wm::controls::ControlState;
use crate::wm::hit_region::Hit;
use crate::wm::window;

/// Window title; also how the launcher and taskbar find the window.
pub const TITLE: &str = "Browser";
/// Page shown when the app is launched without an address.
pub const HOME_URL: &str = "http://example.com";
/// Preferred window size.
const WINDOW_W: u32 = 1024;
const WINDOW_H: u32 = 700;
/// Screen margin kept around a clamped window.
const SCREEN_MARGIN: u32 = 24;
/// Longest page title shown in the caption, in characters.
const CAPTION_CHARS: usize = 48;
/// Scroll step of the shell diagnostics, in px.
const SHELL_SCROLL_PX: i32 = 3 * 16;

/// The single browser instance.
pub static mut INSTANCE: Option<Browser> = None;
static mut CONTROLS: ControlState = ControlState::new();

/// The browser, when initialised.
pub fn instance() -> Option<&'static mut Browser> {
    // SAFETY: input and rendering run on the single UI thread.
    unsafe { (*core::ptr::addr_of_mut!(INSTANCE)).as_mut() }
}

/// Focus and track state of the browser's controls.
pub fn controls() -> &'static mut ControlState {
    // SAFETY: same single-thread discipline as `instance`.
    unsafe { &mut *core::ptr::addr_of_mut!(CONTROLS) }
}

/// Create the browser state on first use.
pub fn init() {
    // SAFETY: single UI thread.
    let slot = unsafe { &mut *core::ptr::addr_of_mut!(INSTANCE) };
    if slot.is_none() {
        let mut b = Browser::new();
        b.load_bookmarks();
        *slot = Some(b);
        crate::println!("[browser] Browser application initialized");
    }
}

/// Toggle a bookmark for the current page.
pub fn toggle_bookmark() {
    if let Some(b) = instance() {
        b.toggle_bookmark();
    }
}

/// Number of saved bookmarks.
pub fn bookmark_count() -> usize {
    instance().map_or(0, |b| b.bookmark_count())
}

/// Window size clamped to the screen.
fn window_size(screen_w: u32, screen_h: u32, panel_h: u32) -> (u32, u32) {
    let w = WINDOW_W.min(screen_w.saturating_sub(2 * SCREEN_MARGIN));
    let h = WINDOW_H.min(screen_h.saturating_sub(panel_h + 2 * SCREEN_MARGIN));
    (w, h)
}

fn create_window() -> u32 {
    let screen = crate::display::fb::get_info();
    let panel_h = crate::wm::panel::panel_height();
    let (w, h) = window_size(screen.width, screen.height, panel_h);
    let x = screen.width.saturating_sub(w) / 2;
    let y = (screen.height.saturating_sub(panel_h + h) / 2).max(SCREEN_MARGIN / 2);
    window::create(x, y, w, h, TITLE).unwrap_or(0)
}

/// Open (or focus) the window and go to `url`.
pub fn open(url: &str) -> u32 {
    init();
    let win_id = match window::find_by_title(TITLE) {
        Some(id) => {
            window::focus(id);
            id
        }
        None => create_window(),
    };
    if win_id == 0 {
        return 0;
    }
    if let Some(b) = instance() {
        b.navigate(if url.is_empty() { HOME_URL } else { url });
    }
    win_id
}

/// Navigate the existing browser to `url`.
pub fn navigate(url: &str) {
    if let Some(b) = instance() {
        b.navigate(url);
    }
}

/// Shell diagnostic: scroll the page one step.
pub fn scroll(down: bool) {
    if let Some(b) = instance() {
        b.scroll_by(if down {
            SHELL_SCROLL_PX
        } else {
            -SHELL_SCROLL_PX
        });
    }
}

/// Keyboard input while the Browser window is focused. True when consumed.
pub fn handle_key(scancode: u8, ch: Option<char>) -> bool {
    instance().is_some_and(|b| input::handle_key(b, scancode, ch))
}

fn on_hit(hit: &Hit) -> bool {
    instance().is_some_and(|b| input::on_hit(b, hit))
}

/// Window caption: the page title after the app name.
fn caption(b: &Browser) -> alloc::string::String {
    let title = b.page_title();
    if title.is_empty() {
        return alloc::string::String::from(TITLE);
    }
    let short: alloc::string::String = title.chars().take(CAPTION_CHARS).collect();
    alloc::format!("{TITLE} - {short}")
}

/// Called from `wm::tick`: draw the Browser window.
#[cfg(feature = "qemu_raspi5b")]
pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
    let Some(win_id) = window::find_by_title(TITLE) else {
        return;
    };
    let Some((x, y, w, h)) = window::get_info(win_id) else {
        return;
    };
    let Some(b) = instance() else { return };
    let focused = window::focused_id() == win_id;
    let target = crate::wm::scene::window_target(win_id, fb);
    let title = caption(b);
    let (cx, cy, cw, ch) =
        crate::wm::window_frame::draw_frame(&target, win_id, x, y, w, h, &title, focused);
    crate::wm::hit_region::set_handler(win_id, on_hit);
    let mut ui = crate::wm::controls::Ui::new(&target, win_id, (cx, cy), (cw, ch), controls());
    view::frame(&mut ui, b, (cx, cy), (cw, ch));
    ui.finish();
    b.tick_queue();
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn window_size_is_clamped_to_the_screen() {
        assert_eq!(window_size(1920, 1080, 40), (WINDOW_W, WINDOW_H));
        let (w, h) = window_size(800, 600, 40);
        assert!(w <= 800 - 2 * SCREEN_MARGIN && h <= 600 - 40 - 2 * SCREEN_MARGIN);
    }
}
