//! Start-menu data model: searchable items, view modes and the keyboard
//! state machine. Everything here is pure so it can be tested on the host.

use alloc::string::String;
use alloc::vec::Vec;

/// Longest search text the field accepts.
pub const MAX_QUERY_LEN: usize = 48;

/// Where an item's launch behaviour lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Index into the built-in `APPS` registry.
    Builtin(usize),
    /// Index into the installed-package app list.
    Installed(usize),
}

/// One launchable entry as the menu lists it.
#[derive(Clone, Debug)]
pub struct Item {
    pub name: String,
    /// Extra words the search also matches (not shown).
    pub keywords: String,
    pub source: Source,
}

/// What the content area shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The pinned grid.
    Pinned,
    /// The alphabetical list of every app.
    AllApps,
}

/// The effective content mode: a non-empty query always shows results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Pinned,
    AllApps,
    Results,
}

/// Entries of the power menu, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerAction {
    Lock,
    LogOut,
    Restart,
    ShutDown,
}

impl PowerAction {
    /// Menu order.
    pub const ALL: [PowerAction; 4] = [
        PowerAction::Lock,
        PowerAction::LogOut,
        PowerAction::Restart,
        PowerAction::ShutDown,
    ];

    /// Label shown in the power menu.
    pub fn label(self) -> &'static str {
        match self {
            PowerAction::Lock => "Lock",
            PowerAction::LogOut => "Log out",
            PowerAction::Restart => "Restart",
            PowerAction::ShutDown => "Shut down",
        }
    }

    /// Whether the kernel has a real path behind the entry. There is no
    /// session lock screen and the greeter only runs once at boot, so Lock
    /// and Log out stay disabled rather than pretend.
    pub fn is_enabled(self) -> bool {
        matches!(self, PowerAction::Restart | PowerAction::ShutDown)
    }
}

/// Keys the menu understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Enter,
    Esc,
}

/// What a key press asks the caller to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// State changed (or nothing to do); keep the menu open.
    Handled,
    /// Launch the entry at this position of the visible list.
    Launch(usize),
    /// Close the menu.
    Close,
}

/// Geometry the keyboard handler needs from the current layout.
#[derive(Clone, Copy, Debug)]
pub struct NavCtx {
    /// Entries in the visible list.
    pub count: usize,
    /// Grid columns (1 in list modes).
    pub cols: usize,
    /// Rows that fit on screen in list modes.
    pub page: usize,
}

/// Search text, view and selection.
#[derive(Clone, Debug)]
pub struct Nav {
    pub query: String,
    pub view: View,
    /// Position in the visible list.
    pub selected: Option<usize>,
    /// First visible row in list modes.
    pub scroll: usize,
    pub power_open: bool,
}

impl Nav {
    pub const fn new() -> Self {
        Self {
            query: String::new(),
            view: View::Pinned,
            selected: None,
            scroll: 0,
            power_open: false,
        }
    }

    pub fn mode(&self) -> Mode {
        if !self.query.trim().is_empty() {
            return Mode::Results;
        }
        match self.view {
            View::Pinned => Mode::Pinned,
            View::AllApps => Mode::AllApps,
        }
    }

    /// Switch to `view`, dropping selection and scroll.
    pub fn show(&mut self, view: View) {
        self.view = view;
        self.selected = None;
        self.scroll = 0;
    }

    /// Apply one key. `ctx` describes the list the key acts on.
    pub fn on_key(&mut self, key: Key, ctx: NavCtx) -> Outcome {
        if self.power_open {
            self.power_open = false;
            if key == Key::Esc {
                return Outcome::Handled;
            }
        }
        match key {
            Key::Esc => self.escape(),
            Key::Enter => self
                .selected
                .filter(|&pos| pos < ctx.count)
                .map_or(Outcome::Handled, Outcome::Launch),
            Key::Char(c) => self.type_char(c),
            Key::Backspace => self.backspace(),
            Key::Up | Key::Down | Key::Left | Key::Right | Key::PageUp | Key::PageDown => {
                self.move_selection(key, ctx);
                Outcome::Handled
            }
        }
    }

    fn escape(&mut self) -> Outcome {
        if self.query.is_empty() {
            return Outcome::Close;
        }
        self.query.clear();
        self.selected = None;
        self.scroll = 0;
        Outcome::Handled
    }

    fn type_char(&mut self, c: char) -> Outcome {
        let leading_space = c == ' ' && self.query.is_empty();
        if !leading_space && self.query.len() < MAX_QUERY_LEN {
            self.query.push(c);
            self.selected = Some(0);
            self.scroll = 0;
        }
        Outcome::Handled
    }

    fn backspace(&mut self) -> Outcome {
        self.query.pop();
        self.selected = if self.query.is_empty() { None } else { Some(0) };
        self.scroll = 0;
        Outcome::Handled
    }

