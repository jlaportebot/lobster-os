//! URL handling for the browser: address-bar normalisation, relative
//! resolution, scheme checks and fragment splitting. Pure string logic.

use alloc::string::String;

/// Longest URL the lobster-net device accepts.
pub const MAX_URL_BYTES: usize = 3800;

/// Why a URL was rejected before touching the network.
#[derive(Debug, PartialEq, Eq)]
pub enum UrlError {
    /// Nothing was typed.
    Empty,
    /// Longer than the device accepts.
    TooLong,
    /// Contains whitespace or control characters.
    BadChar,
}

impl UrlError {
    /// Message for the error page.
    #[must_use]
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

/// Turn address-bar text into a URL: `example.com` becomes `http://example.com`.
#[must_use]
pub fn normalize_input(text: &str) -> String {
    let t = text.trim();
    if t.contains("://") || t.is_empty() {
        return String::from(t);
    }
    let mut out = String::from("http://");
    out.push_str(t);
    out
}

/// Length of the `scheme:` prefix of `url` (colon included), when valid.
fn scheme_len(url: &str) -> Option<usize> {
    let mut chars = url.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    for (i, c) in chars {
        if c == ':' {
            return Some(i + 1);
        }
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
            return None;
        }
    }
    None
}

/// Scheme of `url` in lowercase, when it has one.
#[must_use]
pub fn scheme_of(url: &str) -> Option<String> {
    let n = scheme_len(url)?;
    Some(url[..n - 1].to_ascii_lowercase())
}

/// True for the only schemes the browser fetches.
#[must_use]
pub fn is_fetchable(url: &str) -> bool {
    matches!(scheme_of(url).as_deref(), Some("http" | "https"))
}

/// Split `url` into the part to fetch and the `#fragment` (without the hash).
#[must_use]
pub fn split_fragment(url: &str) -> (&str, Option<&str>) {
    match url.find('#') {
        Some(i) => (&url[..i], Some(&url[i + 1..])),
        None => (url, None),
    }
}

/// `scheme://authority` prefix and the path+query that follows it.
fn split_origin(url: &str) -> (&str, &str) {
    let after_scheme = scheme_len(url).map_or(0, |n| n + 2).min(url.len());
    let rest = &url[after_scheme..];
    match rest.find(['/', '?']) {
        Some(i) => url.split_at(after_scheme + i),
        None => (url, ""),
    }
}

/// Remove `.` and `..` segments from an absolute path.
fn remove_dot_segments(path: &str) -> String {
    let mut stack: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    let mut segments = path.split('/').peekable();
    while let Some(seg) = segments.next() {
        let last = segments.peek().is_none();
        match seg {
            "." => {}
            ".." => {
                stack.pop();
            }
            _ => stack.push(seg),
        }
        if last && (seg == "." || seg == "..") {
            stack.push("");
        }
    }
    let joined = stack.join("/");
    if joined.starts_with('/') {
        joined
    } else {
        alloc::format!("/{joined}")
    }
}

/// Resolve a path-bearing reference (no scheme, not `//`) against the base.
fn resolve_path(origin: &str, base_path: &str, href: &str) -> String {
    let (href_path, tail) = match href.find(['?', '#']) {
        Some(i) => href.split_at(i),
        None => (href, ""),
    };
    let merged = if href_path.starts_with('/') {
        String::from(href_path)
    } else {
        let dir = base_path.rfind('/').map_or("/", |i| &base_path[..=i]);
        alloc::format!("{dir}{href_path}")
    };
    alloc::format!("{origin}{}{tail}", remove_dot_segments(&merged))
}

