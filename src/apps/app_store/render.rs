//! Draws one frame of the App Store window.

use super::actions::Action;
use super::job::{Job, Stage};
use super::layout::{self, Layout};
use super::model::{self, Entry, Section, Status};
use super::state::State;
use crate::display::icons::Glyph;
use crate::display::text::{self, Face};
use crate::wm::controls::{scroll, style, ButtonKind, Look, Rect, Ui, LABEL_PX, PROGRESS_H};
use crate::wm::hit_region::HitKind;
use crate::wm::theme;
use alloc::format;
use alloc::string::String;

const SIDEBAR_ITEM_H: u32 = 38;
const SIDEBAR_GAP: u32 = 4;
const SIDEBAR_PAD: i32 = 10;
const TITLE_PX: u32 = 24;
const NAME_PX: u32 = 15;
const DESC_PX: u32 = 13;
const SELECTED_ALPHA: u8 = 70;
const HOVER_ALPHA: u8 = 18;
/// Edge of the app icon in a row and in the details pane.
const ROW_ICON: u32 = 48;
const DETAILS_ICON: u32 = 64;
const ICON_GAP: i32 = 14;
const BADGE_H: u32 = 20;
const OFFLINE_RED: u32 = 0xFFD9534F;
const OFFLINE_ALPHA: u8 = 46;
const DETAILS_PAD: i32 = 20;
const DETAILS_BUTTON_H: u32 = 34;
const LINE_GAP: i32 = 6;
/// Width of the details pane's Close button.
const CLOSE_BUTTON_W: u32 = 88;

/// Draw everything into `ui`, whose view covers the client area.
pub fn frame(ui: &mut Ui, client: (u32, u32), st: &mut State) {
    let l = layout::compute(client.0, client.1, st.selected.is_some(), st.job.is_some());
    ui.fill(Rect::new(0, 0, client.0, client.1), theme::current().win_bg);
    sidebar(ui, &l, st);
    header(ui, &l, st);
    status(ui, &l, st);
    list(ui, &l, st);
    ui.set_view(Rect::new(0, 0, client.0, client.1), (0, 0));
    if let (Some(pane), Some(entry)) = (l.details, st.selected_entry().cloned()) {
        details(ui, pane, &entry, st.busy());
    }
    if let (Some(bar), Some(job)) = (l.job_bar, st.job.as_ref()) {
        job_bar(ui, bar, job);
    }
}

fn sidebar(ui: &mut Ui, l: &Layout, st: &State) {
    let th = theme::current();
    ui.fill(l.sidebar, th.panel_bg);
    ui.fill(
        Rect::new(l.sidebar.right() - 1, 0, 1, l.sidebar.h),
        th.panel_border,
    );
    let updates = model::update_count(&st.entries);
    for (i, section) in model::sections(&st.entries).iter().enumerate() {
        let y = SIDEBAR_PAD + (i as u32 * (SIDEBAR_ITEM_H + SIDEBAR_GAP)) as i32;
        let w = l.sidebar.w - 2 * SIDEBAR_PAD as u32 - 1;
        let item = Rect::new(SIDEBAR_PAD, y, w, SIDEBAR_ITEM_H);
        let badge = if *section == Section::Updates {
            updates
        } else {
            0
        };
        sidebar_item(ui, item, (i, section), *section == st.section, badge);
    }
}

fn section_glyph(section: &Section) -> Glyph {
    match section {
        Section::Discover => Glyph::Grid,
        Section::Category(_) => Glyph::Folder,
        Section::Installed => Glyph::Check,
        Section::Updates => Glyph::Refresh,
    }
}

fn sidebar_item(ui: &mut Ui, item: Rect, entry: (usize, &Section), selected: bool, badge: usize) {
    let th = theme::current();
    let key = Action::Section(entry.0 as u16).key();
    if selected {
        ui.blend(item, 8, theme::accent_color(), SELECTED_ALPHA);
    } else if ui.look(item, true) != Look::Normal {
        ui.blend(item, 8, th.label_text, HOVER_ALPHA);
    }
    if ui.has_focus(key) {
        ui.outline(item, theme::accent_color());
    }
    let color = if selected {
        th.label_text
    } else {
        th.label_text_dim
    };
    ui.glyph(
        section_glyph(entry.1),
        20,
        Rect::new(item.x + 8, item.y, 28, item.h),
        color,
    );
    let s = style(
        if selected { Face::UiBold } else { Face::Ui },
        LABEL_PX + 1,
        color,
    );
    let top = item.y + (item.h.saturating_sub(text::line_height(&s)) / 2) as i32;
    let reserve = if badge > 0 { 36 } else { 8 };
    ui.text_fitted(item.x + 44, top, item.w - 44 - reserve, entry.1.title(), &s);
    if badge > 0 {
        count_badge(ui, item, badge);
    }
    ui.register(key, item, HitKind::Tab, true);
}

