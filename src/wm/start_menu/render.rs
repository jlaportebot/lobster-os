//! Start-menu drawing. Every clickable rectangle is registered while it is
//! drawn, so what the user sees and what a click hits cannot drift apart.

use alloc::string::String;

use super::layout::{self, Layout, Rect};
use super::model::{self, Item, Mode, PowerAction};
use super::{screen_layout, state, visible_list, Target};
use crate::display::fb::FramebufferInfo;
use crate::display::graphics::{blend_rect, blend_rounded_rect, fill_circle, fill_rounded_rect};
use crate::display::icons::{self, Glyph};
use crate::display::text::{self, Face, TextStyle};
use crate::wm::theme::{self, Theme};

/// Surface opacity: translucent enough to hint at the desktop behind it.
const SURFACE_ALPHA: u8 = 0xF5;
/// Layers of the soft shadow and the opacity of each.
const SHADOW_STEPS: u32 = 6;
const SHADOW_ALPHA: u8 = 10;
/// Opacity of the selection / hover wash.
const HIGHLIGHT_ALPHA: u8 = 70;
/// Opacity of the scrollbar track.
const FAINT_ALPHA: u8 = 40;
/// Opacity of the line above the user bar.
const SEPARATOR_ALPHA: u8 = 80;
/// Corner radius of tiles, rows and fields.
const ITEM_RADIUS: u32 = 8;
/// Text sizes.
const NAME_SIZE_GRID: u32 = 12;
const NAME_SIZE_LIST: u32 = 14;
const SECTION_SIZE: u32 = 15;
const POWER_ICON: u32 = 20;
const SEARCH_ICON: u32 = 16;
const WHITE: u32 = 0xFFFF_FFFF;
const BLACK: u32 = 0xFF00_0000;
/// Gap between an icon and its label.
const ICON_GAP: u32 = 12;
/// Caret width and the room left after the typed text.
const CARET_WIDTH: u32 = 1;

fn style(face: Face, size: u32, color: u32) -> TextStyle {
    TextStyle { face, size, color }
}

/// Draw `text` at `x`, vertically centred in `rect`, clipped to `max_w`.
fn draw_label(fb: &FramebufferInfo, rect: Rect, x: u32, max_w: u32, text: &str, st: &TextStyle) {
    let y = rect.y + rect.h.saturating_sub(text::line_height(st)) / 2;
    text::draw_fitted(fb, x, y, max_w, text, st);
}

fn draw_shadow(fb: &FramebufferInfo, menu: Rect) {
    for step in (1..=SHADOW_STEPS).rev() {
        let rect = (
            menu.x.saturating_sub(step),
            menu.y.saturating_sub(step),
            menu.w + 2 * step,
            menu.h + 2 * step,
        );
        blend_rounded_rect(fb, rect, layout::MENU_RADIUS + step, BLACK, SHADOW_ALPHA);
    }
}

fn draw_surface(fb: &FramebufferInfo, theme: &Theme, menu: Rect) {
    draw_shadow(fb, menu);
    let rect = (menu.x, menu.y, menu.w, menu.h);
    blend_rounded_rect(fb, rect, layout::MENU_RADIUS, theme.menu_bg, SURFACE_ALPHA);
}

fn draw_search(fb: &FramebufferInfo, theme: &Theme, layout: &Layout) {
    let field = layout.search;
    let accent = theme::accent_argb();
    fill_rounded_rect(fb, field.x, field.y, field.w, field.h, ITEM_RADIUS, accent);
    fill_rounded_rect(
        fb,
        field.x + 1,
        field.y + 1,
        field.w - 2,
        field.h - 2,
        ITEM_RADIUS - 1,
        theme.input_bg,
    );
    let icon_y = field.y + (field.h - SEARCH_ICON) / 2;
    icons::draw_glyph(
        fb,
        Glyph::Search,
        SEARCH_ICON,
        field.x + 12,
        icon_y,
        theme.label_text_dim,
    );
    let text_x = field.x + 12 + SEARCH_ICON + 8;
    let max_w = field.w.saturating_sub(text_x - field.x + 12);
    let query = &state().nav.query;
    if query.is_empty() {
        let hint = style(Face::Ui, NAME_SIZE_LIST, theme.label_text_dim);
        draw_label(fb, field, text_x, max_w, "Type to search", &hint);
        draw_caret(fb, theme, field, text_x);
        return;
    }
    let typed = style(Face::Ui, NAME_SIZE_LIST, theme.label_text);
    draw_label(fb, field, text_x, max_w, query, &typed);
    let caret_x = text_x + text::width(query, &typed).min(max_w);
    draw_caret(fb, theme, field, caret_x);
}

fn draw_caret(fb: &FramebufferInfo, theme: &Theme, field: Rect, x: u32) {
    let height = field.h - 16;
    blend_rect(
        fb,
        x,
        field.y + 8,
        CARET_WIDTH,
        height,
        theme.label_text,
        0xFF,
    );
}

