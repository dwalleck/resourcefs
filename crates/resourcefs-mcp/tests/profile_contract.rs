use std::{fs, path::Path, time::Duration};

use resourcefs_core::{
    DiscoveryLimitInput, MAX_ARTIFACT_BYTES, MAX_DISCOVERY_RESULTS, MAX_IMAGE_BYTES,
    MAX_SESSION_BYTES, MAX_TEXT_BYTES, MAX_TEXT_COLUMNS, MAX_TEXT_LINES, ServerLimits,
    ServerLimitsInput, StorageLimitInput, TextLimitInput,
};
use resourcefs_mcp::{MAX_ALLOWLIST_ENTRIES, MAX_PROFILE_BYTES, ProfileDocument, ProfileErrorKind};
use resourcefs_sources::{
    MAX_COMMAND_ARGUMENT_BYTES, MAX_COMMAND_ARGUMENTS, MAX_COMMAND_ENVIRONMENT_ENTRIES,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusRow {
    name: String,
    profile: Value,
    accepted: bool,
    error: Option<ProfileErrorKind>,
}

fn parse(value: &Value) -> Result<ProfileDocument, resourcefs_mcp::ProfileError> {
    let fixture = local_fixture();
    parse_in(value, fixture.path())
}

fn parse_in(value: &Value, base: &Path) -> Result<ProfileDocument, resourcefs_mcp::ProfileError> {
    ProfileDocument::from_slice_in(
        &serde_json::to_vec(value).expect("serialize hand-authored profile fixture"),
        base,
    )
}

/// A configuration base containing every path the valid local fixtures name.
fn local_fixture() -> TempDir {
    let directory = tempdir().expect("local profile fixture tempdir");
    let base = directory.path();
    for child in [
        "skills",
        "skills/skill-0",
        "notes",
        "vault",
        "vaults",
        "vaults/vault-0",
    ] {
        fs::create_dir(base.join(child)).expect("local directory fixture");
    }
    fs::create_dir(base.join("rules")).expect("rules directory fixture");
    fs::write(base.join("rules/main.json"), "{}").expect("rules manifest fixture");
    fs::write(base.join("rules/rule-0.json"), "{}").expect("rule manifest fixture");
    fs::create_dir(base.join("memory")).expect("memory directory fixture");
    fs::write(base.join("memory/memory-0"), "").expect("memory root fixture");
    fs::create_dir(base.join("agents")).expect("agents directory fixture");
    fs::write(base.join("agents/export.json"), "[]").expect("agent export fixture");
    fs::write(base.join("agents/export-0.json"), "[]").expect("agent export fixture");
    directory
}

/// A base holding `count` kind-appropriate local entries for cardinality rows.
fn cardinality_fixture(kind: &str, count: usize) -> TempDir {
    let directory = tempdir().expect("cardinality fixture tempdir");
    let base = directory.path();
    let present = count.min(MAX_ALLOWLIST_ENTRIES);
    match kind {
        "skills" | "vault" => {
            let parent = if kind == "skills" { "skills" } else { "vaults" };
            let stem = if kind == "skills" { "skill" } else { "vault" };
            fs::create_dir(base.join(parent)).expect("cardinality parent fixture");
            for index in 0..present {
                fs::create_dir(base.join(format!("{parent}/{stem}-{index}")))
                    .expect("cardinality directory fixture");
            }
        }
        "rules" | "agentExport" | "memory" => {
            let parent = match kind {
                "rules" => "rules",
                "agentExport" => "agents",
                _ => "memory",
            };
            fs::create_dir(base.join(parent)).expect("cardinality parent fixture");
            for index in 0..present {
                let name = match kind {
                    "rules" => format!("{parent}/rule-{index}.json"),
                    "agentExport" => format!("{parent}/export-{index}.json"),
                    _ => format!("{parent}/memory-{index}"),
                };
                fs::write(base.join(name), "").expect("cardinality file fixture");
            }
        }
        _ => {}
    }
    directory
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
            "skills" => json!(format!("skills/skill-{index}")),
            "rules" => json!(format!("rules/rule-{index}.json")),
            "agentExport" => json!(format!("agents/export-{index}.json")),
            "memory" => json!({
                "name":format!("memory-{index}"),
                "path":format!("memory/memory-{index}")
            }),
            "vault" => json!({
                "name":format!("vault-{index}"),
                "path":format!("vaults/vault-{index}")
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
fn nested_limits_map_to_the_core_aggregate_without_clamping() {
    let omitted = parse(&json!({"schemaVersion":1}))
        .expect("omitted limits")
        .server_limits();
    let empty = parse(&json!({
        "schemaVersion":1,
        "limits":{"text":{},"discovery":{},"storage":{}}
    }))
    .expect("empty limit groups")
    .server_limits();
    assert_eq!(omitted, ServerLimits::default());
    assert_eq!(empty, omitted);

    for (field, maximum) in [
        ("text.bytes", MAX_TEXT_BYTES),
        ("text.lines", MAX_TEXT_LINES),
        ("text.columns", MAX_TEXT_COLUMNS),
        ("discovery.searchMatches", MAX_DISCOVERY_RESULTS),
        ("discovery.globEntries", MAX_DISCOVERY_RESULTS),
        ("discovery.listingEntries", MAX_DISCOVERY_RESULTS),
        ("imageBytes", MAX_IMAGE_BYTES),
        ("storage.objectBytes", MAX_ARTIFACT_BYTES),
        ("storage.sessionBytes", MAX_SESSION_BYTES),
    ] {
        for signed_value in [-1_i64, 0, maximum as i64, maximum as i64 + 1] {
            let profile_result = parse(&profile_with_limit(field, signed_value));
            let direct_result = usize::try_from(signed_value)
                .map_err(|_| ())
                .and_then(|value| {
                    ServerLimits::new(input_with_limit(field, value)).map_err(|_| ())
                });
            assert_eq!(
                profile_result.is_ok(),
                direct_result.is_ok(),
                "{field}={signed_value}"
            );
            match (profile_result, direct_result) {
                (Ok(profile), Ok(direct)) => {
                    assert_eq!(profile.server_limits(), direct, "{field}={signed_value}");
                }
                (Err(profile_error), Err(())) => {
                    assert!(
                        profile_error.to_string().contains(field),
                        "{field}={signed_value} diagnostic must identify the field: {profile_error}"
                    );
                }
                _ => panic!("{field}={signed_value} profile/core acceptance diverged"),
            }
        }
    }

    for (object_bytes, session_bytes, accepted) in [(1, 2, true), (2, 2, true), (2, 1, false)] {
        let profile = json!({
            "schemaVersion":1,
            "limits":{"storage":{
                "objectBytes":object_bytes,
                "sessionBytes":session_bytes
            }}
        });
        assert_eq!(
            parse(&profile).is_ok(),
            accepted,
            "objectBytes={object_bytes}, sessionBytes={session_bytes}"
        );
    }

    for (value, accepted) in [(-1_i64, false), (0, false), (32, true), (33, false)] {
        let result = parse(&json!({
            "schemaVersion":1,
            "limits":{"processConcurrency":value}
        }));
        assert_eq!(result.is_ok(), accepted, "processConcurrency={value}");
        if let Ok(profile) = result {
            assert_eq!(profile.process_concurrency(), value as usize);
        }
    }
}

#[test]
fn session_configuration_resolves_paths_and_validates_the_signed_ttl_interval() {
    let fixture = local_fixture();
    let expected_root = fixture
        .path()
        .canonicalize()
        .expect("canonical profile directory");
    // A file-URL round trip normalizes Windows verbatim prefixes independently of profile code.
    let expected_root = url::Url::from_directory_path(expected_root)
        .expect("canonical directory URI")
        .to_file_path()
        .expect("native directory path");
    let relative = parse_in(
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"cache","retentionTtlSeconds":0}
        }),
        fixture.path(),
    )
    .expect("relative session configuration");
    assert_eq!(
        relative.session_storage_config().cache_root(),
        expected_root.join("cache")
    );
    assert_eq!(
        relative.session_storage_config().retention_ttl(),
        Duration::ZERO
    );

    let absolute_cache = tempdir().expect("absolute session cache");
    let absolute = parse_in(
        &json!({
            "schemaVersion":1,
            "session":{
                "cacheDirectory":absolute_cache.path(),
                "retentionTtlSeconds":86_400
            }
        }),
        fixture.path(),
    )
    .expect("absolute session configuration");
    assert_eq!(
        absolute.session_storage_config().cache_root(),
        absolute_cache.path()
    );
    assert_eq!(
        absolute.session_storage_config().retention_ttl(),
        Duration::from_secs(86_400)
    );

    for (seconds, accepted) in [(-1_i64, false), (0, true), (86_400, true), (86_401, false)] {
        let result = parse_in(
            &json!({
                "schemaVersion":1,
                "session":{"cacheDirectory":"cache","retentionTtlSeconds":seconds}
            }),
            fixture.path(),
        );
        assert_eq!(result.is_ok(), accepted, "retentionTtlSeconds={seconds}");
        if let Err(error) = result {
            assert!(
                error.to_string().contains("session.retentionTtlSeconds"),
                "retentionTtlSeconds={seconds} diagnostic must identify the field: {error}"
            );
        }
    }

    let empty_cache = parse_in(
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":""}
        }),
        fixture.path(),
    )
    .expect_err("empty cacheDirectory");
    assert!(empty_cache.to_string().contains("session.cacheDirectory"));
}

