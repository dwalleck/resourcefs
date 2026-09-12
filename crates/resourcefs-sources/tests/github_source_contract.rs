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

#[derive(Clone, Copy, Debug)]
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
    BlobRetryAfterBeyondDeadline,
    BlobDenied,
    BlobAbsent,
    BlobRetryAfterWithinDeadline,
    BlobAborted,
    OmitBlobUrl,
    ForeignSiblingLink,
    NullBinaryTreeSize,
    OpaqueEntryLink,
    QueryBearingEntryLink,
    OmitTreeUrl,
    OversizedEntrySize,
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
    if request.target().contains("/git/blobs/") {
        let blob_failure = match mutation {
            Mutation::BlobUnavailable => Some(("503 Service Unavailable", vec![])),
            Mutation::BlobDenied => Some(("401 Unauthorized", vec![])),
            Mutation::BlobAbsent => Some(("404 Not Found", vec![])),
            // Guidance that cannot fit the remaining logical deadline, so the
            // refusal is a deadline error produced before the deadline expires.
            Mutation::BlobRetryAfterBeyondDeadline => Some((
                "429 Too Many Requests",
                vec![("Retry-After".to_owned(), "120".to_owned())],
            )),
            // Guidance that fits the remaining deadline: an ordinary rate limit,
            // not a deadline refusal.
            Mutation::BlobRetryAfterWithinDeadline => Some((
                "429 Too Many Requests",
                vec![("Retry-After".to_owned(), "1".to_owned())],
            )),
            _ => None,
        };
        if let Some((status, headers)) = blob_failure {
            return FixtureResponse::Response {
                status,
                headers,
                body: b"{}".to_vec(),
            };
        }
        // A dropped connection is the last ordinary class: the object itself is
        // fine, the request did not arrive.
        if matches!(mutation, Mutation::BlobAborted) {
            return FixtureResponse::Abort;
        }
    }
    let host = request.host().to_owned();
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
        // A sibling the caller never selects carries a link naming another
        // deployment's object. Only the selected entry is judged, so this must
        // not fail a read of a different path.
        Mutation::ForeignSiblingLink
            if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) =>
        {
            let entries = body["tree"].as_array_mut().expect("tree entries");
            let entry = entries
                .iter_mut()
                .find(|entry| entry["path"] == "run.sh")
                .expect("executable entry");
            let shas = entry["sha"].as_str().expect("entry sha").to_owned();
            entry["url"] = json!(format!(
                "https://foreign.invalid/repos/owner/repo/git/blobs/{shas}"
            ));
        }
        // The blob response omits its own top-level link. Presence and identity
        // answer different questions, so this is a malformed document rather
        // than a contradiction.
        Mutation::OmitBlobUrl if target.contains("/git/blobs/") => {
            body.as_object_mut().expect("blob object").remove("url");
        }
        Mutation::NullBinaryTreeSize
            if target.ends_with("/git/trees/d9cf1e2c10ed61b36c812e36c3f79acd6971126c") =>
        {
            let entries = body["tree"].as_array_mut().expect("tree entries");
            let binary = entries.first_mut().expect("binary entry");
            binary
                .as_object_mut()
                .expect("binary object")
                .insert("size".to_owned(), Value::Null);
        }
        Mutation::OversizedEntrySize
            if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) =>
        {
            let entries = body["tree"].as_array_mut().expect("tree entries");
            let entry = entries
                .iter_mut()
                .find(|entry| entry["path"] == "run.sh")
                .expect("executable entry");
            entry["size"] = json!(9_000_000);
        }
        Mutation::OmitTreeUrl if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) => {
            body.as_object_mut().expect("tree object").remove("url");
        }
        Mutation::OpaqueEntryLink if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) => {
            let entries = body["tree"].as_array_mut().expect("tree entries");
            let entry = entries
                .iter_mut()
                .find(|entry| entry["path"] == "run.sh")
                .expect("executable entry");
            entry["url"] = json!("not-a-url-at-all");
        }
        Mutation::QueryBearingEntryLink
            if target.ends_with(&format!("/git/trees/{ROOT_TREE_SHA}")) =>
        {
            let entries = body["tree"].as_array_mut().expect("tree entries");
            let entry = entries
                .iter_mut()
                .find(|entry| entry["path"] == "run.sh")
                .expect("executable entry");
            let url = entry["url"].as_str().expect("entry url").to_owned();
            entry["url"] = json!(format!("{url}?ref=main"));
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
        | Mutation::OmitBinaryTreeSize
        | Mutation::BlobRetryAfterBeyondDeadline
        | Mutation::BlobDenied
        | Mutation::BlobAbsent
        | Mutation::BlobRetryAfterWithinDeadline
        | Mutation::BlobAborted
        | Mutation::OmitBlobUrl
        | Mutation::ForeignSiblingLink
        | Mutation::NullBinaryTreeSize
        | Mutation::OpaqueEntryLink
        | Mutation::QueryBearingEntryLink
        | Mutation::OmitTreeUrl
        | Mutation::OversizedEntrySize => {}
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
    // Every contradictory shape must produce its own typed verdict, and must do
    // so without publishing a Facts document. The request count is asserted too:
    // it separates a refusal that happened before the blob GET (3 requests: the
    // repository, the commit and the root tree) from one that happened after it
    // (5: those, plus the two trees of this path and the blob).
    for (mutation, expected_category, expected_reason, expected_requests) in [
        (
            Mutation::WrongTreeSha,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamIdentityMismatch,
            3,
        ),
        (
            Mutation::WrongTreeLink,
            ErrorCategory::NotFound,
            ErrorReason::UpstreamIdentityMismatch,
            3,
        ),
        (
            Mutation::TruncatedTree,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamMalformed,
            3,
        ),
        (
            // A directory that names one entry twice is not a Git tree at all.
            Mutation::DuplicateTreeName,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamMalformed,
            3,
        ),
        (
            Mutation::UnrepresentableTreeName,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamMalformed,
            3,
        ),
        (
            // An entry the caller never selects is still covered by the
            // reconstructed tree hash, so renaming one is a contradiction.
            Mutation::UnselectedTreeName,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamIdentityMismatch,
            3,
        ),
        (
            Mutation::ContradictoryModeType,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamMalformed,
            3,
        ),
        (
            Mutation::OmitTreeUrl,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamMalformed,
            3,
        ),
        (
            // The blob's own link is required too: absent is a malformed
            // document, not a contradiction.
            Mutation::OmitBlobUrl,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamMalformed,
            5,
        ),
        (
            Mutation::WrongBlobSha,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamIdentityMismatch,
            5,
        ),
        (
            Mutation::WrongBlobLink,
            ErrorCategory::NotFound,
            ErrorReason::UpstreamIdentityMismatch,
            5,
        ),
        (
            Mutation::WrongBlobEncoding,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamMalformed,
            5,
        ),
        (
            Mutation::WrongBlobContent,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamIdentityMismatch,
            5,
        ),
        (
            Mutation::MalformedBlobBase64,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamMalformed,
            5,
        ),
        (
            Mutation::WrongBlobSize,
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamIdentityMismatch,
            5,
        ),
    ] {
        let (listener, source, _session) =
            fixture_source(ReadAcquisitionLimits::default(), mutation).await;
        let error = read(&source, "docs/%CE%BB%20space%252F%3Araw.bin")
            .await
            .expect_err("contradictory fixture must fail");
        assert_eq!(error.category(), expected_category, "{mutation:?}");
        assert_eq!(
            error.details().map(|details| details.reason()),
            Some(expected_reason),
            "{mutation:?}"
        );
        let requests = listener.requests();
        assert_eq!(
            requests.len(),
            expected_requests,
            "{mutation:?} stopped at the contradiction rather than continuing"
        );
        // The further the walk got, the more it must not have asked for: a
        // contradiction in the root tree never reaches the blob.
        assert_eq!(
            requests
                .iter()
                .any(|request| request.contains("/git/blobs/")),
            expected_requests == 5,
            "{mutation:?}"
        );
    }
}