fn draw_section(fb: &FramebufferInfo, theme: &Theme, layout: &Layout) {
    let mode = state().nav.mode();
    let title = match mode {
        Mode::Pinned => "Pinned",
        Mode::AllApps => "All apps",
        Mode::Results => "Results",
    };
    let bold = style(Face::UiBold, SECTION_SIZE, theme.label_text);
    draw_label(
        fb,
        layout.section_title,
        layout.section_title.x + 4,
        layout.section_title.w / 2,
        title,
        &bold,
    );
    if mode == Mode::Results {
        return;
    }
    let button = layout.section_button;
    let (label, glyph) = if mode == Mode::Pinned {
        ("All apps", Glyph::ChevronRight)
    } else {
        ("Back", Glyph::ChevronLeft)
    };
    blend_rounded_rect(
        fb,
        (button.x, button.y, button.w, button.h),
        ITEM_RADIUS,
        theme.menu_item_hover,
        0xFF,
    );
    let face = style(Face::Ui, NAME_SIZE_GRID, theme.label_text);
    draw_label(
        fb,
        button,
        button.x + 12,
        button.w - 12 - SEARCH_ICON - 8,
        label,
        &face,
    );
    let glyph_x = button.x + button.w - SEARCH_ICON - 8;
    icons::draw_glyph(
        fb,
        glyph,
        SEARCH_ICON,
        glyph_x,
        button.y + (button.h - SEARCH_ICON) / 2,
        theme.label_text,
    );
    state().add_hit(button, Target::ViewSwitch);
}

fn highlight(fb: &FramebufferInfo, rect: Rect) {
    let rect = (rect.x, rect.y, rect.w, rect.h);
    blend_rounded_rect(fb, rect, ITEM_RADIUS, theme::accent_argb(), HIGHLIGHT_ALPHA);
}

fn draw_grid(fb: &FramebufferInfo, theme: &Theme, layout: &Layout, list: &[usize]) {
    let selected = state().nav.selected;
    let name_style = style(Face::Ui, NAME_SIZE_GRID, theme.label_text);
    for (slot, &index) in list.iter().enumerate() {
        let cell = layout.grid_cell(slot);
        if selected == Some(slot) {
            highlight(fb, cell);
        }
        let item = &state().items[index];
        let icon_x = cell.x + (cell.w - layout::GRID_ICON) / 2;
        icons::draw(
            fb,
            icons::for_app(&item.name),
            layout::GRID_ICON,
            icon_x,
            cell.y + 10,
        );
        let max_w = cell.w - 6;
        let shown = text::width(&item.name, &name_style).min(max_w);
        let text_y = cell.y + 10 + layout::GRID_ICON + 8;
        text::draw_fitted(
            fb,
            cell.x + (cell.w - shown) / 2,
            text_y,
            max_w,
            &item.name,
            &name_style,
        );
        state().add_hit(cell, Target::Item(slot));
    }
}

fn draw_row(fb: &FramebufferInfo, theme: &Theme, row: Rect, item: &Item, selected: bool) {
    if selected {
        highlight(fb, row);
    }
    let icon_y = row.y + (row.h - layout::LIST_ICON) / 2;
    icons::draw(
        fb,
        icons::for_app(&item.name),
        layout::LIST_ICON,
        row.x + 10,
        icon_y,
    );
    let text_x = row.x + 10 + layout::LIST_ICON + ICON_GAP;
    let name_style = style(Face::Ui, NAME_SIZE_LIST, theme.label_text);
    draw_label(
        fb,
        row,
        text_x,
        row.w.saturating_sub(text_x - row.x + 8),
        &item.name,
        &name_style,
    );
}

fn draw_scrollbar(fb: &FramebufferInfo, theme: &Theme, layout: &Layout, count: usize) {
    let st = state();
    let track = layout.scroll_track();
    blend_rounded_rect(
        fb,
        (track.x, track.y, track.w, track.h),
        track.w / 2,
        theme.label_text_dim,
        FAINT_ALPHA,
    );
    let thumb = layout.scroll_thumb(count, st.nav.scroll);
    fill_rounded_rect(
        fb,
        thumb.x,
        thumb.y,
        thumb.w,
        thumb.h,
        thumb.w / 2,
        theme.label_text_dim,
    );
    st.add_hit(track, Target::Track);
    st.add_hit(thumb, Target::Thumb);
}

fn draw_list(fb: &FramebufferInfo, theme: &Theme, layout: &Layout, list: &[usize]) {
    let st = state();
    let page = layout.list_page();
    st.nav.scroll = st.nav.scroll.min(list.len().saturating_sub(page));
    let scrollbar = layout.needs_scrollbar(list.len());
    for slot in 0..page {
        let pos = st.nav.scroll + slot;
        let Some(&index) = list.get(pos) else { break };
        let row = layout.list_row(slot, scrollbar);
        draw_row(
            fb,
            theme,
            row,
            &st.items[index],
            st.nav.selected == Some(pos),
        );
        st.add_hit(row, Target::Item(pos));
    }
    if scrollbar {
        draw_scrollbar(fb, theme, layout, list.len());
    }
}

