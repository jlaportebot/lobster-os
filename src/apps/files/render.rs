//! Draws one frame of the Files window.

use super::layout::{self, Layout};
use super::model::{self, Entry, SortKey, ViewMode};
use super::nav::{self, Place};
use super::paths;
use super::state::State;
use super::targets::Target;
use crate::display::icons::{Glyph, Icon};
use crate::display::text::{self, Face};
use crate::wm::controls::toolbar::SEP_W;
use crate::wm::controls::{scroll, style, ButtonKind, Column, CrumbItem, Rect, Ui, LABEL_PX};
use crate::wm::hit_region::HitKind;
use crate::wm::theme;
use alloc::string::String;
use alloc::vec::Vec;

/// Side of a place's icon in the sidebar.
const PLACE_ICON: u32 = 24;
/// Left padding of a place's label after its icon.
const PLACE_TEXT_X: i32 = 44;
/// Alpha of the wash behind the current place.
const PLACE_ALPHA: u8 = 70;
/// Alpha of the dimming veil over disabled places.
const DISABLED_VEIL: u8 = 150;
/// Space left of the status text.
const STATUS_PAD: i32 = 12;
/// Text size of the status bar.
const STATUS_PX: u32 = LABEL_PX - 1;
/// Text size of the empty-folder notice.
const EMPTY_PX: u32 = LABEL_PX + 1;
/// Padding inside the breadcrumb field.
const CRUMB_FIELD_PAD: u32 = 8;

/// Draw everything into `ui`, whose view covers the client area.
pub fn frame(ui: &mut Ui, st: &mut State, client: (u32, u32)) {
    let l = layout::compute(client.0, client.1, st.view);
    st.layout = Some(l);
    let th = theme::current();
    ui.fill(Rect::new(0, 0, client.0, client.1), th.win_bg);
    items(ui, &l, st);
    ui.set_view(Rect::new(0, 0, client.0, client.1), (0, 0));
    toolbar(ui, &l, st);
    sidebar(ui, &l, st);
    status(ui, &l, st);
}

fn toolbar(ui: &mut Ui, l: &Layout, st: &State) {
    let th = theme::current();
    ui.fill(l.toolbar, th.panel_bg);
    ui.fill(
        Rect::new(0, l.toolbar.bottom() - 1, l.toolbar.w, 1),
        th.panel_border,
    );
    ui.icon_button(
        Target::Back.key(),
        l.back,
        Glyph::ArrowLeft,
        st.history.can_back(),
    );
    ui.icon_button(
        Target::Forward.key(),
        l.forward,
        Glyph::ArrowRight,
        st.history.can_forward(),
    );
    ui.icon_button(
        Target::Up.key(),
        l.up,
        Glyph::ArrowUp,
        st.cwd() != paths::ROOT,
    );
    breadcrumbs(ui, l.crumbs, st.cwd());
    ui.button(
        Target::NewFolder.key(),
        l.new_folder,
        "New folder",
        ButtonKind::Secondary,
        true,
    );
    let keys = [Target::ViewList.key(), Target::ViewIcons.key()];
    let selected = usize::from(st.view == ViewMode::Icons);
    ui.segmented(&keys, &["List", "Icons"], l.view_toggle, selected);
}

fn breadcrumbs(ui: &mut Ui, r: Rect, cwd: &str) {
    let crumbs = paths::crumbs(cwd);
    let widths: Vec<u32> = crumbs.iter().map(|c| Ui::crumb_width(&c.label)).collect();
    let avail = r.w.saturating_sub(CRUMB_FIELD_PAD);
    let first = paths::first_visible_crumb(&widths, SEP_W, Ui::crumb_width("\u{2026}"), avail);
    let mut items: Vec<CrumbItem> = Vec::new();
    if first > 0 {
        items.push(CrumbItem {
            key: Target::Crumb(first - 1).key(),
            label: "\u{2026}",
        });
    }
    for (i, crumb) in crumbs.iter().enumerate().skip(first) {
        items.push(CrumbItem {
            key: Target::Crumb(i).key(),
            label: &crumb.label,
        });
    }
    ui.breadcrumbs(r, &items);
}