    fn move_selection(&mut self, key: Key, ctx: NavCtx) {
        if ctx.count == 0 {
            return;
        }
        let page = ctx.page.max(1) as isize;
        let cols = ctx.cols.max(1) as isize;
        let step = match key {
            Key::Up => -cols,
            Key::Down => cols,
            Key::Left if cols > 1 => -1,
            Key::Right if cols > 1 => 1,
            Key::PageUp => -page,
            Key::PageDown => page,
            _ => return,
        };
        let last = ctx.count as isize - 1;
        let target = match self.selected {
            None if step > 0 => 0,
            None => return,
            Some(pos) => pos as isize + step,
        };
        let in_grid = cols > 1;
        if in_grid && (target < 0 || target > last) {
            return;
        }
        let pos = target.clamp(0, last) as usize;
        self.selected = Some(pos);
        self.keep_selection_visible(ctx);
    }

    fn keep_selection_visible(&mut self, ctx: NavCtx) {
        let Some(pos) = self.selected else { return };
        if ctx.cols > 1 {
            return;
        }
        let page = ctx.page.max(1);
        if pos < self.scroll {
            self.scroll = pos;
        } else if pos >= self.scroll + page {
            self.scroll = pos + 1 - page;
        }
    }
}

impl Default for Nav {
    fn default() -> Self {
        Self::new()
    }
}

fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_ascii_lowercase().contains(needle_lower)
}

/// Case-insensitive substring match over the name and the keywords.
pub fn matches(item: &Item, query: &str) -> bool {
    let needle = query.trim().to_ascii_lowercase();
    needle.is_empty()
        || contains_ignore_case(&item.name, &needle)
        || contains_ignore_case(&item.keywords, &needle)
}

fn name_key(item: &Item) -> String {
    item.name.to_ascii_lowercase()
}

/// Indices of every item, sorted by name.
pub fn alphabetical(items: &[Item]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| name_key(&items[i]));
    order
}

/// Indices of the matching items: names that start with the query first,
/// then the rest, each group alphabetical.
pub fn search(items: &[Item], query: &str) -> Vec<usize> {
    let needle = query.trim().to_ascii_lowercase();
    let mut hits: Vec<usize> = alphabetical(items)
        .into_iter()
        .filter(|&i| matches(&items[i], query))
        .collect();
    hits.sort_by_key(|&i| !name_key(&items[i]).starts_with(&needle));
    hits
}

/// Indices of the pinned entries (the built-in registry, in order),
/// limited to what the grid can show.
pub fn pinned(items: &[Item], capacity: usize) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| matches!(item.source, Source::Builtin(_)))
        .map(|(i, _)| i)
        .take(capacity)
        .collect()
}

/// The list the keyboard and the hit regions index into.
pub fn visible(items: &[Item], nav: &Nav, grid_capacity: usize) -> Vec<usize> {
    match nav.mode() {
        Mode::Pinned => pinned(items, grid_capacity),
        Mode::AllApps => alphabetical(items),
        Mode::Results => search(items, &nav.query),
    }
}

/// Initial for the avatar circle (uppercase, `?` for an empty name).
pub fn initial(name: &str) -> char {
    name.chars().next().map_or('?', |c| c.to_ascii_uppercase())
}

