//! Start menu: search, pinned grid, All apps list and power options.
//!
//! The menu is a modal overlay: while it is open every key and every left
//! press belongs to it, and a press outside closes it without reaching
//! whatever is underneath. Geometry lives in [`layout`], searching and the
//! keyboard state machine in [`model`], drawing in [`render`]; this file
//! owns the open/closed state and turns input into launches.

mod layout;
mod model;
mod render;

use alloc::string::String;
use alloc::vec::Vec;

use crate::display::fb::get_info;
use crate::wm::hit_region::{HitKind, HitRegion, HitRegistry};
use crate::wm::launcher::{self, InstalledApp};
use crate::wm::window;
use layout::Layout;
use model::{Item, Key, Mode, Nav, NavCtx, Outcome, PowerAction, Source, View};

pub use render::draw;

/// One launchable application entry
pub struct AppEntry {
    pub name: &'static str,
    /// Launch behavior
    pub launch: fn(),
}

/// The full application catalog
pub static APPS: &[AppEntry] = &[
    AppEntry {
        name: "Terminal",
        launch: || {
            focus_or_create("Terminal", 720, 500, || {
                let _ = crate::apps::terminal::open_terminal_window(100, 80, 720, 500);
            });
        },
    },
    AppEntry {
        name: "Files",
        launch: || {
            crate::apps::files::open_window();
        },
    },
    AppEntry {
        name: "Editor",
        launch: || {
            focus_or_create("Editor", 520, 400, || {
                let _ = window::create(140, 120, 520, 400, "Editor");
            });
        },
    },
    AppEntry {
        name: "Calculator",
        launch: || {
            focus_or_create("Calculator", 300, 400, || {
                let _ = window::create(160, 140, 300, 400, "Calculator");
            });
        },
    },
    AppEntry {
        name: "Task Manager",
        launch: || {
            crate::apps::task_manager::open();
        },
    },
    AppEntry {
        name: "Settings",
        launch: || {
            crate::apps::settings::open_window();
        },
    },
    AppEntry {
        name: "Network",
        launch: || {
            focus_or_create("Network", 420, 300, || {
                let _ = window::create(240, 220, 420, 300, "Network");
            });
        },
    },
    AppEntry {
        name: "App Store",
        launch: || {
            crate::apps::app_store::open_window();
        },
    },
    AppEntry {
        name: "Images",
        launch: || {
            focus_or_create("Image Viewer", 460, 340, || {
                let _ = window::create(280, 260, 460, 340, "Image Viewer");
            });
        },
    },
    AppEntry {
        name: "Browser",
        launch: || {
            focus_or_create("Browser", 900, 620, || {
                crate::apps::browser::open("http://example.com");
            });
        },
    },
    AppEntry {
        name: "Notes",
        launch: || {
            focus_or_create("Notes", 700, 480, || {
                crate::apps::notes::init();
                let _ = window::create(260, 140, 700, 480, "Notes");
            });
        },
    },
    AppEntry {
        name: "About",
        launch: || {
            focus_or_create("About", 360, 260, || {
                let _ = window::create(300, 280, 360, 260, "About");
            });
        },
    },
];

/// Search keywords per built-in app, so "console" finds the Terminal.
const KEYWORDS: &[(&str, &str)] = &[
    ("Terminal", "shell console command line"),
    ("Files", "file manager explorer folders"),
    ("Editor", "text code write"),
    ("Calculator", "math calc numbers"),
    ("Task Manager", "processes monitor kill"),
    ("Settings", "preferences control options"),
    ("Network", "wifi ethernet internet"),
    ("App Store", "packages install software"),
    ("Images", "image viewer pictures photos"),
    ("Browser", "web internet"),
    ("Notes", "memo text"),
    ("About", "system info version"),
];

/// Action behind a hit region, packed into the region's `tag`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    /// Entry at this position of the visible list.
    Item(usize),
    /// The Pinned / All apps switch button.
    ViewSwitch,
    /// The search field.
    Search,
    /// The power button.
    PowerButton,
    /// Power popup entry.
    PowerEntry(usize),
    /// Scrollbar thumb.
    Thumb,
    /// Scrollbar track outside the thumb.
    Track,
}

/// Bits of a tag that hold the payload; the rest hold the target kind.
const TAG_PAYLOAD_BITS: u32 = 16;
const TAG_PAYLOAD_MASK: u32 = (1 << TAG_PAYLOAD_BITS) - 1;

