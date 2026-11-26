//! Switching the display mode, shared by the Display page and the revert
//! timer.

/// Current mode `(width, height)`.
#[must_use]
pub fn current() -> (u32, u32) {
    let info = crate::display::fb::get_info();
    (info.width, info.height)
}

/// Switch to `width` x `height`. The window manager re-fits windows,
/// dialogs and popups to the new size.
pub fn apply(width: u32, height: u32) -> Result<(), &'static str> {
    #[cfg(target_arch = "aarch64")]
    {
        crate::wm::display_change::apply_mode(width, height)
            .map(|_| ())
            .map_err(|_| "mode rejected")
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (width, height);
        Err("display mode switching needs the kernel")
    }
}
