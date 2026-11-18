//! Forgiving HTML tokenizer: tags, attributes, entities, comments, raw-text
//! elements. It never fails; anything it cannot parse is kept as text.

use alloc::string::String;
use alloc::vec::Vec;

/// One attribute of a start tag; the name is lowercase, the value decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    /// Lowercase attribute name.
    pub name: String,
    /// Entity-decoded value (empty for valueless attributes).
    pub value: String,
}

/// A lexical unit of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// `<name attr=...>` or `<name/>`.
    Start {
        /// Lowercase tag name.
        name: String,
        /// Attributes in source order.
        attrs: Vec<Attr>,
        /// Written with a trailing slash.
        self_closing: bool,
    },
    /// `</name>`.
    End(String),
    /// Character data with entities decoded; whitespace untouched.
    Text(String),
}

/// Decode UTF-8, turning invalid sequences into U+FFFD.
#[must_use]
pub fn decode_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Named character references the decoder knows.
const NAMED_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{a0}'),
    ("iexcl", '\u{a1}'),
    ("cent", '\u{a2}'),
    ("pound", '\u{a3}'),
    ("yen", '\u{a5}'),
    ("sect", '\u{a7}'),
    ("copy", '\u{a9}'),
    ("laquo", '\u{ab}'),
    ("not", '\u{ac}'),
    ("reg", '\u{ae}'),
    ("deg", '\u{b0}'),
    ("plusmn", '\u{b1}'),
    ("micro", '\u{b5}'),
    ("para", '\u{b6}'),
    ("middot", '\u{b7}'),
    ("raquo", '\u{bb}'),
    ("frac14", '\u{bc}'),
    ("frac12", '\u{bd}'),
    ("frac34", '\u{be}'),
    ("iquest", '\u{bf}'),
    ("times", '\u{d7}'),
    ("divide", '\u{f7}'),
    ("szlig", '\u{df}'),
    ("agrave", '\u{e0}'),
    ("aacute", '\u{e1}'),
    ("acirc", '\u{e2}'),
    ("auml", '\u{e4}'),
    ("aring", '\u{e5}'),
    ("ccedil", '\u{e7}'),
    ("egrave", '\u{e8}'),
    ("eacute", '\u{e9}'),
    ("ecirc", '\u{ea}'),
    ("euml", '\u{eb}'),
    ("iacute", '\u{ed}'),
    ("ntilde", '\u{f1}'),
    ("oacute", '\u{f3}'),
    ("ouml", '\u{f6}'),
    ("uacute", '\u{fa}'),
    ("uuml", '\u{fc}'),
    ("ndash", '\u{2013}'),
    ("mdash", '\u{2014}'),
    ("lsquo", '\u{2018}'),
    ("rsquo", '\u{2019}'),
    ("ldquo", '\u{201c}'),
    ("rdquo", '\u{201d}'),
    ("bull", '\u{2022}'),
    ("hellip", '\u{2026}'),
    ("trade", '\u{2122}'),
    ("larr", '\u{2190}'),
    ("uarr", '\u{2191}'),
    ("rarr", '\u{2192}'),
    ("darr", '\u{2193}'),
    ("minus", '\u{2212}'),
    ("euro", '\u{20ac}'),
];

/// Longest entity name worth looking at; keeps a stray `&` cheap.
const MAX_ENTITY_LEN: usize = 10;

/// Decode the reference body between `&` and `;`, if it names a character.
fn entity_char(body: &str) -> Option<char> {
    if let Some(num) = body.strip_prefix('#') {
        return numeric_entity(num);
    }
    NAMED_ENTITIES
        .iter()
        .find(|(name, _)| *name == body)
        .map(|(_, c)| *c)
}

