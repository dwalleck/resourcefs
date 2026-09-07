//! Live read-only smoke over a real Jira Cloud Site Mount (rfs-pm0y).
//!
//! Deterministic TLS contracts remain permanent. These ignored rows target
//! tenant IssueBean shapes, ID/key resolution, fields, and native query order.
//! Direct resources use GET; JQL uses Jira's enhanced read-only POST endpoint.
//! Query order is compared with an independently provisioned fixture receipt.
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

async fn live_source(
    site_url: &str,
    site_id: &str,
    email: &str,
    token: &str,
) -> (AtlassianSource, session_support::ScratchFixture) {
    let origin = AllowedOrigin::new(site_url, false).expect("Atlassian origin");
    let token = Secret::new(token.to_owned()).expect("API token");
    let credential =
        OriginCredential::basic(origin.clone(), email, &token).expect("API-token Basic credential");
    let substrate = HttpSubstrate::new(
        OriginAllowlist::new(vec![origin.clone()]),
        HttpCeilings::default(),
        vec![credential],
    )
    .expect("HTTP substrate");
    let site = AtlassianSite::new(
        AtlassianSiteId::new(site_id.to_owned()).expect("Site ID"),
        origin,
    )
    .expect("Atlassian Site Mount");
    let session = session_support::scratch_fixture().await;
    let source = AtlassianSourceMount::new(vec![site], Arc::new(substrate))
        .expect("Atlassian mount")
        .bind(session.path_session().clone());
    (source, session)
}

async fn read(source: &AtlassianSource, reference: String) -> SourceResource {
    source
        .read(
            &PathReference::parse(&reference).expect("Path Reference"),
            &OperationGuard::new(),
        )
        .await
        .unwrap_or_else(|error| {
            panic!("live Jira read: {:?} {}", error.category(), error.message())
        })
}

#[tokio::test]
#[ignore = "live Jira Cloud smoke; needs read-only ATLASSIAN_* fixture variables"]
async fn live_jira_issue_read() {
    let Some(config) = live_config() else {
        return;
    };
    let (source, _session) = live_source(
        &config.site_url,
        &config.site_id,
        &config.email,
        &config.token,
    )
    .await;
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

#[tokio::test]
#[ignore = "reader-only live Jira projects; needs ATLASSIAN_PROJECT_ID and ATLASSIAN_PROJECT_KEY"]
async fn live_jira_project_browse() {
    let Some(config) = live_config() else { return };
    let (Ok(project_id), Ok(project_key)) = (
        std::env::var("ATLASSIAN_PROJECT_ID"),
        std::env::var("ATLASSIAN_PROJECT_KEY"),
    ) else {
        eprintln!("missing ATLASSIAN_PROJECT_ID or ATLASSIAN_PROJECT_KEY; skipping");
        return;
    };
    let (source, _session) = live_source(
        &config.site_url,
        &config.site_id,
        &config.email,
        &config.token,
    )
    .await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 2)
        .expect("lower live limits");
    let stable_reference = format!("jira://{}/projects/{project_id}", config.site_id);
    let stable = read(&source, stable_reference.clone()).await;
    let alias = read(
        &source,
        format!("jira://{}/project-keys/{project_key}", config.site_id),
    )
    .await;
    assert_eq!(stable.canonical_reference(), stable_reference);
    assert_eq!(alias.canonical_reference(), stable_reference);
    assert_eq!(alias.content(), stable.content());
    assert_eq!(stable.content_type(), "text/markdown; charset=utf-8");
    assert!(stable.content().contains(&project_key));
    let collection = format!("jira://{}/projects", config.site_id);
    let mut next = Some(collection.clone());
    let mut visited = std::collections::HashSet::new();
    let mut found = false;
    let mut followed = false;
    while let Some(reference) = next {
        assert!(
            visited.insert(reference.clone()),
            "source continuation must advance"
        );
        assert!(visited.len() <= 1000, "fixture traversal safety ceiling");
        let result = read(&source, reference).await;
        assert_eq!(result.canonical_reference(), collection);
        assert_eq!(result.content_type(), "text/markdown; charset=utf-8");
        assert_eq!(
            result.version_tag(),
            &VersionTag::from_content(result.content().as_bytes())
        );
        found |= result.content().contains(&stable_reference);
        next = result.continuation().map(str::to_owned);
        if let Some(reference) = &next {
            let typed = PathReference::parse(reference).expect("typed native continuation");
            assert!(
                typed
                    .projection()
                    .and_then(|selector| selector.source_offset())
                    .is_some()
            );
            followed = true;
        }
        if found && visited.len() > 1 {
            break;
        }
    }
    assert!(
        found,
        "known reader-visible fixture must occur on its source pages"
    );
    assert!(
        followed && visited.len() > 1,
        "exercise a real native continuation"
    );
}