#[test]
fn logging_configuration_validates_the_signed_rotation_and_retention_intervals() {
    let fixture = local_fixture();
    let absolute_log = fixture.path().join("absolute.log");
    for profile in [
        json!({"schemaVersion":1}),
        json!({"schemaVersion":1,"logging":{}}),
        json!({"schemaVersion":1,"logging":{"level":"error","destination":{"kind":"stderr"}}}),
        json!({"schemaVersion":1,"logging":{"level":"warn","destination":{"kind":"stderr"}}}),
        json!({"schemaVersion":1,"logging":{"level":"info","destination":{"kind":"stderr"}}}),
        json!({"schemaVersion":1,"logging":{"level":"debug","destination":{"kind":"stderr"}}}),
        json!({"schemaVersion":1,"logging":{"destination":{"kind":"file","path":"relative.log","rotationBytes":0,"retainFiles":1}}}),
        json!({"schemaVersion":1,"logging":{"destination":{"kind":"file","path":absolute_log,"rotationBytes":10_485_760,"retainFiles":10}}}),
    ] {
        parse_in(&profile, fixture.path())
            .unwrap_or_else(|error| panic!("valid logging profile must decode: {error}"));
    }

    for (field, value, expected_kind) in [
        ("rotationBytes", -1_i64, ProfileErrorKind::LimitExceeded),
        ("rotationBytes", 10_485_761, ProfileErrorKind::LimitExceeded),
        ("retainFiles", -1, ProfileErrorKind::LimitExceeded),
        ("retainFiles", 0, ProfileErrorKind::LimitExceeded),
        ("retainFiles", 11, ProfileErrorKind::LimitExceeded),
    ] {
        let mut profile = json!({
            "schemaVersion":1,
            "logging":{
                "destination":{
                    "kind":"file",
                    "path":"resourcefs.log"
                }
            }
        });
        profile["logging"]["destination"][field] = json!(value);
        let result = parse_in(&profile, fixture.path());
        let error = result.expect_err("invalid logging limit must fail");
        assert_eq!(error.kind(), expected_kind, "{field}={value}: {error}");
        assert!(
            error.to_string().contains(field),
            "{field}={value} diagnostic must identify the field: {error}"
        );
    }

    let empty_path = parse_in(
        &json!({
            "schemaVersion":1,
            "logging":{"destination":{"kind":"file","path":""}}
        }),
        fixture.path(),
    )
    .expect_err("empty logging path");
    assert_eq!(empty_path.kind(), ProfileErrorKind::InvalidProfile);
    assert!(empty_path.to_string().contains("logging.destination.path"));
}

