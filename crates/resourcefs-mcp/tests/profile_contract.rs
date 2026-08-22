use std::fs;

use resourcefs_mcp::{MAX_ALLOWLIST_ENTRIES, MAX_PROFILE_BYTES, ProfileDocument, ProfileErrorKind};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::tempdir;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusRow {
    name: String,
    profile: Value,
    accepted: bool,
    error: Option<ProfileErrorKind>,
}

fn parse(value: &Value) -> Result<ProfileDocument, resourcefs_mcp::ProfileError> {
    ProfileDocument::from_slice(
        &serde_json::to_vec(value).expect("serialize hand-authored profile fixture"),
    )
}

fn corpus() -> Vec<CorpusRow> {
    serde_json::from_str(include_str!("fixtures/profile_corpus.json"))
        .expect("valid profile corpus")
}

fn with_null(mut profile: Value, parent_pointer: &str, field: &str) -> Value {
    profile
        .pointer_mut(parent_pointer)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("fixture parent {parent_pointer} must be an object"))
        .insert(field.to_owned(), Value::Null);
    profile
}

fn source(kind: &str, body: Value) -> Value {
    let mut object = body.as_object().expect("source body object").clone();
    object.insert("kind".to_owned(), Value::String(kind.to_owned()));
    object.insert("id".to_owned(), Value::String(format!("{kind}-source")));
    object.insert("required".to_owned(), Value::Bool(false));
    Value::Object(object)
}

fn minimal_sources() -> Vec<Value> {
    vec![
        source(
            "https",
            json!({"origins":[{"baseUrl":"https://example.test/api/","allowPrivateNetwork":false}]}),
        ),
        source(
            "github",
            json!({
                "allowPrivateNetwork": false,
                "credential":{"kind":"environment","name":"GITHUB_TOKEN"},
                "repositories":[{"name":"owner/repository"}]
            }),
        ),
        source(
            "ssh",
            json!({
                "command":{"argv":["ssh"]},
                "hosts":[{"alias":"example","remoteRoots":["/srv/repository"]}]
            }),
        ),
        source(
            "documents",
            json!({
                "converters":[{
                    "extensions":["md"],
                    "input":"stdin",
                    "command":{"argv":["converter"]}
                }]
            }),
        ),
        source("skills", json!({"roots":["skills"]})),
        source("rules", json!({"manifests":["rules/main.json"]})),
        source("memory", json!({"roots":[{"name":"notes","path":"notes"}]})),
        source(
            "vault",
            json!({"vaults":[{"name":"personal","path":"vault"}]}),
        ),
        source("agentExport", json!({"manifests":["agents/export.json"]})),
        source(
            "downstreamMcp",
            json!({
                "servers":[{
                    "id":"docs",
                    "schemes":["docs"],
                    "transport":{"kind":"stdio","command":{"argv":["docs-server"]}}
                }]
            }),
        ),
    ]
}

fn source_with_entries(kind: &str, count: usize) -> Value {
    let entries = match kind {
        "https" => {
            vec![json!({"baseUrl":"https://example.test/","allowPrivateNetwork":false}); count]
        }
        "github" => vec![json!({"name":"owner/repository"}); count],
        "ssh" => vec![json!({"alias":"host","remoteRoots":["/srv"]}); count],
        "documents" => vec![
            json!({
                "extensions":["txt"],
                "input":"stdin",
                "command":{"argv":["converter"]}
            });
            count
        ],
        "skills" | "rules" | "agentExport" => vec![json!("entry"); count],
        "memory" | "vault" => vec![json!({"name":"entry","path":"entry"}); count],
        "downstreamMcp" => vec![
            json!({
                "id":"server",
                "schemes":["example"],
                "transport":{"kind":"stdio","command":{"argv":["server"]}}
            });
            count
        ],
        _ => panic!("unknown fixture source kind {kind}"),
    };
    match kind {
        "https" => source(kind, json!({"origins":entries})),
        "github" => source(
            kind,
            json!({
                "allowPrivateNetwork":false,
                "credential":{"kind":"environment","name":"TOKEN"},
                "repositories":entries
            }),
        ),
        "ssh" => source(kind, json!({"command":{"argv":["ssh"]},"hosts":entries})),
        "documents" => source(kind, json!({"converters":entries})),
        "skills" | "memory" => source(kind, json!({"roots":entries})),
        "rules" | "agentExport" => source(kind, json!({"manifests":entries})),
        "vault" => source(kind, json!({"vaults":entries})),
        "downstreamMcp" => source(kind, json!({"servers":entries})),
        _ => unreachable!("fixture source kind checked above"),
    }
}

