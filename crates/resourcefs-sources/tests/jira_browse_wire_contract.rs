use resourcefs_core::ErrorCategory;
use resourcefs_sources::inspect_project_wire_for_test;
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