impl Target {
    pub(super) fn to_tag(self) -> u32 {
        let (kind, payload) = match self {
            Target::Item(pos) => (1, pos as u32),
            Target::ViewSwitch => (2, 0),
            Target::Search => (3, 0),
            Target::PowerButton => (4, 0),
            Target::PowerEntry(i) => (5, i as u32),
            Target::Thumb => (6, 0),
            Target::Track => (7, 0),
        };
        (kind << TAG_PAYLOAD_BITS) | (payload & TAG_PAYLOAD_MASK)
    }

    fn from_tag(tag: u32) -> Option<Target> {
        let payload = (tag & TAG_PAYLOAD_MASK) as usize;
        match tag >> TAG_PAYLOAD_BITS {
            1 => Some(Target::Item(payload)),
            2 => Some(Target::ViewSwitch),
            3 => Some(Target::Search),
            4 => Some(Target::PowerButton),
            5 => Some(Target::PowerEntry(payload)),
            6 => Some(Target::Thumb),
            7 => Some(Target::Track),
            _ => None,
        }
    }
}

/// Everything the menu remembers between frames.
pub(super) struct State {
    pub(super) open: bool,
    pub(super) nav: Nav,
    pub(super) items: Vec<Item>,
    installed: Vec<InstalledApp>,
    hits: HitRegistry,
    /// Hovered power popup entry.
    pub(super) power_hover: Option<usize>,
    /// While dragging the scrollbar: pointer offset inside the thumb.
    drag_grab: Option<u32>,
}

static mut STATE: State = State {
    open: false,
    nav: Nav::new(),
    items: Vec::new(),
    installed: Vec::new(),
    hits: HitRegistry::new(),
    power_hover: None,
    drag_grab: None,
};

/// The menu state.
pub(super) fn state() -> &'static mut State {
    // SAFETY: the WM input handlers and the compositor draw run on the one
    // kernel render thread, so there is never a second live reference.
    unsafe { &mut *core::ptr::addr_of_mut!(STATE) }
}

impl State {
    /// Forget the regions of the previous frame; drawing re-registers them.
    pub(super) fn clear_hits(&mut self) {
        self.hits.clear();
    }

    /// Register a clickable rectangle for `target`.
    pub(super) fn add_hit(&mut self, rect: layout::Rect, target: Target) {
        let kind = match target {
            Target::Search => HitKind::TextInput,
            Target::Thumb | Target::Track => HitKind::ScrollBar,
            Target::Item(_) | Target::PowerEntry(_) => HitKind::MenuItem,
            Target::ViewSwitch | Target::PowerButton => HitKind::Button,
        };
        let region = HitRegion::new(rect.x, rect.y, rect.w, rect.h, target.to_tag(), kind);
        // The registry holds more regions than any layout can show.
        let _ = self.hits.add(region);
    }

    fn target_at(&self, x: u32, y: u32) -> Option<Target> {
        self.hits
            .hit_test(x, y)
            .and_then(|region| Target::from_tag(region.tag))
    }
}

/// Layout for the current screen above the taskbar.
pub(super) fn screen_layout() -> Layout {
    let fb = get_info();
    layout::compute(fb.width, fb.height, crate::wm::panel::panel_height())
}

/// Entries the menu lists: the built-in registry, then installed apps.
fn build_items(installed: &[InstalledApp]) -> Vec<Item> {
    let builtin = APPS.iter().enumerate().map(|(i, app)| Item {
        name: String::from(app.name),
        keywords: String::from(
            KEYWORDS
                .iter()
                .find(|(name, _)| *name == app.name)
                .map_or("", |(_, words)| words),
        ),
        source: Source::Builtin(i),
    });
    let extra = installed.iter().enumerate().map(|(i, app)| Item {
        name: app.name.clone(),
        keywords: app.keywords.clone(),
        source: Source::Installed(i),
    });
    builtin.chain(extra).collect()
}

/// Whether the menu is showing.
pub fn is_open() -> bool {
    state().open
}

fn open() {
    let st = state();
    st.installed = launcher::scan_installed_apps();
    st.items = build_items(&st.installed);
    st.nav = Nav::new();
    st.power_hover = None;
    st.drag_grab = None;
    st.open = true;
}

/// Open the menu when closed, close it when open.
pub fn toggle() {
    if is_open() {
        close();
    } else {
        open();
    }
    // Force a full recomposition so the menu appears/disappears immediately.
    crate::wm::compositor::mark_full_redraw();
    let label = if is_open() { "open" } else { "closed" };
    crate::println!("[wm] Start menu {}", label);
}

/// Close the menu and drop everything it holds.
pub fn close() {
    let st = state();
    if !st.open {
        return;
    }
    st.open = false;
    st.nav = Nav::new();
    st.drag_grab = None;
    st.hits.clear();
    crate::wm::compositor::mark_full_redraw();
}

