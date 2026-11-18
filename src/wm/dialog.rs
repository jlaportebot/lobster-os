//! Modal dialog system — alert/confirm/prompt + file picker
//!
//! A single global modal stack (max depth 4). Only the top dialog receives input.
//! Drawn last in the compositor so it's always on top.
//!
//! Public API:
//!   - alert(title, message)
//!   - confirm(title, message) -> bool
//!   - prompt(title, message, default) -> Option<String>
//!   - file_picker(title, mode, path) -> Option<String>
//!   - is_open() -> bool
//!   - draw(fb)
//!   - handle_click(px, py) -> bool
//!   - handle_key(scancode: u8) -> bool
//!   - take_result() -> Option<DialogResult>

use crate::display::fb::FramebufferInfo;
use crate::display::graphics::{draw_rect, draw_rect_outline, draw_text, fill_rounded_rect};
use crate::fs::vfs::list_dir;
use crate::wm::theme;
use crate::wm::widget::{Button, TextInput, WidgetState};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// Maximum dialog stack depth
const MAX_DIALOG_DEPTH: usize = 4;

/// Dialog dimensions
const DIALOG_W: u32 = 400;
const DIALOG_H: u32 = 200;
const DIALOG_TITLE_H: u32 = 32;
const BUTTON_W: u32 = 80;
const BUTTON_H: u32 = 28;
const BUTTON_GAP: u32 = 12;
const CONTENT_PAD: u32 = 20;
const INPUT_H: u32 = 32;

/// File picker specific dimensions
const FILE_PICKER_H: u32 = 400;
const FILE_LIST_ITEM_H: u32 = 24;
const FILE_LIST_VISIBLE: usize = 12;

/// Dialog kind variants
#[derive(Clone, Debug)]
pub enum DialogKind {
    /// Simple alert with OK button
    Alert {
        title: String,
        message: String,
        ok_label: String,
    },
    /// Confirmation with OK/Cancel
    Confirm {
        title: String,
        message: String,
        ok_label: String,
        cancel_label: String,
    },
    /// Text input prompt
    Prompt {
        title: String,
        message: String,
        default_value: String,
        ok_label: String,
        cancel_label: String,
    },
    /// File open/save picker
    FilePicker {
        title: String,
        mode: FilePickerMode,
        path: String,
        filename: String,
        filter: String,
    },
}

/// File picker mode
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilePickerMode {
    Open,
    Save,
}

/// Result of a dialog interaction
#[derive(Clone, Debug, PartialEq)]
pub enum DialogResult {
    /// Alert dismissed
    AlertOk,
    /// Confirm result
    Confirm(bool),
    /// Prompt result (Some = OK with value, None = Cancel)
    Prompt(Option<String>),
    /// File picker result (Some = path, None = Cancel)
    FilePicker(Option<String>),
}

impl fmt::Display for DialogResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialogResult::AlertOk => write!(f, "AlertOk"),
            DialogResult::Confirm(v) => write!(f, "Confirm({})", v),
            DialogResult::Prompt(v) => write!(f, "Prompt({:?})", v),
            DialogResult::FilePicker(v) => write!(f, "FilePicker({:?})", v),
        }
    }
}

/// A dialog instance on the stack
struct Dialog {
    kind: DialogKind,
    /// Buttons for this dialog (max 3: OK, Cancel, and one extra)
    buttons: [Option<Button>; 3],
    button_count: usize,
    /// Text input for Prompt/FilePicker
    text_input: Option<TextInput>,
    /// Focused button index (0..button_count-1)
    focused_btn: usize,
    /// File picker state
    file_entries: Vec<FileEntry>,
    file_selected: Option<usize>,
    file_scroll: usize,
}

/// File entry for the picker list
#[derive(Clone, Debug)]
struct FileEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

/// Global modal dialog stack
static mut DIALOG_STACK: [Option<Dialog>; MAX_DIALOG_DEPTH] = [const { None }; MAX_DIALOG_DEPTH];
static mut STACK_DEPTH: usize = 0;

/// Pending result from the top dialog (consumed by take_result)
static mut PENDING_RESULT: Option<DialogResult> = None;

/// Initialize the dialog system
pub fn init() {
    unsafe {
        STACK_DEPTH = 0;
        PENDING_RESULT = None;
    }
}

/// Check if any dialog is open
pub fn is_open() -> bool {
    unsafe { STACK_DEPTH > 0 }
}

/// Get the current stack depth
fn depth() -> usize {
    unsafe { STACK_DEPTH }
}

/// Push a dialog onto the stack
fn push_dialog(dialog: Dialog) -> bool {
    unsafe {
        if STACK_DEPTH >= MAX_DIALOG_DEPTH {
            return false;
        }
        DIALOG_STACK[STACK_DEPTH] = Some(dialog);
        STACK_DEPTH += 1;
        true
    }
}

/// Pop the top dialog
fn pop_dialog() -> Option<Dialog> {
    unsafe {
        if STACK_DEPTH == 0 {
            return None;
        }
        STACK_DEPTH -= 1;
        DIALOG_STACK[STACK_DEPTH].take()
    }
}