/// A per-target request counter shared by the revalidation tests.
///
/// Returns a router that hands each request to `respond` together with whether
/// this target has been seen before, so a test can answer the second and later
/// hits differently (a conditional 304, or a failure of the revalidation).
fn counting_router<F>(
    respond: F,
) -> impl Fn(&tls::FixtureRequest, &Value, Corpus, Mutation) -> FixtureResponse + Send + Sync + 'static
where
    F: Fn(bool, &tls::FixtureRequest, &Value, Corpus, Mutation) -> FixtureResponse
        + Send
        + Sync
        + 'static,
{
    let counts = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
    move |request, fixture, corpus, mutation| {
        let revalidation = {
            let mut counts = counts.lock().expect("revalidation counters");
            let count = counts.entry(request.target().to_owned()).or_default();
            let revalidation = *count > 0;
            *count += 1;
            revalidation
        };
        respond(revalidation, request, fixture, corpus, mutation)
    }
}

/// Parks one named upstream response inside the read.
///
/// The barrier is the point: a test that changes session state must do so while
/// the read is provably inside that request, so the refusal is provoked by the
/// change rather than by ordinary sequencing.
struct ResponseBarrier {
    entered: Arc<tokio::sync::Notify>,
    blocked: Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
    release: std::sync::mpsc::Sender<()>,
}

