//! Block/inline layout: turns a [`Document`] into positioned text runs and
//! rectangles. The output is shared by the renderer and by link hit-testing,
//! and is rebuilt only on load and on resize.

use super::dom::{Document, NodeKind, Tag};
use crate::display::text::{self, Face, TextStyle};
use alloc::string::String;
use alloc::vec::Vec;

/// Run cap: pages beyond this are cut and flagged as truncated.
pub const MAX_RUNS: usize = 40_000;
/// Marker for runs that are not part of a link.
pub const NO_LINK: u32 = u32::MAX;

/// Body text colour.
pub const COLOR_TEXT: u32 = 0xFF20_2020;
/// Link colour.
pub const COLOR_LINK: u32 = 0xFF0B_57D0;
/// Emphasis colour: the font set has no italic face, so `<i>`/`<em>` keep the
/// regular face and are tinted instead.
pub const COLOR_EM: u32 = 0xFF6A_3D9A;

/// A styled piece of text on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Left edge in page coordinates.
    pub x: u32,
    /// Top of the text line box in page coordinates.
    pub y: u32,
    /// Total advance including box padding.
    pub w: u32,
    /// Inset of the text inside the run (boxes only).
    pub pad: u32,
    /// The displayed characters (every one has a glyph).
    pub text: String,
    /// Typeface.
    pub face: Face,
    /// Requested pixel size.
    pub size: u32,
    /// Text colour.
    pub color: u32,
    /// Underline the run.
    pub underline: bool,
    /// Draw an outline around the run (image placeholders).
    pub boxed: bool,
    /// Index into [`Layout::links`], or [`NO_LINK`].
    pub link: u32,
    /// Top of the line box, for hit-testing.
    pub top: u32,
    /// Bottom of the line box, for hit-testing.
    pub bottom: u32,
}

/// A filled or outlined rectangle (rules, quote bars, code backgrounds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
    /// Fill colour.
    pub color: u32,
}

/// Finished layout of one page at one width.
#[derive(Debug, Default)]
pub struct Layout {
    /// Text runs ordered by line.
    pub runs: Vec<Run>,
    /// Decorations drawn under the text.
    pub rects: Vec<Rect>,
    /// Link targets as written in the document (unresolved).
    pub links: Vec<String>,
    /// `(id or name, y)` of every anchor.
    pub anchors: Vec<(String, u32)>,
    /// Total page height.
    pub height: u32,
    /// Widest content, for clipping decisions.
    pub content_width: u32,
    /// Run or node cap reached.
    pub truncated: bool,
}

impl Layout {
    /// Link id under page point (`x`, `y`).
    #[must_use]
    pub fn link_at(&self, x: u32, y: u32) -> Option<u32> {
        let first = self.runs.partition_point(|r| r.bottom <= y);
        self.runs[first..]
            .iter()
            .take_while(|r| r.top <= y)
            .find(|r| r.link != NO_LINK && x >= r.x && x < r.x + r.w)
            .map(|r| r.link)
    }

    /// Page y of the anchor called `name`.
    #[must_use]
    pub fn anchor_y(&self, name: &str) -> Option<u32> {
        self.anchors
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, y)| *y)
    }
}

/// Horizontal page padding.
const PAGE_PAD_X: u32 = 12;
/// Padding above the first line.
const PAGE_PAD_TOP: u32 = 8;
/// Padding below the last line.
const PAGE_PAD_BOTTOM: u32 = 16;
/// Body text size.
const BODY_SIZE: u32 = 14;
/// `<small>` and image placeholder size.
const SMALL_SIZE: u32 = 12;
/// Monospace size (the atlas generated for the Mono face).
const CODE_SIZE: u32 = 13;
/// Extra leading under every line.
const LINE_GAP: u32 = 4;
/// Vertical space around paragraphs, lists, quotes and code blocks.
const BLOCK_MARGIN: u32 = 10;
/// Vertical space around headings.
const HEADING_MARGIN: u32 = 14;
/// Vertical space between list items.
const ITEM_MARGIN: u32 = 2;
/// Indent per list level; the marker sits in this gutter.
const LIST_INDENT: u32 = 28;
/// Gap between a list marker and the item text.
const MARKER_GAP: u32 = 8;
/// Indent of block quotes.
const QUOTE_INDENT: u32 = 24;
/// Offset of the quote bar from the quote's outer edge.
const QUOTE_BAR_X: u32 = 8;
/// Width of the quote bar.
const QUOTE_BAR_W: u32 = 3;
/// Inset of preformatted text inside its background.
const PRE_PAD: u32 = 8;
/// Vertical padding of the preformatted background.
const PRE_VPAD: u32 = 4;
/// Gap between table cells.
const CELL_GAP: u32 = 24;
/// Inset of image placeholder text inside its outline.
const BOX_PAD: u32 = 4;
/// Longest alt text shown in a placeholder, in characters.
const MAX_ALT_CHARS: usize = 48;
/// Tab stops in preformatted text, in columns.
const TAB_WIDTH: usize = 8;
/// Smallest usable content width when indentation eats the page.
const MIN_CONTENT_WIDTH: u32 = 40;
/// Rule thickness.
const RULE_H: u32 = 1;
/// Quote text colour.
const COLOR_QUOTE: u32 = 0xFF50_5050;
/// Inline code colour.
const COLOR_CODE: u32 = 0xFF7A_2E2E;
/// Image placeholder colour.
const COLOR_IMG: u32 = 0xFF66_6666;
/// Rule and quote-bar colour.
const COLOR_RULE: u32 = 0xFFBB_BBBB;
/// Preformatted block background.
const COLOR_PRE_BG: u32 = 0xFFF2_F2F2;
/// Note appended when the page was cut.
const TRUNCATED_NOTE: &str = "[Page truncated]";
/// Symbols (beyond ASCII and Latin-1) the font atlases cover.
const ATLAS_SYMBOLS: &[char] = &[
    '\u{2013}', '\u{2014}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2026}',
    '\u{2190}', '\u{2191}', '\u{2192}', '\u{2193}', '\u{2212}', '\u{25b2}', '\u{25b6}', '\u{25bc}',
    '\u{25c0}', '\u{25cf}', '\u{2605}', '\u{2713}', '\u{2715}',
];

