//! About: version, build, CPU, memory, uptime, display and machine.

use super::{heading, info_card, Flow};
use crate::apps::settings::format::{bytes_text, cpu_name, uptime_text};
use crate::apps::settings::{display, system};
use crate::display::text::Face;
use crate::wm::controls::{style, Ui};
use crate::wm::theme;
use alloc::string::String;

/// Height of the product banner.
const BANNER_H: u32 = 96;
/// Text size of the product name.
const NAME_PX: u32 = 28;

pub fn draw(ui: &mut Ui, flow: &mut Flow) {
    let th = theme::current();
    let banner = flow.take(BANNER_H);
    ui.card(banner);
    ui.text(
        banner.x + 20,
        banner.y + 16,
        "LobsterOS",
        &style(Face::UiBold, NAME_PX, th.label_text),
    );
    let tagline = style(Face::Ui, 14, th.label_text_dim);
    ui.text(
        banner.x + 20,
        banner.y + 58,
        "A bare-metal Rust desktop for the Raspberry Pi 5",
        &tagline,
    );
    flow.space(8);
    heading(ui, flow, "SYSTEM");
    let (ram_total, ram_used) = system::ram();
    let (w, h) = display::current();
    let lines = [
        ("Version", String::from(env!("CARGO_PKG_VERSION"))),
        ("Kernel build", system::build_id()),
        ("Processor", cpu_name(system::midr())),
        ("Cores", alloc::format!("{} online", system::online_cores())),
        (
            "Memory",
            alloc::format!("{} used of {}", bytes_text(ram_used), bytes_text(ram_total)),
        ),
        ("Uptime", uptime_text(crate::arch::ArchTimer::uptime_ms())),
        ("Display", alloc::format!("{w} \u{d7} {h}")),
        ("Machine", String::from(system::machine())),
    ];
    info_card(ui, flow, &lines);
}
