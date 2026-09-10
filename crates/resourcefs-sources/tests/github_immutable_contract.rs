#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;

use resourcefs_core::{
    AllowedOrigin, HttpCeilings, OperationGuard, PathReference, ReadAcquisitionLimits, Secret,
    SourceAdapter, SourceResource,
};
use resourcefs_sources::{
    GithubConfig, GithubDeployment, GithubRepository, GithubSource, GithubSourceMount,
    HttpSubstrate, MutationGrants, OriginCredential, SecretReference,
};
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};
use tls::{FixtureResponse, TlsListener};

const COMMIT_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TREE_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PARENT_SHA: &str = "cccccccccccccccccccccccccccccccccccccccc";
const SECOND_PARENT_SHA: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn host_from_head(head: &str) -> &str {
    head.lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
                .map(|(_, value)| value.trim())
        })
        .expect("Host header")
}

fn repository_json(host: &str) -> Value {
    json!({
        "id": 1,
        "node_id": "R_node",
        "name": "repo",
        "full_name": "owner/repo",
        "owner": {
            "id": 2,
            "node_id": "owner_node",
            "login": "owner",
            "url": format!("https://{host}/users/owner"),
            "html_url": "https://github.example/owner"
        },
        "url": format!("https://{host}/repos/owner/repo"),
        "html_url": "https://github.example/owner/repo"
    })
}

fn actor_json(host: &str, id: u64, login: &str) -> Value {
    json!({
        "id": id,
        "node_id": format!("{login}_node"),
        "login": login,
        "url": format!("https://{host}/users/{login}"),
        "html_url": format!("https://github.example/{login}")
    })
}

fn commit_json(host: &str) -> Value {
    json!({
        "sha": COMMIT_SHA,
        "node_id": "commit_node",
        "url": format!("https://{host}/repos/owner/repo/commits/{COMMIT_SHA}"),
        "html_url": format!("https://github.example/owner/repo/commit/{COMMIT_SHA}"),
        "comments_url": format!("https://{host}/repos/owner/repo/commits/{COMMIT_SHA}/comments"),
        "commit": {
            "author": {"name": "Native Author", "email": "author@example.test", "date": "2026-01-02T03:04:05Z"},
            "committer": {"name": "Native Committer", "email": "committer@example.test", "date": "2026-01-02T03:05:06Z"},
            "message": "immutable commit\nwith exact message",
            "tree": {
                "sha": TREE_SHA,
                "url": format!("https://{host}/repos/owner/repo/git/trees/{TREE_SHA}")
            },
            "url": format!("https://{host}/repos/owner/repo/git/commits/{COMMIT_SHA}")
        },
        "author": actor_json(host, 3, "author-login"),
        "committer": null,
        "parents": [{
            "sha": PARENT_SHA,
            "url": format!("https://{host}/repos/owner/repo/commits/{PARENT_SHA}"),
            "html_url": format!("https://github.example/owner/repo/commit/{PARENT_SHA}")
        }, {
            "sha": SECOND_PARENT_SHA,
            "url": format!("https://{host}/repos/owner/repo/commits/{SECOND_PARENT_SHA}"),
            "html_url": format!("https://github.example/owner/repo/commit/{SECOND_PARENT_SHA}")
        }]
    })
}

#[derive(Clone, Copy)]
enum FixtureMode {
    Valid,
    Enterprise,
    MinimalLinks,
    WrongSha,
    WrongLink,
    HostileLogin,
    WrongRepository,
    MalformedNative,
    ZeroRepositoryId,
    AbsentAuthor,
}

