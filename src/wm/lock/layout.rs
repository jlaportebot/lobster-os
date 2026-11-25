//! Pure geometry of the lock/login overlay, computed from the screen size so
//! drawing and hit-testing always agree and the maths is host-tested.

use core::ops::Range;

/// Side of a [`Rect`] test point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// True when (`px`, `py`) is inside.
    #[must_use]
    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    /// Right edge (exclusive).
    #[must_use]
    pub fn right(&self) -> u32 {
        self.x + self.w
    }

    /// Bottom edge (exclusive).
    #[must_use]
    pub fn bottom(&self) -> u32 {
        self.y + self.h
    }
}

/// Top of the clock as a fraction (1/8) of the screen height.
const CLOCK_TOP_DIVISOR: u32 = 8;
/// Height of the large clock line (Mono 26 cell).
pub const TIME_LINE_H: u32 = 32;
/// Gap between the clock and the date line.
const DATE_GAP: u32 = 12;
/// Height of a date or name line.
pub const TITLE_LINE_H: u32 = 26;
/// Avatar circle radius.
pub const AVATAR_RADIUS: u32 = 48;
/// Avatar centre as a fraction (21/50) of the screen height.
const AVATAR_CY_NUM: u32 = 21;
const AVATAR_CY_DEN: u32 = 50;
/// Gap between avatar and name.
const NAME_GAP: u32 = 14;
/// Gap between name and password field.
const FIELD_GAP: u32 = 18;
/// Password field width.
const FIELD_W: u32 = 240;
/// Height of the field and the unlock button.
const CONTROL_H: u32 = 40;
/// Button width.
const BUTTON_W: u32 = 90;
/// Gap between field and button.
const CONTROL_GAP: u32 = 10;
/// Gap between the controls and the message line.
const MESSAGE_GAP: u32 = 14;
/// Height of a message line.
pub const MESSAGE_LINE_H: u32 = 20;

/// Account row width in the greeter list.
const ROW_W: u32 = 340;
/// Account row height.
pub const ROW_H: u32 = 56;
/// Gap between account rows.
const ROW_GAP: u32 = 8;
/// Vertical space reserved above and below the list for title and margins.
const LIST_MARGIN: u32 = 200;
/// Row avatar radius.
pub const ROW_AVATAR_RADIUS: u32 = 20;

/// Positions of everything on the password screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Password {
    /// Horizontal centre of the screen.
    pub center_x: u32,
    /// Top of the clock line.
    pub time_y: u32,
    /// Top of the date line.
    pub date_y: u32,
    /// Avatar centre y.
    pub avatar_cy: u32,
    /// Top of the user-name line.
    pub name_y: u32,
    /// Password field.
    pub field: Rect,
    /// Unlock button.
    pub button: Rect,
    /// Top of the first message line (status).
    pub message_y: u32,
    /// Top of the second message line (Caps Lock hint).
    pub caps_y: u32,
}

/// Geometry of the password screen for a `width` x `height` screen.
#[must_use]
pub fn password(width: u32, height: u32) -> Password {
    let center_x = width / 2;
    let time_y = height / CLOCK_TOP_DIVISOR;
    let date_y = time_y + TIME_LINE_H + DATE_GAP;
    let avatar_cy = height * AVATAR_CY_NUM / AVATAR_CY_DEN;
    let name_y = avatar_cy + AVATAR_RADIUS + NAME_GAP;
    let controls_y = name_y + TITLE_LINE_H + FIELD_GAP;
    let left = center_x.saturating_sub((FIELD_W + CONTROL_GAP + BUTTON_W) / 2);
    let field = Rect {
        x: left,
        y: controls_y,
        w: FIELD_W,
        h: CONTROL_H,
    };
    let button = Rect {
        x: field.right() + CONTROL_GAP,
        y: controls_y,
        w: BUTTON_W,
        h: CONTROL_H,
    };
    let message_y = controls_y + CONTROL_H + MESSAGE_GAP;
    Password {
        center_x,
        time_y,
        date_y,
        avatar_cy,
        name_y,
        field,
        button,
        message_y,
        caps_y: message_y + MESSAGE_LINE_H,
    }
}

/// How many account rows fit on a screen of this height.
#[must_use]
pub fn max_rows(height: u32) -> usize {
    (height.saturating_sub(LIST_MARGIN) / (ROW_H + ROW_GAP)).max(1) as usize
}

/// Which accounts to show: a window of at most `max_rows` that keeps
/// `selected` visible.
#[must_use]
pub fn visible_range(count: usize, selected: usize, max_rows: usize) -> Range<usize> {
    let shown = count.min(max_rows);
    let first = (selected + 1).saturating_sub(shown);
    first..first + shown
}

/// Top of the "Choose a user" heading for a list of `rows` rows.
#[must_use]
pub fn list_title_y(height: u32, rows: usize) -> u32 {
    list_top(height, rows).saturating_sub(TITLE_LINE_H + ROW_GAP * 2)
}

fn list_top(height: u32, rows: usize) -> u32 {
    let block = rows as u32 * (ROW_H + ROW_GAP);
    height.saturating_sub(block) / 2
}

