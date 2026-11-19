//! Draws one frame of the Settings window.

use super::layout::{self, Layout};
use super::model::{Action, Page};
use super::pages::{self, Flow};
use super::state::State;
use crate::display::text::{self, Face};
use crate::wm::controls::{scroll, style, ButtonKind, Rect, Ui, LABEL_PX, SHADOW_ALPHA};
use crate::wm::hit_region::HitKind;
use crate::wm::theme;

/// Height of one sidebar entry and the gap between entries.
const SIDEBAR_ITEM_H: u32 = 40;
const SIDEBAR_GAP: u32 = 4;
/// Padding inside the sidebar.
const SIDEBAR_PAD: i32 = 10;
/// Text size of the page title.
const TITLE_PX: u32 = 24;
/// Alpha of the selected sidebar entry's accent wash.
const SELECTED_ALPHA: u8 = 70;
/// Height and width of the resolution confirm bar.
const CONFIRM_H: u32 = 60;
const CONFIRM_MARGIN: i32 = 16;

/// Draw everything into `ui`, whose view covers the client area.
pub fn frame(ui: &mut Ui, client: (u32, u32), st: &mut State) {
    let l = layout::compute(client.0, client.1);
    ui.fill(Rect::new(0, 0, client.0, client.1), theme::current().win_bg);
    sidebar(ui, &l, st.page);
    title(ui, &l, st.page);
    ui.set_modal(st.resolution_menu.is_open());
    content(ui, &l, st);
    ui.set_view(Rect::new(0, 0, client.0, client.1), (0, 0));
    if st.confirm.is_pending() {
        confirm_bar(ui, client);
    }
}

fn sidebar(ui: &mut Ui, l: &Layout, current: Page) {
    let th = theme::current();
    ui.fill(l.sidebar, th.panel_bg);
    ui.fill(
        Rect::new(l.sidebar.right() - 1, 0, 1, l.sidebar.h),
        th.panel_border,
    );
    for (i, page) in Page::ALL.iter().enumerate() {
        let y = SIDEBAR_PAD + (i as u32 * (SIDEBAR_ITEM_H + SIDEBAR_GAP)) as i32;
        let w = l.sidebar.w - 2 * SIDEBAR_PAD as u32 - 1;
        let item = Rect::new(SIDEBAR_PAD, y, w, SIDEBAR_ITEM_H);
        sidebar_item(ui, item, *page, *page == current);
    }
}

fn sidebar_item(ui: &mut Ui, item: Rect, page: Page, selected: bool) {
    let th = theme::current();
    let key = Action::SelectPage(page).key();
    let look = ui.look(item, true);
    if selected {
        ui.blend(item, 8, theme::accent_color(), SELECTED_ALPHA);
    } else if look != crate::wm::controls::Look::Normal {
        ui.blend(item, 8, th.label_text, 18);
    }
    if ui.has_focus(key) {
        ui.outline(item, theme::accent_color());
        ui.outline(item.inset(1), theme::accent_color());
    }
    let color = if selected {
        th.label_text
    } else {
        th.label_text_dim
    };
    ui.glyph(
        page.glyph(),
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
    ui.text_fitted(item.x + 44, top, item.w - 52, page.name(), &s);
    ui.register(key, item, HitKind::Tab, true);
}

fn title(ui: &Ui, l: &Layout, page: Page) {
    let th = theme::current();
    let s = style(Face::UiBold, TITLE_PX, th.label_text);
    let top = l.title.y + (l.title.h.saturating_sub(text::line_height(&s)) / 2) as i32;
    ui.text(l.content_x, top, page.name(), &s);
}

fn content(ui: &mut Ui, l: &Layout, st: &mut State) {
    let view_h = l.viewport.h;
    st.scroll = scroll::clamp_scroll(st.scroll, st.content_h, view_h);
    ui.set_view(l.viewport, (0, -st.scroll));
    let top = l.viewport.y + 4;
    let mut flow = Flow {
        x: l.content_x,
        y: top,
        w: l.content_w,
    };
    pages::draw(st.page, ui, &mut flow, st);
    st.content_h = flow.height_from(l.viewport.y);
    ui.set_view(l.scrollbar, (0, 0));
    ui.scrollbar(
        Action::PageScroll.key(),
        l.scrollbar,
        st.content_h,
        view_h,
        st.scroll,
    );
}

fn confirm_bar(ui: &mut Ui, client: (u32, u32)) {
    use super::resolution::Outcome;
    let th = theme::current();
    let w = client.0.min(560).saturating_sub(2 * CONFIRM_MARGIN as u32);
    let x = (client.0 - w) as i32 / 2;
    let bar = Rect::new(
        x,
        client.1 as i32 - CONFIRM_H as i32 - CONFIRM_MARGIN,
        w,
        CONFIRM_H,
    );
    ui.blend(bar.offset(0, 3), 10, 0x000000, SHADOW_ALPHA);
    ui.ring(bar, 10, 1, theme::accent_color(), th.menu_bg);
    let secs = match super::state::get()
        .confirm
        .peek(crate::arch::ArchTimer::uptime_ms())
    {
        Outcome::Waiting { secs_left } => secs_left,
        _ => 0,
    };
    let msg = alloc::format!("Keep this resolution? Reverting in {secs} s");
    let s = style(Face::UiBold, LABEL_PX, th.label_text);
    let top = bar.y + (bar.h.saturating_sub(text::line_height(&s)) / 2) as i32;
    ui.text_fitted(bar.x + 16, top, w.saturating_sub(250), &msg, &s);
    let keep = Rect::new(bar.right() - 224, bar.y + 14, 100, 32);
    let revert = Rect::new(bar.right() - 112, bar.y + 14, 100, 32);
    ui.button(
        Action::ResolutionKeep.key(),
        keep,
        "Keep",
        ButtonKind::Primary,
        true,
    );
    ui.button(
        Action::ResolutionRevert.key(),
        revert,
        "Revert",
        ButtonKind::Secondary,
        true,
    );
}
