//! Live read-only smoke over the real GitHub API (rfs-45ww).
//!
//! The permanent adapter contracts run against a deterministic fake upstream,
//! which can only be as faithful as the fixture author's model of GitHub — the
//! `/repositories/{id}` Link form (review finding F1) was captured in
//! `.rfs-45ww/evidence.md` and still missed the fixture. This suite exists to
//! catch that class of drift: every row targets a shape the fake cannot be
//! trusted to imitate, against public `rust-lang/rust` data, GET only.
//!
//! Ignored by default and skipped without a token; run explicitly with
//! `GITHUB_TOKEN="$(gh auth token)" cargo test -p resourcefs-sources --test
//! github_live_smoke -- --ignored --nocapture`.
#[path = "support/mod.rs"]
mod session_support;

use std::sync::Arc;

use resourcefs_core::{
    AllowedOrigin, DiscoveryEngine, ErrorCategory, HttpCeilings, OperationGuard, OriginAllowlist,
    PathReference, SearchLimits, SearchOptions, SearchRequest, SearchTarget, Secret, ServerLimits,
    SourceAdapter, SourceResource,
};
use resourcefs_sources::{
    GithubConfig, GithubRepository, GithubSource, GithubSourceMount, HttpSubstrate, MutationGrants,
    OriginCredential, SecretReference,
};

const REPOSITORY: &str = "rust-lang/rust";
/// Evidence P1–P6 fixture: 7 conversation comments, 6 inline comments, 4 files.
const SMALL_PR: u64 = 159_232;
/// 56 changed files at the time of writing — past the endpoint's 30-row default.
const WIDE_PR: u64 = 161_878;

async fn live_source(token: String) -> Option<GithubSource> {
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
    Some(
        GithubSourceMount::new(config, Arc::new(substrate))
            .bind(session.path_session().clone())
            .expect("GitHub source"),
    )
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
#[ignore = "live GitHub smoke; needs GITHUB_TOKEN"]
async fn live_github_reads_hold_up() {
    let Ok(token) = std::env::var("GITHUB_TOKEN") else {
        eprintln!("GITHUB_TOKEN is not set; skipping the live smoke");
        return;
    };
    let source = live_source(token).await.expect("live source");

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