fn sidebar(ui: &mut Ui, l: &Layout, st: &State) {
    let th = theme::current();
    ui.fill(l.sidebar, th.panel_bg);
    ui.fill(
        Rect::new(l.sidebar.right() - 1, l.sidebar.y, 1, l.sidebar.h),
        th.panel_border,
    );
    let places = nav::places();
    for (i, place) in places.iter().enumerate() {
        let r = layout::place_rect(l, i);
        place_row(ui, r, place, place.path == st.cwd(), Some(Target::Place(i)));
    }
    let trash = Place {
        label: nav::TRASH_LABEL,
        path: String::new(),
        icon: Icon::FileTrashEmpty,
        enabled: false,
        create: false,
    };
    place_row(ui, layout::place_rect(l, places.len()), &trash, false, None);
}

fn place_row(ui: &mut Ui, r: Rect, place: &Place, current: bool, target: Option<Target>) {
    let th = theme::current();
    if current {
        ui.blend(r, 8, theme::accent_color(), PLACE_ALPHA);
    } else if target.is_some() && ui.look(r, true) != crate::wm::controls::Look::Normal {
        ui.blend(r, 8, th.label_text, 22);
    }
    let icon_box = Rect::new(r.x + 8, r.y, PLACE_ICON, r.h);
    ui.icon(place.icon, PLACE_ICON, icon_box);
    let color = if place.enabled {
        th.label_text
    } else {
        th.label_text_dim
    };
    let s = style(
        if current { Face::UiBold } else { Face::Ui },
        LABEL_PX,
        color,
    );
    let top = r.y + (r.h.saturating_sub(text::line_height(&s)) / 2) as i32;
    ui.text_fitted(
        r.x + PLACE_TEXT_X,
        top,
        r.w.saturating_sub(PLACE_TEXT_X as u32 + 6),
        place.label,
        &s,
    );
    if !place.enabled {
        ui.blend(icon_box, 0, th.panel_bg, DISABLED_VEIL);
    }
    if let (Some(target), true) = (target, place.enabled) {
        ui.register(target.key(), r, HitKind::Tab, false);
    }
}

fn items(ui: &mut Ui, l: &Layout, st: &mut State) {
    let n = st.entries.len();
    st.cols = layout::view_cols(st.view, l.body.w);
    st.content_h = layout::content_h(st.view, n, st.cols);
    if let Some(i) = st.reveal.take() {
        let r = layout::item_rect(st.view, i, st.cols, l.body.w);
        st.scroll = layout::scroll_to_show(st.scroll, r.y, r.h, l.body.h);
    }
    st.scroll = scroll::clamp_scroll(st.scroll, st.content_h, l.body.h);
    if st.view == ViewMode::List {
        ui.set_view(l.header, (0, 0));
        header(ui, l, st);
    }
    ui.set_view(l.body, (0, -st.scroll));
    ui.register(Target::Body.key(), ui.view_extent(), HitKind::Custom, false);
    body(ui, l, st);
    ui.set_view(l.scrollbar, (0, 0));
    ui.scrollbar(
        Target::Scrollbar.key(),
        l.scrollbar,
        st.content_h,
        l.body.h,
        st.scroll,
    );
}

fn header(ui: &mut Ui, l: &Layout, st: &State) {
    let c = layout::columns(l.body.w);
    let mk = |key: SortKey, col: Option<layout::Column>, label: &'static str, right: bool| {
        col.map(|(x, w)| Column {
            key: Target::Header(key).key(),
            x,
            w,
            label,
            sorted: (st.sort.key == key).then_some(st.sort.descending),
            right,
        })
    };
    let cols: Vec<Column> = [
        mk(SortKey::Name, Some(c.name), "Name", false),
        mk(SortKey::Size, c.size, "Size", true),
        mk(SortKey::Modified, c.modified, "Modified", false),
        mk(SortKey::Type, c.kind, "Type", false),
    ]
    .into_iter()
    .flatten()
    .collect();
    ui.column_header(
        Rect::new(l.header.x, l.header.y, l.header.w, l.header.h),
        &cols,
    );
}

