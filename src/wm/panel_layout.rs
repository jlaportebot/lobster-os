//! Taskbar geometry and behaviour as pure functions.
//!
//! Everything the panel decides without touching the framebuffer or the
//! window table lives here so it can be unit-tested on the host: where each
//! button goes at a given screen width, how the task strip degrades when it
//! runs out of room, what a click on a task button means, and how the clock
//! and badge text read. `panel.rs` feeds it a snapshot of the windows and
//! paints the result; the very same item list is used for hit-testing, so
//! what is drawn is exactly what is clickable.

use alloc::string::String;
use alloc::vec::Vec;

/// Total taskbar height in px. The single source of truth for the work area.
pub const PANEL_HEIGHT: u32 = 48;
/// Vertical inset of every button inside the bar.
pub const BUTTON_INSET_Y: u32 = 4;
/// Height of every button.
pub const BUTTON_H: u32 = PANEL_HEIGHT - 2 * BUTTON_INSET_Y;
/// Space between the bar's left edge and the start button.
pub const EDGE_PAD: u32 = 8;
/// Space between neighbouring buttons.
pub const GAP: u32 = 4;
/// Space between the start/pinned/task strip and the tray.
pub const SECTION_GAP: u32 = 8;
/// Width of the start button.
pub const START_W: u32 = 44;
/// Width of a pinned-app button and of an icon-only task button.
pub const ICON_BUTTON_W: u32 = 44;
/// Widest a labelled task button grows.
pub const TASK_MAX_W: u32 = 176;
/// Narrowest a labelled task button may get before the strip goes icon-only.
pub const TASK_MIN_W: u32 = 104;
/// Width of one tray glyph button.
pub const TRAY_ICON_W: u32 = 36;
/// Width of the clock block (time over date).
pub const CLOCK_W: u32 = 84;
/// Width of one workspace dot cell.
pub const WORKSPACE_CELL_W: u32 = 18;
/// Width of the show-desktop strip at the far right edge.
pub const SHOW_DESKTOP_W: u32 = 10;
/// Apps pinned next to the start button, by `start_menu::APPS` name.
pub const PINNED_APPS: [&str; 4] = ["Terminal", "Files", "Browser", "Settings"];
/// Largest unread count shown literally on the bell badge.
const BADGE_MAX: usize = 9;

/// An axis-aligned screen rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// True when the point lies inside the rectangle.
    #[must_use]
    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// What a taskbar item does when clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Toggle the start menu.
    Start,
    /// Pinned app by index into [`PINNED_APPS`].
    Pinned(usize),
    /// A running window of an unpinned app.
    Window(u32),
    /// The "+k" chip: windows that did not fit in the strip.
    Overflow,
    /// Workspace dot by index.
    Workspace(usize),
    /// Network tray glyph.
    Network,
    /// Volume tray glyph.
    Volume,
    /// Notification bell.
    Bell,
    /// Clock and date.
    Clock,
    /// Show-desktop strip.
    ShowDesktop,
}

/// What the panel needs to know about one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WinView {
    pub id: u32,
    /// Index into [`PINNED_APPS`] when the window belongs to a pinned app.
    pub pinned: Option<usize>,
    pub focused: bool,
    pub minimized: bool,
}

/// One laid-out taskbar item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub rect: Rect,
    pub action: Action,
    /// The window a task or pinned button stands for, when one is running.
    pub window: Option<WinView>,
    /// Whether a title label is drawn next to the icon.
    pub labelled: bool,
    /// Windows behind an [`Action::Overflow`] chip.
    pub hidden: u32,
}

/// How the strip of unpinned running windows is fitted into its room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskFit {
    pub width: u32,
    pub labelled: bool,
    pub visible: usize,
    pub hidden: usize,
}

/// Fit `count` task buttons into `avail` px: labelled while each gets at
/// least [`TASK_MIN_W`], then icon-only, then clipped with room kept for a
/// "+k" chip.
#[must_use]
pub fn fit_tasks(avail: u32, count: usize) -> TaskFit {
    if count == 0 {
        return TaskFit {
            width: 0,
            labelled: false,
            visible: 0,
            hidden: 0,
        };
    }
    let n = count as u32;
    let share = avail.saturating_sub((n - 1) * GAP) / n;
    if share >= TASK_MIN_W {
        return TaskFit {
            width: share.min(TASK_MAX_W),
            labelled: true,
            visible: count,
            hidden: 0,
        };
    }
    let slot = ICON_BUTTON_W + GAP;
    let slots = ((avail + GAP) / slot) as usize;
    if slots >= count {
        return TaskFit {
            width: ICON_BUTTON_W,
            labelled: false,
            visible: count,
            hidden: 0,
        };
    }
    let visible = slots.saturating_sub(1);
    TaskFit {
        width: ICON_BUTTON_W,
        labelled: false,
        visible,
        hidden: count - visible,
    }
}

