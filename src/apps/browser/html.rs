//! Forgiving HTML tokenizer: tags, attributes, entities, comments, raw-text
//! elements. It never fails; anything it cannot parse is kept as text.
//!
//! Tokens borrow from the source, so tokenizing allocates nothing: the
//! kernel heap is a first-fit list whose cost grows with the number of live
//! blocks, and a page can contain tens of thousands of tokens.

use alloc::string::String;

/// Decode UTF-8, turning invalid sequences into U+FFFD.
#[must_use]
pub fn decode_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Names of U+00A0..=U+00FF, in code point order (the HTML Latin-1 set).
const LATIN1_NAMES: [&str; 96] = [
    "nbsp", "iexcl", "cent", "pound", "curren", "yen", "brvbar", "sect", "uml", "copy", "ordf",
    "laquo", "not", "shy", "reg", "macr", "deg", "plusmn", "sup2", "sup3", "acute", "micro",
    "para", "middot", "cedil", "sup1", "ordm", "raquo", "frac14", "frac12", "frac34", "iquest",
    "Agrave", "Aacute", "Acirc", "Atilde", "Auml", "Aring", "AElig", "Ccedil", "Egrave", "Eacute",
    "Ecirc", "Euml", "Igrave", "Iacute", "Icirc", "Iuml", "ETH", "Ntilde", "Ograve", "Oacute",
    "Ocirc", "Otilde", "Ouml", "times", "Oslash", "Ugrave", "Uacute", "Ucirc", "Uuml", "Yacute",
    "THORN", "szlig", "agrave", "aacute", "acirc", "atilde", "auml", "aring", "aelig", "ccedil",
    "egrave", "eacute", "ecirc", "euml", "igrave", "iacute", "icirc", "iuml", "eth", "ntilde",
    "ograve", "oacute", "ocirc", "otilde", "ouml", "divide", "oslash", "ugrave", "uacute", "ucirc",
    "uuml", "yacute", "thorn", "yuml",
];

/// Named references outside Latin-1 (and the ASCII markup characters).
const NAMED_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
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

/// First code point of the Latin-1 name table.
const LATIN1_FIRST: u32 = 0xA0;

/// Longest entity name worth looking at; keeps a stray `&` cheap.
const MAX_ENTITY_LEN: usize = 10;

/// Decode the reference body between `&` and `;`, if it names a character.
fn entity_char(body: &str) -> Option<char> {
    if let Some(num) = body.strip_prefix('#') {
        return numeric_entity(num);
    }
    if let Some(i) = LATIN1_NAMES.iter().position(|n| *n == body) {
        return char::from_u32(LATIN1_FIRST + i as u32);
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
    decode_entities_into(text, &mut out);
    out
}

/// Append `text` to `out` with character references replaced.
pub fn decode_entities_into(text: &str, out: &mut String) {
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
}

/// A lexical unit of the document, borrowing from the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok<'a> {
    /// `<name attr=...>` or `<name/>`.
    Start {
        /// Tag name as written.
        name: &'a str,
        /// Raw attribute text; iterate it with [`attrs`].
        attrs: &'a str,
        /// Written with a trailing slash.
        self_closing: bool,
    },
    /// `</name>`.
    End(&'a str),
    /// Character data exactly as written (entities not yet decoded).
    Text(&'a str),
}

/// Elements whose content is not markup.
fn is_raw_text(name: &str) -> bool {
    name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style")
}

/// Streaming tokenizer over one document.
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    /// Start tokenizing `src`.
    #[must_use]
    pub fn new(src: &'a str) -> Self {
        Lexer { src, pos: 0 }
    }

    fn peek(&self, ahead: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + ahead).copied()
    }

    fn starts_with_ci(&self, needle: &str) -> bool {
        let end = self.pos + needle.len();
        self.src
            .as_bytes()
            .get(self.pos..end)
            .is_some_and(|s| s.eq_ignore_ascii_case(needle.as_bytes()))
    }

    /// Advance past the next `needle` (or to EOF).
    fn skip_past(&mut self, needle: &str) {
        self.pos = match self.src[self.pos..].find(needle) {
            Some(i) => self.pos + i + needle.len(),
            None => self.src.len(),
        };
    }

    /// True when the `<` at `at` opens markup rather than being literal text.
    fn opens_markup(&self, at: usize) -> bool {
        let b = self.src.as_bytes();
        match b.get(at + 1) {
            Some(b'!' | b'?') => true,
            Some(b'/') => b.get(at + 2).is_some_and(u8::is_ascii_alphabetic),
            Some(c) => c.is_ascii_alphabetic(),
            None => false,
        }
    }

    fn text_token(&mut self) -> Tok<'a> {
        let start = self.pos;
        let mut at = start;
        loop {
            let found = self.src[at..].find('<').map(|i| at + i);
            match found {
                Some(i) if i == start || !self.opens_markup(i) => at = i + 1,
                Some(i) => {
                    self.pos = i;
                    return Tok::Text(&self.src[start..i]);
                }
                None => {
                    self.pos = self.src.len();
                    return Tok::Text(&self.src[start..]);
                }
            }
        }
    }

    /// Parse the markup at `<`; `None` when it produced no token.
    fn markup(&mut self) -> Option<Tok<'a>> {
        match self.peek(1) {
            Some(b'!') if self.starts_with_ci("<!--") => {
                self.pos += 4;
                self.skip_past("-->");
                None
            }
            Some(b'!' | b'?') => {
                self.skip_past(">");
                None
            }
            Some(b'/') => {
                self.pos += 2;
                Some(self.end_tag())
            }
            _ => {
                self.pos += 1;
                self.start_tag()
            }
        }
    }

    fn take_name(&mut self) -> &'a str {
        let start = self.pos;
        while let Some(b) = self.peek(0) {
            if b.is_ascii_whitespace() || b == b'/' || b == b'>' {
                break;
            }
            self.pos += 1;
        }
        &self.src[start..self.pos]
    }

    fn end_tag(&mut self) -> Tok<'a> {
        let name = self.take_name();
        self.skip_past(">");
        Tok::End(name)
    }

    fn start_tag(&mut self) -> Option<Tok<'a>> {
        let name = self.take_name();
        let attr_start = self.pos;
        let Some(close) = tag_end(self.src, attr_start) else {
            self.pos = self.src.len();
            return None;
        };
        let raw = &self.src[attr_start..close];
        self.pos = close + 1;
        let self_closing = raw.trim_end().ends_with('/');
        if is_raw_text(name) && !self_closing {
            self.skip_raw_text(name);
        }
        Some(Tok::Start {
            name,
            attrs: raw,
            self_closing,
        })
    }

    /// Leave `pos` on the `</name` that ends a script or style element.
    fn skip_raw_text(&mut self, name: &str) {
        while let Some(i) = self.src[self.pos..].find("</") {
            self.pos += i;
            let end = self.pos + 2 + name.len();
            let hit = self.src.as_bytes().get(self.pos + 2..end);
            if hit.is_some_and(|h| h.eq_ignore_ascii_case(name.as_bytes())) {
                return;
            }
            self.pos += 2;
        }
        self.pos = self.src.len();
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Tok<'a>;

    fn next(&mut self) -> Option<Tok<'a>> {
        while self.pos < self.src.len() {
            if self.peek(0) != Some(b'<') || !self.opens_markup(self.pos) {
                return Some(self.text_token());
            }
            if let Some(tok) = self.markup() {
                return Some(tok);
            }
        }
        None
    }
}

