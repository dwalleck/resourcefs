use std::{
    path::Path,
    time::{Duration, Instant},
};

use resourcefs_core::{
    CatalogAddress, ErrorCategory, MAX_PATH_REFERENCE_BYTES, PathReference, ResourceAddress,
    WorkspaceAddress, WorkspacePath, WorkspaceRoot, WorkspaceRootId, WorkspaceRootSet,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusRow {
    kind: String,
    input: String,
    platform: String,
    expected: Expected,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expected {
    status: String,
    address_kind: Option<String>,
    value: Option<String>,
    root: Option<String>,
    selector: Option<String>,
    category: Option<String>,
}

fn corpus() -> Vec<CorpusRow> {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/workspace_references.json"
    ))
    .expect("workspace reference corpus must be valid JSON")
}

fn applies_to_host(platform: &str) -> bool {
    platform == "all" || platform == "windows" && cfg!(windows) || platform == "posix" && cfg!(unix)
}

fn address_observation(address: &WorkspaceAddress) -> (&'static str, String, Option<&str>) {
    match address {
        WorkspaceAddress::Relative(path) => (
            "relative",
            path.as_path().to_string_lossy().into_owned(),
            None,
        ),
        WorkspaceAddress::Absolute(path) => ("absolute", path.to_string_lossy().into_owned(), None),
        WorkspaceAddress::FileUri(uri) => ("fileUri", uri.as_str().to_owned(), None),
        WorkspaceAddress::Canonical { root, path } => (
            "canonical",
            path.as_path().to_string_lossy().into_owned(),
            Some(root.as_str()),
        ),
    }
}

#[test]
fn golden_workspace_references() {
    for row in corpus()
        .into_iter()
        .filter(|row| applies_to_host(&row.platform))
    {
        assert_eq!(row.kind, "parse", "unsupported corpus kind in {row:?}");
        let parsed = PathReference::parse(row.input.clone());
        match row.expected.status.as_str() {
            "ok" => {
                let reference = parsed.unwrap_or_else(|error| {
                    panic!("expected {:?} to parse, got {error}", row.input)
                });
                let (kind, value, root) =
                    address_observation(reference.workspace_address().expect("workspace address"));
                assert_eq!(Some(kind), row.expected.address_kind.as_deref(), "{row:?}");
                assert_eq!(
                    Some(value.as_str()),
                    row.expected.value.as_deref(),
                    "{row:?}"
                );
                assert_eq!(root, row.expected.root.as_deref(), "{row:?}");
                assert_eq!(
                    reference
                        .selector_candidate()
                        .map(|selected| selected.selector().as_str()),
                    row.expected.selector.as_deref(),
                    "{row:?}",
                );
            }
            "error" => {
                let error = parsed.unwrap_err();
                assert_eq!(
                    error.category().as_str(),
                    row.expected.category.as_deref().expect("error category"),
                    "{row:?}",
                );
            }
            status => panic!("unknown corpus status {status:?}"),
        }
    }
}

#[test]
fn reference_byte_limit_is_exact() {
    let at_limit = "a".repeat(MAX_PATH_REFERENCE_BYTES);
    let reference = PathReference::parse(at_limit.clone()).expect("64 KiB must parse");
    assert_eq!(
        address_observation(reference.workspace_address().expect("workspace address")),
        ("relative", at_limit, None)
    );

    let over_limit = "a".repeat(MAX_PATH_REFERENCE_BYTES + 1);
    let error = PathReference::parse(over_limit).expect_err("one byte over must fail");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
}

#[test]
fn canonical_construction_percent_encodes_ambiguous_component_characters() {
    let root = WorkspaceRootId::new("workspace").expect("valid root id");
    let path = WorkspacePath::new(Path::new("notes/résumé % #1:2")).expect("valid path");
    let reference = PathReference::canonical(root, path);

    assert_eq!(
        reference.requested(),
        "rfs://workspace/workspace/notes/résumé %25 %231%3A2"
    );
    assert!(matches!(
        reference.workspace_address().expect("workspace address"),
        WorkspaceAddress::Canonical { .. }
    ));
    assert_eq!(PathReference::parse(reference.requested()), Ok(reference));
}

