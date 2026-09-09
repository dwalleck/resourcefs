use std::time::{Duration, Instant};

use resourcefs_core::{
    ConversationCommentId, DiffFileIndex, ErrorCategory, GithubRepositoryIdentity, IssueAddress,
    IssueNumber, IssueResource, MAX_PATH_REFERENCE_BYTES, PathReference, PullRequestAddress,
    PullRequestFact, PullRequestNumber, PullRequestResource, ResourceAddress, ReviewCommentId,
    ReviewId,
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
fn creation_targets_round_trip_and_are_write_only() {
    let rows = [
        ("issue://Owner/Repo/new", "issue://owner/repo/new"),
        (
            "issue://Owner/Repo/42/comments/new",
            "issue://owner/repo/42/comments/new",
        ),
        ("pr://Owner/Repo/new", "pr://owner/repo/new"),
        (
            "pr://Owner/Repo/7/comments/new",
            "pr://owner/repo/7/comments/new",
        ),
        ("issue://owner/repo/new:raw", "issue://owner/repo/new:raw"),
        (
            "issue://owner/repo/18446744073709551615/comments/new:1",
            "issue://owner/repo/18446744073709551615/comments/new:1",
        ),
        ("pr://owner/repo/new:raw", "pr://owner/repo/new:raw"),
        (
            "pr://owner/repo/18446744073709551615/comments/new:1",
            "pr://owner/repo/18446744073709551615/comments/new:1",
        ),
    ];
    for (input, canonical) in rows {
        let parsed = PathReference::parse(input)
            .unwrap_or_else(|error| panic!("[C2] {input} failed: {error}"));
        assert!(parsed.is_creation_target(), "[C2] {input} target kind");
        assert_eq!(parsed.requested(), canonical, "[C2] {input}");
        assert_eq!(
            PathReference::parse(parsed.requested()),
            Ok(parsed),
            "[C2] {input} round trip"
        );
    }
    assert!(
        !PathReference::parse("issue://owner/repo/1/title")
            .expect("[C2] ordinary Field")
            .is_creation_target(),
        "[C2] ordinary Field is not a Creation Target"
    );

    let issue = PathReference::parse("issue://owner/repo/new").expect("[C2] issue target");
    assert!(matches!(
        issue.address(),
        ResourceAddress::Issue(IssueAddress::New { repository })
            if repository.as_str() == "owner/repo"
    ));
    let issue_comment =
        PathReference::parse("issue://owner/repo/42/comments/new").expect("[C2] issue comment");
    assert!(matches!(
        issue_comment.address(),
        ResourceAddress::Issue(IssueAddress::Item {
            resource: IssueResource::CommentsNew,
            ..
        })
    ));

    let pull = PathReference::parse("pr://owner/repo/new").expect("[C2] pull target");
    assert!(matches!(
        pull.address(),
        ResourceAddress::PullRequest(PullRequestAddress::New { repository })
            if repository.as_str() == "owner/repo"
    ));
    let pull_comment =
        PathReference::parse("pr://owner/repo/7/comments/new").expect("[C2] pull comment");
    assert!(matches!(
        pull_comment.address(),
        ResourceAddress::PullRequest(PullRequestAddress::Item {
            resource: PullRequestResource::CommentsNew,
            ..
        })
    ));

    for input in [
        "issue://owner/repo/new/extra",
        "issue://owner/repo/1/comments/new/extra",
        "issue://owner/repo/-1/comments/new",
        "issue://owner/repo/18446744073709551616/comments/new",
        "pr://owner/repo/-1/comments/new",
        "pr://owner/repo/18446744073709551616/comments/new",
        "pr://owner/repo/new/extra",
        "pr://owner/repo/1/comments/new/extra",
    ] {
        let error = PathReference::parse(input).expect_err("[C2] malformed target");
        assert_eq!(
            error.category(),
            ErrorCategory::InvalidReference,
            "[C2] {input}"
        );
    }
    let prefix = "issue://owner/";
    let boundary = format!(
        "{prefix}{}",
        "r".repeat(MAX_PATH_REFERENCE_BYTES - prefix.len())
    );
    assert_eq!(boundary.len(), MAX_PATH_REFERENCE_BYTES, "[C2] boundary");
    assert_eq!(
        PathReference::parse(&boundary)
            .expect_err("[C2] overlong repository component")
            .category(),
        ErrorCategory::InvalidReference,
        "[C2] exact Path Reference ceiling remains component-bounded"
    );
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
    let input = format!("pr://{owner}/{repository}/18446744073709551615/comments/new:1");
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

#[test]
fn facts_canonical_grammar_does_not_reinterpret_legacy_projections() {
    for input in ["pr://Owner/Repo/7/facts", "pr://owner/repo/0007/facts"] {
        assert_eq!(
            PathReference::parse(input).expect("facts").requested(),
            "pr://owner/repo/7/facts"
        );
    }
    for input in [
        "pr://owner/repo/0/facts",
        "pr://owner/repo/18446744073709551616/facts",
        "pr://owner/repo/7/facts/1",
        "issue://owner/repo/7/facts",
        "pr://owner/repo/new/facts",
    ] {
        assert!(PathReference::parse(input).is_err(), "{input}");
    }
    for input in [
        "pr://owner/repo/7/diff",
        "pr://owner/repo/7/diff/1",
        "pr://owner/repo/7/body",
    ] {
        assert_eq!(
            PathReference::parse(input).expect("legacy").requested(),
            input
        );
    }
}

#[test]
fn conversation_comment_facts_routes_and_cursor_scope() {
    // Canonical spellings round-trip exactly.
    for (input, canonical) in [
        (
            "pr://Owner/Repo/0007/comments/facts",
            "pr://owner/repo/7/comments/facts",
        ),
        (
            "pr://owner/repo/7/comments/9001/facts",
            "pr://owner/repo/7/comments/9001/facts",
        ),
    ] {
        let parsed =
            PathReference::parse(input).unwrap_or_else(|error| panic!("{input} failed: {error}"));
        assert_eq!(parsed.requested(), canonical, "{input}");
        assert_eq!(
            PathReference::parse(parsed.requested()),
            Ok(parsed.clone()),
            "{input} round trip"
        );
    }
    assert!(matches!(
        PathReference::parse("pr://owner/repo/7/comments/facts")
            .expect("collection")
            .address(),
        ResourceAddress::PullRequest(PullRequestAddress::Item {
            resource: PullRequestResource::Facts(resourcefs_core::PullRequestFact::Comments),
            ..
        })
    ));
    assert!(matches!(
        PathReference::parse("pr://owner/repo/7/comments/9001/facts")
            .expect("item")
            .address(),
        ResourceAddress::PullRequest(PullRequestAddress::Item {
            resource: PullRequestResource::Facts(resourcefs_core::PullRequestFact::Comment(id)),
            ..
        }) if id.get() == 9_001
    ));

    // Malformed or reinterpreted spellings stay refusals.
    for input in [
        "pr://owner/repo/7/comments/facts/1",
        "pr://owner/repo/7/comments/facts/facts",
        "pr://owner/repo/7/comments/new/facts",
        "pr://owner/repo/7/comments/0/facts",
        "pr://owner/repo/7/comments/9001/facts/1",
        "pr://owner/repo/0/comments/facts",
        "issue://owner/repo/7/comments/facts",
    ] {
        assert_eq!(
            PathReference::parse(input)
                .expect_err("refused conversation-comment facts spelling")
                .category(),
            ErrorCategory::InvalidReference,
            "{input}"
        );
    }

    // The cursor belongs to the collection and nowhere else; every other
    // pr:// route keeps the verdict it had before the split existed.
    let collection = PathReference::parse("pr://owner/repo/7/comments/facts:cursor:e30")
        .expect("collection cursor");
    assert_eq!(
        collection
            .projection()
            .and_then(resourcefs_core::ProjectionSelector::source_cursor)
            .map(resourcefs_core::SourceCursor::as_str),
        Some("e30")
    );
    for input in [
        "pr://owner/repo/7/facts:cursor:e30",
        "pr://owner/repo/7/comments/9001/facts:cursor:e30",
        "pr://owner/repo:cursor:e30",
        "pr://owner/repo/7/comments:cursor:e30",
        "pr://owner/repo/7/reviews:cursor:e30",
    ] {
        assert_eq!(
            PathReference::parse(input)
                .expect_err("cursor outside the conversation-comment collection")
                .category(),
            ErrorCategory::InvalidReference,
            "{input}"
        );
    }

    // Native page and line selectors keep their existing parse behaviour; the
    // adapter refuses them at read time.
    for input in [
        "pr://owner/repo/7/comments/facts:page:2",
        "pr://owner/repo/7/comments/facts:raw",
        "pr://owner/repo/7/comments/facts:2-4",
    ] {
        assert_eq!(
            PathReference::parse(input)
                .expect("parsed selector")
                .requested(),
            input,
            "{input}"
        );
    }
}

#[test]
fn review_and_inline_facts_routes_and_cursor_scope() {
    // Canonical spellings round-trip exactly, including the collection/item
    // split that shares a prefix with the human review routes.
    for (input, canonical) in [
        (
            "pr://Owner/Repo/0007/reviews/facts",
            "pr://owner/repo/7/reviews/facts",
        ),
        (
            "pr://owner/repo/7/reviews/9001/facts",
            "pr://owner/repo/7/reviews/9001/facts",
        ),
        (
            "pr://Owner/Repo/0007/review-comments/facts",
            "pr://owner/repo/7/review-comments/facts",
        ),
        (
            "pr://owner/repo/7/review-comments/9001/facts",
            "pr://owner/repo/7/review-comments/9001/facts",
        ),
    ] {
        let parsed =
            PathReference::parse(input).unwrap_or_else(|error| panic!("{input} failed: {error}"));
        assert_eq!(parsed.requested(), canonical, "{input}");
        assert_eq!(
            PathReference::parse(parsed.requested()),
            Ok(parsed.clone()),
            "{input} round trip"
        );
    }
    assert!(matches!(
        PathReference::parse("pr://owner/repo/7/reviews/facts")
            .expect("review collection")
            .address(),
        ResourceAddress::PullRequest(PullRequestAddress::Item {
            resource: PullRequestResource::Facts(PullRequestFact::Reviews),
            ..
        })
    ));
    assert!(matches!(
        PathReference::parse("pr://owner/repo/7/reviews/9001/facts")
            .expect("review item")
            .address(),
        ResourceAddress::PullRequest(PullRequestAddress::Item {
            resource: PullRequestResource::Facts(PullRequestFact::Review(id)),
            ..
        }) if id.get() == 9_001
    ));
    assert!(matches!(
        PathReference::parse("pr://owner/repo/7/review-comments/facts")
            .expect("inline collection")
            .address(),
        ResourceAddress::PullRequest(PullRequestAddress::Item {
            resource: PullRequestResource::Facts(PullRequestFact::ReviewComments),
            ..
        })
    ));
    assert!(matches!(
        PathReference::parse("pr://owner/repo/7/review-comments/9001/facts")
            .expect("inline item")
            .address(),
        ResourceAddress::PullRequest(PullRequestAddress::Item {
            resource: PullRequestResource::Facts(PullRequestFact::ReviewComment(id)),
            ..
        }) if id.get() == 9_001
    ));

    // Malformed, non-positive and reinterpreted spellings stay refusals.
    for input in [
        "pr://owner/repo/7/reviews/facts/1",
        "pr://owner/repo/7/reviews/facts/facts",
        "pr://owner/repo/7/reviews/0/facts",
        "pr://owner/repo/7/reviews/9001/facts/1",
        "pr://owner/repo/7/review-comments/facts/1",
        "pr://owner/repo/7/review-comments/0/facts",
        "pr://owner/repo/7/review-comments/9001/facts/1",
        "pr://owner/repo/0/reviews/facts",
        "pr://owner/repo/0/review-comments/facts",
        "issue://owner/repo/7/reviews/facts",
        "issue://owner/repo/7/review-comments/facts",
    ] {
        assert_eq!(
            PathReference::parse(input)
                .expect_err("refused review/inline facts spelling")
                .category(),
            ErrorCategory::InvalidReference,
            "{input}"
        );
    }

    // The human review routes keep their meaning and never become facts.
    for (input, expected) in [
        ("pr://owner/repo/7/reviews", PullRequestResource::Reviews),
        (
            "pr://owner/repo/7/reviews/9001",
            PullRequestResource::Review(ReviewId::new(9_001).expect("review id")),
        ),
        (
            "pr://owner/repo/7/review-comments",
            PullRequestResource::ReviewComments,
        ),
        (
            "pr://owner/repo/7/review-comments/9001",
            PullRequestResource::ReviewComment(ReviewCommentId::new(9_001).expect("comment id")),
        ),
    ] {
        let parsed = PathReference::parse(input).expect("human review route");
        assert_eq!(parsed.requested(), input, "{input}");
        let ResourceAddress::PullRequest(address) = parsed.address() else {
            panic!("{input} is not a pull request address");
        };
        assert_eq!(address.resource(), Some(expected), "{input} human meaning");
    }

    // A source cursor belongs to the three discussion collections and to no
    // item or human route.
    for input in [
        "pr://owner/repo/7/reviews/facts:cursor:e30",
        "pr://owner/repo/7/review-comments/facts:cursor:e30",
    ] {
        let parsed = PathReference::parse(input).expect("collection cursor");
        assert_eq!(
            parsed
                .projection()
                .and_then(resourcefs_core::ProjectionSelector::source_cursor)
                .map(resourcefs_core::SourceCursor::as_str),
            Some("e30"),
            "{input}"
        );
    }
    for input in [
        "pr://owner/repo/7/reviews/9001/facts:cursor:e30",
        "pr://owner/repo/7/review-comments/9001/facts:cursor:e30",
        "pr://owner/repo/7/reviews:cursor:e30",
        "pr://owner/repo/7/review-comments:cursor:e30",
        "pr://owner/repo/7/reviews/9001:cursor:e30",
        "pr://owner/repo/7/review-comments/9001:cursor:e30",
    ] {
        assert_eq!(
            PathReference::parse(input)
                .expect_err("cursor outside a discussion collection")
                .category(),
            ErrorCategory::InvalidReference,
            "{input}"
        );
    }
}
