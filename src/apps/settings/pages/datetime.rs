//! Date & time: live clock, zone, 24-hour switch.

use super::{heading, info_card, setting, Flow};
use crate::apps::settings::format::utc_offset_text;
use crate::apps::settings::model::Action;
use crate::display::text::Face;
use crate::wm::controls::{style, Ui};
use crate::wm::theme;
use alloc::string::String;

/// Height of the big clock card.
const CLOCK_CARD_H: u32 = 110;
/// Text size of the clock read-out.
const CLOCK_PX: u32 = 40;
/// Text size of the date line.
const DATE_PX: u32 = 16;
/// Width of the switch cell.
const SWITCH_CELL_W: u32 = 44;

pub fn draw(ui: &mut Ui, flow: &mut Flow) {
    let th = theme::current();
    heading(ui, flow, "NOW");
    let card = flow.take(CLOCK_CARD_H);
    ui.card(card);
    let (time, date) = clock_texts();
    let big = style(Face::UiBold, CLOCK_PX, th.label_text);
    let small = style(Face::Ui, DATE_PX, th.label_text_dim);
    ui.text(card.x + 20, card.y + 14, &time, &big);
    ui.text(card.x + 22, card.y + 70, &date, &small);
    flow.space(8);
    heading(ui, flow, "TIME ZONE");
    let zone = utc_offset_text(crate::clock::utc_offset_secs());
    info_card(
        ui,
        flow,
        &[
            ("Time zone", zone),
            ("Set by", String::from("Set automatically from the host")),
        ],
    );
    flow.space(8);
    heading(ui, flow, "FORMAT");
    let cell = setting(
        ui,
        flow,
        "24-hour clock",
        "Also changes the clock in the taskbar",
        SWITCH_CELL_W,
    );
    ui.switch(
        Action::Clock24.key(),
        cell.x,
        cell.y + 4,
        crate::clock::use_24_hour(),
        true,
    );
}

fn clock_texts() -> (String, String) {
    let uptime = crate::arch::ArchTimer::uptime_ms();
    let Some(now) = crate::clock::local_at(uptime) else {
        return (
            String::from("--:--:--"),
            String::from("The host did not provide the time"),
        );
    };
    let hms = if crate::clock::use_24_hour() {
        alloc::format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second)
    } else {
        let suffix = if now.hour < 12 { "AM" } else { "PM" };
        let hour = if now.hour % 12 == 0 {
            12
        } else {
            now.hour % 12
        };
        alloc::format!("{}:{:02}:{:02} {}", hour, now.minute, now.second, suffix)
    };
    let date = alloc::format!(
        "{}, {} {} {}",
        now.weekday_name(),
        now.day,
        now.month_name(),
        now.year
    );
    (hms, date)
}