#[test]
fn catalog_root_lines_parse_and_teach() {
    let trailing =
        PathReference::parse("rfs://workspace/root.with-dots/").expect("trailing root reference");
    let slashless =
        PathReference::parse("rfs://workspace/root.with-dots").expect("slashless root reference");

    assert_eq!(trailing, slashless);
    assert_eq!(trailing.requested(), "rfs://workspace/root.with-dots/");
    let WorkspaceAddress::Canonical { root, path } = trailing
        .workspace_address()
        .expect("canonical workspace address")
    else {
        panic!("root reference must be canonical");
    };
    assert_eq!(root.as_str(), "root.with-dots");
    assert!(path.is_root());
}

#[test]
fn workspace_root_sentinel_is_not_general_empty_path_input() {
    let root = WorkspacePath::root();
    assert!(root.is_root());
    assert_eq!(
        WorkspacePath::new("")
            .expect_err("empty relative path")
            .category(),
        ErrorCategory::InvalidReference
    );
    assert_eq!(
        WorkspacePath::new(".")
            .expect_err("dot relative path")
            .category(),
        ErrorCategory::InvalidReference
    );
}

#[test]
fn catalog_references_are_typed_with_one_alias() {
    let sources = PathReference::parse("rfs://").expect("source catalog");
    assert!(matches!(
        sources.address(),
        ResourceAddress::Catalog(CatalogAddress::Sources)
    ));
    assert_eq!(sources.requested(), "rfs://");

    let workspace = PathReference::parse("rfs://workspace").expect("workspace catalog");
    let alias = PathReference::parse("rfs://workspace/").expect("workspace catalog alias");
    assert_eq!(workspace, alias);
    assert!(matches!(
        workspace.address(),
        ResourceAddress::Catalog(CatalogAddress::Workspace)
    ));
    assert_eq!(workspace.requested(), "rfs://workspace");

    for rejected in [
        "rfs://:raw",
        "rfs://workspace:raw",
        "rfs://workspace/:raw",
        "rfs://bogus",
    ] {
        assert_eq!(
            PathReference::parse(rejected)
                .expect_err("catalog neighbor must remain invalid")
                .category(),
            ErrorCategory::InvalidReference,
            "{rejected}"
        );
    }
}

#[test]
fn empty_root_sets_represent_scratch_only_authority() {
    let empty = WorkspaceRootSet::new(Vec::new(), None).expect("scratch-only root set");
    assert!(empty.roots().is_empty());
    assert_eq!(empty.primary(), None);

    let deferred = WorkspaceRootSet::new(Vec::new(), Some("workspace"))
        .expect("scratch-only root set may accept a deferred external selector");
    assert!(deferred.roots().is_empty());
    assert_eq!(deferred.primary(), None);
}

#[test]
fn root_sets_are_order_independent_and_primary_selection_is_exact() {
    let alpha = WorkspaceRoot::new(
        WorkspaceRootId::new("alpha").expect("id"),
        url::Url::from_directory_path(std::env::temp_dir().join("alpha"))
            .expect("directory file URI")
            .to_string(),
        Some("chosen".to_owned()),
    )
    .expect("root");
    let beta = WorkspaceRoot::new(
        WorkspaceRootId::new("beta").expect("id"),
        url::Url::from_directory_path(std::env::temp_dir().join("beta"))
            .expect("directory file URI")
            .to_string(),
        Some("other".to_owned()),
    )
    .expect("root");

    let first =
        WorkspaceRootSet::new(vec![alpha.clone(), beta.clone()], Some("chosen")).expect("root set");
    let reordered = WorkspaceRootSet::new(vec![beta, alpha], Some("chosen")).expect("root set");

    assert_eq!(first.primary().map(WorkspaceRootId::as_str), Some("alpha"));
    assert!(first.equivalent_to(&reordered));
}

#[test]
fn workspace_root_rejects_invalid_selector_name() {
    let error = WorkspaceRoot::new(
        WorkspaceRootId::new("client-root").expect("root id"),
        url::Url::from_directory_path(std::env::temp_dir().join("client"))
            .expect("directory file URI")
            .to_string(),
        Some("bad/name".to_owned()),
    )
    .expect_err("selector name must use Workspace Root ID grammar");
    assert_eq!(error.category(), ErrorCategory::InvalidReference);
}

#[test]
fn malformed_unicode_artifact_token_returns_error_instead_of_panicking() {
    let input = format!("artifact://{}՞-1", "6".repeat(31));
    let error = PathReference::parse(input).expect_err("non-ASCII artifact token");
    assert_eq!(error.category(), ErrorCategory::InvalidReference);
}

