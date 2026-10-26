//! Text Editor for LobsterOS

#[cfg(not(feature = "qemu_raspi5b"))]
mod editor_impl {
    use crate::println;
    use crate::wm::widget::TextInput;
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    /// Simple text editor with gap buffer and undo/redo
    pub struct Editor {
        pub buffer: Vec<String>,
        pub cursor_x: usize,
        pub cursor_y: usize,
        pub filename: Option<String>,
        pub modified: bool,
        pub text_input: TextInput,
        /// Undo stack: each entry is (buffer, cursor_x, cursor_y, modified)
        undo_stack: Vec<(Vec<String>, usize, usize, bool)>,
        /// Redo stack
        redo_stack: Vec<(Vec<String>, usize, usize, bool)>,
        /// Max undo history size
        max_undo: usize,
    }

    impl Editor {
        pub fn new() -> Self {
            Self {
                buffer: Vec::new(),
                cursor_x: 0,
                cursor_y: 0,
                filename: None,
                modified: false,
                text_input: TextInput::ZERO,
                undo_stack: Vec::new(),
                redo_stack: Vec::new(),
                max_undo: 100,
            }
        }

        /// Save current state to undo stack
        fn save_undo(&mut self) {
            if self.undo_stack.len() >= self.max_undo {
                self.undo_stack.remove(0);
            }
            self.undo_stack.push((
                self.buffer.clone(),
                self.cursor_x,
                self.cursor_y,
                self.modified,
            ));
            // Clear redo stack on new action
            self.redo_stack.clear();
        }

        /// Create a new empty file
        pub fn new_file(&mut self) {
            self.save_undo();
            self.buffer.clear();
            self.buffer.push(String::new());
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.filename = None;
            self.modified = false;
        }

        /// Open a file
        pub fn open(&mut self, path: &str) -> Result<(), &'static str> {
            use crate::fs::vfs::read_file;
            let content = read_file(path).map_err(|_| "Failed to read file")?;
            let content_str = core::str::from_utf8(&content).map_err(|_| "Invalid UTF-8")?;

            self.save_undo();
            self.buffer.clear();
            for line in content_str.lines() {
                self.buffer.push(line.to_string());
            }

            self.cursor_x = 0;
            self.cursor_y = 0;
            self.filename = Some(path.to_string());
            self.modified = false;
            Ok(())
        }

        /// Save the current file
        pub fn save(&mut self) -> Result<(), &'static str> {
            let path = self.filename.as_ref().ok_or("No filename")?;