fn profile_with_limit(field: &str, value: i64) -> Value {
    let limits = match field {
        "text.bytes" => json!({"text":{"bytes":value}}),
        "text.lines" => json!({"text":{"lines":value}}),
        "text.columns" => json!({"text":{"columns":value}}),
        "discovery.searchMatches" => json!({"discovery":{"searchMatches":value}}),
        "discovery.globEntries" => json!({"discovery":{"globEntries":value}}),
        "discovery.listingEntries" => json!({"discovery":{"listingEntries":value}}),
        "imageBytes" => json!({"imageBytes":value}),
        "storage.objectBytes" => json!({"storage":{"objectBytes":value}}),
        "storage.sessionBytes" => json!({"storage":{"sessionBytes":value}}),
        _ => panic!("unknown limit field {field}"),
    };
    json!({"schemaVersion":1,"limits":limits})
}

fn input_with_limit(field: &str, value: usize) -> ServerLimitsInput {
    match field {
        "text.bytes" => ServerLimitsInput {
            text: TextLimitInput {
                bytes: Some(value),
                ..TextLimitInput::default()
            },
            ..ServerLimitsInput::default()
        },
        "text.lines" => ServerLimitsInput {
            text: TextLimitInput {
                lines: Some(value),
                ..TextLimitInput::default()
            },
            ..ServerLimitsInput::default()
        },
        "text.columns" => ServerLimitsInput {
            text: TextLimitInput {
                columns: Some(value),
                ..TextLimitInput::default()
            },
            ..ServerLimitsInput::default()
        },
        "discovery.searchMatches" => ServerLimitsInput {
            discovery: DiscoveryLimitInput {
                search_matches: Some(value),
                ..DiscoveryLimitInput::default()
            },
            ..ServerLimitsInput::default()
        },
        "discovery.globEntries" => ServerLimitsInput {
            discovery: DiscoveryLimitInput {
                glob_entries: Some(value),
                ..DiscoveryLimitInput::default()
            },
            ..ServerLimitsInput::default()
        },
        "discovery.listingEntries" => ServerLimitsInput {
            discovery: DiscoveryLimitInput {
                listing_entries: Some(value),
                ..DiscoveryLimitInput::default()
            },
            ..ServerLimitsInput::default()
        },
        "imageBytes" => ServerLimitsInput {
            image_bytes: Some(value),
            ..ServerLimitsInput::default()
        },
        "storage.objectBytes" => ServerLimitsInput {
            storage: StorageLimitInput {
                object_bytes: Some(value),
                ..StorageLimitInput::default()
            },
            ..ServerLimitsInput::default()
        },
        "storage.sessionBytes" => ServerLimitsInput {
            storage: StorageLimitInput {
                session_bytes: Some(value),
                ..StorageLimitInput::default()
            },
            ..ServerLimitsInput::default()
        },
        _ => panic!("unknown limit field {field}"),
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
        "limits":{"text":{},"discovery":{},"storage":{}},
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
        "text",
        "discovery",
        "imageBytes",
        "storage",
        "processConcurrency",
    ] {
        cases.push((
            format!("limits.{field}"),
            with_null(base.clone(), "/limits", field),
        ));
    }
    for field in ["bytes", "lines", "columns"] {
        cases.push((
            format!("limits.text.{field}"),
            with_null(base.clone(), "/limits/text", field),
        ));
    }
    for field in ["searchMatches", "globEntries", "listingEntries"] {
        cases.push((
            format!("limits.discovery.{field}"),
            with_null(base.clone(), "/limits/discovery", field),
        ));
    }
    for field in ["objectBytes", "sessionBytes"] {
        cases.push((
            format!("limits.storage.{field}"),
            with_null(base.clone(), "/limits/storage", field),
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

    let missing_parent = tempdir().expect("missing base parent");
    let missing_base = missing_parent.path().join("missing");
    assert_eq!(
        ProfileDocument::from_slice_in(&over, &missing_base)
            .expect_err("profile ceiling must fail before base-directory access")
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
        for count in [0, 1, 4_096, 4_097] {
            let accepted = matches!(count, 1 | 4_096);
            let profile = json!({
                "schemaVersion":1,
                "sources":[source_with_entries(kind, count)]
            });
            let fixture = cardinality_fixture(kind, count);
            let result = parse_in(&profile, fixture.path());
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
fn network_source_configuration_matches_constructors() {
    let https = |origins: Value| source("https", json!({"origins":origins}));
    let github = |repositories: Value| {
        source(
            "github",
            json!({
                "allowPrivateNetwork":false,
                "credential":{"kind":"environment","name":"GITHUB_TOKEN"},
                "repositories":repositories
            }),
        )
    };
    let ssh = |hosts: Value| source("ssh", json!({"command":{"argv":["ssh"]},"hosts":hosts}));
    let downstream = |servers: Value| source("downstreamMcp", json!({"servers":servers}));

    for (name, source, accepted) in [
        (
            "https-valid",
            https(json!([{
                "baseUrl":"https://example.test/api",
                "allowPrivateNetwork":false,
                "credential":{
                    "header":"Authorization",
                    "scheme":"Bearer",
                    "secret":{"kind":"environment","name":"TOKEN"}
                }
            }])),
            true,
        ),
        (
            "https-userinfo",
            https(json!([{
                "baseUrl":"https://user@example.test/api",
                "allowPrivateNetwork":false
            }])),
            false,
        ),
        (
            "https-overlap",
            https(json!([
                {"baseUrl":"https://example.test/api","allowPrivateNetwork":false},
                {"baseUrl":"https://example.test/api/v1","allowPrivateNetwork":false}
            ])),
            false,
        ),
        (
            "https-routing-header",
            https(json!([{
                "baseUrl":"https://example.test/api",
                "allowPrivateNetwork":false,
                "credential":{
                    "header":"Host",
                    "secret":{"kind":"environment","name":"TOKEN"}
                }
            }])),
            false,
        ),
        (
            "github-valid",
            github(json!([{"name":"owner/repository"}])),
            true,
        ),
        (
            "github-duplicate",
            github(json!([
                {"name":"Owner/Repository"},
                {"name":"owner/repository"}
            ])),
            false,
        ),
        (
            "github-recursive-secret-helper",
            source(
                "github",
                json!({
                    "allowPrivateNetwork":false,
                    "credential":{
                        "kind":"command",
                        "command":{
                            "argv":["helper"],
                            "environment":{
                                "TOKEN":{
                                    "kind":"secret",
                                    "secret":{"kind":"environment","name":"TOKEN"}
                                }
                            }
                        }
                    },
                    "repositories":[{"name":"owner/repository"}]
                }),
            ),
            false,
        ),
        (
            "ssh-empty-command",
            source(
                "ssh",
                json!({
                    "command":{"argv":[]},
                    "hosts":[{"alias":"host","remoteRoots":["/srv"]}]
                }),
            ),
            false,
        ),
        (
            "ssh-invalid-environment-name",
            source(
                "ssh",
                json!({
                    "command":{
                        "argv":["ssh"],
                        "environment":{"BAD=NAME":{"kind":"literal","value":"x"}}
                    },
                    "hosts":[{"alias":"host","remoteRoots":["/srv"]}]
                }),
            ),
            false,
        ),
        (
            "ssh-case-colliding-environment-names",
            source(
                "ssh",
                json!({
                    "command":{
                        "argv":["ssh"],
                        "environment":{
                            "Path":{"kind":"literal","value":"one"},
                            "PATH":{"kind":"literal","value":"two"}
                        }
                    },
                    "hosts":[{"alias":"host","remoteRoots":["/srv"]}]
                }),
            ),
            false,
        ),
        (
            "ssh-valid",
            ssh(json!([{"alias":"host","remoteRoots":["/srv","/srv2"]}])),
            true,
        ),
        (
            "ssh-relative-root",
            ssh(json!([{"alias":"host","remoteRoots":["srv"]}])),
            false,
        ),
        (
            "downstream-stdio-valid",
            downstream(json!([{
                "id":"docs",
                "schemes":["docs"],
                "transport":{"kind":"stdio","command":{"argv":["docs-server"]}}
            }])),
            true,
        ),
        (
            "downstream-claim-collision",
            downstream(json!([
                {
                    "id":"one",
                    "schemes":["Docs"],
                    "transport":{"kind":"stdio","command":{"argv":["docs-server"]}}
                },
                {
                    "id":"two",
                    "schemes":["docs"],
                    "transport":{"kind":"stdio","command":{"argv":["docs-server"]}}
                }
            ])),
            false,
        ),
        (
            "downstream-built-in-claim",
            downstream(json!([{
                "id":"docs",
                "schemes":["artifact"],
                "transport":{"kind":"stdio","command":{"argv":["docs-server"]}}
            }])),
            false,
        ),
        (
            "downstream-http-userinfo",
            downstream(json!([{
                "id":"docs",
                "schemes":["docs"],
                "transport":{
                    "kind":"http",
                    "endpoint":"https://user@example.test/mcp",
                    "allowPrivateNetwork":false
                }
            }])),
            false,
        ),
    ] {
        let result = parse(&profile_with_source(source));
        assert_eq!(
            result.is_ok(),
            accepted,
            "network conversion row {name}: {result:?}"
        );
    }
}

#[test]
fn load_resolves_local_paths_beneath_the_profile_directory() {
    let directory = tempdir().expect("profile tempdir");
    fs::create_dir(directory.path().join("skills")).expect("skills fixture");
    let path = directory.path().join("profile.json");

    let valid =
        json!({"schemaVersion":1,"sources":[source("skills", json!({"roots":["skills"]}))]});
    fs::write(
        &path,
        serde_json::to_vec(&valid).expect("serialize profile"),
    )
    .expect("write valid profile");
    ProfileDocument::load(&path)
        .expect("relative local roots resolve beneath the profile directory");

    let missing = json!({"schemaVersion":1,"sources":[source("skills", json!({"roots":["missing-skills"]}))]});
    fs::write(
        &path,
        serde_json::to_vec(&missing).expect("serialize profile"),
    )
    .expect("write missing-root profile");
    let error = ProfileDocument::load(&path).expect_err("missing local root must fail");
    assert_eq!(error.kind(), ProfileErrorKind::InvalidProfile);
}

#[test]
fn from_slice_in_rejects_missing_and_non_directory_bases() {
    let directory = tempdir().expect("base tempdir");
    let missing = directory.path().join("missing");
    let error = ProfileDocument::from_slice_in(br#"{"schemaVersion":1}"#, &missing)
        .expect_err("missing base directory must fail");
    assert_eq!(error.kind(), ProfileErrorKind::Io);

    let file = directory.path().join("not-a-directory");
    fs::write(&file, "x").expect("file fixture");
    let error = ProfileDocument::from_slice_in(br#"{"schemaVersion":1}"#, &file)
        .expect_err("file base must fail");
    assert_eq!(error.kind(), ProfileErrorKind::Io);
}

#[test]
fn local_source_configuration_matches_constructors() {
    let fixture = local_fixture();
    let base = fixture.path();
    let outside = tempdir().expect("outside tempdir");
    fs::create_dir(outside.path().join("outside-dir")).expect("outside directory fixture");
    fs::write(outside.path().join("outside.txt"), "outside").expect("outside file fixture");
    let outside_dir = outside
        .path()
        .join("outside-dir")
        .to_string_lossy()
        .into_owned();
    let outside_file = outside
        .path()
        .join("outside.txt")
        .to_string_lossy()
        .into_owned();
    let contained_dir = base.join("skills").to_string_lossy().into_owned();

    let documents = |converters: Value| source("documents", json!({"converters":converters}));
    let skills = |roots: Value| source("skills", json!({"roots":roots}));
    let rules = |manifests: Value| source("rules", json!({"manifests":manifests}));
    let memory = |roots: Value| source("memory", json!({"roots":roots}));
    let vault = |vaults: Value| source("vault", json!({"vaults":vaults}));
    let agent_export = |manifests: Value| source("agentExport", json!({"manifests":manifests}));

    let rows: Vec<(String, Value, bool)> = vec![
        (
            "documents-valid-stdin".to_owned(),
            documents(json!([{
                "extensions":["md"],
                "input":"stdin",
                "command":{"argv":["converter"]}
            }])),
            true,
        ),
        (
            "documents-valid-path".to_owned(),
            documents(json!([{
                "extensions":["md"],
                "input":"path",
                "command":{"argv":["converter"]}
            }])),
            true,
        ),
        (
            "documents-duplicate-extension".to_owned(),
            documents(json!([
                {"extensions":["md"],"input":"stdin","command":{"argv":["converter"]}},
                {"extensions":["md"],"input":"path","command":{"argv":["converter"]}}
            ])),
            false,
        ),
        (
            "documents-uppercase-extension".to_owned(),
            documents(json!([{
                "extensions":["MD"],
                "input":"stdin",
                "command":{"argv":["converter"]}
            }])),
            false,
        ),
        (
            "documents-empty-extension".to_owned(),
            documents(json!([{
                "extensions":[""],
                "input":"stdin",
                "command":{"argv":["converter"]}
            }])),
            false,
        ),
        (
            "documents-unsafe-extension".to_owned(),
            documents(json!([{
                "extensions":["md.txt"],
                "input":"stdin",
                "command":{"argv":["converter"]}
            }])),
            false,
        ),
        (
            "documents-empty-command".to_owned(),
            documents(json!([{
                "extensions":["md"],
                "input":"stdin",
                "command":{"argv":[]}
            }])),
            false,
        ),
        (
            "documents-empty-converters".to_owned(),
            documents(json!([])),
            false,
        ),
        ("skills-valid".to_owned(), skills(json!(["skills"])), true),
        (
            "skills-absolute-contained".to_owned(),
            skills(json!([contained_dir])),
            true,
        ),
        (
            "skills-missing".to_owned(),
            skills(json!(["skills/missing"])),
            false,
        ),
        (
            "skills-file-target".to_owned(),
            skills(json!(["rules/main.json"])),
            false,
        ),
        (
            "skills-relative-escape".to_owned(),
            skills(json!(["../"])),
            false,
        ),
        (
            "skills-absolute-escape".to_owned(),
            skills(json!([outside_dir.clone()])),
            false,
        ),
        (
            "skills-duplicate-alias".to_owned(),
            skills(json!(["skills", "./skills"])),
            false,
        ),
        (
            "rules-valid".to_owned(),
            rules(json!(["rules/main.json"])),
            true,
        ),
        (
            "rules-missing".to_owned(),
            rules(json!(["rules/missing.json"])),
            false,
        ),
        (
            "rules-directory-target".to_owned(),
            rules(json!(["skills"])),
            false,
        ),
        (
            "rules-absolute-escape".to_owned(),
            rules(json!([outside_file.clone()])),
            false,
        ),
        (
            "rules-duplicate-alias".to_owned(),
            rules(json!(["rules/main.json", "./rules/main.json"])),
            false,
        ),
        (
            "memory-valid-file-and-directory".to_owned(),
            memory(json!([
                {"name":"notes","path":"notes"},
                {"name":"manifest","path":"rules/main.json"}
            ])),
            true,
        ),
        (
            "memory-duplicate-name".to_owned(),
            memory(json!([
                {"name":"dup","path":"notes"},
                {"name":"dup","path":"memory/memory-0"}
            ])),
            false,
        ),
        (
            "memory-duplicate-target".to_owned(),
            memory(json!([
                {"name":"one","path":"notes"},
                {"name":"two","path":"notes"}
            ])),
            false,
        ),
        (
            "memory-missing".to_owned(),
            memory(json!([{"name":"missing","path":"memory/missing"}])),
            false,
        ),
        (
            "memory-absolute-escape".to_owned(),
            memory(json!([{"name":"escape","path":outside_file}])),
            false,
        ),
        (
            "memory-invalid-name".to_owned(),
            memory(json!([{"name":"bad/name","path":"notes"}])),
            false,
        ),
        (
            "vault-valid".to_owned(),
            vault(json!([{"name":"personal","path":"vault"}])),
            true,
        ),
        (
            "vault-file-target".to_owned(),
            vault(json!([{"name":"bad","path":"rules/main.json"}])),
            false,
        ),
        (
            "vault-duplicate-target".to_owned(),
            vault(json!([
                {"name":"one","path":"vault"},
                {"name":"two","path":"vaults/../vault"}
            ])),
            false,
        ),
        (
            "vault-missing".to_owned(),
            vault(json!([{"name":"missing","path":"vaults/missing"}])),
            false,
        ),
        (
            "vault-absolute-escape".to_owned(),
            vault(json!([{"name":"escape","path":outside_dir}])),
            false,
        ),
        (
            "agent-export-valid".to_owned(),
            agent_export(json!(["agents/export.json"])),
            true,
        ),
        (
            "agent-export-missing".to_owned(),
            agent_export(json!(["agents/missing.json"])),
            false,
        ),
        (
            "agent-export-directory-target".to_owned(),
            agent_export(json!(["skills"])),
            false,
        ),
        (
            "agent-export-duplicate-alias".to_owned(),
            agent_export(json!(["agents/export.json", "./agents/export.json"])),
            false,
        ),
    ];

    for (name, source, accepted) in rows {
        let result = parse_in(&profile_with_source(source), base);
        assert_eq!(
            result.is_ok(),
            accepted,
            "local conversion row {name}: {result:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn local_source_symlink_aliases_and_escapes_are_rejected() {
    use std::os::unix::fs::symlink;

    let fixture = local_fixture();
    let base = fixture.path();
    let outside = tempdir().expect("outside tempdir");
    fs::create_dir(outside.path().join("outside-dir")).expect("outside directory fixture");
    symlink(base.join("skills"), base.join("skills-alias")).expect("contained alias symlink");
    symlink(
        outside.path().join("outside-dir"),
        base.join("skills-escape"),
    )
    .expect("escaping symlink");

    let skills = |roots: Value| source("skills", json!({"roots":roots}));
    for (name, roots) in [
        (
            "alias-collides-with-target",
            json!(["skills", "skills-alias"]),
        ),
        ("symlink-escape", json!(["skills-escape"])),
    ] {
        let result = parse_in(&profile_with_source(skills(roots)), base);
        assert!(result.is_err(), "symlink row {name}: {result:?}");
    }
}

#[test]
fn command_cardinality_matches_typed_configuration() {
    let command_source = |argv: Value, environment: Option<Value>| {
        let mut command = json!({"argv":argv});
        if let Some(environment) = environment {
            command["environment"] = environment;
        }
        source(
            "downstreamMcp",
            json!({
                "servers":[{
                    "id":"docs",
                    "schemes":["docs"],
                    "transport":{"kind":"stdio","command":command}
                }]
            }),
        )
    };

    for (name, argv, accepted) in [
        (
            "exact-argument-count",
            json!(vec!["x"; MAX_COMMAND_ARGUMENTS]),
            true,
        ),
        (
            "over-argument-count",
            json!(vec!["x"; MAX_COMMAND_ARGUMENTS + 1]),
            false,
        ),
        (
            "exact-argument-bytes",
            json!(["x".repeat(MAX_COMMAND_ARGUMENT_BYTES)]),
            true,
        ),
        (
            "over-argument-bytes",
            json!(["x".repeat(MAX_COMMAND_ARGUMENT_BYTES + 1)]),
            false,
        ),
    ] {
        let result = parse(&profile_with_source(command_source(argv, None)));
        assert_eq!(
            result.is_ok(),
            accepted,
            "command argv row {name}: {result:?}"
        );
    }

    for (name, count, accepted) in [
        ("empty-environment", 0, true),
        ("exact-environment", MAX_COMMAND_ENVIRONMENT_ENTRIES, true),
        (
            "over-environment",
            MAX_COMMAND_ENVIRONMENT_ENTRIES + 1,
            false,
        ),
    ] {
        let environment = (0..count)
            .map(|index| {
                (
                    format!("NAME_{index}"),
                    json!({"kind":"inherit","name":"PATH"}),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let result = parse(&profile_with_source(command_source(
            json!(["server"]),
            Some(Value::Object(environment)),
        )));
        assert_eq!(
            result.is_ok(),
            accepted,
            "command environment row {name}: {result:?}"
        );
    }
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
