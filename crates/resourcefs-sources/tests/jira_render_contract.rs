use std::time::{Duration, Instant};

use resourcefs_core::{ErrorCategory, MAX_HTTP_FETCH_BYTES};
use resourcefs_sources::{JiraWireLookupForTest, inspect_jira_render_for_test};

const ORIGIN: &str = "https://acme.invalid/";

fn issue(fields: &str, names: &str, schemas: &str) -> String {
    format!(
        r#"{{
            "id":"10001",
            "key":"NEW-2",
            "self":"https://acme.invalid/rest/api/3/issue/10001",
            "fields":{fields},
            "names":{names},
            "schema":{schemas}
        }}"#
    )
}

#[test]
fn aggregate_orders_and_links_all_visible_fields() {
    let body = issue(
        r#"{"zeta":null,"alpha":"first","middle":{"b":2,"a":1}}"#,
        r#"{"zeta":"Same","alpha":"Same","middle":"Middle"}"#,
        r#"{
            "zeta":{"type":"any"},
            "alpha":{"type":"string"},
            "middle":{"type":"object"}
        }"#,
    );
    let rendered = inspect_jira_render_for_test(
        body.as_bytes(),
        ORIGIN,
        "acme",
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect("rendered issue");

    assert_eq!(rendered.canonical_reference, "jira://acme/issues/10001");
    let alpha = rendered.aggregate.find("### alpha").expect("alpha");
    let middle = rendered.aggregate.find("### middle").expect("middle");
    let zeta = rendered.aggregate.find("### zeta").expect("zeta");
    assert!(alpha < middle && middle < zeta, "stable field-ID order");
    for (id, name, native_type) in [
        ("alpha", "Same", "string"),
        ("middle", "Middle", "object"),
        ("zeta", "Same", "any"),
    ] {
        let reference = format!("jira://acme/issues/10001/fields/{id}");
        assert!(rendered.aggregate.contains(&format!("Name: {name:?}")));
        assert!(
            rendered
                .aggregate
                .contains(&format!("Native Type: {native_type:?}"))
        );
        assert!(rendered.aggregate.contains("Mutable: false"));
        assert!(
            rendered
                .aggregate
                .contains(&format!("Reference: {reference}"))
        );
        assert!(rendered.field_index.contains(&format!("Field ID: {id}")));
        assert!(
            rendered
                .field_index
                .contains(&format!("Reference: {reference}"))
        );
    }
    assert!(rendered.aggregate.contains(r#"{"a":1,"b":2}"#));
    assert_eq!(rendered.fields.len(), 3);
    assert!(rendered.warnings.is_empty());
}

#[test]
fn field_index_uses_present_values_only() {
    let body = issue(
        r#"{"present":"value"}"#,
        r#"{"present":"Present","metadata_only":"Metadata only"}"#,
        r#"{
            "present":{"type":"string"},
            "metadata_only":{"type":"string"}
        }"#,
    );
    let rendered = inspect_jira_render_for_test(
        body.as_bytes(),
        ORIGIN,
        "acme",
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect("rendered issue");
    assert!(rendered.field_index.contains("Field ID: present"));
    assert!(!rendered.field_index.contains("metadata_only"));
    assert_eq!(rendered.fields.len(), 1);
}

#[test]
fn unsupported_adf_nodes_are_visible_and_lossless() {
    let body = issue(
        r#"{
            "description":{
                "version":1,
                "type":"doc",
                "content":[
                    {"type":"paragraph","content":[{"type":"text","text":"Before "}]},
                    {"type":"mysteryPanel","attrs":{"unknown":true},"content":[
                        {"type":"paragraph","content":[{"type":"text","text":"inside"}]}
                    ]},
                    {"type":"paragraph","content":[
                        {"type":"text","text":"after","marks":[{"type":"rainbow"}]}
                    ]}
                ],
                "futureRootMember":{"kept":true}
            }
        }"#,
        r#"{"description":"Description"}"#,
        r#"{"description":{"type":"string","custom":"textarea"}}"#,
    );
    let rendered = inspect_jira_render_for_test(
        body.as_bytes(),
        ORIGIN,
        "acme",
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect("ADF projection");

    let field = &rendered.fields[0];
    assert_eq!(
        field.content_type,
        "application/vnd.atlassian.adf+json; charset=utf-8"
    );
    assert!(
        field
            .canonical_json
            .contains(r#""futureRootMember":{"kept":true}"#)
    );
    assert!(field.canonical_json.contains(r#""attrs":{"unknown":true}"#));
    assert!(rendered.aggregate.contains("Before"));
    assert!(
        rendered
            .aggregate
            .contains("[Unsupported ADF node: mysteryPanel]")
    );
    assert!(
        rendered
            .aggregate
            .contains("[Unsupported ADF mark: rainbow]")
    );
    assert!(rendered.aggregate.contains("inside"));
    assert!(rendered.aggregate.contains("after"));
    assert_eq!(rendered.warnings.len(), 2);
    assert_eq!(rendered.warnings[0].kind, "node");
    assert_eq!(rendered.warnings[0].native_type, "mysteryPanel");
    assert_eq!(rendered.warnings[0].path, "/content/1");
    assert_eq!(
        rendered.warnings[0].field_reference,
        "jira://acme/issues/10001/fields/description"
    );
    assert_eq!(rendered.warnings[1].kind, "mark");
    assert_eq!(rendered.warnings[1].native_type, "rainbow");
}

#[test]
fn malformed_adf_fails_atomically() {
    for value in [
        r#"{"type":"doc","content":[]}"#,
        r#"{"version":2,"type":"doc","content":[]}"#,
        r#"{"version":1,"type":"doc","content":{}}"#,
        r#"{"version":1,"type":"doc","content":[{"type":"paragraph"}]}"#,
        r#"{"version":1,"type":"doc","content":[{"type":"text"}]}"#,
        r#"{"version":1,"type":"doc","content":[{"type":"unknown","content":{}}]}"#,
    ] {
        let body = issue(
            &format!(r#"{{"description":{value}}}"#),
            r#"{"description":"Description"}"#,
            r#"{"description":{"type":"string"}}"#,
        );
        let error = inspect_jira_render_for_test(
            body.as_bytes(),
            ORIGIN,
            "acme",
            JiraWireLookupForTest::StableId("10001".to_owned()),
        )
        .expect_err("malformed ADF authority");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    }
}

#[test]
fn empty_field_set_renders_complete_empty_documents() {
    let body = issue("{}", "{}", "{}");
    let rendered = inspect_jira_render_for_test(
        body.as_bytes(),
        ORIGIN,
        "acme",
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect("empty issue");
    assert!(
        rendered
            .aggregate
            .contains("## Fields\n\nNo visible fields.\n")
    );
    assert!(rendered.field_index.contains("No visible fields.\n"));
    assert!(rendered.fields.is_empty());
}

#[test]
fn adf_maximum_fixture() {
    let value = "x".repeat(MAX_HTTP_FETCH_BYTES - 2_048);
    let fields = format!(
        r#"{{"description":{{"version":1,"type":"doc","content":[{{"type":"paragraph","content":[{{"type":"text","text":"{value}"}}]}}]}}}}"#
    );
    let body = issue(
        &fields,
        r#"{"description":"Description"}"#,
        r#"{"description":{"type":"string"}}"#,
    );
    assert!(body.len() <= MAX_HTTP_FETCH_BYTES);
    let started = Instant::now();
    let rendered = inspect_jira_render_for_test(
        body.as_bytes(),
        ORIGIN,
        "acme",
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect("maximum ADF issue");
    assert!(rendered.aggregate.contains(&value));
    let budget = if cfg!(debug_assertions) {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(2)
    };
    assert!(
        started.elapsed() <= budget,
        "maximum ADF render must stay within the plan budget"
    );
}
