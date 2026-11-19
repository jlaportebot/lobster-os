//! Sound: output volume (shared with the tray), mute, test tone.

use super::{heading, note, setting, Flow};
use crate::apps::settings::model::Action;
use crate::apps::settings::{sound, state::State};
use crate::wm::controls::{ButtonKind, Ui};
use crate::wm::tray_popups;
use alloc::string::String;

/// Width of the volume slider.
const SLIDER_W: u32 = 260;
/// Width of the switch cell.
const SWITCH_CELL_W: u32 = 44;
/// Width of the test button.
const TEST_BUTTON_W: u32 = 150;
/// Volume range.
const RANGE: (u32, u32) = (0, 100);

pub fn draw(ui: &mut Ui, flow: &mut Flow, st: &mut State) {
    let device = sound::has_device();
    heading(ui, flow, "OUTPUT");
    let level = tray_popups::volume_level();
    let desc = if device {
        "Same level as the taskbar quick settings"
    } else {
        "No audio device in this machine"
    };
    let title = alloc::format!("Volume  {level}%");
    let slider = setting(ui, flow, &title, desc, SLIDER_W);
    ui.slider(Action::Volume.key(), slider, level, RANGE, device);
    let cell = setting(ui, flow, "Mute", "Silence all sound output", SWITCH_CELL_W);
    ui.switch(
        Action::Mute.key(),
        cell.x,
        cell.y + 4,
        sound::is_muted(),
        device,
    );
    if device {
        let test = setting(
            ui,
            flow,
            "Test sound",
            "Plays a short tone through the speakers",
            TEST_BUTTON_W,
        );
        ui.button(
            Action::TestSound.key(),
            test,
            "Play test sound",
            ButtonKind::Secondary,
            true,
        );
        note(ui, flow, &st.notice);
    } else {
        note(
            ui,
            flow,
            &String::from("The test button appears when an audio device is present."),
        );
    }
}
