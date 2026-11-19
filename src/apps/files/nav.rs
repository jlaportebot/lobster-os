//! Navigation: places, loading a folder, history and refresh.

use super::fsops::{Fs, VfsFs};
use super::model::{self, Entry};
use super::paths;
use super::state::State;
use crate::display::icons::Icon;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A sidebar shortcut.
#[derive(Clone, Debug)]
pub struct Place {
    pub label: &'static str,
    pub path: String,
    pub icon: Icon,
    /// Disabled places are shown dimmed and cannot be opened yet.
    pub enabled: bool,
    /// Create the folder when it does not exist yet.
    pub create: bool,
}

/// The sidebar places, in display order.
#[must_use]
pub fn places() -> Vec<Place> {
    let home = crate::users::home_path();
    let sub = |name: &str| paths::join(&home, name);
    let mut out = alloc::vec![
        place("Home", home.clone(), Icon::FileHome, false),
        place("Desktop", sub("Desktop"), Icon::FileDesktop, true),
        place("Documents", sub("Documents"), Icon::FileDocuments, true),
        place("Downloads", sub("Downloads"), Icon::FileDownloads, true),
        place("Disk", "/sd".to_string(), Icon::FileArchive, false),
        place("Root", "/".to_string(), Icon::FileFolder, false),
    ];
    if crate::drivers::lobster_share::detect() {
        out.push(place(
            "Shared",
            "/share".to_string(),
            Icon::FileFolderOpen,
            false,
        ));
    }
    out
}

fn place(label: &'static str, path: String, icon: Icon, create: bool) -> Place {
    Place {
        label,
        path,
        icon,
        enabled: true,
        create,
    }
}

/// Name of the Trash entry (phase 2 enables it).
pub const TRASH_LABEL: &str = "Trash";

/// Folder the app opens in: the user's home when it exists, else the root.
#[must_use]
pub fn start_folder() -> String {
    let home = crate::users::home_path();
    if VfsFs.stat(&home).is_some_and(|e| e.is_dir) {
        home
    } else {
        paths::ROOT.to_string()
    }
}

/// Read the current folder into `st.entries`, sorted. A folder that cannot
/// be listed falls back to its nearest readable parent; the error is queued
/// as an alert.
pub fn load(st: &mut State) {
    loop {
        let cwd = st.cwd().to_string();
        match VfsFs.list(&cwd) {
            Ok(mut entries) => {
                model::sort_entries(&mut entries, st.sort);
                st.entries = entries;
                break;
            }
            Err(err) => {
                super::ops::alert(st, "Cannot open folder", &alloc::format!("{cwd}: {err}"));
                st.entries = Vec::new();
                if cwd == paths::ROOT {
                    break;
                }
                st.history.replace_current(&paths::parent(&cwd));
            }
        }
    }
    st.selection.retain_below(st.entries.len());
}

/// Show `path`, recording history. Refuses paths that are not folders.
pub fn navigate(st: &mut State, path: &str) {
    let target = paths::normalize(path);
    if !VfsFs.stat(&target).is_some_and(|e| e.is_dir) {
        super::ops::alert(
            st,
            "Cannot open folder",
            &alloc::format!("{target} is not a folder or does not exist."),
        );
        return;
    }
    if st.history.navigate(&target) {
        enter(st);
    }
}

fn enter(st: &mut State) {
    st.selection.clear();
    st.clicks.reset();
    st.scroll = 0;
    load(st);
    crate::wm::dirty::mark();
}

/// Open a sidebar place, creating the folder first when it is a
/// per-user folder that does not exist yet.
pub fn open_place(st: &mut State, place: &Place) {
    if place.create && VfsFs.stat(&place.path).is_none() {
        let _ = VfsFs.mkdir(&place.path);
    }
    navigate(st, &place.path);
}

/// Back in history.
pub fn go_back(st: &mut State) {
    if st.history.go_back().is_some() {
        enter(st);
    }
}

/// Forward in history.
pub fn go_forward(st: &mut State) {
    if st.history.go_forward().is_some() {
        enter(st);
    }
}

/// To the parent folder.
pub fn go_up(st: &mut State) {
    let parent = paths::parent(st.cwd());
    navigate(st, &parent);
}

/// Re-read the folder, keeping the selection by name.
pub fn refresh(st: &mut State) {
    let names: Vec<String> = st
        .selection
        .indices()
        .iter()
        .filter_map(|i| st.entry(*i).map(|e| e.name.clone()))
        .collect();
    load(st);
    select_names(st, &names);
    crate::wm::dirty::mark();
}

/// Select exactly the entries called `names` and scroll to the first.
pub fn select_names(st: &mut State, names: &[String]) {
    st.selection.clear();
    for (n, name) in names.iter().enumerate() {
        let Some(i) = st.index_of(name) else { continue };
        if n == 0 {
            st.selection.click(i);
            st.reveal = Some(i);
        } else {
            st.selection.toggle(i);
        }
    }
}

/// Re-sort the listing, keeping the selection by name.
pub fn resort(st: &mut State) {
    let names: Vec<String> = st
        .selection
        .indices()
        .iter()
        .filter_map(|i| st.entry(*i).map(|e| e.name.clone()))
        .collect();
    model::sort_entries(&mut st.entries, st.sort);
    select_names(st, &names);
}

/// Total size of the selected files.
#[must_use]
pub fn selected_bytes(st: &State) -> u64 {
    st.selection
        .indices()
        .iter()
        .filter_map(|i| st.entry(*i))
        .filter(|e: &&Entry| !e.is_dir)
        .map(|e| e.size)
        .sum()
}
