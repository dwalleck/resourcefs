use resourcefs_core::ErrorCategory;
use resourcefs_sources::{inspect_issue_wire_for_test, inspect_project_wire_for_test};
use serde_json::{Value, json};

const ORIGIN: &str = "https://acme.invalid/";
const REQUEST: &str =
    "https://acme.invalid/rest/api/3/project/search?startAt=0&maxResults=100&orderBy=key";

fn project() -> Value {
    json!({"id":"10001","key":"TEAM","name":"Project", "self":"https://acme.invalid/rest/api/3/project/10001"})
}

fn reject_project(body: &[u8]) {
    assert_eq!(
        inspect_project_wire_for_test(body, ORIGIN, None, Some("10001"))
            .unwrap_err()
            .category(),
        ErrorCategory::SourceUnavailable
    );
}

#[test]
fn compact_authority_and_presence_matrix() {
    compact_issue_authority_and_presence_matrix();
    let absent = inspect_project_wire_for_test(
        project().to_string().as_bytes(),
        ORIGIN,
        None,
        Some("10001"),
    )
    .unwrap();
    assert_eq!(absent.projects[0].project_type_key, None);
    assert_eq!(absent.projects[0].archived, None);
    assert_eq!(absent.projects[0].deleted, None);
    for (kind, archived, deleted) in [
        (Value::Null, Value::Null, Value::Null),
        (json!(""), json!(false), json!(false)),
        (json!("software"), json!(true), json!(true)),
    ] {
        let mut body = project();
        body["projectTypeKey"] = kind.clone();
        body["archived"] = archived.clone();
        body["deleted"] = deleted.clone();
        let result =
            inspect_project_wire_for_test(body.to_string().as_bytes(), ORIGIN, None, Some("10001"))
                .unwrap();
        let row = &result.projects[0];
        assert_eq!(row.project_type_key, Some(kind.as_str().map(str::to_owned)));
        assert_eq!(row.archived, Some(archived.as_bool()));
        assert_eq!(row.deleted, Some(deleted.as_bool()));
        assert_ne!(row.rendered, absent.projects[0].rendered);
    }
    for field in ["id", "key", "name", "self"] {
        let mut body = project();
        body.as_object_mut().unwrap().remove(field);
        reject_project(body.to_string().as_bytes());
        for invalid in [Value::Null, json!(42), json!({})] {
            let mut body = project();
            body[field] = invalid;
            reject_project(body.to_string().as_bytes());
        }
    }
    for (field, invalid) in [
        ("projectTypeKey", json!(false)),
        ("archived", json!("false")),
        ("deleted", json!(0)),
    ] {
        let mut body = project();
        body[field] = invalid;
        reject_project(body.to_string().as_bytes());
    }
    for invalid in ["0", "01", "-1", "abc", "10002", &"1".repeat(65)] {
        let mut body = project();
        body["id"] = json!(invalid);
        reject_project(body.to_string().as_bytes());
    }
    for authority in [
        "https://foreign.invalid/rest/api/3/project/10001",
        "https://user@acme.invalid/rest/api/3/project/10001",
        "https://acme.invalid/rest/api/3/issue/10001",
        "https://acme.invalid/rest/api/3/project/10002",
        "https://acme.invalid/rest/api/3/project/10001?x=1",
        "https://acme.invalid/rest/api/3/project/10001#fragment",
    ] {
        let mut body = project();
        body["self"] = json!(authority);
        reject_project(body.to_string().as_bytes());
    }
    let body = project().to_string();
    reject_project(format!("{} trailing", body).as_bytes());
    reject_project(body.replacen('{', "{\"id\":\"10001\",", 1).as_bytes());
    reject_project(
        body.replacen('{', "{\"unknown\":{\"a\":1,\"a\":2},", 1)
            .as_bytes(),
    );
    reject_project(
        body.replacen(
            '{',
            &format!("{{\"deep\":{}0{},", "[".repeat(130), "]".repeat(130)),
            1,
        )
        .as_bytes(),
    );
    let mut unknown = project();
    unknown["unknown"] = json!({"valid":[1, null, false]});
    assert_eq!(
        inspect_project_wire_for_test(unknown.to_string().as_bytes(), ORIGIN, None, Some("10001"))
            .unwrap()
            .projects,
        absent.projects
    );
}

fn page(next: Option<&str>) -> Value {
    let mut body =
        json!({"values":[],"startAt":0,"maxResults":7,"isLast":next.is_none(),"total":0});
    if let Some(next) = next {
        body["nextPage"] = json!(next);
    }
    body
}

