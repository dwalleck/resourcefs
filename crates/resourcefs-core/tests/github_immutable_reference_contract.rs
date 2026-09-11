use resourcefs_core::{
    GithubAddress, GithubCommitId, GithubRepositoryIdentity, GithubSourcePath,
    MAX_PATH_REFERENCE_BYTES, PathReference, ResourceAddress,
};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn immutable_source_references_keep_encoded_path_identity() {
    for path in [
        "facts",
        "nested/facts",
        "caf%C3%A9/%E6%97%A5%E6%9C%AC%E8%AA%9E.txt",
        "a%20b/%25%3A%3F%23",
        "%252F",
        "back%5Cslash",
        "line%0Aname",
    ] {
        let canonical = format!("github://owner/repo/source/{COMMIT}/{path}/facts");
        let reference = PathReference::parse(canonical.clone())
            .expect("immutable source syntax must be addressable");
        assert_eq!(reference.requested(), canonical);
        assert!(reference.projection().is_none());
    }
}

#[test]
fn immutable_addresses_expose_typed_commit_and_decoded_source_identity() {
    let commit = PathReference::parse(format!("github://Owner/Repo/commits/{COMMIT}/facts"))
        .expect("immutable commit syntax");
    let ResourceAddress::Github(GithubAddress::Commit {
        repository,
        commit: parsed_commit,
    }) = commit.address()
    else {
        panic!("expected typed Github commit address");
    };
    assert_eq!(repository.as_str(), "owner/repo");
    assert_eq!(parsed_commit.as_str(), COMMIT);
    assert_eq!(
        commit.requested(),
        format!("github://owner/repo/commits/{COMMIT}/facts")
    );

    let source = PathReference::parse(format!(
        "github://Owner/Repo/source/{COMMIT}/caf%C3%A9/%252F/line%0Aname/facts"
    ))
    .expect("immutable source syntax");
    let ResourceAddress::Github(GithubAddress::Source {
        repository,
        commit: parsed_commit,
        path,
    }) = source.address()
    else {
        panic!("expected typed Github source address");
    };
    assert_eq!(repository.as_str(), "owner/repo");
    assert_eq!(parsed_commit.as_str(), COMMIT);
    assert_eq!(
        path.segments()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["café", "%2F", "line\nname"]
    );
    assert_eq!(path.as_str(), "caf%C3%A9/%252F/line%0Aname");
    assert_eq!(
        source.requested(),
        format!("github://owner/repo/source/{COMMIT}/caf%C3%A9/%252F/line%0Aname/facts")
    );
}

#[test]
fn immutable_segment_canonicalization_is_single_pass_and_uppercase() {
    let reference = PathReference::parse(format!(
        "github://owner/repo/source/{COMMIT}/%7e/%41/%3a/facts"
    ))
    .expect("valid noncanonical escapes");
    assert_eq!(
        reference.requested(),
        format!("github://owner/repo/source/{COMMIT}/~/A/%3A/facts")
    );
    let ResourceAddress::Github(GithubAddress::Source { path, .. }) = reference.address() else {
        panic!("expected source address");
    };
    assert_eq!(
        path.segments()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["~", "A", ":"]
    );
}

#[test]
fn immutable_grammar_rejects_aliases_and_unrepresentable_operands() {
    for input in [
        "github://owner/repo/commits/0123456789abcdef0123456789abcdef0123456/facts",
        "github://owner/repo/commits/0123456789ABCDEF0123456789abcdef01234567/facts",
        "github://owner/repo/commits/main/facts",
        "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/facts",
        "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/./facts",
        "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/../facts",
        "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/%2E%2E/facts",
        "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/%2F/facts",
        "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/%00/facts",
        "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/%GG/facts",
        "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/%C3/facts",
        "github://owner/repo/source/0123456789abcdef0123456789abcdef01234567/colon:name/facts",
    ] {
        assert!(PathReference::parse(input).is_err(), "must reject {input}");
    }
}

#[test]
fn immutable_segment_constructors_preserve_git_filename_bytes() {
    let path = GithubSourcePath::new(vec!["back\\slash".into(), "line\nname".into()])
        .expect("decoded Git segments");
    assert_eq!(
        path.segments()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["back\\slash", "line\nname"]
    );
    assert_eq!(path.as_str(), "back%5Cslash/line%0Aname");
    assert_eq!(
        GithubSourcePath::parse(path.as_str()).expect("encoded path"),
        path
    );

    assert!(GithubSourcePath::new(Vec::new()).is_err());
    assert!(GithubSourcePath::new(vec![".".into()]).is_err());
    assert!(GithubSourcePath::new(vec!["a/b".into()]).is_err());
    assert!(GithubCommitId::new("0123456789abcdef0123456789abcdef0123456").is_err());
}

