//! Window-manager keyboard shortcuts, keyed by Set-1 scancodes.
//!
//! QEMU (and `lobstervm key`) deliver Set-1 make codes, the same space the
//! Files app, lock screen and terminal already use. The table lives here as a
//! pure function so the mapping is host-testable.

/// Set-1 scancode of the `1` key (workspaces 1-4 are `0x02..=0x05`).
const SC_DIGIT_1: u8 = 0x02;
/// Set-1 scancode of the `4` key.
const SC_DIGIT_4: u8 = 0x05;
/// Set-1 scancode of `Tab`.
pub const SC_TAB: u8 = 0x0F;
/// Set-1 scancode of `D`.
const SC_D: u8 = 0x20;
/// Set-1 scancode of `F4`.
const SC_F4: u8 = 0x3E;
/// Set-1 scancode of the up arrow.
const SC_UP: u8 = 0x48;
/// Set-1 scancode of the left arrow.
const SC_LEFT: u8 = 0x4B;
/// Set-1 scancode of the right arrow.
const SC_RIGHT: u8 = 0x4D;
/// Set-1 scancode of the down arrow.
const SC_DOWN: u8 = 0x50;

/// An action a window-manager shortcut asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// Super+D: minimise every window.
    ShowDesktop,
    /// Super+Left: snap the focused window to the left half.
    SnapLeft,
    /// Super+Right: snap the focused window to the right half.
    SnapRight,
    /// Super+Up: maximise the focused window.
    Maximize,
    /// Super+Down: minimise the focused window.
    Minimize,
    /// Super+Alt+1..4: switch to the workspace with this index.
    Workspace(usize),
    /// Alt+F4: close the focused window.
    CloseWindow,
}

/// Map a key press plus modifier state to a shortcut, if any.
#[must_use]
pub fn lookup(scancode: u8, super_held: bool, alt_held: bool) -> Option<Shortcut> {
    if alt_held && !super_held && scancode == SC_F4 {
        return Some(Shortcut::CloseWindow);
    }
    if !super_held {
        return None;
    }
    if alt_held {
        return workspace_for(scancode).map(Shortcut::Workspace);
    }
    match scancode {
        SC_D => Some(Shortcut::ShowDesktop),
        SC_LEFT => Some(Shortcut::SnapLeft),
        SC_RIGHT => Some(Shortcut::SnapRight),
        SC_UP => Some(Shortcut::Maximize),
        SC_DOWN => Some(Shortcut::Minimize),
        _ => None,
    }
}

fn workspace_for(scancode: u8) -> Option<usize> {
    (SC_DIGIT_1..=SC_DIGIT_4)
        .contains(&scancode)
        .then(|| usize::from(scancode - SC_DIGIT_1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_f4_is_set1_0x3e() {
        assert_eq!(lookup(0x3E, false, true), Some(Shortcut::CloseWindow));
    }

    #[test]
    fn f4_without_alt_does_nothing() {
        assert_eq!(lookup(0x3E, false, false), None);
    }

    #[test]
    fn alt_escape_no_longer_closes_windows() {
        assert_eq!(lookup(0x29, false, true), None);
        assert_eq!(lookup(0x01, false, true), None);
    }

    #[test]
    fn super_arrows_use_set1_codes() {
        assert_eq!(lookup(0x4B, true, false), Some(Shortcut::SnapLeft));
        assert_eq!(lookup(0x4D, true, false), Some(Shortcut::SnapRight));
        assert_eq!(lookup(0x48, true, false), Some(Shortcut::Maximize));
        assert_eq!(lookup(0x50, true, false), Some(Shortcut::Minimize));
    }

    #[test]
    fn super_d_shows_desktop() {
        assert_eq!(lookup(0x20, true, false), Some(Shortcut::ShowDesktop));
    }

    #[test]
    fn workspace_digits_map_to_indices() {
        assert_eq!(lookup(0x02, true, true), Some(Shortcut::Workspace(0)));
        assert_eq!(lookup(0x05, true, true), Some(Shortcut::Workspace(3)));
        assert_eq!(lookup(0x06, true, true), None);
    }
}
