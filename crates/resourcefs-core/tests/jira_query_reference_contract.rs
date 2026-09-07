use resourcefs_core::{
    AtlassianSiteId, ErrorCategory, JiraAddress, JiraQuery, MAX_PATH_REFERENCE_BYTES,
    PathReference, ProjectionSelector, ResourceAddress, SourceCursor, SourceResource,
    Utf8ContentType,
};

fn address(query: &str) -> JiraAddress {
    JiraAddress::Query {
        site: AtlassianSiteId::new("acme").expect("site"),
        query: JiraQuery::new(query).expect("nonempty native query"),
    }
}

#[test]
fn canonical_query_corpus_preserves_exact_native_bytes() {
    let pairs = [
        (
            "project = DEMO ORDER BY key DESC",
            "project%20%3D%20DEMO%20ORDER%20BY%20key%20DESC",
        ),
        ("café 東京", "caf%C3%A9%20%E6%9D%B1%E4%BA%AC"),
        ("/\\:%+?#\"", "%2F%5C%3A%25%2B%3F%23%22"),
        ("\0\t\n\r\u{1f}\u{7f}", "%00%09%0A%0D%1F%7F"),
        ("%2F", "%252F"),
        ("a:cursor:opaque", "a%3Acursor%3Aopaque"),
        (" ", "%20"),
        (".", "."),
        ("..", ".."),
        ("new", "new"),
        ("-._~AZaz09", "-._~AZaz09"),
    ];
    for (native, encoded) in pairs {
        let expected = format!("jira://acme/search/{encoded}");
        let constructed = PathReference::jira(address(native), None).expect("construct C2");
        assert_eq!(constructed.requested(), expected, "C2 canonical encoding");
        let parsed = PathReference::parse(&expected).expect("parse C2");
        assert_eq!(constructed, parsed);
        let ResourceAddress::Jira(JiraAddress::Query { query, site }) = parsed.address() else {
            panic!("C2 must retain the query variant")
        };
        assert_eq!(query.as_str(), native, "C2 exact decode once");
        assert_eq!(site.as_str(), "acme");
    }
}

#[test]
fn malformed_query_spellings_fail_without_key_language_restrictions() {
    for segment in [
        "", "%", "%2", "%GG", "%FF", "%C3%28", "%2f", "%5c", "%41", "a+b", "a b", "a/b", "a\\b",
        "a?b", "a#b", "café",
    ] {
        let error = PathReference::parse(format!("jira://acme/search/{segment}"))
            .expect_err("C2 noncanonical or malformed query");
        assert_eq!(
            error.category(),
            ErrorCategory::InvalidReference,
            "C2 {segment:?}"
        );
    }
    assert_eq!(
        JiraQuery::new("").expect_err("empty query").category(),
        ErrorCategory::InvalidReference
    );
    // Existing key paths retain their separator/control restrictions.
    for path in [
        "issue-keys/a%2Fb",
        "project-keys/a%5Cb",
        "issues/1/fields/%00",
    ] {
        assert!(PathReference::parse(format!("jira://acme/{path}")).is_err());
    }
}

#[test]
fn query_selectors_and_source_identity_remain_separate() {
    let native = address("summary ~ \"a:b\"");
    for selector in ["raw", "2-4", "cursor:e30"] {
        let reference = PathReference::jira(
            native.clone(),
            Some(ProjectionSelector::parse(selector).expect("selector")),
        )
        .expect("C2 allowed query selector");
        assert_eq!(
            PathReference::parse(reference.requested()).expect("parse selector"),
            reference
        );
    }
    let offset = ProjectionSelector::parse("offset:1").expect("offset syntax");
    assert_eq!(
        PathReference::jira(native.clone(), Some(offset))
            .expect_err("queries cannot use offset")
            .category(),
        ErrorCategory::InvalidReference
    );
    let cursor = ProjectionSelector::from_source_cursor(
        SourceCursor::new("e30".to_owned()).expect("canonical base64url syntax"),
    );
    let base = PathReference::jira(native.clone(), None).expect("canonical query");
    let paged = PathReference::jira(native, Some(cursor)).expect("source cursor");
    let resource = SourceResource::utf8(
        base.clone(),
        "query rows".to_owned(),
        Utf8ContentType::MARKDOWN,
    )
    .expect("C2 canonical source identity")
    .with_continuation(&paged);
    assert_eq!(resource.canonical_reference(), base.requested());
    assert_eq!(resource.continuation(), Some(paged.requested()));
}

#[test]
fn query_reference_ceiling_accounts_for_encoding_and_selector_bytes() {
    let prefix = "jira://acme/search/";
    let length = MAX_PATH_REFERENCE_BYTES - prefix.len();
    let native = "a".repeat(length);
    let exact = PathReference::jira(address(&native), None).expect("C2 exact ceiling");
    assert_eq!(exact.requested().len(), MAX_PATH_REFERENCE_BYTES);
    assert_eq!(
        PathReference::parse(exact.requested()).expect("parse ceiling"),
        exact
    );
    assert_eq!(
        PathReference::jira(address(&(native + "a")), None)
            .expect_err("ceiling+1")
            .category(),
        ErrorCategory::LimitExceeded
    );
    assert_eq!(
        PathReference::jira(address(&"/".repeat(length / 3 + 1)), None)
            .expect_err("encoded expansion ceiling")
            .category(),
        ErrorCategory::LimitExceeded
    );
    assert_eq!(
        PathReference::jira(
            address(&"a".repeat(length)),
            Some(ProjectionSelector::parse("raw").expect("selector"))
        )
        .expect_err("selector counts")
        .category(),
        ErrorCategory::LimitExceeded
    );
    let unicode = "東京/\\\n".repeat(1_000);
    let large =
        PathReference::jira(address(&unicode), None).expect("long native query bypasses key limit");
    let ResourceAddress::Jira(JiraAddress::Query { query, .. }) = large.address() else {
        panic!("query")
    };
    assert_eq!(query.as_str(), unicode);
}

#[test]
fn query_debug_redacts_native_expression() {
    let query = JiraQuery::new("PRIVATE_QUERY_CANARY").expect("query");
    assert_eq!(query.as_str(), "PRIVATE_QUERY_CANARY");
    assert!(!format!("{query:?}").contains("PRIVATE_QUERY_CANARY"));
    assert!(!format!("{:?}", address(query.as_str())).contains("PRIVATE_QUERY_CANARY"));
}