fn count_badge(ui: &Ui, item: Rect, count: usize) {
    let label = format!("{count}");
    let s = style(
        Face::UiBold,
        DESC_PX,
        crate::wm::controls::on_color(theme::accent_color()),
    );
    let w = (text::width(&label, &s) + 14).max(BADGE_H);
    let pill = Rect::new(
        item.right() - 8 - w as i32,
        item.y + (item.h.saturating_sub(BADGE_H) / 2) as i32,
        w,
        BADGE_H,
    );
    ui.rounded(pill, BADGE_H / 2, theme::accent_color());
    ui.text_centered(pill, &label, &s);
}

fn header(ui: &mut Ui, l: &Layout, st: &State) {
    let th = theme::current();
    let s = style(Face::UiBold, TITLE_PX, th.label_text);
    let top = l.title.y + (l.title.h.saturating_sub(text::line_height(&s)) / 2) as i32;
    ui.text_fitted(l.title.x, top, l.title.w, st.section.title(), &s);
    ui.text_field(Action::Search.key(), l.search, &st.query, "Search apps");
    ui.register(Action::Search.key(), l.search, HitKind::TextInput, true);
    let enabled = !st.busy();
    ui.button(
        Action::Refresh.key(),
        l.refresh,
        "Refresh",
        ButtonKind::Secondary,
        enabled,
    );
}

fn status(ui: &Ui, l: &Layout, st: &State) {
    let th = theme::current();
    let r = l.status;
    let (text_color, line) = if st.offline {
        ui.blend(
            Rect::new(r.x + 12, r.y + 2, r.w.saturating_sub(24), r.h - 4),
            6,
            OFFLINE_RED,
            OFFLINE_ALPHA,
        );
        (
            th.label_text,
            "No connection. Showing installed apps only; press Refresh to retry.".into(),
        )
    } else {
        (th.label_text_dim, status_line(st))
    };
    let s = style(Face::Ui, DESC_PX, text_color);
    let top = r.y + (r.h.saturating_sub(text::line_height(&s)) / 2) as i32;
    ui.text_fitted(
        r.x + layout::PAD,
        top,
        r.w.saturating_sub(2 * layout::PAD as u32),
        &line,
        &s,
    );
}

fn status_line(st: &State) -> String {
    if !st.notice.is_empty() {
        return st.notice.clone();
    }
    let shown = model::visible(&st.entries, &st.section, &st.query).len();
    let word = if shown == 1 { "app" } else { "apps" };
    format!("{shown} {word}")
}

fn list(ui: &mut Ui, l: &Layout, st: &mut State) {
    if l.list.w == 0 {
        return;
    }
    let shown = model::visible(&st.entries, &st.section, &st.query);
    st.content_h = layout::content_height(shown.len());
    st.scroll = scroll::clamp_scroll(st.scroll, st.content_h, l.list.h);
    ui.set_view(l.list, (0, -st.scroll));
    if shown.is_empty() {
        empty_state(ui, l.list, st);
    }
    for (pos, &index) in shown.iter().enumerate() {
        let row = layout::row_rect(l.list, pos, 0);
        let selected = st.selected.as_deref() == Some(st.entries[index].name.as_str());
        entry_row(
            ui,
            row,
            (index, &st.entries[index]),
            selected,
            busy_on(st, &st.entries[index]),
        );
    }
    ui.set_view(l.scrollbar, (0, 0));
    ui.scrollbar(
        Action::ListScroll.key(),
        l.scrollbar,
        st.content_h,
        l.list.h,
        st.scroll,
    );
}

fn busy_on(st: &State, entry: &Entry) -> Option<&'static str> {
    let job = st
        .job
        .as_ref()
        .filter(|j| j.is_active() && j.name == entry.name)?;
    Some(job.label())
}

fn empty_state(ui: &Ui, view: Rect, st: &State) {
    let th = theme::current();
    let msg = if !st.query.trim().is_empty() {
        "No apps match your search."
    } else if st.section == Section::Updates {
        "Everything is up to date."
    } else if st.section == Section::Installed {
        "No apps installed yet."
    } else {
        "The catalogue is empty."
    };
    let s = style(Face::Ui, LABEL_PX + 1, th.label_text_dim);
    ui.text_fitted(
        view.x + layout::PAD,
        view.y + 24,
        view.w - 2 * layout::PAD as u32,
        msg,
        &s,
    );
}