#[test]
fn typed_immutable_addresses_round_trip_through_the_public_constructor() {
    let path = GithubSourcePath::new(vec!["a b".into(), "%2F".into()]).expect("decoded segments");
    let address = GithubAddress::Source {
        repository: GithubRepositoryIdentity::new("Owner", "Repo").expect("repository"),
        commit: GithubCommitId::new(COMMIT).expect("commit id"),
        path,
    };
    let reference = PathReference::github(address.clone(), None).expect("typed address");
    assert_eq!(reference.requested(), address.canonical_reference());
    assert_eq!(
        reference.requested(),
        format!("github://owner/repo/source/{COMMIT}/a%20b/%252F/facts")
    );
    assert!(reference.projection().is_none());
}

#[test]
fn typed_source_path_ceiling_is_exact_for_the_shortest_reference() {
    // The shortest admissible repository plus the fixed framing is the least a
    // canonical reference can add around a path, so a path above that bound
    // could not become a Path Reference for any repository at all. Deriving the
    // bound from the shortest identity is also what keeps the reference
    // ceiling itself inclusive at exactly 64 KiB.
    let repository = GithubRepositoryIdentity::new("o", "r").expect("short identity");
    let shortest = format!("github://{}/source/{COMMIT}/", repository.as_str());
    let exact_path_bytes = MAX_PATH_REFERENCE_BYTES - shortest.len() - "/facts".len();

    let exact = GithubSourcePath::new(vec!["a".repeat(exact_path_bytes)])
        .expect("the shortest reference's ceiling is inclusive");
    assert_eq!(exact.as_str().len(), exact_path_bytes);
    let reference = PathReference::github(
        GithubAddress::Source {
            repository,
            commit: GithubCommitId::new(COMMIT).expect("commit id"),
            path: exact,
        },
        None,
    )
    .expect("an accepted path addresses a reference");
    assert_eq!(reference.requested().len(), MAX_PATH_REFERENCE_BYTES);

    let over = GithubSourcePath::new(vec!["a".repeat(exact_path_bytes + 1)])
        .expect_err("a typed path that cannot fit any Path Reference must not exist");
    assert_eq!(
        over.category(),
        resourcefs_core::ErrorCategory::LimitExceeded
    );
}

#[test]
fn whole_reference_ceiling_survives_a_path_some_repository_can_address() {
    // A path can be addressable by a short repository and still overflow the
    // 64 KiB reference ceiling for a long one. The type promises the former and
    // the reference boundary decides the latter, so the answer is a typed
    // limit failure rather than a minted reference the parser would refuse.
    let path = GithubSourcePath::new(vec!["a".repeat(65_400)]).expect("addressable path");
    let error = PathReference::github(
        GithubAddress::Source {
            repository: GithubRepositoryIdentity::new("o".repeat(39), "r".repeat(100))
                .expect("longest identity"),
            commit: GithubCommitId::new(COMMIT).expect("commit id"),
            path,
        },
        None,
    )
    .expect_err("the whole reference exceeds 64 KiB");
    assert_eq!(
        error.category(),
        resourcefs_core::ErrorCategory::LimitExceeded
    );
}

#[test]
fn facts_documents_admit_a_selector_the_read_path_refuses() {
    // Singular Facts documents take no selector, so a trailing `:raw`,
    // line range or `:page:` is a projection the reader refuses rather than a
    // second identity — the shape `pr://…/facts:raw` already has
    // (`crates/resourcefs-sources/src/github/facts.rs`). The grammar admits it
    // so the refusal belongs to the read path; a canonical identity still
    // requires an unprojected reference.
    for input in [
        format!("github://owner/repo/commits/{COMMIT}/facts:raw"),
        format!("github://owner/repo/source/{COMMIT}/a.rs/facts:12-20"),
        format!("github://owner/repo/commits/{COMMIT}/facts:page:2"),
    ] {
        let reference = PathReference::parse(input.clone()).expect("selector parses");
        assert_eq!(reference.requested(), input);
        assert!(reference.projection().is_some());
    }
}

#[test]
fn immutable_reference_retains_existing_64k_boundary() {
    let prefix = format!("github://owner/repo/source/{COMMIT}/");
    let suffix = "/facts";
    let exact_path_bytes = MAX_PATH_REFERENCE_BYTES - prefix.len() - suffix.len();
    let exact = format!("{prefix}{}{suffix}", "a".repeat(exact_path_bytes));
    assert_eq!(exact.len(), MAX_PATH_REFERENCE_BYTES);
    assert_eq!(
        PathReference::parse(exact.clone())
            .expect("exact boundary")
            .requested(),
        exact
    );

    let over = format!("{prefix}{}{suffix}", "a".repeat(exact_path_bytes + 1));
    assert_eq!(over.len(), MAX_PATH_REFERENCE_BYTES + 1);
    assert!(PathReference::parse(over).is_err());
}