/// Get mutable reference to top dialog
fn top_dialog_mut() -> Option<&'static mut Dialog> {
    unsafe {
        if STACK_DEPTH == 0 {
            return None;
        }
        DIALOG_STACK[STACK_DEPTH - 1].as_mut()
    }
}

/// Get immutable reference to top dialog
fn top_dialog() -> Option<&'static Dialog> {
    unsafe {
        if STACK_DEPTH == 0 {
            return None;
        }
        DIALOG_STACK[STACK_DEPTH - 1].as_ref()
    }
}

/// Calculate dialog position (centered on screen)
fn dialog_position(fb: &FramebufferInfo) -> (u32, u32) {
    let x = (fb.width.saturating_sub(DIALOG_W)) / 2;
    let y = (fb.height.saturating_sub(DIALOG_H)) / 2;
    (x, y)
}

/// Calculate file picker position
fn file_picker_position(fb: &FramebufferInfo) -> (u32, u32) {
    let x = (fb.width.saturating_sub(DIALOG_W)) / 2;
    let y = (fb.height.saturating_sub(FILE_PICKER_H)) / 2;
    (x, y)
}

/// Create standard dialog buttons
fn create_buttons(
    labels: &[&str],
    dialog_x: u32,
    dialog_y: u32,
    dialog_w: u32,
) -> ([Option<Button>; 3], usize) {
    let mut buttons: [Option<Button>; 3] = [None, None, None];
    let count = labels.len().min(3);

    // Buttons are right-aligned at the bottom
    let total_btn_w = count as u32 * BUTTON_W + (count.saturating_sub(1) as u32) * BUTTON_GAP;
    let start_x = dialog_x + dialog_w - CONTENT_PAD - total_btn_w;
    let btn_y = dialog_y + dialog_w.saturating_sub(BUTTON_H + CONTENT_PAD); // Approximate, adjusted per kind

    for (i, label) in labels.iter().enumerate().take(3) {
        let mut btn = Button::ZERO;
        btn.x = start_x + i as u32 * (BUTTON_W + BUTTON_GAP);
        btn.y = btn_y; // Will be adjusted by each dialog kind
        btn.w = BUTTON_W;
        btn.h = BUTTON_H;
        btn.set_text(label);
        btn.state = if i == 0 {
            WidgetState::Focused
        } else {
            WidgetState::Normal
        };
        buttons[i] = Some(btn);
    }

    (buttons, count)
}

/// Show an alert dialog
pub fn alert(title: &str, message: &str) {
    let fb = match crate::wm::compositor::fb_info() {
        Some(fb) => fb,
        None => return,
    };
    let (dx, dy) = dialog_position(&fb);

    let (mut buttons, count) = create_buttons(&["OK"], dx, dy, DIALOG_W);
    // Adjust button Y position
    if let Some(btn) = &mut buttons[0] {
        btn.y = dy + DIALOG_H - BUTTON_H - CONTENT_PAD;
    }

    let dialog = Dialog {
        kind: DialogKind::Alert {
            title: String::from(title),
            message: String::from(message),
            ok_label: String::from("OK"),
        },
        buttons,
        button_count: count,
        text_input: None,
        focused_btn: 0,
        file_entries: Vec::new(),
        file_selected: None,
        file_scroll: 0,
    };
    push_dialog(dialog);
    crate::wm::compositor::mark_full_redraw();
}

/// Show a confirm dialog
pub fn confirm(title: &str, message: &str) {
    let fb = match crate::wm::compositor::fb_info() {
        Some(fb) => fb,
        None => return,
    };
    let (dx, dy) = dialog_position(&fb);

    let (mut buttons, count) = create_buttons(&["OK", "Cancel"], dx, dy, DIALOG_W);
    // Adjust button Y position
    for btn in buttons.iter_mut().flatten() {
        btn.y = dy + DIALOG_H - BUTTON_H - CONTENT_PAD;
    }

    let dialog = Dialog {
        kind: DialogKind::Confirm {
            title: String::from(title),
            message: String::from(message),
            ok_label: String::from("OK"),
            cancel_label: String::from("Cancel"),
        },
        buttons,
        button_count: count,
        text_input: None,
        focused_btn: 0, // Default focus on OK
        file_entries: Vec::new(),
        file_selected: None,
        file_scroll: 0,
    };
    push_dialog(dialog);
    crate::wm::compositor::mark_full_redraw();
}

/// Show a prompt dialog
pub fn prompt(title: &str, message: &str, default: &str) {
    let fb = match crate::wm::compositor::fb_info() {
        Some(fb) => fb,
        None => return,
    };
    let (dx, dy) = dialog_position(&fb);

    let (mut buttons, count) = create_buttons(&["OK", "Cancel"], dx, dy, DIALOG_W);
    for btn in buttons.iter_mut().flatten() {
        btn.y = dy + DIALOG_H - BUTTON_H - CONTENT_PAD;
    }

    // Create text input
    let mut input = TextInput::ZERO;
    input.x = dx + CONTENT_PAD;
    input.y = dy + DIALOG_TITLE_H + 60;
    input.w = DIALOG_W - 2 * CONTENT_PAD;
    input.h = INPUT_H;
    input.set_text(default);
    input.state = WidgetState::Focused;

    let dialog = Dialog {
        kind: DialogKind::Prompt {
            title: String::from(title),
            message: String::from(message),
            default_value: String::from(default),
            ok_label: String::from("OK"),
            cancel_label: String::from("Cancel"),
        },
        buttons,
        button_count: count,
        text_input: Some(input),
        focused_btn: 0,
        file_entries: Vec::new(),
        file_selected: None,
        file_scroll: 0,
    };
    push_dialog(dialog);
    crate::wm::compositor::mark_full_redraw();
}