fn draw_empty(fb: &FramebufferInfo, theme: &Theme, layout: &Layout) {
    let dim = style(Face::Ui, NAME_SIZE_LIST, theme.label_text_dim);
    let center_x = layout.content.x + layout.content.w / 2;
    text::draw_centered(fb, center_x, layout.content.y + 24, "No apps found", &dim);
}

fn draw_content(fb: &FramebufferInfo, theme: &Theme, layout: &Layout, list: &[usize]) {
    match state().nav.mode() {
        Mode::Pinned => draw_grid(fb, theme, layout, list),
        Mode::Results if list.is_empty() => draw_empty(fb, theme, layout),
        Mode::AllApps | Mode::Results => draw_list(fb, theme, layout, list),
    }
}

/// Name for the bottom bar: the account's full name when it has one.
fn user_label() -> String {
    let account = crate::users::current_user();
    let gecos = crate::users::passwd::find_user(&account).map(|entry| entry.gecos);
    model::display_name(&account, gecos.as_deref())
}

fn draw_bar(fb: &FramebufferInfo, theme: &Theme, layout: &Layout) {
    let bar = layout.bar;
    blend_rect(
        fb,
        bar.x + layout::PADDING,
        bar.y,
        bar.w - 2 * layout::PADDING,
        1,
        theme.label_text_dim,
        SEPARATOR_ALPHA,
    );
    let name = user_label();
    let radius = layout::AVATAR_SIZE / 2;
    let center = (bar.x + layout::PADDING + radius, bar.y + bar.h / 2);
    fill_circle(fb, center.0, center.1, radius, theme::accent_argb());
    let initial_style = style(Face::UiBold, SECTION_SIZE, WHITE);
    let initial = String::from(model::initial(&name));
    let line = text::line_height(&initial_style);
    text::draw_centered(fb, center.0, center.1 - line / 2, &initial, &initial_style);
    let name_x = center.0 + radius + ICON_GAP;
    let name_style = style(Face::Ui, NAME_SIZE_LIST, theme.label_text);
    let max_w = layout.power_button.x.saturating_sub(name_x + ICON_GAP);
    draw_label(fb, bar, name_x, max_w, &name, &name_style);
    draw_power_button(fb, theme, layout);
}

fn draw_power_button(fb: &FramebufferInfo, theme: &Theme, layout: &Layout) {
    let button = layout.power_button;
    if state().nav.power_open {
        highlight(fb, button);
    }
    let icon_pos = (
        button.x + (button.w - POWER_ICON) / 2,
        button.y + (button.h - POWER_ICON) / 2,
    );
    icons::draw_glyph(
        fb,
        Glyph::Power,
        POWER_ICON,
        icon_pos.0,
        icon_pos.1,
        theme.label_text,
    );
    state().add_hit(button, Target::PowerButton);
}

fn power_glyph(action: PowerAction) -> Glyph {
    match action {
        PowerAction::Lock => Glyph::Lock,
        PowerAction::LogOut => Glyph::Logout,
        PowerAction::Restart => Glyph::Refresh,
        PowerAction::ShutDown => Glyph::Power,
    }
}

fn draw_power_menu(fb: &FramebufferInfo, theme: &Theme, layout: &Layout) {
    let pop = layout.power_menu;
    fill_rounded_rect(
        fb,
        pop.x,
        pop.y,
        pop.w,
        pop.h,
        ITEM_RADIUS,
        theme.menu_border,
    );
    fill_rounded_rect(
        fb,
        pop.x + 1,
        pop.y + 1,
        pop.w - 2,
        pop.h - 2,
        ITEM_RADIUS - 1,
        theme.menu_bg,
    );
    for (i, action) in PowerAction::ALL.iter().enumerate() {
        let row = layout.power_row(i);
        let enabled = action.is_enabled();
        if enabled && state().power_hover == Some(i) {
            highlight(fb, row);
        }
        let color = if enabled {
            theme.label_text
        } else {
            theme.label_text_dim
        };
        let glyph_y = row.y + (row.h - POWER_ICON) / 2;
        icons::draw_glyph(
            fb,
            power_glyph(*action),
            POWER_ICON,
            row.x + 10,
            glyph_y,
            color,
        );
        let label = style(Face::Ui, NAME_SIZE_LIST, color);
        draw_label(
            fb,
            row,
            row.x + 10 + POWER_ICON + ICON_GAP,
            row.w - 40,
            action.label(),
            &label,
        );
        state().add_hit(row, Target::PowerEntry(i));
    }
}

/// Draw the menu above the windows and the taskbar's neighbours.
pub fn draw(fb: &FramebufferInfo) {
    if !state().open {
        return;
    }
    let theme = theme::current();
    let layout = screen_layout();
    let list = visible_list(&layout);
    state().clear_hits();
    draw_surface(fb, theme, layout.menu);
    draw_search(fb, theme, &layout);
    draw_section(fb, theme, &layout);
    draw_content(fb, theme, &layout, &list);
    draw_bar(fb, theme, &layout);
    if state().nav.power_open {
        draw_power_menu(fb, theme, &layout);
    }
}
