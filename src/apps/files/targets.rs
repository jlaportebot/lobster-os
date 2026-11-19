//! Hit-region keys: what each clickable region of the window means.

use super::model::SortKey;

/// A clickable part of the Files window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Back,
    Forward,
    Up,
    NewFolder,
    ViewList,
    ViewIcons,
    Place(usize),
    Crumb(usize),
    Header(SortKey),
    /// The whole item area (items are resolved from the press position).
    Body,
    Scrollbar,
}

const PLACE_BASE: u32 = 0x100;
const CRUMB_BASE: u32 = 0x200;
const HEADER_BASE: u32 = 0x300;
/// Most places / crumbs a key can address.
const SLOTS: u32 = 0x100;

const HEADER_KEYS: [SortKey; 4] = [
    SortKey::Name,
    SortKey::Size,
    SortKey::Modified,
    SortKey::Type,
];

impl Target {
    /// The hit-region tag for this target (never 0, which means "none").
    #[must_use]
    pub fn key(self) -> u32 {
        match self {
            Target::Back => 1,
            Target::Forward => 2,
            Target::Up => 3,
            Target::NewFolder => 4,
            Target::ViewList => 5,
            Target::ViewIcons => 6,
            Target::Body => 7,
            Target::Scrollbar => 8,
            Target::Place(i) => PLACE_BASE + (i as u32).min(SLOTS - 1),
            Target::Crumb(i) => CRUMB_BASE + (i as u32).min(SLOTS - 1),
            Target::Header(k) => {
                HEADER_BASE + HEADER_KEYS.iter().position(|h| *h == k).unwrap_or(0) as u32
            }
        }
    }

    /// Decode a hit-region tag.
    #[must_use]
    pub fn from_key(key: u32) -> Option<Target> {
        let slot = |base: u32| (key >= base && key < base + SLOTS).then(|| (key - base) as usize);
        Some(match key {
            1 => Target::Back,
            2 => Target::Forward,
            3 => Target::Up,
            4 => Target::NewFolder,
            5 => Target::ViewList,
            6 => Target::ViewIcons,
            7 => Target::Body,
            8 => Target::Scrollbar,
            _ => {
                if let Some(i) = slot(PLACE_BASE) {
                    Target::Place(i)
                } else if let Some(i) = slot(CRUMB_BASE) {
                    Target::Crumb(i)
                } else {
                    Target::Header(*HEADER_KEYS.get(slot(HEADER_BASE)?)?)
                }
            }
        })
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn every_target_round_trips_through_its_key() {
        let all = [
            Target::Back,
            Target::Forward,
            Target::Up,
            Target::NewFolder,
            Target::ViewList,
            Target::ViewIcons,
            Target::Body,
            Target::Scrollbar,
            Target::Place(0),
            Target::Place(6),
            Target::Crumb(0),
            Target::Crumb(9),
            Target::Header(SortKey::Name),
            Target::Header(SortKey::Size),
            Target::Header(SortKey::Modified),
            Target::Header(SortKey::Type),
        ];
        for target in all {
            assert_eq!(Target::from_key(target.key()), Some(target), "{target:?}");
        }
    }

    #[test]
    fn keys_are_unique_and_never_zero() {
        let keys = [
            Target::Back.key(),
            Target::Body.key(),
            Target::Place(1).key(),
            Target::Crumb(1).key(),
            Target::Header(SortKey::Type).key(),
        ];
        for (i, a) in keys.iter().enumerate() {
            assert_ne!(*a, 0);
            assert!(keys[i + 1..].iter().all(|b| a != b));
        }
        assert_eq!(Target::from_key(0), None);
        assert_eq!(Target::from_key(0x304), None);
    }
}
