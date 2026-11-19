//! What each control does. Every handler changes the live system first and
//! then saves the settings file, so nothing is only cosmetic.

use super::model::{Action, Page};
use super::resolution::{Outcome, RESOLUTIONS};
use super::state::{self, NetTest, PackageRow, Pending, QueuedDialog, State};
use super::{accounts, display, sound, store, system};
use crate::wm::theme::{self, FontSize, ThemeId};
use crate::wm::{dialog, wallpaper};
use alloc::string::String;

/// Pixels a keyboard arrow or page key scrolls the content.
pub const SCROLL_STEP: i32 = 48;
/// How long a package list stays cached on the Apps page.
const PACKAGE_REFRESH_MS: u64 = 2000;

fn now_ms() -> u64 {
    crate::arch::ArchTimer::uptime_ms()
}

/// Run `action` (a click, Enter or Space on its control).
pub fn perform(action: Action) {
    let st = state::get();
    match action {
        Action::SelectPage(page) => st.go_to(page),
        Action::Theme(i) => set_theme(i),
        Action::Accent(i) => set_accent(i),
        Action::Wallpaper(i) => set_wallpaper(i),
        Action::LargeText => toggle_large_text(),
        Action::ResolutionMenu => toggle_resolution_menu(st),
        Action::ResolutionPick(i) => pick_resolution(st, i),
        Action::ResolutionKeep => keep_resolution(st),
        Action::ResolutionRevert => revert_resolution(st),
        Action::Mute => toggle_mute(),
        Action::TestSound => play_test_sound(st),
        Action::TestConnection => st.net_test = NetTest::Queued,
        Action::AddUser => ask_add_user(st),
        Action::ChangePassword => super::passwords::start(st),
        Action::RemoveUser(i) => ask_remove_user(st, i),
        Action::Clock24 => toggle_clock(),
        Action::OpenFiles => {
            crate::wm::start_menu::launch_by_name("Files");
        }
        Action::Uninstall(i) => ask_uninstall(st, i),
        Action::DismissPopup => st.resolution_menu.close(),
        Action::Volume | Action::PageScroll | Action::UrlField => {}
    }
    crate::wm::dirty::mark();
}

/// A slider or scrollbar produced `value` for `key`.
pub fn on_drag(key: u32, value: u32) {
    match Action::from_key(key) {
        Some(Action::Volume) => {
            sound::set_volume(value);
            store::save();
        }
        Some(Action::PageScroll) => state::get().scroll = value as i32,
        _ => {}
    }
}

fn set_theme(index: u8) {
    let id = if index == 0 {
        ThemeId::Dark
    } else {
        ThemeId::Light
    };
    theme::set(id);
    store::save();
}

fn set_accent(index: u8) {
    if let Some((name, _)) = theme::ACCENTS.get(usize::from(index)) {
        theme::set_accent_name(name);
    }
}

fn set_wallpaper(index: u8) {
    wallpaper::set_index(usize::from(index));
}

fn toggle_large_text() {
    let next = if theme::font_size() == FontSize::Large {
        FontSize::Medium
    } else {
        FontSize::Large
    };
    theme::set_font_size(next);
    store::save();
}

fn toggle_clock() {
    crate::clock::set_use_24_hour(!crate::clock::use_24_hour());
    store::save();
}

fn toggle_mute() {
    sound::set_muted(!sound::is_muted());
    store::save();
}

fn play_test_sound(st: &mut State) {
    st.notice = if sound::play_test() {
        String::from("Played a test tone.")
    } else {
        String::from("No audio device to play on.")
    };
}

fn toggle_resolution_menu(st: &mut State) {
    let (w, h) = display::current();
    let selected = super::resolution::index_of(w, h).unwrap_or(0);
    st.resolution_menu.toggle(selected);
}

fn pick_resolution(st: &mut State, index: u8) {
    st.resolution_menu.close();
    let Some(&(w, h)) = RESOLUTIONS.get(usize::from(index)) else {
        return;
    };
    let previous = display::current();
    if previous == (w, h) {
        return;
    }
    match display::apply(w, h) {
        Ok(()) => st.confirm.begin(previous, (w, h), now_ms()),
        Err(reason) => st.notice = String::from(reason),
    }
}

fn keep_resolution(st: &mut State) {
    st.confirm.keep();
}

fn revert_resolution(st: &mut State) {
    if let Some((w, h)) = st.confirm.revert() {
        let _ = display::apply(w, h);
    }
}

/// Ask for a dialog. It opens once the mouse button is up: the dialog layer
/// treats the release that follows the opening press as a click outside its
/// text field and would otherwise take the keyboard focus away from it.
pub(super) fn queue_dialog(st: &mut State, title: &str, message: &str, is_prompt: bool) {
    queue(st, title, message, is_prompt, false);
}

/// Queue a prompt whose text is drawn as asterisks.
pub(super) fn queue_secret(st: &mut State, title: &str, message: &str) {
    queue(st, title, message, true, true);
}