/// Character to draw for `c`: itself when a glyph exists, `?` otherwise,
/// `None` for characters that must vanish (controls, zero-width, soft hyphen).
fn display_char(c: char) -> Option<char> {
    match c {
        '\u{a0}' => Some(' '),
        '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{feff}' => None,
        ' '..='~' | '\u{a1}'..='\u{ff}' => Some(c),
        _ if c.is_control() => None,
        _ if ATLAS_SYMBOLS.contains(&c) => Some(c),
        _ => Some('?'),
    }
}

/// Inline formatting inherited down the tree.
#[derive(Clone, Copy)]
struct Inline {
    bold: bool,
    em: bool,
    code: bool,
    small: bool,
    quote: bool,
    size: u32,
    link: u32,
}

/// Resolved font and colour for one run.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Look {
    face: Face,
    size: u32,
    color: u32,
    underline: bool,
    link: u32,
}

impl Inline {
    const BODY: Inline = Inline {
        bold: false,
        em: false,
        code: false,
        small: false,
        quote: false,
        size: BODY_SIZE,
        link: NO_LINK,
    };

    fn look(&self) -> Look {
        let face = if self.code {
            Face::Mono
        } else if self.bold {
            Face::UiBold
        } else {
            Face::Ui
        };
        let size = if self.code {
            CODE_SIZE
        } else if self.small {
            SMALL_SIZE
        } else {
            self.size
        };
        Look {
            face,
            size,
            color: self.color(),
            underline: self.link != NO_LINK,
            link: self.link,
        }
    }

    fn color(&self) -> u32 {
        if self.link != NO_LINK {
            COLOR_LINK
        } else if self.em {
            COLOR_EM
        } else if self.code {
            COLOR_CODE
        } else if self.quote {
            COLOR_QUOTE
        } else {
            COLOR_TEXT
        }
    }
}

impl Look {
    fn style(&self) -> TextStyle {
        TextStyle {
            face: self.face,
            size: self.size,
            color: self.color,
        }
    }
}

/// Work to do when an element's subtree ends.
#[derive(Clone, Copy)]
enum Exit {
    Nothing,
    Block(u32),
    Quote { prev_left: u32, start_y: u32 },
    Pre { prev_left: u32, start_y: u32 },
    List { prev_left: u32 },
    Center,
}

struct Frame {
    node: usize,
    next: usize,
    inline: Inline,
    exit: Exit,
}

/// One open list level.
struct ListCtx {
    ordered: bool,
    count: u32,
}

struct Engine<'a> {
    doc: &'a Document,
    out: Layout,
    width: u32,
    left: u32,
    right: u32,
    y: u32,
    trailing_margin: u32,
    line: Vec<Run>,
    pen: u32,
    has_words: bool,
    pending_space: bool,
    center: u32,
    pre: u32,
    pre_first: bool,
    cells: u32,
    lists: Vec<ListCtx>,
    done: bool,
}

impl<'a> Engine<'a> {
    fn new(doc: &'a Document, width: u32) -> Self {
        let mut e = Engine {
            doc,
            out: Layout::default(),
            width,
            left: PAGE_PAD_X,
            right: 0,
            y: PAGE_PAD_TOP,
            trailing_margin: u32::MAX / 2,
            line: Vec::new(),
            pen: PAGE_PAD_X,
            has_words: false,
            pending_space: false,
            center: 0,
            pre: 0,
            pre_first: false,
            cells: 0,
            lists: Vec::new(),
            done: false,
        };
        e.set_left(PAGE_PAD_X);
        e
    }

    /// Move the left edge, keeping a usable content width.
    fn set_left(&mut self, left: u32) {
        let max_left = (self.width / 2).max(PAGE_PAD_X);
        self.left = left.min(max_left);
        self.right = self
            .width
            .saturating_sub(PAGE_PAD_X)
            .max(self.left + MIN_CONTENT_WIDTH);
        if !self.has_words {
            self.pen = self.left;
        }
    }

