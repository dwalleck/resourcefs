#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use resourcefs_core::{
    AllowedOrigin, ErrorCategory, ErrorReason, HttpCeilings, OperationGuard, PathReference,
    ReadAcquisitionLimits, Secret, SourceAdapter,
};
use resourcefs_sources::{
    GithubConfig, GithubDeployment, GithubRepository, GithubSource, GithubSourceMount,
    HttpSubstrate, MutationGrants, OriginCredential, SecretReference,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
    time::Duration,
};
use tls::{FixtureResponse, TlsListener};

const COMMIT_SHA: &str = "0aa2a666a7286bba34bec5fdc47252b93c0db651";
const ROOT_TREE_SHA: &str = "5fca21cdbcbb9a9afada355222eacd9457d7c3eb";
const BINARY_SHA: &str = "967e0f5bca40b1c9bdf4875861bda015755374f4";
const EXACT_COMMIT_SHA: &str = "7e7f72efcd8b2524971e75b844748465ebd99d49";
const EXACT_BLOB_SHA: &str = "f7497e5e580b6905eae88b10b339c73292fc313e";
const OVER_COMMIT_SHA: &str = "28da95a9edf57b1180a737287bfc927d1f3d143f";
const OVER_BLOB_SHA: &str = "961a3eb80f416473a1e1ccef44dab5bae27c3d6b";

#[derive(Clone, Copy)]
enum Corpus {
    Main,
    Exact,
    Over,
}

#[derive(Clone, Copy)]
enum Mutation {
    None,
    WrongTreeSha,
    WrongTreeLink,
    TruncatedTree,
    DuplicateTreeName,
    UnrepresentableTreeName,
    UnselectedTreeName,
    ContradictoryModeType,
    WrongBlobSha,
    WrongBlobLink,
    WrongBlobEncoding,
    WrongBlobContent,
    MalformedBlobBase64,
    WrongBlobSize,
    BlobUnavailable,
    MalformedOversizedTail,
    OmitBinaryTreeSize,
}

fn host_from_head(head: &str) -> &str {
    head.lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
                .map(|(_, value)| value.trim())
        })
        .expect("Host header")
}

fn fixture_value() -> Value {
    serde_json::from_str(include_str!("fixtures/github_immutable.json")).expect("fixture JSON")
}

