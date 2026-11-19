//! Display resolutions and the "Keep this resolution?" countdown.

/// Selectable modes, largest first.
pub const RESOLUTIONS: [(u32, u32); 6] = [
    (1920, 1080),
    (1600, 900),
    (1366, 768),
    (1280, 720),
    (1024, 768),
    (800, 600),
];

/// Time the user has to confirm a new mode before it is reverted.
pub const CONFIRM_TIMEOUT_MS: u64 = 15_000;
const MS_PER_SEC: u64 = 1_000;

/// Index of `width` x `height` in [`RESOLUTIONS`].
#[must_use]
pub fn index_of(width: u32, height: u32) -> Option<usize> {
    RESOLUTIONS.iter().position(|mode| *mode == (width, height))
}

/// What a countdown tick found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No change is waiting for confirmation.
    Idle,
    /// Counting down.
    Waiting { secs_left: u32 },
    /// Time ran out: switch back to this mode.
    Revert { to: (u32, u32) },
}

/// A resolution change awaiting the user's decision.
#[derive(Clone, Copy, Debug)]
pub struct Confirm {
    pending: Option<Pending>,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    previous: (u32, u32),
    deadline_ms: u64,
}

impl Confirm {
    /// Nothing pending.
    #[must_use]
    pub const fn new() -> Self {
        Self { pending: None }
    }

    /// Whether a change is waiting for Keep or Revert.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Start the countdown after switching from `previous` to `requested`.
    /// A change made while another is pending keeps the original mode as
    /// the revert target.
    pub fn begin(&mut self, previous: (u32, u32), _requested: (u32, u32), now_ms: u64) {
        let target = self.pending.map_or(previous, |pending| pending.previous);
        self.pending = Some(Pending {
            previous: target,
            deadline_ms: now_ms + CONFIRM_TIMEOUT_MS,
        });
    }

    /// Accept the new mode. False when nothing was pending.
    pub fn keep(&mut self) -> bool {
        self.pending.take().is_some()
    }

    /// Abandon the new mode: the mode to go back to, once.
    pub fn revert(&mut self) -> Option<(u32, u32)> {
        self.pending.take().map(|pending| pending.previous)
    }

    /// The countdown state at the current time without consuming a timeout.
    #[must_use]
    pub fn peek(&self, now_ms: u64) -> Outcome {
        let mut copy = *self;
        copy.tick(now_ms)
    }

    /// Advance the countdown to `now_ms`.
    pub fn tick(&mut self, now_ms: u64) -> Outcome {
        let Some(pending) = self.pending else {
            return Outcome::Idle;
        };
        if now_ms >= pending.deadline_ms {
            self.pending = None;
            return Outcome::Revert {
                to: pending.previous,
            };
        }
        let left_ms = pending.deadline_ms - now_ms;
        Outcome::Waiting {
            secs_left: left_ms.div_ceil(MS_PER_SEC) as u32,
        }
    }
}

impl Default for Confirm {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn idle_confirm_reports_idle() {
        let mut confirm = Confirm::new();
        assert!(!confirm.is_pending());
        assert_eq!(confirm.tick(5_000), Outcome::Idle);
    }

    #[test]
    fn counts_down_while_waiting() {
        let mut confirm = Confirm::new();
        confirm.begin((1280, 720), (1024, 768), 1_000);
        assert_eq!(confirm.tick(1_000), Outcome::Waiting { secs_left: 15 });
        assert_eq!(confirm.tick(1_001), Outcome::Waiting { secs_left: 15 });
        assert_eq!(confirm.tick(6_500), Outcome::Waiting { secs_left: 10 });
        assert_eq!(confirm.tick(15_999), Outcome::Waiting { secs_left: 1 });
    }

    #[test]
    fn times_out_exactly_at_the_deadline_and_reverts() {
        let mut confirm = Confirm::new();
        confirm.begin((1280, 720), (1024, 768), 1_000);
        assert_eq!(confirm.tick(16_000), Outcome::Revert { to: (1280, 720) });
        assert!(!confirm.is_pending());
    }

    #[test]
    fn keep_ends_the_countdown_without_reverting() {
        let mut confirm = Confirm::new();
        confirm.begin((1280, 720), (1024, 768), 0);
        assert!(confirm.keep());
        assert!(!confirm.is_pending());
        assert_eq!(confirm.tick(60_000), Outcome::Idle);
    }

    #[test]
    fn explicit_revert_returns_the_previous_mode_once() {
        let mut confirm = Confirm::new();
        confirm.begin((1280, 720), (800, 600), 0);
        assert_eq!(confirm.revert(), Some((1280, 720)));
        assert_eq!(confirm.revert(), None);
    }

    #[test]
    fn keep_with_nothing_pending_is_a_no_op() {
        assert!(!Confirm::new().keep());
    }

    #[test]
    fn a_second_change_keeps_the_original_mode_as_the_revert_target() {
        let mut confirm = Confirm::new();
        confirm.begin((1280, 720), (1024, 768), 0);
        confirm.begin((1024, 768), (800, 600), 2_000);
        assert_eq!(confirm.revert(), Some((1280, 720)));
    }

    #[test]
    fn resolution_list_has_the_six_supported_modes() {
        assert_eq!(RESOLUTIONS.len(), 6);
        assert_eq!(RESOLUTIONS[0], (1920, 1080));
        assert_eq!(RESOLUTIONS[5], (800, 600));
        assert_eq!(index_of(1366, 768), Some(2));
        assert_eq!(index_of(1000, 1000), None);
    }
}
