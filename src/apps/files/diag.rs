//! `fm` shell diagnostics: query and drive the Files app from the serial
//! console so behaviour is provable without the GUI.

use super::model::{Sort, SortKey, ViewMode};
use super::state::State;
use super::{actions, fsops, nav, ops};
use crate::println;
use alloc::string::ToString;

const USAGE: &str = "fm: usage: fm <state|ls|nav|up|back|forward|refresh|select|sel|view|sort|open|copy|move|del|rename|clip> [...]";
/// Most entries `fm ls` prints.
const LS_LIMIT: usize = 40;

fn sort_name(key: SortKey) -> &'static str {
    match key {
        SortKey::Name => "name",
        SortKey::Size => "size",
        SortKey::Modified => "modified",
        SortKey::Type => "type",
    }
}

fn parse_sort(name: &str) -> Option<SortKey> {
    [
        SortKey::Name,
        SortKey::Size,
        SortKey::Modified,
        SortKey::Type,
    ]
    .into_iter()
    .find(|k| sort_name(*k) == name)
}

fn report_state(st: &State) {
    let view = if st.view == ViewMode::List {
        "list"
    } else {
        "icons"
    };
    let Sort { key, descending } = st.sort;
    let dir = if descending { "desc" } else { "asc" };
    println!(
        "fm: cwd={} view={} sort={} {} entries={} selected={} clipboard={} queued={}",
        st.cwd(),
        view,
        sort_name(key),
        dir,
        st.entries.len(),
        st.selection.count(),
        st.clipboard.as_ref().map_or(0, |c| c.paths.len()),
        st.queue.len()
    );
}

fn list(st: &State) {
    println!("fm: cwd {} ({} entries)", st.cwd(), st.entries.len());
    for (i, e) in st.entries.iter().take(LS_LIMIT).enumerate() {
        let mark = if st.selection.contains(i) { '*' } else { ' ' };
        let kind = if e.is_dir { "d" } else { "-" };
        println!("fm: {mark} {kind} {:>8} {}", e.size, e.name);
    }
}

fn select(st: &mut State, name: &str) {
    match st.index_of(name) {
        Some(i) => {
            st.selection.click(i);
            st.reveal = Some(i);
            println!("fm: selected {name}");
        }
        None => println!("fm: no entry named {name}"),
    }
}

fn transfer(st: &mut State, dest: &str, cut: bool) {
    let cap = crate::fs::vfs::MAX_FILE_DATA as u64;
    for src in st.selected_paths() {
        let done = if cut {
            fsops::move_item(&mut fsops::VfsFs, &src, dest, cap)
        } else {
            fsops::copy_item(&mut fsops::VfsFs, &src, dest, cap)
        };
        match done {
            Ok(to) => println!("fm: {src} -> {to}"),
            Err(err) => println!("fm: {}", err.text()),
        }
    }
    nav::refresh(st);
}

fn delete(st: &mut State) {
    for path in st.selected_paths() {
        match fsops::remove_tree(&mut fsops::VfsFs, &path) {
            Ok(()) => println!("fm: deleted {path}"),
            Err(err) => println!("fm: {}", err.text()),
        }
    }
    st.selection.clear();
    nav::load(st);
}

fn rename(st: &mut State, new_name: &str) {
    let selected = st.selected_paths();
    let [path] = selected.as_slice() else {
        println!("fm: select exactly one entry first");
        return;
    };
    let target = super::paths::join(&super::paths::parent(path), new_name);
    match fsops::Fs::rename(&mut fsops::VfsFs, path, &target) {
        Ok(()) => println!("fm: renamed to {new_name}"),
        Err(err) => println!("fm: {err}"),
    }
    nav::refresh(st);
}

fn view_and_sort(st: &mut State, sub: &str, arg: &str) {
    if sub == "view" {
        match arg {
            "list" => actions::set_view(st, ViewMode::List),
            "icons" => actions::set_view(st, ViewMode::Icons),
            _ => println!("fm: view list|icons"),
        }
    } else if let Some(key) = parse_sort(arg) {
        st.sort = st.sort.after_click(key);
        nav::resort(st);
    } else {
        println!("fm: sort name|size|modified|type");
    }
}

/// Run `fm <args...>` (`args[0]` is the subcommand).
pub fn run(args: &[&str]) {
    let st = super::ensure_state();
    let arg = args.get(1).copied().unwrap_or("");
    match args.first().copied() {
        Some("state") => report_state(st),
        Some("ls") => list(st),
        Some("nav") => nav::navigate(st, arg),
        Some("up") => nav::go_up(st),
        Some("back") => nav::go_back(st),
        Some("forward") => nav::go_forward(st),
        Some("refresh") => nav::refresh(st),
        Some("select") => select(st, arg),
        Some("sel") => select_index(st, arg),
        Some(sub @ ("view" | "sort")) => view_and_sort(st, sub, arg),
        Some("open") => actions::open_selection(st),
        Some("copy") => transfer(st, arg, false),
        Some("move") => transfer(st, arg, true),
        Some("del") => delete(st),
        Some("rename") => rename(st, arg),
        Some("clip") => ops::copy_selection(st, arg == "cut"),
        _ => println!("{USAGE}"),
    }
    if matches!(
        args.first().copied(),
        Some("nav" | "up" | "back" | "forward")
    ) {
        println!("fm: cwd {}", st.cwd().to_string());
    }
}

fn select_index(st: &mut State, arg: &str) {
    match arg.parse::<usize>() {
        Ok(i) if i < st.entries.len() => {
            st.selection.click(i);
            println!("fm: selected {i}");
        }
        _ => println!("fm: bad index"),
    }
}