/// Entries of the list the keyboard and hit regions index into.
fn visible_list(layout: &Layout) -> Vec<usize> {
    let st = state();
    model::visible(&st.items, &st.nav, layout.grid_capacity())
}

fn nav_ctx(layout: &Layout, count: usize) -> NavCtx {
    let in_grid = state().nav.mode() == Mode::Pinned;
    NavCtx {
        count,
        cols: if in_grid {
            layout::GRID_COLUMNS as usize
        } else {
            1
        },
        page: layout.list_page(),
    }
}

/// Scroll the list by whole rows (positive moves down).
pub fn scroll(delta: i32) {
    let st = state();
    if !st.open || st.nav.mode() == Mode::Pinned {
        return;
    }
    let layout = screen_layout();
    let max_scroll = visible_list(&layout)
        .len()
        .saturating_sub(layout.list_page());
    let moved = (st.nav.scroll as i64 + i64::from(delta)).clamp(0, max_scroll as i64);
    st.nav.scroll = moved as usize;
}

fn launch_item(item: &Item) {
    match item.source {
        Source::Builtin(i) => (APPS[i].launch)(),
        Source::Installed(i) => {
            let app = state().installed[i].clone();
            if let Err(reason) = launcher::launch_installed(&app) {
                crate::println!("[wm] Start menu: cannot launch {}: {}", app.name, reason);
            }
        }
    }
}

/// Launch the entry at `pos` of the visible list and close the menu.
fn activate(pos: usize) {
    let layout = screen_layout();
    let Some(&index) = visible_list(&layout).get(pos) else {
        return;
    };
    let item = state().items[index].clone();
    close();
    launch_item(&item);
}

fn perform_power(action: PowerAction) {
    close();
    crate::println!("[wm] Start menu power: {}", action.label());
    match action {
        PowerAction::Lock => crate::wm::lock::lock(),
        PowerAction::LogOut => crate::wm::lock::log_out(),
        #[cfg(target_arch = "aarch64")]
        PowerAction::Restart => crate::drivers::power::reset(),
        #[cfg(target_arch = "aarch64")]
        PowerAction::ShutDown => crate::drivers::power::shutdown(),
        #[cfg(not(target_arch = "aarch64"))]
        PowerAction::Restart | PowerAction::ShutDown => {}
    }
}

/// Scancodes as the input device delivers them (set 1; the same codes the
/// dialog and Task Manager key handlers use).
const SC_ESC: u8 = 0x01;
const SC_BACKSPACE: u8 = 0x0E;
const SC_ENTER: u8 = 0x1C;
const SC_UP: u8 = 0x48;
const SC_PAGE_UP: u8 = 0x49;
const SC_LEFT: u8 = 0x4B;
const SC_RIGHT: u8 = 0x4D;
const SC_DOWN: u8 = 0x50;
const SC_PAGE_DOWN: u8 = 0x51;

fn key_from_scancode(scancode: u8, ch: Option<char>) -> Option<Key> {
    match scancode {
        SC_ESC => Some(Key::Esc),
        SC_ENTER => Some(Key::Enter),
        SC_BACKSPACE => Some(Key::Backspace),
        SC_UP => Some(Key::Up),
        SC_DOWN => Some(Key::Down),
        SC_LEFT => Some(Key::Left),
        SC_RIGHT => Some(Key::Right),
        SC_PAGE_UP => Some(Key::PageUp),
        SC_PAGE_DOWN => Some(Key::PageDown),
        _ => ch.filter(|c| (' '..='~').contains(c)).map(Key::Char),
    }
}

/// Handle a key press while the menu is open. Returns true when the menu
/// consumed it; chords with Ctrl, Alt or Super stay with the WM shortcuts.
pub fn handle_key(scancode: u8, ch: Option<char>) -> bool {
    if !is_open() {
        return false;
    }
    let chord =
        crate::usb::hid::alt_held() || crate::usb::hid::ctrl_held() || crate::usb::hid::gui_held();
    let Some(key) = key_from_scancode(scancode, ch).filter(|_| !chord) else {
        return false;
    };
    let layout = screen_layout();
    let count = visible_list(&layout).len();
    let ctx = nav_ctx(&layout, count);
    match state().nav.on_key(key, ctx) {
        Outcome::Handled => {}
        Outcome::Launch(pos) => activate(pos),
        Outcome::Close => {
            close();
        }
    }
    true
}

fn press_scrollbar(layout: &Layout, target: Target, y: u32) {
    let count = visible_list(layout).len();
    let st = state();
    let thumb = layout.scroll_thumb(count, st.nav.scroll);
    if target == Target::Thumb {
        st.drag_grab = Some(y.saturating_sub(thumb.y));
        return;
    }
    let page = layout.list_page();
    let max_scroll = count.saturating_sub(page);
    st.nav.scroll = if y < thumb.y {
        st.nav.scroll.saturating_sub(page)
    } else {
        (st.nav.scroll + page).min(max_scroll)
    };
}

