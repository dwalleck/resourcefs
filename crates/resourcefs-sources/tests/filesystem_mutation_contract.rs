use std::{
    collections::HashMap,
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use resourcefs_core::{
    ArtifactId, ErrorCategory, MAX_ARTIFACT_BYTES, MutationAdapter, MutationEngine,
    MutationOperation, OperationGuard, PathReference, PathSession, ResourceError, ServerLimits,
    SessionStorage, SessionToken, VersionTag, WorkspaceRootId, WriteRequest,
};
use resourcefs_sources::{
    BackingPathVisibility, FilesystemSource, LaunchRoot, LaunchRootSource, MutationGrants,
};
use tempfile::TempDir;
use tokio::sync::Mutex;

#[derive(Default)]
struct MemoryStorage {
    content: Mutex<HashMap<ArtifactId, Vec<u8>>>,
}

#[async_trait]
impl SessionStorage for MemoryStorage {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError> {
        Ok(self
            .content
            .lock()
            .await
            .get(&id)
            .is_some_and(|stored| stored == content))
    }

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError> {
        self.content.lock().await.insert(id, content.to_vec());
        Ok(())
    }

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError> {
        let bytes = self
            .content
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| ResourceError::new(ErrorCategory::NotFound, "missing"))?;
        String::from_utf8(bytes)
            .map_err(|_| ResourceError::new(ErrorCategory::SourceUnavailable, "invalid UTF-8"))
    }

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError> {
        self.content.lock().await.remove(&id);
        Ok(())
    }

    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}

fn session() -> PathSession {
    PathSession::new(
        SessionToken::parse("00000000000000000000000000000077").expect("session token"),
        Arc::new(MemoryStorage::default()),
        ServerLimits::default(),
    )
}

