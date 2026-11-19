//! Document model: a bounded tree built from the token stream.
//!
//! Nodes live in one flat arena linked by child/sibling indices and all text
//! lives in one string, so a page costs a handful of heap blocks instead of
//! several per node (the kernel heap slows down with every live block).
//!
//! The builder repairs the usual authoring mistakes (unclosed `<p>`/`<li>`,
//! stray end tags, mis-nesting) without ever dropping text, and caps both the
//! nesting depth and the node count so hostile pages cannot exhaust memory.

use super::html::{attrs, decode_entities_into, Lexer, Tok};
use alloc::string::String;
use alloc::vec::Vec;

/// Deepest element nesting that creates nodes; deeper elements are flattened.
pub const MAX_DEPTH: usize = 256;
/// Most nodes a document may hold before it is cut off.
pub const MAX_NODES: usize = 60_000;

/// Element kinds the layout engine distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    /// Synthetic document root.
    Root,
    /// `<head>`, `<script>`, `<style>`, `<title>`: never rendered.
    Hidden,
    /// `<h1>`..`<h6>` with the level.
    Heading(u8),
    /// `<p>`.
    P,
    /// `<div>`, `<section>`, `<article>` and similar block containers.
    Div,
    /// `<br>`.
    Br,
    /// `<hr>`.
    Hr,
    /// `<ul>`.
    Ul,
    /// `<ol>`.
    Ol,
    /// `<li>`.
    Li,
    /// `<blockquote>`.
    Blockquote,
    /// `<pre>`.
    Pre,
    /// `<code>`, `<kbd>`, `<tt>`, `<samp>`.
    Code,
    /// `<a>`.
    A,
    /// `<b>`, `<strong>`.
    Bold,
    /// `<i>`, `<em>`, `<cite>`.
    Em,
    /// `<small>`.
    Small,
    /// `<table>`.
    Table,
    /// `<tr>`.
    Tr,
    /// `<td>`.
    Td,
    /// `<th>`.
    Th,
    /// `<img>`.
    Img,
    /// `<center>`.
    Center,
    /// Anything else: transparent inline container.
    Other,
}

/// Index meaning "no node".
pub const NO_NODE: u32 = u32::MAX;

/// A byte range of a shared text arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// First byte.
    pub start: u32,
    /// Length in bytes; [`Span::NONE`] marks an absent value.
    pub len: u32,
}

impl Span {
    /// No value.
    pub const NONE: Span = Span {
        start: 0,
        len: u32::MAX,
    };

    /// The text of the span inside `arena`; empty when absent or out of range.
    #[must_use]
    pub fn get(self, arena: &str) -> &str {
        if self.len == u32::MAX {
            return "";
        }
        arena
            .get(self.start as usize..(self.start + self.len) as usize)
            .unwrap_or("")
    }

    /// True when a value is present (possibly empty).
    #[must_use]
    pub fn is_some(self) -> bool {
        self.len != u32::MAX
    }
}

/// The attributes the layout engine reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attrs {
    /// `href`.
    pub href: Span,
    /// `id`.
    pub id: Span,
    /// `name`.
    pub name: Span,
    /// `alt`.
    pub alt: Span,
}

impl Attrs {
    const NONE: Attrs = Attrs {
        href: Span::NONE,
        id: Span::NONE,
        name: Span::NONE,
        alt: Span::NONE,
    };
}

/// Node payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// An element with its attributes.
    Element {
        /// Element kind.
        tag: Tag,
        /// The attributes layout cares about.
        attrs: Attrs,
    },
    /// Character data (span of [`Document::text`]).
    Text(Span),
}

/// One tree node, linked to its children and next sibling by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    /// What the node is.
    pub kind: NodeKind,
    /// First child, or [`NO_NODE`].
    pub first_child: u32,
    /// Last child, or [`NO_NODE`].
    pub last_child: u32,
    /// Next sibling, or [`NO_NODE`].
    pub next: u32,
}

/// A parsed page.
#[derive(Debug, Default)]
pub struct Document {
    /// Node arena; index 0 is the root.
    pub nodes: Vec<Node>,
    /// Decoded text and attribute values referenced by spans.
    pub text: String,
    /// Contents of `<title>`, whitespace-collapsed.
    pub title: String,
    /// The node cap was hit and the rest of the page ignored.
    pub truncated: bool,
}