/// Show a file picker dialog
pub fn file_picker(title: &str, mode: FilePickerMode, path: &str) {
    let fb = match crate::wm::compositor::fb_info() {
        Some(fb) => fb,
        None => return,
    };
    let (dx, dy) = file_picker_position(&fb);

    let ok_label = match mode {
        FilePickerMode::Open => "Open",
        FilePickerMode::Save => "Save",
    };
    let (mut buttons, count) = create_buttons(&[ok_label, "Cancel"], dx, dy, DIALOG_W);
    for btn in buttons.iter_mut().flatten() {
        btn.y = dy + FILE_PICKER_H - BUTTON_H - CONTENT_PAD;
    }

    // Create filename input for save mode
    let text_input = if mode == FilePickerMode::Save {
        let mut input = TextInput::ZERO;
        input.x = dx + CONTENT_PAD;
        input.y = dy + FILE_PICKER_H - BUTTON_H - CONTENT_PAD - INPUT_H - 10;
        input.w = DIALOG_W - 2 * CONTENT_PAD;
        input.h = INPUT_H;
        input.state = WidgetState::Focused;
        Some(input)
    } else {
        None
    };

    let mut dialog = Dialog {
        kind: DialogKind::FilePicker {
            title: String::from(title),
            mode,
            path: String::from(path),
            filename: String::new(),
            filter: String::new(),
        },
        buttons,
        button_count: count,
        text_input,
        focused_btn: 0,
        file_entries: Vec::new(),
        file_selected: None,
        file_scroll: 0,
    };

    // Load directory entries
    load_directory(&mut dialog);

    push_dialog(dialog);
    crate::wm::compositor::mark_full_redraw();
}

/// Load directory entries for file picker
fn load_directory(dialog: &mut Dialog) {
    if let DialogKind::FilePicker { path, .. } = &mut dialog.kind {
        dialog.file_entries.clear();
        dialog.file_selected = None;
        dialog.file_scroll = 0;

        // Add parent directory entry if not at root
        if path != "/" {
            dialog.file_entries.push(FileEntry {
                name: String::from(".."),
                is_dir: true,
                size: 0,
            });
        }

        // Read directory
        if let Ok(entries) = list_dir(path) {
            for entry in entries {
                let name = entry.name_str().to_string();
                if name.is_empty() || name == "." {
                    continue;
                }
                dialog.file_entries.push(FileEntry {
                    name,
                    is_dir: entry.is_dir,
                    size: entry.size,
                });
            }
        }

        // Sort: directories first, then files, alphabetically
        dialog
            .file_entries
            .sort_by(|a, b| match (a.is_dir, b.is_dir) {
                (true, false) => core::cmp::Ordering::Less,
                (false, true) => core::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            });
    }
}

/// Handle a click on the top dialog
/// Returns true if the click was consumed
pub fn handle_click(px: u32, py: u32) -> bool {
    let Some(dialog) = top_dialog_mut() else {
        return false;
    };

    let fb = match crate::wm::compositor::fb_info() {
        Some(fb) => fb,
        None => return false,
    };

    let (dx, dy) = match &dialog.kind {
        DialogKind::FilePicker { .. } => file_picker_position(&fb),
        _ => dialog_position(&fb),
    };

    // Check button clicks
    for (i, btn_opt) in dialog.buttons.iter_mut().enumerate() {
        if let Some(btn) = btn_opt {
            // Button coordinates are absolute (screen space)
            if px >= btn.x && px < btn.x + btn.w && py >= btn.y && py < btn.y + btn.h {
                dialog.focused_btn = i;
                handle_button_action(dialog, i);
                return true;
            }
        }
    }

    // Check text input click (for Prompt and FilePicker Save)
    if let Some(input) = &mut dialog.text_input {
        let input_x = dx + CONTENT_PAD;
        let input_y = match &dialog.kind {
            DialogKind::Prompt { .. } => dy + DIALOG_TITLE_H + 60,
            DialogKind::FilePicker { .. } => {
                dy + FILE_PICKER_H - BUTTON_H - CONTENT_PAD - INPUT_H - 10
            }
            _ => 0,
        };
        let input_w = DIALOG_W - 2 * CONTENT_PAD;
        if px >= input_x && px < input_x + input_w && py >= input_y && py < input_y + INPUT_H {
            input.state = WidgetState::Focused;
            return true;
        } else {
            input.state = WidgetState::Normal;
        }
    }

    // Check file list click (for FilePicker)
    if let DialogKind::FilePicker { .. } = &dialog.kind {
        let _list_x = dx + CONTENT_PAD;
        let list_y = dy + DIALOG_TITLE_H + 10;
        let _list_w = DIALOG_W - 2 * CONTENT_PAD;
        let list_h = FILE_PICKER_H - DIALOG_TITLE_H - BUTTON_H - CONTENT_PAD * 3 - INPUT_H - 20;

        if py >= list_y && py < list_y + list_h {
            let item_idx = ((py - list_y) / FILE_LIST_ITEM_H) as usize + dialog.file_scroll;
            if item_idx < dialog.file_entries.len() {
                dialog.file_selected = Some(item_idx);
                // Double-click or Enter would accept, but for now single click selects
                crate::wm::compositor::mark_full_redraw();
                return true;
            }
        }
    }

    // Click on dialog background (but not on controls) - consume but do nothing
    if px >= dx && px < dx + DIALOG_W && py >= dy && py < dy + get_dialog_height(&dialog.kind) {
        return true;
    }

    false
}