    fn marker_only(&self) -> bool {
        !self.has_words && !self.line.is_empty()
    }

    fn flush_line(&mut self) {
        if self.line.is_empty() || self.done {
            self.reset_line();
            return;
        }
        let (asc, desc) = self.line.iter().fold((0, 0), |(a, d), r| {
            let st = Look {
                face: r.face,
                size: r.size,
                color: r.color,
                underline: false,
                link: NO_LINK,
            }
            .style();
            (a.max(text::ascent(&st)), d.max(text::descent(&st)))
        });
        let top = self.y;
        let bottom = top + asc + desc + LINE_GAP;
        let shift = self.center_shift();
        let runs = core::mem::take(&mut self.line);
        for mut r in runs {
            let st = TextStyle {
                face: r.face,
                size: r.size,
                color: r.color,
            };
            r.y = top + asc - text::ascent(&st);
            r.top = top;
            r.bottom = bottom;
            r.x += shift;
            self.push_run(r);
        }
        self.y = bottom;
        self.trailing_margin = 0;
        self.reset_line();
    }

    fn center_shift(&self) -> u32 {
        if self.center == 0 || !self.has_words {
            return 0;
        }
        let used = self.pen.saturating_sub(self.left);
        (self.right - self.left).saturating_sub(used) / 2
    }

    fn push_run(&mut self, run: Run) {
        if self.out.runs.len() >= MAX_RUNS {
            self.done = true;
            self.out.truncated = true;
            return;
        }
        self.out.content_width = self.out.content_width.max(run.x + run.w + PAGE_PAD_X);
        self.out.runs.push(run);
    }

    fn reset_line(&mut self) {
        self.line.clear();
        self.pen = self.left;
        self.has_words = false;
        self.pending_space = false;
    }

    /// End the current line; on an empty line leave a blank one instead.
    fn break_line(&mut self) {
        if self.line.is_empty() {
            let st = Inline::BODY.look().style();
            self.y += text::line_height(&st) + LINE_GAP;
            self.trailing_margin = 0;
        } else {
            self.flush_line();
        }
    }

    fn add_margin(&mut self, margin: u32) {
        if margin > self.trailing_margin {
            self.y += margin - self.trailing_margin;
            self.trailing_margin = margin;
        }
    }

    fn begin_block(&mut self, margin: u32) {
        if !self.marker_only() {
            self.flush_line();
        }
        self.add_margin(margin);
    }

    fn end_block(&mut self, margin: u32) {
        self.flush_line();
        self.add_margin(margin);
    }

    fn record_anchor(&mut self, node: usize) {
        for key in ["id", "name"] {
            if let Some(v) = self.doc.attr(node, key) {
                if !v.is_empty() {
                    self.out.anchors.push((String::from(v), self.y));
                }
            }
        }
    }

    fn add_text(&mut self, text: &str, look: Look, space: u32) {
        let w = text::width(text, &look.style());
        let can_merge = self.line.last().is_some_and(|l| {
            !l.boxed
                && l.x + l.w == self.pen
                && l.face == look.face
                && l.size == look.size
                && l.color == look.color
                && l.underline == look.underline
                && l.link == look.link
        });
        if let (true, Some(last)) = (can_merge, self.line.last_mut()) {
            if space > 0 {
                last.text.push(' ');
            }
            last.text.push_str(text);
            last.w += space + w;
        } else {
            self.line.push(new_run(self.pen + space, w, text, look));
        }
        self.pen += space + w;
        self.has_words = true;
        self.pending_space = false;
    }

    fn place_word(&mut self, word: &str, inline: &Inline) {
        let look = inline.look();
        let style = look.style();
        let word_w = text::width(word, &style);
        let mut space = if self.pending_space && self.has_words {
            text::width(" ", &style)
        } else {
            0
        };
        if self.pen + space + word_w > self.right && self.has_words {
            self.flush_line();
            space = 0;
        }
        if self.pen + word_w <= self.right {
            self.add_text(word, look, space);
        } else {
            self.break_word(word, look);
        }
    }

    /// Split an over-long word across lines at character boundaries.
    fn break_word(&mut self, word: &str, look: Look) {
        let style = look.style();
        let mut chunk = String::new();
        let mut chunk_w = 0;
        for c in word.chars() {
            let cw = text::width(c.encode_utf8(&mut [0; 4]), &style);
            if !chunk.is_empty() && self.pen + chunk_w + cw > self.right {
                self.add_text(&chunk, look, 0);
                self.flush_line();
                chunk.clear();
                chunk_w = 0;
            }
            chunk.push(c);
            chunk_w += cw;
        }
        if !chunk.is_empty() {
            self.add_text(&chunk, look, 0);
        }
    }

