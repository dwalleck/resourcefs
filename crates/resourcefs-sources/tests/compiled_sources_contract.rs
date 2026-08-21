use std::{fs, sync::Arc};

use resourcefs_core::{
    DiscoveryEngine, GlobKind, GlobLimits, GlobOptions, GlobRequest, GlobTarget, OperationGuard,
    PathReference, ProjectionSelector, SearchLimits, SearchOptions, SearchRequest, SearchTarget,
    SourceAdapter, WorkspacePath, WorkspaceRootId,
};
use resourcefs_sources::{
    ArtifactSource, BackingPathVisibility, CompiledSources, FilesystemSource, LaunchRoot,
    LaunchRootSource, SessionStore,
};
use tempfile::TempDir;

struct Fixture {
    compiled: CompiledSources,
    artifact_root: String,
    session: resourcefs_sources::StoredSession,
    _cache: TempDir,
    _workspace: TempDir,
}

async fn fixture() -> Fixture {
    let workspace = TempDir::new().expect("workspace");
    fs::write(workspace.path().join("plain.txt"), "workspace bytes\n").expect("workspace file");
    fs::write(
        workspace.path().join("artifact:impostor.txt"),
        "impostor sentinel artifact://-looking text\n",
    )
    .expect("adversarial workspace file");
    let filesystem = FilesystemSource::new(
        LaunchRootSource::Cli(vec![LaunchRoot {
            id: WorkspaceRootId::new("workspace").expect("root ID"),
            path: workspace.path().to_owned(),
        }]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");

    let cache = TempDir::new().expect("session cache");
    let store = SessionStore::open(cache.path())
        .await
        .expect("session store");
    let session = store.create_session().await.expect("stored session");
    let address = session
        .path_session()
        .retain("artifact bytes\n", &OperationGuard::new())
        .await
        .expect("artifact");
    let artifact_root = PathReference::artifact(address, None)
        .expect("artifact root")
        .requested()
        .to_owned();
    let artifacts = ArtifactSource::new(session.path_session().clone());

    Fixture {
        compiled: CompiledSources::new(filesystem, artifacts),
        artifact_root,
        session,
        _cache: cache,
        _workspace: workspace,
    }
}

fn artifact_projection(root: &str, selector: &str) -> PathReference {
    let root = PathReference::parse(root).expect("artifact root");
    let resourcefs_core::ResourceAddress::Artifact(address) = root.address() else {
        panic!("artifact address");
    };
    PathReference::artifact(
        address.clone(),
        Some(ProjectionSelector::parse(selector).expect("selector")),
    )
    .expect("artifact projection")
}

#[tokio::test]
async fn routes_every_reference_by_typed_address_family_only() {
    let fixture = fixture().await;
    let relative_raw = PathReference::parse("plain.txt:raw").expect("relative raw reference");
    let canonical_raw = PathReference::parse("rfs://workspace/workspace/plain.txt:raw")
        .expect("canonical raw reference");
    let canonical_root = PathReference::canonical(
        WorkspaceRootId::new("workspace").expect("root ID"),
        WorkspacePath::new("plain.txt").expect("workspace path"),
    );

    for reference in [relative_raw, canonical_raw, canonical_root] {
        let resource = fixture
            .compiled
            .read(&reference)
            .await
            .expect("workspace route");
        assert_eq!(resource.content(), "workspace bytes\n");
        assert!(
            resource
                .canonical_reference()
                .starts_with("rfs://workspace/")
        );
    }

    for reference in [
        PathReference::parse(&fixture.artifact_root).expect("artifact root"),
        artifact_projection(&fixture.artifact_root, "raw"),
        artifact_projection(&fixture.artifact_root, "1-"),
        artifact_projection(&fixture.artifact_root, "page:2"),
    ] {
        let resource = fixture
            .compiled
            .read(&reference)
            .await
            .expect("artifact route");
        assert!(resource.canonical_reference().starts_with("artifact://"));
        assert!(
            resource.content().ends_with("tifact bytes\n")
                || resource.content() == "artifact bytes\n"
        );
        assert_eq!(resource.backing_file_uri(), None);
    }

    assert_eq!(fixture.session.path_session().artifact_count().await, 1);
}

#[tokio::test]
async fn routes_every_discovery_request_by_typed_family_only() {
    let fixture = fixture().await;
    let discovery = DiscoveryEngine::new(
        Arc::new(fixture.compiled.clone()),
        fixture.session.path_session().clone(),
    );
    let operation = OperationGuard::new();
    let adversarial = "rfs://workspace/workspace/artifact%3Aimpostor.txt";

    // The requested spelling begins with artifact-URI text, but the typed
    // address is Workspace-relative: the Workspace adapter must serve it.
    let exact = discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(
                    PathReference::parse("artifact:impostor.txt").expect("C11 relative reference"),
                ),
                "impostor sentinel",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C11 exact Workspace search request"),
            &operation,
        )
        .await
        .expect("C11 exact Workspace search");
    assert_eq!(
        exact.total_records(),
        1,
        "C11 exact search matches one line"
    );
    assert_eq!(
        exact.groups()[0].reference(),
        adversarial,
        "C11 exact search identity is the canonical Workspace sentinel"
    );
    assert_eq!(
        exact.groups()[0].lines()[0].text(),
        "impostor sentinel artifact://-looking text",
        "C11 exact search line text"
    );

    // Default-primary search stays Workspace even when the pattern itself is
    // artifact-URI text.
    let primary = discovery
        .search(
            SearchRequest::new(
                SearchTarget::primary(),
                "artifact://",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C11 primary Workspace search request"),
            &operation,
        )
        .await
        .expect("C11 primary Workspace search");
    assert_eq!(
        primary.total_records(),
        1,
        "C11 primary search matches one line"
    );
    assert_eq!(
        primary.groups()[0].reference(),
        adversarial,
        "C11 primary search identity is the canonical Workspace sentinel"
    );

    let relative_glob = discovery
        .glob(
            GlobRequest::new(
                GlobTarget::new("*.txt").expect("C11 relative Workspace glob"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &operation,
        )
        .await
        .expect("C11 relative Workspace glob");
    assert_eq!(
        relative_glob.total_records(),
        2,
        "C11 relative glob catalogs both Workspace files"
    );
    let mut workspace_entries: Vec<_> = relative_glob
        .entries()
        .iter()
        .map(|entry| (entry.reference().to_owned(), entry.kind()))
        .collect();
    workspace_entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    assert!(
        workspace_entries.iter().all(|(reference, kind)| {
            reference.starts_with("rfs://workspace/workspace/") && matches!(kind, GlobKind::File)
        }),
        "C11 relative glob returns only Workspace file sentinels"
    );
    assert!(
        workspace_entries
            .iter()
            .any(|(reference, _)| reference == adversarial),
        "C11 relative glob includes the adversarial Workspace file"
    );

    // Artifact-URI-looking text typed Workspace by GlobTarget must still route
    // to the filesystem adapter, matching the encoded canonical spelling.
    let adversarial_glob = discovery
        .glob(
            GlobRequest::new(
                GlobTarget::new("artifact%3A*").expect("C11 adversarial Workspace glob"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &operation,
        )
        .await
        .expect("C11 adversarial Workspace glob");
    assert_eq!(
        adversarial_glob.total_records(),
        1,
        "C11 adversarial glob matches the artifact-named Workspace file"
    );
    assert_eq!(
        adversarial_glob.entries()[0].reference(),
        adversarial,
        "C11 adversarial glob identity is the canonical Workspace sentinel"
    );
    assert_eq!(
        adversarial_glob.entries()[0].kind(),
        GlobKind::File,
        "C11 adversarial glob kind is Workspace file"
    );

    let artifact_reference =
        PathReference::parse(&fixture.artifact_root).expect("C11 artifact root reference");
    let artifact_search = discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(artifact_reference),
                "artifact bytes",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C11 Artifact search request"),
            &operation,
        )
        .await
        .expect("C11 Artifact search");
    assert_eq!(
        artifact_search.total_records(),
        1,
        "C11 Artifact search matches one line"
    );
    assert_eq!(
        artifact_search.groups()[0].reference(),
        fixture.artifact_root.as_str(),
        "C11 Artifact search identity is the canonical Artifact sentinel"
    );

    let artifact_glob = discovery
        .glob(
            GlobRequest::new(
                GlobTarget::new("artifact://*").expect("C11 Artifact glob"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &operation,
        )
        .await
        .expect("C11 Artifact glob");
    assert_eq!(
        artifact_glob.total_records(),
        1,
        "C11 Artifact glob catalogs the current-session Artifact"
    );
    assert_eq!(
        artifact_glob.entries()[0].reference(),
        fixture.artifact_root.as_str(),
        "C11 Artifact glob identity is the canonical Artifact sentinel"
    );
    assert_eq!(
        artifact_glob.entries()[0].kind(),
        GlobKind::Artifact,
        "C11 Artifact glob kind"
    );

    assert!(
        !workspace_entries
            .iter()
            .any(|(reference, _)| reference.starts_with("artifact://")),
        "C11 Workspace globs never leak Artifact identities"
    );
    assert!(
        !artifact_search
            .groups()
            .iter()
            .any(|group| group.reference().starts_with("rfs://workspace/")),
        "C11 Artifact search never leaks Workspace identities"
    );
    assert!(
        !artifact_glob
            .entries()
            .iter()
            .any(|entry| entry.reference().starts_with("rfs://workspace/")),
        "C11 Artifact glob never leaks Workspace identities"
    );

    assert_eq!(
        fixture.session.path_session().artifact_count().await,
        1,
        "C11 discovery retained no recovery Artifacts"
    );
}
