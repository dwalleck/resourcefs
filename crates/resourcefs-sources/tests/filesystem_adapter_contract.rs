use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    sync::Arc,
    time::{Duration, Instant},
};

use resourcefs_core::{
    DiscoveryEngine, ErrorCategory, GlobKind, GlobLimits, GlobOptions, GlobRequest, GlobTarget,
    MAX_ARTIFACT_BYTES, MAX_TEXT_BYTES, MAX_WORKSPACE_ROOTS, OperationGuard, PathReference,
    SearchEngine, SearchLimits, SearchOptions, SearchRequest, SearchTarget, SourceAdapter,
    WorkspaceAddress, WorkspaceRootId,
};
use resourcefs_sources::{
    BackingPathVisibility, ClientRoot, FilesystemSource, LaunchRoot, LaunchRootSource,
    MutationGrants, RootRefreshOutcome, SessionStore, StoredSession,
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
    // Unix retains the trailing-space edge; Windows normalizes that spelling.
    let unicode_path = if cfg!(unix) {
        "notes/ résumé final.txt "
    } else {
        "notes/ résumé final .txt"
    };
    fs::write(root.join(unicode_path), "héllo\n").expect("unicode fixture");
    fs::write(root.join("empty.txt"), "").expect("empty fixture");
    let source = single_source(&root).await.expect("filesystem source");

    let unicode = source
        .read(&reference(unicode_path), &OperationGuard::new())
        .await
        .expect("unicode read");
    assert_eq!(unicode.content(), "héllo\n");
    assert_eq!(
        unicode.canonical_reference(),
        format!("rfs://workspace/workspace/{unicode_path}")
    );

    let empty = source
        .read(&reference("empty.txt"), &OperationGuard::new())
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
        .read(&reference("missing.txt"), &OperationGuard::new())
        .await
        .expect_err("missing should fail");
    assert_eq!(missing.category(), ErrorCategory::NotFound);

    let directory = source
        .read(&reference("directory"), &OperationGuard::new())
        .await
        .expect_err("directory should fail");
    assert_eq!(directory.category(), ErrorCategory::UnsupportedProjection);

    let binary = source
        .read(&reference("binary.bin"), &OperationGuard::new())
        .await
        .expect_err("binary should fail");
    assert_eq!(binary.category(), ErrorCategory::UnsupportedProjection);
}

