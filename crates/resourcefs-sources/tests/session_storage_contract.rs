#![cfg(feature = "test-support")]

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use resourcefs_core::{
    ArtifactId, ErrorCategory, MAX_ARTIFACT_BYTES, OperationGuard, SessionStorage,
};
use resourcefs_sources::{SESSION_CLEANUP_TTL, SessionStore, StorageFailurePoint, StoredSession};
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

async fn store() -> (TempDir, SessionStore) {
    let temporary = TempDir::new().expect("temporary cache");
    let store = SessionStore::open(temporary.path())
        .await
        .expect("session store");
    (temporary, store)
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