#[test]
fn accepts_valid_profiles() {
    for (name, profile) in [
        ("scratch-only", json!({"schemaVersion": 1})),
        (
            "empty-groups",
            json!({
                "schemaVersion":1,
                "workspace":{"roots":[]},
                "limits":{},
                "session":{},
                "logging":{},
                "sources":[]
            }),
        ),
        (
            "complete-source-catalog",
            json!({"schemaVersion":1,"sources":minimal_sources()}),
        ),
    ] {
        parse(&profile).unwrap_or_else(|error| panic!("{name} must be accepted: {error}"));
    }
}

#[test]
fn schema_matches_deserializer() {
    for row in corpus() {
        let result = parse(&row.profile);
        assert_eq!(result.is_ok(), row.accepted, "corpus row {}", row.name);
        if let Some(expected) = row.error {
            assert_eq!(
                result.expect_err("row must fail").kind(),
                expected,
                "corpus row {}",
                row.name
            );
        }
    }
}

#[test]
fn rejects_invalid_profiles() {
    for field in ["workspace", "limits", "session", "logging", "sources"] {
        let mut profile = json!({"schemaVersion":1});
        profile[field] = Value::Null;
        let error = parse(&profile).expect_err("present null optional group must fail");
        assert_eq!(
            error.kind(),
            ProfileErrorKind::InvalidProfile,
            "field {field}"
        );
    }

    let mut non_utf8 = br#"{"schemaVersion":1}"#.to_vec();
    non_utf8.push(0xff);
    assert_eq!(
        ProfileDocument::from_slice(&non_utf8)
            .expect_err("non-UTF-8 profile must fail")
            .kind(),
        ProfileErrorKind::InvalidProfile
    );
}

