//! Live read-only smoke over the real GitHub API (rfs-45ww).
//!
//! The permanent adapter contracts run against a deterministic fake upstream,
//! which can only be as faithful as the fixture author's model of GitHub — the
//! `/repositories/{id}` Link form (review finding F1) was captured in
//! `.rfs-45ww/evidence.md` and still missed the fixture. This suite exists to
//! catch that class of drift: every row targets a shape the fake cannot be
//! trusted to imitate, against public `rust-lang/rust` data, GET only.
//!
//! Ignored by default and skipped without explicit opt-in and a token; run with
//! `RFS_LIVE=1 GITHUB_TOKEN="$(gh auth token)" cargo test -p resourcefs-sources --test
//! github_live_smoke -- --ignored --nocapture`.
#[path = "support/mod.rs"]
mod session_support;

use std::sync::Arc;

use resourcefs_core::{
    AllowedOrigin, DiscoveryEngine, ErrorCategory, HttpCeilings, OperationGuard, OriginAllowlist,
    PathReference, ReadAcquisitionLimits, SearchLimits, SearchOptions, SearchRequest, SearchTarget,
    Secret, ServerLimits, SourceAdapter, SourceResource,
};
use resourcefs_sources::{
    GithubConfig, GithubRepository, GithubSource, GithubSourceMount, HttpSubstrate, MutationGrants,
    OriginCredential, SecretReference,
};

const REPOSITORY: &str = "rust-lang/rust";
/// Evidence P1–P6 fixture: 7 conversation comments, 6 inline comments, 4 files.
const SMALL_PR: u64 = 159_232;
/// A public PR with more than one page of conversation comments.
const MULTIPAGE_PR: u64 = 159_628;
/// 56 changed files at the time of writing — past the endpoint's 30-row default.
const WIDE_PR: u64 = 161_878;

fn live_token() -> Option<String> {
    if std::env::var("RFS_LIVE").as_deref() != Ok("1") {
        eprintln!("RFS_LIVE is not 1; skipping live GitHub smoke");
        return None;
    }
    match std::env::var("GITHUB_TOKEN") {
        Ok(token) if !token.is_empty() => Some(token),
        _ => {
            eprintln!("GITHUB_TOKEN is absent or empty; skipping live GitHub smoke");
            None
        }
    }
}

/// An independent native observation, or `None` when the `gh` CLI is absent.
///
/// `gh` is a dependency beyond the `RFS_LIVE`/`GITHUB_TOKEN` gate, and
/// `scripts/live-smoke.sh` already degrades when it produces nothing, so a
/// runner without it skips this cross-check rather than failing the row.
fn native_gh(token: &str, endpoint: &str) -> Option<serde_json::Value> {
    let native = match std::process::Command::new("gh")
        .args(["api", "-H", "X-GitHub-Api-Version: 2022-11-28", endpoint])
        .env("GH_TOKEN", token)
        .output()
    {
        Ok(native) => native,
        Err(error) => {
            eprintln!("gh CLI is unavailable ({error}); skipping native cross-check");
            return None;
        }
    };
    assert!(native.status.success(), "native read failed");
    Some(serde_json::from_slice(&native.stdout).expect("native JSON"))
}

async fn live_source(token: String) -> (session_support::ScratchFixture, GithubSource) {
    let origin = AllowedOrigin::new("https://api.github.com/", false).expect("origin");
    let secret = Secret::new(token).expect("token is a valid secret");
    let credential =
        OriginCredential::new(origin.clone(), "Authorization", Some("Bearer"), &secret)
            .expect("credential");
    let substrate = HttpSubstrate::new(
        OriginAllowlist::new(vec![origin]),
        HttpCeilings::default(),
        vec![credential],
    )
    .expect("substrate");
    let config = GithubConfig::new(
        "github",
        true,
        MutationGrants::default(),
        resourcefs_sources::GithubDeployment::default(),
        false,
        SecretReference::environment("GITHUB_TOKEN").expect("secret reference"),
        vec![GithubRepository::new(REPOSITORY, MutationGrants::default()).expect("repository")],
        resourcefs_core::ReadAcquisitionLimits::default(),
    )
    .expect("GitHub config");
    let session = session_support::scratch_fixture().await;
    let source = GithubSourceMount::new(config, Arc::new(substrate))
        .bind(session.path_session().clone())
        .expect("GitHub source");
    (session, source)
}