/// Get dialog height for a kind
fn get_dialog_height(kind: &DialogKind) -> u32 {
    match kind {
        DialogKind::FilePicker { .. } => FILE_PICKER_H,
        _ => DIALOG_H,
    }
}

/// Handle button action based on dialog kind and button index
fn handle_button_action(dialog: &mut Dialog, btn_idx: usize) {
    match &dialog.kind {
        DialogKind::Alert { .. } => {
            // Only OK button (index 0)
            set_result(DialogResult::AlertOk);
            pop_dialog();
        }
        DialogKind::Confirm { .. } => {
            // Index 0 = OK, Index 1 = Cancel
            let result = btn_idx == 0;
            set_result(DialogResult::Confirm(result));
            pop_dialog();
        }
        DialogKind::Prompt { .. } => {
            let result = if btn_idx == 0 {
                // OK - get text input value
                dialog.text_input.as_ref().map(|i| i.text_str().to_string())
            } else {
                // Cancel
                None
            };
            set_result(DialogResult::Prompt(result));
            pop_dialog();
        }
        DialogKind::FilePicker { mode, .. } => {
            let result = if btn_idx == 0 {
                // OK - get selected file or typed filename
                if *mode == FilePickerMode::Save {
                    dialog.text_input.as_ref().and_then(|i| {
                        let filename = i.text_str().trim();
                        if filename.is_empty() {
                            return None;
                        }
                        let path = &dialog.kind;
                        if let DialogKind::FilePicker { path, .. } = path {
                            Some(alloc::format!(
                                "{}/{}",
                                path.trim_end_matches('/'),
                                filename
                            ))
                        } else {
                            Some(filename.to_string())
                        }
                    })
                } else {
                    // Open mode - get selected entry
                    dialog.file_selected.and_then(|idx| {
                        if idx < dialog.file_entries.len() {
                            let entry = &dialog.file_entries[idx];
                            if entry.is_dir {
                                // Navigate into directory
                                if let DialogKind::FilePicker { path, .. } = &mut dialog.kind {
                                    let new_path = if entry.name == ".." {
                                        // Go up one level
                                        path.rsplit_once('/')
                                            .map(|(p, _)| p.to_string())
                                            .unwrap_or(String::from("/"))
                                    } else {
                                        alloc::format!(
                                            "{}/{}",
                                            path.trim_end_matches('/'),
                                            entry.name
                                        )
                                    };
                                    *path = new_path;
                                    load_directory(dialog);
                                }
                                None // Don't close, just navigate
                            } else {
                                // Return file path
                                if let DialogKind::FilePicker { path, .. } = &dialog.kind {
                                    Some(alloc::format!(
                                        "{}/{}",
                                        path.trim_end_matches('/'),
                                        entry.name
                                    ))
                                } else {
                                    Some(entry.name.clone())
                                }
                            }
                        } else {
                            None
                        }
                    })
                }
            } else {
                // Cancel
                None
            };

            if result.is_some() || btn_idx == 1 {
                set_result(DialogResult::FilePicker(result));
                pop_dialog();
            }
        }
    }
    crate::wm::compositor::mark_full_redraw();
}

/// Set the pending result
fn set_result(result: DialogResult) {
    unsafe {
        PENDING_RESULT = Some(result);
    }
}

/// Take the pending result (consumes it)
pub fn take_result() -> Option<DialogResult> {
    unsafe { PENDING_RESULT.take() }
}

