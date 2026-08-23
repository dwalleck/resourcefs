//! C9 — reader-mode extraction is deterministic and never derives canonical
//! output from a `Debug` rendering.
//!
//! The oracle is deliberately two-part, because neither half alone is enough:
//!
//! * a **hand-authored** expected Markdown for the small document, written
//!   from the markup rather than from running the extractor — running the
//!   extractor to produce its own expectation would prove nothing;
//! * a **cross-process** digest comparison, which rules out the failure modes
//!   a repeat-run check cannot see. An in-process cache, a memoized token
//!   stream, or an address-dependent iteration order all reproduce perfectly
//!   within one process and diverge across two.
//!
//! The corpus carries a case whose strings straddle tendril's inline/heap
//! storage boundary. That is the exact defect `prove-it-prototype` hit at P4:
//! `{:?}` on a `StrTendril` emits `Tendril<UTF8>(inline: "…")` for short
//! strings and a different form for longer ones, so a digest built that way is
//! stable for short input and silently unstable for long input. Without the
//! boundary case a regression into `{:?}` would pass every other row.

use std::process::Command;

use sha2::{Digest, Sha256};

/// The corpus, in a fixed order so digests are comparable across processes.
const CORPUS: &[&str] = &[
    "well_formed",
    "malformed",
    "scripted",
    "astral",
    "attribute_heavy",
    "deeply_nested",
    "entities",
    "storage_boundary",
    // Raw-text elements whose bodies contain markup-looking text, and list
    // items whose content is wrapped in a block element. Both shapes are
    // ubiquitous in real HTML and both were silently mis-rendered.
    "raw_text_and_lists",
];

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/extract/{name}.html",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

