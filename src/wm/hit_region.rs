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

/// A resolved press inside a registered region.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub win_id: u32,
    pub region: HitRegion,
    /// Press position relative to the client-area origin.
    pub local_x: u32,
    pub local_y: u32,
    /// Press position in screen coordinates.
    pub screen_x: u32,
    pub screen_y: u32,
}

/// Per-window click handler; returns true when the press was consumed.
pub type HitHandler = fn(&Hit) -> bool;

/// Maximum hit regions per window.
const MAX_HIT_REGIONS: usize = 128;

/// Per-window hit-region registry.
/// Regions are cleared whenever the window's app redraws; the handler lives
/// until the window is destroyed.
pub struct HitRegistry {
    regions: [Option<HitRegion>; MAX_HIT_REGIONS],
    count: usize,
    handler: Option<HitHandler>,
}

impl HitRegistry {
    /// Create an empty registry.
    pub const fn new() -> Self {
        Self {
            regions: [None; MAX_HIT_REGIONS],
            count: 0,
            handler: None,
        }
    }

    /// Install the window's dispatch handler.
    pub fn set_handler(&mut self, handler: HitHandler) {
        self.handler = Some(handler);
    }

    /// Drop all regions but keep the handler: apps re-register regions on
    /// every redraw, while the handler is set once for the window's lifetime.
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

/// Public API: install the dispatch handler for a window. Survives `clear`.
pub fn set_handler(win_id: u32, handler: HitHandler) {
    if let Some(reg) = registry_for(win_id) {
        reg.set_handler(handler);
    }
}

/// Result of geometric window picking: the window and, when the point is in
/// its client area, the client-local position (None over the chrome).
type Pick = (u32, Option<(u32, u32)>);

/// Pure geometry: pick the topmost window whose full rectangle contains the
/// point. Only that window is consulted, so a covered window's regions can
/// never be hit "through" the window above it.
/// `windows_top_first` holds (id, (x, y, w, h)) in descending z-order.
fn pick_window(
    windows_top_first: &[(u32, (u32, u32, u32, u32))],
    sx: u32,
    sy: u32,
) -> Option<Pick> {
    let &(id, (x, y, _, _)) = windows_top_first
        .iter()
        .find(|(_, (x, y, w, h))| sx >= *x && sx < x + w && sy >= *y && sy < y + h)?;
    let (client_x, client_y) = crate::wm::window_frame::client_origin(x, y);
    if sx < client_x || sy < client_y {
        return Some((id, None));
    }
    Some((id, Some((sx - client_x, sy - client_y))))
}

/// Pick the topmost visible window under the point using compositor z-order.
fn pick_at(screen_x: u32, screen_y: u32) -> Option<Pick> {
    let mut windows: alloc::vec::Vec<(u32, (u32, u32, u32, u32))> = alloc::vec::Vec::new();
    for layer in compositor::visible_layers_bottom_to_top().iter().rev() {
        if let Some(rect) = crate::wm::window::get_info(layer.id) {
            windows.push((layer.id, rect));
        }
    }
    pick_window(&windows, screen_x, screen_y)
}

/// Id of the topmost visible window whose rectangle contains the point.
#[must_use]
pub fn topmost_window_at(screen_x: u32, screen_y: u32) -> Option<u32> {
    pick_at(screen_x, screen_y).map(|(id, _)| id)
}

/// Resolve a press at screen coordinates to a region of the topmost window
/// under the pointer. None when that window has no region there (chrome or
/// empty client area), so later router stages can claim the press.
pub fn route_hit(screen_x: u32, screen_y: u32) -> Option<Hit> {
    let (win_id, local) = pick_at(screen_x, screen_y)?;
    let (local_x, local_y) = local?;
    let region = *registry_for(win_id)?.hit_test(local_x, local_y)?;
    Some(Hit {
        win_id,
        region,
        local_x,
        local_y,
        screen_x,
        screen_y,
    })
}

/// Run the window's handler for a resolved hit. False when the window has no
/// handler or the handler declined the press.
pub fn dispatch(hit: &Hit) -> bool {
    let handler = registry_for(hit.win_id).and_then(|reg| reg.handler);
    handler.is_some_and(|handler| handler(hit))
}

/// Called when a window is destroyed: drop its regions and handler.
pub fn on_window_destroyed(win_id: u32) {
    if let Some(reg) = registry_for(win_id) {
        reg.clear();
        reg.handler = None;
    }
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

    const CLIENT_DX: u32 = crate::wm::window_frame::BORDER_W;
    const CLIENT_DY: u32 = crate::wm::window_frame::BORDER_W + crate::wm::window_frame::TITLE_H;

    fn no_op_handler(_: &Hit) -> bool {
        true
    }

    fn sample_hit(win_id: u32) -> Hit {
        Hit {
            win_id,
            region: HitRegion::new(0, 0, 1, 1, 0, HitKind::Tab),
            local_x: 0,
            local_y: 0,
            screen_x: 0,
            screen_y: 0,
        }
    }

    #[test]
    fn pick_window_returns_topmost_even_when_lower_window_has_region_there() {
        let top = (2, (50, 50, 100, 100));
        let bottom = (1, (0, 0, 200, 200));
        let picked = pick_window(&[top, bottom], 80, 90);
        assert_eq!(picked.map(|(id, _)| id), Some(2));
    }

    #[test]
    fn pick_window_falls_to_lower_window_outside_the_topmost_rect() {
        let top = (2, (50, 50, 100, 100));
        let bottom = (1, (0, 0, 200, 200));
        let picked = pick_window(&[top, bottom], 10, 10);
        assert_eq!(picked.map(|(id, _)| id), Some(1));
    }

    #[test]
    fn pick_window_reports_no_client_position_over_title_bar() {
        let win = (1, (100, 100, 200, 200));
        assert_eq!(
            pick_window(&[win], 150, 100 + CLIENT_DY - 1),
            Some((1, None))
        );
        assert_eq!(
            pick_window(&[win], 100 + CLIENT_DX - 1, 200),
            Some((1, None))
        );
    }

    #[test]
    fn pick_window_client_origin_uses_border_plus_title() {
        let win = (1, (100, 100, 200, 200));
        let picked = pick_window(&[win], 100 + CLIENT_DX, 100 + CLIENT_DY);
        assert_eq!(picked, Some((1, Some((0, 0)))));
    }

    #[test]
    fn pick_window_returns_none_outside_every_window() {
        let win = (1, (100, 100, 50, 50));
        assert_eq!(pick_window(&[win], 150, 150), None);
        assert_eq!(pick_window(&[], 0, 0), None);
    }

    #[test]
    fn clear_keeps_handler() {
        let mut reg = HitRegistry::new();
        reg.set_handler(no_op_handler);
        reg.add(HitRegion::new(0, 0, 10, 10, 1, HitKind::Tab));
        reg.clear();
        assert!(reg.is_empty());
        assert!(reg.handler.is_some());
    }

    #[test]
    fn destroy_drops_handler_and_regions() {
        let _guard = crate::test_support::serial_guard();
        const WIN: u32 = 31;
        set_handler(WIN, no_op_handler);
        assert!(register(WIN, 0, 0, 4, 4, 0, HitKind::Tab));
        on_window_destroyed(WIN);
        let reg = registry_for(WIN).expect("window id in range");
        assert!(reg.handler.is_none());
        assert!(reg.is_empty());
    }

    #[test]
    fn dispatch_returns_false_without_handler() {
        let _guard = crate::test_support::serial_guard();
        const WIN: u32 = 30;
        on_window_destroyed(WIN);
        assert!(!dispatch(&sample_hit(WIN)));
    }

    #[test]
    fn dispatch_calls_registered_handler() {
        let _guard = crate::test_support::serial_guard();
        const WIN: u32 = 29;
        set_handler(WIN, no_op_handler);
        assert!(dispatch(&sample_hit(WIN)));
        on_window_destroyed(WIN);
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
