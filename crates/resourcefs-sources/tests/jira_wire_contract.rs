use std::time::{Duration, Instant};

use resourcefs_core::{ErrorCategory, MAX_HTTP_FETCH_BYTES};
use resourcefs_sources::{JiraWireLookupForTest, inspect_jira_wire_for_test};

const ORIGIN: &str = "https://acme.invalid/";

fn valid_issue(fields: &str, names: &str, schemas: &str) -> String {
    format!(
        r#"{{
            "id":"10001",
            "key":"NEW-2",
            "self":"https://acme.invalid/rest/api/3/issue/10001",
            "fields":{fields},
            "names":{names},
            "schema":{schemas},
            "expand":"names,schema"
        }}"#
    )
}

#[test]
fn valid_issue_authority_is_typed_and_field_sorted() {
    let body = valid_issue(
        r#"{
            "summary":"Hello",
            "labels":["b","a"],
            "customfield_9":{"z":1,"a":2},
            "empty":null
        }"#,
        r#"{
            "summary":"Summary",
            "labels":"Labels",
            "customfield_9":"Details",
            "empty":""
        }"#,
        r#"{
            "summary":{"type":"string","system":"summary"},
            "labels":{"type":"array","items":"string"},
            "customfield_9":{"type":"object","custom":"vendor:type"},
            "empty":{"type":"any"}
        }"#,
    );
    let observed = inspect_jira_wire_for_test(
        body.as_bytes(),
        ORIGIN,
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect("valid issue wire");

    assert_eq!(observed.issue_id, "10001");
    assert_eq!(observed.issue_key, "NEW-2");
    assert_eq!(
        observed.self_url,
        "https://acme.invalid/rest/api/3/issue/10001"
    );
    let ids = observed
        .fields
        .iter()
        .map(|field| field.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["customfield_9", "empty", "labels", "summary"]);
    assert_eq!(observed.fields[0].name, "Details");
    assert_eq!(observed.fields[0].native_type, "object");
    assert_eq!(observed.fields[0].canonical_json, r#"{"a":2,"z":1}"#);
    assert_eq!(observed.fields[1].canonical_json, "null");
    assert_eq!(observed.fields[2].canonical_json, r#"["b","a"]"#);
    assert_eq!(observed.fields[3].canonical_json, r#""Hello""#);
}

#[test]
fn canonical_json_is_order_and_whitespace_independent() {
    let left = valid_issue(
        r#"{"value":{"z":[3,2,1],"a":{"y":true,"x":false}}}"#,
        r#"{"value":"Value"}"#,
        r#"{"value":{"type":"object"}}"#,
    );
    let right = valid_issue(
        r#"{ "value" : { "a" : { "x" : false, "y" : true }, "z" : [3,2,1] } }"#,
        r#"{"value":"Value"}"#,
        r#"{"value":{"type":"object"}}"#,
    );
    let lookup = JiraWireLookupForTest::StableId("10001".to_owned());
    let left =
        inspect_jira_wire_for_test(left.as_bytes(), ORIGIN, lookup.clone()).expect("left issue");
    let right = inspect_jira_wire_for_test(right.as_bytes(), ORIGIN, lookup).expect("right issue");
    assert_eq!(
        left.fields[0].canonical_json,
        right.fields[0].canonical_json
    );
    assert_eq!(
        left.fields[0].canonical_json,
        r#"{"a":{"x":false,"y":true},"z":[3,2,1]}"#
    );
}

#[test]
fn canonical_json_keeps_native_value_kinds_distinct() {
    let body = valid_issue(
        r#"{
            "array":[],"boolean":false,"empty":"","null":null,
            "number":0,"object":{},"string":"0"
        }"#,
        r#"{
            "array":"array","boolean":"boolean","empty":"empty","null":"null",
            "number":"number","object":"object","string":"string"
        }"#,
        r#"{
            "array":{"type":"array"},"boolean":{"type":"boolean"},
            "empty":{"type":"string"},"null":{"type":"any"},
            "number":{"type":"number"},"object":{"type":"object"},
            "string":{"type":"string"}
        }"#,
    );
    let observed = inspect_jira_wire_for_test(
        body.as_bytes(),
        ORIGIN,
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect("value-kind matrix");
    let values = observed
        .fields
        .iter()
        .map(|field| field.canonical_json.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        ["[]", "false", r#""""#, "null", "0", "{}", r#""0""#]
    );
}

#[test]
fn canonical_json_preserves_arbitrary_precision_numbers() {
    let body = valid_issue(
        r#"{
            "decimal":12345678901234567890.12345678901234567890,
            "equivalent":1.2300e2,
            "large":1e30,
            "negative_zero":-0.0,
            "small":0.00000100
        }"#,
        r#"{
            "decimal":"decimal","equivalent":"equivalent","large":"large",
            "negative_zero":"negative zero","small":"small"
        }"#,
        r#"{
            "decimal":{"type":"number"},"equivalent":{"type":"number"},
            "large":{"type":"number"},"negative_zero":{"type":"number"},
            "small":{"type":"number"}
        }"#,
    );
    let observed = inspect_jira_wire_for_test(
        body.as_bytes(),
        ORIGIN,
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect("arbitrary-precision number matrix");
    let value = |id: &str| {
        observed
            .fields
            .iter()
            .find(|field| field.id == id)
            .expect("numeric field")
            .canonical_json
            .as_str()
    };
    assert_eq!(value("decimal"), "12345678901234567890.1234567890123456789");
    assert_eq!(value("equivalent"), "123");
    assert_eq!(value("large"), "1e+30");
    assert_eq!(value("negative_zero"), "0");
    assert_eq!(value("small"), "0.000001");
}

