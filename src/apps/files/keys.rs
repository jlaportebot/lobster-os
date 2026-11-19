//! Keyboard handling while the Files window is focused.

use super::layout::{self, Move};
use super::state::State;
use super::{nav, ops};
use crate::usb::hid;

const SC_ESC: u8 = 0x01;
const SC_BACKSPACE: u8 = 0x0E;
const SC_ENTER: u8 = 0x1C;
const SC_A: u8 = 0x1E;
const SC_C: u8 = 0x2E;
const SC_X: u8 = 0x2D;
const SC_V: u8 = 0x2F;
const SC_N: u8 = 0x31;
const SC_F2: u8 = 0x3C;
const SC_F5: u8 = 0x3F;
const SC_HOME: u8 = 0x47;
const SC_UP: u8 = 0x48;
const SC_PAGE_UP: u8 = 0x49;
const SC_LEFT: u8 = 0x4B;
const SC_RIGHT: u8 = 0x4D;
const SC_END: u8 = 0x4F;
const SC_DOWN: u8 = 0x50;
const SC_PAGE_DOWN: u8 = 0x51;
const SC_DELETE: u8 = 0x53;
/// Rows a Page Up / Page Down moves when the real page size is unknown.
const DEFAULT_PAGE_ROWS: usize = 8;

fn cursor_move(scancode: u8) -> Option<Move> {
    Some(match scancode {
        SC_UP => Move::Up,
        SC_DOWN => Move::Down,
        SC_LEFT => Move::Left,
        SC_RIGHT => Move::Right,
        SC_HOME => Move::Home,
        SC_END => Move::End,
        SC_PAGE_UP => Move::PageUp,
        SC_PAGE_DOWN => Move::PageDown,
        _ => return None,
    })
}

/// Handle a key press; true when the Files window consumed it.
pub fn handle(st: &mut State, scancode: u8) -> bool {
    let (ctrl, alt, shift) = (hid::ctrl_held(), hid::alt_held(), hid::shift_held());
    let consumed = if ctrl {
        ctrl_key(st, scancode, shift)
    } else if alt {
        alt_key(st, scancode)
    } else {
        plain_key(st, scancode, shift)
    };
    if consumed {
        crate::wm::dirty::mark();
    }
    consumed
}

fn ctrl_key(st: &mut State, scancode: u8, shift: bool) -> bool {
    match scancode {
        SC_A => st.selection.select_all(st.entries.len()),
        SC_C => ops::copy_selection(st, false),
        SC_X => ops::copy_selection(st, true),
        SC_V => ops::paste(st),
        SC_N if shift => ops::ask_new(st, true),
        _ => return false,
    }
    true
}

fn alt_key(st: &mut State, scancode: u8) -> bool {
    match scancode {
        SC_LEFT => nav::go_back(st),
        SC_RIGHT => nav::go_forward(st),
        SC_UP => nav::go_up(st),
        _ => return false,
    }
    true
}

fn plain_key(st: &mut State, scancode: u8, shift: bool) -> bool {
    if let Some(mv) = cursor_move(scancode) {
        move_cursor(st, mv, shift);
        return true;
    }
    match scancode {
        SC_ENTER => super::actions::open_selection(st),
        SC_BACKSPACE => nav::go_up(st),
        SC_DELETE => ops::ask_delete(st),
        SC_F2 => ops::ask_rename(st),
        SC_F5 => nav::refresh(st),
        SC_ESC => st.selection.clear(),
        _ => return false,
    }
    true
}

fn move_cursor(st: &mut State, mv: Move, extend: bool) {
    let page = st.layout.map_or(DEFAULT_PAGE_ROWS, |l| {
        let rows = l.body.h / layout::ROW_H.min(layout::CELL_H);
        (rows as usize).saturating_sub(1).max(1)
    });
    let target = layout::move_target(st.selection.lead(), mv, st.entries.len(), st.cols, page);
    if let Some(i) = target {
        st.selection.move_to(i, extend);
        st.reveal = Some(i);
    }
}
