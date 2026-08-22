use std::process::Command;

use resourcefs_mcp::{ProfileDocument, profile_schema_json};
use serde_json::Value;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_resourcefs")
}

#[test]
fn schema_command_is_deterministic_and_stdout_clean() {
    let first = Command::new(binary())
        .arg("schema")
        .output()
        .expect("run resourcefs schema");
    let second = Command::new(binary())
        .arg("schema")
        .output()
        .expect("run resourcefs schema again");

    assert!(
        first.status.success(),
        "schema stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stderr, b"");
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(
        first.stdout,
        format!("{}\n", profile_schema_json()).as_bytes()
    );
    assert_eq!(
        first.stdout,
        include_bytes!("fixtures/server-profile-v1.schema.json")
    );
    assert_eq!(first.stdout.last(), Some(&b'\n'));
    assert_ne!(
        first.stdout.get(first.stdout.len().saturating_sub(2)),
        Some(&b'\n')
    );
}

#[test]
fn schema_rejects_extra_arguments_as_cli_usage() {
    let output = Command::new(binary())
        .args(["schema", "unexpected"])
        .output()
        .expect("run invalid resourcefs schema command");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert!(!output.stderr.is_empty());
}

#[test]
fn schema_has_stable_draft_identity_and_closed_object_root() {
    let schema: Value = serde_json::from_str(profile_schema_json()).expect("valid emitted schema");
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(
        schema["$id"],
        "https://resourcefs.dev/schema/server-profile-v1.json"
    );
    assert_eq!(schema["title"], "ResourceFS Server Profile v1");
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    assert!(
        schema["description"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(schema["properties"]["schemaVersion"]["const"], 1);
}

#[test]
fn schema_and_deserializer_publish_the_same_source_kinds() {
    let schema: Value = serde_json::from_str(profile_schema_json()).expect("valid emitted schema");
    let serialized = serde_json::to_string(&schema).expect("serialize schema for inspection");

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
        assert!(
            serialized.contains(&format!("\"const\":\"{kind}\"")),
            "schema omits source kind {kind}"
        );
    }

    ProfileDocument::from_slice(br#"{"schemaVersion":1}"#)
        .expect("schema minimal profile must deserialize");
}
