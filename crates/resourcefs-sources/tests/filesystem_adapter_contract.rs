use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    time::{Duration, Instant},
};

use resourcefs_core::{
    ErrorCategory, MAX_TEXT_BYTES, MAX_WORKSPACE_ROOTS, PathReference, SourceAdapter,
    WorkspaceRootId,
};
use resourcefs_sources::{
    BackingPathVisibility, ClientRoot, FilesystemSource, LaunchRoot, LaunchRootSource,
    RootRefreshOutcome,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn reference(path: &str) -> PathReference {
    PathReference::parse(path).expect("fixture reference should be valid")
}

fn create_root() -> (TempDir, std::path::PathBuf) {
    let temporary = TempDir::new().expect("temporary directory");
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).expect("workspace root");
    (temporary, root)
}

#[tokio::test]
async fn reads_utf8_empty_and_unicode_resources() {
    let (_temporary, root) = create_root();
    fs::create_dir(root.join("notes")).expect("notes directory");
    fs::write(root.join("notes/ résumé final.txt "), "héllo\n").expect("unicode fixture");
    fs::write(root.join("empty.txt"), "").expect("empty fixture");
    let source = single_source(&root).await.expect("filesystem source");

    let unicode = source
        .read(&reference("notes/ résumé final.txt "))
        .await
        .expect("unicode read");
    assert_eq!(unicode.content(), "héllo\n");
    assert_eq!(
        unicode.canonical_reference(),
        "rfs://workspace/workspace/notes/ résumé final.txt "
    );

    let empty = source
        .read(&reference("empty.txt"))
        .await
        .expect("empty read");
    assert_eq!(empty.content(), "");
}

#[tokio::test]
async fn maps_missing_directory_and_binary_resources() {
    let (_temporary, root) = create_root();
    fs::create_dir(root.join("directory")).expect("directory fixture");
    fs::write(root.join("binary.bin"), [0xff, 0xfe]).expect("binary fixture");
    let source = single_source(&root).await.expect("filesystem source");

    let missing = source
        .read(&reference("missing.txt"))
        .await
        .expect_err("missing should fail");
    assert_eq!(missing.category(), ErrorCategory::NotFound);

    let directory = source
        .read(&reference("directory"))
        .await
        .expect_err("directory should fail");
    assert_eq!(directory.category(), ErrorCategory::UnsupportedProjection);

    let binary = source
        .read(&reference("binary.bin"))
        .await
        .expect_err("binary should fail");
    assert_eq!(binary.category(), ErrorCategory::UnsupportedProjection);
}

