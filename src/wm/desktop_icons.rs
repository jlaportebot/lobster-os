//! Desktop icons: app shortcuts plus the entries of `~/Desktop`.
//!
//! Icons sit on a snap grid ([`desktop_layout`]), are selected with a click,
//! opened with a double-click or Enter, dragged to rearrange (positions are
//! persisted by [`desktop_files`]), and offer context menus. State lives in
//! one locked struct; anything that can re-enter the window manager (launching
//! apps, dialogs) runs after the lock is released.

use crate::display::fb::{self, FramebufferInfo};
use crate::display::graphics;
use crate::display::icons::{self, Icon};
use crate::display::text::{self, Face};
use crate::sync::spinlock::Spinlock;
use crate::wm::desktop_files::{self, DesktopEntry};
use crate::wm::desktop_layout::{self as layout, Cell, ClickTracker};
use crate::wm::{context_menu, dialog, theme, window};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Width of the clickable icon body.
const ICON_SIZE: u32 = 64;
/// Edge of the app tile drawn inside the body.
const TILE_SIZE: u32 = 48;
/// Tile inset from the body's left edge (centres it).
const TILE_INSET_X: u32 = (ICON_SIZE - TILE_SIZE) / 2;
/// Tile inset from the body's top edge.
const TILE_INSET_Y: u32 = 6;
/// Height reserved below the tile for the label.
const ICON_LABEL_H: u32 = 18;
/// Gap between body and label baseline box.
const LABEL_GAP: u32 = 2;
/// Label size at the Medium font setting.
const LABEL_PX: u32 = 12;
/// Widest label before it is shortened with an ellipsis.
const LABEL_MAX_W: u32 = layout::CELL_W - 8;
/// Corner radius of the selection and hover highlights.
const HIGHLIGHT_RADIUS: u32 = 8;
/// Highlight outset (horizontal, vertical) for the selected state.
const SELECT_PAD: (u32, u32) = (12, 4);
/// Highlight outset (horizontal, vertical) for the hovered state.
const HOVER_PAD: (u32, u32) = (10, 2);
/// Highlight opacity for the selected and hovered states.
const SELECT_ALPHA: u8 = 0x70;
const HOVER_ALPHA: u8 = 0x38;
/// Hover highlight tint (selection uses the theme's focus colour).
const HOVER_TINT: u32 = 0xFFFF_FFFF;
/// Desktop folder is re-read at most this often.
const SCAN_INTERVAL_MS: u64 = 1000;
/// Apps shown as shortcuts, in order (names as in the start menu).
const APP_SHORTCUTS: &[&str] = &[
    "Terminal",
    "Files",
    "Calculator",
    "Task Manager",
    "Settings",
];
/// Context-menu action ids owned by the desktop.
pub const ACTION_NEW_FILE: u32 = 30;
/// Create a folder on the desktop.
pub const ACTION_NEW_FOLDER: u32 = 31;
/// Open the selected icon.
pub const ACTION_OPEN: u32 = 40;
/// Rename the selected file icon.
pub const ACTION_RENAME: u32 = 41;
/// Delete the selected file icon.
pub const ACTION_DELETE: u32 = 42;
/// Fallback grid when the framebuffer is not up yet.
const DEFAULT_GRID: (u32, u32) = (16, 8);
/// Name offered by the new-file prompt.
const NEW_FILE_NAME: &str = "New File.txt";
/// Name offered by the new-folder prompt.
const NEW_FOLDER_NAME: &str = "New Folder";

#[derive(Clone)]
enum Kind {
    App(&'static str),
    File { name: String, is_dir: bool },
}

#[derive(Clone)]
struct DesktopIcon {
    kind: Kind,
    cell: Cell,
    icon: Icon,
}

impl DesktopIcon {
    fn key(&self) -> String {
        match &self.kind {
            Kind::App(name) => alloc::format!("app:{}", name),
            Kind::File { name, .. } => alloc::format!("file:{}", name),
        }
    }