impl ResponseBarrier {
    fn new() -> Self {
        let (release, blocked) = std::sync::mpsc::channel();
        Self {
            entered: Arc::new(tokio::sync::Notify::new()),
            blocked: Arc::new(Mutex::new(blocked)),
            release,
        }
    }

    /// A router that parks `target` until [`Self::release`].
    fn router(
        &self,
        target: String,
    ) -> impl Fn(&tls::FixtureRequest, &Value, Corpus, Mutation) -> FixtureResponse + Send + Sync + 'static
    {
        let entered = Arc::clone(&self.entered);
        let blocked = Arc::clone(&self.blocked);
        move |request, fixture, corpus, mutation| {
            if request.target() == target {
                entered.notify_one();
                blocked
                    .lock()
                    .expect("response barrier")
                    .recv_timeout(Duration::from_secs(5))
                    .expect("blocked response release");
            }
            default_fixture_response(request, fixture, corpus, mutation)
        }
    }

    async fn wait_entered(&self) {
        tokio::time::timeout(Duration::from_secs(3), self.entered.notified())
            .await
            .expect("selected response entered");
    }

    fn release(&self) {
        self.release.send(()).expect("selected response release");
    }
}

#[tokio::test]
async fn source_facts_revalidate_304_preserves_bytes_identity_and_provenance() {
    let (listener, source, _session) = source_for_with_router(
        ReadAcquisitionLimits::default(),
        Mutation::None,
        Corpus::Main,
        counting_router(|revalidation, request, fixture, corpus, mutation| {
            if revalidation {
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
        }),
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
    let (listener, source, _session) = source_for_with_router(
        ReadAcquisitionLimits::default(),
        Mutation::None,
        Corpus::Main,
        counting_router(|revalidation, request, fixture, corpus, mutation| {
            if revalidation && request.target().contains("/git/blobs/") {
                FixtureResponse::Response {
                    status: "503 Service Unavailable",
                    headers: vec![],
                    body: b"{}".to_vec(),
                }
            } else if revalidation {
                FixtureResponse::Response {
                    status: "304 Not Modified",
                    headers: vec![],
                    body: vec![],
                }
            } else {
                default_fixture_response(request, fixture, corpus, mutation)
            }
        }),
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
        let barrier = ResponseBarrier::new();
        let (listener, source, session) = source_for_with_router(
            ReadAcquisitionLimits::default(),
            Mutation::None,
            Corpus::Main,
            barrier.router(blocked_target.clone()),
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
        barrier.wait_entered().await;
        session
            .path_session()
            .cache_remove_namespace("github-http")
            .await
            .expect("generation invalidation");
        barrier.release();
        let error = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .expect("generation refusal")
            .expect("source task")
            .expect_err("invalidated source cannot publish");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
        // Its own reason: the caller's session changed, so the retry is against
        // that, not against a provider that reported itself unavailable.
        assert_eq!(
            error.details().expect("generation details").reason(),
            ErrorReason::CacheGenerationChanged
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

        let barrier = ResponseBarrier::new();
        let (listener, source, _session) = source_for_with_router(
            ReadAcquisitionLimits::default(),
            Mutation::None,
            Corpus::Main,
            barrier.router(blocked_target.clone()),
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
        barrier.wait_entered().await;
        assert!(operation.cancel(), "active source operation cancels");
        barrier.release();
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
    // An ordinary failure of the one blob request keeps the terminal identity
    // the trees already proved, and publishes only bounded failure facts.
    for (mutation, expected_category, expected_reason) in [
        (
            Mutation::BlobUnavailable,
            "source_unavailable",
            "upstream_unavailable",
        ),
        (
            // Guidance that fits the remaining deadline is an ordinary rate
            // limit, not the deadline refusal above.
            Mutation::BlobRetryAfterWithinDeadline,
            "source_unavailable",
            "upstream_rate_limited",
        ),
        (
            Mutation::BlobAborted,
            "source_unavailable",
            "transport_failure",
        ),
        (Mutation::BlobDenied, "permission_denied", "upstream_denied"),
        (
            Mutation::BlobAbsent,
            "not_found",
            "upstream_not_found_or_hidden",
        ),
    ] {
        let (_listener, source, _session) =
            fixture_source(ReadAcquisitionLimits::default(), mutation).await;
        let resource = read(&source, "run.sh")
            .await
            .unwrap_or_else(|error| panic!("{mutation:?} is ordinary: {error}"));
        let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
        assert_eq!(document["data"]["content"]["state"], "unavailable");
        assert_eq!(
            document["data"]["content"]["reason"], "acquisition_failed",
            "{mutation:?}"
        );
        assert_eq!(
            document["data"]["content"]["failure"]["category"], expected_category,
            "{mutation:?}"
        );
        assert_eq!(
            document["data"]["content"]["failure"]["reason"], expected_reason,
            "{mutation:?}"
        );
        assert_eq!(document["data"]["objectType"], "blob");
        assert_eq!(document["data"]["mode"], "100755");
    }

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
async fn source_facts_refuse_the_whole_read_for_a_deadline_refusal() {
    // The blob's 429 carries `Retry-After: 120` while most of the logical
    // deadline remains, so the refusal is a deadline error produced before the
    // deadline expired. A decoded-size or outage retention must not swallow it:
    // DESIGN.md and docs/operating.md both make deadline expiration a whole-read
    // refusal, and a published `acquisition_failed` would be a refusal reported
    // as a successful partial document.
    let (_listener, source, _session) = fixture_source(
        ReadAcquisitionLimits::default(),
        Mutation::BlobRetryAfterBeyondDeadline,
    )
    .await;
    let error = read(&source, "run.sh")
        .await
        .expect_err("a deadline refusal cannot be published as a partial document");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    let details = error.details().expect("typed deadline refusal");
    assert_eq!(details.reason(), ErrorReason::DeadlineExceeded);
    assert_eq!(
        details.retry_guidance(),
        Some(resourcefs_core::RetryGuidance::DelaySeconds(120))
    );
}

#[tokio::test]
async fn source_facts_refuse_the_whole_read_when_the_response_ceiling_bounds_the_blob() {
    // `maxResponseBytes` is an independent caller dimension, and a base64 body
    // is 4/3 the decoded size: the exact decoded boundary corpus (4 MiB decoded,
    // 5,592,408 encoded) is legal under the default decoded cap but not under a
    // caller response ceiling of 4 MiB. The caller is entitled to the typed
    // refusal naming that bound — not a 200 document whose content says
    // acquisition_failed, which a retention predicate keyed on the error *kind*
    // would have produced.
    let limits =
        ReadAcquisitionLimits::new(None, None, Some(4_194_304), None, None, None).expect("limits");
    let (_listener, source, _session) = source_for(limits, Mutation::None, Corpus::Exact).await;
    let error = read_commit(&source, EXACT_COMMIT_SHA, "payload")
        .await
        .expect_err("a response-body refusal cannot be published as a partial document");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    let details = error.details().expect("typed limit refusal");
    assert_eq!(details.reason(), ErrorReason::LimitExceeded);
    assert_eq!(
        details.limit().map(|limit| limit.kind()),
        Some(resourcefs_core::AcquisitionLimitKind::ResponseBodyBytes)
    );
}

#[tokio::test]
async fn source_facts_classify_mid_path_and_absent_components_as_not_found() {
    // A component that resolves to a file, symlink or submodule cannot be
    // descended, so the caller's path provably does not exist in a verified
    // tree. Nothing upstream contradicted itself, so this is not an outage or
    // a malformed provider document.
    for encoded in ["run.sh/anything", "link/anything", "submodule/anything"] {
        let (_listener, source, _session) =
            fixture_source(ReadAcquisitionLimits::default(), Mutation::None).await;
        let error = read(&source, encoded)
            .await
            .expect_err("a non-directory component cannot be descended");
        assert_eq!(error.category(), ErrorCategory::NotFound, "{encoded}");
        assert_eq!(
            error.details().map(|details| details.reason()),
            Some(ErrorReason::UpstreamNotFoundOrHidden),
            "{encoded}"
        );
    }

    // The sibling branch that answers "this path is not here" for a name the
    // verified directory does not contain, kept beside it so both stay one rule.
    let (listener, source, _session) =
        fixture_source(ReadAcquisitionLimits::default(), Mutation::None).await;
    let error = read(&source, "docs/absent")
        .await
        .expect_err("an absent name is not present in a verified tree");
    assert_eq!(error.category(), ErrorCategory::NotFound);
    assert_eq!(
        error.details().map(|details| details.reason()),
        Some(ErrorReason::UpstreamNotFoundOrHidden)
    );
    assert_eq!(listener.requests().len(), 4, "root tree then the docs tree");
}

#[tokio::test]
async fn source_facts_read_a_path_whose_sibling_entries_carry_opaque_links() {
    // Only the selected entry's supplied link is judged. A sibling entry the
    // caller never selects cannot fail the read, and a value that is not a URL
    // at all stays an opaque provider observation rather than a contradiction.
    for (mutation, path) in [
        (
            Mutation::OpaqueEntryLink,
            "docs/%CE%BB%20space%252F%3Araw.bin",
        ),
        (
            Mutation::QueryBearingEntryLink,
            "docs/%CE%BB%20space%252F%3Araw.bin",
        ),
        (Mutation::OpaqueEntryLink, "run.sh"),
        (Mutation::QueryBearingEntryLink, "run.sh"),
        // An unselected entry naming another deployment's object is not this
        // read's contradiction: the link is never read, retained or followed.
        // This is the case that fails if validation widens back to every entry
        // of every traversed tree.
        (
            Mutation::ForeignSiblingLink,
            "docs/%CE%BB%20space%252F%3Araw.bin",
        ),
    ] {
        let (_listener, source, _session) =
            fixture_source(ReadAcquisitionLimits::default(), mutation).await;
        let resource = read(&source, path)
            .await
            .unwrap_or_else(|error| panic!("{mutation:?} on {path} must not refuse: {error}"));
        let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
        assert_eq!(document["kind"], "github.source");
    }
}

#[tokio::test]
async fn source_facts_refuse_a_size_refusal_without_claiming_an_observed_size() {
    // A Git tree entry carries no size the reconstructed tree hash covers, so an
    // over-stated downstream size drives the pre-fetch refusal and is never
    // cross-checked against the bytes. The refusal therefore names its bound and
    // omits `observed` rather than publishing an unverified number as a
    // measurement, and the blob is never requested.
    let (listener, source, _session) = fixture_source(
        ReadAcquisitionLimits::default(),
        Mutation::OversizedEntrySize,
    )
    .await;
    let resource = read(&source, "run.sh")
        .await
        .expect("a metadata-only refusal is not a read failure");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["data"]["content"]["state"], "unavailable");
    assert_eq!(document["data"]["content"]["reason"], "decoded_size_limit");
    assert_eq!(
        document["data"]["content"]["limit"]["kind"],
        "decoded_content_bytes"
    );
    assert!(
        !document["data"]["content"]["limit"]
            .as_object()
            .expect("limit object")
            .contains_key("observed"),
        "an unverified upstream size is not an observation"
    );
    assert!(
        !listener
            .requests()
            .iter()
            .any(|request| request.contains("/git/blobs/")),
        "the refusal happens before the blob request"
    );
}

#[tokio::test]
async fn source_facts_publish_presence_aware_terminal_size() {
    // `null` and an omitted key are different upstream observations in the one
    // family whose purpose is byte-exact fidelity.
    // The supplied branch: this fixture's executable entry carries size 29, and
    // a verified blob is the one case where that number is cross-checked.
    let (_listener, source, _session) =
        fixture_source(ReadAcquisitionLimits::default(), Mutation::None).await;
    let resource = read(&source, "run.sh").await.expect("source facts");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["data"]["content"]["state"], "available");
    assert_eq!(document["data"]["sizeBytes"], 29);
    assert!(
        document["unavailableFacts"]
            .as_array()
            .expect("facts")
            .is_empty(),
        "a supplied size leaves nothing unavailable"
    );

    for (mutation, expected_reason, present) in [
        (Mutation::OmitBinaryTreeSize, "omitted", false),
        (Mutation::NullBinaryTreeSize, "null", true),
    ] {
        let (_listener, source, _session) =
            fixture_source(ReadAcquisitionLimits::default(), mutation).await;
        let resource = read(&source, "docs/%CE%BB%20space%252F%3Araw.bin")
            .await
            .expect("source facts");
        let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
        assert_eq!(document["data"]["content"]["state"], "available");
        // An explicit null and an absent key are different upstream
        // observations. Asserted on presence, because reading a missing key
        // back through `Value` indexing also yields Null.
        assert_eq!(
            document["data"]
                .as_object()
                .expect("data object")
                .contains_key("sizeBytes"),
            present,
            "{mutation:?}"
        );
        if present {
            assert_eq!(document["data"]["sizeBytes"], Value::Null, "{mutation:?}");
        }
        assert!(
            document["unavailableFacts"]
                .as_array()
                .expect("facts")
                .iter()
                .any(|entry| entry["field"] == "sizeBytes" && entry["reason"] == expected_reason),
            "{mutation:?} records why the size is unavailable"
        );
    }
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
        // This corpus supplies no tree-entry size at all, so nothing unverified
        // is published for it. Read through `as_object`, not `Value` indexing:
        // indexing a missing key also yields Null.
        assert!(
            !document["data"]
                .as_object()
                .expect("data object")
                .contains_key("sizeBytes"),
            "an unsupplied size is not invented"
        );
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
    // Eight components cost eight trees plus the repository and commit: the
    // shared attempt ceiling is spent before the blob is reached, so the read is
    // refused rather than published as a document with no content. One ledger
    // covers the whole walk, and exhaustion is a typed refusal naming the
    // dimension the caller could have raised.
    let limits =
        ReadAcquisitionLimits::new(Some(10), None, None, None, None, None).expect("limits");
    let (listener, source, _session) = fixture_source(limits, Mutation::None).await;
    let error = read(&source, "deep/d1/d2/d3/d4/d5/d6/payload")
        .await
        .expect_err("the blob stage cannot be reached inside the attempt ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    let details = error.details().expect("typed attempt refusal");
    assert_eq!(details.reason(), ErrorReason::LimitExceeded);
    let limit = details.limit().expect("attempt limit detail");
    assert_eq!(
        limit.kind(),
        resourcefs_core::AcquisitionLimitKind::Attempts
    );
    assert_eq!(limit.bound(), 10);
    assert_eq!(limit.observed(), Some(11));
    assert_eq!(listener.requests().len(), 10);
    assert!(
        !listener
            .requests()
            .iter()
            .any(|request| request.contains("/git/blobs/"))
    );
}