async fn fixture(
    mode: FixtureMode,
) -> (TlsListener, GithubSource, session_support::ScratchFixture) {
    let api_prefix = if matches!(mode, FixtureMode::Enterprise) {
        "/api/v3"
    } else {
        ""
    };
    let listener = TlsListener::serve_request_router(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        tls::match_cert(),
        move |request| {
            let host = format!("{}{api_prefix}", host_from_head(request.head()));
            let mut body = match request.target().strip_prefix(api_prefix).expect("configured API prefix") {
                "/repos/owner/repo" => repository_json(&host),
                "/repos/owner/repo/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" => {
                    commit_json(&host)
                }
                target => panic!("unexpected immutable commit request: {target}"),
            };
            match mode {
                FixtureMode::WrongRepository if !request.target().contains("/commits/") => {
                    body["full_name"] = Value::String("other/repo".to_owned());
                }
                FixtureMode::ZeroRepositoryId if !request.target().contains("/commits/") => {
                    body["id"] = json!(0);
                }
                FixtureMode::WrongSha if request.target().contains("/commits/") => {
                    body["sha"] =
                        Value::String("dddddddddddddddddddddddddddddddddddddddd".to_owned());
                }
                FixtureMode::WrongLink if request.target().contains("/commits/") => {
                    body["url"] = Value::String(
                        "https://foreign.invalid/repos/owner/repo/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .to_owned(),
                    );
                }
                FixtureMode::HostileLogin if request.target().contains("/commits/") => {
                    body["author"]["login"] = Value::String("//foreign.invalid/".to_owned());
                }
                FixtureMode::MalformedNative if request.target().contains("/commits/") => {
                    body["commit"]["author"]["date"] = json!(17);
                }
                FixtureMode::AbsentAuthor if request.target().contains("/commits/") => {
                    body.as_object_mut().expect("commit object").remove("author");
                }
                FixtureMode::Valid => {}
                _ => {}
            }
            if request.target().contains("/commits/")
                && matches!(mode, FixtureMode::WrongSha | FixtureMode::MinimalLinks)
            {
                for field in ["html_url", "comments_url"] {
                    body.as_object_mut().expect("commit object").remove(field);
                }
                body["commit"].as_object_mut().expect("native commit").remove("url");
            }
            FixtureResponse::Response {
                status: "200 OK",
                headers: vec![("ETag".to_owned(), format!("\"{}\"", request.target()))],
                body: body.to_string().into_bytes(),
            }
        },
    )
    .await;
    let port = listener.address.port();
    let api = format!("https://{}:{port}{api_prefix}/", tls::FIXTURE_HOST);
    let origin = AllowedOrigin::new(&api, true).expect("origin");
    let secret = Secret::new("immutable-commit-fixture".to_owned()).expect("secret");
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
        "github-immutable-commit",
        false,
        MutationGrants::default(),
        GithubDeployment::new(Some(api), Some("https://github.example".to_owned()))
            .expect("deployment"),
        true,
        SecretReference::environment("UNUSED_IMMUTABLE_COMMIT_TOKEN").expect("reference"),
        vec![GithubRepository::new("owner/repo", MutationGrants::default()).expect("repository")],
        ReadAcquisitionLimits::default(),
    )
    .expect("config");
    let session = session_support::scratch_fixture().await;
    let source = GithubSourceMount::new(config, Arc::new(substrate))
        .bind(session.path_session().clone())
        .expect("source");
    (listener, source, session)
}

async fn read_reference(
    source: &GithubSource,
    reference: &str,
) -> Result<SourceResource, resourcefs_core::ResourceError> {
    source
        .read(
            &PathReference::parse(reference)?,
            &OperationGuard::new(),
            None,
        )
        .await
}

#[tokio::test]
async fn immutable_commit_facts_acquire_repository_and_exact_commit() {
    let (listener, source, _session) = fixture(FixtureMode::Valid).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let resource = read_reference(&source, &reference)
        .await
        .expect("commit facts");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["schemaVersion"], json!({"major": 1, "minor": 0}));
    assert_eq!(document["kind"], "github.commit");
    assert_eq!(document["request"]["commitSha"], COMMIT_SHA);
    assert_eq!(document["observed"]["commitSha"], COMMIT_SHA);
    assert_eq!(document["observed"]["treeSha"], TREE_SHA);
    assert_eq!(document["repository"]["observed"]["fullName"], "owner/repo");
    assert_eq!(
        document["data"]["message"],
        "immutable commit\nwith exact message"
    );
    assert_eq!(document["data"]["author"]["name"], "Native Author");
    assert_eq!(document["data"]["authorAccount"]["login"], "author-login");
    assert!(document["data"]["committerAccount"].is_null());
    assert_eq!(document["data"]["parents"][0]["sha"], PARENT_SHA);
    assert_eq!(document["data"]["parents"][1]["sha"], SECOND_PARENT_SHA);
    assert_eq!(
        document["upstream"]["repository"]["body"]["etag"],
        "\"/repos/owner/repo\""
    );
    assert_eq!(
        document["upstream"]["commit"]["body"]["etag"],
        "\"/repos/owner/repo/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\""
    );
    assert_eq!(
        listener.requests(),
        [
            "GET /repos/owner/repo HTTP/1.1",
            "GET /repos/owner/repo/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa HTTP/1.1"
        ]
    );
}

