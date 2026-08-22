#![cfg(feature = "test-support")]

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use resourcefs_core::{
    ArtifactId, ErrorCategory, MAX_ARTIFACT_BYTES, OperationGuard, SessionStorage,
};
use resourcefs_sources::{
    SESSION_CLEANUP_TTL, SessionStorageConfig, SessionStore, StorageFailurePoint, StoredSession,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn directory_snapshot(root: &Path) -> Vec<(PathBuf, [u8; 32])> {
    fn visit(root: &Path, current: &Path, output: &mut Vec<(PathBuf, [u8; 32])>) {
        let mut entries: Vec<_> = fs::read_dir(current)
            .expect("snapshot directory")
            .map(|entry| entry.expect("snapshot entry"))
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).expect("snapshot metadata");
            if metadata.file_type().is_symlink() {
                output.push((
                    path.strip_prefix(root)
                        .expect("relative symlink")
                        .to_owned(),
                    Sha256::digest(b"symlink").into(),
                ));
            } else if metadata.is_dir() {
                visit(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root).expect("relative file").to_owned(),
                    Sha256::digest(fs::read(&path).expect("snapshot file")).into(),
                ));
            }
        }
    }

    let mut snapshot = Vec::new();
    visit(root, root, &mut snapshot);
    snapshot
}

fn config(cache_root: impl Into<PathBuf>, retention_ttl_seconds: i64) -> SessionStorageConfig {
    SessionStorageConfig::new(cache_root, retention_ttl_seconds)
        .expect("valid session storage config")
}

async fn store() -> (TempDir, SessionStore) {
    let temporary = TempDir::new().expect("temporary cache");
    let store = SessionStore::open_with(config(
        temporary.path(),
        SESSION_CLEANUP_TTL.as_secs() as i64,
    ))
    .await
    .expect("session store");
    (temporary, store)
}

#[test]
fn storage_configuration_ttl_boundaries_are_exact() {
    let cache_root = PathBuf::from("cache");
    for (seconds, accepted) in [(-1, false), (0, true), (86_400, true), (86_401, false)] {
        let result = SessionStorageConfig::new(cache_root.clone(), seconds);
        assert_eq!(result.is_ok(), accepted, "retentionTtlSeconds={seconds}");
        match result {
            Ok(config) => assert_eq!(
                config.retention_ttl(),
                Duration::from_secs(seconds as u64),
                "retentionTtlSeconds={seconds}"
            ),
            Err(error) => {
                assert_eq!(error.category(), ErrorCategory::LimitExceeded);
                assert!(error.message().contains("session.retentionTtlSeconds"));
            }
        }
    }
}

#[tokio::test]
async fn relative_and_absolute_cache_bases_use_one_owned_namespace() {
    let current = std::env::current_dir().expect("current directory");
    let relative_temporary = TempDir::new_in(&current).expect("relative cache parent");
    let relative_root = relative_temporary
        .path()
        .strip_prefix(&current)
        .expect("relative cache path")
        .to_owned();
    assert!(relative_root.is_relative());
    let relative_store =
        SessionStore::open_with(config(relative_root, SESSION_CLEANUP_TTL.as_secs() as i64))
            .await
            .expect("relative session store");
    assert_eq!(
        relative_store.sessions_root_for_test(),
        relative_temporary
            .path()
            .canonicalize()
            .expect("canonical relative cache")
            .join("resourcefs")
    );

    let absolute_temporary = TempDir::new().expect("absolute cache parent");
    let absolute_store = SessionStore::open_with(config(
        absolute_temporary.path(),
        SESSION_CLEANUP_TTL.as_secs() as i64,
    ))
    .await
    .expect("absolute session store");
    assert_eq!(
        absolute_store.sessions_root_for_test(),
        absolute_temporary
            .path()
            .canonicalize()
            .expect("canonical absolute cache")
            .join("resourcefs")
    );
}

#[tokio::test]
async fn cleanup_never_traverses_the_operator_cache_base() {
    let temporary = TempDir::new().expect("operator cache base");
    let sibling = temporary.path().join("abcdef0123456789abcdef0123456789");
    fs::create_dir(&sibling).expect("sibling session-shaped directory");
    fs::write(sibling.join("session.lock"), "operator-owned\n").expect("sibling lease");
    let marker = sibling.join("disconnected");
    fs::write(&marker, "operator-owned\n").expect("sibling marker");
    fs::File::open(&marker)
        .expect("open sibling marker")
        .set_times(
            fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
        )
        .expect("age sibling marker");
    let before = directory_snapshot(&sibling);

    let store = SessionStore::open_with(config(
        temporary.path(),
        SESSION_CLEANUP_TTL.as_secs() as i64,
    ))
    .await
    .expect("contained session store");

    assert_eq!(directory_snapshot(&sibling), before);
    assert_eq!(
        store.sessions_root_for_test(),
        temporary
            .path()
            .canonicalize()
            .expect("canonical cache base")
            .join("resourcefs")
    );
}

