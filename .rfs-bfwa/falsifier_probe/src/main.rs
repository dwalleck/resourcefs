//! Throwaway C3 falsifier: does the real core grammar accept a canonical
//! `:cursor:` continuation on a `pr://` facts reference?
//!
//! Reads one full Path Reference per line on stdin and prints a JSON verdict
//! per line. No production code is modified.

use std::io::{self, BufRead};

use resourcefs_core::{PathReference, ProjectionSelector};

fn main() {
    let stdin = io::stdin();
    let mut verdicts = Vec::new();
    for line in stdin.lock().lines() {
        let reference = line.expect("stdin line");
        let reference = reference.trim();
        if reference.is_empty() {
            continue;
        }
        let parsed = PathReference::parse(reference.to_owned());
        let projection = parsed.as_ref().ok().and_then(|parsed| parsed.projection());
        let selector_kind = match projection {
            Some(selector) if selector.source_cursor().is_some() => "cursor",
            Some(selector) if selector.page_offset().is_some() => "page",
            Some(selector) if selector.line_selection().is_some() => "lines",
            Some(_) => "other",
            None => "none",
        };
        let cursor_value = projection
            .and_then(ProjectionSelector::source_cursor)
            .map(|cursor| cursor.as_str().to_owned());
        let error_category = parsed
            .as_ref()
            .err()
            .map(|error| format!("{:?}", error.category()));
        verdicts.push(format!(
            "{{\"reference_parse_ok\":{},\"selector_kind\":\"{}\",\"cursor_value\":{},\"error_category\":{}}}",
            parsed.is_ok(),
            selector_kind,
            cursor_value.map_or("null".to_owned(), |value| format!("\"{value}\"")),
            error_category.map_or("null".to_owned(), |value| format!("\"{value}\"")),
        ));
    }
    println!("[{}]", verdicts.join(","));
}