#[test]
fn parses_production_shaped_reference_within_budget() {
    let file_name = format!("{} file.txt", "é".repeat(2_000));
    let started = Instant::now();
    let reference = PathReference::parse(file_name.clone()).expect("long unicode path");
    let elapsed = started.elapsed();

    assert_eq!(
        address_observation(reference.workspace_address().expect("workspace address")),
        ("relative", file_name, None)
    );
    assert!(
        elapsed <= Duration::from_millis(5),
        "4 KiB reference parsing took {elapsed:?}"
    );
}

/// Expected outcome for one row of the hand-authored `local://` grammar oracle.
///
/// The oracle is written by hand from the approved spec, never computed from the
/// parser: each row states the family the reference must land in, or the exact
/// error category it must produce.
#[derive(Debug, PartialEq, Eq)]
enum GrammarExpectation {
    /// Parses to `ResourceAddress::Local` whose decoded name is exactly this string.
    LocalNamed(String),
    /// Parses to the Session Scratch family root.
    LocalRoot,
    /// Parses to `ResourceAddress::Https` whose canonical URL is exactly this string.
    Https(String),
    /// Parses to a non-local family; the label names it for failure reporting.
    OtherFamily(&'static str),
    Rejected(ErrorCategory),
}

/// Classifies a parse result without consulting the parser's own branch logic.
fn classify(input: &str) -> GrammarExpectation {
    match PathReference::parse(input.to_owned()) {
        Ok(reference) => match reference.address() {
            ResourceAddress::Local(address) => match address.name() {
                Some(name) => GrammarExpectation::LocalNamed(name.as_str().to_owned()),
                None => GrammarExpectation::LocalRoot,
            },
            ResourceAddress::Https(address) => {
                GrammarExpectation::Https(address.as_str().to_owned())
            }
            ResourceAddress::Workspace(_) => GrammarExpectation::OtherFamily("workspace"),
            ResourceAddress::Artifact(_) => GrammarExpectation::OtherFamily("artifact"),
            ResourceAddress::Catalog(_) => GrammarExpectation::OtherFamily("catalog"),
            ResourceAddress::Issue(_) => GrammarExpectation::OtherFamily("issue"),
            ResourceAddress::Jira(_) => GrammarExpectation::OtherFamily("jira"),
            ResourceAddress::PullRequest(_) => GrammarExpectation::OtherFamily("pullRequest"),
        },
        Err(error) => GrammarExpectation::Rejected(error.category()),
    }
}

/// C1: `https://` is a first-class address family, and admitting it changes no
/// existing family's parse result.
///
/// The table is authored from `spec.md`'s grammar, never computed by calling the
/// parser: each expected value is written out by hand so a parser bug cannot
/// define its own expectation.
#[test]
fn https_grammar() {
    let expected: Vec<(String, GrammarExpectation)> = vec![
        // Accepted HTTPS URLs.
        (
            "https://example.com/doc".to_owned(),
            GrammarExpectation::Https("https://example.com/doc".to_owned()),
        ),
        // Percent-encoded separators are legal in a URL and must survive to the
        // wire: the encoded-separator guard is a filesystem-containment control
        // and does not apply to this family.
        (
            "https://example.com/search?q=a%2Fb".to_owned(),
            GrammarExpectation::Https("https://example.com/search?q=a%2Fb".to_owned()),
        ),
        (
            "https://example.com/a%2Fb/doc".to_owned(),
            GrammarExpectation::Https("https://example.com/a%2Fb/doc".to_owned()),
        ),
        (
            "https://example.com/a%5Cb/doc".to_owned(),
            GrammarExpectation::Https("https://example.com/a%5Cb/doc".to_owned()),
        ),
        // Explicit port, fragment, and bare origin.
        (
            "https://example.com:8443/doc".to_owned(),
            GrammarExpectation::Https("https://example.com:8443/doc".to_owned()),
        ),
        (
            "https://example.com/doc#frag".to_owned(),
            GrammarExpectation::Https("https://example.com/doc#frag".to_owned()),
        ),
        (
            "https://example.com".to_owned(),
            GrammarExpectation::Https("https://example.com/".to_owned()),
        ),
        // An internationalized host normalizes to its Punycode form.
        (
            "https://例え.jp/doc".to_owned(),
            GrammarExpectation::Https("https://xn--r8jz45g.jp/doc".to_owned()),
        ),
        // Rejected: plain HTTP is not an allowlisted transport.
        (
            "http://example.com/doc".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        // Rejected: a scheme with no host cannot name an origin.
        (
            "https://".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        // Rejected: still an unsupported scheme.
        (
            "ftp://example.com/doc".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        // Rejected: credentials come from the Server Profile, never the
        // reference — embedded userinfo would carry secret material into
        // canonical references, catalogs, errors, and logs.
        (
            "https://user:pass@example.com/doc".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        (
            "https://user@example.com/doc".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        // Every existing family keeps its prior parse result.
        (
            "rfs://workspace/root/notes.md".to_owned(),
            GrammarExpectation::OtherFamily("workspace"),
        ),
        (
            "artifact://0123456789abcdef0123456789abcdef-1".to_owned(),
            GrammarExpectation::OtherFamily("artifact"),
        ),
        (
            "rfs://".to_owned(),
            GrammarExpectation::OtherFamily("catalog"),
        ),
        (
            "rfs://workspace".to_owned(),
            GrammarExpectation::OtherFamily("catalog"),
        ),
        (
            "local://plan.md".to_owned(),
            GrammarExpectation::LocalNamed("plan.md".to_owned()),
        ),
        ("local://".to_owned(), GrammarExpectation::LocalRoot),
        (
            "notes/plan.md".to_owned(),
            GrammarExpectation::OtherFamily("workspace"),
        ),
        (
            "file:///tmp/plan.md".to_owned(),
            GrammarExpectation::OtherFamily("workspace"),
        ),
        // The narrowed guard still owns the filesystem families: an encoded
        // separator in a workspace path or a scratch name is still rejected.
        (
            "a%2Fb".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        (
            "a%5Cb".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        (
            "local://a%2Fb".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        // A malformed escape in a filesystem reference is still rejected.
        (
            "bad%ZZescape".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
    ];

    for (input, want) in &expected {
        assert_eq!(&classify(input), want, "grammar row for {input:?}");
    }
}

#[test]
fn local_name_grammar() {
    let two_hundred_fifty_five = "n".repeat(255);
    let two_hundred_fifty_six = "n".repeat(256);
    let accepted_unicode = "设计.md";

    // Hand-authored accept/reject table: input -> expected outcome, from the
    // approved spec's name grammar (printable UTF-8; no separator, decoded or
    // literal; not `.`/`..`/empty; at most 255 UTF-8 bytes).
    let expected: Vec<(String, GrammarExpectation)> = vec![
        // Accepted scratch names.
        (
            "local://plan.md".to_owned(),
            GrammarExpectation::LocalNamed("plan.md".to_owned()),
        ),
        (
            "local://review notes.md".to_owned(),
            GrammarExpectation::LocalNamed("review notes.md".to_owned()),
        ),
        (
            format!("local://{accepted_unicode}"),
            GrammarExpectation::LocalNamed(accepted_unicode.to_owned()),
        ),
        (
            format!("local://{two_hundred_fifty_five}"),
            GrammarExpectation::LocalNamed(two_hundred_fifty_five.clone()),
        ),
        // A percent-encoded literal `%` decodes back to the intended name.
        (
            "local://100%25.md".to_owned(),
            GrammarExpectation::LocalNamed("100%.md".to_owned()),
        ),
        // Rejected scratch names.
        (
            format!("local://{two_hundred_fifty_six}"),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        (
            "local://a%2Fb".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        (
            "local://a%5Cb".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        // Literal separators are owned solely by the scratch-name check.
        (
            "local://a/b".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        (
            "local://a\\b".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        (
            "local://..".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        (
            "local://.".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        // Spec Revision 1: the bare family root is a valid reference addressing
        // this session's scratch listing, mirroring `rfs://` and `rfs://workspace`.
        ("local://".to_owned(), GrammarExpectation::LocalRoot),
        (
            "local://bell\u{7}.md".to_owned(),
            GrammarExpectation::Rejected(ErrorCategory::InvalidReference),
        ),
        // Every existing family keeps its prior parse result.
        (
            "rfs://workspace/root/notes.md".to_owned(),
            GrammarExpectation::OtherFamily("workspace"),
        ),
        (
            "artifact://0123456789abcdef0123456789abcdef-1".to_owned(),
            GrammarExpectation::OtherFamily("artifact"),
        ),
        (
            "rfs://".to_owned(),
            GrammarExpectation::OtherFamily("catalog"),
        ),
        (
            "rfs://workspace".to_owned(),
            GrammarExpectation::OtherFamily("catalog"),
        ),
        (
            "notes/plan.md".to_owned(),
            GrammarExpectation::OtherFamily("workspace"),
        ),
        (
            "file:///tmp/plan.md".to_owned(),
            GrammarExpectation::OtherFamily("workspace"),
        ),
    ];

    for (input, want) in &expected {
        assert_eq!(&classify(input), want, "grammar row for {input:?}");
    }
}

#[test]
fn local_references_render_canonically_and_round_trip() {
    for name in ["plan.md", "review notes.md", "设计.md", "100%.md"] {
        let reference = PathReference::local(name).expect("valid scratch name");
        let ResourceAddress::Local(parsed) = reference.address() else {
            panic!("{name} must parse as a Session Scratch Resource");
        };
        assert_eq!(
            parsed.name().expect("a named scratch reference").as_str(),
            name
        );

        let reparsed =
            PathReference::parse(reference.requested().to_owned()).expect("canonical round-trip");
        assert_eq!(&reparsed, &reference, "round-trip for {name:?}");
    }
}

#[test]
fn maximum_reference_parses_within_boundary_budget() {
    let input = "a".repeat(MAX_PATH_REFERENCE_BYTES);
    let started = Instant::now();
    PathReference::parse(input).expect("maximum reference must parse");
    let elapsed = started.elapsed();

    assert!(
        elapsed <= Duration::from_millis(50),
        "64 KiB reference parsing took {elapsed:?}"
    );
}

/// C19 — the encoded-separator guard covers exactly the forms that reach
/// `percent_decode`, and reaching it unvalidated would be a panic.
///
/// The guard is deliberately **not** asserted as a family list. `percent_decode`
/// indexes `bytes[index + 1]` under an `expect("percent escapes were
/// validated")`, so a form that reaches it without prior validation aborts the
/// process rather than returning `invalid_reference`. A family-list assertion
/// would still pass in that world; this table cannot, because every row that
/// decodes carries a **malformed trailing escape** whose only two honest
/// outcomes are a clean rejection or no decoding at all.
///
/// Each row therefore pins one of three outcomes, and the test completing at
/// all is itself the evidence that no form panicked.
#[test]
fn encoded_separator_guard_scope() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Outcome {
        /// Parses, and the encoding survives into the canonical spelling.
        PreservesEncoding,
        /// Rejected because a filesystem-backed family forbids the separator.
        RejectsSeparator,
        /// Rejected because the escape itself is malformed.
        RejectsMalformed,
    }

    // Hand-authored from the signed spec, not derived from the parser.
    let table: &[(&str, Outcome)] = &[
        // Filesystem-backed families still reject encoded separators.
        ("notes%2Fsecret.md", Outcome::RejectsSeparator),
        ("notes%5Csecret.md", Outcome::RejectsSeparator),
        (
            "rfs://workspace/repo/notes%2Fsecret.md",
            Outcome::RejectsSeparator,
        ),
        ("local://notes%2Fsecret.md", Outcome::RejectsSeparator),
        // ...and still reject a malformed escape rather than panicking on it.
        ("notes%.md", Outcome::RejectsMalformed),
        ("notes%2.md", Outcome::RejectsMalformed),
        ("local://notes%.md", Outcome::RejectsMalformed),
        ("rfs://workspace/repo/notes%.md", Outcome::RejectsMalformed),
        // HTTPS never decodes, so an encoded separator is meaningful to the
        // origin and survives verbatim in path and query alike.
        ("https://example.com/a%2Fb/doc", Outcome::PreservesEncoding),
        (
            "https://example.com/search?q=a%2Fb",
            Outcome::PreservesEncoding,
        ),
        ("https://example.com/a%5Cb", Outcome::PreservesEncoding),
        // A trailing escape is the panic case if HTTPS ever started decoding.
        ("https://example.com/a%", Outcome::PreservesEncoding),
    ];

    for (input, expected) in table {
        let parsed = PathReference::parse((*input).to_owned());
        match expected {
            Outcome::PreservesEncoding => {
                let reference =
                    parsed.unwrap_or_else(|error| panic!("{input} must parse; got {error}"));
                let encoded = input.rsplit_once('/').map_or(*input, |(_, tail)| tail);
                assert!(
                    reference.requested().contains(encoded),
                    "{input} must reach the wire with its encoding intact, got {}",
                    reference.requested()
                );
            }
            Outcome::RejectsSeparator | Outcome::RejectsMalformed => {
                let error = parsed.expect_err(&format!("{input} must be rejected, not decoded"));
                assert_eq!(
                    error.category(),
                    ErrorCategory::InvalidReference,
                    "{input} must be rejected as an invalid reference"
                );
            }
        }
    }
}