#[test]
fn project_pages_require_authoritative_advancing_offsets() {
    let next = "https://acme.invalid/rest/api/3/project/search?startAt=13&maxResults=7&orderBy=key";
    for next in [
        next.to_owned(),
        next.replace("maxResults=7", "maxResults=100"),
    ] {
        let result = inspect_project_wire_for_test(
            page(Some(&next)).to_string().as_bytes(),
            ORIGIN,
            Some(REQUEST),
            None,
        )
        .unwrap();
        assert_eq!(result.next_offset, Some(13));
        assert_eq!(result.max_results, Some(7));
    }
    let terminal = inspect_project_wire_for_test(
        page(None).to_string().as_bytes(),
        ORIGIN,
        Some(REQUEST),
        None,
    )
    .unwrap();
    assert_eq!(terminal.next_offset, None);
    for invalid in [
        next.replace("acme.invalid", "foreign.invalid"),
        next.replace("project/search", "issue/search"),
        next.replace("startAt=13", "startAt=0"),
        next.replace("startAt=13", "startAt=01"),
        next.replace("startAt=13", "startAt=18446744073709551616"),
        next.replace("maxResults=7", "maxResults=8"),
        next.replace("orderBy=key", "orderBy=name"),
        format!("{next}&startAt=14"),
        format!("{next}#fragment"),
    ] {
        reject_page(&page(Some(&invalid)));
    }
    for field in ["values", "startAt", "maxResults", "isLast", "nextPage"] {
        let mut body = page(Some(next));
        body.as_object_mut().unwrap().remove(field);
        reject_page(&body);
        body[field] = Value::Null;
        reject_page(&body);
    }
    for (field, invalid) in [
        ("startAt", json!(1)),
        ("maxResults", json!(0)),
        ("maxResults", json!(101)),
        ("isLast", json!(true)),
        ("values", json!({})),
    ] {
        let mut body = page(Some(next));
        body[field] = invalid;
        reject_page(&body);
    }
    let mut duplicate = page(None);
    duplicate["values"] = json!([project(), project()]);
    reject_page(&duplicate);
    let mut overfull = page(None);
    overfull["maxResults"] = json!(1);
    overfull["values"] = json!([project(), project()]);
    reject_page(&overfull);
}

fn reject_page(body: &Value) {
    assert_eq!(
        inspect_project_wire_for_test(body.to_string().as_bytes(), ORIGIN, Some(REQUEST), None)
            .unwrap_err()
            .category(),
        ErrorCategory::SourceUnavailable
    );
}

fn issue() -> Value {
    json!({
        "id": "20001", "key": "TEAM-1",
        "self": "https://acme.invalid/rest/api/3/issue/20001",
        "fields": {
            "summary": "Compact summary",
            "project": {"id":"10001","self":"https://acme.invalid/rest/api/3/project/10001"}
        }
    })
}

fn reject_issue_page(body: &[u8]) {
    assert_eq!(
        inspect_issue_wire_for_test(body, ORIGIN)
            .unwrap_err()
            .category(),
        ErrorCategory::SourceUnavailable
    );
}

fn reject_issue(row: Value) {
    reject_issue_page(json!({"issues":[row]}).to_string().as_bytes());
}

