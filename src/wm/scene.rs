//! Scene compositor.
//!
//! Every window owns an off-screen surface. Apps draw into their surface
//! (through [`window_target`]) using ordinary screen coordinates; nothing
//! but this module writes window pixels to the screen. A frame is built
//! into a back buffer — wallpaper, desktop icons, windows bottom-to-top,
//! then the shell overlays — and only the damaged rectangles are rebuilt
//! and copied to the device framebuffer, so stacking is always correct and
//! the screen never shows a half-drawn frame.

use alloc::vec::Vec;

use crate::display::fb::{self, FramebufferInfo};
use crate::display::graphics;
use crate::wm::{compositor, window};

const MAX_DAMAGE_RECTS: usize = 8;
/// Drop shadow: offset and darkness of the two strips beside each window.
const SHADOW_OFFSET: u32 = 6;
const SHADOW_ALPHA: u8 = 70;
const SHADOW_COLOR: u32 = 0xFF00_0000;

/// A screen rectangle: `x0,y0` inclusive, `x1,y1` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Rect {
    #[must_use]
    pub fn from_xywh(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self {
            x0: x,
            y0: y,
            x1: x.saturating_add(w),
            y1: y.saturating_add(h),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    #[must_use]
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }

    #[must_use]
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    #[must_use]
    pub fn clamp_to(&self, w: u32, h: u32) -> Rect {
        Rect {
            x0: self.x0.min(w),
            y0: self.y0.min(h),
            x1: self.x1.min(w),
            y1: self.y1.min(h),
        }
    }
}

/// Pending screen damage: a short list of disjoint rectangles.
pub struct DamageList {
    rects: [Rect; MAX_DAMAGE_RECTS],
    len: usize,
}

impl DamageList {
    #[must_use]
    pub const fn new() -> Self {
        const EMPTY: Rect = Rect {
            x0: 0,
            y0: 0,
            x1: 0,
            y1: 0,
        };
        Self {
            rects: [EMPTY; MAX_DAMAGE_RECTS],
            len: 0,
        }
    }

    /// Add a rectangle, merging it with any it overlaps. When the list is
    /// full everything collapses into one bounding box.
    pub fn add(&mut self, rect: Rect) {
        if rect.is_empty() {
            return;
        }
        let mut merged = rect;
        let mut index = 0;
        while index < self.len {
            if self.rects[index].intersects(&merged) {
                merged = merged.union(&self.rects[index]);
                self.len -= 1;
                self.rects[index] = self.rects[self.len];
                index = 0;
            } else {
                index += 1;
            }
        }
        if self.len == MAX_DAMAGE_RECTS {
            for other in &self.rects[..self.len] {
                merged = merged.union(other);
            }
            self.len = 0;
        }
        self.rects[self.len] = merged;
        self.len += 1;
    }