/// Name shown in the bottom bar: the first GECOS field when set, else the
/// account name.
pub fn display_name(username: &str, gecos: Option<&str>) -> String {
    let full = gecos
        .and_then(|g| g.split(',').next())
        .map(str::trim)
        .filter(|g| !g.is_empty());
    String::from(full.unwrap_or(username))
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn item(name: &str, keywords: &str, source: Source) -> Item {
        Item {
            name: String::from(name),
            keywords: String::from(keywords),
            source,
        }
    }

    fn sample() -> Vec<Item> {
        alloc::vec![
            item("Terminal", "shell console", Source::Builtin(0)),
            item("Calculator", "math", Source::Builtin(1)),
            item("Settings", "preferences control", Source::Builtin(2)),
            item("hello", "greets you", Source::Installed(0)),
        ]
    }

    fn names(items: &[Item], order: &[usize]) -> Vec<String> {
        order.iter().map(|&i| items[i].name.clone()).collect()
    }

    fn ctx(count: usize) -> NavCtx {
        NavCtx {
            count,
            cols: 1,
            page: 3,
        }
    }

    #[test]
    fn search_is_case_insensitive_substring() {
        let items = sample();
        assert_eq!(names(&items, &search(&items, "CALC")), ["Calculator"]);
        assert_eq!(names(&items, &search(&items, "ermin")), ["Terminal"]);
    }

    #[test]
    fn search_matches_keywords() {
        let items = sample();
        assert_eq!(names(&items, &search(&items, "console")), ["Terminal"]);
        assert_eq!(names(&items, &search(&items, "Preferences")), ["Settings"]);
    }

    #[test]
    fn empty_query_matches_everything() {
        let items = sample();
        assert_eq!(search(&items, "").len(), items.len());
        assert_eq!(search(&items, "   ").len(), items.len());
    }

    #[test]
    fn unknown_query_has_no_results() {
        assert!(search(&sample(), "zzz").is_empty());
    }

    #[test]
    fn name_prefix_matches_rank_first() {
        let items = alloc::vec![
            item("Notes", "memo", Source::Builtin(0)),
            item("Editor", "write notes", Source::Builtin(1)),
        ];
        assert_eq!(names(&items, &search(&items, "no")), ["Notes", "Editor"]);
        assert_eq!(names(&items, &search(&items, "ed")), ["Editor"]);
    }

    #[test]
    fn all_apps_is_alphabetical_ignoring_case() {
        let items = sample();
        assert_eq!(
            names(&items, &alphabetical(&items)),
            ["Calculator", "hello", "Settings", "Terminal"]
        );
    }

    #[test]
    fn pinned_lists_only_builtins_up_to_capacity() {
        let items = sample();
        assert_eq!(pinned(&items, 10), [0, 1, 2]);
        assert_eq!(pinned(&items, 2), [0, 1]);
    }

    #[test]
    fn typing_selects_first_match_and_enter_launches_it() {
        let mut nav = Nav::new();
        for c in "calc".chars() {
            assert_eq!(nav.on_key(Key::Char(c), ctx(1)), Outcome::Handled);
        }
        assert_eq!(nav.mode(), Mode::Results);
        assert_eq!(nav.selected, Some(0));
        assert_eq!(nav.on_key(Key::Enter, ctx(1)), Outcome::Launch(0));
    }

    #[test]
    fn enter_without_selection_or_results_does_nothing() {
        let mut nav = Nav::new();
        assert_eq!(nav.on_key(Key::Enter, ctx(5)), Outcome::Handled);
        nav.on_key(Key::Char('z'), ctx(0));
        assert_eq!(nav.on_key(Key::Enter, ctx(0)), Outcome::Handled);
    }

    #[test]
    fn escape_clears_the_query_then_closes() {
        let mut nav = Nav::new();
        nav.on_key(Key::Char('t'), ctx(2));
        assert_eq!(nav.on_key(Key::Esc, ctx(2)), Outcome::Handled);
        assert!(nav.query.is_empty());
        assert_eq!(nav.mode(), Mode::Pinned);
        assert_eq!(nav.on_key(Key::Esc, ctx(2)), Outcome::Close);
    }

    #[test]
    fn escape_closes_an_open_power_menu_first() {
        let mut nav = Nav::new();
        nav.power_open = true;
        assert_eq!(nav.on_key(Key::Esc, ctx(1)), Outcome::Handled);
        assert!(!nav.power_open);
        assert_eq!(nav.on_key(Key::Esc, ctx(1)), Outcome::Close);
    }

    #[test]
    fn backspace_edits_and_empties_the_query() {
        let mut nav = Nav::new();
        nav.on_key(Key::Char('a'), ctx(1));
        nav.on_key(Key::Char('b'), ctx(1));
        nav.on_key(Key::Backspace, ctx(1));
        assert_eq!(nav.query, "a");
        nav.on_key(Key::Backspace, ctx(1));
        assert_eq!(nav.selected, None);
        nav.on_key(Key::Backspace, ctx(1));
        assert!(nav.query.is_empty());
    }

    #[test]
    fn leading_space_and_overlong_text_are_ignored() {
        let mut nav = Nav::new();
        nav.on_key(Key::Char(' '), ctx(1));
        assert!(nav.query.is_empty());
        for _ in 0..MAX_QUERY_LEN + 10 {
            nav.on_key(Key::Char('x'), ctx(1));
        }
        assert_eq!(nav.query.len(), MAX_QUERY_LEN);
    }

    #[test]
    fn up_down_move_and_scroll_in_a_list() {
        let mut nav = Nav::new();
        nav.show(View::AllApps);
        assert_eq!(nav.on_key(Key::Up, ctx(8)), Outcome::Handled);
        assert_eq!(nav.selected, None);
        for _ in 0..5 {
            nav.on_key(Key::Down, ctx(8));
        }
        assert_eq!(nav.selected, Some(4));
        assert_eq!(nav.scroll, 2);
        for _ in 0..20 {
            nav.on_key(Key::Down, ctx(8));
        }
        assert_eq!(nav.selected, Some(7));
        nav.on_key(Key::Up, ctx(8));
        assert_eq!(nav.selected, Some(6));
    }

    #[test]
    fn grid_moves_by_columns_and_stays_inside() {
        let grid = NavCtx {
            count: 8,
            cols: 3,
            page: 1,
        };
        let mut nav = Nav::new();
        nav.on_key(Key::Down, grid);
        assert_eq!(nav.selected, Some(0));
        nav.on_key(Key::Down, grid);
        assert_eq!(nav.selected, Some(3));
        nav.on_key(Key::Right, grid);
        assert_eq!(nav.selected, Some(4));
        nav.on_key(Key::Down, grid);
        assert_eq!(nav.selected, Some(7));
        nav.on_key(Key::Down, grid);
        assert_eq!(nav.selected, Some(7));
        nav.on_key(Key::Up, grid);
        assert_eq!(nav.selected, Some(4));
    }

    #[test]
    fn display_name_prefers_gecos() {
        assert_eq!(display_name("jo", Some("Jo Smith,,,")), "Jo Smith");
        assert_eq!(display_name("jo", Some("")), "jo");
        assert_eq!(display_name("user", None), "user");
        assert_eq!(initial("alice"), 'A');
        assert_eq!(initial(""), '?');
    }
}