fn body(ui: &mut Ui, l: &Layout, st: &State) {
    if st.entries.is_empty() {
        empty_notice(ui, l);
        return;
    }
    for (i, entry) in st.entries.iter().enumerate() {
        let cell = layout::item_rect(st.view, i, st.cols, l.body.w).offset(l.body.x, l.body.y);
        if ui.visible(cell).is_none() {
            continue;
        }
        let selected = st.selection.contains(i);
        let icon = crate::display::icons::for_file_name(&entry.name, entry.is_dir);
        match st.view {
            ViewMode::List => list_row(ui, cell, entry, icon, selected, l.body.w),
            ViewMode::Icons => ui.item_tile(cell, icon, &entry.name, selected),
        }
    }
}

fn list_row(ui: &Ui, row: Rect, entry: &Entry, icon: Icon, selected: bool, view_w: u32) {
    let c = layout::columns(view_w);
    let size = if entry.is_dir {
        String::from("\u{2014}")
    } else {
        model::size_text(entry.size)
    };
    let local = i64::from(crate::clock::utc_offset_secs());
    let when = model::date_text((entry.mtime as i64 + local).max(0) as u64);
    let mut cells: Vec<(i32, u32, &str, bool)> = Vec::new();
    if let Some((x, w)) = c.size {
        cells.push((x, w, &size, true));
    }
    if let Some((x, w)) = c.modified {
        cells.push((x, w, &when, false));
    }
    if let Some((x, w)) = c.kind {
        cells.push((x, w, model::type_label(entry), false));
    }
    ui.item_row(row, icon, (&entry.name, c.name.1), &cells, selected);
}

fn empty_notice(ui: &Ui, l: &Layout) {
    let s = style(Face::Ui, EMPTY_PX, theme::current().label_text_dim);
    let r = Rect::new(l.body.x, l.body.y + 24, l.body.w, text::line_height(&s));
    ui.text_centered(r, "This folder is empty", &s);
}

fn status(ui: &mut Ui, l: &Layout, st: &State) {
    let th = theme::current();
    ui.fill(l.status, th.panel_bg);
    ui.fill(Rect::new(0, l.status.y, l.status.w, 1), th.panel_border);
    let s = style(Face::Ui, STATUS_PX, th.label_text_dim);
    let top = l.status.y + (l.status.h.saturating_sub(text::line_height(&s)) / 2) as i32;
    let left = status_text(st);
    let free = free_text();
    let free_w = text::width(&free, &s);
    let room = l.status.w.saturating_sub(free_w + 3 * STATUS_PAD as u32);
    ui.text_fitted(STATUS_PAD, top, room, &left, &s);
    ui.text(
        l.status.right() - STATUS_PAD - free_w as i32,
        top,
        &free,
        &s,
    );
}

fn status_text(st: &State) -> String {
    let n = st.entries.len();
    let noun = if n == 1 { "item" } else { "items" };
    let picked = st.selection.count();
    if picked == 0 {
        return alloc::format!("{n} {noun}");
    }
    let bytes = nav::selected_bytes(st);
    alloc::format!(
        "{n} {noun}  |  {picked} selected ({})",
        model::size_text(bytes)
    )
}

fn free_text() -> String {
    match crate::fs::ext2::usage() {
        Some(u) => {
            let block = u64::from(u.block_size);
            let (free, total) = (u.blocks_free * block, u.blocks_total * block);
            alloc::format!(
                "Disk: {} free of {}",
                model::size_text(free),
                model::size_text(total)
            )
        }
        None => String::from("No disk mounted"),
    }
}