fn queue(st: &mut State, title: &str, message: &str, is_prompt: bool, is_secret: bool) {
    st.queued_dialog = Some(QueuedDialog {
        title: String::from(title),
        message: String::from(message),
        is_prompt,
        is_secret,
    });
}

fn open_queued_dialog(st: &mut State) {
    if crate::wm::input::mouse_buttons().0 {
        return;
    }
    let Some(request) = st.queued_dialog.take() else {
        return;
    };
    if request.is_secret {
        dialog::prompt_secret(&request.title, &request.message);
    } else if request.is_prompt {
        dialog::prompt(&request.title, &request.message, "");
    } else {
        dialog::confirm(&request.title, &request.message);
    }
}

fn ask_add_user(st: &mut State) {
    st.pending = Some(Pending::AddUser);
    queue_dialog(st, "Add user", "Name for the new account:", true);
}

fn ask_remove_user(st: &mut State, index: u8) {
    let rows = accounts::rows();
    let Some(row) = rows.get(usize::from(index)) else {
        return;
    };
    st.pending = Some(Pending::RemoveUser(row.name.clone()));
    let message = alloc::format!(
        "Remove the account '{}'? Its home folder is kept.",
        row.name
    );
    queue_dialog(st, "Remove user", &message, false);
}

fn ask_uninstall(st: &mut State, index: u8) {
    let Some(row) = st.packages.get(usize::from(index)) else {
        return;
    };
    st.pending = Some(Pending::Uninstall(row.name.clone()));
    let message = alloc::format!("Uninstall '{}'?", row.name);
    queue_dialog(st, "Uninstall app", &message, false);
}

/// Per-frame housekeeping that does not need the window: the resolution
/// countdown, the queued connection test and dialog answers.
pub fn background_tick() {
    let st = state::get();
    if let Outcome::Revert { to } = st.confirm.tick(now_ms()) {
        let _ = display::apply(to.0, to.1);
        crate::wm::notifications::info("Display", "Reverted to the previous resolution");
    }
    advance_net_test(st);
    open_queued_dialog(st);
    if st.pending.is_some() && st.queued_dialog.is_none() {
        collect_dialog_answer(st);
    }
}

fn advance_net_test(st: &mut State) {
    match st.net_test {
        NetTest::Queued => {
            st.net_test = NetTest::Running;
            crate::wm::dirty::mark();
        }
        NetTest::Running => {
            let url = if st.url.is_empty() {
                state::DEFAULT_TEST_URL
            } else {
                &st.url
            };
            let report = system::fetch_report(url);
            st.net_test = NetTest::Done(report);
            crate::wm::dirty::mark();
        }
        NetTest::Idle | NetTest::Done(_) => {}
    }
}

fn collect_dialog_answer(st: &mut State) {
    use crate::wm::dialog::DialogResult;
    let Some(result) = dialog::take_result() else {
        return;
    };
    let pending = st.pending.take();
    if let Some(Pending::Password(step)) = pending {
        st.notice = super::passwords::advance(st, step, result);
        crate::wm::dirty::mark();
        return;
    }
    st.notice = match (pending, result) {
        (Some(Pending::AddUser), DialogResult::Prompt(Some(name))) => add_user_notice(st, &name),
        (Some(Pending::RemoveUser(name)), DialogResult::Confirm(true)) => {
            report(accounts::remove_user(&name), &name, "removed")
        }
        (Some(Pending::Uninstall(name)), DialogResult::Confirm(true)) => uninstall(st, &name),
        _ => String::new(),
    };
    crate::wm::dirty::mark();
}

fn add_user_notice(st: &mut State, raw: &str) -> String {
    let name = super::users::clean_name(raw);
    super::passwords::begin_new_user(st, name)
}

fn report(result: Result<(), String>, name: &str, verb: &str) -> String {
    match result {
        Ok(()) => alloc::format!("User '{name}' {verb}."),
        Err(reason) => reason,
    }
}

fn uninstall(st: &mut State, name: &str) -> String {
    let outcome =
        crate::pkg::install::Installer::new().and_then(|mut installer| installer.remove(name));
    st.packages_loaded_ms = None;
    match outcome {
        Ok(()) => alloc::format!("'{name}' was uninstalled."),
        Err(e) => alloc::format!("Could not uninstall '{name}': {e:?}"),
    }
}

/// Reload the installed-package list when it is stale.
pub fn refresh_packages(st: &mut State) {
    let now = now_ms();
    if st
        .packages_loaded_ms
        .is_some_and(|at| now.saturating_sub(at) < PACKAGE_REFRESH_MS)
    {
        return;
    }
    st.packages_loaded_ms = Some(now);
    let Ok(db) = crate::pkg::install::PackageDatabase::load() else {
        st.packages.clear();
        return;
    };
    st.packages = db
        .list_installed()
        .into_iter()
        .map(|p| PackageRow {
            name: p.metadata.name.clone(),
            version: p.metadata.version.clone(),
            size: p.metadata.size,
        })
        .collect();
}

/// Open the page `page` (used by deep links and `settings <page>`).
pub fn show(page: Page) {
    state::get().go_to(page);
}
