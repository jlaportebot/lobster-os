//! Pure text/HTML logic for the Lobster Browser (O4.3).
//!
//! Everything here is hardware-free so it runs under the host unit tests
//! (`cargo test --features std --lib`). `browser.rs` owns the UI and the
//! network call; this module owns the parts that used to panic the kernel:
//! the line buffer, entity decoding and the history stack.

use alloc::string::String;
use alloc::vec::Vec;

/// Maximum number of rendered lines kept per page.
pub const MAX_LINES: usize = 400;
/// Maximum bytes per rendered line.
pub const LINE_LEN: usize = 110;
/// Longest URL the browser will hand to the network device.
pub const MAX_URL_BYTES: usize = 3800;
/// Maximum history entries kept in memory.
pub const MAX_HISTORY: usize = 64;

// ─── Entity decoding ─────────────────────────────────────────────────

/// Longest entity body we look at (`&#x10FFFF;` is 10 bytes; named ones are shorter).
const MAX_ENTITY_LEN: usize = 12;

/// Fallback glyph for characters the ASCII-only renderer cannot draw.
const REPLACEMENT: u8 = b'?';

/// Try to decode one HTML entity at the start of `s` (which must begin with `&`).
///
/// Returns `(byte, consumed)` where `byte` is the printable ASCII byte to emit
/// and `consumed` is the number of input bytes the entity occupied, or `None`
/// if `s` does not start with a well-formed, known entity (the caller then
/// emits the `&` literally and moves on by one byte).
pub fn decode_entity(s: &[u8]) -> Option<(u8, usize)> {
    if s.first() != Some(&b'&') {
        return None;
    }
    let window = &s[..s.len().min(MAX_ENTITY_LEN)];
    let semi = window.iter().position(|&b| b == b';')?;
    let body = &window[1..semi];
    if body.is_empty() {
        return None;
    }
    let consumed = semi + 1;

    if body[0] == b'#' {
        let (digits, radix) = match body.get(1) {
            Some(b'x') | Some(b'X') => (&body[2..], 16),
            _ => (&body[1..], 10),
        };
        if digits.is_empty() {
            return None;
        }
        let mut v: u32 = 0;
        for &d in digits {
            let n = (d as char).to_digit(radix)?;
            // Saturate: a huge value is simply "not a character" below.
            v = v.saturating_mul(radix).saturating_add(n);
        }
        return Some((ascii_for_codepoint(v), consumed));
    }

    let b = match body {
        b"amp" => b'&',
        b"lt" => b'<',
        b"gt" => b'>',
        b"quot" => b'"',
        b"apos" => b'\'',
        b"nbsp" => b' ',
        _ => return None,
    };
    Some((b, consumed))
}

/// Map a Unicode code point to the printable ASCII byte we can draw.
fn ascii_for_codepoint(cp: u32) -> u8 {
    match cp {
        0x20..=0x7E => cp as u8,
        0x09 | 0x0A | 0x0D => b' ',
        0xA0 => b' ', // no-break space
        _ => REPLACEMENT,
    }
}

/// Decode every entity in `raw` (used for attribute values).
pub fn decode_entities(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'&' {
            if let Some((b, used)) = decode_entity(&raw[i..]) {
                out.push(b);
                i += used;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    out
}

// ─── Tags ────────────────────────────────────────────────────────────

/// Find the end of the tag that starts at `html[start] == b'<'`.
///
/// Returns the index of the closing `>` (or `html.len()` for an unterminated
/// tag). `>` inside a quoted attribute value does not end the tag, and
/// `<!-- ... -->` comments are skipped whole.
pub fn tag_end(html: &[u8], start: usize) -> usize {
    let n = html.len();
    if html[start..].starts_with(b"<!--") {
        return match html[start + 4..].windows(3).position(|w| w == b"-->") {
            Some(off) => start + 4 + off + 2,
            None => n,
        };
    }
    let mut j = start + 1;
    let mut quote: Option<u8> = None;
    while j < n {
        let c = html[j];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == b'"' || c == b'\'' {
                    quote = Some(c);
                } else if c == b'>' {
                    return j;
                }
            }
        }
        j += 1;
    }
    n
}

