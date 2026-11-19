//! Settings model: pages, and the action each control triggers.

use crate::display::icons::Glyph;

/// One page of the Settings window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Appearance,
    Display,
    Sound,
    Network,
    Users,
    DateTime,
    Storage,
    Apps,
    About,
}

impl Page {
    /// Every page in sidebar order.
    pub const ALL: [Page; 9] = [
        Page::Appearance,
        Page::Display,
        Page::Sound,
        Page::Network,
        Page::Users,
        Page::DateTime,
        Page::Storage,
        Page::Apps,
        Page::About,
    ];

    /// Sidebar label and page title.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Display => "Display",
            Page::Sound => "Sound",
            Page::Network => "Network",
            Page::Users => "Users",
            Page::DateTime => "Date & time",
            Page::Storage => "Storage",
            Page::Apps => "Apps",
            Page::About => "About",
        }
    }

    /// Symbolic icon shown next to the label.
    #[must_use]
    pub const fn glyph(self) -> Glyph {
        match self {
            Page::Appearance => Glyph::Grid,
            Page::Display => Glyph::Desktop,
            Page::Sound => Glyph::VolumeHigh,
            Page::Network => Glyph::Wifi,
            Page::Users => Glyph::Home,
            Page::DateTime => Glyph::Refresh,
            Page::Storage => Glyph::Folder,
            Page::Apps => Glyph::Menu,
            Page::About => Glyph::Settings,
        }
    }

    /// Page named `name` (case-insensitive); "date" and "datetime" are
    /// accepted for the Date & time page.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Page> {
        let name = name.trim();
        let alias = name.eq_ignore_ascii_case("date") || name.eq_ignore_ascii_case("datetime");
        if alias {
            return Some(Page::DateTime);
        }
        Page::ALL
            .into_iter()
            .find(|page| page.name().eq_ignore_ascii_case(name))
    }

    fn index(self) -> usize {
        Page::ALL.iter().position(|page| *page == self).unwrap_or(0)
    }

    /// The page `delta` rows away, clamped at the first and last page.
    #[must_use]
    pub fn neighbour(self, delta: i32) -> Page {
        let last = Page::ALL.len() as i32 - 1;
        let target = (self.index() as i32 + delta).clamp(0, last);
        Page::ALL[target as usize]
    }

    /// The next page, wrapping from the last to the first.
    #[must_use]
    pub fn cycle(self) -> Page {
        Page::ALL[(self.index() + 1) % Page::ALL.len()]
    }
}

/// What a control does when activated. The numeric key doubles as the hit
/// region tag and the keyboard-focus identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SelectPage(Page),
    /// 0 = Dark, 1 = Light.
    Theme(u8),
    Accent(u8),
    Wallpaper(u8),
    LargeText,
    ResolutionMenu,
    ResolutionPick(u8),
    ResolutionKeep,
    ResolutionRevert,
    Volume,
    Mute,
    TestSound,
    TestConnection,
    AddUser,
    RemoveUser(u8),
    Clock24,
    OpenFiles,
    Uninstall(u8),
    PageScroll,
    /// Click on the scrim behind an open popup.
    DismissPopup,
    /// The URL text field on the Network page.
    UrlField,
    /// Change (or remove) the signed-in user's password.
    ChangePassword,
}

/// Bits of a key that hold the action parameter.
const PARAM_BITS: u32 = 8;
const PARAM_MASK: u32 = (1 << PARAM_BITS) - 1;

impl Action {
    fn parts(self) -> (u32, u32) {
        match self {
            Action::SelectPage(page) => (1, page.index() as u32),
            Action::Theme(i) => (2, u32::from(i)),
            Action::Accent(i) => (3, u32::from(i)),
            Action::Wallpaper(i) => (4, u32::from(i)),
            Action::LargeText => (5, 0),
            Action::ResolutionMenu => (6, 0),
            Action::ResolutionPick(i) => (7, u32::from(i)),
            Action::ResolutionKeep => (8, 0),
            Action::ResolutionRevert => (9, 0),
            Action::Volume => (10, 0),
            Action::Mute => (11, 0),
            Action::TestSound => (12, 0),
            Action::TestConnection => (13, 0),
            Action::AddUser => (14, 0),
            Action::RemoveUser(i) => (15, u32::from(i)),
            Action::Clock24 => (16, 0),
            Action::OpenFiles => (17, 0),
            Action::Uninstall(i) => (18, u32::from(i)),
            Action::PageScroll => (19, 0),
            Action::DismissPopup => (20, 0),
            Action::UrlField => (21, 0),
            Action::ChangePassword => (22, 0),
        }
    }