fn switch_view() {
    let nav = &mut state().nav;
    match nav.view {
        View::Pinned => nav.show(View::AllApps),
        View::AllApps => nav.show(View::Pinned),
    }
}

fn press_target(layout: &Layout, target: Target, y: u32) {
    match target {
        Target::Item(pos) => activate(pos),
        Target::ViewSwitch => switch_view(),
        Target::Search => {}
        Target::PowerButton => state().nav.power_open = true,
        Target::PowerEntry(i) => {
            if let Some(action) = PowerAction::ALL.get(i).filter(|a| a.is_enabled()) {
                perform_power(*action);
            }
        }
        Target::Thumb | Target::Track => press_scrollbar(layout, target, y),
    }
}

/// Handle a left press. Returns true when the menu consumed it, which is
/// always the case while it is open: a press outside only closes it.
pub fn handle_click(px: u32, py: u32) -> bool {
    if !is_open() {
        return false;
    }
    let layout = screen_layout();
    let st = state();
    if st.nav.power_open {
        if layout.power_menu.contains(px, py) {
            if let Some(entry @ Target::PowerEntry(_)) = st.target_at(px, py) {
                press_target(&layout, entry, py);
            }
        } else {
            st.nav.power_open = false;
        }
        return true;
    }
    if !layout.menu.contains(px, py) {
        close();
        return true;
    }
    if let Some(target) = st.target_at(px, py) {
        press_target(&layout, target, py);
    }
    true
}

fn drag_scrollbar(layout: &Layout, y: u32) {
    let st = state();
    let Some(grab) = st.drag_grab else { return };
    let count = visible_list(layout).len();
    st.nav.scroll = layout.scroll_for_thumb(count, y.saturating_sub(grab));
}

/// Track the pointer: hover highlight and scrollbar dragging.
pub fn handle_move(px: u32, py: u32) {
    let st = state();
    if !st.open {
        return;
    }
    if st.drag_grab.is_some() {
        if crate::wm::input::mouse_buttons().0 {
            drag_scrollbar(&screen_layout(), py);
        } else {
            st.drag_grab = None;
        }
        return;
    }
    st.power_hover = None;
    match st.target_at(px, py) {
        Some(Target::Item(pos)) => st.nav.selected = Some(pos),
        Some(Target::PowerEntry(i)) => st.power_hover = Some(i),
        _ => {}
    }
}

/// Launch an app by name (used by the shell `app <name>` command and the
/// desktop's own automation paths).
pub fn launch_by_name(name: &str) -> bool {
    for app in APPS {
        if app.name.eq_ignore_ascii_case(name) {
            (app.launch)();
            return true;
        }
    }
    false
}

/// Focus an existing window with this exact title, else run `create`.
fn focus_or_create(title: &str, _w: u32, _h: u32, create: impl FnOnce()) {
    for id in 1..=window::max_id() {
        if window::exists(id) && window::get_title(id) == title {
            window::focus(id);
            return;
        }
    }
    create();
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn control_scancodes_map_to_menu_keys() {
        assert_eq!(key_from_scancode(SC_ESC, Some('\u{1b}')), Some(Key::Esc));
        assert_eq!(key_from_scancode(SC_ENTER, Some('\n')), Some(Key::Enter));
        assert_eq!(key_from_scancode(SC_BACKSPACE, None), Some(Key::Backspace));
        assert_eq!(key_from_scancode(SC_DOWN, None), Some(Key::Down));
    }

    #[test]
    fn printable_characters_become_search_input() {
        assert_eq!(key_from_scancode(0x2E, Some('c')), Some(Key::Char('c')));
        assert_eq!(key_from_scancode(0x39, Some(' ')), Some(Key::Char(' ')));
        assert_eq!(key_from_scancode(0x3B, None), None);
    }

    #[test]
    fn hit_tags_round_trip_every_target() {
        let targets = [
            Target::Item(37),
            Target::ViewSwitch,
            Target::Search,
            Target::PowerButton,
            Target::PowerEntry(3),
            Target::Thumb,
            Target::Track,
        ];
        for target in targets {
            assert_eq!(Target::from_tag(target.to_tag()), Some(target));
        }
    }

    #[test]
    fn builtin_items_carry_search_keywords() {
        let items = build_items(&[]);
        let terminal = items.iter().find(|item| item.name == "Terminal");
        assert!(terminal.is_some_and(|item| item.keywords.contains("console")));
        assert_eq!(items.len(), APPS.len());
    }
}
