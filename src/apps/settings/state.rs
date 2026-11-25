//! Mutable state of the Settings window, kept between frames.

use super::model::Page;
use super::resolution::Confirm;
use crate::wm::controls::{ControlState, Dropdown};
use alloc::string::String;
use alloc::vec::Vec;

/// Number of selectable resolutions (rows of the dropdown).
const RESOLUTION_COUNT: usize = super::resolution::RESOLUTIONS.len();
/// URL the connection test starts with.
pub const DEFAULT_TEST_URL: &str = "http://example.com";

/// Progress of the "Test connection" button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetTest {
    Idle,
    /// Requested; the "Testing" frame has not been drawn yet.
    Queued,
    /// "Testing" is on screen: run the fetch now.
    Running,
    Done(String),
}

/// What an open dialog was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    AddUser,
    RemoveUser(String),
    Uninstall(String),
    /// Change-password flow, one dialog per step.
    Password(super::passwords::Step),
}

/// A dialog waiting for the mouse button to be released before it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedDialog {
    pub title: String,
    pub message: String,
    /// Text prompt (true) or yes/no confirmation (false).
    pub is_prompt: bool,
    /// Prompt whose text is hidden (passwords).
    pub is_secret: bool,
}

/// One installed package as shown on the Apps page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageRow {
    pub name: String,
    pub version: String,
    pub size: u64,
}

/// Everything the window remembers.
pub struct State {
    pub page: Page,
    pub scroll: i32,
    /// Content height measured in the last frame.
    pub content_h: u32,
    pub resolution_menu: Dropdown,
    pub confirm: Confirm,
    pub net_test: NetTest,
    pub url: String,
    /// One-line result of the last Users / Apps action.
    pub notice: String,
    pub pending: Option<Pending>,
    pub queued_dialog: Option<QueuedDialog>,
    pub packages: Vec<PackageRow>,
    pub packages_loaded_ms: Option<u64>,
}

impl State {
    /// State of a freshly opened window.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            page: Page::Appearance,
            scroll: 0,
            content_h: 0,
            resolution_menu: Dropdown::new(RESOLUTION_COUNT),
            confirm: Confirm::new(),
            net_test: NetTest::Idle,
            url: String::new(),
            notice: String::new(),
            pending: None,
            queued_dialog: None,
            packages: Vec::new(),
            packages_loaded_ms: None,
        }
    }

    /// Switch page and reset what belongs to the previous one.
    pub fn go_to(&mut self, page: Page) {
        self.page = page;
        self.scroll = 0;
        self.notice.clear();
        self.resolution_menu.close();
        self.packages_loaded_ms = None;
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

static mut STATE: State = State::new();

/// The window state. Settings runs on the single UI thread.
pub fn get() -> &'static mut State {
    // SAFETY: only the UI thread (input handlers and the render pass) calls
    // this, never reentrantly across a held borrow.
    unsafe { &mut *core::ptr::addr_of_mut!(STATE) }
}

static mut CONTROLS: ControlState = ControlState::new();

/// Focus and track state of the controls (separate from [`State`] so a
/// frame can borrow both at once).
pub fn controls() -> &'static mut ControlState {
    // SAFETY: same single-thread discipline as `get`.
    unsafe { &mut *core::ptr::addr_of_mut!(CONTROLS) }
}