fn skip_space(b: &[u8], mut i: usize) -> usize {
    while b.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

/// One attribute starting at `i`: (name range, value range, next index).
/// `None` when the input ends inside a quoted value.
fn scan_attribute(
    b: &[u8],
    i: usize,
) -> Option<(core::ops::Range<usize>, core::ops::Range<usize>, usize)> {
    let mut j = i;
    while let Some(&c) = b.get(j) {
        if c.is_ascii_whitespace() || c == b'=' || c == b'>' || (c == b'/' && j > i) {
            break;
        }
        j += 1;
    }
    let name = i..j;
    let after_name = if j == i { (i + 1).min(b.len()) } else { j };
    let k = skip_space(b, after_name);
    if b.get(k) != Some(&b'=') {
        return Some((name, after_name..after_name, after_name));
    }
    let v = skip_space(b, k + 1);
    match b.get(v) {
        Some(&q) if q == b'"' || q == b'\'' => {
            let close = b[v + 1..].iter().position(|&c| c == q)? + v + 1;
            Some((name, v + 1..close, close + 1))
        }
        _ => {
            let mut e = v;
            while b
                .get(e)
                .is_some_and(|c| !c.is_ascii_whitespace() && *c != b'>')
            {
                e += 1;
            }
            Some((name, v..e, e))
        }
    }
}

/// Index of the `>` closing the tag whose attributes start at `from`.
fn tag_end(src: &str, from: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut i = from;
    loop {
        i = skip_space(b, i);
        match b.get(i)? {
            b'>' => return Some(i),
            b'/' => i += 1,
            _ => i = scan_attribute(b, i)?.2,
        }
    }
}

/// Iterator over the `(name, raw value)` pairs of a start tag's attributes.
pub struct AttrIter<'a> {
    raw: &'a str,
    pos: usize,
}

/// Iterate the attributes of [`Tok::Start::attrs`]; names keep their case,
/// values are not entity-decoded.
#[must_use]
pub fn attrs(raw: &str) -> AttrIter<'_> {
    AttrIter { raw, pos: 0 }
}

impl<'a> Iterator for AttrIter<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        let b = self.raw.as_bytes();
        loop {
            self.pos = skip_space(b, self.pos);
            match b.get(self.pos)? {
                b'/' => self.pos += 1,
                _ => break,
            }
        }
        let (name, value, next) = scan_attribute(b, self.pos)?;
        self.pos = next;
        Some((&self.raw[name], &self.raw[value]))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Attr {
        name: String,
        value: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Token {
        Start {
            name: String,
            attrs: Vec<Attr>,
            self_closing: bool,
        },
        End(String),
        Text(String),
    }

    /// Owned, normalised tokens (lowercase names, decoded values and text).
    fn tokenize(html: &str) -> Vec<Token> {
        Lexer::new(html)
            .map(|t| match t {
                Tok::Start {
                    name,
                    attrs: raw,
                    self_closing,
                } => Token::Start {
                    name: name.to_ascii_lowercase(),
                    attrs: attrs(raw)
                        .map(|(n, v)| Attr {
                            name: n.to_ascii_lowercase(),
                            value: decode_entities(v),
                        })
                        .collect(),
                    self_closing,
                },
                Tok::End(name) => Token::End(name.to_ascii_lowercase()),
                Tok::Text(text) => Token::Text(decode_entities(text)),
            })
            .collect()
    }

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
    fn latin1_names_cover_the_whole_range() {
        assert_eq!(
            decode_entities("na&iuml;ve &Eacute;t&eacute; &yuml; &frac12; &Uuml;"),
            "na\u{ef}ve \u{c9}t\u{e9} \u{ff} \u{bd} \u{dc}"
        );
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
