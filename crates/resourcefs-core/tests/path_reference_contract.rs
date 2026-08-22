use std::{
    path::Path,
    time::{Duration, Instant},
};

use resourcefs_core::{
    ErrorCategory, MAX_PATH_REFERENCE_BYTES, PathReference, WorkspaceAddress, WorkspacePath,
    WorkspaceRoot, WorkspaceRootId, WorkspaceRootSet,
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
        "file:///workspace/alpha",
        Some("chosen".to_owned()),
    )
    .expect("root");
    let beta = WorkspaceRoot::new(
        WorkspaceRootId::new("beta").expect("id"),
        "file:///workspace/beta",
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
        "file:///workspace/client",
        Some("bad/name".to_owned()),
    )
    .expect_err("selector name must use Workspace Root ID grammar");
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
