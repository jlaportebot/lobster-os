//! Output volume and mute, shared with the tray quick-settings slider.

use crate::wm::tray_popups;
use core::sync::atomic::{AtomicBool, Ordering};

static MUTED: AtomicBool = AtomicBool::new(false);
/// Pitch of the test sound in millihertz (A4).
const TEST_TONE_MHZ: u64 = 440_000;
/// Length of the test sound.
const TEST_TONE_MS: u64 = 300;

/// Whether output is muted.
#[must_use]
pub fn is_muted() -> bool {
    MUTED.load(Ordering::Relaxed)
}

/// Whether an audio output device exists in this machine.
#[must_use]
pub fn has_device() -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        crate::drivers::lobster_audio::detect()
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        false
    }
}

/// Push the effective volume (0 while muted) to the audio device.
pub fn apply() {
    #[cfg(target_arch = "aarch64")]
    {
        let level = if is_muted() {
            0
        } else {
            tray_popups::volume_level()
        };
        if crate::drivers::lobster_audio::detect() {
            crate::drivers::lobster_audio::set_volume(level);
        }
    }
}

/// Set volume (percent) from the Settings slider or the settings file.
pub fn set_volume(percent: u32) {
    tray_popups::set_volume_level(percent);
    apply();
}

/// Mute or unmute.
pub fn set_muted(muted: bool) {
    MUTED.store(muted, Ordering::Relaxed);
    apply();
}

/// The tray slider finished a drag: follow it and persist.
pub fn volume_changed() {
    apply();
    super::store::save();
}

/// Play a short tone. False when there is no device.
pub fn play_test() -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        apply();
        crate::drivers::lobster_audio::beep(TEST_TONE_MHZ, TEST_TONE_MS) > 0
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (TEST_TONE_MHZ, TEST_TONE_MS);
        false
    }
}
