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
    time::Duration,
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

/// A GitHub App account exactly as the REST API spells it: the login keeps its
/// `[bot]` suffix, the API route percent-encodes the brackets
/// (`/users/dependabot%5Bbot%5D`) and the profile link names the app slug under
/// `/apps/<slug>`. Ids, node ids and slugs below are the provider's own values,
/// read from `gh api users/dependabot%5Bbot%5D`, `gh api
/// users/github-actions%5Bbot%5D` and the `author` of
/// `repos/actions/checkout/commits/e8d4307400f9427dba7cb98e488d6ab85f1cec5f`.
fn app_account_json(host: &str, id: u64, node_id: &str, login: &str, slug: &str) -> Value {
    let encoded = login.replace('[', "%5B").replace(']', "%5D");
    json!({
        "id": id,
        "node_id": node_id,
        "login": login,
        "url": format!("https://{host}/users/{encoded}"),
        "html_url": format!("https://github.example/apps/{slug}")
    })
}

const DEPENDABOT_ID: u64 = 49_699_333;
const GITHUB_ACTIONS_ID: u64 = 41_898_282;

/// Which account link an app-account fixture bends, so each case exercises one
/// half of the relaxation and nothing else.
#[derive(Clone, Copy, Debug)]
enum AppCase {
    /// The provider's own spelling of both accounts.
    Accepted,
    /// The API link names a login that is not this account.
    WrongApiAccount,
    /// The profile link names an app that is not this account's slug.
    WrongAppLink,
    /// A foreign origin on the API link.
    ForeignApiLink,
    /// A foreign origin on the profile link.
    ForeignWebLink,
}

/// Which unusable login a fixture serves.
#[derive(Clone, Copy, Debug)]
enum LoginCase {
    Empty,
    Dot,
    DotDot,
}

/// How a fixture drops the commit's own required API link.
#[derive(Clone, Copy, Debug)]
enum RequiredLinkCase {
    /// The provider omits the key.
    Omitted,
    /// The provider sends an explicit null.
    Null,
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
    /// Both native accounts are GitHub App accounts, bent by `AppCase`.
    AppAccounts(AppCase),
    /// The author's login is one of `LoginCase`'s unusable values.
    MalformedLogin(LoginCase),
    /// The commit's own required API link is absent or null.
    RequiredCommitLink(RequiredLinkCase),
    /// A custom deployment whose API base carries an escaped path byte.
    EncodedPrefix,
}

async fn fixture(
    mode: FixtureMode,
) -> (TlsListener, GithubSource, session_support::ScratchFixture) {
    fixture_with_limits(mode, ReadAcquisitionLimits::default()).await
}