#[tokio::test]
#[ignore = "reader-only live Jira browse; needs retained project and issue fixtures"]
async fn live_jira_browse() {
    let Some(config) = live_config() else { return };
    let Ok(project_id) = std::env::var("ATLASSIAN_PROJECT_ID") else {
        eprintln!("missing ATLASSIAN_PROJECT_ID; skipping");
        return;
    };
    let (source, _session) = live_source(
        &config.site_url,
        &config.site_id,
        &config.email,
        &config.token,
    )
    .await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 3)
        .expect("lower live limits");
    let project = format!("jira://{}/projects/{project_id}", config.site_id);
    let issue = format!("jira://{}/issues/{}", config.site_id, config.issue_id);
    let direct = read(&source, issue.clone()).await;
    assert_eq!(direct.canonical_reference(), issue);
    let summary = read(&source, format!("{issue}/fields/summary")).await;
    let summary: String = serde_json::from_str(summary.content()).expect("direct summary Field");
    let summary_row = format!(
        "Summary: {}",
        serde_json::to_string(&summary).expect("summary JSON")
    );
    for (collection, identity, cursor) in [
        (
            format!("jira://{}/projects", config.site_id),
            project.clone(),
            false,
        ),
        (
            format!("jira://{}/issues", config.site_id),
            issue.clone(),
            true,
        ),
        (format!("{project}/issues"), issue.clone(), true),
    ] {
        let mut next = Some(collection.clone());
        let mut visited = std::collections::HashSet::new();
        let mut found = false;
        while let Some(reference) = next {
            assert!(
                visited.insert(reference.clone()),
                "native continuation must advance"
            );
            assert!(visited.len() <= 1000, "fixture traversal safety ceiling");
            let page = read(&source, reference).await;
            assert_eq!(page.canonical_reference(), collection);
            assert_eq!(page.content_type(), "text/markdown; charset=utf-8");
            assert_eq!(
                page.version_tag(),
                &VersionTag::from_content(page.content().as_bytes())
            );
            if page.content().contains(&format!("Reference: {identity}\n")) {
                found = true;
                if cursor {
                    assert!(
                        page.content().contains(&summary_row),
                        "selected summary agrees with direct Field read"
                    );
                    assert!(
                        page.content().contains(&project),
                        "selected project identity agrees with fixture"
                    );
                }
            }
            next = page.continuation().map(str::to_owned);
            if let Some(reference) = &next {
                let typed = PathReference::parse(reference).expect("typed source continuation");
                let selector = typed.projection().expect("native page selector");
                if cursor {
                    assert!(selector.source_cursor().is_some());
                } else {
                    assert!(selector.source_offset().is_some());
                }
            }
            if found && visited.len() > 1 {
                break;
            }
        }
        assert!(found, "reader-visible fixture must occur in {collection}");
        assert!(
            visited.len() > 1,
            "exercise native pagination for {collection}"
        );
    }
}

