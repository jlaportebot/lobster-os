//! Wall-clock time.
//!
//! The kernel has no battery-backed clock of its own. At boot the platform
//! driver hands over the current UTC time and the local UTC offset
//! ([`set_boot_time`]); from then on wall time is that anchor plus monotonic
//! uptime. Calendar conversion is pure arithmetic and unit-tested on the host.

use core::sync::atomic::{AtomicI32, AtomicU64, Ordering};

const SECS_PER_MINUTE: u64 = 60;
const SECS_PER_HOUR: u64 = 3_600;
const SECS_PER_DAY: u64 = 86_400;
const DAYS_PER_WEEK: u64 = 7;
/// 1970-01-01 was a Thursday; weekdays count from Sunday = 0.
const EPOCH_WEEKDAY: u64 = 4;
const MS_PER_SEC: u64 = 1_000;

const WEEKDAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// UTC epoch seconds at the moment `BOOT_UPTIME_MS` was sampled (0 = unknown).
static BOOT_EPOCH_SECS: AtomicU64 = AtomicU64::new(0);
static BOOT_UPTIME_MS: AtomicU64 = AtomicU64::new(0);
static UTC_OFFSET_SECS: AtomicI32 = AtomicI32::new(0);

/// A broken-down calendar date and time of day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: i64,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// 0 = Sunday
    pub weekday: u8,
}

impl DateTime {
    /// Convert seconds since 1970-01-01T00:00:00 (in whatever zone the caller
    /// has already applied) into calendar fields.
    #[must_use]
    pub fn from_epoch_secs(secs: u64) -> Self {
        let days = secs / SECS_PER_DAY;
        let rem = secs % SECS_PER_DAY;
        let (year, month, day) = civil_from_days(days as i64);
        Self {
            year,
            month,
            day,
            hour: (rem / SECS_PER_HOUR) as u8,
            minute: (rem % SECS_PER_HOUR / SECS_PER_MINUTE) as u8,
            second: (rem % SECS_PER_MINUTE) as u8,
            weekday: ((days + EPOCH_WEEKDAY) % DAYS_PER_WEEK) as u8,
        }
    }

    #[must_use]
    pub fn weekday_name(&self) -> &'static str {
        WEEKDAY_NAMES[usize::from(self.weekday) % WEEKDAY_NAMES.len()]
    }

    #[must_use]
    pub fn month_name(&self) -> &'static str {
        MONTH_NAMES[usize::from(self.month.saturating_sub(1)) % MONTH_NAMES.len()]
    }
}

/// Days since the epoch → (year, month, day) in the proleptic Gregorian
/// calendar (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    const DAYS_PER_ERA: i64 = 146_097;
    const DAYS_TO_0000_03_01: i64 = 719_468;
    let z = days + DAYS_TO_0000_03_01;
    let era = z.div_euclid(DAYS_PER_ERA);
    let day_of_era = z.rem_euclid(DAYS_PER_ERA);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u8;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u8;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Anchor wall time: `epoch_secs` (UTC) was the time when uptime was
/// `uptime_ms`; `utc_offset_secs` is the local zone's offset from UTC.
pub fn set_boot_time(epoch_secs: u64, uptime_ms: u64, utc_offset_secs: i32) {
    BOOT_EPOCH_SECS.store(epoch_secs, Ordering::Relaxed);
    BOOT_UPTIME_MS.store(uptime_ms, Ordering::Relaxed);
    UTC_OFFSET_SECS.store(utc_offset_secs, Ordering::Relaxed);
}

/// True once a platform clock has provided the time.
#[must_use]
pub fn is_set() -> bool {
    BOOT_EPOCH_SECS.load(Ordering::Relaxed) != 0
}

