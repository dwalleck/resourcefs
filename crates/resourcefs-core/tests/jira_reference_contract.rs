use std::{
    io::Cursor,
    time::{Duration, Instant},
};

use resourcefs_core::{
    AtlassianSiteId, ErrorCategory, JiraAddress, JiraFieldId, JiraIssueId, JiraIssueKey,
    JiraIssueResource, JiraProjectId, JiraProjectKey, MAX_ATLASSIAN_SITE_ID_BYTES,
    MAX_JIRA_ISSUE_ID_BYTES, MAX_JIRA_PROJECT_ID_BYTES, MAX_JIRA_SEGMENT_BYTES,
    MAX_PATH_REFERENCE_BYTES, PathReference, ProjectionSelector, ResourceAddress, SearchRecord,
    SourceOffset, SourceResource, Utf8ContentType, select_utf8,
};

#[test]
fn canonical_jira_references_round_trip() {
    let rows = [
        (
            "jira://acme/issues/10001",
            "jira://acme/issues/10001",
            "stable issue",
        ),
        (
            "jira://acme/issue-keys/TEAM-7",
            "jira://acme/issue-keys/TEAM-7",
            "issue-key alias",
        ),
        (
            "jira://acme/issue-keys/TEAM%207",
            "jira://acme/issue-keys/TEAM%207",
            "encoded issue-key alias",
        ),
        (
            "jira://acme/issues/10001/fields",
            "jira://acme/issues/10001/fields",
            "Field index",
        ),
        (
            "jira://acme/issues/10001/fields/customfield_10000",
            "jira://acme/issues/10001/fields/customfield_10000",
            "ordinary Field",
        ),
        (
            "jira://acme/issues/10001/fields/caf%C3%A9:raw",
            "jira://acme/issues/10001/fields/caf%C3%A9:raw",
            "Unicode Field with selector",
        ),
        (
            "jira://acme/issues/10001:2-4",
            "jira://acme/issues/10001:2-4",
            "Aggregate lines",
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
fn jira_addresses_are_typed_once() {
    let issue = PathReference::parse("jira://acme/issues/10001").expect("stable issue");
    let ResourceAddress::Jira(JiraAddress::Issue {
        site,
        issue_id,
        resource: JiraIssueResource::Aggregate,
    }) = issue.address()
    else {
        panic!("typed stable issue")
    };
    assert_eq!(site.as_str(), "acme");
    assert_eq!(issue_id.as_str(), "10001");

    let alias = PathReference::parse("jira://acme/issue-keys/TEAM%207").expect("alias");
    let ResourceAddress::Jira(JiraAddress::IssueKeyAlias { site, issue_key }) = alias.address()
    else {
        panic!("typed issue-key alias")
    };
    assert_eq!(site.as_str(), "acme");
    assert_eq!(issue_key.as_str(), "TEAM 7");

    let field = PathReference::parse("jira://acme/issues/10001/fields/caf%C3%A9").expect("Field");
    let ResourceAddress::Jira(JiraAddress::Issue {
        resource: JiraIssueResource::Field(field_id),
        ..
    }) = field.address()
    else {
        panic!("typed Field")
    };
    assert_eq!(field_id.as_str(), "café");
}

#[test]
fn identifier_constructors_enforce_domain_boundaries() {
    assert_eq!(
        AtlassianSiteId::new("a").expect("minimum Site ID").as_str(),
        "a"
    );
    assert!(AtlassianSiteId::new("a-b9").is_ok());
    assert!(AtlassianSiteId::new("a".repeat(MAX_ATLASSIAN_SITE_ID_BYTES)).is_ok());
    for invalid in ["", "-a", "a-", "A", "a_b", "é", "a..b"] {
        assert!(
            AtlassianSiteId::new(invalid).is_err(),
            "Site ID {invalid:?}"
        );
    }
    assert!(AtlassianSiteId::new("a".repeat(MAX_ATLASSIAN_SITE_ID_BYTES + 1)).is_err());

    assert_eq!(
        JiraIssueId::new("1").expect("minimum issue ID").as_str(),
        "1"
    );
    assert!(JiraIssueId::new("9".repeat(MAX_JIRA_ISSUE_ID_BYTES)).is_ok());
    for invalid in ["", "0", "01", "+1", "-1", " 1", "١"] {
        assert!(JiraIssueId::new(invalid).is_err(), "issue ID {invalid:?}");
    }
    assert!(JiraIssueId::new("9".repeat(MAX_JIRA_ISSUE_ID_BYTES + 1)).is_err());

    assert_eq!(
        JiraIssueKey::new("TEAM 7").expect("issue key").as_str(),
        "TEAM 7"
    );
    assert_eq!(JiraFieldId::new("café").expect("Field ID").as_str(), "café");
    assert!(JiraIssueKey::new("x".repeat(MAX_JIRA_SEGMENT_BYTES)).is_ok());
    assert!(JiraFieldId::new("x".repeat(MAX_JIRA_SEGMENT_BYTES)).is_ok());
    for invalid in ["", ".", "..", "new", "a/b", "a\\b", "a\0b", "a\nb"] {
        assert!(JiraIssueKey::new(invalid).is_err(), "issue key {invalid:?}");
        assert!(JiraFieldId::new(invalid).is_err(), "Field ID {invalid:?}");
    }
    assert!(JiraIssueKey::new("x".repeat(MAX_JIRA_SEGMENT_BYTES + 1)).is_err());
    assert!(JiraFieldId::new("x".repeat(MAX_JIRA_SEGMENT_BYTES + 1)).is_err());
}

#[test]
fn invalid_and_dormant_jira_references_are_rejected() {
    for input in [
        "jira://",
        "jira://acme",
        "jira://acme/issues",
        "jira://acme/issues/0",
        "jira://acme/issues/01",
        "jira://acme/issues/-1",
        "jira://acme/issues/not-a-number",
        "jira://acme/issue-keys",
        "jira://acme/issue-keys/new",
        "jira://acme/issue-keys/TEAM 7",
        "jira://acme/issue-keys/TEAM%2d7",
        "jira://acme/issue-keys/TEAM%2F7",
        "jira://acme/issue-keys/TEAM%5C7",
        "jira://acme/issue-keys/TEAM%ZZ7",
        "jira://acme/issues/1/fields/new",
        "jira://acme/issues/1/fields/a b",
        "jira://acme/issues/1/fields/a%2fb",
        "jira://acme/issues/1/fields/a%5Cb",
        "jira://acme/issues/1/comments",
        "jira://acme/issues/1/fields/name/extra",
        "jira://ACME/issues/1",
        "jira://-acme/issues/1",
        "jira://acme-/issues/1",
    ] {
        let error = PathReference::parse(input).expect_err("invalid Jira reference");
        assert_eq!(error.category(), ErrorCategory::InvalidReference, "{input}");
    }
}

#[test]
fn rejects_noncanonical_segments() {
    for input in [
        "jira://acme/issue-keys/TEAM 7",
        "jira://acme/issue-keys/TEAM%2d7",
        "jira://acme/issue-keys/café",
        "jira://acme/issues/1/fields/a b",
        "jira://acme/issues/1/fields/a%2fb",
    ] {
        assert!(
            PathReference::parse(input).is_err(),
            "{input} must use one safe canonical percent-encoded spelling"
        );
    }
}

#[test]
fn jira_reference_global_ceiling_remains_authoritative() {
    let prefix = "jira://acme/issue-keys/";
    let at_ceiling = format!(
        "{prefix}{}",
        "x".repeat(MAX_PATH_REFERENCE_BYTES - prefix.len())
    );
    assert_eq!(at_ceiling.len(), MAX_PATH_REFERENCE_BYTES);
    assert_eq!(
        PathReference::parse(&at_ceiling)
            .expect_err("segment ceiling still applies")
            .category(),
        ErrorCategory::InvalidReference
    );

    let over_ceiling = format!("{at_ceiling}x");
    assert_eq!(
        PathReference::parse(over_ceiling)
            .expect_err("global ceiling")
            .category(),
        ErrorCategory::LimitExceeded
    );
}

#[test]
fn maximum_jira_reference_parse_stays_within_budget() {
    let reference = format!(
        "jira://{}/issues/{}/fields/{}",
        "s".repeat(MAX_ATLASSIAN_SITE_ID_BYTES),
        "9".repeat(MAX_JIRA_ISSUE_ID_BYTES),
        "x".repeat(MAX_JIRA_SEGMENT_BYTES)
    );
    let started = Instant::now();
    for _ in 0..1_000 {
        PathReference::parse(&reference).expect("maximum Jira reference");
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "1,000 maximum Jira references must average below the 2 ms budget"
    );
}

#[test]
fn browse_reference_identity_and_selector_families() {
    for input in [
        "jira://acme/projects",
        "jira://acme/projects:offset:1",
        "jira://acme/projects:offset:18446744073709551615",
        "jira://acme/projects:raw",
        "jira://acme/projects:2-4",
        "jira://acme/projects:raw:2-4",
        "jira://acme/projects/1",
        "jira://acme/projects/18446744073709551616",
        "jira://acme/projects/1:raw",
        "jira://acme/project-keys/TEAM",
        "jira://acme/project-keys/TEAM%207",
        "jira://acme/project-keys/caf%C3%A9",
        "jira://acme/project-keys/a%3Ab%3Fc%23d%25",
        "jira://acme/project-keys/TEAM:1-2",
    ] {
        let reference = PathReference::parse(input).expect(input);
        assert_eq!(reference.requested(), input);
        let ResourceAddress::Jira(address) = reference.address() else {
            panic!("typed Jira reference");
        };
        assert_eq!(address.site().as_str(), "acme");
        let reconstructed = PathReference::jira(address.clone(), reference.projection().cloned())
            .expect("typed round trip");
        assert_eq!(reconstructed, reference);
    }
    let collection = PathReference::parse("jira://acme/projects:offset:7").expect("offset");
    assert!(matches!(
        collection.address(),
        ResourceAddress::Jira(JiraAddress::Projects { .. })
    ));
    let selector = collection.projection().expect("source selection");
    assert_eq!(selector.source_offset().expect("typed offset").get(), 7);
    assert_eq!(selector.page_offset(), None);
    assert_eq!(selector.line_selection(), None);
    assert!(!selector.is_raw());
    assert_eq!(
        select_utf8(
            Cursor::new(b"must not become the whole page"),
            Some(selector)
        )
        .expect_err("adapter must consume source selector")
        .category(),
        ErrorCategory::UnsupportedProjection,
    );
    let project = PathReference::parse("jira://acme/projects/10").expect("project");
    let ResourceAddress::Jira(JiraAddress::Project { project_id, .. }) = project.address() else {
        panic!("typed project");
    };
    assert_eq!(project_id.as_str(), "10");
    let alias = PathReference::parse("jira://acme/project-keys/TEAM%207").expect("alias");
    let ResourceAddress::Jira(JiraAddress::ProjectKeyAlias { project_key, .. }) = alias.address()
    else {
        panic!("typed project-key alias");
    };
    assert_eq!(project_key.as_str(), "TEAM 7");
    for input in [
        "jira://acme/projects/",
        "jira://acme/projects/0",
        "jira://acme/projects/01",
        "jira://acme/projects/-1",
        "jira://acme/projects/+1",
        "jira://acme/projects/1.0",
        "jira://acme/projects/1/fields",
        "jira://acme/projects/1/issues",
        "jira://acme/issues",
        "jira://acme/project-keys",
        "jira://acme/project-keys/new",
        "jira://acme/project-keys/.",
        "jira://acme/project-keys/..",
        "jira://acme/project-keys/a/b",
        "jira://acme/project-keys/a%2Fb",
        "jira://acme/project-keys/a%5Cb",
        "jira://acme/project-keys/a%00b",
        "jira://acme/project-keys/a%0Ab",
        "jira://acme/project-keys/%FF",
        "jira://acme/project-keys/%41",
        "jira://acme/project-keys/caf%c3%a9",
        "jira://acme/project-keys/café",
        "jira://acme/project-keys/TEAM 7",
        "jira://acme/projects:offset:",
        "jira://acme/projects:offset:0",
        "jira://acme/projects:offset:01",
        "jira://acme/projects:offset:+1",
        "jira://acme/projects:offset:-1",
        "jira://acme/projects:offset:١",
        "jira://acme/projects:offset:18446744073709551616",
        "jira://acme/projects:offset:1:raw",
        "jira://acme/projects:raw:offset:1",
        "jira://acme/projects:offset:1:2-3",
        "jira://acme/projects:offset:1:offset:2",
        "jira://acme/projects:cursor:abc",
        "jira://acme/projects/1:offset:1",
        "jira://acme/project-keys/TEAM:offset:1",
        "jira://acme/issues/1:offset:1",
        "jira://acme/issues/1/fields:offset:1",
        "jira://acme/issues/1/fields/summary:offset:1",
        "jira://acme/issue-keys/TEAM-1:offset:1",
    ] {
        assert_eq!(
            PathReference::parse(input).expect_err(input).category(),
            ErrorCategory::InvalidReference
        );
    }
    for input in [
        "jira://acme/projects/1",
        "jira://acme/project-keys/TEAM",
        "jira://acme/issues/1",
    ] {
        let reference = PathReference::parse(input).expect("direct resource");
        let ResourceAddress::Jira(address) = reference.address() else {
            panic!("Jira")
        };
        assert!(PathReference::jira(address.clone(), Some(selector.clone())).is_err());
    }
}

#[test]
fn project_identifiers_and_offsets_keep_their_distinct_bounds() {
    for id in ["1".to_owned(), "9".repeat(MAX_JIRA_PROJECT_ID_BYTES)] {
        assert_eq!(
            JiraProjectId::new(id.clone()).expect("project ID").as_str(),
            id
        );
        PathReference::parse(format!("jira://acme/projects/{id}")).expect("bounded project ID");
    }
    for invalid in ["", "0", "01", "-1", "+1", " 1", "١"] {
        assert!(JiraProjectId::new(invalid).is_err());
    }
    let oversized = "9".repeat(MAX_JIRA_PROJECT_ID_BYTES + 1);
    assert!(JiraProjectId::new(oversized.clone()).is_err());
    assert!(PathReference::parse(format!("jira://acme/projects/{oversized}")).is_err());
    for key in ["café".to_owned(), "x".repeat(MAX_JIRA_SEGMENT_BYTES)] {
        assert_eq!(
            JiraProjectKey::new(key.clone())
                .expect("project key")
                .as_str(),
            key
        );
    }
    for invalid in ["", ".", "..", "new", "a/b", "a\\b", "a\0b", "a\nb"] {
        assert!(JiraProjectKey::new(invalid).is_err());
    }
    assert!(JiraProjectKey::new("x".repeat(MAX_JIRA_SEGMENT_BYTES + 1)).is_err());
    assert!(SourceOffset::new(0).is_err());
    assert_eq!(SourceOffset::new(1).expect("first continuation").get(), 1);
    assert_eq!(
        SourceOffset::new(u64::MAX).expect("largest offset").get(),
        u64::MAX
    );
    assert_eq!(
        ProjectionSelector::parse("page:1")
            .expect("legacy")
            .source_offset(),
        None
    );
}

#[test]
fn project_search_pages_and_resource_identity_remain_distinct() {
    let page = PathReference::parse("jira://acme/projects:offset:7").expect("page");
    let selected =
        SearchRecord::new(page.clone(), 1, "selected-page metadata").expect("page record");
    let first = SearchRecord::new(
        PathReference::parse("jira://acme/projects").expect("first page"),
        1,
        "selected-page metadata",
    )
    .expect("first-page record");
    assert_ne!(
        selected, first,
        "the selected native page must remain part of hit identity"
    );
    assert!(SourceResource::utf8(page, "metadata".to_owned(), Utf8ContentType::MARKDOWN).is_err());
    for input in ["jira://acme/projects", "jira://acme/projects/1"] {
        let reference = PathReference::parse(input).expect("canonical resource");
        assert!(SearchRecord::new(reference.clone(), 1, "metadata").is_ok());
        assert_eq!(
            SourceResource::utf8(reference, "metadata".to_owned(), Utf8ContentType::MARKDOWN)
                .expect("unselected resource")
                .canonical_reference(),
            input,
        );
    }
    for input in [
        "jira://acme/project-keys/TEAM",
        "jira://acme/projects:raw",
        "jira://acme/projects:1-2",
    ] {
        let reference = PathReference::parse(input).expect("readable but not canonical identity");
        assert!(SearchRecord::new(reference.clone(), 1, "metadata").is_err());
        assert!(
            SourceResource::utf8(reference, "metadata".to_owned(), Utf8ContentType::MARKDOWN)
                .is_err()
        );
    }
}

#[test]
fn source_offsets_do_not_change_global_selector_markers() {
    // The existing generic final-digit rule still sees line 7, not a native source offset.
    let workspace = PathReference::parse("file:offset:7").expect("literal workspace path");
    assert!(workspace.projection().is_none());
    let candidate = workspace
        .selector_candidate()
        .expect("legacy line candidate")
        .selector();
    assert!(candidate.line_selection().is_some());
    assert!(candidate.source_offset().is_none());
    let https = PathReference::parse("https://example.com/file:offset:7").expect("HTTPS");
    let ResourceAddress::Https(address) = https.address() else {
        panic!("HTTPS address")
    };
    assert_eq!(address.as_str(), "https://example.com/file:offset");
    assert!(
        https
            .projection()
            .expect("legacy line selection")
            .line_selection()
            .is_some()
    );
    let local = PathReference::parse("local://file:offset:7").expect("scratch name");
    assert!(local.projection().is_none());
    assert!(
        local
            .local_selector_candidate()
            .expect("legacy local candidate")
            .selector()
            .line_selection()
            .is_some()
    );
}