fn replace_api(value: &mut Value, host: &str) {
    match value {
        Value::String(text) => *text = text.replace("@API@", &format!("https://{host}/")),
        Value::Array(values) => values.iter_mut().for_each(|value| replace_api(value, host)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| replace_api(value, host)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn generated_tree(sha: &str, template: &Value) -> Value {
    let count = template["entryCount"].as_u64().expect("template count");
    let prefix = template["entryPrefix"].as_str().expect("template prefix");
    let object_sha = template["objectSha"].as_str().expect("template object SHA");
    let size = template["sizeBytes"].as_u64().expect("template size");
    let tree = (0..count)
        .map(|index| {
            json!({
                "mode": "100644",
                "path": format!("{prefix}{index:04}"),
                "sha": object_sha,
                "size": size,
                "type": "blob",
                "url": format!("@API@repos/owner/repo/git/blobs/{object_sha}")
            })
        })
        .collect::<Vec<_>>();
    json!({
        "sha": sha,
        "tree": tree,
        "truncated": false,
        "url": format!("@API@repos/owner/repo/git/trees/{sha}")
    })
}

fn boundary_meta(fixture: &Value, corpus: Corpus) -> &Value {
    &fixture["decodedBoundary"][match corpus {
        Corpus::Exact => "exact",
        Corpus::Over => "over",
        Corpus::Main => panic!("main corpus has no decoded boundary"),
    }]
}

fn boundary_tree(sha: &str, blob_sha: &str) -> Value {
    json!({
        "sha": sha,
        "tree": [{
            "mode": "100644",
            "path": "payload",
            "sha": blob_sha,
            "type": "blob",
            "url": format!("@API@repos/owner/repo/git/blobs/{blob_sha}")
        }],
        "truncated": false,
        "url": format!("@API@repos/owner/repo/git/trees/{sha}")
    })
}

fn boundary_blob(meta: &Value) -> Value {
    let size = meta["decodedSizeBytes"]
        .as_u64()
        .expect("boundary decoded size") as usize;
    let byte = meta["byteValue"].as_u64().expect("boundary byte value") as u8;
    let content = STANDARD.encode(vec![byte; size]);
    let sha = meta["objectSha"].as_str().expect("boundary blob SHA");
    json!({
        "content": content,
        "encoding": "base64",
        "sha": sha,
        "url": format!("@API@repos/owner/repo/git/blobs/{sha}")
    })
}
fn response_body(target: &str, fixture: &Value, corpus: Corpus) -> Value {
    let suffix = target
        .strip_prefix("/repos/owner/repo")
        .expect("repository fixture route");
    match suffix {
        "" => fixture["repository"].clone(),
        value if value.starts_with("/commits/") => {
            let sha = value.strip_prefix("/commits/").expect("commit SHA");
            if !matches!(corpus, Corpus::Main) && sha == boundary_meta(fixture, corpus)["commitSha"]
            {
                boundary_meta(fixture, corpus)["commit"].clone()
            } else {
                fixture["commit"].clone()
            }
        }
        value if value.starts_with("/git/trees/") => {
            let sha = value.strip_prefix("/git/trees/").expect("tree SHA");
            if fixture["trees"][sha].is_object() {
                fixture["trees"][sha].clone()
            } else if !matches!(corpus, Corpus::Main)
                && sha == boundary_meta(fixture, corpus)["treeSha"]
            {
                boundary_tree(
                    sha,
                    boundary_meta(fixture, corpus)["objectSha"]
                        .as_str()
                        .expect("blob SHA"),
                )
            } else {
                generated_tree(sha, &fixture["treeTemplates"][sha])
            }
        }
        value if value.starts_with("/git/blobs/") => {
            let sha = value.strip_prefix("/git/blobs/").expect("blob SHA");
            if !matches!(corpus, Corpus::Main) && sha == boundary_meta(fixture, corpus)["objectSha"]
            {
                boundary_blob(boundary_meta(fixture, corpus))
            } else {
                fixture["blobs"][sha].clone()
            }
        }
        _ => panic!("unexpected source fixture request: {target}"),
    }
}

fn default_fixture_response(
    request: &tls::FixtureRequest,
    fixture: &Value,
    corpus: Corpus,
    mutation: Mutation,
) -> FixtureResponse {
    if matches!(mutation, Mutation::BlobUnavailable) && request.target().contains("/git/blobs/") {
        return FixtureResponse::Response {
            status: "503 Service Unavailable",
            headers: vec![],
            body: b"{}".to_vec(),
        };
    }
    let host = host_from_head(request.head()).to_owned();
    let mut body = response_body(request.target(), fixture, corpus);
    mutate(request.target(), &mut body, mutation);
    replace_api(&mut body, &host);
    FixtureResponse::Response {
        status: "200 OK",
        headers: vec![("ETag".to_owned(), format!("\"{}\"", request.target()))],
        body: body.to_string().into_bytes(),
    }
}

fn mutate(target: &str, body: &mut Value, mutation: Mutation) {
    match mutation {
        Mutation::None | Mutation::BlobUnavailable => {}
        Mutation::WrongTreeSha if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) => {
            body["sha"] = json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        }
        Mutation::WrongTreeLink if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) => {
            body["url"] = json!(
                "https://foreign.invalid/repos/owner/repo/git/trees/".to_owned() + ROOT_TREE_SHA
            );
        }
        Mutation::TruncatedTree if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) => {
            body["truncated"] = json!(true);
        }
        Mutation::DuplicateTreeName if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) => {
            let first = body["tree"].as_array().expect("tree entries")[0].clone();
            body["tree"]
                .as_array_mut()
                .expect("tree entries")
                .push(first);
        }
        Mutation::UnrepresentableTreeName
            if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) =>
        {
            body["tree"][0]["path"] = json!("invalid/name");
        }
        Mutation::UnselectedTreeName
            if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) =>
        {
            body["tree"][0]["path"] = json!("wide-renamed");
        }
        Mutation::ContradictoryModeType
            if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) =>
        {
            let entries = body["tree"].as_array_mut().expect("tree entries");
            let odd = entries
                .iter_mut()
                .find(|entry| entry["path"] == "odd-mode")
                .expect("odd mode");
            odd["type"] = json!("tree");
        }
        Mutation::WrongBlobSha if target.ends_with(&format!("/git/blobs/{BINARY_SHA}")) => {
            body["sha"] = json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        }
        Mutation::WrongBlobLink if target.ends_with(&format!("/git/blobs/{BINARY_SHA}")) => {
            body["url"] = json!(
                "https://foreign.invalid/repos/owner/repo/git/blobs/".to_owned() + BINARY_SHA
            );
        }
        Mutation::WrongBlobEncoding if target.ends_with(&format!("/git/blobs/{BINARY_SHA}")) => {
            body["encoding"] = json!("utf8");
        }
        Mutation::WrongBlobContent if target.ends_with(&format!("/git/blobs/{BINARY_SHA}")) => {
            body["content"] = json!(STANDARD.encode([1u8; 13]));
        }
        Mutation::MalformedBlobBase64 if target.ends_with(&format!("/git/blobs/{BINARY_SHA}")) => {
            body["content"] = json!("AB==\n");
        }
        Mutation::WrongBlobSize if target.ends_with(&format!("/git/blobs/{BINARY_SHA}")) => {
            body["size"] = json!(12);
        }
        Mutation::MalformedOversizedTail
            if target.ends_with(&format!("/git/blobs/{OVER_BLOB_SHA}")) =>
        {
            let content = body["content"].as_str().expect("oversized content");
            let mut mutated = content.to_owned();
            let last_data = mutated.len().checked_sub(2).expect("padded content");
            mutated.replace_range(last_data..last_data + 1, "h");
            body["content"] = json!(mutated);
        }
        Mutation::OmitBinaryTreeSize
            if target.ends_with("/git/trees/d9cf1e2c10ed61b36c812e36c3f79acd6971126c") =>
        {
            let entries = body["tree"].as_array_mut().expect("tree entries");
            let binary = entries.first_mut().expect("binary entry");
            binary
                .as_object_mut()
                .expect("binary object")
                .remove("size");
        }
        Mutation::WrongTreeSha
        | Mutation::WrongTreeLink
        | Mutation::TruncatedTree
        | Mutation::DuplicateTreeName
        | Mutation::UnrepresentableTreeName
        | Mutation::UnselectedTreeName
        | Mutation::ContradictoryModeType
        | Mutation::WrongBlobSha
        | Mutation::WrongBlobLink
        | Mutation::WrongBlobEncoding
        | Mutation::WrongBlobContent
        | Mutation::MalformedBlobBase64
        | Mutation::MalformedOversizedTail
        | Mutation::WrongBlobSize
        | Mutation::OmitBinaryTreeSize => {}
    }
}