fn digest(markdown: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(markdown.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Renders every corpus document and returns `name=digest` lines.
///
/// Used both in-process and — via the harness re-executing this binary — in a
/// separate process, so the two results can be compared.
fn corpus_digests() -> Vec<String> {
    CORPUS
        .iter()
        .map(|name| {
            let markdown = resourcefs_sources::extract_markdown_for_test(&fixture(name));
            format!("{name}={}", digest(&markdown))
        })
        .collect()
}

/// A child-process entry point: writes the digests to a file and exits.
///
/// `cargo test` runs this same binary, so setting the environment variable and
/// re-executing gives a genuinely separate process without a second build
/// target or any network. The digests go to a **file** rather than stdout
/// because libtest captures stdout — a child that printed them would appear to
/// produce nothing, which is how the first version of this fence failed.
/// The write is staged and renamed rather than written in place. `exit` does
/// not wait for other threads, so an in-place write could be observed
/// half-finished by the parent; the parent then dropped the mangled line while
/// filtering and reported a *determinism* failure, pointing at extraction
/// rather than at the handoff. Renaming publishes the file in one step, so the
/// parent sees all of it or none of it.
fn maybe_run_as_child() {
    let Some(path) = std::env::var_os("RFS_EXTRACT_DIGEST_CHILD") else {
        return;
    };
    let body = corpus_digests().join("\n");
    let staged = std::path::PathBuf::from(&path).with_extension("partial");
    std::fs::write(&staged, body).expect("child stages its digests");
    std::fs::rename(&staged, &path).expect("child publishes its digests atomically");
    std::process::exit(0);
}

#[test]
fn extraction_is_deterministic() {
    maybe_run_as_child();

    let first = corpus_digests();
    let second = corpus_digests();
    assert_eq!(
        first, second,
        "extraction must be byte-identical on repeat within one process"
    );

    // Cross-process: re-execute this test binary with the child marker set.
    let binary = std::env::current_exe().expect("current test binary");
    let handoff = std::env::temp_dir().join(format!(
        "rfs-extract-digests-{}-{}.txt",
        std::process::id(),
        CORPUS.len()
    ));
    let _ = std::fs::remove_file(&handoff);
    // Run exactly this one test in the child. Without the filter the child ran
    // the whole harness, so every test called `maybe_run_as_child`, each wrote
    // the same handoff path, and the first `exit(0)` cut the others off — a
    // race that grew with the file's test count and surfaced as a bogus
    // determinism failure.
    let output = Command::new(&binary)
        .args(["extraction_is_deterministic", "--exact", "--test-threads=1"])
        .env("RFS_EXTRACT_DIGEST_CHILD", &handoff)
        .output()
        .expect("re-execute the test binary as a child process");
    assert!(
        output.status.success(),
        "child process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let child: Vec<String> = std::fs::read_to_string(&handoff)
        .expect("child wrote its digests")
        .lines()
        .map(str::to_owned)
        .filter(|line| line.contains('='))
        .collect();
    let _ = std::fs::remove_file(&handoff);

    assert_eq!(
        first, child,
        "digests must agree across separate processes; a difference means \
         canonical output depends on in-process state, allocation addresses, \
         or a storage discriminator such as tendril's inline/heap form"
    );

    // The boundary row must actually be present, or the fence is decorative.
    assert!(
        child
            .iter()
            .any(|line| line.starts_with("storage_boundary=")),
        "the tendril storage-boundary case must be in the corpus"
    );
}

#[test]
fn extraction_matches_golden() {
    maybe_run_as_child();

    // Hand-authored from `well_formed.html` by reading the markup, not by
    // running the extractor.
    let expected = "\
# Title

A paragraph with a [link](https://example.com/doc).

## Section

- first

- second
";
    let actual = resourcefs_sources::extract_markdown_for_test(&fixture("well_formed"));
    assert_eq!(
        actual, expected,
        "extraction must match the hand-authored expectation byte for byte"
    );
}

#[test]
fn dropped_elements_never_reach_output() {
    maybe_run_as_child();

    let markdown = resourcefs_sources::extract_markdown_for_test(&fixture("scripted"));
    for forbidden in ["must not appear", "color:red", "M0 0", "hidden fallback"] {
        assert!(
            !markdown.contains(forbidden),
            "content of a dropped element reached the output: {forbidden:?} in {markdown:?}"
        );
    }
    assert!(
        markdown.contains("Visible text."),
        "visible content must survive: {markdown:?}"
    );
}

#[test]
fn storage_boundary_strings_render_as_plain_text() {
    maybe_run_as_child();

    let markdown = resourcefs_sources::extract_markdown_for_test(&fixture("storage_boundary"));
    // A `Debug` rendering of a tendril contains its storage discriminator.
    // Asserting on the rendered text is what makes the `{:?}` regression fail
    // here rather than only in the cross-process digest comparison.
    assert!(
        !markdown.contains("Tendril") && !markdown.contains("inline:"),
        "canonical output must not carry a tendril Debug rendering: {markdown:?}"
    );
    assert!(
        markdown.contains("[ab](/s)"),
        "short (inline-stored) link must render as plain text: {markdown:?}"
    );
    assert!(
        markdown.contains("[abcdefghijklmnopqrstuvwxyz0123456789](/a-considerably-longer-href-past-the-inline-threshold)"),
        "long (heap-stored) link must render as plain text: {markdown:?}"
    );
}

/// C9 — a raw-text element's body is text, never markup.
///
/// `<script>` and `<style>` bodies are tokenized in a raw-text state, which
/// only the sink can request. Returning `Continue` for them left their bodies
/// read as markup, so a `<style>` appearing inside a script *string* opened a
/// second dropped element that its `</script>` never closed: the skip depth
/// never returned to zero and every character token afterwards was suppressed.
/// The document silently truncated to whatever preceded the script, returned
/// `Ok`, and fed a content-derived Version Tag.
///
/// The assertion is on the surviving text, not merely on length: a truncation
/// that happened to keep the byte count would otherwise pass.
#[test]
fn raw_text_bodies_do_not_truncate_the_document() {
    let markdown = resourcefs_sources::extract_markdown_for_test(
        r#"<html><body><p>Before</p><script>var t = "<style>";</script><p>After</p></body></html>"#,
    );
    assert!(
        markdown.contains("Before") && markdown.contains("After"),
        "content after a script containing markup-shaped text must survive; got {markdown:?}"
    );
    assert!(
        !markdown.contains("var t"),
        "script source must never reach the output; got {markdown:?}"
    );

    // HTML ignores a self-closing flag on `<script>`: the element runs to the
    // next `</script>`. Honouring the flag leaked script text into the output.
    let self_closing = resourcefs_sources::extract_markdown_for_test(
        r#"<html><body><script/>leaked source<p>x</p></body></html>"#,
    );
    assert!(
        !self_closing.contains("leaked source"),
        "a self-closing script tag must not leak its body; got {self_closing:?}"
    );

    let styled = resourcefs_sources::extract_markdown_for_test(
        r#"<html><body><style>p{content:"<script>"}</style><p>Shown</p></body></html>"#,
    );
    assert!(
        styled.contains("Shown") && !styled.contains("content:"),
        "style bodies drop out without swallowing the document; got {styled:?}"
    );
}

/// C9 — a list item keeps its marker however its content is wrapped.
///
/// `<li>` sets the marker, and a block element opening immediately inside it
/// flushed an empty line before any text arrived. Clearing the prefix on that
/// empty flush discarded the marker, so `<li><p>text</p></li>` — an extremely
/// common shape — rendered as a bare paragraph. For `<ol>` the counter had
/// already advanced, so the items that did keep a marker renumbered (1, 3, 5…),
/// which is the more damaging failure: the output looks like a list and is
/// wrong about its own ordering.
#[test]
fn list_items_keep_their_marker_when_content_is_wrapped() {
    let unordered = resourcefs_sources::extract_markdown_for_test(
        r"<ul><li><p>Wrapped</p></li><li>Bare</li><li><div>Div</div></li></ul>",
    );
    assert_eq!(
        unordered, "- Wrapped\n\n- Bare\n\n- Div\n",
        "every item keeps its bullet regardless of wrapping"
    );

    let ordered = resourcefs_sources::extract_markdown_for_test(
        r"<ol><li><p>One</p></li><li>Two</li><li><div>Three</div></li></ol>",
    );
    assert_eq!(
        ordered, "1. One\n\n2. Two\n\n3. Three\n",
        "ordered items number consecutively; a lost marker must not skip a number"
    );

    // An empty item must not leave its marker behind to compound onto the next.
    let empty = resourcefs_sources::extract_markdown_for_test(r"<ul><li></li><li>x</li></ul>");
    assert_eq!(empty, "- x\n", "an unspent marker must not accumulate");
}
