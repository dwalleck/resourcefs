use std::fs;

use resourcefs_core::{
    OperationGuard, PathReference, ProjectionSelector, SourceAdapter, WorkspacePath,
    WorkspaceRootId,
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