/// Handle a key press on the top dialog
/// Returns true if the key was consumed
pub fn handle_key(scancode: u8) -> bool {
    let Some(dialog) = top_dialog_mut() else {
        return false;
    };

    // Enter = accept default button, Escape = cancel
    match scancode {
        0x1C => {
            // Enter
            handle_button_action(dialog, dialog.focused_btn);
            return true;
        }
        0x01 => {
            // Escape
            handle_escape(dialog);
            return true;
        }
        0x0F => {
            // Tab
            cycle_focus(dialog);
            crate::wm::compositor::mark_full_redraw();
            return true;
        }
        _ => {}
    }

    // Handle text input keys
    if let Some(input) = &mut dialog.text_input {
        if input.state == WidgetState::Focused {
            // Route printable keys to text input
            let shift = crate::usb::hid::shift_held();
            if let Some(ch) = crate::usb::hid::scancode_to_ascii(scancode, shift) {
                if ch.is_ascii_graphic() || ch == ' ' {
                    if input.text_len < 127 {
                        input.insert_char(ch as u8);
                        crate::wm::compositor::mark_full_redraw();
                        return true;
                    }
                }
            }
            // Backspace
            if scancode == 0x0E {
                input.backspace();
                crate::wm::compositor::mark_full_redraw();
                return true;
            }
        }
    }

    // Handle file picker navigation
    if let DialogKind::FilePicker { .. } = &dialog.kind {
        match scancode {
            0x48 => {
                // Up arrow
                if let Some(sel) = dialog.file_selected {
                    if sel > 0 {
                        dialog.file_selected = Some(sel - 1);
                        if dialog.file_selected.unwrap() < dialog.file_scroll {
                            dialog.file_scroll = dialog.file_selected.unwrap();
                        }
                        crate::wm::compositor::mark_full_redraw();
                    }
                } else if !dialog.file_entries.is_empty() {
                    dialog.file_selected = Some(0);
                    crate::wm::compositor::mark_full_redraw();
                }
                return true;
            }
            0x50 => {
                // Down arrow
                if let Some(sel) = dialog.file_selected {
                    if sel + 1 < dialog.file_entries.len() {
                        dialog.file_selected = Some(sel + 1);
                        if dialog.file_selected.unwrap() >= dialog.file_scroll + FILE_LIST_VISIBLE {
                            dialog.file_scroll =
                                dialog.file_selected.unwrap() - FILE_LIST_VISIBLE + 1;
                        }
                        crate::wm::compositor::mark_full_redraw();
                    }
                } else if !dialog.file_entries.is_empty() {
                    dialog.file_selected = Some(0);
                    crate::wm::compositor::mark_full_redraw();
                }
                return true;
            }
            0x1C => {
                // Enter on selected file
                if let Some(sel) = dialog.file_selected {
                    if sel < dialog.file_entries.len() {
                        let entry = &dialog.file_entries[sel];
                        if entry.is_dir {
                            if let DialogKind::FilePicker { path, .. } = &mut dialog.kind {
                                let new_path = if entry.name == ".." {
                                    path.rsplit_once('/')
                                        .map(|(p, _)| p.to_string())
                                        .unwrap_or(String::from("/"))
                                } else {
                                    alloc::format!("{}/{}", path.trim_end_matches('/'), entry.name)
                                };
                                *path = new_path;
                                load_directory(dialog);
                                crate::wm::compositor::mark_full_redraw();
                            }
                        } else {
                            // Accept file
                            handle_button_action(dialog, 0);
                        }
                    }
                }
                return true;
            }
            _ => {}
        }
    }

    false
}

/// Handle Escape key
fn handle_escape(dialog: &mut Dialog) {
    match &dialog.kind {
        DialogKind::Alert { .. } => {
            set_result(DialogResult::AlertOk);
        }
        DialogKind::Confirm { .. } => {
            set_result(DialogResult::Confirm(false));
        }
        DialogKind::Prompt { .. } => {
            set_result(DialogResult::Prompt(None));
        }
        DialogKind::FilePicker { .. } => {
            set_result(DialogResult::FilePicker(None));
        }
    }
    pop_dialog();
    crate::wm::compositor::mark_full_redraw();
}

/// Cycle focus between buttons and text input
fn cycle_focus(dialog: &mut Dialog) {
    let total_focusable = dialog.button_count + if dialog.text_input.is_some() { 1 } else { 0 };
    if total_focusable == 0 {
        return;
    }

    dialog.focused_btn = (dialog.focused_btn + 1) % total_focusable;

    // Update button states
    for (i, btn_opt) in dialog.buttons.iter_mut().enumerate() {
        if let Some(btn) = btn_opt {
            btn.state = if i == dialog.focused_btn && i < dialog.button_count {
                WidgetState::Focused
            } else {
                WidgetState::Normal
            };
        }
    }

    // Update text input state
    if let Some(input) = &mut dialog.text_input {
        let input_focus_idx = dialog.button_count;
        input.state = if dialog.focused_btn == input_focus_idx {
            WidgetState::Focused
        } else {
            WidgetState::Normal
        };
    }
}