async fn fixture_source(
    limits: ReadAcquisitionLimits,
    mutation: Mutation,
) -> (TlsListener, GithubSource, session_support::ScratchFixture) {
    source_for(limits, mutation, Corpus::Main).await
}

async fn source_for(
    limits: ReadAcquisitionLimits,
    mutation: Mutation,
    corpus: Corpus,
) -> (TlsListener, GithubSource, session_support::ScratchFixture) {
    source_for_with_router(limits, mutation, corpus, default_fixture_response).await
}

async fn source_for_with_router<F>(
    limits: ReadAcquisitionLimits,
    mutation: Mutation,
    corpus: Corpus,
    router: F,
) -> (TlsListener, GithubSource, session_support::ScratchFixture)
where
    F: Fn(&tls::FixtureRequest, &Value, Corpus, Mutation) -> FixtureResponse
        + Send
        + Sync
        + 'static,
{
    let fixture = fixture_value();
    let listener = TlsListener::serve_request_router(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        tls::match_cert(),
        move |request| router(request, &fixture, corpus, mutation),
    )
    .await;
    let port = listener.address.port();
    let api = format!("https://{}:{port}/", tls::FIXTURE_HOST);
    let origin = AllowedOrigin::new(&api, true).expect("origin");
    let secret = Secret::new("immutable-source-fixture".to_owned()).expect("secret");
    let credential = OriginCredential::new(origin, "Authorization", Some("Bearer"), &secret)
        .expect("credential");
    let substrate = HttpSubstrate::with_host_lookup_and_roots(
        tls::fixture_allowlist(port, true),
        HttpCeilings::default(),
        |_| async { Ok::<_, std::io::Error>(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]) },
        &[tls::fixture_ca()],
        vec![credential],
    )
    .expect("TLS substrate");
    let config = GithubConfig::new(
        "github-immutable-source",
        false,
        MutationGrants::default(),
        GithubDeployment::new(Some(api), Some("https://github.example".to_owned()))
            .expect("deployment"),
        true,
        SecretReference::environment("UNUSED_IMMUTABLE_SOURCE_TOKEN").expect("reference"),
        vec![GithubRepository::new("owner/repo", MutationGrants::default()).expect("repository")],
        limits,
    )
    .expect("config");
    let session = session_support::scratch_fixture().await;
    let source = GithubSourceMount::new(config, Arc::new(substrate))
        .bind(session.path_session().clone())
        .expect("source");
    (listener, source, session)
}