async fn fixture_with_limits(
    mode: FixtureMode,
    operator: ReadAcquisitionLimits,
) -> (TlsListener, GithubSource, session_support::ScratchFixture) {
    let api_prefix = match mode {
        FixtureMode::Enterprise => "/api/v3",
        // A configured `apiBaseUrl` may spell a path byte escaped, as
        // `Url::parse` itself does for a space or a non-ASCII label.
        FixtureMode::EncodedPrefix => "/GitHub%20Enterprise/api/v3",
        _ => "",
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
                FixtureMode::AppAccounts(case) if request.target().contains("/commits/") => {
                    let mut author = app_account_json(
                        &host,
                        DEPENDABOT_ID,
                        "MDM6Qm90NDk2OTkzMzM=",
                        "dependabot[bot]",
                        "dependabot",
                    );
                    body["committer"] = app_account_json(
                        &host,
                        GITHUB_ACTIONS_ID,
                        "MDM6Qm90NDE4OTgyODI=",
                        "github-actions[bot]",
                        "github-actions",
                    );
                    match case {
                        AppCase::Accepted => {}
                        AppCase::WrongApiAccount => {
                            author["url"] = json!(format!("https://{host}/users/someone-else"));
                        }
                        AppCase::WrongAppLink => {
                            author["html_url"] = json!("https://github.example/apps/other-app");
                        }
                        AppCase::ForeignApiLink => {
                            author["url"] = json!(
                                "https://foreign.invalid/users/dependabot%5Bbot%5D".to_owned()
                            );
                        }
                        AppCase::ForeignWebLink => {
                            author["html_url"] =
                                json!("https://foreign.invalid/apps/dependabot".to_owned());
                        }
                    }
                    body["author"] = author;
                }
                FixtureMode::MalformedLogin(case) if request.target().contains("/commits/") => {
                    body["author"]["login"] = Value::String(match case {
                        LoginCase::Empty => String::new(),
                        LoginCase::Dot => ".".to_owned(),
                        LoginCase::DotDot => "..".to_owned(),
                    });
                }
                FixtureMode::MalformedNative if request.target().contains("/commits/") => {
                    body["commit"]["author"]["date"] = json!(17);
                }
                FixtureMode::AbsentAuthor if request.target().contains("/commits/") => {
                    body.as_object_mut().expect("commit object").remove("author");
                }
                FixtureMode::RequiredCommitLink(case) if request.target().contains("/commits/") => {
                    match case {
                        RequiredLinkCase::Omitted => {
                            body.as_object_mut().expect("commit object").remove("url");
                        }
                        RequiredLinkCase::Null => body["url"] = Value::Null,
                    }
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
        operator,
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
    read_reference_with_limits(source, reference, None).await
}

async fn read_reference_with_limits(
    source: &GithubSource,
    reference: &str,
    limits: Option<&ReadAcquisitionLimits>,
) -> Result<SourceResource, resourcefs_core::ResourceError> {
    source
        .read(
            &PathReference::parse(reference)?,
            &OperationGuard::new(),
            limits,
        )
        .await
}

/// The reason this family's stable `unavailableFacts` vocabulary records for
/// `field`, or `None` when the document does not record the field at all.
fn unavailable_reason<'a>(document: &'a Value, field: &str) -> Option<&'a str> {
    document["unavailableFacts"]
        .as_array()
        .expect("unavailable facts")
        .iter()
        .find(|entry| entry["field"] == field)
        .map(|entry| entry["reason"].as_str().expect("recorded reason"))
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
async fn immutable_commit_facts_accept_percent_encoded_deployment_prefix() {
    // A configured `apiBaseUrl` may carry a path byte spelled escaped. The
    // deployment's own prefix is compared as it is spelled on both sides, so
    // the provider's escaping of the same byte is not read as a contradiction —
    // and only the route below the prefix is compared decoded.
    let (listener, source, _session) = fixture(FixtureMode::EncodedPrefix).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let resource = read_reference(&source, &reference)
        .await
        .expect("escaped-prefix deployment commit facts");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    assert_eq!(document["observed"]["commitSha"], COMMIT_SHA);
    assert_eq!(
        document["data"]["authorAccount"]["links"]["apiUrl"],
        format!(
            "https://{}:{}/GitHub%20Enterprise/api/v3/users/author-login",
            tls::FIXTURE_HOST,
            listener.address.port()
        )
    );
    assert_eq!(
        listener.requests(),
        [
            "GET /GitHub%20Enterprise/api/v3/repos/owner/repo HTTP/1.1",
            "GET /GitHub%20Enterprise/api/v3/repos/owner/repo/commits/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa HTTP/1.1"
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
    // `FixtureMode::WrongSha` also omits the three optional SHA-bearing links
    // (the shared `MinimalLinks` set), so the equality mutation is isolated:
    // every link this fixture keeps is the one the requested SHA expects, and
    // the observed SHA alone decides the refusal.
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
async fn immutable_commit_facts_refuse_absent_or_null_required_commit_link() {
    // Presence and identity are different questions. The shared required-link
    // guard answers a contradiction for both an absent and a foreign link, so a
    // commit whose own API link is missing must be reported as a malformed
    // document rather than as an object that does not exist.
    for case in [RequiredLinkCase::Omitted, RequiredLinkCase::Null] {
        let (_listener, source, _session) = fixture(FixtureMode::RequiredCommitLink(case)).await;
        let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
        let error = read_reference(&source, &reference)
            .await
            .expect_err("a commit without its own API link must refuse");
        assert_eq!(
            error.category(),
            resourcefs_core::ErrorCategory::SourceUnavailable,
            "{case:?}"
        );
        assert!(
            matches!(
                error.details().map(|details| details.reason()),
                Some(resourcefs_core::ErrorReason::UpstreamMalformed)
            ),
            "{case:?}: {error}"
        );
    }
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
async fn immutable_commit_facts_accept_github_app_accounts() {
    // A GitHub App's account is the one place the provider's own links cannot be
    // rebuilt from the login: the API route percent-encodes the `[bot]` suffix
    // and the profile link names the app slug. Both `validate_account` call
    // sites carry such a shape here.
    let (listener, source, _session) = fixture(FixtureMode::AppAccounts(AppCase::Accepted)).await;
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let resource = read_reference(&source, &reference)
        .await
        .expect("app accounts are accounts");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    for (key, login, api_route, web_route) in [
        (
            "authorAccount",
            "dependabot[bot]",
            "/users/dependabot%5Bbot%5D",
            "https://github.example/apps/dependabot",
        ),
        (
            "committerAccount",
            "github-actions[bot]",
            "/users/github-actions%5Bbot%5D",
            "https://github.example/apps/github-actions",
        ),
    ] {
        assert_eq!(document["data"][key]["login"], login, "{key} login");
        assert_eq!(
            document["data"][key]["links"]["apiUrl"],
            format!(
                "https://{}:{}{api_route}",
                tls::FIXTURE_HOST,
                listener.address.port()
            ),
            "{key} API link"
        );
        assert_eq!(
            document["data"][key]["links"]["htmlUrl"], web_route,
            "{key} profile link"
        );
    }
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test]
async fn immutable_commit_facts_refuse_app_account_links_naming_another_account_or_origin() {
    // The app-account relaxation accepts the provider's two spellings of this
    // one account and nothing else: each case bends exactly one supplied link,
    // so a check that accepted any `/apps/` profile, any bracketed login or any
    // origin would publish an account the native record does not name.
    for case in [
        AppCase::WrongApiAccount,
        AppCase::WrongAppLink,
        AppCase::ForeignApiLink,
        AppCase::ForeignWebLink,
    ] {
        let (listener, source, _session) = fixture(FixtureMode::AppAccounts(case)).await;
        let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
        let outcome = read_reference(&source, &reference).await;
        assert_eq!(
            outcome
                .as_ref()
                .err()
                .and_then(|error| error.details())
                .map(|details| details.reason()),
            Some(resourcefs_core::ErrorReason::UpstreamIdentityMismatch),
            "{case:?} must be an identity contradiction"
        );
        assert_eq!(listener.requests().len(), 2, "{case:?}");
    }
}

#[tokio::test]
async fn immutable_commit_facts_refuse_unusable_account_logins() {
    // An empty login names the user *collection* and a dot segment never
    // survives URL parsing, so neither can be the account the native record
    // claims to identify. Each is malformed rather than a contradiction with a
    // link — which is the verdict the link comparison alone would reach here,
    // since these fixtures keep the links of a real login.
    for case in [LoginCase::Empty, LoginCase::Dot, LoginCase::DotDot] {
        let (listener, source, _session) = fixture(FixtureMode::MalformedLogin(case)).await;
        let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
        let outcome = read_reference(&source, &reference).await;
        assert_eq!(
            outcome
                .as_ref()
                .err()
                .and_then(|error| error.details())
                .map(|details| details.reason()),
            Some(resourcefs_core::ErrorReason::UpstreamMalformed),
            "{case:?} must be malformed rather than a link contradiction"
        );
        assert_eq!(listener.requests().len(), 2, "{case:?}");
    }
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
    // The native committer is `null` rather than absent, and the two are
    // different observations: the owned document must keep the key with a null
    // value and record the `null` reason. Asserting only `is_null()` cannot
    // tell that apart from a dropped key, because serde reads a missing key
    // back as `Null` too.
    assert!(
        present["data"].get("committerAccount").is_some(),
        "a null account keeps its key"
    );
    assert!(present["data"]["committerAccount"].is_null());
    assert_eq!(
        unavailable_reason(&present, "committerAccount"),
        Some("null")
    );
    assert_eq!(listener.requests().len(), 2);

    let (_listener, source, _session) = fixture(FixtureMode::AbsentAuthor).await;
    let absent = read_reference(&source, &reference)
        .await
        .expect("absent account");
    let absent: Value = serde_json::from_str(absent.content()).expect("absent JSON");
    assert!(absent["data"].get("authorAccount").is_none());
    assert!(absent["data"]["committerAccount"].is_null());
    // Both spellings appear in this one document, so the record distinguishes
    // them where the serialized values alone would not: the dropped key is
    // `omitted` and the null one is `null`.
    assert_eq!(
        unavailable_reason(&absent, "authorAccount"),
        Some("omitted")
    );
    assert!(
        absent["data"].get("committerAccount").is_some(),
        "a null account keeps its key"
    );
    assert_eq!(
        unavailable_reason(&absent, "committerAccount"),
        Some("null")
    );
}

#[tokio::test]
async fn immutable_commit_facts_report_effective_limits_and_enforce_the_attempt_bound() {
    // `docs/operating.md` documents `"acquisition": {"maxAttempts": 2}` for this
    // route: the two attempts are exactly what repository plus exact-commit
    // acquisition need, and the owned document must report the intersection of
    // the profile's bounds with the caller's, per dimension.
    let (listener, source, _session) = fixture_with_limits(
        FixtureMode::Valid,
        ReadAcquisitionLimits::new(
            Some(2),
            Some(Duration::from_secs(10)),
            Some(1_048_576),
            Some(2_097_152),
            Some(4_194_304),
            None,
        )
        .expect("documented profile limits"),
    )
    .await;
    // The caller both lowers and raises dimensions against the profile bounds,
    // so each reported value names which side decided that dimension.
    let caller = ReadAcquisitionLimits::new(
        Some(5),
        Some(Duration::from_secs(3)),
        Some(524_288),
        Some(4_194_304),
        Some(8_388_608),
        None,
    )
    .expect("caller limits");
    let reference = format!("github://owner/repo/commits/{COMMIT_SHA}/facts");
    let resource = read_reference_with_limits(&source, &reference, Some(&caller))
        .await
        .expect("both GETs fit the profile attempt bound");
    let document: Value = serde_json::from_str(resource.content()).expect("facts JSON");
    for (field, expected) in [
        ("maxAttempts", 2.0),
        ("timeoutMs", 3000.0),
        ("maxResponseBytes", 524_288.0),
        ("maxAcceptedBodyBytes", 2_097_152.0),
        ("maxRepresentationBytes", 4_194_304.0),
    ] {
        assert_eq!(
            document["acquisition"]["limits"][field]
                .as_f64()
                .expect("numeric limit"),
            expected,
            "{field}"
        );
    }
    assert_eq!(document["acquisition"]["usage"]["attemptedRequests"], 2);
    assert_eq!(
        listener.requests().len(),
        2,
        "the attempt bound admits both GETs"
    );

    // The bound is enforced, not merely echoed: with one attempt the read
    // cannot reach the commit after the repository.
    let (listener, source, _session) = fixture_with_limits(
        FixtureMode::Valid,
        ReadAcquisitionLimits::new(Some(1), None, None, None, None, None).expect("one attempt"),
    )
    .await;
    let error = read_reference(&source, &reference)
        .await
        .expect_err("one attempt cannot acquire the repository and the commit");
    assert_eq!(
        error.category(),
        resourcefs_core::ErrorCategory::LimitExceeded
    );
    assert_eq!(
        error
            .details()
            .and_then(|details| details.limit())
            .map(|limit| (limit.kind(), limit.bound())),
        Some((resourcefs_core::AcquisitionLimitKind::Attempts, 1))
    );
    assert_eq!(
        listener.requests().len(),
        1,
        "the second GET is refused before it is sent"
    );
}