fn numeric_entity(num: &str) -> Option<char> {
    let (digits, radix) = match num.strip_prefix(['x', 'X']) {
        Some(hex) => (hex, 16),
        None => (num, 10),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let value = u32::from_str_radix(digits, radix).unwrap_or(u32::MAX);
    Some(
        char::from_u32(value)
            .filter(|c| *c != '\0')
            .unwrap_or('\u{fffd}'),
    )
}

/// Replace character references in `text` with the characters they name.
#[must_use]
pub fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let mut end = rest.len().min(MAX_ENTITY_LEN + 2);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let window = &rest[1..end];
        let decoded = window
            .find(';')
            .and_then(|semi| entity_char(&window[..semi]).map(|c| (c, semi + 2)));
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Elements whose content is not markup.
fn is_raw_text(name: &str) -> bool {
    name == "script" || name == "style"
}

/// Byte-level scanner over the document.
struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    text: String,
    out: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn bytes(&self) -> &'a [u8] {
        self.src.as_bytes()
    }

    fn peek(&self, ahead: usize) -> Option<u8> {
        self.bytes().get(self.pos + ahead).copied()
    }

    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            let raw = core::mem::take(&mut self.text);
            self.out.push(Token::Text(decode_entities(&raw)));
        }
    }

    fn starts_with_ci(&self, needle: &str) -> bool {
        let end = self.pos + needle.len();
        self.bytes()
            .get(self.pos..end)
            .is_some_and(|s| s.eq_ignore_ascii_case(needle.as_bytes()))
    }

    /// Advance past the next `needle` (or to EOF); false when it was absent.
    fn skip_past(&mut self, needle: &str) -> bool {
        match self.src[self.pos..].find(needle) {
            Some(i) => {
                self.pos += i + needle.len();
                true
            }
            None => {
                self.pos = self.src.len();
                false
            }
        }
    }

    fn run(mut self) -> Vec<Token> {
        while self.pos < self.src.len() {
            if self.peek(0) == Some(b'<') {
                self.markup();
            } else {
                self.plain_text();
            }
        }
        self.flush_text();
        self.out
    }

    fn plain_text(&mut self) {
        let end = self.src[self.pos..]
            .find('<')
            .map_or(self.src.len(), |i| self.pos + i);
        self.text.push_str(&self.src[self.pos..end]);
        self.pos = end;
    }

    /// Handle the construct starting at `<`.
    fn markup(&mut self) {
        let next = self.peek(1);
        match next {
            Some(b'!') if self.starts_with_ci("<!--") => {
                self.flush_text();
                self.pos += 4;
                self.skip_past("-->");
            }
            Some(b'!' | b'?') => {
                self.flush_text();
                self.skip_past(">");
            }
            Some(b'/') if self.peek(2).is_some_and(|b| b.is_ascii_alphabetic()) => {
                self.flush_text();
                self.pos += 2;
                self.end_tag();
            }
            Some(b) if b.is_ascii_alphabetic() => {
                self.flush_text();
                self.pos += 1;
                self.start_tag();
            }
            _ => {
                self.text.push('<');
                self.pos += 1;
            }
        }
    }

    fn take_name(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.peek(0) {
            if b.is_ascii_whitespace() || b == b'/' || b == b'>' {
                break;
            }
            self.pos += 1;
        }
        self.src[start..self.pos].to_ascii_lowercase()
    }

    fn end_tag(&mut self) {
        let name = self.take_name();
        self.skip_past(">");
        self.out.push(Token::End(name));
    }

    fn start_tag(&mut self) {
        let name = self.take_name();
        let mut attrs = Vec::new();
        let mut self_closing = false;
        loop {
            self.skip_space();
            match self.peek(0) {
                None => return self.abandon_tag(),
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                Some(b'/') => {
                    self.pos += 1;
                    self_closing = self.peek(0) == Some(b'>');
                }
                Some(_) => match self.attribute() {
                    Some(attr) => attrs.push(attr),
                    None => return self.abandon_tag(),
                },
            }
        }
        let raw = is_raw_text(&name) && !self_closing;
        let closer = alloc::format!("</{name}");
        self.out.push(Token::Start {
            name,
            attrs,
            self_closing,
        });
        if raw {
            self.skip_raw_text(&closer);
        }
    }

    /// The input ended inside a tag: drop the unfinished tag and everything after.
    fn abandon_tag(&mut self) {
        self.pos = self.src.len();
    }

    fn skip_raw_text(&mut self, closer: &str) {
        while let Some(i) = self.src[self.pos..].find("</") {
            self.pos += i;
            if self.starts_with_ci(closer) {
                return;
            }
            self.pos += 2;
        }
        self.pos = self.src.len();
    }

    fn skip_space(&mut self) {
        while self.peek(0).is_some_and(|b| b.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    /// Parse one attribute; `None` when the input ends inside the tag.
    fn attribute(&mut self) -> Option<Attr> {
        let start = self.pos;
        while let Some(b) = self.peek(0) {
            if b.is_ascii_whitespace() || b == b'=' || b == b'>' || (b == b'/' && self.pos > start)
            {
                break;
            }
            self.pos += 1;
        }
        let name = self.src[start..self.pos].to_ascii_lowercase();
        if self.pos == start {
            self.pos += 1;
        }
        self.skip_space();
        if self.peek(0) != Some(b'=') {
            return Some(Attr {
                name,
                value: String::new(),
            });
        }
        self.pos += 1;
        self.skip_space();
        let value = self.attribute_value()?;
        Some(Attr {
            name,
            value: decode_entities(&value),
        })
    }

    fn attribute_value(&mut self) -> Option<String> {
        let quote = self.peek(0)?;
        if quote == b'"' || quote == b'\'' {
            self.pos += 1;
            let rel = self.src[self.pos..].find(quote as char)?;
            let value = String::from(&self.src[self.pos..self.pos + rel]);
            self.pos += rel + 1;
            return Some(value);
        }
        let start = self.pos;
        while self
            .peek(0)
            .is_some_and(|b| !b.is_ascii_whitespace() && b != b'>')
        {
            self.pos += 1;
        }
        Some(String::from(&self.src[start..self.pos]))
    }
}

/// Split `html` into tokens.
#[must_use]
pub fn tokenize(html: &str) -> Vec<Token> {
    Lexer {
        src: html,
        pos: 0,
        text: String::new(),
        out: Vec::new(),
    }
    .run()
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::vec;

    fn start(name: &str, attrs: &[(&str, &str)], self_closing: bool) -> Token {
        Token::Start {
            name: name.into(),
            attrs: attrs
                .iter()
                .map(|(n, v)| Attr {
                    name: (*n).into(),
                    value: (*v).into(),
                })
                .collect(),
            self_closing,
        }
    }

    fn text(s: &str) -> Token {
        Token::Text(s.into())
    }

    #[test]
    fn plain_tags_and_text() {
        assert_eq!(
            tokenize("<p>Hi</p>"),
            vec![start("p", &[], false), text("Hi"), Token::End("p".into())]
        );
    }

    #[test]
    fn tag_and_attribute_names_are_lowercased() {
        assert_eq!(
            tokenize("<A HREF='x'>"),
            vec![start("a", &[("href", "x")], false)]
        );
    }

    #[test]
    fn attribute_quoting_styles() {
        let t = tokenize(r#"<a href="a b" title='c d' id=e data-x hidden>"#);
        assert_eq!(
            t,
            vec![start(
                "a",
                &[
                    ("href", "a b"),
                    ("title", "c d"),
                    ("id", "e"),
                    ("data-x", ""),
                    ("hidden", "")
                ],
                false
            )]
        );
    }

    #[test]
    fn self_closing_forms() {
        assert_eq!(tokenize("<br/>"), vec![start("br", &[], true)]);
        assert_eq!(
            tokenize("<img src=a.png />"),
            vec![start("img", &[("src", "a.png")], true)]
        );
    }

    #[test]
    fn spaces_around_equals_are_allowed() {
        assert_eq!(
            tokenize("<a href = \"x\">"),
            vec![start("a", &[("href", "x")], false)]
        );
    }

    #[test]
    fn comments_doctype_and_cdata_are_dropped() {
        let t = tokenize("<!DOCTYPE html><!-- c <b> --><p>x<![CDATA[junk]]>y</p><?xml v?>");
        assert_eq!(
            t,
            vec![
                start("p", &[], false),
                text("x"),
                text("y"),
                Token::End("p".into())
            ]
        );
    }

    #[test]
    fn unterminated_comment_swallows_the_rest_only() {
        assert_eq!(tokenize("a<!-- b"), vec![text("a")]);
    }

    #[test]
    fn script_and_style_content_is_skipped() {
        let t = tokenize("<script>if (a<b) {x='</p>'}</script>ok<style>p{}</style>!");
        let texts: Vec<_> = t.iter().filter(|t| matches!(t, Token::Text(_))).collect();
        assert_eq!(texts, vec![&text("ok"), &text("!")]);
        assert!(t.contains(&Token::End("script".into())));
    }

    #[test]
    fn script_without_end_swallows_to_eof() {
        assert_eq!(
            tokenize("a<script>b c"),
            vec![text("a"), start("script", &[], false)]
        );
    }

    #[test]
    fn stray_less_than_is_text() {
        assert_eq!(tokenize("a < b <3 c"), vec![text("a < b <3 c")]);
    }

    #[test]
    fn truncated_tag_keeps_preceding_text() {
        assert_eq!(tokenize("hello <a href=\"x"), vec![text("hello ")]);
        assert_eq!(tokenize("hello <"), vec![text("hello <")]);
    }

    #[test]
    fn named_entities() {
        assert_eq!(
            decode_entities("&amp; &lt; &gt; &quot; &#39; &copy; &mdash;"),
            "& < > \" ' \u{a9} \u{2014}"
        );
        assert_eq!(decode_entities("a&nbsp;b"), "a\u{a0}b");
    }

    #[test]
    fn numeric_entities() {
        assert_eq!(
            decode_entities("&#8212;&#x2014;&#X41;&#65;"),
            "\u{2014}\u{2014}AA"
        );
    }

    #[test]
    fn bad_entities_degrade_gracefully() {
        assert_eq!(
            decode_entities("&#0;&#xD800;&#1114112;"),
            "\u{fffd}\u{fffd}\u{fffd}"
        );
        assert_eq!(decode_entities("&bogus; & x &amp"), "&bogus; & x &amp");
        assert_eq!(decode_entities("&#;&#x;"), "&#;&#x;");
    }

    #[test]
    fn entities_in_attribute_values_and_text_tokens() {
        let t = tokenize("<a href=\"?a=1&amp;b=2\">x &lt; y</a>");
        assert_eq!(t[0], start("a", &[("href", "?a=1&b=2")], false));
        assert_eq!(t[1], text("x < y"));
    }

    #[test]
    fn entity_window_respects_char_boundaries() {
        assert_eq!(
            decode_entities("&\u{fffd}\u{fffd}\u{fffd}\u{fffd}x"),
            "&\u{fffd}\u{fffd}\u{fffd}\u{fffd}x"
        );
    }

    #[test]
    fn invalid_utf8_becomes_replacement_char() {
        assert_eq!(decode_bytes(b"a\xffb\xc3"), "a\u{fffd}b\u{fffd}");
        assert_eq!(decode_bytes("h\u{e9}llo".as_bytes()), "h\u{e9}llo");
    }

    #[test]
    fn end_tag_with_junk_is_still_an_end_tag() {
        assert_eq!(tokenize("</p foo>"), vec![Token::End("p".into())]);
    }

    #[test]
    fn attribute_without_closing_quote_at_eof_is_dropped_not_panicking() {
        let _ = tokenize("<a href='x");
        let _ = tokenize("<a href=");
        let _ = tokenize("<a =>");
        let _ = tokenize("</");
        let _ = tokenize("<!");
    }
}