            let content = self.buffer.join("\n");
            // Write to file using VFS
            use crate::fs::vfs::{create, open, write};
            let _ = create(path);
            let fd = match open(path, 1) {
                // O_WRONLY
                Ok(fd) => fd,
                Err(_) => return Err("Failed to open file for writing"),
            };
            let _ = write(fd, content.as_bytes());
            self.modified = false;
            Ok(())
        }

        /// Save as a new file
        pub fn save_as(&mut self, path: &str) -> Result<(), &'static str> {
            self.filename = Some(path.to_string());
            self.save()
        }

        /// Undo last change
        pub fn undo(&mut self) -> bool {
            if let Some(state) = self.undo_stack.pop() {
                self.redo_stack.push((
                    self.buffer.clone(),
                    self.cursor_x,
                    self.cursor_y,
                    self.modified,
                ));
                self.buffer = state.0;
                self.cursor_x = state.1;
                self.cursor_y = state.2;
                self.modified = state.3;
                true
            } else {
                false
            }
        }

        /// Redo last undone change
        pub fn redo(&mut self) -> bool {
            if let Some(state) = self.redo_stack.pop() {
                self.undo_stack.push((
                    self.buffer.clone(),
                    self.cursor_x,
                    self.cursor_y,
                    self.modified,
                ));
                self.buffer = state.0;
                self.cursor_x = state.1;
                self.cursor_y = state.2;
                self.modified = state.3;
                true
            } else {
                false
            }
        }

        /// Insert a character at cursor position
        pub fn insert_char(&mut self, c: char) {
            self.save_undo();
            if self.cursor_y >= self.buffer.len() {
                self.buffer.push(String::new());
            }
            self.buffer[self.cursor_y].insert(self.cursor_x, c);
            self.cursor_x += 1;
            self.modified = true;
        }

        /// Delete character before cursor (backspace)
        pub fn backspace(&mut self) {
            self.save_undo();
            if self.cursor_x > 0 {
                self.buffer[self.cursor_y].remove(self.cursor_x - 1);
                self.cursor_x -= 1;
                self.modified = true;
            } else if self.cursor_y > 0 {
                // Join with previous line
                let prev_len = self.buffer[self.cursor_y - 1].len();
                let current_line = self.buffer.remove(self.cursor_y);
                self.buffer[self.cursor_y - 1].push_str(&current_line);
                self.cursor_y -= 1;
                self.cursor_x = prev_len;
                self.modified = true;
            }
        }

        /// Insert newline
        pub fn newline(&mut self) {
            self.save_undo();
            if self.cursor_y >= self.buffer.len() {
                self.buffer.push(String::new());
                self.cursor_y += 1;
                self.cursor_x = 0;
                return;
            }

            let current = &self.buffer[self.cursor_y];
            let rest = current[self.cursor_x..].to_string();
            self.buffer[self.cursor_y].truncate(self.cursor_x);
            self.buffer.insert(self.cursor_y + 1, rest);
            self.cursor_y += 1;
            self.cursor_x = 0;
            self.modified = true;
        }

        /// Move cursor up
        pub fn move_up(&mut self) {
            if self.cursor_y > 0 {
                self.cursor_y -= 1;
                self.cursor_x = self.cursor_x.min(self.buffer[self.cursor_y].len());
            }
        }

        /// Move cursor down
        pub fn move_down(&mut self) {
            if self.cursor_y + 1 < self.buffer.len() {
                self.cursor_y += 1;
                self.cursor_x = self.cursor_x.min(self.buffer[self.cursor_y].len());
            }
        }

        /// Move cursor left
        pub fn move_left(&mut self) {
            if self.cursor_x > 0 {
                self.cursor_x -= 1;
            } else if self.cursor_y > 0 {
                self.cursor_y -= 1;
                self.cursor_x = self.buffer[self.cursor_y].len();
            }
        }

        /// Move cursor right
        pub fn move_right(&mut self) {
            if self.cursor_x < self.buffer[self.cursor_y].len() {
                self.cursor_x += 1;
            } else if self.cursor_y + 1 < self.buffer.len() {
                self.cursor_y += 1;
                self.cursor_x = 0;
            }
        }

        /// Get current line
        pub fn current_line(&self) -> &str {
            if self.cursor_y < self.buffer.len() {
                &self.buffer[self.cursor_y]
            } else {
                ""
            }
        }

        /// Get all text as single string
        pub fn get_text(&self) -> String {
            self.buffer.join("\n")
        }

        /// Simple syntax highlighting for Rust
        pub fn highlight_line(&self, line: &str) -> Vec<(usize, usize, &'static str)> {
            let mut spans = Vec::new();
            let mut i = 0;
            let chars: Vec<char> = line.chars().collect();

            while i < chars.len() {
                let ch = chars[i];

                // Comments
                if ch == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    spans.push((i, line.len(), "comment"));
                    break;
                }

                // Strings
                if ch == '"' {
                    let start = i;
                    i += 1;
                    while i < chars.len() {
                        if chars[i] == '"' && (i == 0 || chars[i - 1] != '\\') {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                    spans.push((start, i, "string"));
                    continue;
                }

                // Characters
                if ch == '\'' {
                    let start = i;
                    i += 1;
                    while i < chars.len() {
                        if chars[i] == '\'' && (i == 0 || chars[i - 1] != '\\') {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                    spans.push((start, i, "char"));
                    continue;
                }

                // Keywords
                if ch.is_alphabetic() || ch == '_' {
                    let start = i;
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    let word = &line[start..i];
                    let color = match word {
                        "fn" | "let" | "mut" | "const" | "if" | "else" | "while" | "for"
                        | "loop" | "match" | "return" | "break" | "continue" | "struct"
                        | "enum" | "impl" | "trait" | "type" | "mod" | "use" | "pub" | "crate"
                        | "super" | "self" | "async" | "await" | "unsafe" | "ref" | "move"
                        | "box" | "dyn" => "keyword",
                        "true" | "false" | "None" | "Some" | "Ok" | "Err" => "literal",
                        _ => "identifier",
                    };
                    spans.push((start, i, color));
                    continue;
                }

                // Numbers
                if ch.is_ascii_digit() {
                    let start = i;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                        i += 1;
                    }
                    spans.push((start, i, "number"));
                    continue;
                }

                // Operators and punctuation
                if "+-*/%=<>!&|^~.,;:()[]{}".contains(ch) {
                    spans.push((i, i + 1, "operator"));
                    i += 1;
                    continue;
                }

                i += 1;
            }

            spans
        }
    }

    impl Default for Editor {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Global editor instance
    static mut EDITOR: Editor = Editor {
        buffer: Vec::new(),
        cursor_x: 0,
        cursor_y: 0,
        filename: None,
        modified: false,
        text_input: crate::wm::widget::TextInput::ZERO,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        max_undo: 100,
    };

    /// Initialize the editor
    pub fn init() {
        unsafe {
            EDITOR = Editor::new();
        }
        println!("[editor] Text editor initialized");
    }

    /// Get the global editor instance
    pub fn get_editor() -> &'static mut Editor {
        unsafe { &mut EDITOR }
    }
}