async fn engine(root: &std::path::Path, grants: MutationGrants) -> MutationEngine {
    let source = FilesystemSource::new(
        LaunchRootSource::Profile(vec![LaunchRoot::new(
            WorkspaceRootId::new("workspace").expect("root ID"),
            root.to_owned(),
            grants,
        )]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");
    let adapter: Arc<dyn MutationAdapter> = Arc::new(source);
    MutationEngine::new(adapter, session())
}

fn reference(path: &str) -> PathReference {
    PathReference::parse(path).expect("workspace reference")
}

#[tokio::test]
async fn create_replace_state_matrix_preserves_bytes_and_permissions() {
    let workspace = TempDir::new().expect("workspace");
    let engine = engine(workspace.path(), MutationGrants::new(true, true, true)).await;
    let path = reference("fixture.txt");

    let created = engine
        .write(
            WriteRequest::new(path.clone(), "alpha\r\nbeta\r\n".to_owned(), None)
                .expect("create request"),
            &OperationGuard::new(),
        )
        .await
        .expect("create");
    assert_eq!(created.operation(), MutationOperation::Created);
    assert_eq!(
        fs::read(workspace.path().join("fixture.txt")).expect("created bytes"),
        b"alpha\r\nbeta\r\n"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            workspace.path().join("fixture.txt"),
            fs::Permissions::from_mode(0o640),
        )
        .expect("fixture mode");
    }

    let current = created.version_tag().expect("created tag").clone();
    let replaced = engine
        .write(
            WriteRequest::new(
                path.clone(),
                "replacement\n".to_owned(),
                Some(current.clone()),
            )
            .expect("replace request"),
            &OperationGuard::new(),
        )
        .await
        .expect("replace");
    assert_eq!(replaced.operation(), MutationOperation::Replaced);
    assert_eq!(
        fs::read(workspace.path().join("fixture.txt")).expect("replaced bytes"),
        b"replacement\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(workspace.path().join("fixture.txt"))
                .expect("replacement metadata")
                .permissions()
                .mode()
                & 0o7777,
            0o640
        );
    }

    let before = fs::read(workspace.path().join("fixture.txt")).expect("before stale");
    let stale = engine
        .write(
            WriteRequest::new(path.clone(), "stale\n".to_owned(), Some(current))
                .expect("stale request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("stale replacement");
    assert_eq!(stale.category(), ErrorCategory::VersionConflict);

    assert_eq!(
        fs::read(workspace.path().join("fixture.txt")).expect("after stale"),
        before
    );

    let append = format!(
        "[{}#{}]\nPUT >$:\n+tail",
        replaced.canonical_reference().requested(),
        replaced.version_tag().expect("replacement tag")
    );
    let appended = engine
        .edit(&append, &OperationGuard::new())
        .await
        .expect("append edit");
    assert_eq!(
        fs::read(workspace.path().join("fixture.txt")).expect("appended bytes"),
        b"replacement\ntail\n"
    );
    let cut = format!(
        "[{}#{}]\nCUT 1.=1",
        appended.canonical_reference().requested(),
        appended.version_tag().expect("appended tag")
    );
    let cut_receipt = engine
        .edit(&cut, &OperationGuard::new())
        .await
        .expect("cut edit");
    assert_eq!(
        fs::read(workspace.path().join("fixture.txt")).expect("cut bytes"),
        b"tail\n"
    );
    let rem = format!(
        "[{}#{}]\nREM",
        cut_receipt.canonical_reference().requested(),
        cut_receipt.version_tag().expect("cut tag")
    );
    let deleted = engine
        .edit(&rem, &OperationGuard::new())
        .await
        .expect("REM");
    assert_eq!(deleted.operation(), MutationOperation::Deleted);
    assert!(!workspace.path().join("fixture.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_replacements_serialize_and_reject_one_stale_contender() {
    let workspace = TempDir::new().expect("workspace");
    fs::write(workspace.path().join("fixture.txt"), "original\n").expect("fixture");
    let engine = engine(workspace.path(), MutationGrants::new(false, true, false)).await;
    let expected = VersionTag::from_content(b"original\n");
    let first_engine = engine.clone();
    let first_tag = expected.clone();
    let first = tokio::spawn(async move {
        first_engine
            .write(
                WriteRequest::new(
                    reference("fixture.txt"),
                    "first\n".to_owned(),
                    Some(first_tag),
                )
                .expect("first request"),
                &OperationGuard::new(),
            )
            .await
    });
    let second = tokio::spawn(async move {
        engine
            .write(
                WriteRequest::new(
                    reference("fixture.txt"),
                    "second\n".to_owned(),
                    Some(expected),
                )
                .expect("second request"),
                &OperationGuard::new(),
            )
            .await
    });

    let outcomes = [
        first.await.expect("first task"),
        second.await.expect("second task"),
    ];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| result
                .as_ref()
                .is_err_and(|error| error.category() == ErrorCategory::VersionConflict))
            .count(),
        1
    );
    let final_bytes = fs::read(workspace.path().join("fixture.txt")).expect("final bytes");
    assert!(final_bytes == b"first\n" || final_bytes == b"second\n");
}

#[tokio::test]
async fn policy_precedes_state_and_creation_never_creates_parents() {
    let workspace = TempDir::new().expect("workspace");
    fs::write(workspace.path().join("existing.txt"), "existing\n").expect("existing fixture");
    let denied = engine(workspace.path(), MutationGrants::default()).await;

    for path in ["missing.txt", "existing.txt"] {
        let error = denied
            .write(
                WriteRequest::new(reference(path), "content".to_owned(), None)
                    .expect("denied request"),
                &OperationGuard::new(),
            )
            .await
            .expect_err("denied create");
        assert_eq!(error.category(), ErrorCategory::PermissionDenied, "{path}");
    }

    let granted = engine(workspace.path(), MutationGrants::new(true, false, false)).await;
    let error = granted
        .write(
            WriteRequest::new(reference("missing/child.txt"), "content".to_owned(), None)
                .expect("missing-parent request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("missing parent");
    assert_eq!(error.category(), ErrorCategory::NotFound);
    assert!(!workspace.path().join("missing").exists());
}
#[tokio::test]
async fn mv_is_atomic_no_clobber_and_preserves_both_entries_on_conflict() {
    let workspace = TempDir::new().expect("workspace");
    let engine = engine(workspace.path(), MutationGrants::new(true, false, true)).await;
    let created = engine
        .write(
            WriteRequest::new(reference("source.txt"), "source bytes\n".to_owned(), None)
                .expect("source create"),
            &OperationGuard::new(),
        )
        .await
        .expect("source fixture");
    fs::write(
        workspace.path().join("destination.txt"),
        "destination bytes\n",
    )
    .expect("destination fixture");
    let patch = format!(
        "[{}#{}]\nMV destination.txt",
        created.canonical_reference().requested(),
        created.version_tag().expect("source tag")
    );

    let conflict = engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect_err("existing destination");
    assert_eq!(conflict.category(), ErrorCategory::VersionConflict);
    assert_eq!(
        fs::read(workspace.path().join("source.txt")).expect("source preserved"),
        b"source bytes\n"
    );
    assert_eq!(
        fs::read(workspace.path().join("destination.txt")).expect("destination preserved"),
        b"destination bytes\n"
    );

    fs::remove_file(workspace.path().join("destination.txt")).expect("remove conflict fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        fs::write(workspace.path().join("other.txt"), "other\n").expect("other fixture");
        symlink(
            std::path::Path::new("other.txt"),
            workspace.path().join("destination.txt"),
        )
        .expect("linked destination");
        let linked = engine
            .edit(&patch, &OperationGuard::new())
            .await
            .expect_err("linked destination");
        assert_eq!(linked.category(), ErrorCategory::PermissionDenied);
        assert_eq!(
            fs::read(workspace.path().join("source.txt")).expect("source after linked denial"),
            b"source bytes\n"
        );
        fs::remove_file(workspace.path().join("destination.txt")).expect("remove linked fixture");
    }
    let moved = engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect("MV");
    assert_eq!(moved.operation(), MutationOperation::Moved);
    assert_eq!(
        moved
            .source_reference()
            .expect("source reference")
            .requested(),
        "rfs://workspace/workspace/source.txt"
    );
    assert_eq!(
        moved.canonical_reference().requested(),
        "rfs://workspace/workspace/destination.txt"
    );
    assert!(moved.version_tag().is_some());
    assert_eq!(moved.displayed_ranges(), None);
    assert!(!workspace.path().join("source.txt").exists());
    assert_eq!(
        fs::read(workspace.path().join("destination.txt")).expect("moved bytes"),
        b"source bytes\n"
    );
}

#[tokio::test]
async fn mv_can_cross_workspace_roots_with_exact_independent_grants() {
    let temporary = TempDir::new().expect("temporary roots");
    let alpha = temporary.path().join("alpha");
    let beta = temporary.path().join("beta");
    fs::create_dir(&alpha).expect("alpha root");
    fs::create_dir(&beta).expect("beta root");
    let source = FilesystemSource::new(
        LaunchRootSource::Profile(vec![
            LaunchRoot::new(
                WorkspaceRootId::new("alpha").expect("alpha ID"),
                alpha.clone(),
                MutationGrants::new(true, false, true),
            ),
            LaunchRoot::new(
                WorkspaceRootId::new("beta").expect("beta ID"),
                beta.clone(),
                MutationGrants::new(true, false, false),
            ),
        ]),
        Some("alpha".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("multi-root source");
    let engine = MutationEngine::new(Arc::new(source), session());
    let created = engine
        .write(
            WriteRequest::new(
                reference("rfs://workspace/alpha/source.txt"),
                "cross-root\n".to_owned(),
                None,
            )
            .expect("source create"),
            &OperationGuard::new(),
        )
        .await
        .expect("source");
    assert_eq!(
        created.canonical_reference().requested(),
        "rfs://workspace/alpha/source.txt"
    );
    assert!(
        alpha.join("source.txt").exists(),
        "source path missing; beta path exists={}, base path exists={}",
        beta.join("source.txt").exists(),
        temporary.path().join("source.txt").exists()
    );
    let patch = format!(
        "[{}#{}]\nMV rfs://workspace/beta/moved.txt",
        created.canonical_reference().requested(),
        created.version_tag().expect("source tag")
    );

    engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect("cross-root same-Source MV");

    assert!(!alpha.join("source.txt").exists());
    assert_eq!(
        fs::read(beta.join("moved.txt")).expect("moved bytes"),
        b"cross-root\n"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn mv_never_copies_across_filesystems() {
    use std::os::unix::fs::MetadataExt;

    let source_root = TempDir::new().expect("source root");
    let destination_root = TempDir::new_in("/dev/shm").expect("cross-filesystem root");
    assert_ne!(
        fs::metadata(source_root.path())
            .expect("source metadata")
            .dev(),
        fs::metadata(destination_root.path())
            .expect("destination metadata")
            .dev(),
        "fixture roots must use distinct filesystems"
    );
    let source = FilesystemSource::new(
        LaunchRootSource::Profile(vec![
            LaunchRoot::new(
                WorkspaceRootId::new("source").expect("source ID"),
                source_root.path().to_owned(),
                MutationGrants::new(true, false, true),
            ),
            LaunchRoot::new(
                WorkspaceRootId::new("destination").expect("destination ID"),
                destination_root.path().to_owned(),
                MutationGrants::new(true, false, false),
            ),
        ]),
        Some("source".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("cross-filesystem source");
    let engine = MutationEngine::new(Arc::new(source), session());
    let created = engine
        .write(
            WriteRequest::new(
                reference("rfs://workspace/source/source.txt"),
                "do not copy\n".to_owned(),
                None,
            )
            .expect("source create"),
            &OperationGuard::new(),
        )
        .await
        .expect("source");
    let patch = format!(
        "[{}#{}]\nMV rfs://workspace/destination/moved.txt",
        created.canonical_reference().requested(),
        created.version_tag().expect("source tag")
    );

    let error = engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect_err("cross-filesystem MV");

    assert_eq!(error.category(), ErrorCategory::UnsupportedMutation);
    assert!(source_root.path().join("source.txt").exists());
    assert!(!destination_root.path().join("moved.txt").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn retargeted_links_never_mutate_outside() {
    use std::os::unix::fs::symlink;

    let workspace = TempDir::new().expect("workspace");
    let outside = TempDir::new().expect("outside");
    let outside_target = outside.path().join("sentinel.txt");
    let outside_destination = outside.path().join("moved.txt");
    fs::write(&outside_target, "outside sentinel\n").expect("outside sentinel");
    symlink(outside.path(), workspace.path().join("linked-parent")).expect("escaping parent link");
    let engine = engine(workspace.path(), MutationGrants::new(true, false, true)).await;
    let created = engine
        .write(
            WriteRequest::new(reference("source.txt"), "inside\n".to_owned(), None)
                .expect("source create"),
            &OperationGuard::new(),
        )
        .await
        .expect("source");
    let patch = format!(
        "[{}#{}]\nMV linked-parent/moved.txt",
        created.canonical_reference().requested(),
        created.version_tag().expect("source tag")
    );

    let error = engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect_err("escaping parent link");

    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    assert_eq!(
        fs::read(&outside_target).expect("outside sentinel after"),
        b"outside sentinel\n"
    );
    assert!(!outside_destination.exists());
    assert!(workspace.path().join("source.txt").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn linked_and_binary_targets_fail_without_state_change() {
    use std::os::unix::fs::symlink;

    let workspace = TempDir::new().expect("workspace");
    fs::write(workspace.path().join("target.txt"), "target\n").expect("target fixture");
    fs::write(workspace.path().join("binary.bin"), b"good\xff").expect("binary fixture");
    symlink(
        workspace.path().join("target.txt"),
        workspace.path().join("link.txt"),
    )
    .expect("contained link");
    let engine = engine(workspace.path(), MutationGrants::new(false, true, false)).await;

    let linked = engine
        .write(
            WriteRequest::new(
                reference("link.txt"),
                "changed\n".to_owned(),
                Some(VersionTag::from_content(b"target\n")),
            )
            .expect("linked request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("linked mutation");
    assert_eq!(linked.category(), ErrorCategory::PermissionDenied);
    assert_eq!(
        fs::read(workspace.path().join("target.txt")).expect("target"),
        b"target\n"
    );

    let binary_before = fs::read(workspace.path().join("binary.bin")).expect("binary before");
    let binary = engine
        .write(
            WriteRequest::new(
                reference("binary.bin"),
                "changed\n".to_owned(),
                Some(VersionTag::from_content(&binary_before)),
            )
            .expect("binary request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("binary mutation");
    assert_eq!(binary.category(), ErrorCategory::UnsupportedMutation);
    assert_eq!(
        fs::read(workspace.path().join("binary.bin")).expect("binary after"),
        binary_before
    );
}

#[tokio::test]
async fn exact_limit_write_budget_and_one_over_rejection() {
    let workspace = TempDir::new().expect("workspace");
    let engine = engine(workspace.path(), MutationGrants::new(true, false, false)).await;
    let content = "x".repeat(MAX_ARTIFACT_BYTES);
    let started = Instant::now();
    engine
        .write(
            WriteRequest::new(reference("maximum.txt"), content, None)
                .expect("exact-limit request"),
            &OperationGuard::new(),
        )
        .await
        .expect("exact-limit write");
    let elapsed = started.elapsed();
    assert_eq!(
        fs::metadata(workspace.path().join("maximum.txt"))
            .expect("maximum metadata")
            .len(),
        MAX_ARTIFACT_BYTES as u64
    );
    assert!(
        elapsed <= Duration::from_secs(5),
        "64 MiB create took {elapsed:?}"
    );

    let error = WriteRequest::new(
        reference("over.txt"),
        "x".repeat(MAX_ARTIFACT_BYTES + 1),
        None,
    )
    .expect_err("one-over mutation input");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert!(!workspace.path().join("over.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_has_no_missing_window() {
    const REPLACEMENTS: usize = 128;
    const BYTES: usize = 1024 * 1024;
    let workspace = TempDir::new().expect("workspace");
    let path = workspace.path().join("fixture.txt");
    let old = vec![b'A'; BYTES];
    let new = vec![b'B'; BYTES];
    fs::write(&path, &old).expect("initial fixture");
    let engine = engine(workspace.path(), MutationGrants::new(false, true, false)).await;
    let stop = Arc::new(AtomicBool::new(false));
    let invalid = Arc::new(AtomicUsize::new(0));
    let read_errors = Arc::new(AtomicUsize::new(0));
    let reader = {
        let stop = Arc::clone(&stop);
        let invalid = Arc::clone(&invalid);
        let read_errors = Arc::clone(&read_errors);
        let path = path.clone();
        let old = old.clone();
        let new = new.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match fs::read(&path) {
                    Ok(content) if content == old || content == new => {}
                    Ok(_) => {
                        invalid.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        read_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        })
    };

    let mut maximum_replacement = Duration::ZERO;
    let mut current = VersionTag::from_content(&old);
    for iteration in 0..REPLACEMENTS {
        let content = if iteration % 2 == 0 { &new } else { &old };
        let replacement_started = Instant::now();
        let receipt = engine
            .write(
                WriteRequest::new(
                    reference("fixture.txt"),
                    String::from_utf8(content.clone()).expect("ASCII fixture"),
                    Some(current),
                )
                .expect("replace request"),
                &OperationGuard::new(),
            )
            .await
            .expect("atomic replacement");
        current = receipt.version_tag().expect("replacement tag").clone();
        maximum_replacement = maximum_replacement.max(replacement_started.elapsed());
    }
    stop.store(true, Ordering::Release);
    reader.join().expect("reader thread");

    assert_eq!(invalid.load(Ordering::Acquire), 0);
    assert_eq!(read_errors.load(Ordering::Acquire), 0);
    assert!(
        maximum_replacement <= Duration::from_secs(5),
        "slowest one-MiB replacement took {maximum_replacement:?}"
    );
}