    fn label(&self) -> &str {
        match &self.kind {
            Kind::App(name) => name,
            Kind::File { name, .. } => name,
        }
    }
}

/// Dialog the desktop is waiting on.
enum Pending {
    NewItem { is_dir: bool },
    Rename(String),
    Delete(String),
}

/// A press on an icon that may turn into a drag.
struct Drag {
    idx: usize,
    grab: (u32, u32),
    press: (u32, u32),
    active: bool,
    pos: (u32, u32),
}

struct State {
    icons: Vec<DesktopIcon>,
    selected: Option<usize>,
    hovered: Option<usize>,
    clicks: ClickTracker,
    drag: Option<Drag>,
    pending: Option<Pending>,
    saved: Vec<(String, Cell)>,
    layout_loaded: bool,
    listing: Option<Vec<DesktopEntry>>,
    next_scan_ms: u64,
}

static STATE: Spinlock<State> = Spinlock::new(State {
    icons: Vec::new(),
    selected: None,
    hovered: None,
    clicks: ClickTracker::new(),
    drag: None,
    pending: None,
    saved: Vec::new(),
    layout_loaded: false,
    listing: None,
    next_scan_ms: 0,
});

fn redraw() {
    crate::wm::compositor::mark_full_redraw();
}

fn now_ms() -> u64 {
    crate::arch::ArchTimer::uptime_ms()
}

fn grid() -> (u32, u32) {
    let info = fb::device_info();
    if info.width == 0 {
        return DEFAULT_GRID;
    }
    (
        layout::cols_for(info.width),
        layout::rows_for(info.height, crate::wm::panel::PANEL_HEIGHT),
    )
}

/// Give every icon a cell: its saved one when valid and free, else the
/// first free cell. Saved positions are honoured first so a new icon never
/// takes a cell a later icon owns.
fn assign_cells(icons: &mut [DesktopIcon], saved: &[(String, Cell)]) {
    let (cols, rows) = grid();
    let mut used: Vec<Cell> = Vec::new();
    let mut placed = alloc::vec![false; icons.len()];
    for (i, icon) in icons.iter_mut().enumerate() {
        let key = icon.key();
        let want = saved.iter().find(|(k, _)| *k == key).map(|(_, c)| *c);
        if let Some(cell) = want.filter(|c| c.col < cols && c.row < rows && !used.contains(c)) {
            icon.cell = cell;
            used.push(cell);
            placed[i] = true;
        }
    }
    for (icon, _) in icons.iter_mut().zip(&placed).filter(|(_, done)| !**done) {
        icon.cell = layout::first_free(&used, rows);
        used.push(icon.cell);
    }
}

fn app_icons() -> Vec<DesktopIcon> {
    APP_SHORTCUTS
        .iter()
        .map(|name| DesktopIcon {
            kind: Kind::App(name),
            cell: Cell { col: 0, row: 0 },
            icon: icons::for_app(name),
        })
        .collect()
}

fn file_icon(entry: &DesktopEntry) -> DesktopIcon {
    DesktopIcon {
        kind: Kind::File {
            name: entry.name.clone(),
            is_dir: entry.is_dir,
        },
        cell: Cell { col: 0, row: 0 },
        icon: icons::for_file_name(&entry.name, entry.is_dir),
    }
}

fn snapshot(icons: &[DesktopIcon]) -> Vec<(String, Cell)> {
    icons.iter().map(|i| (i.key(), i.cell)).collect()
}

/// Remember current positions so a rebuild keeps icons where they are.
fn remember_positions(st: &mut State) {
    for (key, cell) in snapshot(&st.icons) {
        match st.saved.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = cell,
            None => st.saved.push((key, cell)),
        }
    }
}

/// Rebuild the icon list from the desktop folder. `keep_positions` records
/// where icons currently are first; it is false while applying a layout just
/// loaded from disk, which must win over the default placement.
fn rebuild(st: &mut State, entries: &[DesktopEntry], keep_positions: bool) {
    if keep_positions {
        remember_positions(st);
    }
    let selected_key = st.selected.and_then(|i| st.icons.get(i)).map(|i| i.key());
    let mut icons = app_icons();
    icons.extend(entries.iter().map(file_icon));
    assign_cells(&mut icons, &st.saved);
    st.selected = selected_key.and_then(|k| icons.iter().position(|i| i.key() == k));
    st.hovered = None;
    st.drag = None;
    st.icons = icons;
}

/// Initialize desktop icons with the built-in app shortcuts; the desktop
/// folder is picked up by [`tick`] once the filesystem is usable.
pub fn init() {
    let mut st = STATE.lock();
    rebuild(&mut st, &[], true);
    crate::println!("[desktop] {} desktop icons initialized", st.icons.len());
}