/// Lower-cased tag name of a tag body (the bytes between `<` and `>`).
pub fn tag_name(tag: &[u8]) -> Vec<u8> {
    let tag = tag.strip_prefix(b"/").unwrap_or(tag);
    tag.iter()
        .take_while(|b| b.is_ascii_alphanumeric())
        .map(|b| b.to_ascii_lowercase())
        .collect()
}

/// Does this tag name break the current line?
pub fn is_block_tag(name: &[u8]) -> bool {
    matches!(
        name,
        b"p" | b"br"
            | b"div"
            | b"h1"
            | b"h2"
            | b"h3"
            | b"h4"
            | b"h5"
            | b"h6"
            | b"li"
            | b"tr"
            | b"table"
            | b"hr"
            | b"title"
            | b"body"
            | b"html"
            | b"head"
            | b"ul"
            | b"ol"
    )
}

/// Value of attribute `name` in a tag body, entity-decoded.
/// Handles `name="v"`, `name='v'` and unquoted `name=v`.
pub fn attr_value(tag: &[u8], name: &str) -> Option<Vec<u8>> {
    let name = name.as_bytes();
    let n = tag.len();
    // Skip the tag name itself.
    let mut i = tag.iter().position(|b| b.is_ascii_whitespace())?;
    while i < n {
        while i < n && (tag[i].is_ascii_whitespace() || tag[i] == b'/') {
            i += 1;
        }
        let ks = i;
        while i < n && !tag[i].is_ascii_whitespace() && tag[i] != b'=' && tag[i] != b'/' {
            i += 1;
        }
        let key = &tag[ks..i];
        while i < n && tag[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value: &[u8] = &[];
        if i < n && tag[i] == b'=' {
            i += 1;
            while i < n && tag[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < n && (tag[i] == b'"' || tag[i] == b'\'') {
                let q = tag[i];
                i += 1;
                let vs = i;
                while i < n && tag[i] != q {
                    i += 1;
                }
                value = &tag[vs..i];
                i = (i + 1).min(n);
            } else {
                let vs = i;
                while i < n && !tag[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = &tag[vs..i];
            }
        }
        if !key.is_empty() && key.eq_ignore_ascii_case(name) {
            return Some(decode_entities(value));
        }
        if key.is_empty() && i < n {
            i += 1; // guarantee progress on stray bytes such as a lone '='
        }
    }
    None
}

// ─── Page buffer ─────────────────────────────────────────────────────

/// Fixed-size page of text lines with safe, resettable line building.
pub struct Page {
    pub lines: [[u8; LINE_LEN]; MAX_LINES],
    pub line_count: usize,
    /// Set when content was dropped because the page or a line limit was hit.
    pub truncated: bool,
    cur: [u8; LINE_LEN],
    cur_len: usize,
}

impl Page {
    pub const fn new() -> Self {
        Self {
            lines: [[0; LINE_LEN]; MAX_LINES],
            line_count: 0,
            truncated: false,
            cur: [0; LINE_LEN],
            cur_len: 0,
        }
    }

    /// Forget the previous page. Called at the start of every render.
    pub fn reset(&mut self) {
        self.line_count = 0;
        self.truncated = false;
        self.cur = [0; LINE_LEN];
        self.cur_len = 0;
    }

    /// True once no more lines can be stored.
    pub fn is_full(&self) -> bool {
        self.line_count >= MAX_LINES
    }

    /// Finish the current line (no-op if it is empty) and start a fresh one.
    pub fn break_line(&mut self) {
        if self.cur_len > 0 {
            if self.line_count < MAX_LINES {
                self.lines[self.line_count] = self.cur;
                self.line_count += 1;
            } else {
                self.truncated = true;
            }
        }
        // Always reset: stale bytes/length here were the O4.3 overrun.
        self.cur = [0; LINE_LEN];
        self.cur_len = 0;
    }

    /// Append one printable byte, wrapping at `LINE_LEN`. Runs of spaces are
    /// collapsed and leading spaces dropped.
    pub fn push_byte(&mut self, b: u8) {
        if self.is_full() {
            if b != b' ' {
                self.truncated = true;
            }
            return;
        }
        if b == b' ' && (self.cur_len == 0 || self.cur[self.cur_len - 1] == b' ') {
            return;
        }
        if self.cur_len >= LINE_LEN {
            self.break_line();
            if self.is_full() {
                self.truncated = true;
                return;
            }
            if b == b' ' {
                return;
            }
        }
        self.cur[self.cur_len] = b;
        self.cur_len += 1;
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.push_byte(b);
        }
    }

    /// Line `idx` as text (without NUL padding); empty if out of range.
    pub fn line_str(&self, idx: usize) -> &str {
        if idx >= self.line_count {
            return "";
        }
        let l = &self.lines[idx];
        let end = l.iter().position(|&b| b == 0).unwrap_or(LINE_LEN);
        core::str::from_utf8(&l[..end]).unwrap_or("")
    }

    /// Strip HTML into `self`. Never panics, whatever the input.
    pub fn render_html(&mut self, html: &[u8]) {
        self.reset();
        let n = html.len();
        let mut i = 0usize;
        while i < n {
            let c = html[i];
            if c == b'<' {
                let j = tag_end(html, i);
                let body_end = j.min(n);
                let tag = &html[(i + 1).min(body_end)..body_end];
                let name = tag_name(tag);
                if is_block_tag(&name) {
                    self.break_line();
                } else if name == b"img" {
                    if let Some(alt) = attr_value(tag, "alt") {
                        if !alt.is_empty() {
                            self.push_byte(b' ');
                            self.push_byte(b'[');
                            for &b in &alt {
                                self.push_byte(printable(b));
                            }
                            self.push_byte(b']');
                            self.push_byte(b' ');
                        }
                    }
                }
                i = j.saturating_add(1);
                continue;
            }
            if c == b'&' {
                if let Some((b, used)) = decode_entity(&html[i..]) {
                    self.push_byte(b);
                    i += used;
                    continue;
                }
            }
            if c == b'\n' {
                self.break_line();
            } else if c == b'\t' {
                self.push_byte(b' ');
            } else if (0x20..0x7F).contains(&c) {
                self.push_byte(c);
            }
            i += 1;
        }
        self.break_line();
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new()
    }
}

fn printable(b: u8) -> u8 {
    if (0x20..0x7F).contains(&b) {
        b
    } else {
        b' '
    }
}

// ─── Navigation helpers ──────────────────────────────────────────────

/// Why a URL was rejected before touching the network.
#[derive(Debug, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    TooLong,
    BadChar,
}

impl UrlError {
    pub fn message(&self) -> &'static str {
        match self {
            UrlError::Empty => "Empty URL",
            UrlError::TooLong => "URL too long",
            UrlError::BadChar => "URL contains invalid characters",
        }
    }
}