async fn read(source: &GithubSource, reference: &str) -> SourceResource {
    source
        .read(
            &PathReference::parse(reference).expect("reference"),
            &OperationGuard::new(),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("{reference}: {:?} {}", error.category(), error.message()))
}

async fn read_with_limits(
    source: &GithubSource,
    reference: &str,
    limits: &ReadAcquisitionLimits,
) -> SourceResource {
    source
        .read(
            &PathReference::parse(reference).expect("reference"),
            &OperationGuard::new(),
            Some(limits),
        )
        .await
        .unwrap_or_else(|error| panic!("{reference}: {:?} {}", error.category(), error.message()))
}

async fn read_err(source: &GithubSource, reference: &str) -> ErrorCategory {
    source
        .read(
            &PathReference::parse(reference).expect("reference"),
            &OperationGuard::new(),
            None,
        )
        .await
        .map(|resource| {
            panic!(
                "{reference}: expected an error, read {} bytes",
                resource.content().len()
            )
        })
        .unwrap_err()
        .category()
}

fn first_reference_id(listing: &str, prefix: &str) -> u64 {
    listing
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .and_then(|rest| rest.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no `{prefix}<id>` row in listing:\n{listing}"))
}

#[tokio::test]
#[ignore = "live GitHub smoke; needs RFS_LIVE=1 and GITHUB_TOKEN"]
async fn live_github_reads_hold_up() {
    let Some(token) = live_token() else { return };
    let (_session, source) = live_source(token).await;

    // L1 — a >1,000-issue collection paginates through GitHub's real
    // `/repositories/{id}?…&after=…` Link targets and names page 11.
    let listing = read(&source, "issue://rust-lang/rust").await;
    let rows = listing
        .content()
        .matches("\n- issue://rust-lang/rust/")
        .count();
    eprintln!(
        "L1 issue collection: {rows} issue rows, continuation {:?}",
        listing.continuation()
    );
    // Ten pages fetch 1,000 objects, but the issues endpoint interleaves pull
    // requests and the collection filters them out — on rust-lang/rust that
    // leaves roughly a third as issue rows.
    assert!(
        (1..=1_000).contains(&rows),
        "expected 1..=1000 issue rows, got {rows}"
    );
    assert_eq!(
        listing.continuation(),
        Some("issue://rust-lang/rust:page:11")
    );
    assert!(
        listing
            .content()
            .ends_with("\nContinuation: issue://rust-lang/rust:page:11\n")
    );
    let page_eleven = read(&source, "issue://rust-lang/rust:page:11").await;
    assert!(
        page_eleven
            .content()
            .starts_with("# Issues: rust-lang/rust\n")
    );
    eprintln!(
        "L1 page 11: {} rows, continuation {:?}",
        page_eleven.content().matches("\n- issue://").count(),
        page_eleven.continuation()
    );

    // L2 — a real PR aggregate and its distinct projections, with real
    // `issue_url` / `pull_request_url` parent checks on single comments.
    let base = format!("pr://rust-lang/rust/{SMALL_PR}");
    let aggregate = read(&source, &base).await;
    for expected in [
        "Kind: pull request",
        "Merged: true",
        &format!("Reference: {base}/reviews/"),
        &format!("Reference: {base}/review-comments/"),
        &format!("- {base}/diff/4 — "),
    ] {
        assert!(
            aggregate.content().contains(expected),
            "L2 aggregate lacks {expected}"
        );
    }
    assert!(!aggregate.content().contains(&format!("- {base}/diff/5 — ")));
    eprintln!("L2 aggregate: {} bytes", aggregate.content().len());
    let comments = read(&source, &format!("{base}/comments")).await;
    let comment_prefix = format!("{base}/comments/");
    let comment_id = first_reference_id(comments.content(), &comment_prefix);
    let comment = read(&source, &format!("{comment_prefix}{comment_id}")).await;
    assert!(comment.content().starts_with("Author: "));
    let inline = read(&source, &format!("{base}/review-comments")).await;
    let inline_prefix = format!("{base}/review-comments/");
    let inline_id = first_reference_id(inline.content(), &inline_prefix);
    let inline_comment = read(&source, &format!("{inline_prefix}{inline_id}")).await;
    assert!(inline_comment.content().contains("Diff hunk: "));
    let first_file = read(&source, &format!("{base}/diff/1")).await;
    assert!(first_file.content().starts_with("File: "));
    eprintln!("L2 projections: comment {comment_id}, inline {inline_id}, diff/1 ok");

    // L3 — a diff file index past the endpoint's 30-row default resolves on
    // its 100-per-page listing page.
    let wide = format!("pr://rust-lang/rust/{WIDE_PR}");
    let forty_fifth = read(&source, &format!("{wide}/diff/45")).await;
    assert!(forty_fifth.content().starts_with("File: "));
    eprintln!(
        "L3 diff/45: {}",
        forty_fifth.content().lines().next().unwrap_or_default()
    );

    // L4 — one object, one canonical namespace, and parents are verified.
    assert_eq!(
        read_err(&source, &format!("issue://rust-lang/rust/{SMALL_PR}")).await,
        ErrorCategory::NotFound,
        "L4 a pull request is not readable as an issue"
    );
    assert_eq!(
        read_err(
            &source,
            &format!("issue://rust-lang/rust/1/comments/{comment_id}")
        )
        .await,
        ErrorCategory::NotFound,
        "L4 a PR comment is not served under another parent"
    );
    assert_eq!(
        read_err(&source, "issue://rust-lang/rust/999999999").await,
        ErrorCategory::NotFound,
        "L4 an absent object"
    );
    eprintln!("L4 kind and parent refusals: not_found");

    // L5 — a repeat read revalidates through the real ETag/304 path and
    // returns identical content; a selected read keeps the same identity.
    let title = format!("{base}/title");
    let first = read(&source, &title).await;
    let second = read(&source, &title).await;
    assert_eq!(first.content(), second.content());
    assert_eq!(first.version_tag(), second.version_tag());
    eprintln!("L5 title twice: {:?}", first.content());

    // L6 — search over the live aggregate attributes hits to real lines.
    let session = session_support::scratch_fixture().await;
    let engine = DiscoveryEngine::new(
        Arc::new(source),
        session.path_session().clone(),
        ServerLimits::default(),
    );
    let search = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(PathReference::parse(&base).expect("aggregate")),
                "^Kind: ",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("search request"),
            &OperationGuard::new(),
        )
        .await
        .expect("live search");
    assert_eq!(search.total_records(), 1);
    assert_eq!(search.groups()[0].reference(), base);
    assert_eq!(search.groups()[0].lines()[0].line(), 3);
    eprintln!("L6 search: {:?}", search.groups()[0].lines()[0].text());
}

