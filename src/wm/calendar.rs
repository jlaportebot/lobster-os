//! Month calendar shown by the taskbar clock: pure month maths plus the
//! popup body painted inside the shared tray-popup card.

use crate::display::fb::FramebufferInfo;
use crate::display::graphics::blend_rounded_rect;
use crate::display::text::{self, Face};
use crate::wm::theme::{self, Theme};

/// Days in a week.
pub const DAYS_PER_WEEK: usize = 7;
/// Rows a month grid needs at most.
pub const GRID_ROWS: usize = 6;
const WEEKDAY_LETTERS: [&str; DAYS_PER_WEEK] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
const MONTH_FULL_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS_IN_MONTH: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
/// Month offsets of Sakamoto's day-of-week algorithm.
const SAKAMOTO_OFFSETS: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
const FEBRUARY: u8 = 2;
const MARCH: u8 = 3;
const CELL_W: u32 = 40;
const CELL_H: u32 = 30;
const GRID_LEFT: u32 = 20;
const TITLE_TOP: u32 = 44;
const HEADER_TOP: u32 = 84;
const GRID_TOP: u32 = 112;
const TITLE_PX: u32 = 16;
const CELL_PX: u32 = 13;
const TODAY_RADIUS: u32 = 6;
const TODAY_ALPHA: u8 = 255;

/// True in leap years of the Gregorian calendar.
#[must_use]
pub fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days in `month` (1..=12) of `year`; 0 for an invalid month.
#[must_use]
pub fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        FEBRUARY if is_leap_year(year) => 29,
        1..=12 => DAYS_IN_MONTH[usize::from(month) - 1],
        _ => 0,
    }
}

/// Weekday of a date, Sunday = 0 (Sakamoto's algorithm).
#[must_use]
pub fn weekday_of(year: i64, month: u8, day: u8) -> u8 {
    let shifted = if month < MARCH { year - 1 } else { year };
    let offset = SAKAMOTO_OFFSETS[usize::from(month.clamp(1, 12)) - 1];
    let total = shifted + shifted / 4 - shifted / 100 + shifted / 400 + offset + i64::from(day);
    total.rem_euclid(DAYS_PER_WEEK as i64) as u8
}

/// A month laid out as weeks starting on Sunday; 0 marks an empty cell.
#[must_use]
pub fn month_grid(year: i64, month: u8) -> [[u8; DAYS_PER_WEEK]; GRID_ROWS] {
    let mut grid = [[0u8; DAYS_PER_WEEK]; GRID_ROWS];
    let start = usize::from(weekday_of(year, month, 1));
    for day in 1..=days_in_month(year, month) {
        let slot = start + usize::from(day) - 1;
        grid[slot / DAYS_PER_WEEK][slot % DAYS_PER_WEEK] = day;
    }
    grid
}

fn cell_origin(row: usize, col: usize, x0: u32, y0: u32) -> (u32, u32) {
    (
        x0 + GRID_LEFT + col as u32 * CELL_W,
        y0 + GRID_TOP + row as u32 * CELL_H,
    )
}

fn draw_centered_in(fb: &FramebufferInfo, rect: (u32, u32, u32, u32), s: &str, color: u32) {
    let style = theme::chrome_text(Face::Ui, CELL_PX, color, rect.3);
    let top = theme::centered_line_y(rect.1, rect.3, &style);
    text::draw_centered(fb, rect.0 + rect.2 / 2, top, s, &style);
}

fn draw_title(fb: &FramebufferInfo, th: &Theme, x0: u32, y0: u32, year: i64, month: u8) {
    let name = MONTH_FULL_NAMES[usize::from(month.clamp(1, 12)) - 1];
    let title = alloc::format!("{} {}", name, year);
    let style = theme::chrome_text(Face::UiBold, TITLE_PX, th.label_text, CELL_H);
    text::draw(fb, x0 + GRID_LEFT, y0 + TITLE_TOP, &title, &style);
}

fn draw_weekday_row(fb: &FramebufferInfo, th: &Theme, x0: u32, y0: u32) {
    for (col, letter) in WEEKDAY_LETTERS.iter().enumerate() {
        let (x, _) = cell_origin(0, col, x0, y0);
        let rect = (x, y0 + HEADER_TOP, CELL_W, CELL_H);
        draw_centered_in(fb, rect, letter, th.label_text_dim);
    }
}

/// Paint the calendar for `today` (`None` when no wall clock is set) inside
/// the popup card whose top-left corner is (`x0`, `y0`).
pub fn draw(fb: &FramebufferInfo, th: &Theme, x0: u32, y0: u32, today: Option<(i64, u8, u8)>) {
    let Some((year, month, day)) = today else {
        let rect = (x0, y0 + TITLE_TOP, 320, CELL_H);
        draw_centered_in(fb, rect, "Clock not set", th.label_text_dim);
        return;
    };
    draw_title(fb, th, x0, y0, year, month);
    draw_weekday_row(fb, th, x0, y0);
    let grid = month_grid(year, month);
    for (row, week) in grid.iter().enumerate() {
        for (col, &value) in week.iter().enumerate() {
            if value == 0 {
                continue;
            }
            let (x, y) = cell_origin(row, col, x0, y0);
            let is_today = value == day;
            if is_today {
                let pill = (x + 2, y + 2, CELL_W - 4, CELL_H - 4);
                blend_rounded_rect(fb, pill, TODAY_RADIUS, theme::accent_color(), TODAY_ALPHA);
            }
            let color = if is_today { 0xFFFFFFFF } else { th.label_text };
            draw_centered_in(
                fb,
                (x, y, CELL_W, CELL_H),
                &alloc::format!("{}", value),
                color,
            );
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn leap_years_follow_the_gregorian_rule() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2100));
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(2026));
    }

    #[test]
    fn month_lengths_cover_february_and_invalid_months() {
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 11), 30);
        assert_eq!(days_in_month(2026, 12), 31);
        assert_eq!(days_in_month(2026, 13), 0);
    }

    #[test]
    fn weekday_matches_known_dates() {
        assert_eq!(weekday_of(2026, 11, 17), 2);
        assert_eq!(weekday_of(1970, 1, 1), 4);
        assert_eq!(weekday_of(2000, 2, 29), 2);
    }

    #[test]
    fn november_2026_grid_starts_on_sunday_and_ends_on_the_thirtieth() {
        let grid = month_grid(2026, 11);
        assert_eq!(grid[0][0], 1);
        assert_eq!(grid[4][1], 30);
        let filled = grid.iter().flatten().filter(|d| **d != 0).count();
        assert_eq!(filled, 30);
    }

    #[test]
    fn a_month_can_need_six_rows() {
        let grid = month_grid(2026, 8);
        assert_ne!(grid[5][0], 0);
    }
}