/// Draw the top dialog (call from compositor after everything else)
pub fn draw(fb: &FramebufferInfo) {
    let Some(dialog) = top_dialog() else {
        return;
    };

    let theme = theme::current();
    let (dx, dy) = match &dialog.kind {
        DialogKind::FilePicker { .. } => file_picker_position(fb),
        _ => dialog_position(fb),
    };
    let dw = DIALOG_W;
    let dh = get_dialog_height(&dialog.kind);

    // Dim background
    draw_rect(fb, 0, 0, fb.width, fb.height, 0xAA000000);

    // Dialog card background
    fill_rounded_rect(fb, dx, dy, dw, dh, 12, theme.menu_bg);
    draw_rect_outline(fb, dx, dy, dw, dh, theme.menu_border);

    // Title bar
    let title = match &dialog.kind {
        DialogKind::Alert { title, .. } => title,
        DialogKind::Confirm { title, .. } => title,
        DialogKind::Prompt { title, .. } => title,
        DialogKind::FilePicker { title, .. } => title,
    };
    draw_text(
        fb,
        dx + 16,
        dy + (DIALOG_TITLE_H - theme.font_size) / 2,
        title,
        theme.label_text,
        theme.menu_bg,
    );

    // Close button (top-right)
    let close_x = dx + dw - 36;
    let close_y = dy + 4;
    fill_rounded_rect(fb, close_x, close_y, 28, 28, 6, theme.close_btn);
    draw_text(
        fb,
        close_x + 8,
        close_y + 4,
        "x",
        0xFFFFFFFF,
        theme.close_btn,
    );

    // Content area
    let content_y = dy + DIALOG_TITLE_H;

    match &dialog.kind {
        DialogKind::Alert { message, .. } => {
            draw_text(
                fb,
                dx + CONTENT_PAD,
                content_y + 20,
                message,
                theme.label_text,
                theme.menu_bg,
            );
        }
        DialogKind::Confirm { message, .. } => {
            draw_text(
                fb,
                dx + CONTENT_PAD,
                content_y + 20,
                message,
                theme.label_text,
                theme.menu_bg,
            );
        }
        DialogKind::Prompt { message, .. } => {
            draw_text(
                fb,
                dx + CONTENT_PAD,
                content_y + 10,
                message,
                theme.label_text,
                theme.menu_bg,
            );
            // Text input drawn below
        }
        DialogKind::FilePicker { path, .. } => {
            // Path display
            let path_text = alloc::format!("Path: {}", path);
            draw_text(
                fb,
                dx + CONTENT_PAD,
                content_y + 5,
                &path_text,
                theme.label_text_dim,
                theme.menu_bg,
            );

            // File list
            let list_x = dx + CONTENT_PAD;
            let list_y = content_y + 30;
            let list_w = dw - 2 * CONTENT_PAD;
            let list_h = FILE_PICKER_H - DIALOG_TITLE_H - BUTTON_H - CONTENT_PAD * 3 - INPUT_H - 20;

            // List background
            draw_rect(fb, list_x, list_y, list_w, list_h, theme.input_bg);
            draw_rect_outline(fb, list_x, list_y, list_w, list_h, theme.input_border);

            // Visible entries
            let visible =
                FILE_LIST_VISIBLE.min(dialog.file_entries.len().saturating_sub(dialog.file_scroll));
            for i in 0..visible {
                let idx = dialog.file_scroll + i;
                let entry = &dialog.file_entries[idx];
                let item_y = list_y + i as u32 * FILE_LIST_ITEM_H;

                // Selection highlight
                if dialog.file_selected == Some(idx) {
                    draw_rect(
                        fb,
                        list_x + 2,
                        item_y,
                        list_w - 4,
                        FILE_LIST_ITEM_H,
                        theme.menu_item_hover,
                    );
                }

                // Icon + name
                let icon = if entry.is_dir { "📁" } else { "📄" };
                draw_text(
                    fb,
                    list_x + 8,
                    item_y + 4,
                    icon,
                    theme.label_text,
                    theme.menu_bg,
                );
                draw_text(
                    fb,
                    list_x + 28,
                    item_y + 4,
                    &entry.name,
                    theme.label_text,
                    theme.menu_bg,
                );
                if !entry.is_dir && entry.size > 0 {
                    let size_str = format_size(entry.size);
                    draw_text(
                        fb,
                        list_x + list_w - 80,
                        item_y + 4,
                        &size_str,
                        theme.label_text_dim,
                        theme.menu_bg,
                    );
                }
            }

            // Scroll indicator
            if dialog.file_entries.len() > FILE_LIST_VISIBLE {
                let entries_len = dialog.file_entries.len() as u32;
                let visible = FILE_LIST_VISIBLE as u32;
                let scroll_h = list_h * visible / entries_len;
                let scroll_y = list_y + list_h * dialog.file_scroll as u32 / entries_len;
                draw_rect(
                    fb,
                    list_x + list_w - 6,
                    scroll_y,
                    4,
                    scroll_h.max(10),
                    theme.scroll_thumb,
                );
            }
        }
    }

    // Draw text input (for Prompt and FilePicker Save)
    if let Some(input) = &dialog.text_input {
        let input_x = dx + CONTENT_PAD;
        let input_y = match &dialog.kind {
            DialogKind::Prompt { .. } => content_y + 50,
            DialogKind::FilePicker { .. } => {
                dy + FILE_PICKER_H - BUTTON_H - CONTENT_PAD - INPUT_H - 10
            }
            _ => 0,
        };
        // We need to draw at absolute coordinates, so adjust input's internal coords temporarily
        // Since TextInput::draw uses offset_x/offset_y, we pass (0,0) and input has absolute coords
        // Actually TextInput stores window-local coords. Let's draw manually.
        let theme = theme::current();
        let border = match input.state {
            WidgetState::Focused => theme.input_border_focused,
            _ => theme.input_border,
        };

        draw_rect(fb, input_x, input_y, input.w, input.h, theme.input_bg);
        draw_rect_outline(fb, input_x, input_y, input.w, input.h, border);

        if input.text_len > 0 {
            graphics::draw_text(
                fb,
                input_x + 4,
                input_y + 4,
                input.text_str(),
                theme.input_text,
                theme.input_bg,
            );
        }

        if input.state == WidgetState::Focused {
            let cursor_px = input_x + 4 + (input.cursor_pos as u32 - input.scroll_offset) * 8;
            for dy in 2..(input.h - 2) {
                graphics::draw_pixel(fb, cursor_px, input_y + dy, theme.input_cursor);
                graphics::draw_pixel(fb, cursor_px + 1, input_y + dy, theme.input_cursor);
            }
        }
    }

    // Draw buttons
    for btn_opt in &dialog.buttons {
        if let Some(btn) = btn_opt {
            let theme = theme::current();
            let bg = match btn.state {
                WidgetState::Focused => theme.btn_bg_hover,
                WidgetState::Pressed => theme.btn_bg_pressed,
                WidgetState::Hovered => theme.btn_bg_hover,
                _ => theme.btn_bg,
            };
            draw_rect(fb, btn.x, btn.y, btn.w, btn.h, bg);
            draw_rect_outline(fb, btn.x, btn.y, btn.w, btn.h, theme.btn_border);

            if btn.text_len > 0 {
                let text = btn.text_str();
                let char_w = 8u32;
                let text_w = text.chars().count() as u32 * char_w;
                let text_x = btn.x + (btn.w.saturating_sub(text_w)) / 2;
                let text_y = btn.y + (btn.h.saturating_sub(theme.font_size)) / 2;
                draw_text(fb, text_x, text_y, text, theme.btn_text, bg);
            }
        }
    }
}