#[cfg(not(feature = "qemu_raspi5b"))]
pub use editor_impl::*;

#[cfg(feature = "qemu_raspi5b")]
mod editor_stub {
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Real file-backed text editor for QEMU builds: loads/saves through the
    /// VFS (which persists to the ext2 /sd disk), renders with line numbers,
    /// cursor, and a modified-indicator in the status bar.
    pub struct Editor {
        pub buffer: Vec<String>,
        pub cursor_x: usize,
        pub cursor_y: usize,
        pub filename: Option<String>,
        pub modified: bool,
        /// First visible line (render-time vertical scroll)
        pub top: usize,
    }

    impl Editor {
        pub fn new() -> Self {
            Self {
                buffer: Vec::new(),
                cursor_x: 0,
                cursor_y: 0,
                filename: None,
                modified: false,
                top: 0,
            }
        }
        pub fn new_file(&mut self) {
            self.buffer.clear();
            self.buffer.push(String::new());
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.filename = None;
            self.modified = false;
            self.top = 0;
        }
        pub fn open(&mut self, path: &str) -> Result<(), &'static str> {
            let data = crate::fs::vfs::read_file(path).map_err(|_| "cannot read file")?;
            let text = core::str::from_utf8(&data).map_err(|_| "not utf-8")?;
            self.buffer = text.lines().map(String::from).collect();
            if self.buffer.is_empty() {
                self.buffer.push(String::new());
            }
            self.filename = Some(String::from(path));
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.modified = false;
            self.top = 0;
            Ok(())
        }
        pub fn save(&mut self) -> Result<(), &'static str> {
            let path = self.filename.clone().ok_or("no filename")?;
            self.save_as(&path)
        }
        pub fn save_as(&mut self, path: &str) -> Result<(), &'static str> {
            let mut body = String::new();
            for (i, line) in self.buffer.iter().enumerate() {
                body.push_str(line);
                if i + 1 < self.buffer.len() {
                    body.push('\n');
                }
            }
            crate::fs::vfs::write_file(path, body.as_bytes()).map_err(|_| "write failed")?;
            self.filename = Some(String::from(path));
            self.modified = false;
            crate::wm::notifications::success("Editor", &alloc::format!("Saved {}", path));
            Ok(())
        }
        pub fn insert_char(&mut self, c: char) {
            if self.cursor_y >= self.buffer.len() {
                self.buffer.push(String::new());
            }
            let line = &mut self.buffer[self.cursor_y];
            let byte_idx = line
                .char_indices()
                .nth(self.cursor_x)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            line.insert(byte_idx, c);
            self.cursor_x += 1;
            self.modified = true;
        }
        pub fn backspace(&mut self) {
            if self.cursor_y < self.buffer.len() {
                let line = &mut self.buffer[self.cursor_y];
                if self.cursor_x > 0 {
                    let byte_idx = line
                        .char_indices()
                        .nth(self.cursor_x - 1)
                        .map(|(i, _)| i)
                        .unwrap_or(line.len());
                    let end = line
                        .char_indices()
                        .nth(self.cursor_x)
                        .map(|(i, _)| i)
                        .unwrap_or(line.len());
                    line.replace_range(byte_idx..end, "");
                    self.cursor_x -= 1;
                } else if self.cursor_y > 0 {
                    let line = self.buffer.remove(self.cursor_y);
                    self.cursor_y -= 1;
                    self.cursor_x = self.buffer[self.cursor_y].chars().count();
                    self.buffer[self.cursor_y].push_str(&line);
                }
                self.modified = true;
            }
        }
        pub fn newline(&mut self) {
            if self.cursor_y >= self.buffer.len() {
                self.buffer.push(String::new());
                self.cursor_y = self.buffer.len() - 1;
            }
            let line = self.buffer.remove(self.cursor_y);
            let byte_idx = line
                .char_indices()
                .nth(self.cursor_x)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            let tail = String::from(&line[byte_idx..]);
            let head = String::from(&line[..byte_idx]);
            self.buffer.insert(self.cursor_y, head);
            self.buffer.insert(self.cursor_y + 1, tail);
            self.cursor_y += 1;
            self.cursor_x = 0;
            self.modified = true;
        }
        pub fn move_up(&mut self) {
            if self.cursor_y > 0 {
                self.cursor_y -= 1;
                self.cursor_x = self
                    .cursor_x
                    .min(self.buffer[self.cursor_y].chars().count());
            }
        }
        pub fn move_down(&mut self) {
            if self.cursor_y + 1 < self.buffer.len() {
                self.cursor_y += 1;
                self.cursor_x = self
                    .cursor_x
                    .min(self.buffer[self.cursor_y].chars().count());
            }
        }
        pub fn move_left(&mut self) {
            if self.cursor_x > 0 {
                self.cursor_x -= 1;
            } else if self.cursor_y > 0 {
                self.cursor_y -= 1;
                self.cursor_x = self.buffer[self.cursor_y].chars().count();
            }
        }
        pub fn move_right(&mut self) {
            if self.cursor_y < self.buffer.len() {
                let len = self.buffer[self.cursor_y].chars().count();
                if self.cursor_x < len {
                    self.cursor_x += 1;
                } else if self.cursor_y + 1 < self.buffer.len() {
                    self.cursor_y += 1;
                    self.cursor_x = 0;
                }
            }
        }
        pub fn current_line(&self) -> &str {
            ""
        }
        pub fn get_text(&self) -> String {
            String::new()
        }
    }

    impl Default for Editor {
        fn default() -> Self {
            Self::new()
        }
    }

    pub fn init() {
        crate::println!("[editor] Text editor initialized (qemu stub)");
    }

    /// The active buffer of the tabbed editor.
    pub fn get_editor() -> &'static mut Editor {
        super::tabs::active_buffer()
    }
}