/// UTC epoch seconds for a given uptime, or `None` if no clock was provided.
#[must_use]
pub fn epoch_secs_at(uptime_ms: u64) -> Option<u64> {
    let anchor = BOOT_EPOCH_SECS.load(Ordering::Relaxed);
    if anchor == 0 {
        return None;
    }
    let elapsed_ms = uptime_ms.saturating_sub(BOOT_UPTIME_MS.load(Ordering::Relaxed));
    Some(anchor + elapsed_ms / MS_PER_SEC)
}

/// Local calendar time for a given uptime.
#[must_use]
pub fn local_at(uptime_ms: u64) -> Option<DateTime> {
    let utc = epoch_secs_at(uptime_ms)?;
    let offset = i64::from(UTC_OFFSET_SECS.load(Ordering::Relaxed));
    let local = (utc as i64).saturating_add(offset).max(0) as u64;
    Some(DateTime::from_epoch_secs(local))
}

/// Local UTC offset in seconds.
#[must_use]
pub fn utc_offset_secs() -> i32 {
    UTC_OFFSET_SECS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_zero_is_thursday_first_of_january_1970() {
        let dt = DateTime::from_epoch_secs(0);
        assert_eq!((dt.year, dt.month, dt.day), (1970, 1, 1));
        assert_eq!((dt.hour, dt.minute, dt.second), (0, 0, 0));
        assert_eq!(dt.weekday_name(), "Thu");
    }

    #[test]
    fn converts_a_known_timestamp() {
        // 2026-11-10 03:33:49 UTC, a Tuesday (cross-checked with `date -u -d @1794281629`).
        let dt = DateTime::from_epoch_secs(1_794_281_629);
        assert_eq!((dt.year, dt.month, dt.day), (2026, 11, 10));
        assert_eq!((dt.hour, dt.minute, dt.second), (3, 33, 49));
        assert_eq!(dt.weekday_name(), "Tue");
        assert_eq!(dt.month_name(), "Nov");
    }

    #[test]
    fn handles_leap_day_and_century_rules() {
        // 2024-02-29 12:00:00 UTC
        let leap = DateTime::from_epoch_secs(1_709_208_000);
        assert_eq!((leap.year, leap.month, leap.day), (2024, 2, 29));
        // 2000-03-01 00:00:00 UTC (2000 is a leap year: divisible by 400)
        let y2k = DateTime::from_epoch_secs(951_868_800);
        assert_eq!((y2k.year, y2k.month, y2k.day), (2000, 3, 1));
        // 2100-03-01 00:00:00 UTC (2100 is NOT a leap year)
        let y2100 = DateTime::from_epoch_secs(4_107_542_400);
        assert_eq!((y2100.year, y2100.month, y2100.day), (2100, 3, 1));
    }

    #[test]
    fn last_second_of_the_year_rolls_over() {
        // 2025-12-31 23:59:59 UTC and the second after it
        let before = DateTime::from_epoch_secs(1_767_225_599);
        assert_eq!((before.year, before.month, before.day), (2025, 12, 31));
        assert_eq!((before.hour, before.minute, before.second), (23, 59, 59));
        let after = DateTime::from_epoch_secs(1_767_225_600);
        assert_eq!((after.year, after.month, after.day), (2026, 1, 1));
    }

    #[test]
    fn wall_time_advances_with_uptime_and_applies_offset() {
        set_boot_time(1_794_281_629, 2_000, -18_000);
        assert!(is_set());
        assert_eq!(epoch_secs_at(2_000), Some(1_794_281_629));
        assert_eq!(epoch_secs_at(62_000), Some(1_794_281_689));
        // An uptime before the anchor never goes backwards past it.
        assert_eq!(epoch_secs_at(0), Some(1_794_281_629));
        let local = local_at(2_000).expect("clock was set");
        // 03:33:49 UTC minus five hours is 22:33:49 the previous day.
        assert_eq!((local.year, local.month, local.day), (2026, 11, 9));
        assert_eq!((local.hour, local.minute), (22, 33));
        assert_eq!(local.weekday_name(), "Mon");
    }
}
