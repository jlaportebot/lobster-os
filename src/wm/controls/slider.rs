//! Slider maths: value <-> pixel position on a horizontal track.
//!
//! Pure functions so the click-to-jump, drag and clamping behaviour is
//! host-testable without a render target.

/// Value under pixel column `px` of a track starting at `track_x` and
/// `track_w` px wide, rounded to the nearest step and clamped to
/// `min..=max`. A degenerate track or range yields `min`.
#[must_use]
pub fn value_at(px: i32, track_x: i32, track_w: u32, min: u32, max: u32) -> u32 {
    if track_w == 0 || max <= min {
        return min;
    }
    let offset = i64::from(px) - i64::from(track_x);
    let offset = offset.clamp(0, i64::from(track_w));
    let span = i64::from(max - min);
    let width = i64::from(track_w);
    let scaled = (offset * span + width / 2) / width;
    min + scaled as u32
}

/// Pixel column of the thumb centre for `value` (clamped into the range).
#[must_use]
pub fn thumb_x(value: u32, track_x: i32, track_w: u32, min: u32, max: u32) -> i32 {
    if max <= min {
        return track_x;
    }
    let value = value.clamp(min, max);
    let span = i64::from(max - min);
    let offset = (i64::from(value - min) * i64::from(track_w) + span / 2) / span;
    track_x + offset as i32
}

/// `value` moved by `delta` steps, clamped to `min..=max`.
#[must_use]
pub fn step(value: u32, min: u32, max: u32, delta: i32) -> u32 {
    let moved = i64::from(value) + i64::from(delta);
    moved.clamp(i64::from(min), i64::from(max)) as u32
}
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn value_at_track_start_is_min() {
        assert_eq!(value_at(100, 100, 300, 0, 100), 0);
    }

    #[test]
    fn value_at_track_end_is_max() {
        assert_eq!(value_at(400, 100, 300, 0, 100), 100);
    }

    #[test]
    fn value_at_clamps_outside_the_track() {
        assert_eq!(value_at(-50, 100, 300, 0, 100), 0);
        assert_eq!(value_at(9000, 100, 300, 0, 100), 100);
    }

    #[test]
    fn value_at_midpoint_rounds_to_nearest() {
        assert_eq!(value_at(250, 100, 300, 0, 100), 50);
        assert_eq!(value_at(101, 100, 300, 0, 100), 0);
        assert_eq!(value_at(102, 100, 300, 0, 100), 1);
    }

    #[test]
    fn thumb_position_roundtrips_for_every_value() {
        for value in 0..=100 {
            let x = thumb_x(value, 100, 300, 0, 100);
            assert_eq!(value_at(x, 100, 300, 0, 100), value);
        }
    }

    #[test]
    fn degenerate_tracks_return_min() {
        assert_eq!(value_at(10, 0, 0, 5, 50), 5);
        assert_eq!(value_at(10, 0, 100, 7, 7), 7);
        assert_eq!(thumb_x(7, 20, 100, 7, 7), 20);
    }

    #[test]
    fn step_moves_and_clamps() {
        assert_eq!(step(50, 0, 100, 5), 55);
        assert_eq!(step(98, 0, 100, 5), 100);
        assert_eq!(step(2, 0, 100, -5), 0);
    }
}
