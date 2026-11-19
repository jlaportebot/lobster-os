//! Keyboard and pointer input for the browser window.

use super::state::{Browser, SCROLL_STEP};
use super::url::MAX_URL_BYTES;
use super::view::{
    KEY_ADDRESS, KEY_BACK, KEY_FORWARD, KEY_LINK_BASE, KEY_RELOAD, KEY_SCROLL, KEY_STAR,
};
use crate::wm::controls::{drag, scroll, Track};
use crate::wm::hit_region::Hit;

const SC_ESC: u8 = 0x01;
const SC_BACKSPACE: u8 = 0x0E;
const SC_ENTER: u8 = 0x1C;
const SC_D: u8 = 0x20;
const SC_L: u8 = 0x26;
const SC_R: u8 = 0x13;
const SC_SPACE: u8 = 0x39;
const SC_F5: u8 = 0x3F;
const SC_HOME: u8 = 0x47;
const SC_UP: u8 = 0x48;
const SC_PAGE_UP: u8 = 0x49;
const SC_LEFT: u8 = 0x4B;
const SC_RIGHT: u8 = 0x4D;
const SC_END: u8 = 0x4F;
const SC_DOWN: u8 = 0x50;
const SC_PAGE_DOWN: u8 = 0x51;
/// Overlap kept between pages when paging, in px.
const PAGE_OVERLAP: i32 = 40;

/// Pointer press inside the browser window.
pub fn on_hit(b: &mut Browser, hit: &Hit) -> bool {
    crate::wm::window::focus(hit.win_id);
    let key = hit.region.tag;
    b.address_focus = key == KEY_ADDRESS;
    b.replace_on_type = b.address_focus;
    match key {
        KEY_BACK => {
            b.back();
        }
        KEY_FORWARD => {
            b.forward();
        }
        KEY_RELOAD => b.reload(),
        KEY_STAR => b.toggle_bookmark(),
        KEY_SCROLL => begin_scroll_drag(b, hit),
        KEY_ADDRESS => {}
        k if k >= KEY_LINK_BASE => follow_link(b, k - KEY_LINK_BASE),
        _ => return false,
    }
    super::controls().focus = if b.address_focus { KEY_ADDRESS } else { 0 };
    crate::wm::dirty::mark();
    true
}

fn follow_link(b: &mut Browser, id: u32) {
    let href = b
        .page
        .as_ref()
        .and_then(|p| p.layout.links.get(id as usize))
        .cloned();
    if let Some(href) = href {
        b.follow(&href);
    }
}

fn begin_scroll_drag(b: &Browser, hit: &Hit) {
    let Some(track) = super::controls().track(KEY_SCROLL) else {
        return;
    };
    let grab = grab_offset(b, &track, hit.screen_y as i32);
    let value = track.value_at(hit.screen_x as i32, hit.screen_y as i32, grab);
    on_drag(KEY_SCROLL, value);
    drag::begin(KEY_SCROLL, track, grab, on_drag);
}

/// Where inside the thumb a press landed; presses outside grab its middle.
fn grab_offset(b: &Browser, track: &Track, press_y: i32) -> i32 {
    let Track::Vertical {
        y,
        h,
        view_h,
        content_h,
    } = *track
    else {
        return 0;
    };
    let (offset, len) = scroll::thumb(h, view_h, content_h, b.scroll);
    let top = y + offset as i32;
    if press_y >= top && press_y < top + len as i32 {
        press_y - top
    } else {
        (len / 2) as i32
    }
}

fn on_drag(key: u32, value: u32) {
    if key != KEY_SCROLL {
        return;
    }
    if let Some(b) = super::instance() {
        b.scroll = value as i32;
        b.clamp_scroll();
    }
}

/// Keyboard input while the browser window is focused. True when consumed.
pub fn handle_key(b: &mut Browser, scancode: u8, ch: Option<char>) -> bool {
    let ctrl = crate::usb::hid::ctrl_held();
    let consumed = if ctrl && matches!(scancode, SC_L | SC_R | SC_D) {
        ctrl_shortcut(b, scancode)
    } else if b.address_focus {
        address_key(b, scancode, ch)
    } else {
        page_key(b, scancode)
    };
    if consumed {
        crate::wm::dirty::mark();
    }
    consumed
}

fn ctrl_shortcut(b: &mut Browser, scancode: u8) -> bool {
    match scancode {
        SC_L => focus_address(b),
        SC_R => b.reload(),
        _ => b.toggle_bookmark(),
    }
    true
}

fn focus_address(b: &mut Browser) {
    b.address_focus = true;
    b.replace_on_type = true;
    super::controls().focus = KEY_ADDRESS;
}

fn leave_address(b: &mut Browser) {
    b.address_focus = false;
    b.replace_on_type = false;
    super::controls().focus = 0;
}

fn address_key(b: &mut Browser, scancode: u8, ch: Option<char>) -> bool {
    match (scancode, ch) {
        (SC_ENTER, _) => {
            let target = b.address.clone();
            leave_address(b);
            b.navigate(&target);
        }
        (SC_ESC, _) => {
            b.address = b.url.clone();
            leave_address(b);
        }
        (SC_BACKSPACE, _) => {
            b.replace_on_type = false;
            b.address.pop();
        }
        (_, Some(c)) if c.is_ascii_graphic() || c == ' ' => type_char(b, c),
        _ => return false,
    }
    true
}

fn type_char(b: &mut Browser, c: char) {
    if b.replace_on_type {
        b.address.clear();
        b.replace_on_type = false;
    }
    if b.address.len() < MAX_URL_BYTES {
        b.address.push(c);
    }
}

fn page_key(b: &mut Browser, scancode: u8) -> bool {
    let page = b.view_h as i32 - PAGE_OVERLAP;
    let shift = crate::usb::hid::shift_held();
    let alt = crate::usb::hid::alt_held();
    match scancode {
        SC_UP => b.scroll_by(-SCROLL_STEP),
        SC_DOWN => b.scroll_by(SCROLL_STEP),
        SC_PAGE_UP => b.scroll_by(-page),
        SC_PAGE_DOWN => b.scroll_by(page),
        SC_SPACE => b.scroll_by(if shift { -page } else { page }),
        SC_HOME => b.scroll = 0,
        SC_END => b.scroll = b.max_scroll(),
        SC_F5 => b.reload(),
        SC_LEFT if alt => return b.back(),
        SC_RIGHT if alt => return b.forward(),
        SC_BACKSPACE => return b.back(),
        _ => return false,
    }
    true
}