/// The window a pinned app's button should act on: the focused one, else
/// the first running.
#[must_use]
pub fn pinned_window(views: &[WinView], index: usize) -> Option<WinView> {
    let mut mine = views.iter().filter(|v| v.pinned == Some(index));
    let first = mine.next().copied()?;
    if first.focused {
        return Some(first);
    }
    Some(mine.find(|v| v.focused).copied().unwrap_or(first))
}

/// Number of tray items placed left of the clock.
const TRAY_GLYPHS: [Action; 3] = [Action::Network, Action::Volume, Action::Bell];

/// Left edge of the tray (workspace dots) at `screen_w`.
#[must_use]
pub fn tray_left(screen_w: u32, workspaces: usize) -> u32 {
    let glyphs = TRAY_GLYPHS.len() as u32 * (TRAY_ICON_W + GAP);
    let dots = workspaces as u32 * WORKSPACE_CELL_W + GAP;
    screen_w.saturating_sub(SHOW_DESKTOP_W + CLOCK_W + glyphs + dots)
}

fn plain(rect: Rect, action: Action) -> Item {
    Item {
        rect,
        action,
        window: None,
        labelled: false,
        hidden: 0,
    }
}

fn button(x: u32, y: u32, w: u32) -> Rect {
    Rect {
        x,
        y: y + BUTTON_INSET_Y,
        w,
        h: BUTTON_H,
    }
}

/// Tray items, right to left from the show-desktop strip, returned in
/// left-to-right order.
fn tray_items(screen_w: u32, y: u32, workspaces: usize) -> Vec<Item> {
    let mut items = Vec::new();
    let mut x = tray_left(screen_w, workspaces);
    for index in 0..workspaces {
        let rect = button(x, y, WORKSPACE_CELL_W);
        items.push(plain(rect, Action::Workspace(index)));
        x += WORKSPACE_CELL_W;
    }
    x += GAP;
    for action in TRAY_GLYPHS {
        items.push(plain(button(x, y, TRAY_ICON_W), action));
        x += TRAY_ICON_W + GAP;
    }
    items.push(plain(button(x, y, CLOCK_W), Action::Clock));
    let strip = Rect {
        x: screen_w.saturating_sub(SHOW_DESKTOP_W),
        y,
        w: SHOW_DESKTOP_W,
        h: PANEL_HEIGHT,
    };
    items.push(plain(strip, Action::ShowDesktop));
    items
}

fn pinned_items(views: &[WinView], y: u32, x: &mut u32) -> Vec<Item> {
    let mut items = Vec::new();
    for index in 0..PINNED_APPS.len() {
        let mut item = plain(button(*x, y, ICON_BUTTON_W), Action::Pinned(index));
        item.window = pinned_window(views, index);
        items.push(item);
        *x += ICON_BUTTON_W + GAP;
    }
    items
}

fn task_items(views: &[WinView], y: u32, mut x: u32, avail: u32) -> Vec<Item> {
    let loose: Vec<WinView> = views
        .iter()
        .filter(|v| v.pinned.is_none())
        .copied()
        .collect();
    let fit = fit_tasks(avail, loose.len());
    let mut items = Vec::new();
    for view in loose.iter().take(fit.visible) {
        let mut item = plain(button(x, y, fit.width), Action::Window(view.id));
        item.window = Some(*view);
        item.labelled = fit.labelled;
        items.push(item);
        x += fit.width + GAP;
    }
    if fit.hidden > 0 && avail >= ICON_BUTTON_W {
        let mut chip = plain(button(x, y, ICON_BUTTON_W), Action::Overflow);
        chip.hidden = fit.hidden as u32;
        chip.window = loose.get(fit.visible).copied();
        items.push(chip);
    }
    items
}

/// Lay out the whole bar for a screen `screen_w` wide whose bar starts at
/// row `y`. Items never overlap and the task strip never reaches the tray.
#[must_use]
pub fn layout(screen_w: u32, y: u32, workspaces: usize, views: &[WinView]) -> Vec<Item> {
    let mut x = EDGE_PAD;
    let mut items = Vec::new();
    items.push(plain(button(x, y, START_W), Action::Start));
    x += START_W + SECTION_GAP;
    items.extend(pinned_items(views, y, &mut x));
    let tray = tray_items(screen_w, y, workspaces);
    let strip_end = tray_left(screen_w, workspaces).saturating_sub(SECTION_GAP);
    items.extend(task_items(views, y, x, strip_end.saturating_sub(x)));
    items.extend(tray);
    items
}

/// The item under a point, if any.
#[must_use]
pub fn item_at(items: &[Item], px: u32, py: u32) -> Option<&Item> {
    items.iter().find(|item| item.rect.contains(px, py))
}

