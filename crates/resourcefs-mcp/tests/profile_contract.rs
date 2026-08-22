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
    let entries = (0..count)
        .map(|index| match kind {
            "https" => json!({
                "baseUrl":format!("https://host{index}.example.test/"),
                "allowPrivateNetwork":false
            }),
            "github" => json!({"name":format!("owner/repository-{index}")}),
            "ssh" => json!({
                "alias":format!("host-{index}"),
                "remoteRoots":[format!("/srv/root-{index}")]
            }),
            "documents" => json!({
                "extensions":[format!("ext{index}")],
                "input":"stdin",
                "command":{"argv":["converter"]}
            }),
            "skills" | "rules" | "agentExport" => json!(format!("entry-{index}")),
            "memory" | "vault" => json!({
                "name":format!("entry-{index}"),
                "path":format!("entry-{index}")
            }),
            "downstreamMcp" => json!({
                "id":format!("server-{index}"),
                "schemes":[format!("example{index}")],
                "transport":{"kind":"stdio","command":{"argv":["server"]}}
            }),
            _ => panic!("unknown fixture source kind {kind}"),
        })
        .collect::<Vec<_>>();
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
fn set_source_id(source: &mut Value, id: &str) {
    source
        .as_object_mut()
        .expect("source fixture must be an object")
        .insert("id".to_owned(), Value::String(id.to_owned()));
}

fn set_source_grants(source: &mut Value, grants: Value) {
    source
        .as_object_mut()
        .expect("source fixture must be an object")
        .insert("grants".to_owned(), grants);
}