    /// Stable numeric identity of this action (never 0).
    #[must_use]
    pub fn key(self) -> u32 {
        let (tag, param) = self.parts();
        (tag << PARAM_BITS) | param
    }

    /// Inverse of [`Action::key`]; `None` for keys no action produces.
    #[must_use]
    pub fn from_key(key: u32) -> Option<Action> {
        let param = key & PARAM_MASK;
        let small = param as u8;
        let action = match key >> PARAM_BITS {
            1 => Action::SelectPage(*Page::ALL.get(param as usize)?),
            2 => Action::Theme(small),
            3 => Action::Accent(small),
            4 => Action::Wallpaper(small),
            5 => Action::LargeText,
            6 => Action::ResolutionMenu,
            7 => Action::ResolutionPick(small),
            8 => Action::ResolutionKeep,
            9 => Action::ResolutionRevert,
            10 => Action::Volume,
            11 => Action::Mute,
            12 => Action::TestSound,
            13 => Action::TestConnection,
            14 => Action::AddUser,
            15 => Action::RemoveUser(small),
            16 => Action::Clock24,
            17 => Action::OpenFiles,
            18 => Action::Uninstall(small),
            19 => Action::PageScroll,
            20 => Action::DismissPopup,
            21 => Action::UrlField,
            22 => Action::ChangePassword,
            _ => return None,
        };
        Some(action)
    }
}
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn pages_are_listed_in_sidebar_order() {
        let names: Vec<&str> = Page::ALL.iter().map(|p| p.name()).collect();
        assert_eq!(
            names,
            [
                "Appearance",
                "Display",
                "Sound",
                "Network",
                "Users",
                "Date & time",
                "Storage",
                "Apps",
                "About"
            ]
        );
    }

    #[test]
    fn from_name_is_case_insensitive_and_accepts_aliases() {
        assert_eq!(Page::from_name("display"), Some(Page::Display));
        assert_eq!(Page::from_name("ABOUT"), Some(Page::About));
        assert_eq!(Page::from_name("date"), Some(Page::DateTime));
        assert_eq!(Page::from_name("Date & time"), Some(Page::DateTime));
        assert_eq!(Page::from_name("nonsense"), None);
    }

    #[test]
    fn neighbour_clamps_at_the_ends() {
        assert_eq!(Page::Appearance.neighbour(-1), Page::Appearance);
        assert_eq!(Page::About.neighbour(1), Page::About);
        assert_eq!(Page::Display.neighbour(1), Page::Sound);
        assert_eq!(Page::Sound.neighbour(-1), Page::Display);
    }

    #[test]
    fn cycle_wraps_around() {
        assert_eq!(Page::About.cycle(), Page::Appearance);
        assert_eq!(Page::Appearance.cycle(), Page::Display);
    }

    #[test]
    fn action_keys_roundtrip() {
        let samples = [
            Action::SelectPage(Page::Storage),
            Action::Theme(1),
            Action::Accent(5),
            Action::Wallpaper(6),
            Action::LargeText,
            Action::ResolutionMenu,
            Action::ResolutionPick(4),
            Action::ResolutionKeep,
            Action::ResolutionRevert,
            Action::Volume,
            Action::Mute,
            Action::TestSound,
            Action::TestConnection,
            Action::AddUser,
            Action::RemoveUser(3),
            Action::Clock24,
            Action::OpenFiles,
            Action::Uninstall(9),
            Action::PageScroll,
            Action::DismissPopup,
            Action::UrlField,
            Action::ChangePassword,
        ];
        for action in samples {
            assert_eq!(Action::from_key(action.key()), Some(action));
        }
    }

    #[test]
    fn unknown_keys_decode_to_none() {
        assert_eq!(Action::from_key(0), None);
        assert_eq!(Action::from_key(0xFFFF_FFFF), None);
    }

    #[test]
    fn distinct_actions_have_distinct_keys() {
        assert_ne!(Action::Accent(1).key(), Action::Accent(2).key());
        assert_ne!(Action::Accent(1).key(), Action::Wallpaper(1).key());
    }
}