#[tokio::test]
#[ignore = "reader-only live Jira JQL; needs owned fixture receipt and ATLASSIAN_READER_* credentials"]
async fn live_jira_jql() {
    if std::env::var("RFS_LIVE").as_deref() != Ok("1") {
        return;
    }
    let names = [
        "RFS_ATLASSIAN_SITE",
        "ATLASSIAN_READER_EMAIL",
        "ATLASSIAN_READER_API_TOKEN",
    ];
    if names.iter().any(|name| std::env::var(name).is_err()) {
        eprintln!("missing reader-only Jira JQL environment; skipping");
        return;
    }
    let site_url = std::env::var(names[0]).expect("checked environment");
    let site_id = "live";
    let email = std::env::var(names[1]).expect("checked environment");
    let token = std::env::var(names[2]).expect("checked environment");
    let receipt_path = std::env::var("ATLASSIAN_FIXTURE_RECEIPT")
        .unwrap_or_else(|_| ".resourcefs/atlassian-fixture-state.json".into());
    let receipt_path = std::fs::canonicalize(receipt_path).expect("owned fixture receipt path");
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(receipt_path).expect("owned fixture receipt"))
            .expect("fixture receipt JSON");
    assert_eq!(
        receipt["site"]["origin"]
            .as_str()
            .expect("receipt origin")
            .trim_end_matches('/'),
        site_url.trim_end_matches('/')
    );
    let project = &receipt["jira"]["projects"][0];
    let project_key = project["key"].as_str().expect("receipt project key");
    let prefix = format!("{project_key}-");
    let mut rows: Vec<_> = receipt["jira"]["issues"]
        .as_array()
        .expect("receipt issues")
        .iter()
        .filter(|row| row["key"].as_str().expect("issue key").starts_with(&prefix))
        .collect();
    assert!(
        (2..=10).contains(&rows.len()),
        "owned fixture must contain two through ten issues"
    );
    rows.sort_by_key(|row| {
        row["key"]
            .as_str()
            .expect("key")
            .rsplit('-')
            .next()
            .expect("suffix")
            .parse::<u64>()
            .expect("numeric issue suffix")
    });
    let (source, _session) = live_source(&site_url, site_id, &email, &token).await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 2)
        .expect("lower native limits");
    for row in &rows {
        let id = row["id"].as_str().expect("stable ID");
        let key = row["key"].as_str().expect("issue key");
        let stable = format!("jira://{site_id}/issues/{id}");
        let direct = read(&source, stable.clone()).await;
        let alias = read(&source, format!("jira://{site_id}/issue-keys/{key}")).await;
        assert_eq!(direct.canonical_reference(), stable);
        assert_eq!(alias.canonical_reference(), stable);
        assert_eq!(alias.content(), direct.content());
    }
    for direction in ["ASC", "DESC"] {
        let query = format!(
            "project = {} ORDER BY key {direction}",
            project["id"].as_str().expect("project ID")
        );
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let owner = format!("jira://{site_id}/search/{}", encoded.replace('+', "%20"));
        let mut next = Some(owner.clone());
        let mut visited = std::collections::HashSet::new();
        let mut actual = Vec::new();
        while let Some(reference) = next {
            assert!(
                visited.insert(reference.clone()),
                "C4 native token must advance"
            );
            assert!(visited.len() <= 12, "owned-fixture traversal bound");
            let page = read(&source, reference).await;
            assert_eq!(page.canonical_reference(), owner);
            let issue_prefix = format!("Reference: jira://{site_id}/issues/");
            actual.extend(
                page.content()
                    .lines()
                    .filter_map(|line| line.strip_prefix(&issue_prefix).map(str::to_owned)),
            );
            next = page.continuation().map(str::to_owned);
        }
        let mut expected: Vec<_> = rows
            .iter()
            .map(|row| row["id"].as_str().expect("ID").to_owned())
            .collect();
        if direction == "DESC" {
            expected.reverse();
        }
        assert_eq!(
            actual, expected,
            "C4 native order versus independent receipt and direct reads"
        );
        assert!(visited.len() >= 2, "C4 exercise real opaque continuation");
    }
}
