//! Notes — persistent sticky notes
//!
//! Each note is a text file under /sd/home/user/notes/. The window shows a
//! note list on the left and the selected note's content on the right;
//! typing edits, and content auto-saves to disk on change (write-through).

use crate::display::fb::get_info;
use crate::display::graphics as g;
use crate::wm::theme;
use crate::wm::window;

const NOTES_DIR: &str = "/sd/home/user/notes";
const MAX_NOTES: usize = 16;
const TITLE_MAX: usize = 24;

pub struct Notes {
    titles: [alloc::string::String; MAX_NOTES],
    count: usize,
    selected: usize,
    /// In-memory content of the selected note
    content: alloc::string::String,
    loaded: bool,
}

impl Notes {
    pub fn new() -> Self {
        Self {
            titles: core::array::from_fn(|_| alloc::string::String::new()),
            count: 0,
            selected: 0,
            content: alloc::string::String::new(),
            loaded: false,
        }
    }

    /// Scan the notes directory (creating it if absent).
    pub fn refresh(&mut self) {
        self.count = 0;
        self.loaded = true;
        // Only create the directory if it truly doesn't exist on disk —
        // resolving first triggers lazy expansion so we don't duplicate
        // the VFS node (a second 'notes' node would get its own ext2 dir).
        if crate::fs::vfs::resolve_path(NOTES_DIR).is_err() {
            let _ = crate::fs::vfs::create_dir(NOTES_DIR);
        }
        for i in 0..64 {
            if self.count >= MAX_NOTES {
                break;
            }
            let path = alloc::format!("{}/note{}.txt", NOTES_DIR, i);
            match crate::fs::vfs::read_file(&path) {
                Ok(data) => {
                    let text = core::str::from_utf8(&data).unwrap_or("");
                    let first_line: alloc::string::String = text
                        .lines()
                        .next()
                        .unwrap_or("untitled")
                        .chars()
                        .take(TITLE_MAX)
                        .collect();
                    self.titles[self.count] = first_line;
                    self.count += 1;
                }
                Err(_) => continue,
            }
        }
    }

    fn note_path(idx: usize) -> alloc::string::String {
        // Titles map back to noteN.txt by scan order — we re-scan to find N
        // cheap approximation: store index-stable filenames during refresh
        // (see refresh_with_ids)
        alloc::format!("{}/note{}.txt", NOTES_DIR, idx)
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.count {
            self.selected = idx;
            if let Ok(data) = crate::fs::vfs::read_file(&Self::note_path(idx)) {
                self.content =
                    alloc::string::String::from(core::str::from_utf8(&data).unwrap_or(""));
            }
        }
    }

    /// Insert a character into the selected note + persist.
    pub fn type_char(&mut self, c: char) {
        if self.count == 0 {
            return;
        }
        match c {
            '\n' => self.content.push('\n'),
            '\u{8}' => {
                self.content.pop();
            }
            _ => self.content.push(c),
        }
        let path = Self::note_path(self.selected);
        match crate::fs::vfs::write_file(&path, self.content.as_bytes()) {
            Ok(n) => crate::println!("[notes] saved {} bytes to {}", n, path),
            Err(e) => crate::println!("[notes] save {} failed: {:?}", path, e),
        }
        // Update the title line
        let first: alloc::string::String = self
            .content
            .lines()
            .next()
            .unwrap_or("untitled")
            .chars()
            .take(TITLE_MAX)
            .collect();
        self.titles[self.selected] = first;
    }

    pub fn new_note(&mut self) {
        if self.count >= MAX_NOTES {
            return;
        }
        // Find a free filename
        for i in 0..64 {
            let path = alloc::format!("{}/note{}.txt", NOTES_DIR, i);
            if crate::fs::vfs::read_file(&path).is_err() {
                let _ = crate::fs::vfs::write_file(&path, b"New note\n");
                self.refresh();
                self.select(self.count.saturating_sub(1));
                return;
            }
        }
    }

