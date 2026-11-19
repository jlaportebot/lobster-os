//! Storage: /sd usage from the ext2 superblock, RAM, shortcut to Files.

use super::{heading, info_card, setting, Flow};
use crate::apps::settings::format::{bytes_text, percent};
use crate::apps::settings::model::Action;
use crate::apps::settings::system;
use crate::wm::controls::{ButtonKind, Rect, Ui, PROGRESS_H};
use alloc::string::String;

/// Width of the Open in Files button cell.
const OPEN_W: u32 = 160;
/// Height of a usage card.
const USAGE_CARD_H: u32 = 88;
/// Inner padding of the usage card.
const PAD: i32 = 16;

pub fn draw(ui: &mut Ui, flow: &mut Flow) {
    heading(ui, flow, "PERSISTENT DISK  (/sd)");
    match crate::fs::ext2::usage() {
        Some(usage) => disk_card(ui, flow, &usage),
        None => info_card(
            ui,
            flow,
            &[("Status", String::from("No persistent disk is mounted"))],
        ),
    }
    let open = setting(
        ui,
        flow,
        "Browse files",
        "Open the disk in the Files app",
        OPEN_W,
    );
    ui.button(
        Action::OpenFiles.key(),
        open,
        "Open in Files",
        ButtonKind::Primary,
        true,
    );
    flow.space(8);
    heading(ui, flow, "MEMORY");
    let (total, used) = system::ram();
    usage_bar(ui, flow, "RAM", used, total);
}

fn disk_card(ui: &mut Ui, flow: &mut Flow, usage: &crate::fs::ext2::Ext2Usage) {
    let block = u64::from(usage.block_size);
    let total = usage.blocks_total * block;
    let free = usage.blocks_free * block;
    usage_bar(ui, flow, "Space", total - free, total);
    let inodes_used = usage.inodes_total - usage.inodes_free;
    let inode_text = alloc::format!(
        "{} of {} used ({}%)",
        inodes_used,
        usage.inodes_total,
        percent(u64::from(inodes_used), u64::from(usage.inodes_total))
    );
    info_card(
        ui,
        flow,
        &[("Free", bytes_text(free)), ("Inodes", inode_text)],
    );
}

/// Card with a title, `used of total` text and a bar.
fn usage_bar(ui: &mut Ui, flow: &mut Flow, title: &str, used: u64, total: u64) {
    use crate::display::text::Face;
    use crate::wm::controls::style;
    let th = crate::wm::theme::current();
    let card = flow.take(USAGE_CARD_H);
    ui.card(card);
    let head = style(Face::UiBold, 14, th.label_text);
    let detail = style(Face::Ui, 13, th.label_text_dim);
    ui.text(card.x + PAD, card.y + 14, title, &head);
    let text = alloc::format!("{} used of {}", bytes_text(used), bytes_text(total));
    ui.text(card.x + PAD, card.y + 36, &text, &detail);
    let bar = Rect::new(
        card.x + PAD,
        card.y + 66,
        card.w - 2 * PAD as u32,
        PROGRESS_H,
    );
    ui.progress(bar, percent(used, total));
}