fn entry_row(ui: &mut Ui, row: Rect, entry: (usize, &Entry), selected: bool, busy: Option<&str>) {
    let th = theme::current();
    let (index, e) = entry;
    let wash = Rect::new(row.x + 8, row.y + 2, row.w.saturating_sub(16), row.h - 4);
    if selected {
        ui.blend(wash, 8, theme::accent_color(), SELECTED_ALPHA);
    } else if ui.look(row, true) != Look::Normal {
        ui.blend(wash, 8, th.label_text, HOVER_ALPHA);
    }
    let icon_x = row.x + layout::PAD;
    ui.icon(e.icon, ROW_ICON, Rect::new(icon_x, row.y, ROW_ICON, row.h));
    let text_x = icon_x + ROW_ICON as i32 + ICON_GAP;
    let text_w = (layout::row_button(row).x - text_x - ICON_GAP).max(0) as u32;
    row_text(ui, Rect::new(text_x, row.y, text_w, row.h), e);
    ui.register(Action::Row(index as u16).key(), row, HitKind::Tab, false);
    row_button(ui, row, index, e, busy);
}

fn row_text(ui: &Ui, area: Rect, e: &Entry) {
    let th = theme::current();
    let name_s = style(Face::UiBold, NAME_PX, th.label_text);
    let desc_s = style(Face::Ui, DESC_PX, th.label_text_dim);
    let block = text::line_height(&name_s) + text::line_height(&desc_s) + LINE_GAP as u32;
    let top = area.y + (area.h.saturating_sub(block) / 2) as i32;
    let name_w = ui.text(area.x, top, &e.name, &name_s);
    let meta = meta_line(e);
    let meta_x = area.x + name_w as i32 + 10;
    let meta_room = (area.right() - meta_x).max(0) as u32;
    ui.text_fitted(meta_x, top + 1, meta_room, &meta, &desc_s);
    let desc_y = top + (text::line_height(&name_s) as i32) + LINE_GAP;
    ui.text_fitted(area.x, desc_y, area.w, &e.description, &desc_s);
}

fn meta_line(e: &Entry) -> String {
    let version = match (&e.installed, e.version.is_empty()) {
        (Some(have), true) => have.clone(),
        _ => e.version.clone(),
    };
    format!("{version}  -  {}", model::format_size(e.size))
}

fn row_button(ui: &mut Ui, row: Rect, index: usize, e: &Entry, busy: Option<&str>) {
    let r = layout::row_button(row);
    let key = Action::RowButton(index as u16).key();
    if let Some(stage) = busy {
        ui.button(key, r, stage, ButtonKind::Secondary, false);
        return;
    }
    let kind = match e.status() {
        Status::Installed => ButtonKind::Secondary,
        Status::Available | Status::UpdateAvailable => ButtonKind::Primary,
    };
    ui.button(key, r, e.status().button(), kind, true);
}

fn details(ui: &mut Ui, pane: Rect, e: &Entry, busy: bool) {
    let th = theme::current();
    ui.fill(pane, th.win_bg);
    ui.fill(Rect::new(pane.x, pane.y, 1, pane.h), th.panel_border);
    let x = pane.x + DETAILS_PAD;
    let w = pane.w.saturating_sub(2 * DETAILS_PAD as u32);
    ui.icon(
        e.icon,
        DETAILS_ICON,
        Rect::new(x, pane.y + 12, DETAILS_ICON, DETAILS_ICON),
    );
    let title = style(Face::UiBold, TITLE_PX - 2, th.label_text);
    let tx = x + DETAILS_ICON as i32 + ICON_GAP;
    ui.text_fitted(
        tx,
        pane.y + 20,
        w.saturating_sub(DETAILS_ICON + ICON_GAP as u32),
        &e.name,
        &title,
    );
    let sub = style(Face::Ui, DESC_PX, th.label_text_dim);
    ui.text_fitted(
        tx,
        pane.y + 20 + text::line_height(&title) as i32 + 2,
        w,
        &e.category,
        &sub,
    );
    let y = detail_rows(ui, (x, pane.y + 12 + DETAILS_ICON as i32 + 16, w), e);
    detail_buttons(ui, pane, y, e, busy);
}

fn detail_rows(ui: &Ui, at: (i32, i32, u32), e: &Entry) -> i32 {
    let th = theme::current();
    let (x, mut y, w) = at;
    let body = style(Face::Ui, LABEL_PX, th.label_text);
    let dim = style(Face::Ui, DESC_PX, th.label_text_dim);
    let text = if e.details.is_empty() {
        &e.description
    } else {
        &e.details
    };
    let measure = |t: &str| text::width(t, &body);
    for line in crate::wm::controls::item_view::wrap_lines(text, w, &measure, DETAILS_LINES) {
        ui.text_fitted(x, y, w, &line, &body);
        y += text::line_height(&body) as i32 + 2;
    }
    y += 10;
    for (label, value) in facts(e) {
        ui.text_fitted(x, y, w, &format!("{label}: {value}"), &dim);
        y += text::line_height(&dim) as i32 + 4;
    }
    y + 8
}