#[tokio::test]
async fn rejects_invalid_root_configuration() {
    let temporary = TempDir::new().expect("temporary directory");
    let missing = single_source(&temporary.path().join("missing"))
        .await
        .expect_err("missing root should fail");
    assert_eq!(missing.category(), ErrorCategory::SourceUnavailable);

    let file = temporary.path().join("file.txt");
    fs::write(&file, "not a directory").expect("file root fixture");
    let not_directory = single_source(&file)
        .await
        .expect_err("file root should fail");
    assert_eq!(not_directory.category(), ErrorCategory::SourceUnavailable);
    let alpha = temporary.path().join("alpha");
    let beta = temporary.path().join("beta");
    fs::create_dir(&alpha).expect("alpha root");
    fs::create_dir(&beta).expect("beta root");
    let duplicate_id = launch_source(
        vec![launch_root("same", &alpha), launch_root("same", &beta)],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
    .expect_err("duplicate root IDs must fail");
    assert_eq!(duplicate_id.category(), ErrorCategory::InvalidReference);

    let duplicate_uri = launch_source(
        vec![launch_root("alpha", &alpha), launch_root("alias", &alpha)],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
    .expect_err("duplicate canonical roots must fail");
    assert_eq!(duplicate_uri.category(), ErrorCategory::InvalidReference);

    let client = temporary.path().join("client");
    fs::create_dir(&client).expect("client root");
    let client_uri =
        url::Url::from_directory_path(fs::canonicalize(&client).expect("canonical client root"))
            .expect("client root URI")
            .to_string();
    let colliding_id = WorkspaceRootId::new(format!(
        "client-{:x}",
        Sha256::digest(client_uri.as_bytes())
    ))
    .expect("derived client ID");
    let source = launch_source(
        vec![LaunchRoot {
            id: colliding_id,
            path: alpha,
        }],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("collision launch source");
    let refresh = source.begin_client_root_refresh().await;
    let collision = source
        .complete_client_root_refresh(
            source.start_client_root_acquisition(refresh),
            vec![ClientRoot {
                uri: client_uri,
                name: None,
            }],
        )
        .await
        .expect_err("one connection cannot remap a canonical root ID");
    assert_eq!(collision.category(), ErrorCategory::InvalidReference);
}

#[cfg(unix)]
#[tokio::test]
async fn permits_contained_symlink() {
    use std::os::unix::fs::symlink;

    let (_temporary, root) = create_root();
    fs::create_dir(root.join("sub")).expect("subdirectory fixture");
    fs::write(root.join("target.txt"), "inside").expect("inside fixture");
    symlink(root.join("target.txt"), root.join("link.txt")).expect("contained symlink");
    symlink(
        root.join("sub/../target.txt"),
        root.join("normalized-link.txt"),
    )
    .expect("contained normalizing symlink");
    let source = single_source(&root).await.expect("filesystem source");

    for link in ["link.txt", "normalized-link.txt"] {
        let resource = source
            .read(&reference(link))
            .await
            .expect("contained link read");
        assert_eq!(resource.content(), "inside");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let (temporary, root) = create_root();
    let outside = temporary.path().join("outside.txt");
    fs::write(&outside, "outside secret").expect("outside fixture");
    symlink(&outside, root.join("escape.txt")).expect("escaping symlink");
    let source = single_source(&root).await.expect("filesystem source");

    let error = source
        .read(&reference("escape.txt"))
        .await
        .expect_err("escaping link should fail");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    assert!(!error.message().contains("outside secret"));
}

fn exact_byte_limit_content() -> Vec<u8> {
    let mut content = Vec::with_capacity(MAX_TEXT_BYTES);
    for _ in 0..96 {
        content.extend(std::iter::repeat_n(b'x', 511));
        content.push(b'\n');
    }
    assert_eq!(content.len(), MAX_TEXT_BYTES);
    content
}

async fn assert_limit_exceeded(source: &FilesystemSource, path: &str) {
    let error = source
        .read(&reference(path))
        .await
        .expect_err("one-over fixture should fail");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
}

#[tokio::test]
async fn enforces_hard_read_limits() {
    let (_temporary, root) = create_root();
    let exact = exact_byte_limit_content();
    fs::write(root.join("exact.txt"), &exact).expect("exact-byte fixture");

    let mut bytes_over = exact.clone();
    bytes_over.push(b'x');
    fs::write(root.join("bytes-over.txt"), bytes_over).expect("byte-over fixture");
    fs::write(root.join("lines-over.txt"), "x\n".repeat(3_001)).expect("line-over fixture");
    fs::write(root.join("columns-over.txt"), "x".repeat(513)).expect("column-over fixture");

    let source = single_source(&root).await.expect("filesystem source");
    let exact_resource = source
        .read(&reference("exact.txt"))
        .await
        .expect("exact limit should pass");
    assert_eq!(exact_resource.content().as_bytes(), exact);

    assert_limit_exceeded(&source, "bytes-over.txt").await;
    assert_limit_exceeded(&source, "lines-over.txt").await;
    assert_limit_exceeded(&source, "columns-over.txt").await;
}

#[tokio::test]
async fn reads_maximum_sized_file_within_budget() {
    let (_temporary, root) = create_root();
    let exact = exact_byte_limit_content();
    fs::write(root.join("maximum.txt"), &exact).expect("maximum fixture");
    let source = single_source(&root).await.expect("filesystem source");

    let started = Instant::now();
    let resource = source
        .read(&reference("maximum.txt"))
        .await
        .expect("maximum read");
    let elapsed = started.elapsed();

    assert_eq!(resource.content().as_bytes(), exact);
    assert!(
        elapsed <= Duration::from_millis(100),
        "maximum-sized read took {elapsed:?}"
    );
}

#[tokio::test]
async fn filesystem_streams_narrow_large_range() {
    const SOURCE_BYTES: u64 = 256 * 1024 * 1024;
    const TARGET: &str = "unique target π\r\n";
    const EXPECTED_TAG: &str =
        "sha256:7340d5b10ba225d1a0dd49203cbdae47afc05a4937227033b728f881b32d4f5e";

    let (_temporary, root) = create_root();
    let path = root.join("large.txt");
    let mut file = fs::File::create(&path).expect("large source fixture");
    file.set_len(SOURCE_BYTES - TARGET.len() as u64 - 1)
        .expect("sparse source prefix");
    file.seek(SeekFrom::End(0)).expect("source end");
    file.write_all(b"\n").expect("first line terminator");
    file.write_all(TARGET.as_bytes())
        .expect("unique final line");
    drop(file);
    assert_eq!(
        fs::metadata(&path).expect("large source metadata").len(),
        SOURCE_BYTES
    );
    let expected_tag = EXPECTED_TAG;
    let source = single_source(&root).await.expect("filesystem source");

    let full_error = source
        .read(&reference("large.txt"))
        .await
        .expect_err("complete large source exceeds the inline source limit");
    assert_eq!(full_error.category(), ErrorCategory::LimitExceeded);

    let started = Instant::now();
    let selected = source
        .read(&reference("large.txt:2"))
        .await
        .expect("narrow selection from large source");
    let elapsed = started.elapsed();
    assert_eq!(selected.content(), TARGET);
    assert_eq!(selected.version_tag().as_str(), expected_tag);
    assert!(selected.content().len() <= 70 * 1024 * 1024);
    if !cfg!(debug_assertions) {
        assert!(
            elapsed <= Duration::from_secs(10),
            "256 MiB selection took {elapsed:?}"
        );
    }
}

#[cfg(feature = "test-support")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_stream_delivery_is_rejected() {
    let temporary = TempDir::new().expect("temporary directory");
    let original = temporary.path().join("original");
    let replacement = temporary.path().join("replacement");
    fs::create_dir(&original).expect("original root");
    fs::create_dir(&replacement).expect("replacement root");
    fs::write(original.join("stale-stream-delivery.txt"), "alpha\nbeta\n")
        .expect("stale read fixture");
    let source = launch_source(
        vec![launch_root("workspace", &original)],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");
    let gate = source
        .arm_test_delivery_gate("rfs://workspace/workspace/stale-stream-delivery.txt")
        .await;
    let reader = source.clone();
    let pending =
        tokio::spawn(async move { reader.read(&reference("stale-stream-delivery.txt:2")).await });
    tokio::time::timeout(Duration::from_secs(10), gate.wait_until_entered())
        .await
        .expect("read reaches delivery gate");

    let refresh = source.begin_client_root_refresh().await;
    source
        .complete_client_root_refresh(
            source.start_client_root_acquisition(refresh),
            vec![client_root(&replacement, "replacement")],
        )
        .await
        .expect("replace root authority");
    gate.release();

    let error = pending
        .await
        .expect("read task")
        .expect_err("stale selected content must not be delivered");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
}

fn launch_root(id: &str, path: &std::path::Path) -> LaunchRoot {
    LaunchRoot {
        id: WorkspaceRootId::new(id).expect("fixture root ID"),
        path: path.to_owned(),
    }
}

async fn launch_source(
    roots: Vec<LaunchRoot>,
    primary: Option<&str>,
    visibility: BackingPathVisibility,
) -> Result<FilesystemSource, resourcefs_core::ResourceError> {
    FilesystemSource::new(
        LaunchRootSource::Cli(roots),
        primary.map(str::to_owned),
        visibility,
    )
    .await
}

async fn single_source(
    root: &std::path::Path,
) -> Result<FilesystemSource, resourcefs_core::ResourceError> {
    launch_source(
        vec![launch_root("workspace", root)],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
}

#[tokio::test]
async fn relative_reads_require_unique_primary() {
    let temporary = TempDir::new().expect("temporary directory");
    let alpha = temporary.path().join("alpha");
    let beta = temporary.path().join("beta");
    fs::create_dir(&alpha).expect("alpha root");
    fs::create_dir(&beta).expect("beta root");
    fs::write(alpha.join("shared.txt"), "alpha").expect("alpha fixture");
    fs::write(beta.join("shared.txt"), "beta").expect("beta fixture");
    let roots = || vec![launch_root("alpha", &alpha), launch_root("beta", &beta)];

    let no_primary = launch_source(roots(), None, BackingPathVisibility::Hidden)
        .await
        .expect("multi-root source");
    let ambiguous = no_primary
        .read(&reference("shared.txt"))
        .await
        .expect_err("relative read without a primary must fail");
    assert_eq!(ambiguous.category(), ErrorCategory::AmbiguousReference);

    for (id, content) in [("alpha", "alpha"), ("beta", "beta")] {
        let canonical = format!("rfs://workspace/{id}/shared.txt");
        let resource = no_primary
            .read(&reference(&canonical))
            .await
            .expect("canonical read");
        assert_eq!(resource.content(), content);
        assert_eq!(resource.canonical_reference(), canonical);
    }

    let selected = launch_source(roots(), Some("beta"), BackingPathVisibility::Hidden)
        .await
        .expect("selected source");
    assert_eq!(
        selected
            .read(&reference("shared.txt"))
            .await
            .expect("selected relative read")
            .content(),
        "beta"
    );

    let unmatched = launch_source(roots(), Some("missing"), BackingPathVisibility::Hidden)
        .await
        .expect("unmatched primary disables relative reads");
    let error = unmatched
        .read(&reference("shared.txt"))
        .await
        .expect_err("unmatched primary must fail");
    assert_eq!(error.category(), ErrorCategory::AmbiguousReference);

    let unknown = no_primary
        .read(&reference("rfs://workspace/missing/shared.txt"))
        .await
        .expect_err("unknown canonical root must fail");
    assert_eq!(unknown.category(), ErrorCategory::InvalidReference);
}

fn client_root(path: &std::path::Path, name: &str) -> ClientRoot {
    ClientRoot {
        uri: url::Url::from_directory_path(path)
            .expect("client root file URI")
            .to_string(),
        name: Some(name.to_owned()),
    }
}

#[tokio::test]
async fn client_root_refresh_replaces_and_restores_launch_authority() {
    let temporary = TempDir::new().expect("temporary directory");
    let launch = temporary.path().join("launch");
    let alpha = temporary.path().join("alpha");
    let beta = temporary.path().join("beta");
    for root in [&launch, &alpha, &beta] {
        fs::create_dir(root).expect("root fixture");
    }
    fs::write(launch.join("shared.txt"), "launch").expect("launch fixture");
    fs::write(alpha.join("shared.txt"), "alpha").expect("alpha fixture");
    fs::write(beta.join("shared.txt"), "beta").expect("beta fixture");
    let source = launch_source(
        vec![launch_root("launch", &launch)],
        Some("beta"),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("launch source");
    let client_roots = vec![client_root(&alpha, "alpha"), client_root(&beta, "beta")];

    let refresh = source.begin_client_root_refresh().await;
    let suspended = source
        .read(&reference("shared.txt"))
        .await
        .expect_err("reads must suspend during refresh");
    assert_eq!(suspended.category(), ErrorCategory::SourceUnavailable);
    let outcome = source
        .complete_client_root_refresh(
            source.start_client_root_acquisition(refresh),
            client_roots.clone(),
        )
        .await
        .expect("client refresh");
    assert!(matches!(
        outcome,
        RootRefreshOutcome::Applied {
            generation: 2,
            ref removed,
            ref active,
        } if removed == &[WorkspaceRootId::new("launch").expect("launch root ID")]
            && active.len() == 2
    ));
    let selected = source
        .read(&reference("shared.txt"))
        .await
        .expect("selected client root");
    assert_eq!(selected.content(), "beta");
    assert!(
        selected
            .canonical_reference()
            .starts_with("rfs://workspace/client-")
    );
    assert!(!selected.canonical_reference().contains("beta"));
    let selected_canonical = selected.canonical_reference().to_owned();

    let refresh = source.begin_client_root_refresh().await;
    let mut reordered = client_roots;
    reordered.reverse();
    assert_eq!(
        source
            .complete_client_root_refresh(source.start_client_root_acquisition(refresh), reordered,)
            .await
            .expect("equivalent client refresh"),
        RootRefreshOutcome::Unchanged { generation: 2 }
    );
    assert_eq!(
        source
            .read(&reference("shared.txt"))
            .await
            .expect("stable selected client root")
            .canonical_reference(),
        selected_canonical
    );

    let refresh = source.begin_client_root_refresh().await;
    let outcome = source
        .complete_client_root_refresh(source.start_client_root_acquisition(refresh), Vec::new())
        .await
        .expect("empty client roots restore launch roots");
    assert!(matches!(
        outcome,
        RootRefreshOutcome::Applied {
            generation: 3,
            ref removed,
            ..
        } if removed.len() == 2
    ));
    assert_eq!(
        source
            .read(&reference("shared.txt"))
            .await
            .expect("restored launch root")
            .content(),
        "launch"
    );
}

#[tokio::test(start_paused = true)]
async fn parked_root_refresh_does_not_expire_before_acquisition_starts() {
    let (_temporary, root) = create_root();
    fs::write(root.join("fixture.txt"), "fixture").expect("fixture");
    let source = single_source(&root).await.expect("filesystem source");

    let parked = source.begin_client_root_refresh().await;
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    let outcome = source
        .complete_client_root_refresh(source.start_client_root_acquisition(parked), Vec::new())
        .await
        .expect("parked refresh remains valid before acquisition");
    assert_eq!(outcome, RootRefreshOutcome::Unchanged { generation: 1 });
}

#[tokio::test(start_paused = true)]
async fn expired_root_refresh_disables_authority_and_cannot_clobber_recovery() {
    let (_temporary, root) = create_root();
    fs::write(root.join("fixture.txt"), "fixture").expect("fixture");
    let source = single_source(&root).await.expect("filesystem source");

    let expired = source.start_client_root_acquisition(source.begin_client_root_refresh().await);
    tokio::time::advance(Duration::from_secs(5)).await;
    tokio::task::yield_now().await;
    let disabled = source
        .read(&reference("fixture.txt"))
        .await
        .expect_err("expired acquisition must disable authority");
    assert_eq!(disabled.category(), ErrorCategory::SourceUnavailable);

    let recovery = source.begin_client_root_refresh().await;
    let late = source
        .complete_client_root_refresh(expired, Vec::new())
        .await
        .expect_err("late completion must fail");
    assert_eq!(late.category(), ErrorCategory::SourceUnavailable);
    assert_eq!(
        source
            .complete_client_root_refresh(
                source.start_client_root_acquisition(recovery),
                Vec::new(),
            )
            .await
            .expect("newer refresh recovers launch roots"),
        RootRefreshOutcome::Applied {
            generation: 2,
            removed: Vec::new(),
            active: vec![WorkspaceRootId::new("workspace").expect("root ID")],
        }
    );
    assert_eq!(
        source
            .read(&reference("fixture.txt"))
            .await
            .expect("recovered authority")
            .content(),
        "fixture"
    );
}

#[tokio::test]
async fn client_display_names_select_but_never_define_identity() {
    let temporary = TempDir::new().expect("temporary directory");
    let launch = temporary.path().join("launch");
    let alpha = temporary.path().join("alpha");
    let beta = temporary.path().join("beta");
    for root in [&launch, &alpha, &beta] {
        fs::create_dir(root).expect("root fixture");
    }
    fs::write(alpha.join("shared.txt"), "alpha").expect("alpha fixture");
    fs::write(beta.join("shared.txt"), "beta").expect("beta fixture");
    let source = launch_source(
        vec![launch_root("launch", &launch)],
        Some("selected"),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("launch source");

    let refresh = source.begin_client_root_refresh().await;
    source
        .complete_client_root_refresh(
            source.start_client_root_acquisition(refresh),
            vec![client_root(&alpha, "alpha"), client_root(&beta, "selected")],
        )
        .await
        .expect("valid client roots");
    assert_eq!(
        source
            .read(&reference("shared.txt"))
            .await
            .expect("unique valid display-name match")
            .content(),
        "beta"
    );
    let alpha_absolute = alpha.join("shared.txt").to_string_lossy().into_owned();
    let alpha_identity = source
        .read(&reference(&alpha_absolute))
        .await
        .expect("alpha absolute read")
        .canonical_reference()
        .to_owned();

    let refresh = source.begin_client_root_refresh().await;
    source
        .complete_client_root_refresh(
            source.start_client_root_acquisition(refresh),
            vec![
                client_root(&beta, "selected"),
                client_root(&alpha, "selected"),
            ],
        )
        .await
        .expect("duplicate display names remain valid");
    assert_eq!(
        source
            .read(&reference("shared.txt"))
            .await
            .expect_err("duplicate primary names are ambiguous")
            .category(),
        ErrorCategory::AmbiguousReference
    );
    assert_eq!(
        source
            .read(&reference(&alpha_absolute))
            .await
            .expect("alpha identity after display-name change")
            .canonical_reference(),
        alpha_identity
    );
}

#[tokio::test]
async fn invalid_client_display_name_fails_closed() {
    let temporary = TempDir::new().expect("temporary directory");
    let launch = temporary.path().join("launch");
    let client = temporary.path().join("client");
    fs::create_dir(&launch).expect("launch root");
    fs::create_dir(&client).expect("client root");
    fs::write(launch.join("fixture.txt"), "launch").expect("launch fixture");
    let source = launch_source(
        vec![launch_root("launch", &launch)],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("launch source");

    let refresh = source.begin_client_root_refresh().await;
    let error = source
        .complete_client_root_refresh(
            source.start_client_root_acquisition(refresh),
            vec![client_root(&client, "bad/name")],
        )
        .await
        .expect_err("invalid client display name must reject the root set");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    let disabled = source
        .read(&reference("fixture.txt"))
        .await
        .expect_err("invalid root set must fail closed");
    assert_eq!(disabled.category(), ErrorCategory::SourceUnavailable);
}

#[tokio::test]
async fn failed_and_superseded_refreshes_fail_closed() {
    let (_temporary, root) = create_root();
    fs::write(root.join("fixture.txt"), "fixture").expect("fixture");
    let source = single_source(&root).await.expect("filesystem source");

    let first = source.begin_client_root_refresh().await;
    let second = source.begin_client_root_refresh().await;
    assert_eq!(
        source
            .complete_client_root_refresh(source.start_client_root_acquisition(first), Vec::new(),)
            .await
            .expect("stale completion"),
        RootRefreshOutcome::Superseded
    );
    assert_eq!(
        source
            .read(&reference("fixture.txt"))
            .await
            .expect_err("newer refresh still suspends authority")
            .category(),
        ErrorCategory::SourceUnavailable
    );
    assert_eq!(
        source
            .complete_client_root_refresh(source.start_client_root_acquisition(second), Vec::new(),)
            .await
            .expect("current completion"),
        RootRefreshOutcome::Unchanged { generation: 1 }
    );

    let invalid = source.begin_client_root_refresh().await;
    let error = source
        .complete_client_root_refresh(
            source.start_client_root_acquisition(invalid),
            vec![ClientRoot {
                uri: "https://example.com/root".to_owned(),
                name: Some("remote".to_owned()),
            }],
        )
        .await
        .expect_err("non-file client root must fail");
    assert_eq!(error.category(), ErrorCategory::InvalidReference);
    assert_eq!(
        source
            .read(&reference("fixture.txt"))
            .await
            .expect_err("failed refresh disables authority")
            .category(),
        ErrorCategory::SourceUnavailable
    );

    let recovery = source.begin_client_root_refresh().await;
    let outcome = source
        .complete_client_root_refresh(source.start_client_root_acquisition(recovery), Vec::new())
        .await
        .expect("launch recovery");
    assert!(matches!(
        outcome,
        RootRefreshOutcome::Applied {
            generation: 2,
            ref removed,
            ..
        } if removed.is_empty()
    ));

    let failed = source.begin_client_root_refresh().await;
    source
        .fail_client_root_refresh(source.start_client_root_acquisition(failed))
        .await;
    assert_eq!(
        source
            .read(&reference("fixture.txt"))
            .await
            .expect_err("explicit acquisition failure disables authority")
            .category(),
        ErrorCategory::SourceUnavailable
    );
}

#[tokio::test]
async fn literal_paths_precede_selectors() {
    let (_temporary, root) = create_root();
    fs::write(root.join("notes"), "first\nsecond\n").expect("base fixture");
    fs::write(root.join("notes:2"), "literal").expect("literal fixture");
    fs::write(root.join("notes%3A2"), "single pass").expect("percent fixture");
    let source = launch_source(
        vec![launch_root("workspace", &root)],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");

    let literal = source
        .read(&reference("notes:2"))
        .await
        .expect("literal selector-shaped filename");
    assert_eq!(literal.content(), "literal");
    assert_eq!(
        literal.canonical_reference(),
        "rfs://workspace/workspace/notes%3A2"
    );

    let single_pass = source
        .read(&reference("notes%253A2"))
        .await
        .expect("single-pass percent filename");
    assert_eq!(single_pass.content(), "single pass");

    fs::remove_file(root.join("notes:2")).expect("remove literal fixture");
    let projection = source
        .read(&reference("notes:2"))
        .await
        .expect("selector executes after the literal candidate is absent");
    assert_eq!(projection.content(), "second\n");
    assert_eq!(
        projection.version_tag(),
        &resourcefs_core::VersionTag::from_content(b"first\nsecond\n")
    );
}

#[tokio::test]
async fn absolute_and_file_references_map_through_declared_roots() {
    let temporary = TempDir::new().expect("temporary directory");
    let root = temporary.path().join("root");
    let outside = temporary.path().join("outside");
    fs::create_dir(&root).expect("workspace root");
    fs::create_dir(&outside).expect("outside directory");
    let file = root.join("fixture.txt");
    fs::write(&file, "fixture").expect("workspace fixture");
    let outside_file = outside.join("secret.txt");
    fs::write(&outside_file, "secret").expect("outside fixture");
    let source = launch_source(
        vec![launch_root("workspace", &root)],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");

    for spelling in [
        file.to_string_lossy().into_owned(),
        url::Url::from_file_path(&file).expect("file URI").into(),
    ] {
        let resource = source
            .read(&reference(&spelling))
            .await
            .expect("declared absolute reference");
        assert_eq!(resource.content(), "fixture");
        assert_eq!(
            resource.canonical_reference(),
            "rfs://workspace/workspace/fixture.txt"
        );
    }

    for spelling in [
        outside_file.to_string_lossy().into_owned(),
        url::Url::from_file_path(&outside_file)
            .expect("outside file URI")
            .into(),
    ] {
        let error = source
            .read(&reference(&spelling))
            .await
            .expect_err("outside absolute reference must fail");
        assert_eq!(error.category(), ErrorCategory::PermissionDenied);
        assert!(!error.message().contains("secret"));
        assert!(!error.message().contains(&outside.to_string_lossy() as &str));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_non_native_windows_absolute_spelling_before_ambient_mapping() {
    let current = std::env::current_dir().expect("current directory");
    let source = single_source(&current).await.expect("filesystem source");

    for prefix in ["C:\\secret-", "\\\\server\\share\\secret-"] {
        let fixture = tempfile::Builder::new()
            .prefix(prefix)
            .tempfile_in(&current)
            .expect("non-native path fixture");
        fs::write(fixture.path(), "must not be read").expect("fixture content");
        let spelling = fixture
            .path()
            .file_name()
            .expect("fixture file name")
            .to_string_lossy()
            .into_owned();

        let error = source
            .read(&reference(&spelling))
            .await
            .expect_err("non-native absolute spelling must not use ambient cwd");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }
}
#[cfg(unix)]
#[tokio::test]
async fn configured_root_symlink_accepts_declared_absolute_spelling() {
    use std::os::unix::fs::symlink;

    let temporary = TempDir::new().expect("temporary directory");
    let actual = temporary.path().join("actual");
    let declared = temporary.path().join("declared");
    fs::create_dir(&actual).expect("actual root");
    fs::write(actual.join("fixture.txt"), "fixture").expect("fixture");
    symlink(&actual, &declared).expect("declared root symlink");
    let source = single_source(&declared).await.expect("filesystem source");

    for spelling in [
        declared.join("fixture.txt").to_string_lossy().into_owned(),
        url::Url::from_file_path(declared.join("fixture.txt"))
            .expect("declared file URI")
            .into(),
    ] {
        let resource = source
            .read(&reference(&spelling))
            .await
            .expect("declared absolute spelling");
        assert_eq!(resource.content(), "fixture");
        assert_eq!(
            resource.canonical_reference(),
            "rfs://workspace/workspace/fixture.txt"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn overlapping_absolute_inputs_are_ambiguous() {
    use std::os::unix::fs::symlink;

    let temporary = TempDir::new().expect("temporary directory");
    let parent = temporary.path().join("parent");
    let nested = parent.join("nested");
    fs::create_dir_all(&nested).expect("nested root");
    let nested_file = nested.join("fixture.txt");
    fs::write(&nested_file, "nested").expect("nested fixture");
    symlink(&nested_file, parent.join("alias.txt")).expect("nested alias");
    let source = launch_source(
        vec![
            launch_root("parent", &parent),
            launch_root("nested", &nested),
        ],
        Some("parent"),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("overlapping source");

    for spelling in [
        nested_file.to_string_lossy().into_owned(),
        url::Url::from_file_path(&nested_file)
            .expect("nested file URI")
            .into(),
        parent.join("alias.txt").to_string_lossy().into_owned(),
    ] {
        let error = source
            .read(&reference(&spelling))
            .await
            .expect_err("overlapping absolute input must fail");
        assert_eq!(error.category(), ErrorCategory::AmbiguousReference);
    }

    for spelling in [
        nested.to_string_lossy().into_owned(),
        url::Url::from_file_path(&nested)
            .expect("nested root URI")
            .into(),
    ] {
        let error = source
            .read(&reference(&spelling))
            .await
            .expect_err("overlapping root directory input must stay ambiguous");
        assert_eq!(error.category(), ErrorCategory::AmbiguousReference);
    }
}

#[tokio::test]
async fn backing_uri_requires_visibility_policy() {
    let (_temporary, root) = create_root();
    let file = root.join("fixture.txt");
    fs::write(&file, "fixture").expect("fixture");
    let roots = || vec![launch_root("workspace", &root)];

    let hidden = launch_source(roots(), None, BackingPathVisibility::Hidden)
        .await
        .expect("hidden source")
        .read(&reference("fixture.txt"))
        .await
        .expect("hidden read");
    assert_eq!(hidden.backing_file_uri(), None);

    let visible = launch_source(roots(), None, BackingPathVisibility::Visible)
        .await
        .expect("visible source")
        .read(&reference("fixture.txt"))
        .await
        .expect("visible read");
    let expected =
        url::Url::from_file_path(fs::canonicalize(file).expect("canonical fixture path"))
            .expect("backing file URI")
            .to_string();
    assert_eq!(visible.backing_file_uri(), Some(expected.as_str()));
    assert_eq!(
        visible.canonical_reference(),
        "rfs://workspace/workspace/fixture.txt"
    );
}

#[tokio::test]
async fn root_count_limit_is_exact() {
    let temporary = TempDir::new().expect("temporary directory");
    let mut roots = Vec::with_capacity(MAX_WORKSPACE_ROOTS + 1);
    for index in 0..=MAX_WORKSPACE_ROOTS {
        let path = temporary.path().join(format!("root-{index}"));
        fs::create_dir(&path).expect("root fixture");
        roots.push(launch_root(&format!("root-{index}"), &path));
    }

    let accepted = launch_source(
        roots[..MAX_WORKSPACE_ROOTS].to_vec(),
        Some("root-0"),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("256 roots must pass");
    let missing = accepted
        .read(&reference("missing.txt"))
        .await
        .expect_err("missing fixture");
    assert_eq!(missing.category(), ErrorCategory::NotFound);

    let rejected = launch_source(roots, Some("root-0"), BackingPathVisibility::Hidden)
        .await
        .expect_err("257 roots must fail");
    assert_eq!(rejected.category(), ErrorCategory::LimitExceeded);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retargeted_links_never_escape() {
    use std::{
        os::unix::fs::symlink,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    let temporary = TempDir::new().expect("temporary directory");
    let root = temporary.path().join("root");
    let inside = root.join("inside");
    let outside = temporary.path().join("outside");
    fs::create_dir_all(&inside).expect("inside directory");
    fs::create_dir(&outside).expect("outside directory");
    fs::write(inside.join("value.txt"), "inside").expect("inside fixture");
    fs::write(outside.join("value.txt"), "outside sentinel").expect("outside fixture");
    let alias = root.join("alias");
    symlink(&inside, &alias).expect("initial alias");
    let source = launch_source(
        vec![launch_root("workspace", &root)],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");

    let running = Arc::new(AtomicBool::new(true));
    let writer_running = Arc::clone(&running);
    let writer_alias = alias.clone();
    let writer = thread::spawn(move || {
        while writer_running.load(Ordering::Relaxed) {
            let _ = fs::remove_file(&writer_alias);
            let _ = symlink(&outside, &writer_alias);
            let _ = fs::remove_file(&writer_alias);
            let _ = symlink(&inside, &writer_alias);
        }
    });

    for _ in 0..500 {
        match source.read(&reference("alias/value.txt")).await {
            Ok(resource) => assert_eq!(resource.content(), "inside"),
            Err(error) => assert!(matches!(
                error.category(),
                ErrorCategory::NotFound
                    | ErrorCategory::PermissionDenied
                    | ErrorCategory::SourceUnavailable
            )),
        }
    }
    running.store(false, Ordering::Relaxed);
    writer.join().expect("retarget writer");
}
