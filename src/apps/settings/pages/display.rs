//! Display: resolution (with confirm countdown) and scale.

use super::{heading, note, setting, Flow};
use crate::apps::settings::model::Action;
use crate::apps::settings::resolution::{index_of, RESOLUTIONS};
use crate::apps::settings::{display, state::State};
use crate::wm::controls::Ui;
use alloc::string::String;
use alloc::vec::Vec;

/// Width of the resolution dropdown.
const DROPDOWN_W: u32 = 200;

/// Label for a mode, e.g. `1280 x 720`.
fn mode_label(w: u32, h: u32) -> String {
    alloc::format!("{w} \u{d7} {h}")
}

pub fn draw(ui: &mut Ui, flow: &mut Flow, st: &mut State) {
    heading(ui, flow, "RESOLUTION");
    let (w, h) = display::current();
    let current = match index_of(w, h) {
        Some(_) => mode_label(w, h),
        None => alloc::format!("{} (custom)", mode_label(w, h)),
    };
    let field = setting(
        ui,
        flow,
        "Screen resolution",
        "Changes apply at once; you confirm within 15 seconds",
        DROPDOWN_W,
    );
    ui.dropdown(Action::ResolutionMenu.key(), field, &current, true);
    flow.space(8);
    heading(ui, flow, "SCALE");
    let scale = setting(
        ui,
        flow,
        "Display scale",
        "Scaling arrives with the display scale work (O2.9)",
        DROPDOWN_W,
    );
    ui.dropdown(0, scale, "100%", false);
    note(ui, flow, "Only 100% is available in this build.");
    if st.resolution_menu.is_open() {
        open_popup(ui, st, field);
    }
}

fn open_popup(ui: &mut Ui, st: &State, field: crate::wm::controls::Rect) {
    let labels: Vec<String> = RESOLUTIONS.iter().map(|&(w, h)| mode_label(w, h)).collect();
    let items: Vec<&str> = labels.iter().map(String::as_str).collect();
    let keys: Vec<u32> = (0..items.len())
        .map(|i| Action::ResolutionPick(i as u8).key())
        .collect();
    let (w, h) = display::current();
    let selected = index_of(w, h).unwrap_or(usize::MAX);
    let view = ui.view_extent();
    ui.dropdown_popup(
        field,
        &items,
        &keys,
        &st.resolution_menu,
        selected,
        Action::DismissPopup.key(),
        view,
    );
}