fn profile_with_source(source: Value) -> Value {
    json!({"schemaVersion":1,"sources":[source]})
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

#[test]
fn grant_matrix() {
    let supported = [
        ("https", [false, false, false]),
        ("github", [true, true, false]),
        ("ssh", [false, false, false]),
        ("documents", [false, false, false]),
        ("skills", [true, true, true]),
        ("rules", [true, true, true]),
        ("memory", [false, false, false]),
        ("vault", [true, true, true]),
        ("agentExport", [false, false, false]),
        ("downstreamMcp", [false, false, false]),
    ];
    let operations = ["create", "update", "delete"];

    for (kind, expected) in supported {
        let source = source_with_entries(kind, 1);
        parse(&profile_with_source(source.clone()))
            .unwrap_or_else(|error| panic!("{kind}: absent grants must be accepted: {error}"));

        let mut false_grants = source.clone();
        set_source_grants(
            &mut false_grants,
            json!({"create":false,"update":false,"delete":false}),
        );
        parse(&profile_with_source(false_grants))
            .unwrap_or_else(|error| panic!("{kind}: false grants must be accepted: {error}"));

        for ((operation, accepted), index) in operations.into_iter().zip(expected).zip(0..) {
            let mut granted = source.clone();
            set_source_grants(&mut granted, json!({operation:true}));
            assert_eq!(
                parse(&profile_with_source(granted)).is_ok(),
                accepted,
                "{kind} operation {operation} at index {index}"
            );
        }
    }

    for (name, grants) in [
        ("null", json!({"create":null})),
        ("number", json!({"create":1})),
        ("string", json!({"create":"true"})),
        ("writable-shorthand", json!({"writable":true})),
    ] {
        let mut source = source_with_entries("skills", 1);
        set_source_grants(&mut source, grants);
        assert!(
            parse(&profile_with_source(source)).is_err(),
            "malformed grant row {name}"
        );
    }

    let mut github_subset = source_with_entries("github", 1);
    set_source_grants(&mut github_subset, json!({"create":true}));
    github_subset["repositories"][0]["grants"] = json!({"create":true});
    assert!(parse(&profile_with_source(github_subset)).is_ok());

    let mut github_superset = source_with_entries("github", 1);
    set_source_grants(&mut github_superset, json!({"create":true}));
    github_superset["repositories"][0]["grants"] = json!({"update":true});
    assert!(parse(&profile_with_source(github_superset)).is_err());

    let mut vault_subset = source_with_entries("vault", 1);
    set_source_grants(&mut vault_subset, json!({"update":true}));
    vault_subset["vaults"][0]["grants"] = json!({"update":true});
    assert!(parse(&profile_with_source(vault_subset)).is_ok());

    let mut vault_superset = source_with_entries("vault", 1);
    set_source_grants(&mut vault_superset, json!({"update":true}));
    vault_superset["vaults"][0]["grants"] = json!({"delete":true});
    assert!(parse(&profile_with_source(vault_superset)).is_err());
}

#[test]
fn source_catalog_matrix() {
    let kinds = [
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
    ];
    for kind in kinds {
        parse(&profile_with_source(source_with_entries(kind, 1)))
            .unwrap_or_else(|error| panic!("single {kind} source must be accepted: {error}"));
    }
    parse(&json!({"schemaVersion":1,"sources":minimal_sources()}))
        .expect("the complete ten-kind catalog must be accepted");

    let first = source_with_entries("https", 1);
    let mut duplicate_kind = source_with_entries("https", 1);
    set_source_id(&mut duplicate_kind, "second-https");
    assert!(parse(&json!({"schemaVersion":1,"sources":[first,duplicate_kind]})).is_err());

    let mut first = source_with_entries("https", 1);
    let mut second = source_with_entries("github", 1);
    set_source_id(&mut first, "shared");
    set_source_id(&mut second, "shared");
    assert!(parse(&json!({"schemaVersion":1,"sources":[first,second]})).is_err());

    let mut upper = source_with_entries("https", 1);
    let mut lower = source_with_entries("github", 1);
    set_source_id(&mut upper, "CaseSensitive");
    set_source_id(&mut lower, "casesensitive");
    parse(&json!({"schemaVersion":1,"sources":[upper,lower]}))
        .expect("source IDs are case-sensitive");

    for (name, id, accepted) in [
        ("empty", String::new(), false),
        ("one-byte", "a".to_owned(), true),
        ("valid-punctuation", "a.b_c-d9".to_owned(), true),
        ("invalid-first", "-source".to_owned(), false),
        ("invalid-rest", "source/name".to_owned(), false),
        ("unicode", "sourcé".to_owned(), false),
        ("exact-128", format!("a{}", "b".repeat(127)), true),
        ("over-128", format!("a{}", "b".repeat(128)), false),
    ] {
        let mut source = source_with_entries("https", 1);
        set_source_id(&mut source, &id);
        assert_eq!(
            parse(&profile_with_source(source)).is_ok(),
            accepted,
            "source ID row {name}"
        );
    }

    let roots = (0..256)
        .map(|index| json!({"id":format!("root-{index}"),"path":format!("root-{index}")}))
        .collect::<Vec<_>>();
    parse(&json!({"schemaVersion":1,"workspace":{"roots":roots}}))
        .expect("256 roots must be accepted at the catalog ceiling");
    let roots = (0..257)
        .map(|index| json!({"id":format!("root-{index}"),"path":format!("root-{index}")}))
        .collect::<Vec<_>>();
    assert!(parse(&json!({"schemaVersion":1,"workspace":{"roots":roots}})).is_err());

    let sources = (0..257)
        .map(|index| {
            let mut source = source_with_entries("https", 1);
            set_source_id(&mut source, &format!("source-{index}"));
            source
        })
        .collect::<Vec<_>>();
    let error = parse(&json!({"schemaVersion":1,"sources":sources}))
        .expect_err("257 sources must fail the defensive catalog ceiling");
    assert!(
        error.to_string().contains("sources"),
        "source ceiling must fail before duplicate-kind validation: {error}"
    );

    let mut claims = source_with_entries("downstreamMcp", 2);
    claims["servers"][0]["id"] = json!("first");
    claims["servers"][0]["schemes"] = json!(["Docs"]);
    claims["servers"][1]["id"] = json!("second");
    claims["servers"][1]["schemes"] = json!(["docs"]);
    assert!(
        parse(&profile_with_source(claims)).is_err(),
        "scheme claims must collide after lowercase normalization"
    );
}