/// Iterator over the children of a node.
pub struct Children<'a> {
    doc: &'a Document,
    next: u32,
}

impl Iterator for Children<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.next == NO_NODE {
            return None;
        }
        let idx = self.next as usize;
        self.next = self.doc.nodes.get(idx)?.next;
        Some(idx)
    }
}

impl Document {
    /// Children of node `idx` in document order.
    #[must_use]
    pub fn children(&self, idx: usize) -> Children<'_> {
        let first = self.nodes.get(idx).map_or(NO_NODE, |n| n.first_child);
        Children {
            doc: self,
            next: first,
        }
    }

    /// Text of a span.
    #[must_use]
    pub fn str(&self, span: Span) -> &str {
        span.get(&self.text)
    }

    /// Value of attribute `name` (`href`, `id`, `name` or `alt`) on element `idx`.
    #[must_use]
    pub fn attr(&self, idx: usize, name: &str) -> Option<&str> {
        let NodeKind::Element { attrs, .. } = self.nodes.get(idx)?.kind else {
            return None;
        };
        let span = match name {
            "href" => attrs.href,
            "id" => attrs.id,
            "name" => attrs.name,
            "alt" => attrs.alt,
            _ => return None,
        };
        span.is_some().then(|| self.str(span))
    }
}

/// Elements that never have content or an end tag.
const VOID_ELEMENTS: &[&str] = &[
    "br", "hr", "img", "meta", "link", "base", "input", "param", "source", "track", "wbr", "embed",
    "col", "area",
];

/// Map a lowercase tag name to its [`Tag`].
fn classify(name: &str) -> Tag {
    match name {
        "h1" => Tag::Heading(1),
        "h2" => Tag::Heading(2),
        "h3" => Tag::Heading(3),
        "h4" => Tag::Heading(4),
        "h5" => Tag::Heading(5),
        "h6" => Tag::Heading(6),
        "p" => Tag::P,
        "div" | "section" | "article" | "main" | "header" | "footer" | "nav" | "aside" | "form"
        | "fieldset" | "figure" | "figcaption" | "dl" | "dd" | "dt" | "address" | "details"
        | "summary" | "caption" | "body" | "html" => Tag::Div,
        "br" => Tag::Br,
        "hr" => Tag::Hr,
        "ul" | "menu" | "dir" => Tag::Ul,
        "ol" => Tag::Ol,
        "li" => Tag::Li,
        "blockquote" => Tag::Blockquote,
        "pre" | "listing" | "xmp" => Tag::Pre,
        "code" | "kbd" | "tt" | "samp" | "var" => Tag::Code,
        "a" => Tag::A,
        "b" | "strong" => Tag::Bold,
        "i" | "em" | "cite" | "dfn" => Tag::Em,
        "small" | "sub" | "sup" => Tag::Small,
        "table" => Tag::Table,
        "tr" => Tag::Tr,
        "td" => Tag::Td,
        "th" => Tag::Th,
        "img" => Tag::Img,
        "center" => Tag::Center,
        "head" | "title" | "script" | "style" | "template" | "svg" => Tag::Hidden,
        _ => Tag::Other,
    }
}

/// Inline containers a block start tag may close through.
fn is_inline(tag: Tag) -> bool {
    matches!(
        tag,
        Tag::A | Tag::Bold | Tag::Em | Tag::Small | Tag::Code | Tag::Other
    )
}

/// Block elements whose start tag closes an open paragraph.
fn closes_paragraph(tag: Tag) -> bool {
    matches!(
        tag,
        Tag::P
            | Tag::Div
            | Tag::Heading(_)
            | Tag::Ul
            | Tag::Ol
            | Tag::Li
            | Tag::Blockquote
            | Tag::Pre
            | Tag::Table
            | Tag::Hr
            | Tag::Center
    )
}

/// An element currently open while building.
struct Open<'a> {
    name: &'a str,
    tag: Tag,
    /// Node created for this element; `NO_NODE` once the depth cap flattened it.
    node: u32,
    /// Node that receives this element's children.
    child_parent: u32,
}

struct Builder<'a> {
    doc: Document,
    stack: Vec<Open<'a>>,
    real_depth: usize,
    title_node: u32,
}

