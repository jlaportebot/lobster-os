//! User-process GUI windows (C1)
//!
//! EL0 processes draw through validated syscalls instead of touching
//! the framebuffer directly:
//!   SYS_WIN_CREATE(x, y, w, h, title_ptr)  → win_id
//!   SYS_WIN_FILL(win_id, x, y, w, h, argb) → filled rect inside the window
//!   SYS_WIN_FLUSH(win_id)                 → mark visible/dirty
//!   SYS_WIN_CLOSE(win_id)                 → tear down
//!
//! The kernel owns the backing buffer; the WM composites user windows
//! with the standard themed frame so they behave like any other window.

use crate::display::fb::FramebufferInfo;

const MAX_USER_WINDOWS: usize = 4;
const MAX_TITLE: usize = 32;

#[derive(Clone, Copy)]
struct UserWindow {
    active: bool,
    owner_pid: u64,
    wm_id: Option<u32>,
    buf: *mut u32,
    buf_w: u32,
    buf_h: u32,
    capacity: usize,
    title: [u8; MAX_TITLE],
    title_len: usize,
    dirty: bool,
}

static mut WINDOWS: [UserWindow; MAX_USER_WINDOWS] = [UserWindow {
    active: false,
    owner_pid: 0,
    wm_id: None,
    buf: core::ptr::null_mut(),
    buf_w: 0,
    buf_h: 0,
    capacity: 0,
    title: [0; MAX_TITLE],
    title_len: 0,
    dirty: false,
}; MAX_USER_WINDOWS];

fn alloc_buffer(w: u32, h: u32) -> (usize, *mut u32) {
    let pixels = (w as usize) * (h as usize);
    if pixels == 0 {
        return (0, core::ptr::null_mut());
    }
    let layout = core::alloc::Layout::from_size_align(pixels * 4, 16).ok();
    let layout = match layout {
        Some(l) => l,
        None => return (0, core::ptr::null_mut()),
    };
    let ptr = unsafe { crate::memory::heap::heap_alloc(layout) };
    if ptr.is_null() {
        return (0, core::ptr::null_mut());
    }
    unsafe { core::ptr::write_bytes(ptr, 0, pixels * 4) };
    (pixels, ptr as *mut u32)
}

/// Create a user window. Returns the syscall error code (-1 on failure).
pub fn sys_win_create(x: u64, y: u64, w: u64, h: u64, title_ptr: u64) -> u64 {
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        return u64::MAX;
    }
    let pid = crate::sched::current_pid();
    unsafe {
        for slot in WINDOWS.iter_mut() {
            if slot.active {
                continue;
            }
            let (cap, buf) = alloc_buffer(w as u32, h as u32);
            if buf.is_null() {
                return u64::MAX;
            }
            // Copy the title from USER memory (bounded read via byte walk)
            let mut tlen = 0usize;
            let mut i = 0usize;
            while tlen < MAX_TITLE - 1 && i < 64 {
                let ch = *((title_ptr + i as u64) as *const u8);
                if ch == 0 {
                    break;
                }
                slot.title[tlen] = ch;
                tlen += 1;
                i += 1;
            }
            slot.title_len = tlen;
            slot.owner_pid = pid;
            slot.buf = buf;
            slot.capacity = cap;
            slot.buf_w = w as u32;
            slot.buf_h = h as u32;
            slot.dirty = true;
            slot.active = true;

            // Register a WM window so it gets chrome + input like any app
            let t = core::str::from_utf8(&slot.title[..tlen]).unwrap_or("EL0 App");
            slot.wm_id = crate::wm::window::create(x as u32, y as u32, w as u32 + 8, h as u32 + 34, t);
            crate::wm::dirty::mark();
            return (slot_index(slot) + 1) as u64; // 1-based id
        }
    }
    u64::MAX
}

unsafe fn slot_index(slot: &UserWindow) -> usize {
    let base = WINDOWS.as_ptr() as usize;
    (slot as *const UserWindow as usize - base) / core::mem::size_of::<UserWindow>()
}

unsafe fn slot_by_id(win_id: u64) -> Option<&'static mut UserWindow> {
    if win_id == 0 || win_id as usize > MAX_USER_WINDOWS {
        return None;
    }
    let slot = &mut WINDOWS[(win_id - 1) as usize];
    if !slot.active {
        return None;
    }
    // Ownership check — only the creator may draw
    if slot.owner_pid != crate::sched::current_pid() {
        return None;
    }
    Some(slot)
}

/// Fill a rectangle inside a user window with an ARGB color.
pub fn sys_win_fill(win_id: u64, x: u64, y: u64, w: u64, h: u64, argb: u64) -> u64 {
    unsafe {
        let slot = match slot_by_id(win_id) {
            Some(s) => s,
            None => return u64::MAX,
        };
        let bw = slot.buf_w as u64;
        let bh = slot.buf_h as u64;
        if x >= bw || y >= bh {
            return u64::MAX;
        }
        let x_end = (x + w).min(bw);
        let y_end = (y + h).min(bh);
        let pixel = argb as u32;
        for py in y..y_end {
            let row = slot.buf.add((py * bw) as usize);
            for px in x..x_end {
                core::ptr::write_volatile(row.add(px as usize), pixel);
            }
        }
        slot.dirty = true;
        0
    }
}

/// Flush: mark the window visible (the WM composites it each dirty cycle).
pub fn sys_win_flush(win_id: u64) -> u64 {
    unsafe {
        match slot_by_id(win_id) {
            Some(slot) => {
                slot.dirty = true;
                crate::wm::dirty::mark();
                0
            }
            None => u64::MAX,
        }
    }
}

/// Close and free a user window.
pub fn sys_win_close(win_id: u64) -> u64 {
    unsafe {
        let slot = match slot_by_id(win_id) {
            Some(s) => s,
            None => return u64::MAX,
        };
        if let Some(wm_id) = slot.wm_id {
            crate::wm::window::destroy(wm_id);
        }
        if !slot.buf.is_null() {
            let layout = core::alloc::Layout::from_size_align(slot.capacity * 4, 16)
                .unwrap_or(core::alloc::Layout::from_size_align(1, 1).unwrap());
            crate::memory::heap::heap_free(slot.buf as *mut u8, layout);
        }
        slot.active = false;
        slot.wm_id = None;
        slot.buf = core::ptr::null_mut();
        crate::wm::dirty::mark();
        0
    }
}

/// Composite all user windows into the framebuffer. Called from the WM
/// tick after the static apps, before the panel.
pub fn render_to_windows(fb: &FramebufferInfo) {
    unsafe {
        for slot in WINDOWS.iter_mut() {
            if !slot.active {
                continue;
            }
            let Some(wm_id) = slot.wm_id else { continue };
            let Some((wx, wy, ww, wh)) = crate::wm::window::get_info(wm_id) else {
                continue;
            };
            let focused = crate::wm::window::focused_id() == wm_id;
            let title = core::str::from_utf8(&slot.title[..slot.title_len]).unwrap_or("EL0 App");
            let (cx, cy, cw, ch) =
                crate::wm::window_frame::draw_frame(fb, wm_id, wx, wy, ww, wh, title, focused);
            // Blit the backing buffer into the client area (clip to both)
            if cw == 0 || ch == 0 || slot.buf.is_null() {
                continue;
            }
            for py in 0..ch.min(slot.buf_h) {
                for px in 0..cw.min(slot.buf_w) {
                    let argb = *slot.buf.add((py * slot.buf_w + px) as usize);
                    crate::display::graphics::draw_pixel(
                        fb,
                        cx + px,
                        cy + py,
                        argb,
                    );
                }
            }
        }
    }
}
