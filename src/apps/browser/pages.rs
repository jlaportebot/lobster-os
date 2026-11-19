//! Generated pages and status-line text. Pure string builders, host-tested.

use crate::net::fetch::{reason_phrase, FetchError};
use alloc::format;
use alloc::string::String;

/// Link target of the Retry button on an error page; handled by the browser
/// itself and never fetched.
pub const RETRY_HREF: &str = "lobster:retry";

/// Escape the characters that would be read as markup.
#[must_use]
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// HTML of the page shown when a request produced no response.
#[must_use]
pub fn error_page_html(url: &str, error: FetchError) -> String {
    format!(
        "<title>Cannot load page</title><h2>Cannot load this page</h2>\
         <p>{}</p><p><b>Address:</b> {}</p>\
         <p><a href=\"{}\">Retry</a></p>",
        escape_html(error.message()),
        escape_html(url),
        RETRY_HREF
    )
}

/// HTML of the page shown for links the browser cannot follow.
#[must_use]
pub fn unsupported_page_html(url: &str) -> String {
    format!(
        "<title>Unsupported link</title><h2>Unsupported link</h2>\
         <p>Only http and https addresses can be opened.</p><p><b>Address:</b> {}</p>",
        escape_html(url)
    )
}

/// Facts shown in the status line after a load.
pub struct LoadInfo {
    /// HTTP status, 0 for generated pages.
    pub status: u16,
    /// Body size in bytes.
    pub bytes: usize,
    /// Time to fetch, in milliseconds.
    pub load_ms: u64,
    /// Time to parse and lay out, in milliseconds.
    pub layout_ms: u64,
    /// Page was cut by a size cap.
    pub truncated: bool,
}

/// Status line text for a finished load.
#[must_use]
pub fn status_text(info: &LoadInfo) -> String {
    let mut out = String::new();
    if info.status != 0 {
        out.push_str(&format!("{}", info.status));
        let reason = reason_phrase(info.status);
        if !reason.is_empty() {
            out.push(' ');
            out.push_str(reason);
        }
        out.push_str(" - ");
    }
    out.push_str(&format!(
        "{} bytes - load {} ms - layout {} ms",
        info.bytes, info.load_ms, info.layout_ms
    ));
    if info.truncated {
        out.push_str(" - truncated");
    }
    out
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn escape_covers_markup_characters() {
        assert_eq!(
            escape_html("<a href=\"x\">&"),
            "&lt;a href=&quot;x&quot;&gt;&amp;"
        );
    }

    #[test]
    fn error_page_names_url_reason_and_retry() {
        let html = error_page_html("http://127.0.0.1:9/<x>", FetchError::Failed);
        assert!(html.contains("http://127.0.0.1:9/&lt;x&gt;"));
        assert!(html.contains(FetchError::Failed.message()));
        assert!(html.contains(RETRY_HREF));
        assert!(!html.contains("<x>"));
    }

    #[test]
    fn status_text_includes_code_size_and_timings() {
        let info = LoadInfo {
            status: 404,
            bytes: 12,
            load_ms: 30,
            layout_ms: 4,
            truncated: false,
        };
        assert_eq!(
            status_text(&info),
            "404 Not Found - 12 bytes - load 30 ms - layout 4 ms"
        );
    }

    #[test]
    fn generated_pages_have_no_status_code() {
        let info = LoadInfo {
            status: 0,
            bytes: 5,
            load_ms: 0,
            layout_ms: 1,
            truncated: true,
        };
        assert_eq!(
            status_text(&info),
            "5 bytes - load 0 ms - layout 1 ms - truncated"
        );
    }

    #[test]
    fn unknown_status_has_no_reason() {
        let info = LoadInfo {
            status: 418,
            bytes: 0,
            load_ms: 0,
            layout_ms: 0,
            truncated: false,
        };
        assert!(status_text(&info).starts_with("418 - 0 bytes"));
    }
}