/// Select an icon (single selection).
pub fn select(idx: Option<usize>) {
    STATE.lock().selected = idx;
    redraw();
}

/// Currently selected icon index.
pub fn selected() -> Option<usize> {
    STATE.lock().selected
}

/// Update hover state from cursor movement.
pub fn update_hover(cx: u32, cy: u32) {
    let hit = hit_test(cx, cy);
    let mut st = STATE.lock();
    if st.hovered != hit {
        st.hovered = hit;
        drop(st);
        redraw();
    }
}

fn body_rect(cell: Cell) -> (u32, u32, u32, u32) {
    let (x, y) = cell.origin();
    let pad = SELECT_PAD.0;
    (
        x - pad.min(x),
        y,
        ICON_SIZE + 2 * pad,
        ICON_SIZE + ICON_LABEL_H,
    )
}

/// Icon index under a point, if any.
pub fn hit_test(cx: u32, cy: u32) -> Option<usize> {
    let st = STATE.lock();
    st.icons.iter().position(|icon| {
        let (x, y, w, h) = body_rect(icon.cell);
        cx >= x && cx < x + w && cy >= y && cy < y + h
    })
}

/// Press on an icon: select it, open on a double-click, else arm a drag.
pub fn press(idx: usize, px: u32, py: u32) {
    let open = {
        let mut st = STATE.lock();
        let Some(icon) = st.icons.get(idx).cloned() else {
            return;
        };
        st.selected = Some(idx);
        let is_double = st.clicks.press(idx, now_ms());
        let (ix, iy) = icon.cell.origin();
        st.drag = (!is_double).then_some(Drag {
            idx,
            grab: (px.saturating_sub(ix), py.saturating_sub(iy)),
            press: (px, py),
            active: false,
            pos: (ix, iy),
        });
        is_double.then_some(icon.kind)
    };
    redraw();
    if let Some(kind) = open {
        activate(kind);
    }
}

/// Press on empty desktop: clear selection and any half-finished click.
pub fn press_empty() {
    let mut st = STATE.lock();
    st.selected = None;
    st.clicks.reset();
    st.drag = None;
    drop(st);
    redraw();
}

/// Pointer moved while a press may be dragging an icon.
pub fn drag_move(px: u32, py: u32) {
    let mut st = STATE.lock();
    let Some(drag) = st.drag.as_mut() else {
        return;
    };
    let travelled = px.abs_diff(drag.press.0).max(py.abs_diff(drag.press.1));
    if !drag.active && travelled >= layout::DRAG_THRESHOLD {
        drag.active = true;
    }
    if drag.active {
        drag.pos = (
            px.saturating_sub(drag.grab.0),
            py.saturating_sub(drag.grab.1),
        );
        st.clicks.reset();
        drop(st);
        redraw();
    }
}

/// Button released: drop a dragged icon on the nearest free cell.
pub fn release() {
    let snapshot = {
        let mut st = STATE.lock();
        let Some(drag) = st.drag.take() else {
            return;
        };
        if !drag.active {
            return;
        }
        drop_icon(&mut st, &drag);
        snapshot(&st.icons)
    };
    redraw();
    desktop_files::save_layout(&snapshot);
}

fn drop_icon(st: &mut State, drag: &Drag) {
    let (cols, rows) = grid();
    let target = layout::snap(drag.pos.0, drag.pos.1, cols, rows);
    let others: Vec<Cell> = st
        .icons
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != drag.idx)
        .map(|(_, icon)| icon.cell)
        .collect();
    if let (Some(cell), Some(icon)) = (
        layout::resolve_drop(target, &others),
        st.icons.get_mut(drag.idx),
    ) {
        icon.cell = cell;
    }
}

fn activate(kind: Kind) {
    match kind {
        Kind::App(name) => {
            crate::wm::start_menu::launch_by_name(name);
        }
        Kind::File { name, is_dir } => desktop_files::open(&name, is_dir),
    }
}

/// Enter opens the selected icon when no window has focus.
pub fn handle_key(scancode: u8) -> bool {
    const SC_ENTER: u8 = 0x1C;
    if scancode != SC_ENTER || window::get_info(window::focused_id()).is_some() {
        return false;
    }
    let kind = {
        let st = STATE.lock();
        st.selected
            .and_then(|i| st.icons.get(i))
            .map(|i| i.kind.clone())
    };
    kind.map(activate).is_some()
}

