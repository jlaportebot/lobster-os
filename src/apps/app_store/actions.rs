//! What the store's controls do: control keys, job control and "Open".

use super::backend;
use super::job::{Job, Op};
use super::model::{self, Entry, Status};
use super::state::{self, State};
use crate::pkg::package::AppKind;
use alloc::string::{String, ToString};

const PARAM_BITS: u32 = 16;
const PARAM_MASK: u32 = (1 << PARAM_BITS) - 1;

/// Size of the Terminal window "Open" creates for command-line apps.
const TERMINAL_RECT: (u32, u32, u32, u32) = (120, 90, 720, 460);

/// A control in the window; its [`Action::key`] tags the hit region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Sidebar entry at this position.
    Section(u16),
    Search,
    /// Open the details of the entry at this index.
    Row(u16),
    /// State button of the entry at this index.
    RowButton(u16),
    Refresh,
    DetailsPrimary,
    DetailsRemove,
    DetailsClose,
    DismissJob,
    ListScroll,
}

impl Action {
    fn parts(self) -> (u32, u32) {
        match self {
            Action::Section(i) => (1, u32::from(i)),
            Action::Search => (2, 0),
            Action::Row(i) => (3, u32::from(i)),
            Action::RowButton(i) => (4, u32::from(i)),
            Action::Refresh => (5, 0),
            Action::DetailsPrimary => (6, 0),
            Action::DetailsRemove => (7, 0),
            Action::DetailsClose => (8, 0),
            Action::DismissJob => (9, 0),
            Action::ListScroll => (10, 0),
        }
    }

    /// Hit-region tag of this control.
    #[must_use]
    pub fn key(self) -> u32 {
        let (tag, param) = self.parts();
        (tag << PARAM_BITS) | param
    }

    /// Inverse of [`Action::key`].
    #[must_use]
    pub fn from_key(key: u32) -> Option<Action> {
        let param = key & PARAM_MASK;
        let index = param as u16;
        Some(match key >> PARAM_BITS {
            1 => Action::Section(index),
            2 => Action::Search,
            3 => Action::Row(index),
            4 => Action::RowButton(index),
            5 => Action::Refresh,
            6 => Action::DetailsPrimary,
            7 => Action::DetailsRemove,
            8 => Action::DetailsClose,
            9 => Action::DismissJob,
            10 => Action::ListScroll,
            _ => return None,
        })
    }
}

/// Carry out the control that was clicked or activated.
pub fn perform(action: Action) {
    let st = state::get();
    match action {
        Action::Section(i) => {
            let sections = model::sections(&st.entries);
            if let Some(section) = sections.get(usize::from(i)) {
                st.go_to(section.clone());
            }
        }
        Action::Row(i) => select(st, usize::from(i)),
        Action::RowButton(i) => primary(st, st.entries.get(usize::from(i)).cloned()),
        Action::DetailsPrimary => primary(st, st.selected_entry().cloned()),
        Action::DetailsRemove => start(st, Op::Remove, st.selected.clone().unwrap_or_default()),
        Action::DetailsClose => st.selected = None,
        Action::Refresh => start(st, Op::Refresh, String::new()),
        Action::DismissJob => dismiss(st),
        Action::Search | Action::ListScroll => {}
    }
}

fn select(st: &mut State, index: usize) {
    let Some(entry) = st.entries.get(index) else {
        return;
    };
    st.selected = Some(entry.name.clone());
}

fn dismiss(st: &mut State) {
    if !st.busy() {
        st.job = None;
    }
}

/// The state button: install, update or open the entry.
fn primary(st: &mut State, entry: Option<Entry>) {
    let Some(entry) = entry else { return };
    match entry.status() {
        Status::Available => start(st, Op::Install, entry.name),
        Status::UpdateAvailable => start(st, Op::Update, entry.name),
        Status::Installed => open(st, &entry),
    }
}

/// Queue a job; the first stage runs on the next frame.
pub fn start(st: &mut State, op: Op, name: String) {
    if st.busy() {
        return;
    }
    st.notice.clear();
    st.job = Some(Job::new(op, &name));
    st.stage_shown = false;
}

fn open(st: &mut State, entry: &Entry) {
    let result = match entry.kind {
        AppKind::Gui => spawn(entry),
        AppKind::Cli => run_in_terminal(entry),
    };
    st.notice = match result {
        Ok(()) => alloc::format!("Opened {}.", entry.name),
        Err(reason) => alloc::format!("Could not open {}: {reason}", entry.name),
    };
}

fn spawn(entry: &Entry) -> Result<(), &'static str> {
    if entry.exec.is_empty() {
        return Err("it installed no program");
    }
    let app = crate::wm::launcher::InstalledApp {
        name: entry.name.clone(),
        exec: entry.exec.clone(),
        keywords: String::new(),
    };
    crate::wm::launcher::launch_installed(&app)
}

fn run_in_terminal(entry: &Entry) -> Result<(), &'static str> {
    if entry.exec.is_empty() {
        return Err("it installed no program");
    }
    if crate::wm::window::find_by_title("Terminal").is_none() {
        let (x, y, w, h) = TERMINAL_RECT;
        crate::apps::terminal::open_terminal_window(x, y, w, h).ok_or("no window available")?;
    }
    let command = alloc::format!("{}\n", entry.exec);
    crate::apps::terminal::get_terminal().send_input(command.as_bytes());
    Ok(())
}

/// Rebuild the entry list from the cached catalogue and the database.
pub fn reload_entries(st: &mut State) {
    let Some(backend) = backend::get() else {
        st.notice = "The package database cannot be opened.".to_string();
        return;
    };
    let catalogue = backend.catalogue();
    let installed = backend.installed();
    st.entries = model::merge(&catalogue, &installed);
    if st.selected_entry().is_none() {
        st.selected = None;
    }
}

/// One frame of background work: queue the first refresh, run the pending
/// stage of the current job.
pub fn tick() {
    let st = state::get();
    if !st.started {
        st.started = true;
        reload_entries(st);
        start(st, Op::Refresh, String::new());
        return;
    }
    if !st.stage_shown || !st.busy() {
        return;
    }
    st.stage_shown = false;
    let (Some(job), Some(backend)) = (st.job.as_mut(), backend::get()) else {
        return;
    };
    job.step(backend);
    if !job.is_active() {
        finish(st);
    }
}

fn finish(st: &mut State) {
    let Some(job) = st.job.as_ref() else { return };
    st.notice = job.outcome().unwrap_or_default();
    let is_refresh = job.op == Op::Refresh;
    // A finished refresh needs no progress bar: the status line says it all.
    let quiet_success = is_refresh && job.stage == super::job::Stage::Done;
    if is_refresh || job.stage_is_offline() {
        st.offline = job.stage_is_offline();
    }
    reload_entries(st);
    if quiet_success {
        st.job = None;
    }
}