#[tokio::test]
async fn zero_ttl_deletes_at_disconnect_after_releasing_the_lease() {
    let temporary = TempDir::new().expect("zero-TTL cache");
    let sibling = temporary.path().join("operator-sibling");
    fs::create_dir(&sibling).expect("operator sibling");
    fs::write(sibling.join("sentinel"), "untouched\n").expect("operator sentinel");
    let before = directory_snapshot(&sibling);
    let store = SessionStore::open_with(config(temporary.path(), 0))
        .await
        .expect("zero-TTL store");
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("zero-TTL session");
    let session_directory = session.storage_for_test().session_dir_for_test().to_owned();
    retain(&session, "retained until disconnect").await;

    session
        .mark_disconnected()
        .await
        .expect("zero-TTL disconnect");

    assert!(!session_directory.exists());
    assert_eq!(directory_snapshot(&sibling), before);
}

#[cfg(unix)]
#[tokio::test]
async fn namespace_creation_propagates_permission_denial() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TempDir::new().expect("permission cache");
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o500))
        .expect("deny namespace creation");
    let result = SessionStore::open_with(config(
        temporary.path(),
        SESSION_CLEANUP_TTL.as_secs() as i64,
    ))
    .await;
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
        .expect("restore cache permissions");

    let error = result.expect_err("permission denial must fail");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
}

async fn retain(session: &StoredSession, content: &str) -> resourcefs_core::ArtifactAddress {
    session
        .path_session()
        .retain(content, &OperationGuard::new())
        .await
        .expect("retain artifact")
}

#[tokio::test]
async fn artifact_storage_failure_is_atomic() {
    for point in [
        StorageFailurePoint::Open,
        StorageFailurePoint::Write,
        StorageFailurePoint::Sync,
        StorageFailurePoint::Persist,
    ] {
        let (_temporary, store) = store().await;
        let session = store
            .create_session(resourcefs_core::ServerLimits::default())
            .await
            .expect("stored session");
        let stable = retain(&session, "stable").await;
        let before = directory_snapshot(session.storage_for_test().session_dir_for_test());
        let used_before = session.path_session().used_bytes().await;
        session.storage_for_test().fail_next(point).await;

        let error = session
            .path_session()
            .retain("new content", &OperationGuard::new())
            .await
            .expect_err("injected storage failure");
        assert_eq!(
            error.category(),
            ErrorCategory::SourceUnavailable,
            "{point:?}"
        );
        assert_eq!(session.path_session().used_bytes().await, used_before);
        assert_eq!(session.path_session().artifact_count().await, 1);
        assert_eq!(
            directory_snapshot(session.storage_for_test().session_dir_for_test()),
            before,
            "{point:?}"
        );
        assert_eq!(
            session
                .path_session()
                .read_artifact(&stable)
                .await
                .expect("stable artifact"),
            "stable"
        );
    }
}

#[tokio::test]
async fn remove_failure_keeps_the_published_object_unchanged() {
    let (_temporary, store) = store().await;
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("stored session");
    let address = retain(&session, "kept").await;
    let before = directory_snapshot(session.storage_for_test().session_dir_for_test());
    session
        .storage_for_test()
        .fail_next(StorageFailurePoint::Remove)
        .await;

    let error = session
        .storage_for_test()
        .remove(ArtifactId::new(address.object_id()).expect("artifact ID"))
        .await
        .expect_err("injected remove failure");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert_eq!(
        directory_snapshot(session.storage_for_test().session_dir_for_test()),
        before
    );
    assert_eq!(
        session
            .path_session()
            .read_artifact(&address)
            .await
            .expect("published object remains"),
        "kept"
    );
}

#[tokio::test]
async fn heartbeat_updates_persisted_liveness_within_budget() {
    let (_temporary, store) = store().await;
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("stored session");
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    session
        .storage_for_test()
        .set_age_marker_for_test(false, old)
        .expect("old live marker");

    let started = Instant::now();
    session.heartbeat().await.expect("heartbeat");
    let elapsed = started.elapsed();
    let modified = fs::metadata(
        session
            .storage_for_test()
            .session_dir_for_test()
            .join("last-live"),
    )
    .and_then(|metadata| metadata.modified())
    .expect("heartbeat timestamp");
    assert!(modified > old);
    if !cfg!(debug_assertions) {
        assert!(
            elapsed <= Duration::from_millis(100),
            "heartbeat took {elapsed:?}"
        );
    }
}