#[tokio::test]
async fn root_directory_references_are_valid_and_teach() {
    let temporary = TempDir::new().expect("temporary directory");
    let alpha = temporary.path().join("alpha");
    let dotted = temporary.path().join("dotted");
    let z_last = temporary.path().join("z-last");
    for path in [&alpha, &dotted, &z_last] {
        fs::create_dir(path).expect("workspace root");
    }
    let source = launch_source(
        vec![
            launch_root("alpha", &alpha),
            launch_root("root.with-dots", &dotted),
            launch_root("z-last", &z_last),
        ],
        None,
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("multi-root source");
    let expected = "Resource 'rfs://workspace/root.with-dots/' is a directory; enumerate it with rfs_glob or read a file below it; bounded directory listings are tracked by rfs-hwlm";

    for spelling in [
        "rfs://workspace/root.with-dots/",
        "rfs://workspace/root.with-dots",
    ] {
        let error = source
            .read(&reference(spelling), &OperationGuard::new())
            .await
            .expect_err("root directory read must teach");
        assert_eq!(error.category(), ErrorCategory::UnsupportedProjection);
        assert_eq!(error.message(), expected);
    }

    let file_uri =
        url::Url::from_directory_path(fs::canonicalize(&dotted).expect("canonical dotted root"))
            .expect("file URI")
            .to_string();
    let file_reference = reference(&file_uri);
    assert!(matches!(
        file_reference.workspace_address(),
        Some(WorkspaceAddress::FileUri(_))
    ));
    assert_eq!(
        source
            .read(&file_reference, &OperationGuard::new())
            .await
            .expect_err("file URI root is still a directory")
            .category(),
        ErrorCategory::UnsupportedProjection
    );
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
        vec![LaunchRoot::read_only(colliding_id, alpha)],
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

#[tokio::test]
async fn mutable_matches_profile_and_exact_client_root_authority() {
    let temporary = TempDir::new().expect("temporary directory");
    let root = temporary.path().join("workspace");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).expect("workspace roots");
    fs::write(root.join("fixture.txt"), "root\n").expect("root fixture");
    fs::write(nested.join("fixture.txt"), "nested\n").expect("nested fixture");
    let grants = MutationGrants::new(true, true, true);
    let configured = LaunchRoot::new(
        WorkspaceRootId::new("workspace").expect("root ID"),
        root.clone(),
        grants,
    );
    assert_eq!(configured.id().as_str(), "workspace");
    assert_eq!(configured.path(), root);
    assert_eq!(configured.grants(), grants);
    let source = FilesystemSource::new(
        LaunchRootSource::Profile(vec![configured]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("profile source");

    assert!(
        source
            .read(&reference("fixture.txt"), &OperationGuard::new())
            .await
            .expect("granted profile read")
            .is_mutable()
    );

    let refresh = source.begin_client_root_refresh().await;
    source
        .complete_client_root_refresh(
            source.start_client_root_acquisition(refresh),
            vec![client_root(&root, "workspace")],
        )
        .await
        .expect("exact client refresh");
    assert!(
        source
            .read(&reference("fixture.txt"), &OperationGuard::new())
            .await
            .expect("exact client read")
            .is_mutable(),
        "exact canonical path inherits profile grants"
    );

    let refresh = source.begin_client_root_refresh().await;
    source
        .complete_client_root_refresh(
            source.start_client_root_acquisition(refresh),
            vec![client_root(&nested, "workspace")],
        )
        .await
        .expect("nested client refresh");
    assert!(
        !source
            .read(&reference("fixture.txt"), &OperationGuard::new())
            .await
            .expect("nested client read")
            .is_mutable(),
        "subdirectory client root must not inherit broader grants"
    );

    let cli = FilesystemSource::new(
        LaunchRootSource::Cli(vec![LaunchRoot::new(
            WorkspaceRootId::new("workspace").expect("CLI root ID"),
            root,
            grants,
        )]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("CLI source");
    assert!(
        !cli.read(&reference("fixture.txt"), &OperationGuard::new())
            .await
            .expect("CLI read")
            .is_mutable(),
        "CLI roots remain read-only even when constructed with grants"
    );
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn refresh_cannot_race_held_mutation_authority() {
    let (_temporary, root) = create_root();
    let source = single_source(&root).await.expect("filesystem source");
    let guard = source
        .hold_test_authority()
        .await
        .expect("active mutation authority");
    let refresh_source = source.clone();
    let refresh = tokio::spawn(async move { refresh_source.begin_client_root_refresh().await });

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !refresh.is_finished(),
        "authority refresh must wait for the mutation read guard"
    );
    drop(guard);
    tokio::time::timeout(Duration::from_millis(100), refresh)
        .await
        .expect("refresh proceeds after mutation authority release")
        .expect("refresh task");
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
    let source = FilesystemSource::new(
        LaunchRootSource::Profile(vec![LaunchRoot::new(
            WorkspaceRootId::new("workspace").expect("root ID"),
            root,
            MutationGrants::new(false, true, false),
        )]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("granted filesystem source");
    assert!(
        source
            .read(&reference("target.txt"), &OperationGuard::new())
            .await
            .expect("direct target read")
            .is_mutable()
    );

    for link in ["link.txt", "normalized-link.txt"] {
        let resource = source
            .read(&reference(link), &OperationGuard::new())
            .await
            .expect("contained link read");
        assert_eq!(resource.content(), "inside");
        assert!(
            !resource.is_mutable(),
            "linked Resources are not directly mutable"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn absolute_symlink_target_aliases_preserve_containment() {
    use std::os::unix::fs::symlink;

    let temporary = TempDir::new().expect("temporary directory");
    let parent = temporary.path().canonicalize().expect("canonical parent");
    let root = parent.join("workspace");
    let outside = parent.join("outside");
    fs::create_dir(&root).expect("workspace root");
    fs::create_dir(&outside).expect("outside directory");
    fs::write(root.join("target.txt"), "inside").expect("inside fixture");
    fs::write(outside.join("target.txt"), "outside secret").expect("outside fixture");
    let alias = parent.join("root-alias");
    symlink(&root, &alias).expect("root alias");
    symlink(alias.join("target.txt"), root.join("link.txt")).expect("absolute aliased target");
    let source = FilesystemSource::new(
        LaunchRootSource::Profile(vec![LaunchRoot::new(
            WorkspaceRootId::new("workspace").expect("root ID"),
            root,
            MutationGrants::new(false, true, false),
        )]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("granted filesystem source");

    let resource = source
        .read(&reference("link.txt"), &OperationGuard::new())
        .await
        .expect("contained aliased target");
    assert_eq!(resource.content(), "inside");
    assert!(!resource.is_mutable());

    fs::remove_file(&alias).expect("remove root alias");
    symlink(&outside, &alias).expect("retarget alias outside workspace");
    let error = source
        .read(&reference("link.txt"), &OperationGuard::new())
        .await
        .expect_err("retargeted alias must not escape");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
}

#[cfg(unix)]
#[tokio::test]
async fn dangling_absolute_symlinks_preserve_kernel_resolution_errors() {
    use std::os::unix::fs::symlink;

    let (temporary, root) = create_root();
    let root = root.canonicalize().expect("canonical root");
    let alias = temporary.path().join("root-alias");
    symlink(&root, &alias).expect("root alias");
    fs::write(root.join("secret.txt"), "must not be redirected here").expect("sibling");
    let source = single_source(&root).await.expect("filesystem source");

    for (name, target) in [
        ("missing-parent", root.join("missing-dir/../secret.txt")),
        ("canonical-missing", root.join("absent.txt")),
        ("aliased-missing", alias.join("absent.txt")),
    ] {
        symlink(target, root.join(name)).expect("dangling link");
        assert_eq!(
            fs::read(root.join(name))
                .expect_err("kernel rejects dangling link")
                .kind(),
            std::io::ErrorKind::NotFound,
        );
        let error = source
            .read(&reference(name), &OperationGuard::new())
            .await
            .expect_err("dangling Resource must not read sibling content");
        assert_eq!(error.category(), ErrorCategory::NotFound);
        assert!(
            error.message().contains(name),
            "missing Resource identity: {error}"
        );
    }

    let (_cache, _session, engine) = discovery_fixture(source).await;
    let result = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("*").expect("root glob"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("glob reports dangling entries");
    assert_eq!(result.diagnostics().len(), 3);
    assert!(
        result
            .diagnostics()
            .iter()
            .all(|diagnostic| { diagnostic.category() == ErrorCategory::NotFound })
    );
}

#[cfg(unix)]
#[tokio::test]
async fn absolute_symlink_non_directory_errors_retain_resource_identity() {
    use std::os::unix::fs::symlink;

    let (_temporary, root) = create_root();
    fs::write(root.join("notes.txt"), "regular file").expect("file fixture");
    symlink(root.join("notes.txt/child"), root.join("broken-link")).expect("invalid target");
    assert_eq!(
        fs::read(root.join("broken-link"))
            .expect_err("kernel rejects non-directory")
            .kind(),
        std::io::ErrorKind::NotADirectory,
    );
    let source = single_source(&root).await.expect("filesystem source");
    let error = source
        .read(&reference("broken-link"), &OperationGuard::new())
        .await
        .expect_err("non-directory path must fail");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert!(
        error.message().contains("broken-link"),
        "missing Resource identity: {error}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn absolute_symlinks_to_workspace_root_support_directory_discovery() {
    use std::os::unix::fs::symlink;

    let (temporary, root) = create_root();
    let root = root.canonicalize().expect("canonical root");
    let alias = temporary.path().join("root-alias");
    symlink(&root, &alias).expect("external lexical alias to root");
    symlink(&root, root.join("direct")).expect("canonical root link");
    symlink(&alias, root.join("aliased")).expect("aliased root link");
    fs::write(root.join("a.txt"), "root content").expect("file fixture");
    let source = single_source(&root).await.expect("filesystem source");
    for name in ["direct", "aliased"] {
        let error = source
            .read(&reference(name), &OperationGuard::new())
            .await
            .expect_err("root alias is a directory, not text");
        assert_eq!(error.category(), ErrorCategory::UnsupportedProjection);
        let resource = source
            .read(&reference(&format!("{name}/a.txt")), &OperationGuard::new())
            .await
            .expect("read through root alias");
        assert_eq!(resource.content(), "root content");
        assert_eq!(
            resource.canonical_reference(),
            "rfs://workspace/workspace/a.txt"
        );
    }

    let (_cache, _session, engine) = discovery_fixture(source).await;
    for name in ["direct", "aliased"] {
        let result = engine
            .search(
                SearchRequest::new(
                    SearchTarget::resource(reference(name)),
                    "root content",
                    SearchOptions::default(),
                    0,
                    SearchLimits::default(),
                )
                .expect("root alias search"),
                &OperationGuard::new(),
            )
            .await
            .expect("search directory through root alias");
        assert!(
            result.diagnostics().is_empty(),
            "{:?}",
            result.diagnostics()
        );
        assert_eq!(result.total_records(), 1);
        assert_eq!(
            result.groups()[0].reference(),
            "rfs://workspace/workspace/a.txt"
        );
    }
    for pattern in ["*.txt", "**/*.txt"] {
        let result = engine
            .glob(
                GlobRequest::new(
                    GlobTarget::new(pattern).expect("root alias glob"),
                    GlobOptions::default(),
                    0,
                    GlobLimits::default(),
                ),
                &OperationGuard::new(),
            )
            .await
            .expect("root alias listing and cyclic walk");
        assert!(
            result.diagnostics().is_empty(),
            "{:?}",
            result.diagnostics()
        );
        assert_eq!(result.total_records(), 1);
        assert_eq!(
            result.entries()[0].reference(),
            "rfs://workspace/workspace/a.txt"
        );
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
        .read(&reference("escape.txt"), &OperationGuard::new())
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

#[tokio::test]
async fn returns_complete_projection_above_inline_page_limits() {
    let (_temporary, root) = create_root();
    let exact = exact_byte_limit_content();
    fs::write(root.join("exact.txt"), &exact).expect("exact-byte fixture");

    let mut bytes_over = exact.clone();
    bytes_over.push(b'x');
    fs::write(root.join("bytes-over.txt"), &bytes_over).expect("byte-over fixture");
    let lines_over = "x\n".repeat(3_001);
    fs::write(root.join("lines-over.txt"), &lines_over).expect("line-over fixture");
    let columns_over = "x".repeat(513);
    fs::write(root.join("columns-over.txt"), &columns_over).expect("column-over fixture");

    let source = single_source(&root).await.expect("filesystem source");
    for (path, expected) in [
        ("exact.txt", exact.as_slice()),
        ("bytes-over.txt", bytes_over.as_slice()),
        ("lines-over.txt", lines_over.as_bytes()),
        ("columns-over.txt", columns_over.as_bytes()),
    ] {
        let resource = source
            .read(&reference(path), &OperationGuard::new())
            .await
            .expect("Source Adapter returns complete projection");
        assert_eq!(resource.content().as_bytes(), expected);
    }
}

#[tokio::test]
async fn rejects_projection_one_byte_over_artifact_ceiling() {
    let (_temporary, root) = create_root();
    let path = root.join("object-over.txt");
    let file = fs::File::create(&path).expect("object-over fixture");
    file.set_len((MAX_ARTIFACT_BYTES + 1) as u64)
        .expect("sparse object-over fixture");
    drop(file);

    let source = single_source(&root).await.expect("filesystem source");
    let error = source
        .read(&reference("object-over.txt"), &OperationGuard::new())
        .await
        .expect_err("one byte over object ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
}

#[tokio::test]
async fn reads_maximum_sized_file_within_budget() {
    let (_temporary, root) = create_root();
    let exact = exact_byte_limit_content();
    fs::write(root.join("maximum.txt"), &exact).expect("maximum fixture");
    let source = single_source(&root).await.expect("filesystem source");

    let started = Instant::now();
    let resource = source
        .read(&reference("maximum.txt"), &OperationGuard::new())
        .await
        .expect("maximum read");
    let elapsed = started.elapsed();

    assert_eq!(resource.content().as_bytes(), exact);
    let budget = if cfg!(debug_assertions) {
        Duration::from_secs(2)
    } else {
        Duration::from_millis(100)
    };
    assert!(elapsed <= budget, "maximum-sized read took {elapsed:?}");
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
        .read(&reference("large.txt"), &OperationGuard::new())
        .await
        .expect_err("complete large source exceeds the inline source limit");
    assert_eq!(full_error.category(), ErrorCategory::LimitExceeded);

    let started = Instant::now();
    let selected = source
        .read(&reference("large.txt:2"), &OperationGuard::new())
        .await
        .expect("narrow selection from large source");
    let elapsed = started.elapsed();
    assert_eq!(selected.content(), TARGET);
    assert_eq!(selected.version_tag().as_str(), expected_tag);
    assert!(selected.content().len() <= 70 * 1024 * 1024);
    let budget = if cfg!(debug_assertions) {
        Duration::from_secs(200)
    } else {
        Duration::from_secs(10)
    };
    assert!(elapsed <= budget, "256 MiB selection took {elapsed:?}");
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
    let pending = tokio::spawn(async move {
        reader
            .read(
                &reference("stale-stream-delivery.txt:2"),
                &OperationGuard::new(),
            )
            .await
    });
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
    assert_eq!(error.category(), ErrorCategory::InvalidReference);
}

fn launch_root(id: &str, path: &std::path::Path) -> LaunchRoot {
    LaunchRoot::read_only(
        WorkspaceRootId::new(id).expect("fixture root ID"),
        path.to_owned(),
    )
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

async fn discovery_fixture(source: FilesystemSource) -> (TempDir, StoredSession, DiscoveryEngine) {
    let cache = TempDir::new().expect("discovery cache");
    let store = SessionStore::open_with(
        resourcefs_sources::SessionStorageConfig::new(
            cache.path(),
            resourcefs_sources::SESSION_CLEANUP_TTL.as_secs() as i64,
        )
        .expect("default session storage config"),
    )
    .await
    .expect("discovery session store");
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("discovery session");
    let engine = DiscoveryEngine::new(
        Arc::new(source),
        session.path_session().clone(),
        resourcefs_core::ServerLimits::default(),
    );
    (cache, session, engine)
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
        .read(&reference("shared.txt"), &OperationGuard::new())
        .await
        .expect_err("relative read without a primary must fail");
    assert_eq!(ambiguous.category(), ErrorCategory::AmbiguousReference);

    for (id, content) in [("alpha", "alpha"), ("beta", "beta")] {
        let canonical = format!("rfs://workspace/{id}/shared.txt");
        let resource = no_primary
            .read(&reference(&canonical), &OperationGuard::new())
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
            .read(&reference("shared.txt"), &OperationGuard::new())
            .await
            .expect("selected relative read")
            .content(),
        "beta"
    );

    let unmatched = launch_source(roots(), Some("missing"), BackingPathVisibility::Hidden)
        .await
        .expect("unmatched primary disables relative reads");
    let error = unmatched
        .read(&reference("shared.txt"), &OperationGuard::new())
        .await
        .expect_err("unmatched primary must fail");
    assert_eq!(error.category(), ErrorCategory::AmbiguousReference);

    let unknown = no_primary
        .read(
            &reference("rfs://workspace/missing/shared.txt"),
            &OperationGuard::new(),
        )
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
        .read(&reference("shared.txt"), &OperationGuard::new())
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
        .read(&reference("shared.txt"), &OperationGuard::new())
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
            .read(&reference("shared.txt"), &OperationGuard::new())
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
            .read(&reference("shared.txt"), &OperationGuard::new())
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
        .read(&reference("fixture.txt"), &OperationGuard::new())
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
            .read(&reference("fixture.txt"), &OperationGuard::new())
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
            .read(&reference("shared.txt"), &OperationGuard::new())
            .await
            .expect("unique valid display-name match")
            .content(),
        "beta"
    );
    let alpha_absolute = alpha.join("shared.txt").to_string_lossy().into_owned();
    let alpha_identity = source
        .read(&reference(&alpha_absolute), &OperationGuard::new())
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
            .read(&reference("shared.txt"), &OperationGuard::new())
            .await
            .expect_err("duplicate primary names are ambiguous")
            .category(),
        ErrorCategory::AmbiguousReference
    );
    assert_eq!(
        source
            .read(&reference(&alpha_absolute), &OperationGuard::new())
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
        .read(&reference("fixture.txt"), &OperationGuard::new())
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
            .read(&reference("fixture.txt"), &OperationGuard::new())
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
            .read(&reference("fixture.txt"), &OperationGuard::new())
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
            .read(&reference("fixture.txt"), &OperationGuard::new())
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
        .read(&reference("notes:2"), &OperationGuard::new())
        .await
        .expect("literal selector-shaped filename");
    assert_eq!(literal.content(), "literal");
    assert_eq!(
        literal.canonical_reference(),
        "rfs://workspace/workspace/notes%3A2"
    );

    let single_pass = source
        .read(&reference("notes%253A2"), &OperationGuard::new())
        .await
        .expect("single-pass percent filename");
    assert_eq!(single_pass.content(), "single pass");

    fs::remove_file(root.join("notes:2")).expect("remove literal fixture");
    let projection = source
        .read(&reference("notes:2"), &OperationGuard::new())
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
            .read(&reference(&spelling), &OperationGuard::new())
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
            .read(&reference(&spelling), &OperationGuard::new())
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
            .read(&reference(&spelling), &OperationGuard::new())
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
            .read(&reference(&spelling), &OperationGuard::new())
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
            .read(&reference(&spelling), &OperationGuard::new())
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
            .read(&reference(&spelling), &OperationGuard::new())
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
        .read(&reference("fixture.txt"), &OperationGuard::new())
        .await
        .expect("hidden read");
    assert_eq!(hidden.backing_file_uri(), None);

    let visible = launch_source(roots(), None, BackingPathVisibility::Visible)
        .await
        .expect("visible source")
        .read(&reference("fixture.txt"), &OperationGuard::new())
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
        .read(&reference("missing.txt"), &OperationGuard::new())
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
        match source
            .read(&reference("alias/value.txt"), &OperationGuard::new())
            .await
        {
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

#[tokio::test]
async fn search_groups_lines_and_reports_partial_failures() {
    let (_temporary, root) = create_root();
    fs::write(
        root.join("good.txt"),
        "needle needle\r\nother\nneedle suffix\n",
    )
    .expect("C6 text fixture");
    fs::write(root.join("binary.bin"), b"valid prefix\n\xff\n").expect("C6 binary fixture");
    let source = single_source(&root).await.expect("C6 filesystem source");
    let (_cache, _session, engine) = discovery_fixture(source).await;

    let result = engine
        .search(
            SearchRequest::new(
                SearchTarget::primary(),
                "needle",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C6 search request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C6 partial directory search");
    assert_eq!(result.engine(), SearchEngine::RustRegex, "C6 engine");
    assert_eq!(result.total_records(), 2, "C6 one row per matching line");
    assert_eq!(result.groups().len(), 1, "C6 one matching Resource");
    assert_eq!(
        result.groups()[0].reference(),
        "rfs://workspace/workspace/good.txt",
        "C6 canonical group"
    );
    assert_eq!(
        result.groups()[0]
            .lines()
            .iter()
            .map(|line| (line.line(), line.text()))
            .collect::<Vec<_>>(),
        vec![(1, "needle needle"), (3, "needle suffix")],
        "C6 terminator-free line oracle"
    );
    assert_eq!(result.diagnostics().len(), 1, "C6 one skipped binary");
    assert_eq!(
        result.diagnostics()[0].reference(),
        Some("rfs://workspace/workspace/binary.bin"),
        "C6 diagnostic identity"
    );
    assert_eq!(
        result.diagnostics()[0].category(),
        ErrorCategory::UnsupportedProjection,
        "C6 diagnostic category"
    );

    let exact_error = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(reference("binary.bin")),
                "needle",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C6 exact request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("C6 exact binary must fail");
    assert_eq!(
        exact_error.category(),
        ErrorCategory::UnsupportedProjection,
        "C6 exact-target category"
    );

    let selected = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(reference("good.txt:2-")),
                "needle",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C6 selected Workspace request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C6 selected Workspace search");
    assert_eq!(selected.total_records(), 1, "C6 selected record count");
    assert_eq!(
        selected.groups()[0].lines()[0].line(),
        3,
        "C6 selected original line number"
    );

    fs::write(root.join("literal.txt:2"), "literal marker\n").expect("C6 literal colon file");
    let literal = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(reference("literal.txt:2")),
                "literal marker",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C6 literal request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C6 literal path search");
    assert_eq!(
        literal.groups()[0].reference(),
        "rfs://workspace/workspace/literal.txt%3A2",
        "C6 literal path precedes selector"
    );
}

#[tokio::test]
async fn discovery_filters_are_contained_and_explicit() {
    let (temporary, root) = create_root();
    fs::create_dir_all(root.join("sub")).expect("C5 nested directory");
    fs::create_dir_all(root.join(".git/info")).expect("C5 Git metadata directory");
    fs::create_dir_all(root.join("ignored-dir")).expect("C5 ignored directory");
    fs::create_dir_all(root.join(".hidden-dir")).expect("C5 hidden directory");
    fs::create_dir_all(root.join("ignored-parent/child")).expect("C5 nested ignored directory");
    fs::create_dir_all(root.join(".hidden-parent/child")).expect("C5 nested hidden directory");
    fs::write(
        root.join(".gitignore"),
        "\u{feff}ignored.txt\nignored-dir/\nignored-parent/\nsub/ignored-*.txt\n!sub/ignored-keep.txt\n",
    )
    .expect("C5 root gitignore");
    fs::write(root.join("sub/.gitignore"), "nested-only.txt\n").expect("C5 nested gitignore");
    fs::write(root.join(".ignore"), "visible-from-dot-ignore.txt\n").expect("C5 forbidden .ignore");
    fs::write(
        root.join(".git/info/exclude"),
        "visible-from-info-exclude.txt\n",
    )
    .expect("C5 forbidden info exclude");
    for path in [
        "visible.txt",
        "ignored.txt",
        "sub/ignored-drop.txt",
        "sub/ignored-keep.txt",
        "sub/nested-only.txt",
        "visible-from-dot-ignore.txt",
        "visible-from-info-exclude.txt",
        ".hidden.txt",
        "ignored-dir/value.txt",
        ".hidden-dir/value.txt",
        "ignored-parent/child/value.txt",
        ".hidden-parent/child/value.txt",
    ] {
        fs::write(root.join(path), format!("needle {path}\n")).expect("C5 searchable fixture");
    }
    let source = single_source(&root).await.expect("C5 filesystem source");
    let (_cache, _session, engine) = discovery_fixture(source).await;

    let defaults = engine
        .search(
            SearchRequest::new(
                SearchTarget::primary(),
                "needle",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C5 default request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C5 default discovery");
    let default_references = defaults
        .groups()
        .iter()
        .map(|group| group.reference())
        .collect::<Vec<_>>();
    assert_eq!(
        default_references,
        vec![
            "rfs://workspace/workspace/sub/ignored-keep.txt",
            "rfs://workspace/workspace/visible-from-dot-ignore.txt",
            "rfs://workspace/workspace/visible-from-info-exclude.txt",
            "rfs://workspace/workspace/visible.txt",
        ],
        "C5 contained gitignore and hidden oracle"
    );
    assert!(defaults.diagnostics().is_empty(), "C5 default diagnostics");

    let unfiltered = engine
        .search(
            SearchRequest::new(
                SearchTarget::primary(),
                "needle",
                SearchOptions::new(true, false, true),
                0,
                SearchLimits::default(),
            )
            .expect("C5 unfiltered request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C5 unfiltered discovery");
    assert_eq!(
        unfiltered.total_records(),
        12,
        "C5 disabled controls reveal all twelve fixtures"
    );

    for (pattern, message) in [
        (
            "ignored-dir/*.txt",
            "C5 glob cannot bypass ignored ancestor",
        ),
        (".hidden-dir/*.txt", "C5 glob cannot bypass hidden ancestor"),
    ] {
        let filtered = engine
            .glob(
                GlobRequest::new(
                    GlobTarget::new(pattern).expect("C5 filtered glob target"),
                    GlobOptions::default(),
                    0,
                    GlobLimits::default(),
                ),
                &OperationGuard::new(),
            )
            .await
            .expect("C5 filtered glob");
        assert!(filtered.entries().is_empty(), "{message}");
    }

    let explicit = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(reference("ignored.txt")),
                "needle",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C5 explicit request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C5 explicit ignored file");
    assert_eq!(explicit.total_records(), 1, "C5 exact target bypass");
    assert_eq!(
        explicit.groups()[0].reference(),
        "rfs://workspace/workspace/ignored.txt",
        "C5 exact canonical identity"
    );

    for (path, expected) in [
        (
            "ignored-dir",
            "rfs://workspace/workspace/ignored-dir/value.txt",
        ),
        (
            ".hidden-dir",
            "rfs://workspace/workspace/.hidden-dir/value.txt",
        ),
        (
            "ignored-parent/child",
            "rfs://workspace/workspace/ignored-parent/child/value.txt",
        ),
        (
            ".hidden-parent/child",
            "rfs://workspace/workspace/.hidden-parent/child/value.txt",
        ),
    ] {
        let explicit_directory = engine
            .search(
                SearchRequest::new(
                    SearchTarget::resource(reference(path)),
                    "needle",
                    SearchOptions::default(),
                    0,
                    SearchLimits::default(),
                )
                .expect("C5 explicit directory request"),
                &OperationGuard::new(),
            )
            .await
            .expect("C5 explicit filtered directory");
        assert_eq!(
            explicit_directory.total_records(),
            1,
            "C5 exact directory target bypass for {path}"
        );
        assert_eq!(
            explicit_directory.groups()[0].reference(),
            expected,
            "C5 exact directory canonical identity for {path}"
        );
    }
    let global_excludes = temporary.path().join("global-excludes");
    let global_config = temporary.path().join("global-gitconfig");
    fs::write(&global_excludes, "visible.txt\n").expect("C5 poisoned global excludes");
    fs::write(
        &global_config,
        format!(
            "[core]\n\texcludesFile = {}\n",
            global_excludes.to_string_lossy()
        ),
    )
    .expect("C5 poisoned global config");
    let child = std::process::Command::new(std::env::current_exe().expect("C5 test executable"))
        .arg("global_gitignore_is_not_consulted_child")
        .arg("--exact")
        .arg("--nocapture")
        .env("RFS_C5_GLOBAL_ROOT", &root)
        .env("GIT_CONFIG_GLOBAL", &global_config)
        .env(
            "GIT_CONFIG_SYSTEM",
            temporary.path().join("missing-system-config"),
        )
        .output()
        .expect("C5 isolated global-config child");
    assert!(
        child.status.success(),
        "C5 ambient global Git configuration changed discovery: {}",
        String::from_utf8_lossy(&child.stderr)
    );
}

#[test]
fn global_gitignore_is_not_consulted_child() {
    let Some(root) = std::env::var_os("RFS_C5_GLOBAL_ROOT").map(std::path::PathBuf::from) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("C5 child runtime");
    runtime.block_on(async move {
        let source = single_source(&root).await.expect("C5 child source");
        let (_cache, _session, engine) = discovery_fixture(source).await;
        let result = engine
            .search(
                SearchRequest::new(
                    SearchTarget::primary(),
                    "needle",
                    SearchOptions::default(),
                    0,
                    SearchLimits::default(),
                )
                .expect("C5 child request"),
                &OperationGuard::new(),
            )
            .await
            .expect("C5 child search");
        assert!(
            result
                .groups()
                .iter()
                .any(|group| group.reference() == "rfs://workspace/workspace/visible.txt"),
            "C5 ambient global excludes must not hide Workspace content"
        );
    });
}

#[tokio::test]
async fn glob_language_kinds_and_order() {
    let (_temporary, root) = create_root();
    fs::create_dir_all(root.join("src/deep")).expect("C7 source tree");
    for path in ["src/main.rs", "src/lib.rs", "src/deep/leaf.rs"] {
        fs::write(root.join(path), path).expect("C7 Rust fixture");
    }
    fs::write(root.join("README.md"), "readme").expect("C7 Markdown fixture");
    #[cfg(unix)]
    fs::write(root.join("literal*.txt"), "literal star").expect("C7 escaped-star fixture");
    let source = single_source(&root).await.expect("C7 filesystem source");
    let (_cache, _session, engine) = discovery_fixture(source).await;

    let recursive = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("src/**/*.rs").expect("C7 recursive target"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C7 recursive glob");
    assert_eq!(
        recursive
            .entries()
            .iter()
            .map(|entry| (entry.reference(), entry.kind()))
            .collect::<Vec<_>>(),
        vec![
            ("rfs://workspace/workspace/src/deep/leaf.rs", GlobKind::File,),
            ("rfs://workspace/workspace/src/lib.rs", GlobKind::File),
            ("rfs://workspace/workspace/src/main.rs", GlobKind::File),
        ],
        "C7 recursive slash grammar and bytewise order"
    );

    let canonical = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("rfs://workspace/workspace/src/{main,lib}.rs")
                    .expect("C7 canonical target"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C7 canonical glob");
    assert_eq!(
        canonical
            .entries()
            .iter()
            .map(|entry| entry.reference())
            .collect::<Vec<_>>(),
        vec![
            "rfs://workspace/workspace/src/lib.rs",
            "rfs://workspace/workspace/src/main.rs",
        ],
        "C7 canonical alternation"
    );

    let folded = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("SRC/*.RS").expect("C7 folded target"),
                GlobOptions::new(false, true, false),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C7 ASCII-folded glob");
    assert_eq!(
        folded
            .entries()
            .iter()
            .map(|entry| entry.reference())
            .collect::<Vec<_>>(),
        vec![
            "rfs://workspace/workspace/src/lib.rs",
            "rfs://workspace/workspace/src/main.rs",
        ],
        "C7 ASCII-only case folding"
    );

    let directory = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("src").expect("C7 exact directory target"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C7 exact directory glob");
    assert_eq!(directory.entries().len(), 1, "C7 one directory entry");
    assert_eq!(
        directory.entries()[0].kind(),
        GlobKind::Directory,
        "C7 directory kind"
    );
    assert!(
        directory.text().contains("rfs://workspace/workspace/src/"),
        "C7 directory text has trailing slash"
    );

    // A literal '*' is a native filename only on Unix.
    #[cfg(unix)]
    {
        let escaped = engine
            .glob(
                GlobRequest::new(
                    GlobTarget::new(r"literal\*.txt").expect("C7 escaped target"),
                    GlobOptions::default(),
                    0,
                    GlobLimits::default(),
                ),
                &OperationGuard::new(),
            )
            .await
            .expect("C7 escaped glob");
        assert_eq!(escaped.entries().len(), 1, "C7 escaped match count");
        assert_eq!(
            escaped.entries()[0].reference(),
            "rfs://workspace/workspace/literal*.txt",
            "C7 backslash is an escape"
        );
    }

    let invalid = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("literal\\").expect("C7 shaped dangling escape"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect_err("C7 dangling escape must fail");
    assert_eq!(
        invalid.category(),
        ErrorCategory::InvalidPattern,
        "C7 dangling escape category"
    );

    let absent = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("missing/**/*.txt").expect("C7 absent-prefix target"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C7 absent fixed prefix is a valid no-match glob");
    assert!(absent.entries().is_empty(), "C7 absent-prefix empty result");
    assert!(
        absent.diagnostics().is_empty(),
        "C7 absent-prefix diagnostics"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discovery_links_never_escape_or_duplicate() {
    use std::{
        os::unix::fs::symlink,
        sync::atomic::{AtomicBool, Ordering},
        thread,
    };

    let temporary = TempDir::new().expect("C4 temporary directory");
    let root = temporary.path().join("root");
    let inside = root.join("inside");
    let outside = temporary.path().join("outside");
    fs::create_dir_all(&inside).expect("C4 inside directory");
    fs::create_dir(&outside).expect("C4 outside directory");
    fs::write(inside.join("target.txt"), "inside sentinel\n").expect("C4 inside sentinel");
    fs::write(outside.join("target.txt"), "outside secret sentinel\n")
        .expect("C4 outside sentinel");
    fs::write(root.join(".gitignore"), "ignored-target.txt\n")
        .expect("C5 contained ignore fixture");
    fs::write(root.join(".hidden-target.txt"), "hidden secret sentinel\n")
        .expect("C5 hidden final target");
    fs::write(root.join("ignored-target.txt"), "ignored secret sentinel\n")
        .expect("C5 ignored final target");
    fs::create_dir(root.join(".hidden-dir")).expect("C5 hidden final directory");
    fs::write(
        root.join(".hidden-dir/nested.txt"),
        "hidden directory secret sentinel\n",
    )
    .expect("C5 hidden directory target");
    symlink(inside.join("target.txt"), root.join("alias-a.txt")).expect("C4 first file alias");
    symlink(inside.join("target.txt"), root.join("alias-b.txt")).expect("C4 second file alias");
    symlink(&inside, root.join("alias-dir")).expect("C4 directory alias");
    symlink(&root, inside.join("cycle")).expect("C4 contained cycle");
    symlink(outside.join("target.txt"), root.join("escape.txt")).expect("C4 escaping file");
    symlink(&outside, root.join("escape-dir")).expect("C4 escaping directory");
    let switch = root.join("switch");
    symlink(&inside, &switch).expect("C4 initial switch");
    symlink(
        root.join(".hidden-target.txt"),
        root.join("visible-hidden-alias.txt"),
    )
    .expect("C5 visible alias to hidden file");
    symlink(
        root.join("ignored-target.txt"),
        root.join("visible-ignored-alias.txt"),
    )
    .expect("C5 visible alias to ignored file");
    symlink(root.join(".hidden-dir"), root.join("visible-hidden-dir"))
        .expect("C5 visible alias to hidden directory");

    let source = single_source(&root).await.expect("C4 filesystem source");
    let (_cache, _session, engine) = discovery_fixture(source).await;
    let search_request = || {
        SearchRequest::new(
            SearchTarget::primary(),
            "sentinel",
            SearchOptions::default(),
            0,
            SearchLimits::default(),
        )
        .expect("C4 search request")
    };
    let search = engine
        .search(search_request(), &OperationGuard::new())
        .await
        .expect("C4 contained search");
    assert_eq!(search.total_records(), 1, "C4 final file identity once");
    assert_eq!(
        search.groups()[0].reference(),
        "rfs://workspace/workspace/inside/target.txt",
        "C4 final canonical search identity"
    );
    assert!(
        !search.text().contains("outside secret"),
        "C4 no outside search bytes"
    );
    assert!(
        search
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.category() == ErrorCategory::PermissionDenied),
        "C4 escaping links become permission diagnostics"
    );

    let glob = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("**/*.txt").expect("C4 glob target"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C4 contained glob");
    assert_eq!(glob.total_records(), 1, "C4 final glob identity once");
    assert_eq!(
        glob.entries()[0].reference(),
        "rfs://workspace/workspace/inside/target.txt",
        "C4 final canonical glob identity"
    );
    assert!(
        !glob.text().contains("outside secret"),
        "C4 no outside glob bytes"
    );

    let running = Arc::new(AtomicBool::new(true));
    let writer_running = Arc::clone(&running);
    let writer_switch = switch.clone();
    let writer = thread::spawn(move || {
        while writer_running.load(Ordering::Relaxed) {
            let _ = fs::remove_file(&writer_switch);
            let _ = symlink(&outside, &writer_switch);
            let _ = fs::remove_file(&writer_switch);
            let _ = symlink(&inside, &writer_switch);
        }
    });
    for _ in 0..100 {
        let result = engine
            .search(search_request(), &OperationGuard::new())
            .await
            .expect("C4 concurrent search remains partial-successful");
        assert_eq!(
            result.total_records(),
            1,
            "C4 concurrent final identity remains unique"
        );
        assert!(
            !result.text().contains("outside secret"),
            "C4 concurrent retarget never exposes outside bytes"
        );
    }
    running.store(false, Ordering::Relaxed);
    writer.join().expect("C4 retarget writer");
}

#[cfg(feature = "test-support")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_refresh_fences_discovery_delivery() {
    let temporary = TempDir::new().expect("C14 temporary directory");
    let first = temporary.path().join("first");
    let second = temporary.path().join("second");
    fs::create_dir(&first).expect("C14 first root");
    fs::create_dir(&second).expect("C14 second root");
    fs::write(first.join("value.txt"), "old sentinel\n").expect("C14 old fixture");
    fs::write(second.join("value.txt"), "new sentinel\n").expect("C14 new fixture");
    let source = single_source(&first).await.expect("C14 filesystem source");
    let gate = source.arm_test_delivery_gate("*").await;
    let (_cache, _session, engine) = discovery_fixture(source.clone()).await;
    let pending_engine = engine.clone();
    let pending = tokio::spawn(async move {
        pending_engine
            .search(
                SearchRequest::new(
                    SearchTarget::primary(),
                    "sentinel",
                    SearchOptions::default(),
                    0,
                    SearchLimits::default(),
                )
                .expect("C14 pending request"),
                &OperationGuard::new(),
            )
            .await
    });
    gate.wait_until_entered().await;

    let refresh = source.begin_client_root_refresh().await;
    let acquisition = source.start_client_root_acquisition(refresh);
    let outcome = source
        .complete_client_root_refresh(acquisition, vec![client_root(&second, "replacement")])
        .await
        .expect("C14 replacement refresh");
    assert!(
        matches!(
            outcome,
            RootRefreshOutcome::Applied {
                generation: 2,
                ref removed,
                ..
            } if removed == &[WorkspaceRootId::new("workspace").expect("C14 old root ID")]
        ),
        "C14 root generation oracle"
    );
    gate.release();
    let error = pending
        .await
        .expect("C14 pending task")
        .expect_err("C14 removed-root result must not be delivered");
    assert_eq!(
        error.category(),
        ErrorCategory::InvalidReference,
        "C14 stale delivery category"
    );

    let current = engine
        .search(
            SearchRequest::new(
                SearchTarget::primary(),
                "sentinel",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C14 current request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C14 current discovery");
    assert!(
        current.text().contains("new sentinel"),
        "C14 current root is observable"
    );
    assert!(
        !current.text().contains("old sentinel"),
        "C14 removed root is absent"
    );
}

#[cfg(feature = "test-support")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_fences_workspace_discovery_delivery() {
    let (_temporary, root) = create_root();
    fs::write(root.join("value.txt"), "needle\n").expect("C14 cancellation fixture");
    let source = single_source(&root).await.expect("C14 filesystem source");
    let (_cache, session, engine) = discovery_fixture(source.clone()).await;

    let search_gate = source.arm_test_delivery_gate("*").await;
    let search_operation = OperationGuard::new();
    let pending_engine = engine.clone();
    let pending_operation = search_operation.clone();
    let pending_search = tokio::spawn(async move {
        pending_engine
            .search(
                SearchRequest::new(
                    SearchTarget::primary(),
                    "needle",
                    SearchOptions::default(),
                    0,
                    SearchLimits::default(),
                )
                .expect("C14 cancellation search request"),
                &pending_operation,
            )
            .await
    });
    search_gate.wait_until_entered().await;
    search_operation.cancel();
    search_gate.release();
    let search_error = pending_search
        .await
        .expect("C14 cancellation search task")
        .expect_err("C14 cancelled search must not be delivered");
    assert_eq!(
        search_error.category(),
        ErrorCategory::Cancelled,
        "C14 cancelled search category"
    );

    let glob_gate = source.arm_test_delivery_gate("*").await;
    let glob_operation = OperationGuard::new();
    let pending_engine = engine.clone();
    let pending_operation = glob_operation.clone();
    let pending_glob = tokio::spawn(async move {
        pending_engine
            .glob(
                GlobRequest::new(
                    GlobTarget::new("**/*.txt").expect("C14 cancellation glob target"),
                    GlobOptions::default(),
                    0,
                    GlobLimits::default(),
                ),
                &pending_operation,
            )
            .await
    });
    glob_gate.wait_until_entered().await;
    glob_operation.cancel();
    glob_gate.release();
    let glob_error = pending_glob
        .await
        .expect("C14 cancellation glob task")
        .expect_err("C14 cancelled glob must not be delivered");
    assert_eq!(
        glob_error.category(),
        ErrorCategory::Cancelled,
        "C14 cancelled glob category"
    );
    assert_eq!(
        session.path_session().artifact_count().await,
        0,
        "C14 cancellation publishes no recovery Artifact"
    );
}
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wide_directory_keeps_pending_handles_bounded() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let (_temporary, root) = create_root();
    let wide = root.join("wide");
    fs::create_dir(&wide).expect("C4 wide directory");
    for index in 0..4_000 {
        fs::create_dir(wide.join(format!("d{index:04}"))).expect("C4 wide child directory");
    }
    let source = single_source(&root).await.expect("C4 wide source");
    let (_cache, _session, engine) = discovery_fixture(source).await;
    let baseline = fs::read_dir("/proc/self/fd")
        .expect("C4 process descriptor directory")
        .count();
    let running = Arc::new(AtomicBool::new(true));
    let maximum = Arc::new(AtomicUsize::new(baseline));
    let monitor_running = Arc::clone(&running);
    let monitor_maximum = Arc::clone(&maximum);
    let monitor = std::thread::spawn(move || {
        while monitor_running.load(Ordering::Acquire) {
            let current = fs::read_dir("/proc/self/fd")
                .expect("C4 monitored descriptor directory")
                .count();
            monitor_maximum.fetch_max(current, Ordering::Relaxed);
            std::thread::yield_now();
        }
    });

    let result = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("wide/*").expect("C4 wide glob target"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C4 wide glob");
    running.store(false, Ordering::Release);
    monitor.join().expect("C4 descriptor monitor");

    assert_eq!(result.total_records(), 4_000, "C4 wide directory count");
    assert!(
        result.diagnostics().is_empty(),
        "C4 wide traversal diagnostics: {:?}",
        result.diagnostics()
    );
    assert!(
        maximum.load(Ordering::Relaxed) <= baseline + 512,
        "C4 traversal retained too many directory handles: baseline {baseline}, peak {}",
        maximum.load(Ordering::Relaxed)
    );
}

#[tokio::test]
async fn workspace_discovery_scale_is_bounded_and_deterministic() {
    let (_temporary, root) = create_root();
    let tree = root.join("tree");
    fs::create_dir(&tree).expect("C4 scale tree");
    for directory in 0..100 {
        let directory_path = tree.join(format!("d{directory:03}"));
        fs::create_dir(&directory_path).expect("C4 scale directory");
        for file in 0..100 {
            let content = if file % 10 == 0 {
                "needle\n"
            } else {
                "plain\n"
            };
            fs::write(directory_path.join(format!("f{file:03}.txt")), content)
                .expect("C4 scale file");
        }
    }
    fs::write(tree.join("d050/.gitignore"), "ignored-extra.txt\n")
        .expect("C5 nested scale gitignore");
    fs::write(tree.join("d050/ignored-extra.txt"), "needle\n").expect("C5 ignored scale file");
    fs::write(tree.join(".hidden.txt"), "needle\n").expect("C5 hidden scale file");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("missing.txt", tree.join("vanished.txt"))
            .expect("C14 vanished scale entry");
    }

    let source = single_source(&root).await.expect("C4 scale source");
    let (_cache, _session, engine) = discovery_fixture(source).await;
    let mut stable_text = None;
    for run in 0..5 {
        let started = Instant::now();
        let result = engine
            .search(
                SearchRequest::new(
                    SearchTarget::resource(reference("tree")),
                    "needle",
                    SearchOptions::default(),
                    0,
                    SearchLimits::default(),
                )
                .expect("C6 scale search request"),
                &OperationGuard::new(),
            )
            .await
            .expect("C6 scale search");
        assert!(
            started.elapsed() <= Duration::from_secs(10),
            "C6 scale search run {run} exceeded ten seconds: {:?}",
            started.elapsed()
        );
        assert_eq!(result.total_records(), 1_000, "C6 scale match count");
        if let Some(expected) = stable_text.as_ref() {
            assert_eq!(result.text(), expected, "C6 five-run deterministic text");
        } else {
            stable_text = Some(result.text().to_owned());
        }
    }

    let glob_started = Instant::now();
    let glob = engine
        .glob(
            GlobRequest::new(
                GlobTarget::new("tree/**/*.txt").expect("C7 scale glob target"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C7 scale glob");
    assert!(
        glob_started.elapsed() <= Duration::from_secs(10),
        "C7 scale glob exceeded ten seconds: {:?}",
        glob_started.elapsed()
    );
    assert_eq!(glob.total_records(), 10_000, "C7 scale glob count");

    #[cfg(unix)]
    {
        let partial = engine
            .search(
                SearchRequest::new(
                    SearchTarget::resource(reference("tree")),
                    "absent-from-scale-tree",
                    SearchOptions::default(),
                    0,
                    SearchLimits::default(),
                )
                .expect("C14 diagnostic request"),
                &OperationGuard::new(),
            )
            .await
            .expect("C14 diagnostic search");
        assert_eq!(
            partial.diagnostics().len(),
            1,
            "C14 vanished-entry diagnostic"
        );
        assert_eq!(
            partial.diagnostics()[0].category(),
            ErrorCategory::NotFound,
            "C14 vanished-entry category"
        );
    }

    let maximum = "x".repeat(MAX_ARTIFACT_BYTES);
    fs::write(root.join("maximum.txt"), &maximum).expect("C6 maximum file");
    drop(maximum);
    let maximum_started = Instant::now();
    let maximum_search = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(reference("maximum.txt")),
                "absent-pattern",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C6 maximum request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C6 maximum exact search");
    assert_eq!(
        maximum_search.total_records(),
        0,
        "C6 maximum no-match count"
    );
    assert!(
        maximum_started.elapsed() <= Duration::from_secs(2),
        "C6 maximum exact search exceeded two seconds: {:?}",
        maximum_started.elapsed()
    );
}