/// Longest description the details pane wraps.
const DETAILS_LINES: usize = 7;

fn facts(e: &Entry) -> alloc::vec::Vec<(&'static str, String)> {
    let installed = e
        .installed
        .clone()
        .unwrap_or_else(|| "not installed".into());
    let deps = if e.dependencies.is_empty() {
        "none".into()
    } else {
        e.dependencies.join(", ")
    };
    let mut rows = alloc::vec![
        (
            "Version",
            if e.version.is_empty() {
                "unknown".into()
            } else {
                e.version.clone()
            }
        ),
        ("Installed version", installed),
        ("Download size", model::format_size(e.size)),
    ];
    if e.installed_size > 0 {
        rows.push(("On disk", model::format_size(e.installed_size)));
    }
    rows.push(("Dependencies", deps));
    rows
}

fn detail_buttons(ui: &mut Ui, pane: Rect, y: i32, e: &Entry, busy: bool) {
    let x = pane.x + DETAILS_PAD;
    let w = pane.w.saturating_sub(2 * DETAILS_PAD as u32);
    let first = Rect::new(x, y, 112, DETAILS_BUTTON_H);
    let label = e.status().button();
    ui.button(
        Action::DetailsPrimary.key(),
        first,
        label,
        ButtonKind::Primary,
        !busy,
    );
    let mut next = first.right() + 8;
    if e.installed.is_some() {
        let r = Rect::new(next, y, 100, DETAILS_BUTTON_H);
        ui.button(
            Action::DetailsRemove.key(),
            r,
            "Remove",
            ButtonKind::Danger,
            !busy,
        );
        next = r.right() + 8;
    }
    let close = Rect::new(
        next,
        y,
        (x + w as i32 - next).max(72) as u32,
        DETAILS_BUTTON_H,
    );
    ui.button(
        Action::DetailsClose.key(),
        close,
        "Close",
        ButtonKind::Secondary,
        true,
    );
}

fn job_bar(ui: &mut Ui, bar: Rect, job: &Job) {
    let th = theme::current();
    ui.fill(bar, th.panel_bg);
    ui.fill(Rect::new(bar.x, bar.y, bar.w, 1), th.panel_border);
    let title = match job.op {
        super::job::Op::Refresh => String::from("Catalogue"),
        _ => job.name.clone(),
    };
    let head = style(Face::UiBold, LABEL_PX, th.label_text);
    ui.text_fitted(bar.x + layout::PAD, bar.y + 10, bar.w / 2, &title, &head);
    let stage = style(Face::Ui, LABEL_PX, stage_color(job));
    let line = job.error().unwrap_or_else(|| stage_line(job));
    ui.text_fitted(
        bar.x + layout::PAD,
        bar.y + 32,
        bar.w.saturating_sub(180),
        &line,
        &stage,
    );
    progress(
        ui,
        Rect::new(bar.right() - 150, bar.y + 12, 130, PROGRESS_H),
        job,
    );
    if !job.is_active() {
        let r = Rect::new(bar.right() - 110, bar.y + 26, 90, 28);
        ui.button(
            Action::DismissJob.key(),
            r,
            "Dismiss",
            ButtonKind::Secondary,
            true,
        );
    }
}

fn stage_color(job: &Job) -> u32 {
    match job.stage {
        Stage::Failed(_) => OFFLINE_RED,
        _ => theme::current().label_text_dim,
    }
}

fn stage_line(job: &Job) -> String {
    let (done, total) = job.progress();
    match job.stage {
        Stage::Done => job.outcome().unwrap_or_default(),
        _ => format!("{}... (step {} of {})", job.label(), done + 1, total),
    }
}

fn progress(ui: &Ui, r: Rect, job: &Job) {
    let th = theme::current();
    let (done, total) = job.progress();
    ui.rounded(r, r.h / 2, th.progress_track);
    let failed = matches!(job.stage, Stage::Failed(_));
    let fill = if failed {
        OFFLINE_RED
    } else {
        theme::accent_color()
    };
    let w = r.w * done / total.max(1);
    if w > 0 {
        ui.rounded(Rect::new(r.x, r.y, w.max(r.h), r.h), r.h / 2, fill);
    }
}