#[tokio::test]
async fn session_cleanup_respects_lease_and_ttl() {
    let (_temporary, store) = store().await;
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000);

    let fresh = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("fresh session");
    let fresh_token = fresh.path_session().token().as_str().to_owned();
    fresh.mark_disconnected().await.expect("fresh tombstone");
    fresh
        .storage_for_test()
        .set_age_marker_for_test(true, now - SESSION_CLEANUP_TTL + Duration::from_secs(1))
        .expect("fresh age");
    drop(fresh);

    let exact = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("exact session");
    let exact_token = exact.path_session().token().as_str().to_owned();
    exact.mark_disconnected().await.expect("exact tombstone");
    exact
        .storage_for_test()
        .set_age_marker_for_test(true, now - SESSION_CLEANUP_TTL)
        .expect("exact age");
    drop(exact);

    let old = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("old session");
    let old_token = old.path_session().token().as_str().to_owned();
    old.mark_disconnected().await.expect("old tombstone");
    old.storage_for_test()
        .set_age_marker_for_test(true, now - SESSION_CLEANUP_TTL - Duration::from_secs(1))
        .expect("old age");
    drop(old);

    let live = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("live session");
    let live_token = live.path_session().token().as_str().to_owned();
    fs::remove_file(
        live.storage_for_test()
            .session_dir_for_test()
            .join("last-live"),
    )
    .expect("remove live age marker");

    let report = store
        .cleanup_expired_at_for_test(now)
        .await
        .expect("first cleanup");
    assert_eq!(report.removed, 2);
    assert_eq!(report.live, 1);
    assert_eq!(report.fresh, 1);
    let sessions = store.sessions_root_for_test();
    assert!(sessions.join(fresh_token).is_dir());
    assert!(!sessions.join(exact_token).exists());
    assert!(!sessions.join(old_token).exists());
    assert!(sessions.join(&live_token).is_dir());

    live.heartbeat().await.expect("restore abandoned marker");
    live.storage_for_test()
        .set_age_marker_for_test(false, now - SESSION_CLEANUP_TTL - Duration::from_secs(1))
        .expect("abandoned age");

    drop(live);
    let report = store
        .cleanup_expired_at_for_test(now)
        .await
        .expect("abandoned cleanup");
    assert_eq!(report.removed, 1);
    assert!(!sessions.join(live_token).exists());
}

#[cfg(unix)]
#[tokio::test]
async fn cleanup_ignores_symlinked_and_malformed_session_entries() {
    use std::os::unix::fs::symlink;

    let (temporary, store) = store().await;
    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).expect("outside sentinel directory");
    fs::write(outside.join("sentinel"), "untouched").expect("outside sentinel");
    let valid_name = "abcdef0123456789abcdef0123456789";
    symlink(&outside, store.sessions_root_for_test().join(valid_name)).expect("session symlink");
    fs::create_dir(store.sessions_root_for_test().join("not-a-token"))
        .expect("malformed session directory");

    let report = store
        .cleanup_expired_at_for_test(SystemTime::now() + SESSION_CLEANUP_TTL)
        .await
        .expect("safe cleanup");
    assert_eq!(report.removed, 0);
    assert_eq!(
        fs::read_to_string(outside.join("sentinel")).expect("outside sentinel remains"),
        "untouched"
    );
    assert!(store.sessions_root_for_test().join(valid_name).is_symlink());
    assert!(store.sessions_root_for_test().join("not-a-token").is_dir());
}

#[tokio::test]
async fn durable_write_at_object_ceiling_within_budget() {
    let (_temporary, store) = store().await;
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("stored session");
    let mut content = "x".repeat(MAX_ARTIFACT_BYTES);
    content.replace_range(MAX_ARTIFACT_BYTES - 1.., "\n");

    let started = Instant::now();
    let address = retain(&session, &content).await;
    let elapsed = started.elapsed();
    assert_eq!(
        session.path_session().used_bytes().await,
        MAX_ARTIFACT_BYTES
    );
    assert_eq!(session.path_session().artifact_count().await, 1);
    assert_eq!(
        session
            .path_session()
            .read_artifact(&address)
            .await
            .expect("read exact object"),
        content
    );
    if !cfg!(debug_assertions) {
        assert!(
            elapsed <= Duration::from_secs(5),
            "64 MiB durable admission took {elapsed:?}"
        );
    }
}