/// Lowercase `name` into `buf` when it is short enough to be a known tag.
fn lowered<'b>(name: &str, buf: &'b mut [u8; MAX_KNOWN_NAME]) -> &'b str {
    if name.len() > MAX_KNOWN_NAME || !name.is_ascii() {
        return "";
    }
    for (dst, src) in buf.iter_mut().zip(name.bytes()) {
        *dst = src.to_ascii_lowercase();
    }
    core::str::from_utf8(&buf[..name.len()]).unwrap_or("")
}

/// The attributes the document keeps.
#[derive(Clone, Copy)]
enum AttrKey {
    Href,
    Id,
    Name,
    Alt,
}

impl AttrKey {
    fn from_name(name: &str) -> Option<Self> {
        let is = |n: &str| name.eq_ignore_ascii_case(n);
        if is("href") {
            Some(Self::Href)
        } else if is("id") {
            Some(Self::Id)
        } else if is("name") {
            Some(Self::Name)
        } else if is("alt") {
            Some(Self::Alt)
        } else {
            None
        }
    }

    fn slot(self, attrs: &mut Attrs) -> &mut Span {
        match self {
            Self::Href => &mut attrs.href,
            Self::Id => &mut attrs.id,
            Self::Name => &mut attrs.name,
            Self::Alt => &mut attrs.alt,
        }
    }
}

/// Longest tag name the classifier knows.
const MAX_KNOWN_NAME: usize = 12;

impl<'a> Builder<'a> {
    fn new(capacity: usize) -> Self {
        let root = Node {
            kind: NodeKind::Element {
                tag: Tag::Root,
                attrs: Attrs::NONE,
            },
            first_child: NO_NODE,
            last_child: NO_NODE,
            next: NO_NODE,
        };
        let mut nodes = Vec::with_capacity(capacity);
        nodes.push(root);
        Self {
            doc: Document {
                nodes,
                text: String::with_capacity(capacity * 4),
                title: String::new(),
                truncated: false,
            },
            stack: Vec::new(),
            real_depth: 0,
            title_node: NO_NODE,
        }
    }

    fn parent(&self) -> u32 {
        self.stack.last().map_or(0, |o| o.child_parent)
    }

    /// Add a node under the current parent; `None` once the node cap is hit.
    fn add_node(&mut self, kind: NodeKind) -> Option<u32> {
        if self.doc.nodes.len() >= MAX_NODES {
            self.doc.truncated = true;
            return None;
        }
        let idx = self.doc.nodes.len() as u32;
        let parent = self.parent() as usize;
        self.doc.nodes.push(Node {
            kind,
            first_child: NO_NODE,
            last_child: NO_NODE,
            next: NO_NODE,
        });
        let last = self.doc.nodes[parent].last_child;
        if last == NO_NODE {
            self.doc.nodes[parent].first_child = idx;
        } else {
            self.doc.nodes[last as usize].next = idx;
        }
        self.doc.nodes[parent].last_child = idx;
        Some(idx)
    }

    /// Pop open elements down to and including stack index `at`.
    fn close_to(&mut self, at: usize) {
        while self.stack.len() > at {
            if let Some(open) = self.stack.pop() {
                if open.node != NO_NODE {
                    self.real_depth -= 1;
                }
            }
        }
    }

    /// Index of the innermost open element matching `pred`, stopping early at
    /// elements for which `stop` holds.
    fn find_open(&self, pred: impl Fn(Tag) -> bool, stop: impl Fn(Tag) -> bool) -> Option<usize> {
        for (i, open) in self.stack.iter().enumerate().rev() {
            if pred(open.tag) {
                return Some(i);
            }
            if stop(open.tag) {
                return None;
            }
        }
        None
    }

    /// Close elements the new start tag implicitly ends.
    fn apply_implied_end(&mut self, tag: Tag) {
        let target = match tag {
            Tag::Li => self.find_open(|t| t == Tag::Li, |t| matches!(t, Tag::Ul | Tag::Ol)),
            Tag::Td | Tag::Th => self.find_open(
                |t| matches!(t, Tag::Td | Tag::Th),
                |t| matches!(t, Tag::Tr | Tag::Table),
            ),
            Tag::Tr => self.find_open(|t| t == Tag::Tr, |t| t == Tag::Table),
            t if closes_paragraph(t) => self.find_open(|t| t == Tag::P, |t| !is_inline(t)),
            _ => None,
        };
        if let Some(at) = target {
            self.close_to(at);
        }
    }

