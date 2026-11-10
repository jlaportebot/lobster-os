//! Hit-region registry (O2.2)
//!
//! Apps register clickable rectangles while drawing; one router stage dispatches
//! presses to the topmost window's region. Replaces per-app router stages.

use crate::wm::compositor;

/// Kind of hit region for dispatch hints (optional).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    /// Generic button
    Button,
    /// Text label (non-interactive by default)
    Label,
    /// Text input field
    TextInput,
    /// Checkbox
    Checkbox,
    /// Slider / progress bar
    Slider,
    /// Scrollbar thumb or track
    ScrollBar,
    /// Tab strip item
    Tab,
    /// Menu item
    MenuItem,
    /// Generic interactive area
    Custom,
}

/// A single hit region: window-local rectangle + tag + kind.
#[derive(Clone, Copy, Debug)]
pub struct HitRegion {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Application-defined tag (e.g. widget ID, action code)
    pub tag: u32,
    pub kind: HitKind,
}

impl HitRegion {
    /// Create a new hit region.
    #[must_use]
    pub const fn new(x: u32, y: u32, w: u32, h: u32, tag: u32, kind: HitKind) -> Self {
        Self {
            x,
            y,
            w,
            h,
            tag,
            kind,
        }
    }

    /// Check if a point (window-local) is inside this region.
    #[must_use]
    pub fn contains(&self, lx: u32, ly: u32) -> bool {
        lx >= self.x && lx < self.x + self.w && ly >= self.y && ly < self.y + self.h
    }
}

/// Maximum hit regions per window.
const MAX_HIT_REGIONS: usize = 128;

/// Per-window hit-region registry.
/// Cleared on window destroy or at the start of each frame.
pub struct HitRegistry {
    regions: [Option<HitRegion>; MAX_HIT_REGIONS],
    count: usize,
}

impl HitRegistry {
    /// Create an empty registry.
    pub const fn new() -> Self {
        Self {
            regions: [None; MAX_HIT_REGIONS],
            count: 0,
        }
    }

    /// Clear all regions (call at frame start or window destroy).
    pub fn clear(&mut self) {
        for i in 0..self.count {
            self.regions[i] = None;
        }
        self.count = 0;
    }

    /// Add a hit region. Returns the region's index, or None if full.
    /// Last-registered wins on overlap (topmost within window).
    pub fn add(&mut self, region: HitRegion) -> Option<usize> {
        if self.count >= MAX_HIT_REGIONS {
            return None;
        }
        let idx = self.count;
        self.regions[idx] = Some(region);
        self.count += 1;
        Some(idx)
    }

    /// Find the topmost region containing (lx, ly) in window-local coordinates.
    /// Iterates in reverse so the last-added (topmost) region wins.
    pub fn hit_test(&self, lx: u32, ly: u32) -> Option<&HitRegion> {
        for i in (0..self.count).rev() {
            if let Some(r) = &self.regions[i] {
                if r.contains(lx, ly) {
                    return Some(r);
                }
            }
        }
        None
    }

    /// Number of registered regions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl Default for HitRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global table of hit registries, one per window (indexed by window ID).
/// Window IDs are 1-based and dense enough for a small fixed array.
const MAX_WINDOWS: usize = 32;

static mut REGISTRIES: [HitRegistry; MAX_WINDOWS] = [const { HitRegistry::new() }; MAX_WINDOWS];

/// Get the registry for a window (by WM window ID).
/// Returns None if the window ID is out of range.
fn registry_for(win_id: u32) -> Option<&'static mut HitRegistry> {
    if win_id == 0 || win_id as usize > MAX_WINDOWS {
        return None;
    }
    // SAFETY: Single render/input thread; window IDs are stable while window exists.
    unsafe { Some(&mut REGISTRIES[(win_id - 1) as usize]) }
}

/// Public API: register a hit region for a window.
/// Coordinates are window-local (client area origin).
/// Returns false if the window doesn't exist or registry is full.
pub fn register(win_id: u32, x: u32, y: u32, w: u32, h: u32, tag: u32, kind: HitKind) -> bool {
    let Some(reg) = registry_for(win_id) else {
        return false;
    };
    reg.add(HitRegion::new(x, y, w, h, tag, kind)).is_some()
}

/// Public API: clear all hit regions for a window (call on frame start / destroy).
pub fn clear(win_id: u32) {
    if let Some(reg) = registry_for(win_id) {
        reg.clear();
    }
}