/// What a click on a task or pinned button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskAction {
    /// Nothing is running: start the app.
    Launch,
    /// Un-minimise, focus and raise.
    Restore(u32),
    /// The focused window was clicked: minimise it.
    Minimize(u32),
}

/// Map a click on a button standing for `window` to its action.
#[must_use]
pub fn task_click(window: Option<WinView>) -> TaskAction {
    match window {
        None => TaskAction::Launch,
        Some(view) if view.focused && !view.minimized => TaskAction::Minimize(view.id),
        Some(view) => TaskAction::Restore(view.id),
    }
}

/// Show-desktop toggle outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopToggle {
    /// Minimise these windows.
    Minimize(Vec<u32>),
    /// Restore these windows, bottom of the old stack first.
    Restore(Vec<u32>),
}

/// Remembers which windows show-desktop hid so a second click brings back
/// exactly those.
#[derive(Debug, Default)]
pub struct ShowDesktop {
    hidden: Vec<u32>,
}

impl ShowDesktop {
    /// An empty stash.
    #[must_use]
    pub const fn new() -> Self {
        Self { hidden: Vec::new() }
    }

    /// Toggle: hide every window in `visible`, or bring back the stash
    /// (dropping windows that no longer satisfy `exists`).
    pub fn toggle(&mut self, visible: &[u32], exists: impl Fn(u32) -> bool) -> DesktopToggle {
        if self.hidden.is_empty() {
            self.hidden = visible.to_vec();
            return DesktopToggle::Minimize(self.hidden.clone());
        }
        let back: Vec<u32> = self.hidden.drain(..).filter(|id| exists(*id)).collect();
        DesktopToggle::Restore(back)
    }
}

/// Clock time, e.g. `23:13` or `1:05 PM`.
#[must_use]
pub fn time_text(hour: u8, minute: u8, hour24: bool) -> String {
    if hour24 {
        return alloc::format!("{:02}:{:02}", hour, minute);
    }
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let twelve = match hour % 12 {
        0 => 12,
        h => h,
    };
    alloc::format!("{}:{:02} {}", twelve, minute, suffix)
}

/// Short date under the clock, e.g. `Tue 17 Nov`.
#[must_use]
pub fn date_text(weekday: &str, day: u8, month: &str) -> String {
    alloc::format!("{} {} {}", weekday, day, month)
}

