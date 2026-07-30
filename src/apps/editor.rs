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

    /// Simple text editor (stub for qemu)
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
        pub fn open(&mut self, _path: &str) -> Result<(), &'static str> {
            Err("Not available in qemu")
        }
        pub fn save(&mut self) -> Result<(), &'static str> {
            Err("Not available in qemu")
        }
        pub fn save_as(&mut self, _path: &str) -> Result<(), &'static str> {
            Err("Not available in qemu")
        }
        pub fn insert_char(&mut self, _c: char) {}
        pub fn backspace(&mut self) {}
        pub fn newline(&mut self) {}
        pub fn move_up(&mut self) {}
        pub fn move_down(&mut self) {}
        pub fn move_left(&mut self) {}
        pub fn move_right(&mut self) {}
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
