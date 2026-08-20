use resourcefs_core::{
    ErrorCategory, OperationGuard, PathReference, ProjectionSelector, SourceAdapter, VersionTag,
    WorkspacePath, WorkspaceRootId,
};
use resourcefs_sources::{ArtifactSource, SessionStore};
use tempfile::TempDir;

struct Fixture {
    session: resourcefs_sources::StoredSession,
    source: ArtifactSource,
    root: String,
    content: String,
    _temporary: TempDir,
}

async fn fixture() -> Fixture {
    let temporary = TempDir::new().expect("temporary session cache");
    let store = SessionStore::open(temporary.path())
        .await
        .expect("session store");
    let session = store.create_session().await.expect("stored session");
    let content = "αlpha\r\nbeta\ngamma\r\ndelta".to_owned();
    let address = session
        .path_session()
        .retain(&content, &OperationGuard::new())
        .await
        .expect("artifact");
    let root = PathReference::artifact(address, None)
        .expect("artifact root")
        .requested()
        .to_owned();
    let source = ArtifactSource::new(session.path_session().clone());
    Fixture {
        _temporary: temporary,
        session,
        source,
        root,
        content,
    }
}

fn selected(root: &str, selector: &str) -> PathReference {
    let root = PathReference::parse(root).expect("artifact root");
    let resourcefs_core::ResourceAddress::Artifact(address) = root.address() else {
        panic!("artifact root");
    };
    PathReference::artifact(
        address.clone(),
        Some(ProjectionSelector::parse(selector).expect("selector")),
    )
    .expect("selected artifact")
}

#[tokio::test]
async fn artifact_root_raw_ranges_and_pages_match_immutable_bytes() {
    let fixture = fixture().await;
    let expected_tag = VersionTag::from_content(fixture.content.as_bytes());

    let root = fixture
        .source
        .read(&PathReference::parse(&fixture.root).expect("root reference"))
        .await
        .expect("root read");
    assert_eq!(root.canonical_reference(), fixture.root);
    assert_eq!(root.content(), fixture.content);
    assert_eq!(root.version_tag(), &expected_tag);
    assert_eq!(root.backing_file_uri(), None);
    assert!(!root.is_mutable());
    assert_eq!(
        root.artifact_origin(),
        Some(resourcefs_core::ArtifactProjectionOrigin::new(0, 1))
    );

    let raw = fixture
        .source
        .read(&selected(&fixture.root, "raw"))
        .await
        .expect("raw read");
    assert_eq!(raw.content(), fixture.content);
    assert_eq!(raw.version_tag(), &expected_tag);

    let ranges = fixture
        .source
        .read(&selected(&fixture.root, "3-3,1-2,3-3"))
        .await
        .expect("range read");
    assert_eq!(ranges.content(), "gamma\r\nαlpha\r\nbeta\ngamma\r\n");
    assert_eq!(ranges.version_tag(), &expected_tag);
    assert_eq!(ranges.artifact_origin(), None);

    let suffix = fixture
        .source
        .read(&selected(&fixture.root, "2-"))
        .await
        .expect("suffix read");
    assert_eq!(suffix.content(), "beta\ngamma\r\ndelta");
    assert_eq!(
        suffix.artifact_origin(),
        Some(resourcefs_core::ArtifactProjectionOrigin::new(
            "αlpha\r\n".len(),
            2,
        ))
    );

    let page = fixture
        .source
        .read(&selected(&fixture.root, "page:2"))
        .await
        .expect("page read");
    assert_eq!(page.content(), &fixture.content[2..]);
    assert_eq!(page.version_tag(), &expected_tag);
    assert_eq!(
        page.artifact_origin(),
        Some(resourcefs_core::ArtifactProjectionOrigin::new(2, 1))
    );
    assert!(!format!("{page:?}").contains(fixture._temporary.path().to_string_lossy().as_ref()));
}

#[tokio::test]
async fn invalid_page_and_non_artifact_authority_fail_without_disclosure() {
    let fixture = fixture().await;
    let invalid_boundary = fixture
        .source
        .read(&selected(&fixture.root, "page:1"))
        .await
        .expect_err("UTF-8 midpoint");
    assert_eq!(invalid_boundary.category(), ErrorCategory::InvalidReference);

    let end = fixture.content.len();
    let at_end = fixture
        .source
        .read(&selected(&fixture.root, &format!("page:{end}")))
        .await
        .expect_err("end is not a progressing page");
    assert_eq!(at_end.category(), ErrorCategory::InvalidReference);

    let workspace = PathReference::canonical(
        WorkspaceRootId::new("workspace").expect("root ID"),
        WorkspacePath::new("file.txt").expect("workspace path"),
    );
    let wrong_source = fixture
        .source
        .read(&workspace)
        .await
        .expect_err("workspace authority");
    assert_eq!(
        wrong_source.category(),
        ErrorCategory::UnsupportedProjection
    );
    assert!(
        !wrong_source
            .message()
            .contains(fixture._temporary.path().to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn disconnect_invalidates_all_artifact_projections() {
    let fixture = fixture().await;
    fixture.session.path_session().invalidate();
    for reference in [
        PathReference::parse(&fixture.root).expect("root"),
        selected(&fixture.root, "raw"),
        selected(&fixture.root, "2-"),
        selected(&fixture.root, "page:2"),
    ] {
        let error = fixture
            .source
            .read(&reference)
            .await
            .expect_err("inactive artifact");
        assert_eq!(error.category(), ErrorCategory::NotFound);
    }
}