    /// Render into the Notes window client area.
    pub fn render(&self, cx: u32, cy: u32, cw: u32, ch: u32) {
        let fb = match get_info_nonzero() {
            Some(fb) => fb,
            None => return,
        };
        let theme = theme::current();

        // Sidebar: note list
        let side_w = 180u32;
        g::draw_rect(&fb, cx, cy, side_w, ch, theme.menu_bg);
        g::draw_text(
            &fb,
            cx + 8,
            cy + 6,
            "Notes",
            theme.label_text,
            theme.menu_bg,
        );
        g::draw_rect(&fb, cx + 4, cy + 22, side_w - 8, 1, theme.menu_border);

        let mut ny = cy + 30;
        for i in 0..self.count {
            let sel = i == self.selected;
            let bg = if sel {
                theme.menu_item_hover
            } else {
                theme.menu_bg
            };
            g::fill_rounded_rect(&fb, cx + 4, ny, side_w - 8, 22, 4, bg);
            g::draw_text(
                &fb,
                cx + 10,
                ny + 4,
                &self.titles[i],
                if sel {
                    theme.label_text
                } else {
                    theme.label_text_dim
                },
                bg,
            );
            ny += 26;
        }

        // "+ New" button at sidebar bottom
        let new_y = cy + ch.saturating_sub(30);
        g::fill_rounded_rect(&fb, cx + 8, new_y, side_w - 16, 22, 6, theme.btn_bg);
        g::draw_text(
            &fb,
            cx + 14,
            new_y + 4,
            "+ New note",
            theme.btn_text,
            theme.btn_bg,
        );

        // Content area
        let main_x = cx + side_w;
        let main_w = cw.saturating_sub(side_w);
        g::draw_rect(&fb, main_x, cy, main_w, ch, 0xFFFFF8E7);
        g::draw_rect(&fb, main_x, cy, 1, ch, theme.menu_border);

        // Wrap-free simple line render
        let char_h = 16u32;
        let rows = (ch / char_h) as usize;
        let cols = (main_w / 8 - 2) as usize;
        for (i, line) in self.content.lines().take(rows).enumerate() {
            let text: alloc::string::String = line.chars().take(cols).collect();
            g::draw_text(
                &fb,
                main_x + 10,
                cy + 4 + i as u32 * char_h,
                &text,
                0xFF3B3020,
                0xFFFFF8E7,
            );
        }
    }
}

fn get_info_nonzero() -> Option<crate::display::fb::FramebufferInfo> {
    let fb = get_info();
    if fb.width == 0 {
        None
    } else {
        Some(fb)
    }
}

static mut INSTANCE: Option<Notes> = None;

pub fn init() {
    unsafe {
        if INSTANCE.is_none() {
            let mut n = Notes::new();
            n.refresh();
            if n.count == 0 {
                n.new_note();
                n.select(0);
            } else {
                n.select(0);
            }
            INSTANCE = Some(n);
        }
    }
    crate::println!("[notes] Notes app initialized");
}

#[cfg(feature = "qemu_raspi5b")]
pub fn render_to_windows(fb: &crate::display::fb::FramebufferInfo) {
    let Some(win_id) = window::find_by_title("Notes") else {
        return;
    };
    let Some((x, y, w, h)) = window::get_info(win_id) else {
        return;
    };
    let focused = window::focused_id() == win_id;
    let (cx, cy, cw, ch) =
        crate::wm::window_frame::draw_frame(fb, win_id, x, y, w, h, "Notes", focused);
    unsafe {
        if let Some(n) = INSTANCE.as_ref() {
            n.render(cx, cy, cw, ch);
        }
    }
}

/// Type into the selected note (automation + future keyboard routing).
pub fn type_char(c: char) {
    unsafe {
        if let Some(n) = INSTANCE.as_mut() {
            n.type_char(c);
        }
    }
}

pub fn new_note() {
    unsafe {
        if let Some(n) = INSTANCE.as_mut() {
            n.new_note();
        }
    }
}

pub fn note_count() -> usize {
    unsafe { INSTANCE.as_ref().map(|n| n.count).unwrap_or(0) }
}