/// Right-click on the desktop: icon menu over an icon, else desktop menu.
pub fn right_click(x: u32, y: u32) {
    let Some(idx) = hit_test(x, y) else {
        context_menu::show_desktop_menu(x, y);
        return;
    };
    let is_file = {
        let mut st = STATE.lock();
        st.selected = Some(idx);
        matches!(st.icons[idx].kind, Kind::File { .. })
    };
    if is_file {
        let items = [
            ("Open", ACTION_OPEN),
            ("Rename", ACTION_RENAME),
            ("Delete", ACTION_DELETE),
        ];
        context_menu::show(x, y, &items);
    } else {
        context_menu::show(x, y, &[("Open", ACTION_OPEN)]);
    }
    redraw();
}

/// Run a desktop context-menu action; false when the id is not ours.
pub fn handle_action(action: u32) -> bool {
    match action {
        ACTION_NEW_FILE => new_item(false),
        ACTION_NEW_FOLDER => new_item(true),
        ACTION_OPEN => open_selected(),
        ACTION_RENAME => rename_selected(),
        ACTION_DELETE => delete_selected(),
        _ => return false,
    }
    true
}

fn open_selected() {
    let kind = {
        let st = STATE.lock();
        st.selected
            .and_then(|i| st.icons.get(i))
            .map(|i| i.kind.clone())
    };
    if let Some(kind) = kind {
        activate(kind);
    }
}

fn selected_file() -> Option<(String, bool)> {
    let st = STATE.lock();
    match &st.icons.get(st.selected?)?.kind {
        Kind::File { name, is_dir } => Some((name.clone(), *is_dir)),
        Kind::App(_) => None,
    }
}

fn unique_name(base: &str) -> String {
    let st = STATE.lock();
    let taken = |n: &str| st.icons.iter().any(|i| i.label() == n);
    if !taken(base) {
        return base.to_string();
    }
    (2u32..)
        .map(|n| alloc::format!("{} {}", base, n))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}

fn new_item(is_dir: bool) {
    let (title, base) = if is_dir {
        ("New Folder", NEW_FOLDER_NAME)
    } else {
        ("New Text File", NEW_FILE_NAME)
    };
    let default = unique_name(base);
    STATE.lock().pending = Some(Pending::NewItem { is_dir });
    dialog::prompt(title, "Name:", &default);
}

fn rename_selected() {
    let Some((name, _)) = selected_file() else {
        return;
    };
    STATE.lock().pending = Some(Pending::Rename(name.clone()));
    dialog::prompt("Rename", "New name:", &name);
}

fn delete_selected() {
    let Some((name, _)) = selected_file() else {
        return;
    };
    let message = alloc::format!("Delete \"{}\"?", name);
    STATE.lock().pending = Some(Pending::Delete(name));
    dialog::confirm("Delete", &message);
}

fn report(result: Result<(), crate::fs::vfs::VfsError>, what: &str) {
    if let Err(err) = result {
        let body = alloc::format!("{} failed: {:?}", what, err);
        crate::wm::notifications::error("Desktop", &body);
    }
}

fn apply_pending(pending: Pending, result: Option<dialog::DialogResult>) {
    use dialog::DialogResult::{Confirm, Prompt};
    let name_ok = |n: &String| desktop_files::valid_name(n.trim());
    match (pending, result) {
        (Pending::NewItem { is_dir }, Some(Prompt(Some(n)))) if name_ok(&n) => {
            report(desktop_files::create(n.trim(), is_dir), "create");
        }
        (Pending::Rename(old), Some(Prompt(Some(n)))) if name_ok(&n) && n.trim() != old => {
            report(desktop_files::rename(&old, n.trim()), "rename");
            move_saved_key(&old, n.trim());
        }
        (Pending::Delete(name), Some(Confirm(true))) => {
            report(desktop_files::delete(&name), "delete");
        }
        _ => return,
    }
    let mut st = STATE.lock();
    st.next_scan_ms = 0;
    st.listing = None;
}

/// A renamed icon keeps its grid cell.
fn move_saved_key(old: &str, new: &str) {
    let mut st = STATE.lock();
    remember_positions(&mut st);
    let old_key = alloc::format!("file:{}", old);
    if let Some(slot) = st.saved.iter_mut().find(|(k, _)| *k == old_key) {
        slot.0 = alloc::format!("file:{}", new);
    }
}