#[test]
fn authority_presence_and_type_matrix_fails_atomically() {
    let valid = valid_issue(
        r#"{"summary":"Hello"}"#,
        r#"{"summary":"Summary"}"#,
        r#"{"summary":{"type":"string"}}"#,
    );
    for malformed in [
        valid.replace(r#""id":"10001","#, ""),
        valid.replace(r#""key":"NEW-2","#, r#""key":null,"#),
        valid.replace(
            r#""self":"https://acme.invalid/rest/api/3/issue/10001","#,
            "",
        ),
        valid.replace(r#""fields":{"summary":"Hello"}"#, r#""fields":null"#),
        valid.replace(r#""names":{"summary":"Summary"}"#, r#""names":{}"#),
        valid.replace(
            r#""schema":{"summary":{"type":"string"}}"#,
            r#""schema":{"summary":{}}"#,
        ),
        valid.replace(r#""type":"string""#, r#""type":"""#),
    ] {
        let error = inspect_jira_wire_for_test(
            malformed.as_bytes(),
            ORIGIN,
            JiraWireLookupForTest::StableId("10001".to_owned()),
        )
        .expect_err("malformed authority");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    }
}

#[test]
fn duplicate_members_are_rejected_at_every_depth() {
    for body in [
        r#"{"id":"10001","id":"10002"}"#.to_owned(),
        valid_issue(
            r#"{"value":{"x":1,"x":2}}"#,
            r#"{"value":"Value"}"#,
            r#"{"value":{"type":"object"}}"#,
        ),
    ] {
        let error = inspect_jira_wire_for_test(
            body.as_bytes(),
            ORIGIN,
            JiraWireLookupForTest::StableId("10001".to_owned()),
        )
        .expect_err("duplicate member");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    }
}

#[test]
fn excessive_json_depth_fails_without_stack_overflow() {
    let nested = format!("{}0{}", "[".repeat(512), "]".repeat(512));
    let body = valid_issue(
        &format!(r#"{{"value":{nested}}}"#),
        r#"{"value":"Value"}"#,
        r#"{"value":{"type":"array"}}"#,
    );
    let error = inspect_jira_wire_for_test(
        body.as_bytes(),
        ORIGIN,
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect_err("excessive recursive JSON depth");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
}

#[test]
fn stable_identity_and_self_url_confusion_are_rejected() {
    let valid = valid_issue(
        r#"{"summary":"Hello"}"#,
        r#"{"summary":"Summary"}"#,
        r#"{"summary":{"type":"string"}}"#,
    );
    for (body, lookup) in [
        (
            valid.clone(),
            JiraWireLookupForTest::StableId("10002".to_owned()),
        ),
        (
            valid.replace("https://acme.invalid/", "https://other.invalid/"),
            JiraWireLookupForTest::StableId("10001".to_owned()),
        ),
        (
            valid.replace("/rest/api/3/issue/10001", "/rest/api/2/issue/10001"),
            JiraWireLookupForTest::StableId("10001".to_owned()),
        ),
        (
            valid.replace("/issue/10001", "/issue/99999"),
            JiraWireLookupForTest::StableId("10001".to_owned()),
        ),
    ] {
        let error = inspect_jira_wire_for_test(body.as_bytes(), ORIGIN, lookup)
            .expect_err("identity confusion");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    }
}

#[test]
fn moved_issue_key_alias_publishes_returned_stable_identity() {
    let body = valid_issue(
        r#"{"summary":"Hello"}"#,
        r#"{"summary":"Summary"}"#,
        r#"{"summary":{"type":"string"}}"#,
    );
    let observed = inspect_jira_wire_for_test(
        body.as_bytes(),
        ORIGIN,
        JiraWireLookupForTest::IssueKey("OLD-1".to_owned()),
    )
    .expect("moved key alias");
    assert_eq!(observed.issue_id, "10001");
    assert_eq!(observed.issue_key, "NEW-2");
}

#[test]
fn canonical_json_maximum_fixture_within_budget() {
    let value = "x".repeat(MAX_HTTP_FETCH_BYTES - 1_024);
    let body = valid_issue(
        &format!(r#"{{"large":"{value}"}}"#),
        r#"{"large":"Large"}"#,
        r#"{"large":{"type":"string"}}"#,
    );
    assert!(body.len() <= MAX_HTTP_FETCH_BYTES);
    let started = Instant::now();
    let observed = inspect_jira_wire_for_test(
        body.as_bytes(),
        ORIGIN,
        JiraWireLookupForTest::StableId("10001".to_owned()),
    )
    .expect("maximum bounded issue");
    assert_eq!(observed.fields[0].canonical_json.len(), value.len() + 2);
    let budget = if cfg!(debug_assertions) {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(2)
    };
    assert!(
        started.elapsed() <= budget,
        "maximum canonical JSON must stay within the plan budget"
    );
}