    fn flow_text(&mut self, s: &str, inline: &Inline) {
        let mut word = String::new();
        for c in s.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
                if !word.is_empty() {
                    self.place_word(&word, inline);
                    word.clear();
                }
                self.pending_space = true;
            } else if let Some(d) = display_char(c) {
                word.push(d);
            }
        }
        if !word.is_empty() {
            self.place_word(&word, inline);
        }
    }

    fn pre_text(&mut self, s: &str, inline: &Inline) {
        let mut s = s;
        if self.pre_first {
            s = s
                .strip_prefix("\r\n")
                .or_else(|| s.strip_prefix('\n'))
                .unwrap_or(s);
            self.pre_first = false;
        }
        let look = inline.look();
        for (i, seg) in s.split('\n').enumerate() {
            if i > 0 {
                self.break_line();
            }
            let shown = expand_pre_segment(seg);
            if !shown.is_empty() {
                self.add_text(&shown, look, 0);
            }
        }
    }

    fn text(&mut self, s: &str, inline: &Inline) {
        if self.pre > 0 {
            self.pre_text(s, inline);
        } else {
            self.flow_text(s, inline);
        }
    }

    fn place_image(&mut self, alt: &str, inline: &Inline) {
        let label: String = alt
            .chars()
            .filter_map(display_char)
            .take(MAX_ALT_CHARS)
            .collect();
        let label = alloc::format!("[{}]", if label.is_empty() { "image" } else { &label });
        let look = Look {
            face: Face::Ui,
            size: SMALL_SIZE,
            color: if inline.link != NO_LINK {
                COLOR_LINK
            } else {
                COLOR_IMG
            },
            underline: false,
            link: inline.link,
        };
        let w = text::width(&label, &look.style()) + 2 * BOX_PAD;
        let space = if self.pending_space && self.has_words {
            BOX_PAD
        } else {
            0
        };
        if self.pen + space + w > self.right && self.has_words {
            self.flush_line();
        }
        let x = self.pen + if self.has_words { space } else { 0 };
        let mut run = new_run(x, w, &label, look);
        run.boxed = true;
        run.pad = BOX_PAD;
        self.line.push(run);
        self.pen = x + w;
        self.has_words = true;
        self.pending_space = false;
    }

    fn begin_item(&mut self) {
        self.begin_block(ITEM_MARGIN);
        let marker = match self.lists.last_mut() {
            Some(ctx) if ctx.ordered => {
                ctx.count += 1;
                alloc::format!("{}.", ctx.count)
            }
            _ => String::from("\u{2022}"),
        };
        let look = Inline::BODY.look();
        let w = text::width(&marker, &look.style());
        let x = self.left.saturating_sub(w + MARKER_GAP);
        let mut run = new_run(x, w, &marker, look);
        run.color = COLOR_TEXT;
        self.line.push(run);
    }

    fn horizontal_rule(&mut self) {
        self.begin_block(BLOCK_MARGIN);
        let rect = Rect {
            x: self.left,
            y: self.y,
            w: self.right - self.left,
            h: RULE_H,
            color: COLOR_RULE,
        };
        self.out.rects.push(rect);
        self.y += RULE_H;
        self.trailing_margin = 0;
        self.add_margin(BLOCK_MARGIN);
    }

    fn start_cell(&mut self, bold: bool, inline: Inline) -> Inline {
        if self.cells > 0 && self.has_words {
            self.pen += CELL_GAP;
            self.pending_space = false;
        }
        self.cells += 1;
        Inline {
            bold: inline.bold || bold,
            ..inline
        }
    }

    /// Open `node`; returns the frame to walk its children, if any.
    fn enter(&mut self, node: usize, tag: Tag, inline: Inline) -> Option<Frame> {
        if self.enter_leaf(node, tag, &inline) {
            return None;
        }
        let (inner, exit) = match self.enter_block(tag, inline) {
            Some(pair) => pair,
            None => (self.enter_inline(node, tag, inline), Exit::Nothing),
        };
        self.record_anchor(node);
        Some(Frame {
            node,
            next: 0,
            inline: inner,
            exit,
        })
    }

    /// Elements without children; true when `tag` was one of them.
    fn enter_leaf(&mut self, node: usize, tag: Tag, inline: &Inline) -> bool {
        match tag {
            Tag::Hidden | Tag::Root => {}
            Tag::Br => self.break_line(),
            Tag::Hr => self.horizontal_rule(),
            Tag::Img => {
                let alt = self.doc.attr(node, "alt").unwrap_or("");
                self.place_image(alt, inline);
            }
            _ => return false,
        }
        true
    }

    /// Block-level containers; `None` when `tag` is not one.
    fn enter_block(&mut self, tag: Tag, inline: Inline) -> Option<(Inline, Exit)> {
        let mut inner = inline;
        let exit = match tag {
            Tag::Heading(level) => {
                inner.bold = true;
                inner.size = heading_size(level);
                self.simple_block(HEADING_MARGIN)
            }
            Tag::P => self.simple_block(BLOCK_MARGIN),
            Tag::Table => self.simple_block(BLOCK_MARGIN),
            Tag::Div => self.simple_block(0),
            Tag::Tr => {
                self.cells = 0;
                self.simple_block(0)
            }
            Tag::Li => {
                self.begin_item();
                Exit::Block(ITEM_MARGIN)
            }
            Tag::Center => {
                self.begin_block(0);
                self.center += 1;
                Exit::Center
            }
            Tag::Ul | Tag::Ol => self.enter_list(tag == Tag::Ol),
            Tag::Blockquote => {
                inner.quote = true;
                self.enter_quote()
            }
            Tag::Pre => {
                inner.code = true;
                self.enter_pre()
            }
            _ => return None,
        };
        Some((inner, exit))
    }

    fn simple_block(&mut self, margin: u32) -> Exit {
        self.begin_block(margin);
        Exit::Block(margin)
    }

    fn enter_quote(&mut self) -> Exit {
        self.begin_block(BLOCK_MARGIN);
        let exit = Exit::Quote {
            prev_left: self.left,
            start_y: self.y,
        };
        self.set_left(self.left + QUOTE_INDENT);
        exit
    }

    fn enter_pre(&mut self) -> Exit {
        self.begin_block(BLOCK_MARGIN);
        let exit = Exit::Pre {
            prev_left: self.left,
            start_y: self.y,
        };
        self.y += PRE_VPAD;
        self.set_left(self.left + PRE_PAD);
        self.pre += 1;
        self.pre_first = true;
        exit
    }

    /// Inline containers (and table cells), which only change formatting.
    fn enter_inline(&mut self, node: usize, tag: Tag, inline: Inline) -> Inline {
        let mut inner = inline;
        match tag {
            Tag::Td => inner = self.start_cell(false, inline),
            Tag::Th => inner = self.start_cell(true, inline),
            Tag::A => inner.link = self.link_for(node).unwrap_or(inline.link),
            Tag::Bold => inner.bold = true,
            Tag::Em => inner.em = true,
            Tag::Small => inner.small = true,
            Tag::Code => inner.code = true,
            _ => {}
        }
        inner
    }

    fn enter_list(&mut self, ordered: bool) -> Exit {
        let margin = if self.lists.is_empty() {
            BLOCK_MARGIN
        } else {
            0
        };
        self.begin_block(margin);
        let exit = Exit::List {
            prev_left: self.left,
        };
        self.set_left(self.left + LIST_INDENT);
        self.lists.push(ListCtx { ordered, count: 0 });
        exit
    }

    fn link_for(&mut self, node: usize) -> Option<u32> {
        let href = self.doc.attr(node, "href")?;
        self.out.links.push(String::from(href));
        Some((self.out.links.len() - 1) as u32)
    }

    fn leave(&mut self, exit: Exit) {
        match exit {
            Exit::Nothing => {}
            Exit::Block(margin) => self.end_block(margin),
            Exit::Center => {
                self.flush_line();
                self.center -= 1;
            }
            Exit::List { prev_left } => {
                self.lists.pop();
                self.set_left(prev_left);
                let margin = if self.lists.is_empty() {
                    BLOCK_MARGIN
                } else {
                    0
                };
                self.end_block(margin);
            }
            Exit::Quote { prev_left, start_y } => {
                self.flush_line();
                let h = self.y.saturating_sub(start_y);
                self.out.rects.push(Rect {
                    x: prev_left + QUOTE_BAR_X,
                    y: start_y,
                    w: QUOTE_BAR_W,
                    h,
                    color: COLOR_RULE,
                });
                self.set_left(prev_left);
                self.add_margin(BLOCK_MARGIN);
            }
            Exit::Pre { prev_left, start_y } => self.leave_pre(prev_left, start_y),
        }
    }

    fn leave_pre(&mut self, prev_left: u32, start_y: u32) {
        self.flush_line();
        self.y += PRE_VPAD;
        self.pre -= 1;
        self.set_left(prev_left);
        self.out.rects.push(Rect {
            x: prev_left,
            y: start_y,
            w: self.right - prev_left,
            h: self.y - start_y,
            color: COLOR_PRE_BG,
        });
        self.trailing_margin = 0;
        self.add_margin(BLOCK_MARGIN);
    }

    fn walk(&mut self) {
        let doc = self.doc;
        let mut stack = alloc::vec![Frame {
            node: 0,
            next: 0,
            inline: Inline::BODY,
            exit: Exit::Nothing
        }];
        while !self.done {
            let Some(frame) = stack.last_mut() else { break };
            let Some(&child) = doc.nodes[frame.node].children.get(frame.next) else {
                let exit = frame.exit;
                stack.pop();
                self.leave(exit);
                continue;
            };
            frame.next += 1;
            let inline = frame.inline;
            match &doc.nodes[child].kind {
                NodeKind::Text(t) => self.text(t, &inline),
                NodeKind::Element { tag, .. } => {
                    if let Some(f) = self.enter(child, *tag, inline) {
                        stack.push(f);
                    }
                }
            }
        }
    }

    fn finish(mut self) -> Layout {
        self.flush_line();
        self.out.truncated |= self.doc.truncated;
        if self.out.truncated {
            self.append_truncation_note();
        }
        self.out.height = self.y + PAGE_PAD_BOTTOM;
        self.out.content_width = self.out.content_width.max(self.width);
        self.out
    }

    fn append_truncation_note(&mut self) {
        let look = Inline {
            small: true,
            ..Inline::BODY
        }
        .look();
        let w = text::width(TRUNCATED_NOTE, &look.style());
        let mut run = new_run(PAGE_PAD_X, w, TRUNCATED_NOTE, look);
        run.y = self.y;
        run.top = self.y;
        run.bottom = self.y + text::line_height(&look.style()) + LINE_GAP;
        self.y = run.bottom;
        self.out.runs.push(run);
    }
}

