//! Document model: a bounded tree built from the token stream.
//!
//! The builder repairs the usual authoring mistakes (unclosed `<p>`/`<li>`,
//! stray end tags, mis-nesting) without ever dropping text, and caps both the
//! nesting depth and the node count so hostile pages cannot exhaust memory.

use super::html::{tokenize, Attr, Token};
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

/// Node payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// An element with its attributes.
    Element {
        /// Element kind.
        tag: Tag,
        /// Source attributes.
        attrs: Vec<Attr>,
    },
    /// Character data.
    Text(String),
}

/// One tree node; children are indices into [`Document::nodes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// What the node is.
    pub kind: NodeKind,
    /// Child node indices in document order.
    pub children: Vec<usize>,
}

/// A parsed page.
#[derive(Debug, Default)]
pub struct Document {
    /// Node arena; index 0 is the root.
    pub nodes: Vec<Node>,
    /// Contents of `<title>`, whitespace-collapsed.
    pub title: String,
    /// The node cap was hit and the rest of the page ignored.
    pub truncated: bool,
}

impl Document {
    /// Value of attribute `name` on element `idx`.
    #[must_use]
    pub fn attr(&self, idx: usize, name: &str) -> Option<&str> {
        match &self.nodes.get(idx)?.kind {
            NodeKind::Element { attrs, .. } => attrs
                .iter()
                .find(|a| a.name == name)
                .map(|a| a.value.as_str()),
            NodeKind::Text(_) => None,
        }
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
struct Open {
    name: String,
    tag: Tag,
    /// Node created for this element; `None` once the depth cap flattened it.
    node: Option<usize>,
    /// Node that receives this element's children.
    child_parent: usize,
}

struct Builder {
    doc: Document,
    stack: Vec<Open>,
    real_depth: usize,
    title_node: Option<usize>,
}

impl Builder {
    fn new() -> Self {
        let root = Node {
            kind: NodeKind::Element {
                tag: Tag::Root,
                attrs: Vec::new(),
            },
            children: Vec::new(),
        };
        Self {
            doc: Document {
                nodes: alloc::vec![root],
                title: String::new(),
                truncated: false,
            },
            stack: Vec::new(),
            real_depth: 0,
            title_node: None,
        }
    }

    fn parent(&self) -> usize {
        self.stack.last().map_or(0, |o| o.child_parent)
    }

    /// Add a node under the current parent; false once the node cap is hit.
    fn add_node(&mut self, kind: NodeKind) -> Option<usize> {
        if self.doc.nodes.len() >= MAX_NODES {
            self.doc.truncated = true;
            return None;
        }
        let idx = self.doc.nodes.len();
        let parent = self.parent();
        self.doc.nodes.push(Node {
            kind,
            children: Vec::new(),
        });
        self.doc.nodes[parent].children.push(idx);
        Some(idx)
    }

    /// Pop open elements down to and including stack index `at`.
    fn close_to(&mut self, at: usize) {
        while self.stack.len() > at {
            if let Some(open) = self.stack.pop() {
                if open.node.is_some() {
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

    fn start(&mut self, name: String, attrs: Vec<Attr>, self_closing: bool) {
        let tag = classify(&name);
        if name == "body" {
            self.end("head");
        }
        self.apply_implied_end(tag);
        let is_void = VOID_ELEMENTS.contains(&name.as_str());
        let node = if self.real_depth < MAX_DEPTH || is_void {
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
            if tag == Tag::Hidden && name == "title" && self.title_node.is_none() {
                self.title_node = node;
            }
        }
        self.stack.push(Open {
            name,
            tag,
            node,
            child_parent,
        });
    }

    fn end(&mut self, name: &str) {
        if let Some(at) = self.stack.iter().rposition(|o| o.name == name) {
            self.close_to(at);
        }
    }

    fn text(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.add_node(NodeKind::Text(text));
    }

    fn finish(mut self) -> Document {
        if let Some(t) = self.title_node {
            let mut raw = String::new();
            collect_text(&self.doc, t, &mut raw);
            self.doc.title = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        }
        self.doc
    }
}

fn collect_text(doc: &Document, idx: usize, out: &mut String) {
    let mut todo = alloc::vec![idx];
    while let Some(i) = todo.pop() {
        if let NodeKind::Text(t) = &doc.nodes[i].kind {
            out.push_str(t);
        }
        todo.extend(doc.nodes[i].children.iter().rev());
    }
}

/// Parse `html` into a document.
#[must_use]
pub fn parse(html: &str) -> Document {
    let mut builder = Builder::new();
    for token in tokenize(html) {
        if builder.doc.truncated {
            break;
        }
        match token {
            Token::Start {
                name,
                attrs,
                self_closing,
            } => builder.start(name, attrs, self_closing),
            Token::End(name) => builder.end(&name),
            Token::Text(text) => builder.text(text),
        }
    }
    builder.finish()
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::format;

    fn tag_of(doc: &Document, idx: usize) -> Option<Tag> {
        match &doc.nodes[idx].kind {
            NodeKind::Element { tag, .. } => Some(*tag),
            NodeKind::Text(_) => None,
        }
    }

    fn text_of(doc: &Document, idx: usize) -> String {
        let mut out = String::new();
        if let NodeKind::Text(t) = &doc.nodes[idx].kind {
            out.push_str(t);
        }
        for &c in &doc.nodes[idx].children {
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
        let kids = &doc.nodes[0].children;
        assert_eq!(kids.len(), 3);
        assert!(kids.iter().all(|&k| tag_of(&doc, k) == Some(Tag::P)));
    }

    #[test]
    fn unclosed_list_items_become_siblings() {
        let doc = parse("<ul><li>a<li>b<li>c</ul>");
        let ul = doc.nodes[0].children[0];
        assert_eq!(doc.nodes[ul].children.len(), 3);
    }

    #[test]
    fn nested_lists_keep_inner_items_inside_the_outer_item() {
        let doc = parse("<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul>");
        let ul = doc.nodes[0].children[0];
        assert_eq!(doc.nodes[ul].children.len(), 2);
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
        let br = doc.nodes[0].children[1];
        assert_eq!(tag_of(&doc, br), Some(Tag::Br));
        assert!(doc.nodes[br].children.is_empty());
    }

    #[test]
    fn script_and_style_elements_are_hidden() {
        let doc = parse("<script>x</script><style>y</style>z");
        let first = doc.nodes[0].children[0];
        assert_eq!(tag_of(&doc, first), Some(Tag::Hidden));
        assert_eq!(all_text(&doc), "z");
    }

    #[test]
    fn table_cells_close_each_other() {
        let doc = parse("<table><tr><td>a<td>b<tr><td>c</table>");
        let table = doc.nodes[0].children[0];
        assert_eq!(doc.nodes[table].children.len(), 2);
        let row0 = doc.nodes[table].children[0];
        assert_eq!(doc.nodes[row0].children.len(), 2);
    }

    #[test]
    fn attributes_are_reachable() {
        let doc = parse("<a href=\"/x\" id=top>t</a>");
        let a = doc.nodes[0].children[0];
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
        let body = doc.nodes[0].children[1];
        assert_eq!(tag_of(&doc, body), Some(Tag::Div));
    }

    #[test]
    fn heading_levels_are_kept() {
        let doc = parse("<h1>a</h1><h4>b</h4>");
        assert_eq!(
            tag_of(&doc, doc.nodes[0].children[0]),
            Some(Tag::Heading(1))
        );
        assert_eq!(
            tag_of(&doc, doc.nodes[0].children[1]),
            Some(Tag::Heading(4))
        );
    }
}