/// Resolve `href` against the page URL `base`.
#[must_use]
pub fn resolve(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.is_empty() {
        return String::from(base);
    }
    let (base_nofrag, _) = split_fragment(base);
    if href.starts_with('#') {
        return alloc::format!("{base_nofrag}{href}");
    }
    if scheme_len(href).is_some() {
        return String::from(href);
    }
    let (origin, path_query) = split_origin(base_nofrag);
    if href.starts_with("//") {
        let scheme = scheme_of(base).unwrap_or_else(|| String::from("http"));
        return alloc::format!("{scheme}:{href}");
    }
    let base_path = path_query.split('?').next().unwrap_or("/");
    if href.starts_with('?') {
        let path = if base_path.is_empty() { "/" } else { base_path };
        return alloc::format!("{origin}{path}{href}");
    }
    resolve_path(origin, base_path, href)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    const BASE: &str = "http://example.com/a/b/page.html?x=1#top";

    #[test]
    fn resolves_the_documented_table() {
        let cases = [
            ("/x", "http://example.com/x"),
            ("x", "http://example.com/a/b/x"),
            ("../x", "http://example.com/a/x"),
            ("../../x", "http://example.com/x"),
            ("../../../../x", "http://example.com/x"),
            ("./x", "http://example.com/a/b/x"),
            ("?q=2", "http://example.com/a/b/page.html?q=2"),
            ("#frag", "http://example.com/a/b/page.html?x=1#frag"),
            ("//other.org/y", "http://other.org/y"),
            ("https://secure.org/z", "https://secure.org/z"),
            ("mailto:a@b.c", "mailto:a@b.c"),
            ("", BASE),
            ("x/y?z=1#f", "http://example.com/a/b/x/y?z=1#f"),
            ("/a/./b/../c", "http://example.com/a/c"),
        ];
        for (href, want) in cases {
            assert_eq!(resolve(BASE, href), want, "href {href}");
        }
    }

    #[test]
    fn resolves_against_a_bare_host() {
        assert_eq!(resolve("http://example.com", "x"), "http://example.com/x");
        assert_eq!(resolve("https://h.org", "?q"), "https://h.org/?q");
    }

    #[test]
    fn https_base_keeps_scheme_for_protocol_relative() {
        assert_eq!(resolve("https://a.org/p", "//b.org/q"), "https://b.org/q");
    }

    #[test]
    fn normalize_adds_http_to_bare_hosts() {
        assert_eq!(normalize_input("example.com"), "http://example.com");
        assert_eq!(normalize_input("  example.com/x "), "http://example.com/x");
        assert_eq!(normalize_input("localhost:8000"), "http://localhost:8000");
        assert_eq!(normalize_input("https://a.org"), "https://a.org");
        assert_eq!(normalize_input("ftp://a.org"), "ftp://a.org");
    }

    #[test]
    fn scheme_detection() {
        assert_eq!(scheme_of("HTTP://a").as_deref(), Some("http"));
        assert_eq!(scheme_of("mailto:x@y").as_deref(), Some("mailto"));
        assert_eq!(scheme_of("/path:colon"), None);
        assert_eq!(scheme_of("a/b:c"), None);
        assert_eq!(scheme_of("1http://x"), None);
    }

    #[test]
    fn only_http_and_https_are_fetchable() {
        assert!(is_fetchable("http://a"));
        assert!(is_fetchable("HTTPS://a"));
        assert!(!is_fetchable("ftp://a"));
        assert!(!is_fetchable("javascript:alert(1)"));
        assert!(!is_fetchable("a.org"));
    }

    #[test]
    fn fragment_split() {
        assert_eq!(split_fragment("http://a/b#c"), ("http://a/b", Some("c")));
        assert_eq!(split_fragment("http://a/b"), ("http://a/b", None));
        assert_eq!(split_fragment("http://a/b#"), ("http://a/b", Some("")));
    }

    #[test]
    fn validation_rejects_bad_urls() {
        assert_eq!(validate_url("  "), Err(UrlError::Empty));
        assert_eq!(validate_url("http://a b"), Err(UrlError::BadChar));
        let long = alloc::format!("http://{}", "a".repeat(MAX_URL_BYTES));
        assert_eq!(validate_url(&long), Err(UrlError::TooLong));
        assert_eq!(validate_url("http://a.org"), Ok(()));
    }
}