fn compact_issue_authority_and_presence_matrix() {
    let control =
        inspect_issue_wire_for_test(json!({"issues":[issue()]}).to_string().as_bytes(), ORIGIN)
            .unwrap();
    assert_eq!(control.ids, ["20001"]);
    assert!(control.rendered.contains("](jira://test/issues/20001)"));
    assert!(control.rendered.contains("](jira://test/projects/10001)"));
    let mut representations = std::collections::BTreeSet::from([control.rendered.clone()]);
    for status in [
        Value::Null,
        json!({}),
        json!({"name":null}),
        json!({"name":""}),
        json!({"name":"Open"}),
    ] {
        let mut row = issue();
        row["fields"]["status"] = status;
        let result =
            inspect_issue_wire_for_test(json!({"issues":[row]}).to_string().as_bytes(), ORIGIN)
                .unwrap();
        assert!(
            representations.insert(result.rendered),
            "status presence was collapsed"
        );
    }
    for (parent, field) in [
        ("", "id"),
        ("", "key"),
        ("", "self"),
        ("", "fields"),
        ("/fields", "summary"),
        ("/fields", "project"),
        ("/fields/project", "id"),
        ("/fields/project", "self"),
    ] {
        let mut row = issue();
        row.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(field);
        reject_issue(row);
        for invalid in [Value::Null, json!(42), json!([])] {
            let mut row = issue();
            row.pointer_mut(parent).unwrap()[field] = invalid;
            reject_issue(row);
        }
    }
    for invalid in [
        json!(true),
        json!("Open"),
        json!([]),
        json!({"name":false}),
        json!({"name":{}}),
    ] {
        let mut row = issue();
        row["fields"]["status"] = invalid;
        reject_issue(row);
    }
    for invalid in ["", ".", "..", "new", "TEAM/1", "TEAM\n1"] {
        let mut row = issue();
        row["key"] = json!(invalid);
        reject_issue(row);
    }
    for path in ["/id", "/fields/project/id"] {
        for invalid in ["", "0", "01", "-1", "abc", &"1".repeat(65)] {
            let mut row = issue();
            *row.pointer_mut(path).unwrap() = json!(invalid);
            reject_issue(row);
        }
    }
    for path in ["/self", "/fields/project/self"] {
        let original = issue().pointer(path).unwrap().as_str().unwrap().to_owned();
        for invalid in [
            original.replace("acme.invalid", "foreign.invalid"),
            original.replace("acme.invalid", "user:secret@acme.invalid"),
            original.replace("/rest/api/3/", "/rest/api/2/"),
            original.replace("20001", "20002").replace("10001", "10002"),
            original
                .replace("/issue/", "/project/")
                .replace("/project/10001", "/issue/10001"),
            format!("{original}?x=1"),
            format!("{original}#fragment"),
        ] {
            let mut row = issue();
            *row.pointer_mut(path).unwrap() = json!(invalid);
            reject_issue(row);
        }
    }
    for id in ["18446744073709551616".to_owned(), "9".repeat(64)] {
        let mut row = issue();
        row["id"] = json!(id);
        row["self"] = json!(format!("https://acme.invalid/rest/api/3/issue/{id}"));
        row["fields"]["project"]["id"] = json!(id);
        row["fields"]["project"]["self"] =
            json!(format!("https://acme.invalid/rest/api/3/project/{id}"));
        row["fields"]["summary"] = json!("");
        let result =
            inspect_issue_wire_for_test(json!({"issues":[row]}).to_string().as_bytes(), ORIGIN)
                .unwrap();
        assert_eq!(result.ids.as_slice(), std::slice::from_ref(&id));
        assert!(
            result
                .rendered
                .contains(&format!("](jira://test/projects/{id})"))
        );
    }
    let body = json!({"issues":[issue()]}).to_string();
    reject_issue_page(format!("{body} trailing").as_bytes());
    for (needle, replacement) in [
        ("\"issues\":", "\"issues\":[],\"issues\":"),
        ("\"summary\":", "\"summary\":\"other\",\"summary\":"),
        ("\"project\":", "\"unknown\":{\"a\":1,\"a\":2},\"project\":"),
    ] {
        reject_issue_page(body.replacen(needle, replacement, 1).as_bytes());
    }
    reject_issue_page(
        body.replacen(
            "\"summary\":",
            "\"status\":{\"name\":\"Open\",\"name\":\"Closed\"},\"summary\":",
            1,
        )
        .as_bytes(),
    );
    reject_issue_page(json!({"issues":[issue(), issue()]}).to_string().as_bytes());
    let mut unknown = issue();
    unknown["unknown"] = json!({"valid":[1,null,false]});
    let result =
        inspect_issue_wire_for_test(json!({"issues":[unknown]}).to_string().as_bytes(), ORIGIN)
            .unwrap();
    assert_eq!(result.rendered, control.rendered);
}

#[test]
fn issue_pages_require_authoritative_token_state() {
    for rows in [json!([]), json!([issue()])] {
        for is_last in [None, Some(true)] {
            let mut body = json!({"issues":rows});
            if let Some(is_last) = is_last {
                body["isLast"] = json!(is_last);
            }
            let result = inspect_issue_wire_for_test(body.to_string().as_bytes(), ORIGIN).unwrap();
            assert_eq!(result.next_token, None);
        }
        for token in ["opaque token + / = 雪", " ", "0001"] {
            for is_last in [None, Some(false)] {
                let mut body = json!({"issues":rows, "nextPageToken":token});
                if let Some(is_last) = is_last {
                    body["isLast"] = json!(is_last);
                }
                let result =
                    inspect_issue_wire_for_test(body.to_string().as_bytes(), ORIGIN).unwrap();
                assert_eq!(result.next_token.as_deref(), Some(token));
            }
        }
    }
    for body in [
        json!({"issues":[],"isLast":false}),
        json!({"issues":[],"isLast":true,"nextPageToken":"next"}),
        json!({"issues":[],"nextPageToken":""}),
        json!({"issues":[],"nextPageToken":null}),
        json!({"issues":[],"nextPageToken":3}),
        json!({"issues":[],"isLast":null}),
        json!({"issues":[],"isLast":"true"}),
        json!({}),
        json!({"issues":null}),
        json!({"issues":{}}),
        json!({"issues":[null]}),
    ] {
        reject_issue_page(body.to_string().as_bytes());
    }
    reject_issue_page(br#"{"issues":[],"nextPageToken":"a","nextPageToken":"b"}"#);
}

#[test]
fn project_resource_publishes_stable_issue_navigation() {
    let result = inspect_project_wire_for_test(
        project().to_string().as_bytes(),
        ORIGIN,
        None,
        Some("10001"),
    )
    .unwrap();
    let target = "jira://test/projects/10001/issues";
    assert!(
        result.projects[0]
            .rendered
            .contains(&format!("]({target})"))
    );
    let reference = resourcefs_core::PathReference::parse(target).unwrap();
    assert_eq!(reference.requested(), target);
}