    fn push_value(&mut self, raw: &str) -> Span {
        let start = self.doc.text.len();
        decode_entities_into(raw, &mut self.doc.text);
        Span {
            start: start as u32,
            len: (self.doc.text.len() - start) as u32,
        }
    }

    fn read_attrs(&mut self, raw: &str) -> Attrs {
        let mut out = Attrs::NONE;
        for (name, value) in attrs(raw) {
            let Some(key) = AttrKey::from_name(name) else {
                continue;
            };
            if key.slot(&mut out).is_some() {
                continue;
            }
            let span = self.push_value(value);
            *key.slot(&mut out) = span;
        }
        out
    }

    fn start(&mut self, name: &'a str, raw_attrs: &str, self_closing: bool) {
        let mut buf = [0u8; MAX_KNOWN_NAME];
        let lower = lowered(name, &mut buf);
        let tag = classify(lower);
        if lower == "body" {
            self.end("head");
        }
        self.apply_implied_end(tag);
        let is_void = VOID_ELEMENTS.contains(&lower);
        let node = if self.real_depth < MAX_DEPTH || is_void {
            let attrs = self.read_attrs(raw_attrs);
            self.add_node(NodeKind::Element { tag, attrs })
        } else {
            None
        };
        if is_void || self_closing {
            return;
        }
        let child_parent = node.unwrap_or_else(|| self.parent());
        if node.is_some() {
            self.real_depth += 1;
            if tag == Tag::Hidden && lower == "title" && self.title_node == NO_NODE {
                self.title_node = child_parent;
            }
        }
        self.stack.push(Open {
            name,
            tag,
            node: node.unwrap_or(NO_NODE),
            child_parent,
        });
    }

    fn end(&mut self, name: &str) {
        if let Some(at) = self
            .stack
            .iter()
            .rposition(|o| o.name.eq_ignore_ascii_case(name))
        {
            self.close_to(at);
        }
    }

    fn text(&mut self, raw: &str) {
        if raw.is_empty() {
            return;
        }
        let span = self.push_value(raw);
        if span.len > 0 && self.add_node(NodeKind::Text(span)).is_none() {
            let new_len = span.start as usize;
            self.doc.text.truncate(new_len);
        }
    }

    fn finish(mut self) -> Document {
        if self.title_node != NO_NODE {
            let mut words = String::new();
            let mut todo = alloc::vec![self.title_node as usize];
            while let Some(i) = todo.pop() {
                if let NodeKind::Text(span) = self.doc.nodes[i].kind {
                    words.push_str(self.doc.str(span));
                }
                let kids: Vec<usize> = self.doc.children(i).collect();
                todo.extend(kids.into_iter().rev());
            }
            self.doc.title = words.split_whitespace().collect::<Vec<_>>().join(" ");
        }
        self.doc
    }
}

/// Parse `html` into a document.
#[must_use]
pub fn parse(html: &str) -> Document {
    let capacity = (html.len() / NODE_BYTES_ESTIMATE + 16).min(MAX_NODES);
    let mut builder = Builder::new(capacity);
    for token in Lexer::new(html) {
        if builder.doc.truncated {
            break;
        }
        match token {
            Tok::Start {
                name,
                attrs,
                self_closing,
            } => builder.start(name, attrs, self_closing),
            Tok::End(name) => builder.end(name),
            Tok::Text(text) => builder.text(text),
        }
    }
    builder.finish()
}