/// Validate a URL before handing it to the network device.
pub fn validate_url(url: &str) -> Result<(), UrlError> {
    if url.trim().is_empty() {
        return Err(UrlError::Empty);
    }
    if url.len() > MAX_URL_BYTES {
        return Err(UrlError::TooLong);
    }
    if url.bytes().any(|b| b < 0x21 || b == 0x7F) {
        return Err(UrlError::BadChar);
    }
    Ok(())
}

/// Back/forward history. `pos` is the index of the current entry.
#[derive(Default)]
pub struct History {
    entries: Vec<String>,
    pos: usize,
}

impl History {
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            pos: 0,
        }
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.pos).map(|s| s.as_str())
    }

    /// Record a fresh navigation: drops any forward entries.
    pub fn visit(&mut self, url: &str) {
        if !self.entries.is_empty() {
            self.entries.truncate(self.pos + 1);
        }
        self.entries.push(String::from(url));
        if self.entries.len() > MAX_HISTORY {
            self.entries.remove(0);
        }
        self.pos = self.entries.len() - 1;
    }

    /// Step back; returns the URL to load, or `None` at the start.
    pub fn back(&mut self) -> Option<String> {
        if self.pos == 0 || self.entries.is_empty() {
            return None;
        }
        self.pos -= 1;
        Some(self.entries[self.pos].clone())
    }

    /// Step forward; returns the URL to load, or `None` at the end.
    pub fn forward(&mut self) -> Option<String> {
        if self.pos + 1 >= self.entries.len() {
            return None;
        }
        self.pos += 1;
        Some(self.entries[self.pos].clone())
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec;

    fn render(html: &[u8]) -> Page {
        let mut p = Page::new();
        p.render_html(html);
        p
    }

    fn text_of(p: &Page) -> Vec<String> {
        (0..p.line_count)
            .map(|i| String::from(p.line_str(i)))
            .collect()
    }

    // --- the O4.3 panic ---

    #[test]
    fn long_line_wraps_instead_of_overrunning() {
        // 300 bytes with no break: the old code indexed line[110] and panicked.
        let html = vec![b'a'; 300];
        let p = render(&html);
        assert_eq!(p.line_count, 3);
        assert_eq!(p.line_str(0).len(), LINE_LEN);
        assert_eq!(p.line_str(1).len(), LINE_LEN);
        assert_eq!(p.line_str(2).len(), 300 - 2 * LINE_LEN);
        assert!(!p.truncated);
    }

    #[test]
    fn exactly_line_len_then_one_more() {
        let mut html = vec![b'x'; LINE_LEN];
        html.push(b'y');
        let p = render(&html);
        assert_eq!(text_of(&p).len(), 2);
        assert_eq!(p.line_str(1), "y");
    }

    #[test]
    fn lines_are_not_repeated_between_breaks() {
        // The old buffer was never reset, so every later line re-emitted the first.
        let p = render(b"<p>one</p><p>two</p><p>three</p>");
        assert_eq!(text_of(&p), ["one", "two", "three"]);
    }

    #[test]
    fn second_render_does_not_inherit_first_page() {
        let mut p = Page::new();
        p.render_html(b"<p>first page</p><p>more</p>");
        assert_eq!(p.line_count, 2);
        p.render_html(b"second");
        assert_eq!(text_of(&p), ["second"]);
        assert!(!p.truncated);
    }

    #[test]
    fn repeated_requests_with_a_dirty_line_buffer() {
        let mut p = Page::new();
        // Leave a half-built line behind, as a failed/odd render would.
        p.push_bytes(b"leftover");
        p.render_html(b"clean");
        assert_eq!(text_of(&p), ["clean"]);
    }

    #[test]
    fn oversized_response_truncates_and_reports() {
        let mut html = Vec::new();
        for i in 0..(MAX_LINES + 50) {
            html.extend_from_slice(format!("<p>line {}</p>", i).as_bytes());
        }
        let p = render(&html);
        assert_eq!(p.line_count, MAX_LINES);
        assert!(p.truncated);
        assert_eq!(p.line_str(0), "line 0");
    }

    #[test]
    fn huge_single_blob_does_not_panic() {
        let html = vec![b'z'; 200_000];
        let p = render(&html);
        assert_eq!(p.line_count, MAX_LINES);
        assert!(p.truncated);
    }

    #[test]
    fn exactly_full_page_is_not_truncated() {
        let mut html = Vec::new();
        for _ in 0..MAX_LINES {
            html.extend_from_slice(b"<p>x</p>");
        }
        let p = render(&html);
        assert_eq!(p.line_count, MAX_LINES);
        assert!(!p.truncated);
    }

    #[test]
    fn hostile_inputs_never_panic() {
        let cases: [&[u8]; 14] = [
            b"",
            b"<",
            b">",
            b"&",
            b"&#",
            b"&#x",
            b"&#;",
            b"<a href=\"unterminated",
            b"<!--",
            b"<!-- never closed",
            b"<img alt=",
            b"<img alt='&#xFFFFFFFFFF;'>",
            b"\x00\xff\xfe\x80<\x01>&\x7f;",
            b"&&&&&&&&&&&&&&&&&&&&&&&&;;;;;;&#99999999999999999999;",
        ];
        for c in cases {
            let _ = render(c);
        }
        // Every prefix of a nasty document.
        let doc: &[u8] = b"<html><!-- c --><a href=\"x>y\" title='&lt;'>t&amp;&#65;&#x42;</a><img alt=\"a&quot;b\"></html>";
        for k in 0..=doc.len() {
            let _ = render(&doc[..k]);
        }
    }

    // --- entity decoding ---

    #[test]
    fn named_entities_decode() {
        assert_eq!(decode_entity(b"&amp;"), Some((b'&', 5)));
        assert_eq!(decode_entity(b"&lt;"), Some((b'<', 4)));
        assert_eq!(decode_entity(b"&gt;"), Some((b'>', 4)));
        assert_eq!(decode_entity(b"&quot;"), Some((b'"', 6)));
        assert_eq!(decode_entity(b"&apos;"), Some((b'\'', 6)));
        assert_eq!(decode_entity(b"&nbsp;"), Some((b' ', 6)));
    }

    #[test]
    fn numeric_entities_decode() {
        assert_eq!(decode_entity(b"&#65;"), Some((b'A', 5)));
        assert_eq!(decode_entity(b"&#34;"), Some((b'"', 5)));
        assert_eq!(decode_entity(b"&#39;"), Some((b'\'', 5)));
        assert_eq!(decode_entity(b"&#x41;"), Some((b'A', 6)));
        assert_eq!(decode_entity(b"&#X7e;"), Some((b'~', 6)));
        assert_eq!(decode_entity(b"&#160;"), Some((b' ', 6)));
        assert_eq!(decode_entity(b"&#xA0;"), Some((b' ', 6)));
    }

    #[test]
    fn unrepresentable_numeric_entities_become_question_mark() {
        assert_eq!(decode_entity(b"&#8364;"), Some((b'?', 7)));
        assert_eq!(decode_entity(b"&#0;"), Some((b'?', 4)));
        assert_eq!(decode_entity(b"&#x10FFFF;"), Some((b'?', 10)));
        assert_eq!(decode_entity(b"&#xFFFFFFFF;"), Some((b'?', 12)));
        assert_eq!(decode_entity(b"&#99999999;"), Some((b'?', 11)));
        // Longer than MAX_ENTITY_LEN: not an entity, left as literal text.
        assert_eq!(decode_entity(b"&#99999999999;"), None);
    }

    #[test]
    fn malformed_entities_are_left_alone() {
        assert_eq!(decode_entity(b"&"), None);
        assert_eq!(decode_entity(b"&;"), None);
        assert_eq!(decode_entity(b"&#;"), None);
        assert_eq!(decode_entity(b"&#x;"), None);
        assert_eq!(decode_entity(b"&#xZZ;"), None);
        assert_eq!(decode_entity(b"&unknown;"), None);
        assert_eq!(decode_entity(b"&amp"), None);
        assert_eq!(decode_entity(b"AT&T rocks"), None);
    }

    #[test]
    fn entities_in_text_regression() {
        // Old code turned ANY "&xxxxx" into "&" and skipped 5 bytes.
        let p = render(b"Tom &amp; Jerry &lt;3 &gt; &quot;hi&quot; it&apos;s&nbsp;ok &#65;&#x42;");
        assert_eq!(text_of(&p), ["Tom & Jerry <3 > \"hi\" it's ok AB"]);
    }

    #[test]
    fn bare_ampersand_text_survives() {
        let p = render(b"AT&T and R&D and a & b");
        assert_eq!(text_of(&p), ["AT&T and R&D and a & b"]);
    }

    #[test]
    fn entity_at_end_of_input() {
        // The old `i + 5 < n` guard dropped entities in the last 5 bytes.
        assert_eq!(text_of(&render(b"x &amp;")), ["x &"]);
        assert_eq!(text_of(&render(b"&lt;")), ["<"]);
    }

    #[test]
    fn decode_entities_helper() {
        assert_eq!(
            decode_entities(b"a&amp;b&#x21;&bogus;&"),
            b"a&b!&bogus;&".to_vec()
        );
    }

    // --- attributes ---

    #[test]
    fn attr_values_are_entity_decoded() {
        let tag = b"img src=\"a.png\" alt=\"Tom &amp; &quot;Jerry&quot; &#x41;\"";
        assert_eq!(
            attr_value(tag, "alt").unwrap(),
            b"Tom & \"Jerry\" A".to_vec()
        );
        assert_eq!(attr_value(tag, "src").unwrap(), b"a.png".to_vec());
    }

    #[test]
    fn attr_quote_styles() {
        assert_eq!(
            attr_value(b"a href='x&lt;y'", "href").unwrap(),
            b"x<y".to_vec()
        );
        assert_eq!(
            attr_value(b"a href=plain&amp;x", "href").unwrap(),
            b"plain&x".to_vec()
        );
        assert_eq!(attr_value(b"a HREF=\"U\"", "href").unwrap(), b"U".to_vec());
        assert_eq!(
            attr_value(b"a disabled href=\"u\"", "href").unwrap(),
            b"u".to_vec()
        );
        assert_eq!(attr_value(b"a href", "href").unwrap(), Vec::<u8>::new());
        assert!(attr_value(b"a href=\"u\"", "alt").is_none());
        assert!(attr_value(b"br", "alt").is_none());
    }

    #[test]
    fn attr_parser_terminates_on_junk() {
        for t in [
            &b"a ="[..],
            b"a = = =",
            b"a \"",
            b"a ' x",
            b"a /////",
            b"a =\"",
            b"a  ",
        ] {
            let _ = attr_value(t, "href");
        }
    }

    #[test]
    fn img_alt_is_rendered_decoded() {
        let p = render(b"see <img src=x alt=\"A &amp; B\"> here");
        assert_eq!(text_of(&p), ["see [A & B] here"]);
    }

    // --- tags ---

    #[test]
    fn gt_inside_quoted_attribute_does_not_leak_text() {
        let p = render(b"<a title=\"1 > 0\" href='>'>link</a>");
        assert_eq!(text_of(&p), ["link"]);
    }

    #[test]
    fn comments_are_skipped() {
        let p = render(b"a<!-- hidden > text -->b");
        assert_eq!(text_of(&p), ["ab"]);
        let p = render(b"a<!-- never closed");
        assert_eq!(text_of(&p), ["a"]);
    }

    #[test]
    fn block_tags_break_lines_case_insensitively() {
        let p = render(b"<H1>Title</H1><P>para<BR>two</P>");
        assert_eq!(text_of(&p), ["Title", "para", "two"]);
    }

    #[test]
    fn whitespace_collapses() {
        let p = render(b"  a   b \t\t c  ");
        assert_eq!(text_of(&p), ["a b c "]);
    }

    #[test]
    fn non_ascii_and_controls_are_dropped() {
        let p = render(b"a\x00b\x01c\xc3\xa9d\x7f");
        assert_eq!(text_of(&p), ["abcd"]);
    }

    // --- url + history ---

    #[test]
    fn url_validation() {
        assert_eq!(validate_url("http://example.com"), Ok(()));
        assert_eq!(validate_url(""), Err(UrlError::Empty));
        assert_eq!(validate_url("   "), Err(UrlError::Empty));
        assert_eq!(validate_url("http://a b"), Err(UrlError::BadChar));
        assert_eq!(validate_url("http://a\nb"), Err(UrlError::BadChar));
        let long = format!("http://{}", "a".repeat(MAX_URL_BYTES));
        assert_eq!(validate_url(&long), Err(UrlError::TooLong));
        let ok = format!("http://{}", "a".repeat(MAX_URL_BYTES - 7));
        assert_eq!(validate_url(&ok), Ok(()));
    }

    #[test]
    fn history_back_forward_keeps_entries() {
        let mut h = History::new();
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), None);
        h.visit("A");
        h.visit("B");
        h.visit("C");
        assert_eq!(h.back().as_deref(), Some("B"));
        assert_eq!(h.back().as_deref(), Some("A"));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward().as_deref(), Some("B"));
        assert_eq!(h.forward().as_deref(), Some("C"));
        assert_eq!(h.forward(), None);
        assert_eq!(
            h.entries().len(),
            3,
            "back/forward must not duplicate entries"
        );
    }

    #[test]
    fn history_visit_after_back_drops_forward() {
        let mut h = History::new();
        h.visit("A");
        h.visit("B");
        h.visit("C");
        h.back();
        h.back();
        h.visit("D");
        assert_eq!(h.entries(), ["A", "D"]);
        assert_eq!(h.current(), Some("D"));
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::new();
        for i in 0..(MAX_HISTORY * 3) {
            h.visit(&format!("u{}", i));
        }
        assert_eq!(h.entries().len(), MAX_HISTORY);
        assert_eq!(
            h.current(),
            Some(format!("u{}", MAX_HISTORY * 3 - 1).as_str())
        );
    }
}