fn new_run(x: u32, w: u32, text: &str, look: Look) -> Run {
    Run {
        x,
        y: 0,
        w,
        pad: 0,
        text: String::from(text),
        face: look.face,
        size: look.size,
        color: look.color,
        underline: look.underline,
        boxed: false,
        link: look.link,
        top: 0,
        bottom: 0,
    }
}

/// Pixel size of `<h1>`..`<h6>`.
fn heading_size(level: u8) -> u32 {
    match level {
        1 => 24,
        2 => 20,
        3 => 16,
        _ => 14,
    }
}

/// Drop unrenderable characters and expand tabs in one preformatted segment.
fn expand_pre_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    for c in seg.chars() {
        if c == '\t' {
            let pad = TAB_WIDTH - out.chars().count() % TAB_WIDTH;
            out.extend(core::iter::repeat_n(' ', pad));
        } else if let Some(d) = display_char(c) {
            out.push(d);
        }
    }
    out
}

/// Lay `doc` out for a content area `width` px wide.
#[must_use]
pub fn layout(doc: &Document, width: u32) -> Layout {
    let mut engine = Engine::new(doc, width);
    engine.walk();
    engine.finish()
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::apps::browser::dom::parse;
    use crate::display::text::{self, TextStyle};
    use alloc::format;

    const W: u32 = 400;

    fn lay(html: &str, width: u32) -> Layout {
        layout(&parse(html), width)
    }

    fn style_of(r: &Run) -> TextStyle {
        TextStyle {
            face: r.face,
            size: r.size,
            color: r.color,
        }
    }

    fn lines(l: &Layout) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut last_top = None;
        let mut last_end = 0;
        for r in &l.runs {
            if last_top == Some(r.top) {
                if let Some(line) = out.last_mut() {
                    if r.x > last_end {
                        line.push(' ');
                    }
                    line.push_str(&r.text);
                }
            } else {
                out.push(r.text.clone());
                last_top = Some(r.top);
            }
            last_end = r.x + r.w;
        }
        out
    }

    fn run_with<'a>(l: &'a Layout, needle: &str) -> &'a Run {
        l.runs
            .iter()
            .find(|r| r.text.contains(needle))
            .unwrap_or_else(|| panic!("no run containing {needle:?} in {:?}", l.runs))
    }

    #[test]
    fn short_text_is_one_run_at_the_origin_padding() {
        let l = lay("<p>hello world</p>", W);
        assert_eq!(l.runs.len(), 1);
        assert_eq!(l.runs[0].text, "hello world");
        assert!(l.runs[0].x > 0 && l.runs[0].x < 40);
        assert!(l.height > l.runs[0].y);
    }

    #[test]
    fn whitespace_collapses_outside_pre() {
        let l = lay("<p>a   \n\t b  <b>c</b>\n d</p>", W);
        assert_eq!(lines(&l), ["a b c d"]);
    }

    #[test]
    fn text_wraps_at_the_content_width() {
        let words = "the quick brown fox jumps over the lazy dog ".repeat(8);
        let l = lay(&format!("<p>{words}</p>"), 300);
        assert!(l.runs.len() > 3);
        for r in &l.runs {
            assert!(r.x + r.w <= 300, "run {:?} overflows", r.text);
            assert_eq!(r.w, text::width(&r.text, &style_of(r)));
        }
        let joined = lines(&l).join(" ");
        assert_eq!(joined, words.trim());
    }

    #[test]
    fn unbreakable_word_breaks_by_characters() {
        let l = lay(&format!("<p>{}</p>", "a".repeat(300)), 200);
        assert!(l.runs.len() >= 3);
        assert!(l.runs.iter().all(|r| r.x + r.w <= 200));
        let all: String = l.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(all, "a".repeat(300));
    }

    #[test]
    fn headings_use_the_documented_sizes() {
        let l = lay("<h1>A</h1><h2>B</h2><h3>C</h3><h4>D</h4><h6>E</h6>", W);
        let sizes: Vec<_> = l.runs.iter().map(|r| (r.size, r.face)).collect();
        assert_eq!(
            sizes,
            [
                (24, Face::UiBold),
                (20, Face::UiBold),
                (16, Face::UiBold),
                (14, Face::UiBold),
                (14, Face::UiBold)
            ]
        );
    }

    #[test]
    fn blocks_stack_vertically_with_spacing() {
        let l = lay("<p>one</p><p>two</p>", W);
        let (a, b) = (&l.runs[0], &l.runs[1]);
        assert!(b.y > a.y + text::line_height(&style_of(a)));
    }

    #[test]
    fn br_starts_a_new_line() {
        let l = lay("a<br>b<br><br>c", W);
        let ys: Vec<_> = l.runs.iter().map(|r| r.y).collect();
        assert_eq!(ys.len(), 3);
        assert!(ys[0] < ys[1] && ys[1] < ys[2]);
        assert!(
            ys[2] - ys[1] > ys[1] - ys[0],
            "double br leaves a blank line"
        );
    }

    #[test]
    fn unordered_list_has_bullets_left_of_text() {
        let l = lay("<ul><li>one</li><li>two</li></ul>", W);
        let bullets: Vec<_> = l.runs.iter().filter(|r| r.text == "\u{2022}").collect();
        assert_eq!(bullets.len(), 2);
        let one = run_with(&l, "one");
        assert!(bullets[0].x + bullets[0].w <= one.x);
        assert_eq!(bullets[0].y, one.y);
    }

    #[test]
    fn ordered_list_numbers_items() {
        let l = lay("<ol><li>a</li><li>b</li><li>c</li></ol>", W);
        let marks: Vec<_> = l
            .runs
            .iter()
            .filter(|r| r.text.ends_with('.'))
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(marks, ["1.", "2.", "3."]);
    }

    #[test]
    fn nested_lists_indent_and_restart_numbering() {
        let l = lay(
            "<ol><li>a<ol><li>x</li><li>y</li></ol></li><li>b</li></ol>",
            W,
        );
        let marks: Vec<_> = l
            .runs
            .iter()
            .filter(|r| r.text.ends_with('.'))
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(marks, ["1.", "1.", "2.", "2."]);
        assert!(run_with(&l, "x").x > run_with(&l, "a").x);
        assert_eq!(run_with(&l, "b").x, run_with(&l, "a").x);
    }

    #[test]
    fn links_carry_their_href_and_colour() {
        let l = lay("<p>see <a href=\"/x\">this <b>link</b></a> now</p>", W);
        let link_runs: Vec<_> = l.runs.iter().filter(|r| r.link != NO_LINK).collect();
        assert!(!link_runs.is_empty());
        for r in &link_runs {
            assert_eq!(l.links[r.link as usize], "/x");
            assert_eq!(r.color, COLOR_LINK);
            assert!(r.underline);
        }
        assert_eq!(run_with(&l, "see").link, NO_LINK);
        assert_eq!(run_with(&l, "now").link, NO_LINK);
        assert!(link_runs.iter().any(|r| r.face == Face::UiBold));
    }

    #[test]
    fn link_hit_test_uses_the_line_box() {
        let l = lay("<p>go <a href=\"/x\">here</a> ok</p>", W);
        let r = run_with(&l, "here");
        let id = r.link;
        assert_eq!(l.link_at(r.x + r.w / 2, r.y + 2), Some(id));
        assert_eq!(l.link_at(r.x + r.w + 200, r.y + 2), None);
        assert_eq!(l.link_at(r.x + 1, r.bottom + 50), None);
        let plain = run_with(&l, "go");
        assert_eq!(l.link_at(plain.x + 1, plain.y + 1), None);
    }

    #[test]
    fn anchors_record_their_position() {
        let l = lay(
            "<p>top</p><h2 id=\"sec\">Section</h2><a name=\"old\"></a><p>x</p>",
            W,
        );
        let sec = l.anchor_y("sec").expect("sec anchor");
        assert_eq!(sec, run_with(&l, "Section").top);
        assert!(l.anchor_y("old").is_some());
        assert_eq!(l.anchor_y("missing"), None);
    }

    #[test]
    fn pre_keeps_whitespace_uses_mono_and_does_not_wrap() {
        let long = "x".repeat(200);
        let l = lay(&format!("<pre>  a\n    b\n{long}</pre>"), 200);
        assert!(l.runs.iter().all(|r| r.face == Face::Mono));
        assert_eq!(lines(&l)[..2], ["  a", "    b"]);
        let wide = run_with(&l, "xxx");
        assert!(wide.x + wide.w > 200, "long line must overflow, not wrap");
        assert_eq!(wide.text, long);
        assert!(l.content_width > 200);
    }

    #[test]
    fn pre_draws_a_background_rect() {
        let l = lay("<pre>code</pre>", W);
        assert!(!l.rects.is_empty());
    }

    #[test]
    fn blockquote_indents_and_draws_a_bar() {
        let l = lay("<p>plain</p><blockquote>quoted</blockquote>", W);
        assert!(run_with(&l, "quoted").x > run_with(&l, "plain").x);
        assert!(l.rects.iter().any(|r| r.w <= 4 && r.h > 10));
    }

    #[test]
    fn hr_is_a_horizontal_rule() {
        let l = lay("a<hr>b", W);
        assert!(l.rects.iter().any(|r| r.h <= 2 && r.w > W / 2));
    }

    #[test]
    fn table_cells_share_a_line_with_gaps() {
        let l = lay(
            "<table><tr><td>aa</td><td>bb</td></tr><tr><td>cc</td></tr></table>",
            W,
        );
        let (a, b, c) = (run_with(&l, "aa"), run_with(&l, "bb"), run_with(&l, "cc"));
        assert_eq!(a.y, b.y);
        assert!(b.x > a.x + a.w);
        assert!(c.y > a.y);
    }

    #[test]
    fn images_show_boxed_alt_text() {
        let l = lay("<p>x <img src=a.png alt=\"A cat\"> y</p>", W);
        let img = run_with(&l, "A cat");
        assert!(img.boxed);
        let bare = lay("<img src=a.png>", W);
        assert!(bare.runs[0].text.contains("image"));
    }

    #[test]
    fn emphasis_is_tinted_and_bold_uses_the_bold_face() {
        let l = lay("<p><i>it</i> <b>bd</b> <em>em</em></p>", W);
        assert_eq!(run_with(&l, "it").color, COLOR_EM);
        assert_eq!(run_with(&l, "it").face, Face::Ui);
        assert_eq!(run_with(&l, "bd").face, Face::UiBold);
    }

    #[test]
    fn center_shifts_the_line() {
        let plain = lay("<p>mid</p>", W);
        let centred = lay("<center>mid</center>", W);
        assert!(centred.runs[0].x > plain.runs[0].x + 50);
    }

    #[test]
    fn code_uses_mono() {
        let l = lay("<p>run <code>ls -l</code></p>", W);
        assert_eq!(run_with(&l, "ls").face, Face::Mono);
    }

    #[test]
    fn characters_without_glyphs_become_question_marks() {
        let l = lay("<p>a\u{4e2d}b\u{1f600}c &#8212; \u{2022}</p>", W);
        assert_eq!(l.runs[0].text, "a?b?c \u{2014} \u{2022}");
    }

    #[test]
    fn hidden_elements_do_not_render() {
        let l = lay(
            "<head><title>T</title></head><script>x</script><p>shown</p>",
            W,
        );
        assert_eq!(lines(&l), ["shown"]);
    }

    #[test]
    fn run_cap_truncates_and_flags_the_page() {
        let html = "<p>w</p>".repeat(MAX_RUNS + 500);
        let l = lay(&html, W);
        assert!(l.truncated);
        assert!(l.runs.len() <= MAX_RUNS + 2);
    }

    #[test]
    fn tiny_width_still_terminates() {
        let l = lay("<p>abcdef ghi</p>", 5);
        let all: String = l.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(all.replace(' ', ""), "abcdefghi");
    }

    #[test]
    fn height_covers_all_runs() {
        let l = lay("<p>a</p><p>b</p><pre>c\nd</pre>", W);
        assert!(l.runs.iter().all(|r| r.bottom <= l.height));
    }

    /// Deterministic pseudo-random bytes (no `rand` in the kernel crate).
    fn noise(seed: u64, len: usize) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u8
            })
            .collect()
    }

    const SAMPLE: &str = "<html><head><title>t</title></head><body><h1 id=a>H</h1><p class=x>Some <b>bold \
        <a href=\"/y?z=1&amp;w\">link</a></b> text &amp; &#x41;</p><ul><li>one<li>two<ol><li>n</ol></ul>\
        <pre>a\n\tb</pre><table><tr><td>1<td>2</table><blockquote>q</blockquote><img alt=i><hr></body></html>";

    #[test]
    fn random_bytes_never_panic() {
        for seed in 0..200 {
            let bytes = noise(seed, 1 + (seed as usize * 37) % 3000);
            let html = crate::apps::browser::html::decode_bytes(&bytes);
            let l = lay(&html, 50 + (seed as u32 * 13) % 900);
            assert!(l.height > 0);
        }
    }

    #[test]
    fn markup_soup_never_panics() {
        let alphabet: Vec<&str> = alloc::vec![
            "<", ">", "/", "a", "p", "li", "ul", " ", "=", "\"", "'", "&", ";", "#", "!--", "pre",
            "td", "\n"
        ];
        for seed in 0..200u64 {
            let picks = noise(seed, 400);
            let html: String = picks
                .iter()
                .map(|b| alphabet[*b as usize % alphabet.len()])
                .collect();
            let _ = lay(&html, 120);
        }
    }

    #[test]
    fn every_prefix_of_a_page_lays_out() {
        for end in (0..SAMPLE.len()).filter(|i| SAMPLE.is_char_boundary(*i)) {
            let l = lay(&SAMPLE[..end], 200);
            assert!(l.height > 0);
        }
    }

    #[test]
    fn ten_thousand_nested_divs_lay_out_with_their_text() {
        let html = format!("{}deep{}", "<div>".repeat(10_000), "</div>".repeat(10_000));
        let l = lay(&html, W);
        assert_eq!(l.runs.len(), 1);
        assert_eq!(l.runs[0].text, "deep");
    }

    #[test]
    fn deeply_nested_lists_keep_a_readable_width() {
        let html = format!("{}x{}", "<ul><li>".repeat(300), "</li></ul>".repeat(300));
        let l = lay(&html, W);
        let run = run_with(&l, "x");
        assert!(run.x < W, "indent must be capped, got {}", run.x);
    }
}