    /// Remove and return all pending rectangles.
    pub fn take(&mut self) -> ([Rect; MAX_DAMAGE_RECTS], usize) {
        let taken = (self.rects, self.len);
        self.len = 0;
        taken
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for DamageList {
    fn default() -> Self {
        Self::new()
    }
}

/// One window's off-screen pixels (stored framebuffer format; 0 = empty).
struct Surface {
    window_id: u32,
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

struct Scene {
    screen_w: u32,
    screen_h: u32,
    back: Vec<u32>,
    surfaces: Vec<Surface>,
    damage: DamageList,
}

static mut SCENE: Option<Scene> = None;

fn scene() -> Option<&'static mut Scene> {
    // SAFETY: the scene is created once by `init` and only touched from the
    // kernel's single render thread (the main loop and syscalls it runs).
    unsafe { (*core::ptr::addr_of_mut!(SCENE)).as_mut() }
}

/// Create (or re-create after a mode change) the back buffer.
pub fn init() {
    let screen = fb::device_info();
    if screen.width == 0 || screen.height == 0 {
        return;
    }
    let pixel_count = screen.width as usize * screen.height as usize;
    let surfaces = scene()
        .map(|old| core::mem::take(&mut old.surfaces))
        .unwrap_or_default();
    let mut damage = DamageList::new();
    damage.add(Rect::from_xywh(0, 0, screen.width, screen.height));
    let new_scene = Scene {
        screen_w: screen.width,
        screen_h: screen.height,
        back: alloc::vec![0u32; pixel_count],
        surfaces,
        damage,
    };
    // SAFETY: single render thread; see `scene`.
    unsafe {
        *core::ptr::addr_of_mut!(SCENE) = Some(new_scene);
    }
}

/// True once the compositor owns the screen.
#[must_use]
pub fn is_active() -> bool {
    scene().is_some()
}

/// Mark a screen rectangle as needing recomposition.
pub fn damage(rect: Rect) {
    if let Some(scene) = scene() {
        let clamped = rect.clamp_to(scene.screen_w, scene.screen_h);
        scene.damage.add(clamped);
    }
}

/// Mark the whole screen as needing recomposition.
pub fn damage_all() {
    if let Some(scene) = scene() {
        let full = Rect::from_xywh(0, 0, scene.screen_w, scene.screen_h);
        scene.damage.add(full);
    }
}

/// Screen area a window occupies, including its drop shadow.
fn window_extent(x: u32, y: u32, w: u32, h: u32) -> Rect {
    Rect::from_xywh(x, y, w + SHADOW_OFFSET, h + SHADOW_OFFSET)
}

/// Damage the area a window occupies (call before and after moving it).
pub fn damage_window(window_id: u32) {
    if let Some((x, y, w, h)) = window::get_info(window_id) {
        damage(window_extent(x, y, w, h));
    }
}

/// The render target for a window's content. Drawing uses screen
/// coordinates; pixels land in the window's surface, and the window is
/// marked damaged. Falls back to `fallback` when the compositor is off.
#[must_use]
pub fn window_target(window_id: u32, fallback: &FramebufferInfo) -> FramebufferInfo {
    let Some((x, y, w, h)) = window::get_info(window_id) else {
        return *fallback;
    };
    let Some(scene) = scene() else {
        return *fallback;
    };
    if w == 0 || h == 0 {
        return fallback.clipped(0, 0, 0, 0);
    }
    let index = match scene.surfaces.iter().position(|s| s.window_id == window_id) {
        Some(index) => index,
        None => {
            scene.surfaces.push(Surface {
                window_id,
                width: 0,
                height: 0,
                pixels: Vec::new(),
            });
            scene.surfaces.len() - 1
        }
    };
    let surface = &mut scene.surfaces[index];
    if surface.width != w || surface.height != h {
        surface.pixels = alloc::vec![0u32; w as usize * h as usize];
        surface.width = w;
        surface.height = h;
    }
    let extent = window_extent(x, y, w, h).clamp_to(scene.screen_w, scene.screen_h);
    scene.damage.add(extent);
    let screen = fb::device_info();
    // SAFETY: the surface holds exactly w*h pixels and lives in the scene
    // until the window is released; targets are used within one render call.
    let target = unsafe { screen.surface(surface.pixels.as_mut_ptr(), x, y, w, h) };
    // Apps that fetch the framebuffer themselves (fb::get_info) must land in
    // the surface too; `end_window_render` undoes the redirect.
    fb::set_render_target(Some(target));
    target
}

/// End of an app's render call: stop redirecting `fb::get_info`.
pub fn end_window_render() {
    fb::set_render_target(None);
}

/// Forget a window's surface (call when the window is destroyed, while its
/// geometry is still known so the vacated area gets repainted).
pub fn release_window(window_id: u32) {
    damage_window(window_id);
    if let Some(scene) = scene() {
        scene.surfaces.retain(|s| s.window_id != window_id);
    }
}

/// Copy one window's surface into the frame, skipping empty pixels on rows
/// that are not fully covered (rounded corners, animation margins).
fn blit_surface(target: &FramebufferInfo, surface: &Surface, x: u32, y: u32) {
    let width = surface.width as usize;
    let first_row = target.clip_y0.saturating_sub(y);
    let last_row = target.clip_y1.saturating_sub(y).min(surface.height);
    for row in first_row..last_row {
        let start = row as usize * width;
        let line = &surface.pixels[start..start + width];
        let is_solid = line[0] != 0 && line[width - 1] != 0;
        if is_solid {
            graphics::blit_stored(target, x, y + row, line, surface.width, 1);
        } else {
            graphics::blit_stored_keyed(target, x, y + row, line);
        }
    }
}

fn draw_shadow(target: &FramebufferInfo, x: u32, y: u32, w: u32, h: u32) {
    graphics::blend_rect(
        target,
        x + w,
        y + SHADOW_OFFSET,
        SHADOW_OFFSET,
        h,
        SHADOW_COLOR,
        SHADOW_ALPHA,
    );
    graphics::blend_rect(
        target,
        x + SHADOW_OFFSET,
        y + h,
        w.saturating_sub(SHADOW_OFFSET),
        SHADOW_OFFSET,
        SHADOW_COLOR,
        SHADOW_ALPHA,
    );
}

/// Draw every visible window into `target`, bottom-most first.
fn draw_windows(scene: &Scene, target: &FramebufferInfo) {
    let clip = Rect {
        x0: target.clip_x0,
        y0: target.clip_y0,
        x1: target.clip_x1,
        y1: target.clip_y1,
    };
    for layer in compositor::visible_layers_bottom_to_top() {
        if !window_extent(layer.x, layer.y, layer.w, layer.h).intersects(&clip) {
            continue;
        }
        let Some(surface) = scene.surfaces.iter().find(|s| s.window_id == layer.id) else {
            continue;
        };
        draw_shadow(target, layer.x, layer.y, surface.width, surface.height);
        blit_surface(target, surface, layer.x, layer.y);
    }
}

/// Shell layers drawn above the windows.
fn draw_overlays(target: &FramebufferInfo) {
    crate::wm::panel::draw(target);
    crate::wm::start_menu::draw(target);
    crate::wm::tray_popups::draw(target);
    crate::wm::alt_tab::draw(target);
    crate::wm::notifications::draw(target);
    crate::wm::context_menu::draw();
}

/// Copy a finished rectangle of the back buffer to the device.
fn present(scene: &Scene, device: &FramebufferInfo, rect: &Rect) {
    let width = scene.screen_w as usize;
    for row in rect.y0..rect.y1 {
        let start = row as usize * width + rect.x0 as usize;
        let line = &scene.back[start..start + (rect.x1 - rect.x0) as usize];
        graphics::blit_stored(device, rect.x0, row, line, rect.x1 - rect.x0, 1);
    }
}

/// Rebuild and show every damaged part of the screen. Returns true if
/// anything was drawn.
pub fn compose() -> bool {
    let Some(scene) = scene() else {
        return false;
    };
    if scene.damage.is_empty() {
        return false;
    }
    let device = fb::device_info();
    // SAFETY: the back buffer holds screen_w*screen_h pixels and outlives
    // this function; the target is dropped before `scene.back` is touched
    // through any other path.
    let back = unsafe {
        device.surface(
            scene.back.as_mut_ptr(),
            0,
            0,
            scene.screen_w,
            scene.screen_h,
        )
    };
    let (rects, count) = scene.damage.take();
    for rect in &rects[..count] {
        let target = back.clipped(rect.x0, rect.y0, rect.x1, rect.y1);
        fb::set_render_target(Some(target));
        crate::wm::wallpaper::paint_into(&target);
        crate::wm::desktop_icons::draw(&target);
        draw_windows(scene, &target);
        draw_overlays(&target);
        fb::set_render_target(None);
        present(scene, &device, rect);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: u32, y0: u32, x1: u32, y1: u32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    #[test]
    fn empty_rects_are_ignored() {
        let mut list = DamageList::new();
        list.add(rect(5, 5, 5, 9));
        assert!(list.is_empty());
    }

    #[test]
    fn disjoint_rects_stay_separate() {
        let mut list = DamageList::new();
        list.add(rect(0, 0, 10, 10));
        list.add(rect(100, 100, 110, 110));
        let (rects, count) = list.take();
        assert_eq!(count, 2);
        assert_eq!(rects[0], rect(0, 0, 10, 10));
        assert_eq!(rects[1], rect(100, 100, 110, 110));
        assert!(list.is_empty());
    }

    #[test]
    fn overlapping_rects_merge_into_their_bounding_box() {
        let mut list = DamageList::new();
        list.add(rect(0, 0, 10, 10));
        list.add(rect(5, 5, 20, 20));
        let (rects, count) = list.take();
        assert_eq!(count, 1);
        assert_eq!(rects[0], rect(0, 0, 20, 20));
    }

    #[test]
    fn a_bridging_rect_merges_transitively() {
        let mut list = DamageList::new();
        list.add(rect(0, 0, 10, 10));
        list.add(rect(20, 0, 30, 10));
        list.add(rect(8, 0, 22, 10));
        let (rects, count) = list.take();
        assert_eq!(count, 1);
        assert_eq!(rects[0], rect(0, 0, 30, 10));
    }

    #[test]
    fn overflow_collapses_to_one_bounding_box() {
        let mut list = DamageList::new();
        for i in 0..=MAX_DAMAGE_RECTS as u32 {
            list.add(rect(i * 100, 0, i * 100 + 10, 10));
        }
        let (rects, count) = list.take();
        assert_eq!(count, 1);
        assert_eq!(rects[0], rect(0, 0, MAX_DAMAGE_RECTS as u32 * 100 + 10, 10));
    }

    #[test]
    fn clamp_limits_a_rect_to_the_screen() {
        let clamped = rect(1900, 1000, 2500, 1300).clamp_to(1920, 1080);
        assert_eq!(clamped, rect(1900, 1000, 1920, 1080));
        assert!(rect(3000, 0, 3100, 10).clamp_to(1920, 1080).is_empty());
    }
}