#[cfg(feature = "qemu_raspi5b")]
pub use editor_stub::*;

/// Multi-buffer tabbed editor (QEMU direct-FB path): up to MAX_BUFS buffers,
/// a tab strip, inline save-as / find prompt, and keyboard + click routing.
#[cfg(feature = "qemu_raspi5b")]
mod tabs {
    use super::editor_stub::Editor;
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Maximum number of open buffers
    pub const MAX_BUFS: usize = 4;
    /// Height of the tab strip at the top of the client area
    const TAB_H: u32 = 22;
    const TAB_W: u32 = 128;
    const TAB_GAP: u32 = 2;
    const PLUS_W: u32 = 24;
    /// Height of the status bar and of the inline prompt row
    const ROW_H: u32 = 20;
    const CHAR_H: u32 = 16;
    const GUTTER: u32 = 40;
    const FRAME_BORDER: u32 = 2;
    const FRAME_TITLE: u32 = 24;
    const APP_NAME: &str = "Editor";
    const MAX_PROMPT: usize = 120;

    // Set-1 scancodes (same set as gui_shell / task_manager)
    const SC_ESC: u8 = 0x01;
    const SC_1: u8 = 0x02;
    const SC_4: u8 = 0x05;
    const SC_BACKSPACE: u8 = 0x0E;
    const SC_TAB: u8 = 0x0F;
    const SC_F: u8 = 0x21;
    const SC_S: u8 = 0x1F;
    const SC_A: u8 = 0x1E;
    const SC_N: u8 = 0x31;
    const SC_ENTER: u8 = 0x1C;
    const SC_F3: u8 = 0x3D;
    const SC_UP: u8 = 0x48;
    const SC_DOWN: u8 = 0x50;
    const SC_LEFT: u8 = 0x4B;
    const SC_RIGHT: u8 = 0x4D;

    #[derive(Clone, Copy, PartialEq)]
    enum PromptKind {
        SaveAs,
        Find,
    }

    struct Prompt {
        kind: PromptKind,
        text: String,
    }

    struct App {
        bufs: Vec<Editor>,
        active: usize,
        /// Some(..) = prompt mode: keys are redirected to the inline prompt
        prompt: Option<Prompt>,
        last_find: String,
        /// Highlighted find match in the active buffer: (line, char col, char len)
        hl: Option<(usize, usize, usize)>,
        /// One-shot status message (cleared on the next key)
        msg: Option<String>,
    }