#[tokio::test]
async fn immutable_commit_facts_accept_path_prefixed_deployment() {
    let (listener, source, _session) = fixture(FixtureMode::Enterprise).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let resource = read_reference(&source, &reference)
        .await
        .expect("Enterprise commit facts");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["observed"]["commitSha"], COMMIT_SHA);
    assert_eq!(
        document["data"]["authorAccount"]["links"]["apiUrl"],
        format!(
            "https://{}:{}/api/v3/users/author-login",
            tls::FIXTURE_HOST,
            listener.address.port()
        )
    );
    assert_eq!(
        listener.requests(),
        [
            "GET /api/v3/repos/owner/repo HTTP/1.1",
            "GET /api/v3/repos/owner/repo/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa HTTP/1.1"
        ]
    );
}

#[tokio::test]
async fn immutable_commit_facts_accept_absent_optional_commit_links() {
    let (_listener, source, _session) = fixture(FixtureMode::MinimalLinks).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let resource = read_reference(&source, &reference)
        .await
        .expect("commit without optional links");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["observed"]["commitSha"], COMMIT_SHA);
    assert_eq!(
        document["data"]["message"],
        "immutable commit\nwith exact message"
    );
    assert!(document["data"]["links"].get("htmlUrl").is_none());
}

#[tokio::test]
async fn immutable_commit_facts_refuse_wrong_observed_sha_even_with_correct_links() {
    let (listener, source, _session) = fixture(FixtureMode::WrongSha).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let error = read_reference(&source, &reference)
        .await
        .expect_err("wrong SHA fixture must refuse");
    assert!(matches!(
        error.details().map(|details| details.reason()),
        Some(resourcefs_core::ErrorReason::UpstreamIdentityMismatch)
    ));
    assert_eq!(listener.requests().len(), 2);
}
#[tokio::test]
async fn immutable_commit_facts_refuse_foreign_commit_link() {
    let (listener, source, _session) = fixture(FixtureMode::WrongLink).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let error = read_reference(&source, &reference)
        .await
        .expect_err("foreign API link must refuse");
    assert_eq!(error.category(), resourcefs_core::ErrorCategory::NotFound);
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test]
async fn immutable_commit_facts_reject_hostile_account_login_path() {
    let (listener, source, _session) = fixture(FixtureMode::HostileLogin).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let error = read_reference(&source, &reference)
        .await
        .expect_err("hostile login path must refuse");
    assert!(matches!(
        error.details().map(|details| details.reason()),
        Some(resourcefs_core::ErrorReason::UpstreamIdentityMismatch)
    ));
    assert_eq!(listener.requests().len(), 2);
}
#[tokio::test]
async fn immutable_commit_facts_reject_wrong_repository_before_commit() {
    let (listener, source, _session) = fixture(FixtureMode::WrongRepository).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let error = read_reference(&source, &reference)
        .await
        .expect_err("wrong repository fixture must refuse");
    assert!(matches!(
        error.details().map(|details| details.reason()),
        Some(resourcefs_core::ErrorReason::UpstreamIdentityMismatch)
    ));
    assert_eq!(listener.requests().len(), 1);
}

#[tokio::test]
async fn immutable_commit_facts_reject_malformed_native_data() {
    let (listener, source, _session) = fixture(FixtureMode::MalformedNative).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let error = read_reference(&source, &reference)
        .await
        .expect_err("malformed native fixture must refuse");
    assert!(matches!(
        error.details().map(|details| details.reason()),
        Some(resourcefs_core::ErrorReason::UpstreamMalformed)
    ));
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test]
async fn immutable_commit_facts_reject_zero_repository_identity() {
    let (listener, source, _session) = fixture(FixtureMode::ZeroRepositoryId).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let error = read_reference(&source, &reference)
        .await
        .expect_err("zero repository identity must refuse");
    assert!(matches!(
        error.details().map(|details| details.reason()),
        Some(resourcefs_core::ErrorReason::UpstreamMalformed)
    ));
    assert_eq!(listener.requests().len(), 1);
}
#[tokio::test]
async fn immutable_commit_facts_preserve_absent_and_null_accounts() {
    let (listener, source, _session) = fixture(FixtureMode::Valid).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let present = read_reference(&source, &reference)
        .await
        .expect("present account");
    let present: Value = serde_json::from_str(present.content()).expect("present JSON");
    assert_eq!(present["data"]["authorAccount"]["login"], "author-login");
    assert!(present["data"]["committerAccount"].is_null());
    assert_eq!(listener.requests().len(), 2);

    let (_listener, source, _session) = fixture(FixtureMode::AbsentAuthor).await;
    let absent = read_reference(&source, &reference)
        .await
        .expect("absent account");
    let absent: Value = serde_json::from_str(absent.content()).expect("absent JSON");
    assert!(absent["data"].get("authorAccount").is_none());
    assert!(absent["data"]["committerAccount"].is_null());
}
