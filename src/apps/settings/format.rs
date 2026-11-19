//! Text formatting helpers for the Storage, Date & time and About pages.

use alloc::string::String;

const KIB: u64 = 1024;
const SECS_PER_MIN: u64 = 60;
const SECS_PER_HOUR: u64 = 3600;
const SECS_PER_DAY: u64 = 86_400;
const MS_PER_SEC: u64 = 1000;

/// Human size such as `1.5 KB` (binary multiples).
#[must_use]
pub fn bytes_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < KIB {
        return alloc::format!("{bytes} B");
    }
    let mut value = bytes as f64 / KIB as f64;
    let mut unit = 0;
    while value >= KIB as f64 && unit + 1 < UNITS.len() {
        value /= KIB as f64;
        unit += 1;
    }
    alloc::format!("{:.1} {}", value, UNITS[unit])
}

/// `part` as a whole percent of `total`, capped at 100 (0 for an empty total).
#[must_use]
pub fn percent(part: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    (part.min(total) * 100 / total) as u32
}

/// Uptime as its two most significant units, e.g. `1 h 2 min`.
#[must_use]
pub fn uptime_text(uptime_ms: u64) -> String {
    let secs = uptime_ms / MS_PER_SEC;
    if secs >= SECS_PER_DAY {
        let hours = secs % SECS_PER_DAY / SECS_PER_HOUR;
        return alloc::format!("{} d {} h", secs / SECS_PER_DAY, hours);
    }
    if secs >= SECS_PER_HOUR {
        let mins = secs % SECS_PER_HOUR / SECS_PER_MIN;
        return alloc::format!("{} h {} min", secs / SECS_PER_HOUR, mins);
    }
    if secs >= SECS_PER_MIN {
        return alloc::format!("{} min {} s", secs / SECS_PER_MIN, secs % SECS_PER_MIN);
    }
    alloc::format!("{secs} s")
}

/// Decode a `MIDR_EL1` value into e.g. `ARM Cortex-A76 r0p1`.
#[must_use]
pub fn cpu_name(midr: u64) -> String {
    let implementer = (midr >> 24) & 0xFF;
    let variant = (midr >> 20) & 0xF;
    let part = (midr >> 4) & 0xFFF;
    let revision = midr & 0xF;
    let core = match (implementer, part) {
        (0x41, 0xd03) => Some("Cortex-A53"),
        (0x41, 0xd07) => Some("Cortex-A57"),
        (0x41, 0xd08) => Some("Cortex-A72"),
        (0x41, 0xd0b) => Some("Cortex-A76"),
        (0x41, 0xd0c) => Some("Neoverse N1"),
        _ => None,
    };
    match (implementer, core) {
        (0x41, Some(name)) => alloc::format!("ARM {name} r{variant}p{revision}"),
        (0x41, None) => alloc::format!("ARM part {part:#x} r{variant}p{revision}"),
        _ => alloc::format!("Unknown vendor {implementer:#x} part {part:#x} r{variant}p{revision}"),
    }
}

/// `UTC+05:30` style label for an offset in seconds.
#[must_use]
pub fn utc_offset_text(offset_secs: i32) -> String {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.unsigned_abs();
    let hours = abs / SECS_PER_HOUR as u32;
    let mins = abs % SECS_PER_HOUR as u32 / SECS_PER_MIN as u32;
    alloc::format!("UTC{sign}{hours:02}:{mins:02}")
}
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn bytes_use_binary_units_with_one_decimal() {
        assert_eq!(bytes_text(0), "0 B");
        assert_eq!(bytes_text(900), "900 B");
        assert_eq!(bytes_text(1536), "1.5 KB");
        assert_eq!(bytes_text(64 * 1024 * 1024), "64.0 MB");
        assert_eq!(bytes_text(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn percent_handles_zero_total() {
        assert_eq!(percent(5, 0), 0);
        assert_eq!(percent(1, 4), 25);
        assert_eq!(percent(9, 4), 100);
    }

    #[test]
    fn uptime_text_picks_the_two_most_significant_units() {
        assert_eq!(uptime_text(5_000), "5 s");
        assert_eq!(uptime_text(125_000), "2 min 5 s");
        assert_eq!(uptime_text(3_725_000), "1 h 2 min");
        assert_eq!(uptime_text(93_600_000), "1 d 2 h");
    }

    #[test]
    fn midr_decodes_known_arm_cores() {
        assert_eq!(cpu_name(0x410F_D0B1), "ARM Cortex-A76 r0p1");
        assert_eq!(cpu_name(0x410F_D083), "ARM Cortex-A72 r0p3");
    }

    #[test]
    fn midr_with_unknown_part_shows_the_raw_part_number() {
        assert_eq!(cpu_name(0x410F_D999), "ARM part 0xd99 r0p9");
        assert_eq!(cpu_name(0x4E0F_0010), "Unknown vendor 0x4e part 0x1 r0p0");
    }

    #[test]
    fn utc_offset_is_shown_as_signed_hours_and_minutes() {
        assert_eq!(utc_offset_text(0), "UTC+00:00");
        assert_eq!(utc_offset_text(-18_000), "UTC-05:00");
        assert_eq!(utc_offset_text(19_800), "UTC+05:30");
    }
}
