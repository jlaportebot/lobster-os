//! Apps: installed packages (uninstall) and built-in apps.

use super::{heading, note, Flow};
use crate::apps::settings::actions::refresh_packages;
use crate::apps::settings::format::bytes_text;
use crate::apps::settings::model::Action;
use crate::apps::settings::state::State;
use crate::display::icons::Glyph;
use crate::wm::controls::{ButtonKind, Ui};

/// Height of an app row.
const APP_ROW_H: u32 = 60;

pub fn draw(ui: &mut Ui, flow: &mut Flow, st: &mut State) {
    refresh_packages(st);
    heading(ui, flow, "INSTALLED PACKAGES");
    if st.packages.is_empty() {
        note(
            ui,
            flow,
            "No packages are installed. Try `pkg install hello` in the terminal.",
        );
    }
    for (i, pkg) in st.packages.iter().enumerate() {
        let r = flow.take(APP_ROW_H);
        ui.card(r);
        let sub = alloc::format!("version {}  \u{b7}  {}", pkg.version, bytes_text(pkg.size));
        let trailing = Some((
            Action::Uninstall(i as u8).key(),
            "Uninstall",
            ButtonKind::Danger,
            true,
        ));
        ui.list_row(r, Some(Glyph::Grid), &pkg.name, &sub, trailing);
    }
    note(ui, flow, &st.notice);
    flow.space(8);
    heading(ui, flow, "BUILT-IN APPS");
    for app in crate::wm::start_menu::APPS {
        let r = flow.take(APP_ROW_H);
        ui.card(r);
        ui.list_row(r, Some(Glyph::Grid), app.name, "Built in", None);
    }
}