/// Rectangle of the `slot`-th visible row.
#[must_use]
pub fn row_rect(width: u32, height: u32, rows: usize, slot: usize) -> Rect {
    Rect {
        x: width.saturating_sub(ROW_W) / 2,
        y: list_top(height, rows) + slot as u32 * (ROW_H + ROW_GAP),
        w: ROW_W,
        h: ROW_H,
    }
}

/// What a click landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The account at this index of the full account list.
    Account(usize),
    /// The Unlock / Log in button.
    Button,
    /// Nothing clickable.
    Nothing,
}

/// Which screen a click is aimed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Account list with this many accounts and this one highlighted.
    List {
        /// Accounts on offer.
        count: usize,
        /// Highlighted account.
        selected: usize,
    },
    /// Password entry.
    Password,
}

/// Hit-test a click at (`x`, `y`) on a `width` x `height` screen.
#[must_use]
pub fn hit(width: u32, height: u32, screen: Screen, x: u32, y: u32) -> Hit {
    match screen {
        Screen::Password => {
            if password(width, height).button.contains(x, y) {
                Hit::Button
            } else {
                Hit::Nothing
            }
        }
        Screen::List { count, selected } => {
            let range = visible_range(count, selected, max_rows(height));
            let rows = range.len();
            range
                .enumerate()
                .find(|&(slot, _)| row_rect(width, height, rows, slot).contains(x, y))
                .map_or(Hit::Nothing, |(_, index)| Hit::Account(index))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREENS: [(u32, u32); 4] = [(1280, 720), (1024, 768), (800, 600), (640, 480)];

    #[test]
    fn password_screen_fits_every_supported_resolution() {
        for (w, h) in SCREENS {
            let p = password(w, h);
            assert!(p.button.right() <= w, "{w}x{h}");
            assert!(p.caps_y + MESSAGE_LINE_H <= h, "{w}x{h}");
            assert!(p.field.x >= 8, "{w}x{h}");
        }
    }

    #[test]
    fn password_screen_elements_do_not_overlap_vertically() {
        for (w, h) in SCREENS {
            let p = password(w, h);
            assert!(
                p.date_y + TITLE_LINE_H <= p.avatar_cy - AVATAR_RADIUS,
                "{w}x{h}"
            );
            assert!(p.name_y >= p.avatar_cy + AVATAR_RADIUS);
            assert!(p.field.y >= p.name_y + TITLE_LINE_H);
            assert!(p.message_y >= p.field.bottom());
            assert!(p.caps_y >= p.message_y + MESSAGE_LINE_H);
        }
    }

    #[test]
    fn field_and_button_are_disjoint_and_hit_test() {
        let p = password(1280, 720);
        assert!(p.field.right() < p.button.x);
        assert!(p.button.contains(p.button.x + 1, p.button.y + 1));
        assert!(!p.button.contains(p.field.x, p.field.y));
        assert!(!p.field.contains(p.button.x, p.button.y));
    }

    #[test]
    fn controls_are_centred_on_the_screen() {
        let p = password(1280, 720);
        let mid = (p.field.x + p.button.right()) / 2;
        assert!(mid.abs_diff(640) <= 1);
    }

    #[test]
    fn visible_range_keeps_the_selection_in_view() {
        assert_eq!(visible_range(3, 2, 8), 0..3);
        assert_eq!(visible_range(10, 0, 4), 0..4);
        assert_eq!(visible_range(10, 9, 4), 6..10);
        assert_eq!(visible_range(10, 5, 4), 2..6);
    }

    #[test]
    fn rows_are_stacked_inside_the_screen() {
        let n = 3;
        for slot in 0..n {
            let r = row_rect(1280, 720, n, slot);
            assert!(r.bottom() <= 720 && r.right() <= 1280);
        }
        assert!(row_rect(1280, 720, n, 1).y > row_rect(1280, 720, n, 0).bottom() - 1);
        assert!(list_title_y(720, n) < row_rect(1280, 720, n, 0).y);
    }

    #[test]
    fn click_on_unlock_button_is_a_button_hit() {
        let p = password(1280, 720);
        let hit_at = |x, y| hit(1280, 720, Screen::Password, x, y);
        assert_eq!(hit_at(p.button.x + 5, p.button.y + 5), Hit::Button);
        assert_eq!(hit_at(p.field.x + 5, p.field.y + 5), Hit::Nothing);
        assert_eq!(hit_at(0, 0), Hit::Nothing);
    }

    #[test]
    fn click_on_a_list_row_maps_to_its_account_index() {
        let screen = Screen::List {
            count: 3,
            selected: 0,
        };
        let r = row_rect(1280, 720, 3, 2);
        assert_eq!(hit(1280, 720, screen, r.x + 10, r.y + 10), Hit::Account(2));
        assert_eq!(hit(1280, 720, screen, 5, 5), Hit::Nothing);
    }

    #[test]
    fn click_between_rows_hits_nothing() {
        let screen = Screen::List {
            count: 3,
            selected: 0,
        };
        let first = row_rect(1280, 720, 3, 0);
        assert_eq!(
            hit(1280, 720, screen, first.x + 10, first.bottom() + 2),
            Hit::Nothing
        );
    }

    #[test]
    fn max_rows_is_at_least_one() {
        assert_eq!(max_rows(100), 1);
        assert!(max_rows(720) >= 6);
    }
}
