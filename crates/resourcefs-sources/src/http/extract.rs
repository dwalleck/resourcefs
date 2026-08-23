//! Reader-mode extraction: a bounded HTML→Markdown pass owned by this crate.
//!
//! The signed spec chose an in-house extractor over a readability-style
//! heuristic for one reason: the Markdown produced here feeds a
//! content-derived Version Tag. A heuristic extractor's upgrade would change
//! its output, and therefore churn Version Tags for upstream documents that
//! never changed. This pass keeps a fixed tag subset and changes only when we
//! change it, so identical input always yields identical bytes.
//!
//! The corollary is deliberate: there is no boilerplate or navigation
//! stripping. A cluttered page yields cluttered Markdown.
//!
//! # Determinism is a construction rule, not an aspiration
//!
//! Canonical output must never be derived from a `Debug` rendering. The
//! `prove-it-prototype` stage recorded this as premise P4 after its first
//! canonicalization used `{:?}` on a `StrTendril` and emitted
//! `Tendril<UTF8>(inline: "html")` — embedding tendril's inline-versus-heap
//! storage discriminator, a value that flips with string length. Digests would
//! have been silently unstable for longer strings while looking correct for
//! short ones. Every string here therefore reaches the output through explicit
//! `Deref` (`&*tendril`), never through formatting of a library type, and the
//! golden corpus carries a case whose strings straddle that storage boundary
//! so a regression fails loudly instead of quietly.

use std::cell::RefCell;

use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
    states::RawKind,
};

/// Elements whose content never reaches the reader-mode output.
const DROPPED: &[&str] = &["script", "style", "svg", "head", "noscript", "template"];

/// Extracts reader-mode Markdown from a complete HTML document.
///
/// The caller guarantees the document is complete: [`super::HttpSubstrate`]
/// refuses an over-ceiling body outright rather than handing a truncated one
/// here, because extraction over truncated markup can silently drop or mangle
/// structure — an unclosed element swallows the visible content — and a
/// `bounded` flag cannot express "and possibly wrong".
pub fn extract_markdown(html: &str) -> String {
    let sink = MarkdownSink::default();
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(html.into());
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    tokenizer.sink.finish()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Unordered,
    Ordered(u64),
}

#[derive(Default)]
struct State {
    blocks: Vec<String>,
    line: String,
    prefix: String,
    skip: usize,
    pre: usize,
    lists: Vec<ListKind>,
    link_start: Option<usize>,
    link_href: Option<String>,
    row: Vec<String>,
    in_cell: bool,
}

#[derive(Default)]
struct MarkdownSink {
    state: RefCell<State>,
}

impl MarkdownSink {
    fn finish(&self) -> String {
        let mut state = self.state.borrow_mut();
        state.flush();
        let mut out = String::new();
        for block in &state.blocks {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(block);
        }
        out.push('\n');
        out
    }
}

impl State {
    /// Ends the current block, pushing it if it carries content.
    ///
    /// A pending prefix survives a flush that had nothing to write. `<li>` sets
    /// the marker and any block element opening immediately inside it — the
    /// very common `<li><p>text</p></li>` — flushes an empty line before the
    /// text arrives. Clearing unconditionally discarded the marker, so the item
    /// rendered as a bare paragraph; for `<ol>` the counter had already been
    /// consumed, so surviving items renumbered (1, 3, 5…) as well. Keeping the
    /// prefix until it is actually spent makes the marker independent of how
    /// the item's content is wrapped.
    fn flush(&mut self) {
        let body = self.line.trim_end();
        if !body.is_empty() {
            let mut block = String::with_capacity(self.prefix.len() + body.len());
            block.push_str(&self.prefix);
            block.push_str(body);
            self.blocks.push(block);
            self.prefix.clear();
        }
        self.line.clear();
    }

