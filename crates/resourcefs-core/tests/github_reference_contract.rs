use std::time::{Duration, Instant};

use resourcefs_core::{
    ConversationCommentId, DiffFileIndex, ErrorCategory, GithubRepositoryIdentity, IssueAddress,
    IssueNumber, IssueResource, PathReference, PullRequestAddress, PullRequestNumber,
    PullRequestResource, ResourceAddress, ReviewCommentId, ReviewId,
};

#[test]
fn canonical_github_reference_table() {
    let rows = [
        ("issue://", "issue://", "issue-root"),
        (
            "issue://Owner/Repo",
            "issue://owner/repo",
            "issue-collection",
        ),
        (
            "issue://owner/repo:page:11",
            "issue://owner/repo:page:11",
            "issue-page",
        ),
        (
            "issue://owner/repo/42",
            "issue://owner/repo/42",
            "issue-aggregate",
        ),
        (
            "issue://owner/repo/42/title",
            "issue://owner/repo/42/title",
            "issue-title",
        ),
        (
            "issue://owner/repo/42/body:2-4",
            "issue://owner/repo/42/body:2-4",
            "issue-body-lines",
        ),
        (
            "issue://owner/repo/42/comments",
            "issue://owner/repo/42/comments",
            "issue-comments",
        ),
        (
            "issue://owner/repo/42/comments/9001",
            "issue://owner/repo/42/comments/9001",
            "issue-comment",
        ),
        ("pr://", "pr://", "pr-root"),
        ("pr://Owner/Repo", "pr://owner/repo", "pr-collection"),
        ("pr://owner/repo/7", "pr://owner/repo/7", "pr-aggregate"),
        (
            "pr://owner/repo/7/comments/10",
            "pr://owner/repo/7/comments/10",
            "pr-comment",
        ),
        (
            "pr://owner/repo/7/reviews/20",
            "pr://owner/repo/7/reviews/20",
            "pr-review",
        ),
        (
            "pr://owner/repo/7/review-comments/30",
            "pr://owner/repo/7/review-comments/30",
            "pr-review-comment",
        ),
        (
            "pr://owner/repo/7/diff",
            "pr://owner/repo/7/diff",
            "pr-diff",
        ),
        (
            "pr://owner/repo/7/diff/4:raw",
            "pr://owner/repo/7/diff/4:raw",
            "pr-diff-file-raw",
        ),
    ];

    for (input, canonical, label) in rows {
        let parsed = PathReference::parse(input)
            .unwrap_or_else(|error| panic!("{label}: {input} failed: {error}"));
        assert_eq!(parsed.requested(), canonical, "{label}");
        assert_eq!(
            PathReference::parse(parsed.requested()),
            Ok(parsed.clone()),
            "{label} round trip"
        );
    }
}

#[test]
fn github_addresses_are_typed_once() {
    let issue = PathReference::parse("issue://owner/repo/42/comments/9001").expect("issue");
    let ResourceAddress::Issue(IssueAddress::Item {
        repository,
        number,
        resource: IssueResource::Comment(comment),
    }) = issue.address()
    else {
        panic!("typed issue comment")
    };
    assert_eq!(repository.owner(), "owner");
    assert_eq!(repository.repository(), "repo");
    assert_eq!(*number, IssueNumber::new(42).expect("number"));
    assert_eq!(
        *comment,
        ConversationCommentId::new(9_001).expect("comment")
    );

    let pull = PathReference::parse("pr://owner/repo/7/reviews/20").expect("pull");
    let ResourceAddress::PullRequest(PullRequestAddress::Item {
        repository,
        number,
        resource: PullRequestResource::Review(review),
    }) = pull.address()
    else {
        panic!("typed review")
    };
    assert_eq!(repository.as_str(), "owner/repo");
    assert_eq!(*number, PullRequestNumber::new(7).expect("number"));
    assert_eq!(*review, ReviewId::new(20).expect("review"));

    assert_eq!(ReviewCommentId::new(30).expect("review comment").get(), 30);
    assert_eq!(DiffFileIndex::new(4).expect("diff index").get(), 4);
}