    static mut APP: App = App {
        bufs: Vec::new(),
        active: 0,
        prompt: None,
        last_find: String::new(),
        hl: None,
        msg: None,
    };

    fn new_buffer() -> Editor {
        let mut e = Editor::new();
        e.new_file();
        e
    }

    fn app() -> &'static mut App {
        unsafe {
            let a = &mut *core::ptr::addr_of_mut!(APP);
            if a.bufs.is_empty() {
                a.bufs.push(new_buffer());
                a.active = 0;
            }
            a
        }
    }

    /// The active buffer (used by `Editor` consumers that want "the" editor).
    pub fn active_buffer() -> &'static mut Editor {
        let a = app();
        let i = a.active;
        &mut a.bufs[i]
    }

    fn base_name(e: &Editor) -> String {
        match &e.filename {
            Some(p) => String::from(p.rsplit('/').next().unwrap_or(p.as_str())),
            None => String::from("untitled"),
        }
    }

    /// x offset (from the client's left edge) of tab `i`; `i == len` is '+'.
    fn tab_offset(i: usize) -> u32 {
        4 + i as u32 * (TAB_W + TAB_GAP)
    }

    impl App {
        fn switch(&mut self, idx: usize) {
            if idx >= self.bufs.len() {
                return;
            }
            self.active = idx;
            self.prompt = None;
            self.hl = None;
            self.msg = None;
        }

        fn add(&mut self) -> bool {
            if self.bufs.len() >= MAX_BUFS {
                self.msg = Some(String::from("Maximum of 4 buffers open"));
                return false;
            }
            self.bufs.push(new_buffer());
            let idx = self.bufs.len() - 1;
            self.switch(idx);
            true
        }

        fn open_prompt(&mut self, kind: PromptKind) {
            let text = match kind {
                PromptKind::SaveAs => self.bufs[self.active].filename.clone().unwrap_or_default(),
                PromptKind::Find => self.last_find.clone(),
            };
            self.prompt = Some(Prompt { kind, text });
            self.msg = None;
        }

        fn save_active(&mut self, path: Option<&str>) {
            let e = &mut self.bufs[self.active];
            let r = match path {
                Some(p) => e.save_as(p),
                None => e.save(),
            };
            if let Err(err) = r {
                self.msg = Some(alloc::format!("Save failed: {}", err));
            }
        }

        /// Plain substring search from the cursor (wrapping). `skip_current`
        /// starts one char after the cursor (used when repeating).
        fn find(&mut self, query: &str, skip_current: bool) -> bool {
            if query.is_empty() {
                return false;
            }
            let e = &mut self.bufs[self.active];
            let n = e.buffer.len();
            if n == 0 {
                return false;
            }
            let qlen = query.chars().count();
            let mut hit: Option<(usize, usize)> = None;
            for step in 0..=n {
                let y = (e.cursor_y + step) % n;
                let line = &e.buffer[y];
                let start_col = if step == 0 {
                    e.cursor_x + skip_current as usize
                } else {
                    0
                };
                let Some((start_byte, _)) = line.char_indices().nth(start_col) else {
                    continue;
                };
                if let Some(rel) = line[start_byte..].find(query) {
                    hit = Some((y, line[..start_byte + rel].chars().count()));
                    break;
                }
            }
            match hit {
                Some((y, col)) => {
                    e.cursor_y = y;
                    e.cursor_x = col;
                    self.hl = Some((y, col, qlen));
                    self.msg = None;
                    true
                }
                None => {
                    self.hl = None;
                    self.msg = Some(alloc::format!("Not found: {}", query));
                    false
                }
            }
        }

        /// F3 / Enter-in-find: jump to the next match of the last query.
        fn find_next(&mut self) {
            if self.last_find.is_empty() {
                self.open_prompt(PromptKind::Find);
                return;
            }
            let q = self.last_find.clone();
            let repeat = self.hl.is_some();
            self.find(&q, repeat);
        }

        fn prompt_key(&mut self, scancode: u8, shift: bool) {
            match scancode {
                SC_ESC => self.prompt = None,
                SC_ENTER => {
                    let Some(p) = self.prompt.take() else { return };
                    match p.kind {
                        PromptKind::SaveAs => {
                            if !p.text.is_empty() {
                                self.save_active(Some(&p.text));
                            }
                        }
                        PromptKind::Find => {
                            if !p.text.is_empty() {
                                let repeat = self.hl.is_some() && p.text == self.last_find;
                                self.last_find = p.text.clone();
                                self.find(&p.text, repeat);
                            }
                            // Stay in find mode so Enter repeats the search
                            self.prompt = Some(p);
                        }
                    }
                }
                SC_BACKSPACE => {
                    if let Some(p) = self.prompt.as_mut() {
                        p.text.pop();
                    }
                }
                _ => {
                    if let Some(c) = crate::usb::hid::scancode_to_ascii(scancode, shift) {
                        if c >= ' ' && c <= '~' {
                            if let Some(p) = self.prompt.as_mut() {
                                if p.text.len() < MAX_PROMPT {
                                    p.text.push(c);
                                }
                            }
                        }
                    }
                }
            }
        }

        fn edit_key(&mut self, scancode: u8, shift: bool) -> bool {
            let e = &mut self.bufs[self.active];
            match scancode {
                SC_ENTER => e.newline(),
                SC_BACKSPACE => e.backspace(),
                SC_TAB => {
                    for _ in 0..4 {
                        e.insert_char(' ');
                    }
                }
                SC_UP => e.move_up(),
                SC_DOWN => e.move_down(),
                SC_LEFT => e.move_left(),
                SC_RIGHT => e.move_right(),
                _ => match crate::usb::hid::scancode_to_ascii(scancode, shift) {
                    Some(c) if c >= ' ' && c <= '~' => e.insert_char(c),
                    _ => return false,
                },
            }
            // Edits/cursor moves invalidate the find highlight
            self.hl = None;
            true
        }
    }

    /// Keyboard routing for the focused Editor window. Returns true if consumed.
    pub fn handle_key(scancode: u8) -> bool {
        use crate::usb::hid;
        if crate::wm::window::get_title(crate::wm::window::focused_id()) != APP_NAME {
            return false;
        }
        let shift = hid::shift_held();
        let ctrl = hid::ctrl_held();
        let alt = hid::alt_held();
        let a = app();
        a.msg = None;

        let consumed = if a.prompt.is_some() {
            // Prompt mode: every key goes to the inline prompt
            a.prompt_key(scancode, shift);
            true
        } else if scancode == SC_F3 {
            a.find_next();
            true
        } else if alt {
            match scancode {
                SC_1..=SC_4 => {
                    a.switch((scancode - SC_1) as usize);
                    true
                }
                _ => false, // Alt+Tab / Alt+F4 etc. stay with the WM
            }
        } else if ctrl {
            match scancode {
                SC_N => {
                    a.add();
                }
                SC_A => a.open_prompt(PromptKind::SaveAs),
                SC_F => a.open_prompt(PromptKind::Find),
                SC_S => {
                    if a.bufs[a.active].filename.is_some() {
                        a.save_active(None);
                    } else {
                        a.open_prompt(PromptKind::SaveAs);
                    }
                }
                _ => {}
            }
            true
        } else {
            a.edit_key(scancode, shift)
        };
        if consumed {
            crate::wm::dirty::mark();
        }
        consumed
    }

    /// Tab-strip click handling (screen coords). Returns true if consumed.
    pub fn handle_click(cx: u32, cy: u32) -> bool {
        use crate::wm::window;
        let Some(win_id) = window::find_by_title(APP_NAME) else {
            return false;
        };
        let Some((x, y, w, _h)) = window::get_info(win_id) else {
            return false;
        };
        let client_x = x + FRAME_BORDER;
        let client_y = y + FRAME_BORDER + FRAME_TITLE;
        let client_w = w.saturating_sub(2 * FRAME_BORDER);
        if cx < client_x || cx >= client_x + client_w || cy < client_y || cy >= client_y + TAB_H {
            return false;
        }
        let a = app();
        let rel_x = cx - client_x;
        let n = a.bufs.len();
        let slots = if n < MAX_BUFS { n + 1 } else { n };
        for i in 0..slots {
            let bx = tab_offset(i);
            let bw = if i < n { TAB_W } else { PLUS_W };
            if rel_x >= bx && rel_x < bx + bw && cy >= client_y + 2 && cy < client_y + TAB_H - 2 {
                window::focus(win_id);
                if i < n {
                    a.switch(i);
                } else {
                    a.add();
                }
                crate::wm::dirty::mark();
                return true;
            }
        }
        false
    }

    /// Feed a character into the active buffer (shell automation).
    pub fn feed_key(c: char) {
        let a = app();
        let e = &mut a.bufs[a.active];
        match c {
            '\n' => e.newline(),
            '\u{8}' => e.backspace(),
            _ => e.insert_char(c),
        }
        a.hl = None;
    }

    /// Open `path` in a buffer (reusing an existing tab for the same file,
    /// or a pristine untitled one) and make sure the Editor window exists.
    pub fn open_file(path: &str) -> u32 {
        let win_id = match crate::wm::window::find_by_title(APP_NAME) {
            Some(id) => {
                crate::wm::window::focus(id);
                id
            }
            None => match crate::wm::window::create(300, 120, 640, 480, APP_NAME) {
                Some(id) => id,
                None => return 0,
            },
        };
        let a = app();
        if let Some(i) = a
            .bufs
            .iter()
            .position(|b| b.filename.as_deref() == Some(path))
        {
            a.switch(i);
            return win_id;
        }
        let cur = &a.bufs[a.active];
        let pristine = cur.filename.is_none()
            && !cur.modified
            && cur.buffer.len() <= 1
            && cur.buffer.first().map_or(true, |l| l.is_empty());
        let (idx, pushed) = if pristine {
            (a.active, false)
        } else if a.bufs.len() < MAX_BUFS {
            a.bufs.push(new_buffer());
            (a.bufs.len() - 1, true)
        } else {
            crate::println!("[editor] open {}: all {} buffers in use", path, MAX_BUFS);
            return win_id;
        };
        match a.bufs[idx].open(path) {
            Ok(()) => {
                a.switch(idx);
                crate::println!("[editor] opened {}", path);
            }
            Err(err) => {
                if pushed {
                    a.bufs.pop();
                }
                crate::println!("[editor] open {}: {}", path, err);
            }
        }
        win_id
    }

    /// Render the Editor window (QEMU direct-FB path).
    #[cfg(feature = "qemu_raspi5b")]
    pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
        use crate::display::graphics as g;
        let Some(win_id) = crate::wm::window::find_by_title(APP_NAME) else {
            return;
        };
        let Some((x, y, w, h)) = crate::wm::window::get_info(win_id) else {
            return;
        };
        let focused = crate::wm::window::focused_id() == win_id;
        let a = app();

        let title = if a.bufs[a.active].modified {
            "Editor *"
        } else {
            APP_NAME
        };
        let (cx, cy, cw, ch) =
            crate::wm::window_frame::draw_frame(fb, win_id, x, y, w, h, title, focused);
        if cw < GUTTER + 16 || ch < TAB_H + ROW_H + CHAR_H {
            return;
        }
        let theme = crate::wm::theme::current();

        // Client background
        g::draw_rect(fb, cx, cy, cw, ch, 0xFFFFFFFF);

        // ── Tab strip: one tab per buffer (+ '+' while under MAX_BUFS) ──
        g::draw_rect(fb, cx, cy, cw, TAB_H, theme.title_bar_unfocused);
        let n = a.bufs.len();
        let slots = if n < MAX_BUFS { n + 1 } else { n };
        for i in 0..slots {
            let bx = cx + tab_offset(i);
            let by = cy + 2;
            let bw = if i < n { TAB_W } else { PLUS_W };
            if bx + bw > cx + cw {
                break;
            }
            let active = i == a.active && i < n;
            let bg = if active {
                theme.win_border
            } else {
                theme.btn_bg
            };
            let fg = if active { 0xFFFFFFFF } else { theme.btn_text };
            g::draw_rect(fb, bx, by, bw, TAB_H - 4, bg);
            let label = if i < n {
                let e = &a.bufs[i];
                let l = alloc::format!(
                    "{} {}{}",
                    i + 1,
                    base_name(e),
                    if e.modified { "*" } else { "" }
                );
                let max = ((TAB_W - 8) / 8) as usize;
                let mut s: String = l.chars().take(max).collect();
                // keep the unsaved marker visible when the name is truncated
                if e.modified && l.chars().count() > max {
                    s.pop();
                    s.push('*');
                }
                s
            } else {
                String::from("+")
            };
            let tx = if i < n { bx + 4 } else { bx + (PLUS_W - 8) / 2 };
            g::draw_text(fb, tx, by + (TAB_H - 4 - CHAR_H) / 2, &label, fg, bg);
        }

        // ── Layout: text area / [prompt row] / status bar ──
        let prompt_h = if a.prompt.is_some() { ROW_H } else { 0 };
        let bottom_h = ROW_H + prompt_h;
        let text_y = cy + TAB_H;
        let text_h = ch.saturating_sub(TAB_H + bottom_h);
        let rows = ((text_h.saturating_sub(2)) / CHAR_H).max(1) as usize;
        let cols = ((cw - GUTTER - 4) / 8).max(1) as usize;

        let hl = a.hl;
        let prompt_open = a.prompt.is_some();
        let e = &mut a.bufs[a.active];

        // Keep the cursor visible
        if e.cursor_y < e.top {
            e.top = e.cursor_y;
        } else if e.cursor_y >= e.top + rows {
            e.top = e.cursor_y + 1 - rows;
        }
        let left = if e.cursor_x >= cols {
            e.cursor_x - cols / 2
        } else {
            0
        };

        for r in 0..rows {
            let i = e.top + r;
            let Some(line) = e.buffer.get(i) else { break };
            let ly = text_y + 2 + r as u32 * CHAR_H;
            let num = alloc::format!("{:>3}", i + 1);
            g::draw_text(fb, cx + 6, ly, &num, 0xFFB0B8C4, 0xFFFFFFFF);
            let text: String = line.chars().skip(left).take(cols).collect();
            g::draw_text(fb, cx + GUTTER, ly, &text, 0xFF2E3440, 0xFFFFFFFF);

            // Find match: inverted colors
            if let Some((hy, hc, hlen)) = hl {
                if hy == i && hc + hlen > left {
                    let from = hc.saturating_sub(left);
                    let skip_in_match = left.saturating_sub(hc);
                    let take = (hlen - skip_in_match).min(cols.saturating_sub(from));
                    let m: String = line.chars().skip(hc + skip_in_match).take(take).collect();
                    g::draw_text(
                        fb,
                        cx + GUTTER + from as u32 * 8,
                        ly,
                        &m,
                        0xFFFFFFFF,
                        0xFF2E3440,
                    );
                }
            }
        }

        // Text cursor (hidden while the prompt owns the keyboard)
        if !prompt_open && e.cursor_y >= e.top && e.cursor_y < e.top + rows {
            let cur_y = text_y + 2 + (e.cursor_y - e.top) as u32 * CHAR_H;
            let cur_x = cx + GUTTER + (e.cursor_x - left) as u32 * 8;
            g::draw_rect(fb, cur_x, cur_y, 2, CHAR_H, 0xFF2E3440);
        }

        // ── Inline prompt row ──
        let max_chars = (cw.saturating_sub(16) / 8) as usize;
        if let Some(p) = a.prompt.as_ref() {
            let py = cy + ch - bottom_h;
            g::draw_rect(fb, cx, py, cw, ROW_H, theme.title_bar_unfocused);
            g::draw_rect(fb, cx, py, cw, 1, theme.menu_border);
            let label = match p.kind {
                PromptKind::SaveAs => "save as: ",
                PromptKind::Find => "find: ",
            };
            let avail = max_chars.saturating_sub(label.len() + 1);
            let tlen = p.text.chars().count();
            let shown: String = p.text.chars().skip(tlen.saturating_sub(avail)).collect();
            let line = alloc::format!("{}{}_", label, shown);
            g::draw_text(
                fb,
                cx + 8,
                py + 2,
                &line,
                0xFFFFFFFF,
                theme.title_bar_unfocused,
            );
        }

        // ── Status bar ──
        let status_y = cy + ch - ROW_H;
        g::draw_rect(fb, cx, status_y, cw, ROW_H, theme.menu_bg);
        g::draw_rect(fb, cx, status_y, cw, 1, theme.menu_border);
        let name = e
            .filename
            .clone()
            .unwrap_or_else(|| String::from("untitled"));
        let dot = if e.modified { " *" } else { "" };
        let status = match a.msg.as_ref() {
            Some(m) => alloc::format!("{}{} | {}", name, dot, m),
            None => alloc::format!(
                "{}{} | line {} col {} | ^N new ^S save ^A save-as ^F find",
                name,
                dot,
                e.cursor_y + 1,
                e.cursor_x + 1
            ),
        };
        let status: String = status.chars().take(max_chars).collect();
        g::draw_text(
            fb,
            cx + 8,
            status_y + 4,
            &status,
            theme.label_text_dim,
            theme.menu_bg,
        );
    }
}

#[cfg(feature = "qemu_raspi5b")]
pub use tabs::{feed_key, handle_click, handle_key, open_file, render_to_windows, MAX_BUFS};

/// Non-QEMU builds have no tab strip; clicks are never consumed.
#[cfg(not(feature = "qemu_raspi5b"))]
pub fn handle_click(_cx: u32, _cy: u32) -> bool {
    false
}
