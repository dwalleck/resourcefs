use std::{fs, sync::Arc};

use resourcefs_core::{
    DiscoveryAdapter, DiscoveryEngine, ErrorCategory, GlobKind, GlobLimits, GlobOptions,
    GlobRequest, GlobTarget, MutationAccess, MutationAdapter, OperationGuard, PathReference,
    ProjectionSelector, SearchLimits, SearchOptions, SearchRequest, SearchTarget, SourceAdapter,
    WorkspacePath, WorkspaceRootId,
};
use resourcefs_sources::{
    ArtifactSource, BackingPathVisibility, ClientRoot, CompiledSources, FilesystemSource,
    LaunchRoot, LaunchRootSource, LocalSource, SessionStore,
};
use tempfile::TempDir;

// Unix can represent a literal colon; Windows would interpret it as an ADS.
const ADVERSARIAL_FILENAME: &str = if cfg!(unix) {
    "artifact:impostor.txt"
} else {
    "artifact-impostor.txt"
};

struct Fixture {
    compiled: CompiledSources,
    filesystem: FilesystemSource,
    artifact_root: String,
    session: resourcefs_sources::StoredSession,
    _cache: TempDir,
    _workspace: TempDir,
}

async fn fixture() -> Fixture {
    let workspace = TempDir::new().expect("workspace");
    fs::write(workspace.path().join("plain.txt"), "workspace bytes\n").expect("workspace file");
    fs::write(
        workspace.path().join(ADVERSARIAL_FILENAME),
        "impostor sentinel artifact://-looking text\n",
    )
    .expect("adversarial workspace file");
    let filesystem = FilesystemSource::new(
        LaunchRootSource::Cli(vec![LaunchRoot::read_only(
            WorkspaceRootId::new("workspace").expect("root ID"),
            workspace.path().to_owned(),
        )]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");

    let cache = TempDir::new().expect("session cache");
    let store = SessionStore::open_with(
        resourcefs_sources::SessionStorageConfig::new(
            cache.path(),
            resourcefs_sources::SESSION_CLEANUP_TTL.as_secs() as i64,
        )
        .expect("default session storage config"),
    )
    .await
    .expect("session store");
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("stored session");
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
        compiled: CompiledSources::new(
            filesystem.clone(),
            artifacts,
            LocalSource::new(session.path_session().clone()),
            None,
            None,
            None,
        )
        .await
        .expect("compiled sources"),
        filesystem,
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
async fn explicit_acquisition_controls_are_refused_by_dispatch_and_direct_filesystem() {
    let fixture = fixture().await;
    let controls = resourcefs_core::ReadAcquisitionLimits::default();
    for spelling in [
        "plain.txt",
        fixture.artifact_root.as_str(),
        "local://",
        "rfs://",
        "rfs://workspace",
    ] {
        let reference = PathReference::parse(spelling).expect("supported reference");
        fixture
            .compiled
            .read(&reference, &OperationGuard::new(), None)
            .await
            .expect("absent controls retain supported reads");
        let error = fixture
            .compiled
            .read(&reference, &OperationGuard::new(), Some(&controls))
            .await
            .expect_err("explicit controls cannot disappear in dispatch");
        assert_eq!(error.category(), ErrorCategory::UnsupportedProjection);
        assert_eq!(
            error.details().expect("typed refusal").reason(),
            resourcefs_core::ErrorReason::AcquisitionControlsUnsupported
        );
    }
    let reference = PathReference::parse("plain.txt").expect("workspace reference");
    let error = fixture
        .filesystem
        .read(&reference, &OperationGuard::new(), Some(&controls))
        .await
        .expect_err("direct filesystem controls");
    assert_eq!(error.category(), ErrorCategory::UnsupportedProjection);
    assert_eq!(
        error.details().expect("typed refusal").reason(),
        resourcefs_core::ErrorReason::AcquisitionControlsUnsupported
    );
}

#[tokio::test]
async fn unserved_github_family_is_refused_before_configuration() {
    let fixture = fixture().await;
    let reference = PathReference::parse(
        "github://owner/repo/commits/0123456789abcdef0123456789abcdef01234567/facts",
    )
    .expect("immutable commit reference");
    // No GitHub source is mounted here; the answer must still be about the
    // address family, not about the missing profile entry.
    let error = fixture
        .compiled
        .read(&reference, &OperationGuard::new(), None)
        .await
        .expect_err("no compiled source serves github://");
    assert_eq!(error.category(), ErrorCategory::UnsupportedProjection);
    assert!(error.details().is_none());
    // Caller controls cannot reword the refusal: the family, not the control
    // set, is what this build cannot serve.
    let controlled = fixture
        .compiled
        .read(
            &reference,
            &OperationGuard::new(),
            Some(&resourcefs_core::ReadAcquisitionLimits::default()),
        )
        .await
        .expect_err("controls cannot change an unserved family");
    assert_eq!(controlled.category(), ErrorCategory::UnsupportedProjection);
    let mutation = fixture
        .compiled
        .resolve(&reference, MutationAccess::Update)
        .await
        .expect_err("immutable facts accept no mutation");
    assert_eq!(mutation.category(), ErrorCategory::UnsupportedMutation);
    let target = SearchTarget::resource(reference.clone());
    let search = fixture
        .compiled
        .search(
            &target,
            "needle",
            SearchOptions::default(),
            &OperationGuard::new(),
        )
        .await
        .expect_err("no compiled source discovers github://");
    assert_eq!(search.category(), ErrorCategory::UnsupportedProjection);
}

#[tokio::test]
async fn catalogs_and_artifacts_are_immutable_mutation_targets() {
    let fixture = fixture().await;
    for spelling in ["rfs://", "rfs://workspace", fixture.artifact_root.as_str()] {
        let reference = PathReference::parse(spelling).expect("immutable reference");
        let error = fixture
            .compiled
            .resolve(&reference, MutationAccess::Update)
            .await
            .expect_err("immutable mutation target");
        assert_eq!(
            error.category(),
            ErrorCategory::PermissionDenied,
            "{spelling}"
        );
    }
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
            .read(&reference, &OperationGuard::new(), None)
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
            .read(&reference, &OperationGuard::new(), None)
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
async fn unmounted_jira_routes_fail_honestly_without_fallthrough() {
    let fixture = fixture().await;
    let reference = PathReference::parse("jira://acme/issues/10001").expect("Jira reference");
    let operation = OperationGuard::new();

    let read = fixture
        .compiled
        .read(&reference, &operation, None)
        .await
        .expect_err("no Atlassian source is mounted");
    assert_eq!(read.category(), ErrorCategory::SourceUnavailable);

    let mutation = fixture
        .compiled
        .resolve(&reference, MutationAccess::Update)
        .await
        .expect_err("read-only Jira Resource");
    assert_eq!(mutation.category(), ErrorCategory::UnsupportedMutation);

    let search = fixture
        .compiled
        .search(
            &SearchTarget::resource(reference),
            "sentinel",
            SearchOptions::default(),
            &operation,
        )
        .await
        .expect_err("no Atlassian discovery Adapter is mounted");
    assert_eq!(search.category(), ErrorCategory::SourceUnavailable);
}

#[tokio::test]
async fn catalog_reads_route_by_typed_address_family() {
    let fixture = fixture().await;
    let sources = fixture
        .compiled
        .read(
            &PathReference::parse("rfs://").expect("source catalog"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("source catalog read");
    assert_eq!(sources.canonical_reference(), "rfs://");
    assert!(!sources.is_mutable());
    assert_eq!(
        sources.content(),
        concat!(
            "Mounted sources\n",
            "Next discovery step: rfs_read rfs://workspace\n",
            "Selectors: :N | :N-M | :N- | comma-separated ranges | :raw | :page:N\n",
            "artifact:// — artifact://<session>-<id>[:selector] — artifact://00000000000000000000000000000000-1\nlocal:// — local://<name>[:selector] (flat names; bare local:// lists this Path Session\'s scratch) — local://plan.md\n",
            "rfs://workspace — <relative-path> | rfs://workspace/<root>/<path>[:selector] | file://<absolute-path> (relative paths use the Primary Workspace Root) — rfs://workspace/workspace/src/lib.rs\n",
        )
    );

    let canonical = fixture
        .compiled
        .read(
            &PathReference::parse("rfs://workspace").expect("workspace catalog"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("workspace catalog read");
    let alias = fixture
        .compiled
        .read(
            &PathReference::parse("rfs://workspace/").expect("workspace alias"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("workspace alias read");
    assert_eq!(canonical, alias);
    assert_eq!(canonical.canonical_reference(), "rfs://workspace");
    assert_eq!(
        canonical.content(),
        "rfs://workspace/workspace/ (primary)\n"
    );
    assert!(!canonical.is_mutable());
}

#[tokio::test]
async fn workspace_catalog_snapshot_never_mixes_root_generations() {
    let fixture = fixture().await;
    let client = TempDir::new().expect("client root");
    fs::write(client.path().join("plain.txt"), "client bytes\n").expect("client file");
    let client_uri = url::Url::from_directory_path(client.path())
        .expect("client root URI")
        .to_string();

    let refresh = fixture.filesystem.begin_client_root_refresh().await;
    let refreshing = fixture
        .compiled
        .read(
            &PathReference::parse("rfs://workspace").expect("workspace catalog"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect_err("refreshing authority must not deliver a mixed catalog");
    assert_eq!(refreshing.category(), ErrorCategory::SourceUnavailable);

    let acquisition = fixture.filesystem.start_client_root_acquisition(refresh);
    fixture
        .filesystem
        .complete_client_root_refresh(
            acquisition,
            vec![ClientRoot {
                uri: client_uri,
                name: Some("client".to_owned()),
            }],
        )
        .await
        .expect("client root replacement");

    let file_uri = url::Url::from_file_path(client.path().join("plain.txt"))
        .expect("client file URI")
        .to_string();
    let file = fixture
        .compiled
        .read(
            &PathReference::parse(file_uri).expect("client file reference"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("client file read");
    let root_reference = file
        .canonical_reference()
        .strip_suffix("plain.txt")
        .expect("canonical root prefix");
    let catalog = fixture
        .compiled
        .read(
            &PathReference::parse("rfs://workspace").expect("workspace catalog"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("replacement catalog");
    assert_eq!(catalog.content(), format!("{root_reference} (primary)\n"));
    assert!(!catalog.content().contains("rfs://workspace/workspace/"));
}
#[tokio::test]
async fn routes_every_discovery_request_by_typed_family_only() {
    let fixture = fixture().await;
    let discovery = DiscoveryEngine::new(
        Arc::new(fixture.compiled.clone()),
        fixture.session.path_session().clone(),
        resourcefs_core::ServerLimits::default(),
    );
    let operation = OperationGuard::new();
    let adversarial = if cfg!(unix) {
        "rfs://workspace/workspace/artifact%3Aimpostor.txt"
    } else {
        "rfs://workspace/workspace/artifact-impostor.txt"
    };

    // The artifact-named file is Workspace-relative even though its contents
    // look like an Artifact URI. Unix also exercises a literal colon in its name.
    let exact = discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(
                    PathReference::parse(ADVERSARIAL_FILENAME).expect("C11 relative reference"),
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

    // The artifact-named glob must route to the filesystem adapter, including
    // the encoded colon spelling on Unix.
    let adversarial_glob = discovery
        .glob(
            GlobRequest::new(
                GlobTarget::new(if cfg!(unix) {
                    "artifact%3A*"
                } else {
                    "artifact-*"
                })
                .expect("C11 adversarial Workspace glob"),
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
