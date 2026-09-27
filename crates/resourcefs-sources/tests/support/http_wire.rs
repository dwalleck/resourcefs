//! HTTP/1.1 wire helpers shared by the two loopback TLS fixtures.
//!
//! `tls.rs` (the async router fixture the sources and MCP unit tests drive)
//! and `profile_tls.rs` (the thread-hosted fixture `rfs serve` reads through)
//! stay separate servers because their drivers differ, but they speak the same
//! request and response shape. Both include this file by `#[path]`, as they
//! already include `certificates.rs`, so finding the end of a request head,
//! reading its headers, and rendering a response head each have one
//! implementation (rfs-yrvb). Each server still owns its read loop, its size
//! ceiling, and what it does with a request it rejects.

/// Byte offset just past the `\r\n\r\n` that ends a request head, if present.
///
/// `from` is how many bytes an earlier call already scanned without finding
/// the terminator. The search resumes three bytes before it, in case the
/// terminator straddles the previous read, so a read loop that passes its
/// previous length scans each byte a bounded number of times.
pub fn head_end(received: &[u8], from: usize) -> Option<usize> {
    let start = from.saturating_sub(3).min(received.len());
    received[start..]
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| start + position + 4)
}

/// Every `name: value` header line of a request head, after its request line.
///
/// Names are returned as they arrived; values are trimmed. Lines without a
/// colon, including the blank line that ends the head, are not headers.
pub fn headers(head: &str) -> impl Iterator<Item = (&str, &str)> {
    head.lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name, value.trim()))
}

/// The value of the first header named `name`, compared without case.
pub fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    headers(head)
        .find_map(|(candidate, value)| candidate.eq_ignore_ascii_case(name).then_some(value))
}

/// Renders a `Connection: close` response head for `status`.
///
/// `headers` are written in order. `content_length` of `None` omits the
/// header, so the body is delimited by the connection closing.
pub fn response_head(
    status: &str,
    headers: &[(&str, &str)],
    content_length: Option<usize>,
) -> String {
    let mut rendered = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        rendered.push_str(name);
        rendered.push_str(": ");
        rendered.push_str(value);
        rendered.push_str("\r\n");
    }
    if let Some(length) = content_length {
        rendered.push_str(&format!("Content-Length: {length}\r\n"));
    }
    rendered.push_str("Connection: close\r\n\r\n");
    rendered
}