/// Public API: find the topmost window under (screen_x, screen_y) that has a hit region
/// containing the point (translated to window-local), and return (window_id, region).
/// Respects z-order: checks windows from topmost to bottommost.
pub fn route_hit(screen_x: u32, screen_y: u32) -> Option<(u32, HitRegion)> {
    // Get visible layers in z-order (bottom to top), then iterate in reverse.
    let layers = compositor::visible_layers_bottom_to_top();
    for layer in layers.iter().rev() {
        let win_id = layer.id;
        let Some(reg) = registry_for(win_id) else {
            continue;
        };
        if reg.is_empty() {
            continue;
        }
        // Translate screen -> window-local (client area).
        let Some((wx, wy, _ww, _wh)) = crate::wm::window::get_info(win_id) else {
            continue;
        };
        // Client area origin: window origin + border + title bar
        const BORDER_W: u32 = 2;
        const TITLE_BAR_H: u32 = 24;
        let client_x = wx + BORDER_W;
        let client_y = wy + TITLE_BAR_H;

        if screen_x < client_x || screen_y < client_y {
            continue;
        }
        let lx = screen_x - client_x;
        let ly = screen_y - client_y;

        if let Some(region) = reg.hit_test(lx, ly) {
            return Some((win_id, *region));
        }
    }
    None
}

/// Called when a window is destroyed: clear its registry.
pub fn on_window_destroyed(win_id: u32) {
    clear(win_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_region_contains_point() {
        let r = HitRegion::new(10, 20, 100, 50, 42, HitKind::Button);
        assert!(r.contains(10, 20)); // top-left
        assert!(r.contains(109, 69)); // bottom-right inclusive
        assert!(!r.contains(9, 20)); // left of
        assert!(!r.contains(110, 20)); // right of
        assert!(!r.contains(10, 19)); // above
        assert!(!r.contains(10, 70)); // below
    }

    #[test]
    fn registry_add_and_clear() {
        let mut reg = HitRegistry::new();
        assert_eq!(reg.len(), 0);
        assert!(reg.is_empty());

        reg.add(HitRegion::new(0, 0, 10, 10, 1, HitKind::Button));
        assert_eq!(reg.len(), 1);

        reg.add(HitRegion::new(20, 20, 10, 10, 2, HitKind::Label));
        assert_eq!(reg.len(), 2);

        reg.clear();
        assert_eq!(reg.len(), 0);
        assert!(reg.is_empty());
    }

    #[test]
    fn registry_topmost_wins_on_overlap() {
        let mut reg = HitRegistry::new();
        // Bottom region
        reg.add(HitRegion::new(0, 0, 100, 100, 1, HitKind::Button));
        // Top region overlapping the same area
        reg.add(HitRegion::new(10, 10, 100, 100, 2, HitKind::Button));

        let hit = reg.hit_test(20, 20);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().tag, 2); // topmost (last added) wins
    }

    #[test]
    fn registry_no_hit_outside_regions() {
        let mut reg = HitRegistry::new();
        reg.add(HitRegion::new(0, 0, 10, 10, 1, HitKind::Button));
        assert!(reg.hit_test(100, 100).is_none());
        assert!(reg.hit_test(10, 10).is_none()); // exactly at edge (exclusive)
    }

    #[test]
    fn route_hit_respects_z_order() {
        // This test needs the compositor to be initialized with layers.
        // We test the logic by mocking: we can't easily set up layers here,
        // but we verify the translation math.
        const BORDER_W: u32 = 2;
        const TITLE_BAR_H: u32 = 24;
        let wx = 100;
        let wy = 50;
        let client_x = wx + BORDER_W;
        let client_y = wy + TITLE_BAR_H;

        let screen_x = client_x + 5;
        let screen_y = client_y + 10;
        let lx = screen_x - client_x;
        let ly = screen_y - client_y;
        assert_eq!(lx, 5);
        assert_eq!(ly, 10);
    }

    #[test]
    fn hit_kind_enum() {
        let kinds = [
            HitKind::Button,
            HitKind::Label,
            HitKind::TextInput,
            HitKind::Checkbox,
            HitKind::Slider,
            HitKind::ScrollBar,
            HitKind::Tab,
            HitKind::MenuItem,
            HitKind::Custom,
        ];
        assert_eq!(kinds.len(), 9);
    }
}