#[test]
fn github_reference_errors_are_precise() {
    for input in [
        "issue://owner",
        "issue://owner/repo/0",
        "issue://owner/repo/not-a-number",
        "issue://owner/repo/1/unknown",
        "issue://owner/repo/1/comments/0",
        "issue://owner/repo/1/comments/2/extra",
        "pr://owner/repo/0",
        "pr://owner/repo/1/reviews/0",
        "pr://owner/repo/1/review-comments/nope",
        "pr://owner/repo/1/diff/0",
        "pr://owner/repo/1/diff/1/extra",
        "issue://owner/with%2Fslash/1",
        "issue://owner/repo?query/1",
        "pr://owner/repo#fragment/1",
        "issue://øwner/repo/1",
    ] {
        let error = PathReference::parse(input).expect_err("invalid GitHub reference");
        assert_eq!(error.category(), ErrorCategory::InvalidReference, "{input}");
    }

    for value in [0, u64::MAX] {
        if value == 0 {
            assert_eq!(
                IssueNumber::new(value).expect_err("zero issue").category(),
                ErrorCategory::InvalidReference
            );
        } else {
            assert_eq!(IssueNumber::new(value).expect("max issue").get(), value);
        }
    }
}

#[test]
fn repository_identity_is_case_canonical_and_validated() {
    let identity = GithubRepositoryIdentity::parse("Owner/Repo.Name-1").expect("repository");
    assert_eq!(identity.owner(), "owner");
    assert_eq!(identity.repository(), "repo.name-1");
    assert_eq!(identity.as_str(), "owner/repo.name-1");

    for invalid in [
        "",
        "owner",
        "/repo",
        "owner/",
        "owner/repo/extra",
        "./repo",
        "../repo",
        "owner/..",
        "owner/re po",
        "øwner/repo",
    ] {
        assert_eq!(
            GithubRepositoryIdentity::parse(invalid)
                .expect_err("invalid repository")
                .category(),
            ErrorCategory::InvalidReference,
            "{invalid}"
        );
    }
}

#[test]
fn typed_github_constructors_and_accessors_round_trip() {
    let repository = GithubRepositoryIdentity::new("Owner", "Repo").expect("repository");
    let issue_address = IssueAddress::Collection {
        repository: repository.clone(),
    };
    assert_eq!(
        issue_address
            .repository()
            .map(GithubRepositoryIdentity::as_str),
        Some("owner/repo")
    );
    assert_eq!(issue_address.number(), None);
    assert_eq!(issue_address.resource(), None);
    let page = resourcefs_core::ProjectionSelector::parse("page:2").expect("page");
    let issue = PathReference::issue(issue_address, Some(page)).expect("issue constructor");
    assert_eq!(issue.requested(), "issue://owner/repo:page:2");

    let pull_address = PullRequestAddress::Item {
        repository,
        number: PullRequestNumber::new(7).expect("number"),
        resource: PullRequestResource::ReviewComment(
            ReviewCommentId::new(30).expect("review comment"),
        ),
    };
    assert_eq!(
        pull_address
            .repository()
            .map(GithubRepositoryIdentity::as_str),
        Some("owner/repo")
    );
    assert_eq!(pull_address.number().map(PullRequestNumber::get), Some(7));
    assert!(matches!(
        pull_address.resource(),
        Some(PullRequestResource::ReviewComment(id)) if id.get() == 30
    ));
    let pull = PathReference::pull_request(pull_address, None).expect("pull constructor");
    assert_eq!(pull.requested(), "pr://owner/repo/7/review-comments/30");
}

#[test]
#[ignore = "checkpointed-build production-scale budget"]
fn reference_parse_budget() {
    let owner = "o".repeat(39);
    let repository = "r".repeat(100);
    let input = format!(
        "pr://{owner}/{repository}/18446744073709551615/review-comments/18446744073709551615"
    );
    let iterations = 10_000;
    let started = Instant::now();
    for _ in 0..iterations {
        let parsed = PathReference::parse(&input).expect("maximum valid reference");
        assert_eq!(parsed.requested(), input);
    }
    let average = started.elapsed() / iterations;
    assert!(
        average <= Duration::from_millis(1),
        "average maximum GitHub reference parse took {average:?}"
    );
}