fn process_pending() {
    if dialog::is_open() {
        return;
    }
    let pending = STATE.lock().pending.take();
    if let Some(pending) = pending {
        apply_pending(pending, dialog::take_result());
    }
}

/// Per-frame housekeeping: finish dialogs, and re-read `~/Desktop` once a
/// second so files created from the shell or other apps show up.
pub fn tick() {
    process_pending();
    let now = now_ms();
    {
        let mut st = STATE.lock();
        if now < st.next_scan_ms {
            return;
        }
        st.next_scan_ms = now + SCAN_INTERVAL_MS;
    }
    let Some(entries) = desktop_files::scan() else {
        return;
    };
    let needs_layout = !STATE.lock().layout_loaded;
    let saved = if needs_layout {
        desktop_files::load_layout()
    } else {
        Vec::new()
    };
    let mut st = STATE.lock();
    if needs_layout {
        st.saved = saved;
        st.layout_loaded = true;
        st.listing = None;
    }
    if st.listing.as_ref() == Some(&entries) {
        return;
    }
    rebuild(&mut st, &entries, !needs_layout);
    st.listing = Some(entries);
    drop(st);
    redraw();
}

/// One line per icon for the serial `deskicons` diagnostic.
pub fn describe() -> Vec<String> {
    let st = STATE.lock();
    st.icons
        .iter()
        .enumerate()
        .map(|(i, icon)| {
            let (x, y) = icon.cell.origin();
            alloc::format!(
                "{} {} cell=({},{}) at ({},{})",
                i,
                icon.key(),
                icon.cell.col,
                icon.cell.row,
                x,
                y
            )
        })
        .collect()
}

/// Translucent rounded highlight behind a tile and its label.
fn draw_highlight(fb: &FramebufferInfo, at: (u32, u32), selected: bool, hovered: bool) {
    let (pad, alpha, tint) = if selected {
        (
            SELECT_PAD,
            SELECT_ALPHA,
            theme::current().input_border_focused,
        )
    } else if hovered {
        (HOVER_PAD, HOVER_ALPHA, HOVER_TINT)
    } else {
        return;
    };
    let rect = (
        at.0.saturating_sub(pad.0),
        at.1.saturating_sub(pad.1),
        ICON_SIZE + 2 * pad.0,
        ICON_SIZE + ICON_LABEL_H + 2 * pad.1,
    );
    graphics::blend_rounded_rect(fb, rect, HIGHLIGHT_RADIUS, tint, alpha);
}

fn draw_label(fb: &FramebufferInfo, at: (u32, u32), label: &str) {
    let style = theme::chrome_text(
        Face::Ui,
        LABEL_PX,
        theme::current().title_text,
        ICON_LABEL_H,
    );
    let y = theme::centered_line_y(at.1 + ICON_SIZE + LABEL_GAP, ICON_LABEL_H, &style);
    if text::width(label, &style) > LABEL_MAX_W {
        let x = at.0 + ICON_SIZE / 2 - LABEL_MAX_W / 2;
        text::draw_fitted(fb, x, y, LABEL_MAX_W, label, &style);
    } else {
        text::draw_centered(fb, at.0 + ICON_SIZE / 2, y, label, &style);
    }
}

fn draw_one(
    fb: &FramebufferInfo,
    icon: &DesktopIcon,
    at: (u32, u32),
    selected: bool,
    hovered: bool,
) {
    draw_highlight(fb, at, selected, hovered);
    icons::draw(
        fb,
        icon.icon,
        TILE_SIZE,
        at.0 + TILE_INSET_X,
        at.1 + TILE_INSET_Y,
    );
    draw_label(fb, at, icon.label());
}

/// Draw all desktop icons; a dragged icon is drawn last, under the pointer.
pub fn draw(fb: &FramebufferInfo) {
    let st = STATE.lock();
    let dragged = st.drag.as_ref().filter(|d| d.active);
    for (i, icon) in st.icons.iter().enumerate() {
        if dragged.is_some_and(|d| d.idx == i) {
            continue;
        }
        let hovered = st.hovered == Some(i);
        draw_one(
            fb,
            icon,
            icon.cell.origin(),
            st.selected == Some(i),
            hovered,
        );
    }
    if let Some(drag) = dragged {
        draw_one(fb, &st.icons[drag.idx], drag.pos, true, false);
    }
}