#[tokio::test]
#[ignore = "live GitHub smoke; needs RFS_LIVE=1 and GITHUB_TOKEN"]
async fn live_github_facts_preserve_native_identity_and_links() {
    let Some(token) = live_token() else { return };
    let Some(native) = native_gh(&token, &format!("repos/{REPOSITORY}/pulls/{SMALL_PR}")) else {
        return;
    };
    let (_session, source) = live_source(token).await;
    let resource = read(&source, &format!("pr://{REPOSITORY}/{SMALL_PR}/facts")).await;
    let facts: serde_json::Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(facts["schemaVersion"]["major"], 1);
    assert_eq!(
        facts["data"]["id"],
        native["id"].as_u64().expect("native id").to_string()
    );
    for side in ["base", "head"] {
        assert_eq!(facts["data"][side]["commitSha"], native[side]["sha"]);
    }
    assert_eq!(facts["data"]["links"]["apiUrl"], native["url"]);
    assert_eq!(facts["data"]["links"]["htmlUrl"], native["html_url"]);
    assert_eq!(facts["acquisition"]["restApiVersion"], "2022-11-28");
    assert!(resource.continuation().is_none());
}
#[tokio::test]
#[ignore = "live GitHub smoke; needs RFS_LIVE=1 and GITHUB_TOKEN"]
async fn live_github_conversation_comment_facts_hold_up() {
    let Some(token) = live_token() else { return };
    let Some(native_first_value) = native_gh(
        &token,
        &format!("repos/{REPOSITORY}/issues/{MULTIPAGE_PR}/comments?per_page=100&page=1"),
    ) else {
        return;
    };
    let Some(native_second_value) = native_gh(
        &token,
        &format!("repos/{REPOSITORY}/issues/{MULTIPAGE_PR}/comments?per_page=100&page=2"),
    ) else {
        return;
    };
    let native_first = native_first_value
        .as_array()
        .expect("native first comment page");
    let native_second = native_second_value
        .as_array()
        .expect("native second comment page");
    assert!(
        !native_first.is_empty(),
        "fixture PR must have first-page comments"
    );
    assert!(
        !native_second.is_empty(),
        "fixture PR must have second-page comments"
    );
    let native_ids = |page: &[serde_json::Value]| {
        page.iter()
            .map(|comment| {
                comment["id"]
                    .as_u64()
                    .expect("native comment id")
                    .to_string()
            })
            .collect::<Vec<_>>()
    };

    let (_session, source) = live_source(token).await;
    let limits =
        ReadAcquisitionLimits::new(Some(2), None, None, None, None).expect("two-attempt limit");
    let first = read_with_limits(
        &source,
        &format!("pr://{REPOSITORY}/{MULTIPAGE_PR}/comments/facts"),
        &limits,
    )
    .await;
    let first_facts: serde_json::Value =
        serde_json::from_str(first.content()).expect("first collection JSON");
    assert_eq!(
        first_facts["kind"],
        "github.conversation_comment_collection"
    );
    assert_eq!(first_facts["schemaVersion"]["major"], 1);
    assert_eq!(first_facts["request"]["number"], MULTIPAGE_PR.to_string());
    assert_eq!(first_facts["collection"]["scope"], "initial");
    assert_eq!(first_facts["collection"]["state"], "incomplete");
    assert!(
        first_facts["collection"]["continuation"].is_string(),
        "the initial segment names its source continuation"
    );
    let first_records = first_facts["data"]["records"]
        .as_array()
        .expect("first records");
    assert_eq!(
        first_records.len(),
        native_first.len(),
        "first facts segment preserves the native page count"
    );
    assert_eq!(
        first_facts["collection"]["acceptedCount"],
        first_records.len()
    );
    assert_eq!(
        first_records
            .iter()
            .map(|record| record["id"].as_str().expect("fact comment id").to_owned())
            .collect::<Vec<_>>(),
        native_ids(native_first),
        "first facts segment preserves native comment identities"
    );
    let continuation = first
        .continuation()
        .expect("maxAttempts=2 leaves a source continuation")
        .to_owned();

    let second = read_with_limits(&source, &continuation, &limits).await;
    let second_facts: serde_json::Value =
        serde_json::from_str(second.content()).expect("continued collection JSON");
    assert_eq!(
        second_facts["kind"],
        "github.conversation_comment_collection"
    );
    assert_eq!(second_facts["request"]["number"], MULTIPAGE_PR.to_string());
    assert_eq!(second_facts["collection"]["scope"], "continuation");
    assert_eq!(second_facts["collection"]["state"], "complete");
    assert!(second.continuation().is_none());
    let second_records = second_facts["data"]["records"]
        .as_array()
        .expect("second records");
    assert_eq!(
        second_records.len(),
        native_second.len(),
        "continued facts segment preserves the native page count"
    );
    assert_eq!(
        second_records
            .iter()
            .map(|record| record["id"].as_str().expect("fact comment id").to_owned())
            .collect::<Vec<_>>(),
        native_ids(native_second),
        "continued facts segment preserves native comment identities"
    );

    for record in first_records.iter().chain(second_records) {
        assert_eq!(record["kind"], "github.conversation_comment");
        assert_eq!(record["parent"]["number"], MULTIPAGE_PR.to_string());
        assert!(
            record["links"]["issueUrl"]
                .as_str()
                .expect("issue url")
                .ends_with(&format!("/issues/{MULTIPAGE_PR}")),
            "every record names the verified parent"
        );
    }

    let id = native_first[0]["id"].as_u64().expect("native comment id");
    let single = read(
        &source,
        &format!("pr://{REPOSITORY}/{MULTIPAGE_PR}/comments/{id}/facts"),
    )
    .await;
    let single: serde_json::Value = serde_json::from_str(single.content()).expect("comment JSON");
    assert_eq!(single["data"]["id"], id.to_string());
    assert_eq!(single["data"]["nodeId"], native_first[0]["node_id"]);
    assert_eq!(single["data"]["body"], native_first[0]["body"]);
    assert_eq!(single["data"]["parent"]["number"], MULTIPAGE_PR.to_string());
    assert!(single.get("collection").is_none());
    eprintln!(
        "L7 conversation-comment facts: {} + {} records across source segments",
        first_records.len(),
        second_records.len()
    );
}