/// Rough source bytes per node, to size the arena up front.
const NODE_BYTES_ESTIMATE: usize = 24;

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::format;

    fn tag_of(doc: &Document, idx: usize) -> Option<Tag> {
        match doc.nodes[idx].kind {
            NodeKind::Element { tag, .. } => Some(tag),
            NodeKind::Text(_) => None,
        }
    }

    fn kids(doc: &Document, idx: usize) -> Vec<usize> {
        doc.children(idx).collect()
    }

    fn text_of(doc: &Document, idx: usize) -> String {
        let mut out = String::new();
        if let NodeKind::Text(t) = doc.nodes[idx].kind {
            out.push_str(doc.str(t));
        }
        for c in doc.children(idx) {
            out.push_str(&text_of(doc, c));
        }
        out
    }

    fn all_text(doc: &Document) -> String {
        text_of(doc, 0)
    }

    #[test]
    fn root_is_node_zero() {
        let doc = parse("hi");
        assert_eq!(tag_of(&doc, 0), Some(Tag::Root));
        assert_eq!(all_text(&doc), "hi");
    }

    #[test]
    fn title_is_captured_and_not_rendered_text() {
        let doc = parse("<html><head><title>  My \n Page </title></head><body>x</body></html>");
        assert_eq!(doc.title, "My Page");
    }

    #[test]
    fn unclosed_paragraphs_become_siblings() {
        let doc = parse("<p>one<p>two<p>three");
        let kids = &kids(&doc, 0);
        assert_eq!(kids.len(), 3);
        assert!(kids.iter().all(|&k| tag_of(&doc, k) == Some(Tag::P)));
    }

    #[test]
    fn unclosed_list_items_become_siblings() {
        let doc = parse("<ul><li>a<li>b<li>c</ul>");
        let ul = kids(&doc, 0)[0];
        assert_eq!(kids(&doc, ul).len(), 3);
    }

    #[test]
    fn nested_lists_keep_inner_items_inside_the_outer_item() {
        let doc = parse("<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul>");
        let ul = kids(&doc, 0)[0];
        assert_eq!(kids(&doc, ul).len(), 2);
    }

    #[test]
    fn stray_end_tags_are_ignored() {
        let doc = parse("a</div></p></b>b");
        assert_eq!(all_text(&doc), "ab");
    }

    #[test]
    fn misnested_formatting_keeps_all_text() {
        let doc = parse("<b>bold <i>both</b> italic</i> plain");
        assert_eq!(all_text(&doc), "bold both italic plain");
    }

    #[test]
    fn void_elements_have_no_children() {
        let doc = parse("a<br>b<hr>c<img src=x>d");
        assert_eq!(all_text(&doc), "abcd");
        let br = kids(&doc, 0)[1];
        assert_eq!(tag_of(&doc, br), Some(Tag::Br));
        assert!(kids(&doc, br).is_empty());
    }

    #[test]
    fn script_and_style_elements_are_hidden() {
        let doc = parse("<script>x</script><style>y</style>z");
        let first = kids(&doc, 0)[0];
        assert_eq!(tag_of(&doc, first), Some(Tag::Hidden));
        assert_eq!(all_text(&doc), "z");
    }

    #[test]
    fn table_cells_close_each_other() {
        let doc = parse("<table><tr><td>a<td>b<tr><td>c</table>");
        let table = kids(&doc, 0)[0];
        assert_eq!(kids(&doc, table).len(), 2);
        let row0 = kids(&doc, table)[0];
        assert_eq!(kids(&doc, row0).len(), 2);
    }

    #[test]
    fn attributes_are_reachable() {
        let doc = parse("<a href=\"/x\" id=top>t</a>");
        let a = kids(&doc, 0)[0];
        assert_eq!(doc.attr(a, "href"), Some("/x"));
        assert_eq!(doc.attr(a, "id"), Some("top"));
        assert_eq!(doc.attr(a, "name"), None);
    }

    #[test]
    fn nesting_is_capped_without_losing_text() {
        let html = format!("{}deep{}", "<div>".repeat(10_000), "</div>".repeat(10_000));
        let doc = parse(&html);
        assert!(
            doc.nodes.len() <= MAX_DEPTH + 3,
            "{} nodes",
            doc.nodes.len()
        );
        assert_eq!(all_text(&doc), "deep");
    }

    #[test]
    fn node_count_is_capped() {
        let html = "<p>x</p>".repeat(MAX_NODES);
        let doc = parse(&html);
        assert!(doc.nodes.len() <= MAX_NODES);
        assert!(doc.truncated);
    }

    #[test]
    fn body_start_closes_an_unclosed_head() {
        let doc = parse("<head><title>t</title><body>shown");
        assert_eq!(all_text(&doc), "tshown");
        let body = kids(&doc, 0)[1];
        assert_eq!(tag_of(&doc, body), Some(Tag::Div));
    }

    #[test]
    fn heading_levels_are_kept() {
        let doc = parse("<h1>a</h1><h4>b</h4>");
        assert_eq!(tag_of(&doc, kids(&doc, 0)[0]), Some(Tag::Heading(1)));
        assert_eq!(tag_of(&doc, kids(&doc, 0)[1]), Some(Tag::Heading(4)));
    }
}
