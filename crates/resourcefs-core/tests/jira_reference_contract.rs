use std::time::{Duration, Instant};

use resourcefs_core::{
    AtlassianSiteId, ErrorCategory, JiraAddress, JiraFieldId, JiraIssueId, JiraIssueKey,
    JiraIssueResource, MAX_ATLASSIAN_SITE_ID_BYTES, MAX_JIRA_ISSUE_ID_BYTES,
    MAX_JIRA_SEGMENT_BYTES, MAX_PATH_REFERENCE_BYTES, PathReference, ResourceAddress,
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
