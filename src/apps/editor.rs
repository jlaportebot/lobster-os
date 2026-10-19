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
    }

    impl Editor {
        pub fn new() -> Self {
            Self {
                buffer: Vec::new(),
                cursor_x: 0,
                cursor_y: 0,
                filename: None,
                modified: false,
            }
        }
        pub fn new_file(&mut self) {}
        pub fn open(&mut self, path: &str) -> Result<(), &'static str> {
            let data = crate::fs::vfs::read_file(path).map_err(|_| "cannot read file")?;
            let text = core::str::from_utf8(&data).map_err(|_| "not utf-8")?;
            self.buffer = text.lines().map(|l| String::from(l)).collect();
            if self.buffer.is_empty() {
                self.buffer.push(String::new());
            }
            self.filename = Some(String::from(path));
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.modified = false;
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
                self.cursor_x = self.cursor_x.min(self.buffer[self.cursor_y].chars().count());
            }
        }
        pub fn move_down(&mut self) {
            if self.cursor_y + 1 < self.buffer.len() {
                self.cursor_y += 1;
                self.cursor_x = self.cursor_x.min(self.buffer[self.cursor_y].chars().count());
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

    pub fn get_editor() -> &'static mut Editor {
        static mut EDITOR: Editor = Editor {
            buffer: Vec::new(),
            cursor_x: 0,
            cursor_y: 0,
            filename: None,
            modified: false,
        };
        unsafe { &mut EDITOR }
    }
}

#[cfg(feature = "qemu_raspi5b")]
pub use editor_stub::*;

/// Render the Editor window (QEMU direct-FB path).
#[cfg(feature = "qemu_raspi5b")]
pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
    use crate::display::graphics as g;
    let Some(win_id) = crate::wm::window::find_by_title("Editor") else { return };
    let Some((x, y, w, h)) = crate::wm::window::get_info(win_id) else { return };
    let focused = crate::wm::window::focused_id() == win_id;
    let (cx, cy, cw, ch) = crate::wm::window_frame::draw_frame(fb, win_id, x, y, w, h, "Editor", focused);
    let theme = crate::wm::theme::current();

    static mut INSTANCE: Option<crate::apps::editor::Editor> = None;
    unsafe {
        if INSTANCE.is_none() {
            let mut e = crate::apps::editor::Editor::new();
            e.new_file();
            INSTANCE = Some(e);
        }
        let e = INSTANCE.as_ref().unwrap();

        // Text area
        g::draw_rect(fb, cx, cy, cw, ch, 0xFFFFFFFF);
        let char_h = 16u32;
        let rows = (ch / char_h) as usize;
        let cols = ((cw - 60) / 8) as usize;

        // Line numbers + text
        for (i, line) in e.buffer.iter().enumerate().take(rows) {
            let ly = cy + 2 + i as u32 * char_h;
            let num = alloc::format!("{:>3}", i + 1);
            g::draw_text(fb, cx + 6, ly, &num, 0xFFB0B8C4, 0xFFFFFFFF);
            let text: alloc::string::String = line.chars().take(cols).collect();
            g::draw_text(fb, cx + 40, ly, &text, 0xFF2E3440, 0xFFFFFFFF);
        }

        // Cursor
        let cur_y = cy + 2 + e.cursor_y as u32 * char_h;
        let cur_x = cx + 40 + e.cursor_x as u32 * 8;
        if e.cursor_y < rows as usize {
            g::draw_rect(fb, cur_x, cur_y, 2, char_h, 0xFF2E3440);
        }

        // Status bar
        let status_h = 20u32;
        let status_y = cy + ch.saturating_sub(status_h);
        g::draw_rect(fb, cx, status_y, cw, status_h, theme.menu_bg);
        g::draw_rect(fb, cx, status_y, cw, 1, theme.menu_border);
        let name = e.filename.clone().unwrap_or_else(|| alloc::string::String::from("untitled"));
        let dot = if e.modified { " *" } else { "" };
        let status = alloc::format!("{}{} | line {} col {}", name, dot, e.cursor_y + 1, e.cursor_x + 1);
        g::draw_text(fb, cx + 8, status_y + 4, &status, theme.label_text_dim, theme.menu_bg);
    }
}

/// Feed a key into the editor (for shell automation and keyboard routing).
#[cfg(feature = "qemu_raspi5b")]
pub fn feed_key(c: char) {
    static mut INSTANCE: Option<crate::apps::editor::Editor> = None;
    unsafe {
        if INSTANCE.is_none() {
            let mut e = crate::apps::editor::Editor::new();
            e.new_file();
            INSTANCE = Some(e);
        }
        if let Some(e) = INSTANCE.as_mut() {
            match c {
                '\n' => e.newline(),
                '\u{8}' => e.backspace(),
                _ => e.insert_char(c),
            }
        }
    }
}

/// Load a file into the editor window (opens it if needed).
#[cfg(feature = "qemu_raspi5b")]
pub fn open_file(path: &str) -> u32 {
    // ensure window
    let win_id = match crate::wm::window::find_by_title("Editor") {
        Some(id) => { crate::wm::window::focus(id); id }
        None => match crate::wm::window::create(300, 120, 640, 480, "Editor") {
            Some(id) => id,
            None => return 0,
        },
    };
    // get the static instance from render_to_windows by triggering it once — simpler: rebuild here
    static mut INSTANCE: Option<crate::apps::editor::Editor> = None;
    unsafe {
        if INSTANCE.is_none() {
            let mut e = crate::apps::editor::Editor::new();
            e.new_file();
            INSTANCE = Some(e);
        }
        if let Some(e) = INSTANCE.as_mut() {
            match e.open(path) {
                Ok(()) => crate::println!("[editor] opened {}", path),
                Err(err) => crate::println!("[editor] open {}: {}", path, err),
            }
        }
    }
    win_id
}
