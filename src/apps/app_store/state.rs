//! Mutable state of the App Store window, kept between frames.

use super::job::Job;
use super::model::{Entry, Section};
use crate::wm::controls::ControlState;
use alloc::string::String;
use alloc::vec::Vec;

/// Everything the window remembers.
pub struct State {
    pub entries: Vec<Entry>,
    pub section: Section,
    pub query: String,
    /// Name of the app whose details are open.
    pub selected: Option<String>,
    pub scroll: i32,
    /// List height measured in the last frame.
    pub content_h: u32,
    pub job: Option<Job>,
    /// The job's current stage has been drawn; the next tick runs it.
    pub stage_shown: bool,
    /// The last network request failed.
    pub offline: bool,
    /// Last job outcome or catalogue message.
    pub notice: String,
    /// The catalogue refresh at first open has been queued.
    pub started: bool,
}

impl State {
    /// State of a freshly opened window.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            section: Section::Discover,
            query: String::new(),
            selected: None,
            scroll: 0,
            content_h: 0,
            job: None,
            stage_shown: false,
            offline: false,
            notice: String::new(),
            started: false,
        }
    }

    /// Show another section and drop what belonged to the old one.
    pub fn go_to(&mut self, section: Section) {
        self.section = section;
        self.scroll = 0;
        self.selected = None;
    }

    /// The entry whose details are open.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&Entry> {
        let name = self.selected.as_ref()?;
        self.entries.iter().find(|e| &e.name == name)
    }

    /// True while a job runs.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.job.as_ref().is_some_and(Job::is_active)
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

static mut STATE: State = State::new();

/// The window state. The App Store runs on the single UI thread.
pub fn get() -> &'static mut State {
    // SAFETY: only the UI thread (input handlers and the render pass) calls
    // this, never reentrantly across a held borrow.
    unsafe { &mut *core::ptr::addr_of_mut!(STATE) }
}

static mut CONTROLS: ControlState = ControlState::new();

/// Focus and track state of the controls.
pub fn controls() -> &'static mut ControlState {
    // SAFETY: same single-thread discipline as `get`.
    unsafe { &mut *core::ptr::addr_of_mut!(CONTROLS) }
}