/// Format file size for display
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        alloc::format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        alloc::format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        alloc::format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        alloc::format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

// Re-export graphics for the manual text input drawing
use crate::display::graphics;

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn dialog_stack_push_pop_order() {
        let _guard = crate::test_support::serial_guard();
        init();
        assert!(!is_open());
        assert_eq!(depth(), 0);

        // We can't easily test the full dialog without a framebuffer,
        // but we can test the stack logic directly
        // SAFETY: serialized by test_support::serial_guard
        unsafe {
            STACK_DEPTH = 0;
            for i in 0..MAX_DIALOG_DEPTH {
                DIALOG_STACK[i] = None;
            }
        }

        // Test push/pop manually
        let d1 = Dialog {
            kind: DialogKind::Alert {
                title: "T".into(),
                message: "M".into(),
                ok_label: "OK".into(),
            },
            buttons: [None; 3],
            button_count: 0,
            text_input: None,
            focused_btn: 0,
            file_entries: Vec::new(),
            file_selected: None,
            file_scroll: 0,
        };
        push_dialog(d1);
        assert_eq!(depth(), 1);
        assert!(is_open());

        let d2 = Dialog {
            kind: DialogKind::Confirm {
                title: "T".into(),
                message: "M".into(),
                ok_label: "OK".into(),
                cancel_label: "Cancel".into(),
            },
            buttons: [None; 3],
            button_count: 0,
            text_input: None,
            focused_btn: 0,
            file_entries: Vec::new(),
            file_selected: None,
            file_scroll: 0,
        };
        push_dialog(d2);
        assert_eq!(depth(), 2);

        let popped = pop_dialog();
        assert!(popped.is_some());
        assert_eq!(depth(), 1);

        let popped = pop_dialog();
        assert!(popped.is_some());
        assert_eq!(depth(), 0);
        assert!(!is_open());
    }

    #[test]
    fn dialog_max_depth() {
        let _guard = crate::test_support::serial_guard();
        init();
        // SAFETY: serialized by test_support::serial_guard
        unsafe {
            STACK_DEPTH = 0;
            for i in 0..MAX_DIALOG_DEPTH {
                DIALOG_STACK[i] = None;
            }
        }

        for i in 0..MAX_DIALOG_DEPTH {
            let d = Dialog {
                kind: DialogKind::Alert {
                    title: "T".into(),
                    message: "M".into(),
                    ok_label: "OK".into(),
                },
                buttons: [None; 3],
                button_count: 0,
                text_input: None,
                focused_btn: 0,
                file_entries: Vec::new(),
                file_selected: None,
                file_scroll: 0,
            };
            assert!(push_dialog(d), "push {} should succeed", i);
        }

        // One more should fail
        let d = Dialog {
            kind: DialogKind::Alert {
                title: "T".into(),
                message: "M".into(),
                ok_label: "OK".into(),
            },
            buttons: [None; 3],
            button_count: 0,
            text_input: None,
            focused_btn: 0,
            file_entries: Vec::new(),
            file_selected: None,
            file_scroll: 0,
        };
        assert!(!push_dialog(d), "push beyond max depth should fail");
    }

    #[test]
    fn confirm_result_on_enter_escape() {
        let _guard = crate::test_support::serial_guard();
        // Test the result logic without full UI
        init();
        // SAFETY: serialized by test_support::serial_guard
        unsafe {
            STACK_DEPTH = 0;
            for i in 0..MAX_DIALOG_DEPTH {
                DIALOG_STACK[i] = None;
            }
            PENDING_RESULT = None;
        }

        let mut dialog = Dialog {
            kind: DialogKind::Confirm {
                title: "Test".into(),
                message: "Are you sure?".into(),
                ok_label: "OK".into(),
                cancel_label: "Cancel".into(),
            },
            buttons: [None, None, None],
            button_count: 2,
            text_input: None,
            focused_btn: 0,
            file_entries: Vec::new(),
            file_selected: None,
            file_scroll: 0,
        };

        push_dialog(dialog);

        // Simulate Enter on OK (focused_btn = 0)
        handle_button_action(top_dialog_mut().unwrap(), 0);
        let result = take_result();
        assert_eq!(result, Some(DialogResult::Confirm(true)));
        assert!(!is_open());

        // Push again for Escape test
        let mut dialog2 = Dialog {
            kind: DialogKind::Confirm {
                title: "Test".into(),
                message: "Are you sure?".into(),
                ok_label: "OK".into(),
                cancel_label: "Cancel".into(),
            },
            buttons: [None, None, None],
            button_count: 2,
            text_input: None,
            focused_btn: 0,
            file_entries: Vec::new(),
            file_selected: None,
            file_scroll: 0,
        };
        push_dialog(dialog2);

        // Simulate Escape
        handle_escape(top_dialog_mut().unwrap());
        let result = take_result();
        assert_eq!(result, Some(DialogResult::Confirm(false)));
        assert!(!is_open());
    }

    #[test]
    fn prompt_result_value() {
        let _guard = crate::test_support::serial_guard();
        init();
        // SAFETY: serialized by test_support::serial_guard
        unsafe {
            STACK_DEPTH = 0;
            for i in 0..MAX_DIALOG_DEPTH {
                DIALOG_STACK[i] = None;
            }
            PENDING_RESULT = None;
        }

        let mut input = TextInput::ZERO;
        input.set_text("default");

        let mut dialog = Dialog {
            kind: DialogKind::Prompt {
                title: "Test".into(),
                message: "Enter value:".into(),
                default_value: "default".into(),
                ok_label: "OK".into(),
                cancel_label: "Cancel".into(),
            },
            buttons: [None, None, None],
            button_count: 2,
            text_input: Some(input),
            focused_btn: 0,
            file_entries: Vec::new(),
            file_selected: None,
            file_scroll: 0,
        };
        push_dialog(dialog);

        // Simulate OK with modified input
        if let Some(d) = top_dialog_mut() {
            if let Some(inp) = &mut d.text_input {
                inp.set_text("user input");
            }
            handle_button_action(d, 0);
        }
        let result = take_result();
        assert_eq!(
            result,
            Some(DialogResult::Prompt(Some("user input".into())))
        );
        assert!(!is_open());

        // Test Cancel
        let mut input2 = TextInput::ZERO;
        input2.set_text("default");
        let mut dialog2 = Dialog {
            kind: DialogKind::Prompt {
                title: "Test".into(),
                message: "Enter value:".into(),
                default_value: "default".into(),
                ok_label: "OK".into(),
                cancel_label: "Cancel".into(),
            },
            buttons: [None, None, None],
            button_count: 2,
            text_input: Some(input2),
            focused_btn: 1, // Focus on Cancel
            file_entries: Vec::new(),
            file_selected: None,
            file_scroll: 0,
        };
        push_dialog(dialog2);
        handle_button_action(top_dialog_mut().unwrap(), 1);
        let result = take_result();
        assert_eq!(result, Some(DialogResult::Prompt(None)));
        assert!(!is_open());
    }

    #[test]
    fn alert_rendering_geometry() {
        // Test dialog positioning math
        struct MockFb {
            width: u32,
            height: u32,
        }
        impl MockFb {
            fn as_framebuffer_info(&self) -> FramebufferInfo {
                FramebufferInfo {
                    base_addr: 0,
                    size: (self.width * self.height * 4) as u64,
                    width: self.width,
                    height: self.height,
                    pitch: self.width * 4,
                    bpp: 32,
                    virt_addr: 0,
                    origin_x: 0,
                    origin_y: 0,
                    clip_x0: 0,
                    clip_y0: 0,
                    clip_x1: self.width,
                    clip_y1: self.height,
                }
            }
        }

        let fb = MockFb {
            width: 1920,
            height: 1080,
        }
        .as_framebuffer_info();
        let (dx, dy) = dialog_position(&fb);
        assert_eq!(dx, (1920 - 400) / 2); // 760
        assert_eq!(dy, (1080 - 200) / 2); // 440

        let fb2 = MockFb {
            width: 800,
            height: 600,
        }
        .as_framebuffer_info();
        let (dx2, dy2) = dialog_position(&fb2);
        assert_eq!(dx2, (800 - 400) / 2); // 200
        assert_eq!(dy2, (600 - 200) / 2); // 200
    }

    #[test]
    fn file_picker_listing() {
        // Test file entry sorting
        let mut entries = vec![
            FileEntry {
                name: "z.txt".into(),
                is_dir: false,
                size: 100,
            },
            FileEntry {
                name: "a_dir".into(),
                is_dir: true,
                size: 0,
            },
            FileEntry {
                name: "b.txt".into(),
                is_dir: false,
                size: 200,
            },
            FileEntry {
                name: "a.txt".into(),
                is_dir: false,
                size: 50,
            },
        ];

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => core::cmp::Ordering::Less,
            (false, true) => core::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        assert_eq!(entries[0].name, "a_dir");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "a.txt");
        assert_eq!(entries[2].name, "b.txt");
        assert_eq!(entries[3].name, "z.txt");
    }
}