/// Bell badge text: empty for none, the number up to 9, then `9+`.
#[must_use]
pub fn badge_text(unread: usize) -> String {
    match unread {
        0 => String::new(),
        n if n <= BADGE_MAX => alloc::format!("{}", n),
        _ => alloc::format!("{}+", BADGE_MAX),
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::vec;

    const WIDE: u32 = 1920;
    const NARROW: u32 = 1280;
    const BAR_Y: u32 = 1032;
    const WORKSPACES: usize = 4;

    fn view(id: u32, pinned: Option<usize>) -> WinView {
        WinView {
            id,
            pinned,
            focused: false,
            minimized: false,
        }
    }

    fn loose(count: u32) -> Vec<WinView> {
        (1..=count).map(|id| view(id, None)).collect()
    }

    fn assert_disjoint(items: &[Item]) {
        for (i, a) in items.iter().enumerate() {
            for b in items.iter().skip(i + 1) {
                let apart = a.rect.x + a.rect.w <= b.rect.x || b.rect.x + b.rect.w <= a.rect.x;
                assert!(apart, "{:?} overlaps {:?}", a.action, b.action);
            }
        }
    }

    #[test]
    fn idle_bar_has_start_pinned_and_tray_without_overlap() {
        for width in [WIDE, NARROW] {
            let items = layout(width, BAR_Y, WORKSPACES, &[]);
            assert_eq!(items[0].action, Action::Start);
            assert!(items.iter().any(|i| i.action == Action::Pinned(3)));
            assert!(items.iter().any(|i| i.action == Action::Clock));
            assert_disjoint(&items);
            assert!(items.iter().all(|i| i.rect.x + i.rect.w <= width));
        }
    }

    #[test]
    fn tasks_sit_between_pinned_apps_and_tray() {
        let items = layout(NARROW, BAR_Y, WORKSPACES, &loose(3));
        let last_pin = items
            .iter()
            .find(|i| i.action == Action::Pinned(3))
            .unwrap();
        let first_task = items
            .iter()
            .find(|i| i.action == Action::Window(1))
            .unwrap();
        assert!(first_task.rect.x > last_pin.rect.x + last_pin.rect.w);
        let tray = tray_left(NARROW, WORKSPACES);
        for item in items
            .iter()
            .filter(|i| matches!(i.action, Action::Window(_)))
        {
            assert!(item.rect.x + item.rect.w < tray);
        }
    }

    #[test]
    fn few_tasks_are_labelled_at_the_maximum_width() {
        let fit = fit_tasks(1000, 2);
        assert!(fit.labelled);
        assert_eq!(fit.width, TASK_MAX_W);
        assert_eq!((fit.visible, fit.hidden), (2, 0));
    }

    #[test]
    fn crowded_tasks_shrink_to_icons_then_clip_with_a_chip() {
        let shrunk = fit_tasks(300, 4);
        assert!(!shrunk.labelled);
        assert_eq!((shrunk.visible, shrunk.hidden), (4, 0));
        let clipped = fit_tasks(300, 12);
        assert_eq!(clipped.visible + clipped.hidden, 12);
        assert!(clipped.hidden > 0);
        let used = (clipped.visible as u32 + 1) * (ICON_BUTTON_W + GAP);
        assert!(used <= 300 + GAP);
    }

    #[test]
    fn overflow_chip_counts_hidden_windows_and_stays_in_the_strip() {
        let items = layout(NARROW, BAR_Y, WORKSPACES, &loose(40));
        let chip = items.iter().find(|i| i.action == Action::Overflow).unwrap();
        let shown = items
            .iter()
            .filter(|i| matches!(i.action, Action::Window(_)))
            .count();
        assert_eq!(chip.hidden as usize + shown, 40);
        assert!(chip.rect.x + chip.rect.w < tray_left(NARROW, WORKSPACES));
        assert_disjoint(&items);
    }

    #[test]
    fn pinned_running_apps_are_not_duplicated_as_task_buttons() {
        let views = [view(7, Some(0)), view(8, None)];
        let items = layout(WIDE, BAR_Y, WORKSPACES, &views);
        let windows: Vec<_> = items
            .iter()
            .filter(|i| matches!(i.action, Action::Window(_)))
            .collect();
        assert_eq!(windows.len(), 1);
        let pin = items
            .iter()
            .find(|i| i.action == Action::Pinned(0))
            .unwrap();
        assert_eq!(pin.window.map(|w| w.id), Some(7));
    }

    #[test]
    fn pinned_button_prefers_the_focused_window() {
        let mut focused = view(9, Some(1));
        focused.focused = true;
        let views = [view(4, Some(1)), focused];
        assert_eq!(pinned_window(&views, 1).map(|w| w.id), Some(9));
        assert_eq!(pinned_window(&views, 2), None);
    }

    #[test]
    fn click_maps_to_launch_restore_or_minimize() {
        assert_eq!(task_click(None), TaskAction::Launch);
        let mut v = view(3, None);
        assert_eq!(task_click(Some(v)), TaskAction::Restore(3));
        v.focused = true;
        assert_eq!(task_click(Some(v)), TaskAction::Minimize(3));
        v.minimized = true;
        assert_eq!(task_click(Some(v)), TaskAction::Restore(3));
    }

    #[test]
    fn item_at_resolves_the_start_button_and_misses_gaps() {
        let items = layout(WIDE, BAR_Y, WORKSPACES, &[]);
        let start = items[0].rect;
        let hit = item_at(&items, start.x + 2, start.y + 2).map(|i| i.action);
        assert_eq!(hit, Some(Action::Start));
        assert!(item_at(&items, 0, 0).is_none());
    }

    #[test]
    fn show_desktop_hides_then_restores_the_same_windows() {
        let mut state = ShowDesktop::new();
        assert_eq!(
            state.toggle(&[2, 5], |_| true),
            DesktopToggle::Minimize(vec![2, 5])
        );
        assert_eq!(
            state.toggle(&[], |_| true),
            DesktopToggle::Restore(vec![2, 5])
        );
        assert_eq!(state.toggle(&[], |_| true), DesktopToggle::Minimize(vec![]));
    }

    #[test]
    fn show_desktop_skips_windows_closed_in_between() {
        let mut state = ShowDesktop::new();
        state.toggle(&[2, 5], |_| true);
        assert_eq!(
            state.toggle(&[], |id| id == 5),
            DesktopToggle::Restore(vec![5])
        );
    }

    #[test]
    fn clock_and_date_text_format() {
        assert_eq!(time_text(23, 13, true), "23:13");
        assert_eq!(time_text(0, 5, true), "00:05");
        assert_eq!(time_text(0, 5, false), "12:05 AM");
        assert_eq!(time_text(13, 5, false), "1:05 PM");
        assert_eq!(date_text("Tue", 17, "Nov"), "Tue 17 Nov");
    }

    #[test]
    fn badge_text_caps_at_nine_plus() {
        assert_eq!(badge_text(0), "");
        assert_eq!(badge_text(1), "1");
        assert_eq!(badge_text(9), "9");
        assert_eq!(badge_text(10), "9+");
        assert_eq!(badge_text(400), "9+");
    }
}
