//! Live read-only smoke over a real Jira Cloud Site Mount (rfs-pm0y).
//!
//! Deterministic TLS contracts remain permanent. This ignored row targets what
//! their fake cannot prove: the tenant's current IssueBean optional shapes,
//! ID/key alias resolution, field metadata, ADF representation, and validator
//! behavior. It performs GET requests only and asserts shapes, never counts.
#[path = "support/mod.rs"]
mod session_support;

use std::sync::Arc;

use resourcefs_core::{
    AllowedOrigin, AtlassianSiteId, HttpCeilings, OperationGuard, OriginAllowlist, PathReference,
    Secret, SourceAdapter, SourceResource, VersionTag,
};
use resourcefs_sources::{
    AtlassianSite, AtlassianSource, AtlassianSourceMount, HttpSubstrate, OriginCredential,
};

struct LiveConfig {
    site_url: String,
    site_id: String,
    email: String,
    token: String,
    issue_id: String,
    issue_key: String,
    json_field_id: String,
    adf_field_id: String,
}

fn live_config() -> Option<LiveConfig> {
    if std::env::var("RFS_LIVE").as_deref() != Ok("1") {
        eprintln!("RFS_LIVE is not 1; skipping the live Jira smoke");
        return None;
    }
    let required = [
        "ATLASSIAN_SITE_URL",
        "ATLASSIAN_SITE_ID",
        "ATLASSIAN_EMAIL",
        "ATLASSIAN_API_TOKEN",
        "ATLASSIAN_ISSUE_ID",
        "ATLASSIAN_ISSUE_KEY",
        "ATLASSIAN_JSON_FIELD_ID",
        "ATLASSIAN_ADF_FIELD_ID",
    ];
    let missing = required
        .iter()
        .filter(|name| std::env::var(name).is_err())
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        eprintln!(
            "missing Jira live-smoke environment names: {}; skipping",
            missing.join(", ")
        );
        return None;
    }
    Some(LiveConfig {
        site_url: std::env::var("ATLASSIAN_SITE_URL").expect("checked environment"),
        site_id: std::env::var("ATLASSIAN_SITE_ID").expect("checked environment"),
        email: std::env::var("ATLASSIAN_EMAIL").expect("checked environment"),
        token: std::env::var("ATLASSIAN_API_TOKEN").expect("checked environment"),
        issue_id: std::env::var("ATLASSIAN_ISSUE_ID").expect("checked environment"),
        issue_key: std::env::var("ATLASSIAN_ISSUE_KEY").expect("checked environment"),
        json_field_id: std::env::var("ATLASSIAN_JSON_FIELD_ID").expect("checked environment"),
        adf_field_id: std::env::var("ATLASSIAN_ADF_FIELD_ID").expect("checked environment"),
    })
}

async fn live_source(config: &LiveConfig) -> AtlassianSource {
    let origin = AllowedOrigin::new(&config.site_url, false).expect("Atlassian origin");
    let token = Secret::new(config.token.clone()).expect("API token");
    let credential = OriginCredential::basic(origin.clone(), &config.email, &token)
        .expect("API-token Basic credential");
    let substrate = HttpSubstrate::new(
        OriginAllowlist::new(vec![origin.clone()]),
        HttpCeilings::default(),
        vec![credential],
    )
    .expect("HTTP substrate");
    let site = AtlassianSite::new(
        AtlassianSiteId::new(config.site_id.clone()).expect("Site ID"),
        origin,
    )
    .expect("Atlassian Site Mount");
    let session = session_support::scratch_fixture().await;
    AtlassianSourceMount::new(vec![site], Arc::new(substrate))
        .expect("Atlassian mount")
        .bind(session.path_session().clone())
}

async fn read(source: &AtlassianSource, reference: String) -> SourceResource {
    source
        .read(
            &PathReference::parse(&reference).expect("Path Reference"),
            &OperationGuard::new(),
        )
        .await
        .unwrap_or_else(|error| panic!("{reference}: {:?} {}", error.category(), error.message()))
}

#[tokio::test]
#[ignore = "live Jira Cloud smoke; needs read-only ATLASSIAN_* fixture variables"]
async fn live_jira_issue_read() {
    let Some(config) = live_config() else {
        return;
    };
    let source = live_source(&config).await;
    let stable_reference = format!("jira://{}/issues/{}", config.site_id, config.issue_id);
    let alias_reference = format!("jira://{}/issue-keys/{}", config.site_id, config.issue_key);

    let stable = read(&source, stable_reference.clone()).await;
    let alias = read(&source, alias_reference).await;
    assert_eq!(stable.canonical_reference(), stable_reference);
    assert_eq!(alias.canonical_reference(), stable_reference);
    assert_eq!(stable.content(), alias.content());
    assert_eq!(stable.version_tag(), alias.version_tag());
    assert_eq!(stable.content_type(), "text/markdown; charset=utf-8");

    let index_reference = format!("{stable_reference}/fields");
    let index = read(&source, index_reference).await;
    for field_id in [&config.json_field_id, &config.adf_field_id] {
        assert!(
            index
                .content()
                .contains(&format!("Reference: {stable_reference}/fields/{field_id}")),
            "live Field index lacks {field_id}"
        );
    }

    let json = read(
        &source,
        format!("{stable_reference}/fields/{}", config.json_field_id),
    )
    .await;
    assert_eq!(json.content_type(), "application/json; charset=utf-8");
    serde_json::from_str::<serde_json::Value>(json.content()).expect("canonical JSON Field");
    assert_eq!(
        json.version_tag(),
        &VersionTag::from_content(json.content().as_bytes())
    );

    let adf = read(
        &source,
        format!("{stable_reference}/fields/{}", config.adf_field_id),
    )
    .await;
    assert_eq!(
        adf.content_type(),
        "application/vnd.atlassian.adf+json; charset=utf-8"
    );
    let adf_value = serde_json::from_str::<serde_json::Value>(adf.content())
        .expect("complete canonical ADF Field");
    assert_eq!(
        adf_value.get("type").and_then(|value| value.as_str()),
        Some("doc")
    );
    assert_eq!(
        adf_value.get("version").and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        adf.version_tag(),
        &VersionTag::from_content(adf.content().as_bytes())
    );

    let repeated = read(&source, stable_reference).await;
    assert_eq!(repeated.canonical_reference(), stable.canonical_reference());
    assert_eq!(repeated.content(), stable.content());
    assert_eq!(repeated.version_tag(), stable.version_tag());

    let debug = format!("{source:?}");
    for secret in [&config.email, &config.token] {
        assert!(!debug.contains(secret));
        assert!(!stable.content().contains(secret));
        assert!(!index.content().contains(secret));
    }
}
