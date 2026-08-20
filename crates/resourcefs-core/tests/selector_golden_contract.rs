use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    time::{Duration, Instant},
};

use resourcefs_core::{
    ArtifactAddress, ErrorCategory, MAX_ARTIFACT_BYTES, PathReference, ProjectionSelector,
    ResourceAddress, VersionTag, select_utf8,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    cases: Vec<SuccessCase>,
    errors: Vec<ErrorCase>,
    invalid_selectors: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SuccessCase {
    name: String,
    content: String,
    selector: String,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct ErrorCase {
    name: String,
    content: String,
    selector: String,
    category: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/bounded_read_cases.json"))
        .expect("bounded-read fixture must be valid JSON")
}

#[test]
fn selector_golden_contract() {
    for case in fixture().cases {
        let selector = ProjectionSelector::parse(&case.selector)
            .unwrap_or_else(|error| panic!("{}: selector failed: {error}", case.name));
        let selected = select_utf8(Cursor::new(case.content.as_bytes()), Some(&selector))
            .unwrap_or_else(|error| panic!("{}: selection failed: {error}", case.name));
        assert_eq!(selector.is_raw(), case.selector.starts_with("raw"));
        if let Some(lines) = selector.line_selection() {
            assert_eq!(lines.is_raw(), case.selector.starts_with("raw:"));
            assert!(!lines.ranges().is_empty());
        }
        assert_eq!(selected.source_bytes(), case.content.len() as u64);
        assert_eq!(
            selected.version_tag(),
            &VersionTag::from_content(case.content.as_bytes())
        );
        assert_eq!(
            selected.content().as_bytes(),
            case.expected.as_bytes(),
            "{}",
            case.name
        );
    }

    for case in fixture().errors {
        let selector = ProjectionSelector::parse(&case.selector)
            .unwrap_or_else(|error| panic!("{}: selector failed: {error}", case.name));
        let error = select_utf8(Cursor::new(case.content.as_bytes()), Some(&selector))
            .expect_err("past-EOF start must fail atomically");
        assert_eq!(error.category().as_str(), case.category, "{}", case.name);
    }
}

#[test]
fn selected_text_transfers_owned_projection_without_copying() {
    let selected = select_utf8(Cursor::new(b"alpha\nbeta\n"), None).expect("complete selection");
    let (content, version_tag, source_bytes) = selected.into_parts();

    assert_eq!(content, "alpha\nbeta\n");
    assert_eq!(version_tag, VersionTag::from_content(b"alpha\nbeta\n"));
    assert_eq!(source_bytes, 11);
}

#[test]
fn malformed_selectors_are_rejected() {
    for spelling in fixture().invalid_selectors {
        let error =
            ProjectionSelector::parse(&spelling).expect_err("fixture selector should be invalid");
        assert_eq!(
            error.category(),
            ErrorCategory::InvalidReference,
            "{spelling:?}"
        );
        let reference = PathReference::parse(format!("missing:{spelling}"))
            .expect("workspace literal remains syntactically valid");
        assert_eq!(
            reference
                .selector_error()
                .expect("malformed selector candidate")
                .category(),
            ErrorCategory::InvalidReference,
            "{spelling:?}"
        );
    }
}

#[test]
fn page_projection_is_not_a_text_selection() {
    let selector = ProjectionSelector::parse("page:1").expect("valid artifact page syntax");
    assert_eq!(selector.page_offset(), Some(1));
    assert!(!selector.is_raw());
    let error = select_utf8(Cursor::new(b"abc"), Some(&selector))
        .expect_err("page execution belongs to the bounded read engine");
    assert_eq!(error.category(), ErrorCategory::UnsupportedProjection);
}

#[test]
fn invalid_utf8_fails_even_when_the_selected_line_is_valid() {
    let selector = ProjectionSelector::parse("1-1").expect("valid selector");
    let error = select_utf8(Cursor::new(b"good\n\xff\n"), Some(&selector))
        .expect_err("the complete authoritative Resource must be UTF-8");
    assert_eq!(error.category(), ErrorCategory::UnsupportedProjection);
}

#[test]
fn selected_projection_object_ceiling_is_exact() {
    let at_limit = vec![b'x'; MAX_ARTIFACT_BYTES];
    let selected = select_utf8(Cursor::new(at_limit), None).expect("64 MiB must fit");
    assert_eq!(selected.content().len(), MAX_ARTIFACT_BYTES);

    let over_limit = vec![b'x'; MAX_ARTIFACT_BYTES + 1];
    let error = select_utf8(Cursor::new(over_limit), None).expect_err("one byte over must fail");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert!(error.message().contains("narrower selector"));
}

#[test]
fn workspace_page_syntax_remains_a_literal_first_candidate() {
    let reference = PathReference::parse("notes:page:1").expect("valid literal workspace path");
    let Some(resourcefs_core::WorkspaceAddress::Relative(path)) = reference.workspace_address()
    else {
        panic!("workspace literal must remain the primary address")
    };
    assert_eq!(path.as_path().to_string_lossy(), "notes:page:1");
    assert_eq!(
        reference
            .selector_candidate()
            .and_then(|candidate| candidate.selector().page_offset()),
        Some(1)
    );
}

#[test]
fn artifact_reference_grammar_is_typed_and_canonical() {
    let reference = PathReference::parse("artifact://0123456789abcdef0123456789abcdef-7:raw:2-4")
        .expect("valid artifact reference");
    let ResourceAddress::Artifact(address) = reference.address() else {
        panic!("artifact reference must have artifact address")
    };
    assert_eq!(address.session_token(), "0123456789abcdef0123456789abcdef");
    assert_eq!(address.object_id(), 7);
    assert_eq!(
        reference.projection().map(ProjectionSelector::as_str),
        Some("raw:2-4")
    );

    let constructed = PathReference::artifact(
        ArtifactAddress::new("0123456789abcdef0123456789abcdef", 7)
            .expect("valid artifact address"),
        Some(ProjectionSelector::parse("raw:2-4").expect("valid projection")),
    )
    .expect("canonical artifact construction");
    assert_eq!(constructed, reference);
    for invalid in [
        "artifact://0123456789abcdef0123456789abcdef-0",
        "artifact://0123456789ABCDEF0123456789ABCDEF-1",
        "artifact://0123456789abcdef0123456789abcdef-01",
        "artifact://0123456789abcdef0123456789abcdef/1",
        "artifact://0123456789abcdef0123456789abcdef-1:page:0",
    ] {
        assert_eq!(
            resourcefs_core::PathReference::parse(invalid)
                .expect_err("invalid artifact syntax")
                .category(),
            ErrorCategory::InvalidReference,
            "{invalid}"
        );
    }
}

#[test]
fn stress_selection_budget() {
    const SOURCE_BYTES: u64 = 256 * 1024 * 1024;
    const LINE_BYTES: u64 = 1024 * 1024;
    let source = VirtualLines::new(SOURCE_BYTES, LINE_BYTES);
    let selector = ProjectionSelector::parse("256-256").expect("valid final-line selector");
    let started = Instant::now();
    let selected = select_utf8(source, Some(&selector)).expect("large narrow selection");
    let elapsed = started.elapsed();

    assert_eq!(selected.content().len(), LINE_BYTES as usize);
    assert_eq!(
        selected.content().as_bytes()[LINE_BYTES as usize - 1],
        b'\n'
    );
    assert!(selected.content().len() <= 70 * 1024 * 1024);
    if !cfg!(debug_assertions) {
        assert!(
            elapsed <= Duration::from_secs(10),
            "selection took {elapsed:?}"
        );
    }
}

#[derive(Debug)]
struct VirtualLines {
    len: u64,
    line_bytes: u64,
    position: u64,
}

impl VirtualLines {
    fn new(len: u64, line_bytes: u64) -> Self {
        assert_eq!(len % line_bytes, 0);
        Self {
            len,
            line_bytes,
            position: 0,
        }
    }
}

impl Read for VirtualLines {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.len.saturating_sub(self.position);
        let count = remaining.min(buffer.len() as u64) as usize;
        buffer[..count].fill(b'x');
        for (offset, byte) in buffer[..count].iter_mut().enumerate() {
            if (self.position + offset as u64 + 1).is_multiple_of(self.line_bytes) {
                *byte = b'\n';
            }
        }
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for VirtualLines {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = match position {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.len) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };
        if !(0..=i128::from(self.len)).contains(&next) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek out of bounds",
            ));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}