async fn read(
    source: &GithubSource,
    encoded_path: &str,
) -> Result<resourcefs_core::SourceResource, resourcefs_core::ResourceError> {
    read_commit(source, COMMIT_SHA, encoded_path).await
}

async fn read_commit(
    source: &GithubSource,
    commit: &str,
    encoded_path: &str,
) -> Result<resourcefs_core::SourceResource, resourcefs_core::ResourceError> {
    let operation = OperationGuard::new();
    read_commit_with_operation(source, commit, encoded_path, &operation).await
}

async fn read_commit_with_operation(
    source: &GithubSource,
    commit: &str,
    encoded_path: &str,
    operation: &OperationGuard,
) -> Result<resourcefs_core::SourceResource, resourcefs_core::ResourceError> {
    source
        .read(
            &PathReference::parse(format!(
                "github://owner/repo/source/{commit}/{encoded_path}/facts"
            ))?,
            operation,
            None,
        )
        .await
}

fn case_string<'a>(fixture: &'a Value, name: &str, field: &str) -> &'a str {
    fixture["cases"][name][field].as_str().expect("case string")
}

#[tokio::test]
async fn source_facts_cover_every_independent_fixture_case() {
    let fixture = fixture_value();
    for name in [
        "binary",
        "empty",
        "executable",
        "crlf",
        "mixedNewlines",
        "lfs",
        "ordering",
        "factsName",
        "controlName",
        "wide",
        "symlink",
        "submodule",
        "directory",
        "unsupportedMode",
        "tooWide",
        "deepBlobLimit",
        "deepBeforeTerminalLimit",
    ] {
        let (listener, source, _session) =
            fixture_source(ReadAcquisitionLimits::default(), Mutation::None).await;
        let encoded = case_string(&fixture, name, "encodedPath");
        let result = read(&source, encoded).await;
        match case_string(&fixture, name, "expectedState") {
            "error" => {
                let error = result.expect_err("fixture error");
                assert_eq!(
                    error.details().map(|details| details.reason()),
                    Some(ErrorReason::LimitExceeded)
                );
            }
            "available" | "unsupported" | "unavailable" => {
                let resource = result.expect("fixture result");
                let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
                assert_eq!(document["kind"], "github.source");
                assert_eq!(
                    document["request"]["path"],
                    case_string(&fixture, name, "path")
                );
                assert_eq!(
                    document["data"]["path"],
                    case_string(&fixture, name, "path")
                );
                assert_eq!(
                    document["observed"]["containingTreeSha"],
                    case_string(&fixture, name, "containingTreeSha")
                );
                assert_eq!(
                    document["data"]["containingTreeSha"],
                    case_string(&fixture, name, "containingTreeSha")
                );
                assert_eq!(
                    document["data"]["mode"],
                    case_string(&fixture, name, "mode")
                );
                assert_eq!(
                    document["data"]["objectType"],
                    case_string(&fixture, name, "objectType")
                );
                assert_eq!(
                    document["data"]["objectSha"],
                    case_string(&fixture, name, "objectSha")
                );
                assert_eq!(
                    document["data"]["content"]["state"],
                    case_string(&fixture, name, "expectedState")
                );
                if case_string(&fixture, name, "expectedState") == "available" {
                    assert_eq!(
                        document["data"]["content"]["bytesBase64"],
                        case_string(&fixture, name, "bytesBase64")
                    );
                    assert_eq!(
                        document["data"]["content"]["decodedSizeBytes"],
                        fixture["cases"][name]["decodedSizeBytes"]
                    );
                } else {
                    assert_eq!(
                        document["data"]["content"]["reason"],
                        case_string(&fixture, name, "reason")
                    );
                    assert!(
                        !document["data"]["content"]
                            .as_object()
                            .expect("content object")
                            .contains_key("bytesBase64")
                    );
                }
            }
            state => panic!("unexpected fixture state {state}"),
        }
        let depth = case_string(&fixture, name, "path").split('/').count();
        match case_string(&fixture, name, "expectedState") {
            "available" => assert_eq!(listener.requests().len(), depth + 3, "{name}"),
            "unsupported" => {
                assert_eq!(listener.requests().len(), depth + 2, "{name}");
                assert!(
                    !listener
                        .requests()
                        .iter()
                        .any(|request| request.contains("/git/blobs/"))
                );
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn source_facts_verify_tree_and_blob_identity_before_retention() {
    for mutation in [
        Mutation::WrongTreeSha,
        Mutation::WrongTreeLink,
        Mutation::TruncatedTree,
        Mutation::DuplicateTreeName,
        Mutation::UnrepresentableTreeName,
        Mutation::UnselectedTreeName,
        Mutation::ContradictoryModeType,
        Mutation::WrongBlobSha,
        Mutation::WrongBlobLink,
        Mutation::WrongBlobEncoding,
        Mutation::WrongBlobContent,
        Mutation::MalformedBlobBase64,
        Mutation::WrongBlobSize,
    ] {
        let (_listener, source, _session) =
            fixture_source(ReadAcquisitionLimits::default(), mutation).await;
        let error = read(&source, "docs/%CE%BB%20space%252F%3Araw.bin")
            .await
            .expect_err("contradictory fixture must fail");
        assert!(
            error.details().is_some(),
            "identity/malformed failure is machine-readable"
        );
    }
}

#[tokio::test]
async fn source_facts_revalidate_304_preserves_bytes_identity_and_provenance() {
    let counts = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
    let route_counts = Arc::clone(&counts);
    let (listener, source, _session) = source_for_with_router(
        ReadAcquisitionLimits::default(),
        Mutation::None,
        Corpus::Main,
        move |request, fixture, corpus, mutation| {
            let is_revalidation = {
                let mut counts = route_counts.lock().expect("revalidation counters");
                let count = counts.entry(request.target().to_owned()).or_default();
                let is_revalidation = *count > 0;
                *count += 1;
                is_revalidation
            };
            if is_revalidation {
                assert!(
                    request
                        .head()
                        .to_ascii_lowercase()
                        .contains("if-none-match"),
                    "304 must be conditional"
                );
                FixtureResponse::Response {
                    status: "304 Not Modified",
                    headers: vec![],
                    body: vec![],
                }
            } else {
                default_fixture_response(request, fixture, corpus, mutation)
            }
        },
    )
    .await;
    let first = read(&source, "docs/%CE%BB%20space%252F%3Araw.bin")
        .await
        .expect("fresh source facts");
    let second = read(&source, "docs/%CE%BB%20space%252F%3Araw.bin")
        .await
        .expect("304 source facts");
    let first_document: Value = serde_json::from_str(first.content()).expect("fresh facts JSON");
    let second_document: Value =
        serde_json::from_str(second.content()).expect("revalidated facts JSON");
    assert_eq!(
        first_document["upstream"]["repository"]["body"],
        second_document["upstream"]["repository"]["body"]
    );
    assert_eq!(
        second_document["upstream"]["repository"]["body"]["status"],
        200
    );

    assert_eq!(first_document["data"], second_document["data"]);
    assert_eq!(first_document["observed"], second_document["observed"]);
    assert_eq!(
        second_document["data"]["content"]["bytesBase64"],
        "AP+AbGluZQ0KZW5kAA=="
    );
    assert_eq!(second_document["data"]["content"]["decodedSizeBytes"], 13);
    assert_eq!(
        second_document["upstream"]["repository"]["revalidation"]["status"],
        304
    );
    assert_eq!(
        second_document["upstream"]["commit"]["revalidation"]["status"],
        304
    );
    for tree in second_document["upstream"]["trees"]
        .as_object()
        .expect("tree provenance")
        .values()
    {
        assert_eq!(tree["revalidation"]["status"], 304);
    }
    assert_eq!(
        second_document["upstream"]["blobs"][BINARY_SHA]["revalidation"]["status"],
        304
    );
    assert_ne!(
        first.version_tag(),
        second.version_tag(),
        "revalidation provenance is part of the representation identity"
    );
    assert_eq!(
        listener.requests().len(),
        10,
        "fresh plus conditional chain"
    );
}

#[tokio::test]
async fn source_facts_failed_blob_revalidation_never_exposes_stale_bytes() {
    let counts = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
    let route_counts = Arc::clone(&counts);
    let (listener, source, _session) = source_for_with_router(
        ReadAcquisitionLimits::default(),
        Mutation::None,
        Corpus::Main,
        move |request, fixture, corpus, mutation| {
            let is_revalidation = {
                let mut counts = route_counts.lock().expect("revalidation counters");
                let count = counts.entry(request.target().to_owned()).or_default();
                let is_revalidation = *count > 0;
                *count += 1;
                is_revalidation
            };
            if is_revalidation && request.target().contains("/git/blobs/") {
                FixtureResponse::Response {
                    status: "503 Service Unavailable",
                    headers: vec![],
                    body: b"{}".to_vec(),
                }
            } else if is_revalidation {
                FixtureResponse::Response {
                    status: "304 Not Modified",
                    headers: vec![],
                    body: vec![],
                }
            } else {
                default_fixture_response(request, fixture, corpus, mutation)
            }
        },
    )
    .await;
    let fresh = read(&source, "docs/%CE%BB%20space%252F%3Araw.bin")
        .await
        .expect("fresh source facts");
    let fresh_document: Value = serde_json::from_str(fresh.content()).expect("fresh facts JSON");
    assert_eq!(fresh_document["data"]["content"]["state"], "available");
    assert_eq!(
        fresh_document["data"]["content"]["bytesBase64"],
        "AP+AbGluZQ0KZW5kAA=="
    );

    let fresh_requests = listener.requests().len();
    let stale = read(&source, "docs/%CE%BB%20space%252F%3Araw.bin")
        .await
        .expect("verified metadata retention");
    let stale_document: Value =
        serde_json::from_str(stale.content()).expect("retained metadata facts JSON");
    assert_eq!(stale_document["data"]["content"]["state"], "unavailable");
    assert_eq!(
        stale_document["data"]["content"]["reason"],
        "acquisition_failed"
    );
    assert!(
        !stale_document["data"]["content"]
            .as_object()
            .expect("content object")
            .contains_key("bytesBase64")
    );
    assert_eq!(
        stale_document["data"]["objectSha"],
        fresh_document["data"]["objectSha"]
    );
    assert!(
        listener
            .requests()
            .iter()
            .skip(fresh_requests)
            .any(|request| request.contains(&format!("/git/blobs/{BINARY_SHA}")))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn source_facts_generation_change_between_each_response_refuses_publication() {
    let (_listener, control_source, _control_session) =
        fixture_source(ReadAcquisitionLimits::default(), Mutation::None).await;
    let control = read(&control_source, "docs/%CE%BB%20space%252F%3Araw.bin")
        .await
        .expect("uncancelled generation control");
    let control_document: Value =
        serde_json::from_str(control.content()).expect("control facts JSON");
    assert_eq!(control_document["data"]["content"]["state"], "available");

    for blocked_target in [
        "/repos/owner/repo".to_owned(),
        format!("/repos/owner/repo/commits/{COMMIT_SHA}"),
        format!("/repos/owner/repo/git/trees/{ROOT_TREE_SHA}"),
        format!("/repos/owner/repo/git/blobs/{BINARY_SHA}"),
    ] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let route_entered = Arc::clone(&entered);
        let (release, blocked) = std::sync::mpsc::channel();
        let blocked = Arc::new(Mutex::new(blocked));
        let route_blocked = Arc::clone(&blocked);
        let route_target = blocked_target.clone();
        let (listener, source, session) = source_for_with_router(
            ReadAcquisitionLimits::default(),
            Mutation::None,
            Corpus::Main,
            move |request, fixture, corpus, mutation| {
                if request.target() == route_target {
                    route_entered.notify_one();
                    route_blocked
                        .lock()
                        .expect("generation response barrier")
                        .recv_timeout(Duration::from_secs(5))
                        .expect("generation response release");
                }
                default_fixture_response(request, fixture, corpus, mutation)
            },
        )
        .await;
        let source = Arc::new(source);
        let operation = OperationGuard::new();
        let worker_operation = operation.clone();
        let reading = Arc::clone(&source);
        let task = tokio::spawn(async move {
            read_commit_with_operation(
                &reading,
                COMMIT_SHA,
                "docs/%CE%BB%20space%252F%3Araw.bin",
                &worker_operation,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(3), entered.notified())
            .await
            .expect("selected response entered");
        session
            .path_session()
            .cache_remove_namespace("github-http")
            .await
            .expect("generation invalidation");
        release.send(()).expect("selected response release");
        let error = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .expect("generation refusal")
            .expect("source task")
            .expect_err("invalidated source cannot publish");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
        assert_eq!(
            error.details().expect("generation details").reason(),
            ErrorReason::UpstreamUnavailable
        );
        assert!(
            listener
                .requests()
                .iter()
                .any(|request| request.contains(blocked_target.as_str())),
            "the race controller must block the named response"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn source_facts_cancellation_at_metadata_and_verified_blob_is_whole_read_refusal() {
    for blocked_target in [
        "/repos/owner/repo".to_owned(),
        format!("/repos/owner/repo/git/blobs/{BINARY_SHA}"),
    ] {
        let (_control_listener, control_source, _control_session) =
            fixture_source(ReadAcquisitionLimits::default(), Mutation::None).await;
        let control = read(&control_source, "docs/%CE%BB%20space%252F%3Araw.bin")
            .await
            .expect("uncancelled cancellation control");
        let control_document: Value =
            serde_json::from_str(control.content()).expect("control facts JSON");
        assert_eq!(control_document["data"]["content"]["state"], "available");
        assert_eq!(
            control_document["data"]["content"]["bytesBase64"],
            "AP+AbGluZQ0KZW5kAA=="
        );

        let entered = Arc::new(tokio::sync::Notify::new());
        let route_entered = Arc::clone(&entered);
        let (release, blocked) = std::sync::mpsc::channel();
        let blocked = Arc::new(Mutex::new(blocked));
        let route_blocked = Arc::clone(&blocked);
        let route_target = blocked_target.clone();
        let (listener, source, _session) = source_for_with_router(
            ReadAcquisitionLimits::default(),
            Mutation::None,
            Corpus::Main,
            move |request, fixture, corpus, mutation| {
                if request.target() == route_target {
                    route_entered.notify_one();
                    route_blocked
                        .lock()
                        .expect("cancellation response barrier")
                        .recv_timeout(Duration::from_secs(5))
                        .expect("cancellation response release");
                }
                default_fixture_response(request, fixture, corpus, mutation)
            },
        )
        .await;
        let source = Arc::new(source);
        let operation = OperationGuard::new();
        let worker_operation = operation.clone();
        let reading = Arc::clone(&source);
        let task = tokio::spawn(async move {
            read_commit_with_operation(
                &reading,
                COMMIT_SHA,
                "docs/%CE%BB%20space%252F%3Araw.bin",
                &worker_operation,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(3), entered.notified())
            .await
            .expect("selected cancellation response entered");
        assert!(operation.cancel(), "active source operation cancels");
        release
            .send(())
            .expect("selected cancellation response release");
        let error = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .expect("cancellation refusal")
            .expect("source task")
            .expect_err("cancelled source cannot publish metadata or bytes");
        assert_eq!(error.category(), ErrorCategory::Cancelled);
        assert_eq!(
            error.details().expect("cancellation details").reason(),
            ErrorReason::Cancelled
        );
        assert!(
            listener
                .requests()
                .iter()
                .any(|request| request.contains(blocked_target.as_str())),
            "the cancellation controller must block the named response"
        );
    }
}

#[tokio::test]
async fn source_facts_retain_ordinary_blob_failure_but_not_identity_failure() {
    let (_listener, source, _session) =
        fixture_source(ReadAcquisitionLimits::default(), Mutation::BlobUnavailable).await;
    let resource = read(&source, "run.sh")
        .await
        .expect("ordinary blob failure facts");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["data"]["content"]["state"], "unavailable");
    assert_eq!(document["data"]["content"]["reason"], "acquisition_failed");
    assert!(document["data"]["content"]["failure"].is_object());

    let (_listener, source, _session) =
        fixture_source(ReadAcquisitionLimits::default(), Mutation::WrongBlobSha).await;
    let error = read(&source, "docs/%CE%BB%20space%252F%3Araw.bin")
        .await
        .expect_err("blob identity failure must invalidate source facts");
    assert_eq!(
        error.details().map(|details| details.reason()),
        Some(ErrorReason::UpstreamIdentityMismatch)
    );
}

#[tokio::test]
async fn source_facts_accept_exact_decoded_cap_and_reject_cap_plus_one() {
    for (corpus, commit, expected_state, expected_size, encoded_length, blob_sha) in [
        (
            Corpus::Exact,
            EXACT_COMMIT_SHA,
            "available",
            4_194_304usize,
            5_592_408usize,
            EXACT_BLOB_SHA,
        ),
        (
            Corpus::Over,
            OVER_COMMIT_SHA,
            "unavailable",
            4_194_305usize,
            5_592_408usize,
            OVER_BLOB_SHA,
        ),
    ] {
        let (listener, source, _session) =
            source_for(ReadAcquisitionLimits::default(), Mutation::None, corpus).await;
        let resource = read_commit(&source, commit, "payload")
            .await
            .expect("boundary source facts");
        let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
        assert_eq!(document["data"]["content"]["state"], expected_state);
        assert_eq!(
            document["data"]["content"]["reason"].is_string(),
            expected_state == "unavailable"
        );
        assert_eq!(
            document["data"]["content"]["decodedSizeBytes"].as_u64(),
            (expected_state == "available").then_some(expected_size as u64)
        );
        assert_eq!(
            document["data"]["content"]["bytesBase64"]
                .as_str()
                .map(str::len),
            (expected_state == "available").then_some(encoded_length)
        );
        assert_eq!(document["data"]["sizeBytes"], Value::Null);
        assert_eq!(document["data"]["objectSha"], blob_sha);
        assert!(document["upstream"]["blobs"][blob_sha]["body"].is_object());
        assert!(
            listener
                .requests()
                .iter()
                .any(|request| request.contains("/git/blobs/"))
        );
    }

    let (_listener, source, _session) = source_for(
        ReadAcquisitionLimits::default(),
        Mutation::MalformedOversizedTail,
        Corpus::Over,
    )
    .await;
    let error = read_commit(&source, OVER_COMMIT_SHA, "payload")
        .await
        .expect_err("noncanonical oversized base64 must fail");
    assert_eq!(
        error.details().map(|details| details.reason()),
        Some(ErrorReason::UpstreamMalformed)
    );
}

#[tokio::test]
async fn source_facts_apply_decoded_limit_before_and_after_blob_acquisition() {
    let limits =
        ReadAcquisitionLimits::new(Some(10), None, None, None, None, Some(12)).expect("limits");
    let (listener, source, _session) = fixture_source(limits, Mutation::None).await;
    let resource = read(&source, "run.sh").await.expect("reported size limit");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["data"]["content"]["reason"], "decoded_size_limit");
    assert!(
        !listener
            .requests()
            .iter()
            .any(|request| request.contains("/git/blobs/"))
    );

    let (listener, source, _session) = fixture_source(limits, Mutation::OmitBinaryTreeSize).await;
    let resource = read(&source, "docs/%CE%BB%20space%252F%3Araw.bin")
        .await
        .expect("blob-stage limit");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["data"]["content"]["reason"], "decoded_size_limit");
    assert!(
        listener
            .requests()
            .iter()
            .any(|request| request.contains("/git/blobs/"))
    );
    assert_eq!(document["acquisition"]["limits"]["maxDecodedBytes"], 12);
}

#[tokio::test(start_paused = true)]
async fn source_facts_enforce_exact_representation_limit_without_truncation() {
    // Keep paused time from auto-advancing during real loopback I/O: elapsedMs must
    // have a stable width while solving the representation-limit fixed point.
    let clock_guard = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    let (_listener, source, _session) =
        fixture_source(ReadAcquisitionLimits::default(), Mutation::None).await;
    let baseline = read(&source, "run.sh")
        .await
        .expect("baseline source facts");
    let mut exact_size = baseline.content().len();
    for _ in 0..8 {
        let exact_limits =
            ReadAcquisitionLimits::new(Some(10), None, None, None, Some(exact_size), None)
                .expect("exact representation limit");
        let (_listener, source, _session) = fixture_source(exact_limits, Mutation::None).await;
        let exact = read(&source, "run.sh")
            .await
            .expect("representation candidate fits");
        let measured = exact.content().len();
        if measured == exact_size {
            break;
        }
        exact_size = measured;
    }
    let exact_limits =
        ReadAcquisitionLimits::new(Some(10), None, None, None, Some(exact_size), None)
            .expect("fixed-point representation limit");
    let (_listener, source, _session) = fixture_source(exact_limits, Mutation::None).await;
    let exact = read(&source, "run.sh")
        .await
        .expect("exact representation fits");
    assert_eq!(exact.content().len(), exact_size);

    let below_limits =
        ReadAcquisitionLimits::new(Some(10), None, None, None, Some(exact_size - 1), None)
            .expect("below representation limit");
    let (_listener, source, _session) = fixture_source(below_limits, Mutation::None).await;
    let error = read(&source, "run.sh")
        .await
        .expect_err("one byte below representation must fail");
    assert_eq!(
        error.details().map(|details| details.reason()),
        Some(ErrorReason::LimitExceeded)
    );
    clock_guard.abort();
}
#[tokio::test]
async fn source_facts_keep_one_attempt_budget_through_deep_blob_stage() {
    let limits =
        ReadAcquisitionLimits::new(Some(10), None, None, None, None, None).expect("limits");
    let (listener, source, _session) = fixture_source(limits, Mutation::None).await;
    let resource = read(&source, "deep/d1/d2/d3/d4/d5/d6/payload")
        .await
        .expect("metadata-only source facts");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["data"]["content"]["state"], "unavailable");
    assert_eq!(document["data"]["content"]["reason"], "acquisition_failed");
    assert_eq!(document["acquisition"]["usage"]["attemptedRequests"], 10);
    assert!(
        !listener
            .requests()
            .iter()
            .any(|request| request.contains("/git/blobs/"))
    );
}
