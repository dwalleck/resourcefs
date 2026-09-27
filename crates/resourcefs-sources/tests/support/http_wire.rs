//! HTTP/1.1 wire helpers shared by the two loopback TLS fixtures.
//!
//! `tls.rs` (the async router fixture the sources and MCP unit tests drive)
//! and `profile_tls.rs` (the thread-hosted fixture `rfs serve` reads through)
//! stay separate servers because their drivers differ, but they parse the same
//! request shape. Both include this file by `#[path]`, as they already include
//! `certificates.rs`, so the parsing rules cannot drift between them (rfs-yrvb).

/// Byte offset just past the `\r\n\r\n` that ends a request head, if present.
pub fn head_end(received: &[u8]) -> Option<usize> {
    received
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

/// The trimmed value of the first header named `name`, compared without case.
pub fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        line.split_once(':')
            .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
    })
}