    /// Appends text, collapsing whitespace runs outside `<pre>`.
    fn push_text(&mut self, text: &str) {
        if self.pre > 0 {
            self.line.push_str(text);
            return;
        }
        // Whether a separator is needed before the first word depends on the
        // source, not on the buffer: `</a>.` must not gain a space before the
        // period, while `</a> and` must keep the one it had. Inserting a
        // separator unconditionally is the bug that produced `[link](…) .`.
        let leads_with_space = text.starts_with(char::is_whitespace);
        for (index, chunk) in text.split_whitespace().enumerate() {
            let separate = index > 0 || leads_with_space;
            if separate && !self.line.is_empty() && !self.line.ends_with(' ') {
                self.line.push(' ');
            }
            self.line.push_str(chunk);
        }
        // A trailing space in the source is meaningful between inline runs.
        if text.ends_with(char::is_whitespace) && !self.line.is_empty() && !self.line.ends_with(' ')
        {
            self.line.push(' ');
        }
    }

    /// Starts a new block with `prefix`, replacing any unspent one.
    ///
    /// Replacement rather than append, because a prefix now survives an empty
    /// flush: two consecutive empty list items would otherwise accumulate
    /// `"- - "`. The new block defines its own marker outright.
    fn begin_block(&mut self, prefix: &str) {
        self.flush();
        self.prefix.clear();
        self.prefix.push_str(prefix);
    }

    fn list_marker(&mut self) -> String {
        match self.lists.last_mut() {
            Some(ListKind::Ordered(counter)) => {
                *counter += 1;
                let marker = format!("{counter}. ");
                let depth = self.lists.len().saturating_sub(1);
                format!("{}{marker}", "  ".repeat(depth))
            }
            Some(ListKind::Unordered) => {
                let depth = self.lists.len().saturating_sub(1);
                format!("{}- ", "  ".repeat(depth))
            }
            None => "- ".to_owned(),
        }
    }
}