#[test]
fn rejects_explicit_null_at_every_optional_profile_field() {
    let base = json!({
        "schemaVersion":1,
        "workspace":{
            "roots":[{"id":"root","path":".","grants":{}}],
            "primaryRoot":"root",
            "backingPathVisibility":"hidden"
        },
        "limits":{},
        "session":{},
        "logging":{},
        "sources":minimal_sources()
    });
    let mut cases = Vec::new();
    for field in ["workspace", "limits", "session", "logging", "sources"] {
        cases.push((format!("root.{field}"), with_null(base.clone(), "", field)));
    }
    for field in ["roots", "primaryRoot", "backingPathVisibility"] {
        cases.push((
            format!("workspace.{field}"),
            with_null(base.clone(), "/workspace", field),
        ));
    }
    cases.push((
        "workspace.roots.grants".to_owned(),
        with_null(base.clone(), "/workspace/roots/0", "grants"),
    ));
    for field in ["create", "update", "delete"] {
        cases.push((
            format!("workspace.roots.grants.{field}"),
            with_null(base.clone(), "/workspace/roots/0/grants", field),
        ));
    }
    for field in [
        "textBytes",
        "textLines",
        "textColumns",
        "imageBytes",
        "objectBytes",
        "sessionBytes",
        "listingEntries",
        "processConcurrency",
    ] {
        cases.push((
            format!("limits.{field}"),
            with_null(base.clone(), "/limits", field),
        ));
    }
    for field in ["cacheDirectory", "retentionTtlSeconds"] {
        cases.push((
            format!("session.{field}"),
            with_null(base.clone(), "/session", field),
        ));
    }
    for field in ["level", "destination"] {
        cases.push((
            format!("logging.{field}"),
            with_null(base.clone(), "/logging", field),
        ));
    }
    let file_logging = json!({
        "schemaVersion":1,
        "logging":{"destination":{"kind":"file","path":"resourcefs.log"}}
    });
    for field in ["rotationBytes", "retainFiles"] {
        cases.push((
            format!("logging.destination.{field}"),
            with_null(file_logging.clone(), "/logging/destination", field),
        ));
    }
    for (index, source) in minimal_sources().into_iter().enumerate() {
        let kind = source["kind"].as_str().expect("source kind").to_owned();
        let profile = json!({"schemaVersion":1,"sources":[source]});
        cases.push((
            format!("sources[{index}].{kind}.grants"),
            with_null(profile, "/sources/0", "grants"),
        ));
    }
    cases.push((
        "https.origin.credential".to_owned(),
        with_null(
            json!({"schemaVersion":1,"sources":[source_with_entries("https", 1)]}),
            "/sources/0/origins/0",
            "credential",
        ),
    ));
    cases.push((
        "https.credential.scheme".to_owned(),
        with_null(
            json!({
                "schemaVersion":1,
                "sources":[source("https", json!({
                    "origins":[{
                        "baseUrl":"https://example.test/",
                        "allowPrivateNetwork":false,
                        "credential":{
                            "header":"Authorization",
                            "secret":{"kind":"environment","name":"TOKEN"}
                        }
                    }]
                }))]
            }),
            "/sources/0/origins/0/credential",
            "scheme",
        ),
    ));
    cases.push((
        "github.apiBaseUrl".to_owned(),
        with_null(
            json!({"schemaVersion":1,"sources":[source_with_entries("github", 1)]}),
            "/sources/0",
            "apiBaseUrl",
        ),
    ));
    cases.push((
        "github.repository.grants".to_owned(),
        with_null(
            json!({"schemaVersion":1,"sources":[source_with_entries("github", 1)]}),
            "/sources/0/repositories/0",
            "grants",
        ),
    ));
    cases.push((
        "ssh.command.environment".to_owned(),
        with_null(
            json!({"schemaVersion":1,"sources":[source_with_entries("ssh", 1)]}),
            "/sources/0/command",
            "environment",
        ),
    ));
    cases.push((
        "vault.vault.grants".to_owned(),
        with_null(
            json!({"schemaVersion":1,"sources":[source_with_entries("vault", 1)]}),
            "/sources/0/vaults/0",
            "grants",
        ),
    ));
    cases.push((
        "downstream.http.credential".to_owned(),
        with_null(
            json!({
                "schemaVersion":1,
                "sources":[source("downstreamMcp", json!({
                    "servers":[{
                        "id":"server",
                        "schemes":["example"],
                        "transport":{
                            "kind":"http",
                            "endpoint":"https://example.test/mcp",
                            "allowPrivateNetwork":false
                        }
                    }]
                }))]
            }),
            "/sources/0/servers/0/transport",
            "credential",
        ),
    ));

    for (name, profile) in cases {
        assert_eq!(
            parse(&profile).expect_err("explicit null must fail").kind(),
            ProfileErrorKind::InvalidProfile,
            "{name}"
        );
    }
}

#[test]
fn profile_byte_ceiling_is_exact_and_load_is_bounded() {
    let prefix = br#"{"schemaVersion":1}"#;
    let mut exact = prefix.to_vec();
    exact.resize(MAX_PROFILE_BYTES, b' ');
    ProfileDocument::from_slice(&exact).expect("exact profile byte ceiling");

    let mut over = exact.clone();
    over.push(b' ');
    assert_eq!(
        ProfileDocument::from_slice(&over)
            .expect_err("one byte over profile ceiling")
            .kind(),
        ProfileErrorKind::LimitExceeded
    );

    let directory = tempdir().expect("profile tempdir");
    let path = directory.path().join("profile.json");
    fs::write(&path, over).expect("write over-limit profile");
    assert_eq!(
        ProfileDocument::load(&path)
            .expect_err("bounded file load must reject cap+1")
            .kind(),
        ProfileErrorKind::LimitExceeded
    );
}

#[test]
fn nested_allowlist_cardinality_matrix() {
    assert_eq!(MAX_ALLOWLIST_ENTRIES, 4_096);
    for kind in [
        "https",
        "github",
        "ssh",
        "documents",
        "skills",
        "rules",
        "memory",
        "vault",
        "agentExport",
        "downstreamMcp",
    ] {
        for (count, accepted) in [(0, true), (1, true), (4_096, true), (4_097, false)] {
            let profile = json!({
                "schemaVersion":1,
                "sources":[source_with_entries(kind, count)]
            });
            let result = parse(&profile);
            assert_eq!(
                result.is_ok(),
                accepted,
                "{kind} list cardinality {count}: {result:?}"
            );
        }
    }
}