impl TokenSink for MarkdownSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut state = self.state.borrow_mut();
        match token {
            Token::TagToken(tag) => {
                let raw = handle_tag(&mut state, &tag);
                // A raw-text element's body is *text*, not markup, and only the
                // sink can tell the tokenizer so: the standalone `Tokenizer`
                // switches state solely on this return value. Returning
                // `Continue` for `<script>`/`<style>` left their bodies being
                // tokenized as markup, so `<script>var t = "<style>";</script>`
                // emitted a `<style>` start tag that pushed `skip` to 2 while
                // `</script>` only returned it to 1 — suppressing every
                // subsequent character token and silently truncating the
                // document to whatever preceded the script.
                if let Some(kind) = raw {
                    // `name` is cloned rather than formatted: the tokenizer
                    // needs the element name to find the matching end tag, and
                    // a `Debug` rendering here would carry tendril's storage
                    // discriminator into a control path (module note, P4).
                    return TokenSinkResult::RawData(kind);
                }
            }
            Token::CharacterTokens(text) if state.skip == 0 => {
                // Explicit deref to `&str`. Never `{:?}` — see the module note.
                state.push_text(&text);
            }
            // Doctypes, comments, NULs, parse errors and EOF contribute no
            // reader-mode content. Comments are dropped rather than rendered
            // so that markup invisible to a reader stays invisible here.
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

/// Handles one tag, returning the raw-text state the tokenizer must enter.
///
/// `Some(kind)` is returned only for a dropped element whose body HTML treats
/// as text rather than markup; the caller forwards it as
/// `TokenSinkResult::RawData`, which is the only way a sink can drive the
/// standalone tokenizer's state machine.
fn handle_tag(state: &mut State, tag: &Tag) -> Option<RawKind> {
    // `LocalName` derefs to `&str`; comparing the borrowed name keeps the
    // storage representation out of every decision made here.
    let name: &str = &tag.name;
    if DROPPED.contains(&name) {
        match tag.kind {
            TagKind::StartTag => {
                // The self-closing flag is deliberately ignored for raw-text
                // elements. HTML has no self-closing `<script>`: `<script/>`
                // opens an element whose body runs to the next `</script>`,
                // and honouring the flag let script text reach the output.
                // For a foreign element such as `<svg/>` the flag is
                // meaningful, and there is no raw-text state to enter.
                let raw = raw_kind(name);
                if raw.is_some() || !tag.self_closing {
                    state.skip += 1;
                }
                return raw;
            }
            TagKind::EndTag => state.skip = state.skip.saturating_sub(1),
        }
        return None;
    }
    if state.skip > 0 {
        return None;
    }
    match tag.kind {
        TagKind::StartTag => start_tag(state, tag, name),
        TagKind::EndTag => end_tag(state, name),
    }
    None
}

/// The tokenizer state a dropped element's body must be read in.
///
/// `<svg>`, `<head>`, `<noscript>` and `<template>` hold ordinary markup, so
/// they keep the default state and are skipped structurally.
const fn raw_kind(name: &str) -> Option<RawKind> {
    match name.as_bytes() {
        b"script" => Some(RawKind::ScriptData),
        b"style" => Some(RawKind::Rawtext),
        _ => None,
    }
}

fn start_tag(state: &mut State, tag: &Tag, name: &str) {
    match name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = name[1..].parse::<usize>().unwrap_or(1);
            state.begin_block(&format!("{} ", "#".repeat(level)));
        }
        "p" | "div" | "section" | "article" | "main" | "header" | "footer" | "blockquote" => {
            state.flush();
        }
        "br" => state.line.push_str("  \n"),
        "hr" => {
            state.flush();
            state.blocks.push("---".to_owned());
        }
        "ul" => {
            state.flush();
            state.lists.push(ListKind::Unordered);
        }
        "ol" => {
            state.flush();
            state.lists.push(ListKind::Ordered(0));
        }
        "li" => {
            let marker = state.list_marker();
            state.begin_block(&marker);
        }
        "pre" => {
            state.flush();
            state.pre += 1;
            state.blocks.push("```".to_owned());
        }
        "code" if state.pre == 0 => state.line.push('`'),
        "a" => {
            state.link_start = Some(state.line.len());
            state.link_href = href_of(tag);
        }
        "tr" => {
            state.flush();
            state.row.clear();
        }
        "td" | "th" => {
            state.in_cell = true;
            state.line.clear();
        }
        _ => {}
    }
}

fn end_tag(state: &mut State, name: &str) {
    match name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "li" | "blockquote" => state.flush(),
        "ul" | "ol" => {
            state.flush();
            state.lists.pop();
        }
        "pre" => {
            state.flush();
            state.pre = state.pre.saturating_sub(1);
            state.blocks.push("```".to_owned());
        }
        "code" if state.pre == 0 => state.line.push('`'),
        "a" => {
            if let (Some(start), Some(href)) = (state.link_start.take(), state.link_href.take())
                && start <= state.line.len()
            {
                let text = state.line.split_off(start);
                let text = text.trim().to_owned();
                if text.is_empty() {
                    state.line.push_str(&format!("[{href}]({href})"));
                } else {
                    state.line.push_str(&format!("[{text}]({href})"));
                }
            }
        }
        "td" | "th" => {
            let cell = state.line.trim().to_owned();
            state.row.push(cell);
            state.line.clear();
            state.in_cell = false;
        }
        "tr" if !state.row.is_empty() => {
            let row = format!("| {} |", state.row.join(" | "));
            state.blocks.push(row);
            state.row.clear();
        }
        _ => {}
    }
}

/// Returns a link's `href`, read through explicit field access.
fn href_of(tag: &Tag) -> Option<String> {
    tag.attrs
        .iter()
        .find(|attr| &*attr.name.local == "href")
        // `StrTendril` → `&str` by deref, then an owned copy. Formatting the
        // tendril itself would embed its storage discriminator (P4).
        .map(|attr| attr.value.trim().to_owned())
}
