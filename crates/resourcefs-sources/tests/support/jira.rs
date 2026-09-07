#![allow(
    dead_code,
    reason = "integration targets use different fixture operations"
)]
use crate::{
    session_support,
    tls::{self, FIXTURE_HOST, FixtureResponse, TlsListener, match_cert},
};
use resourcefs_core::{
    AllowedOrigin, AtlassianSiteId, HttpCeilings, OperationGuard, OriginAllowlist, PathReference,
    ResourceError, Secret, SourceAdapter, SourceResource,
};
use resourcefs_sources::{
    AtlassianSite, AtlassianSource, AtlassianSourceMount, HttpSubstrate, OriginCredential,
};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
};

pub fn response(body: &str) -> FixtureResponse {
    FixtureResponse::Response {
        status: "200 OK",
        headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body: body.as_bytes().to_vec(),
    }
}

pub fn protocol_response(
    status: &'static str,
    headers: impl IntoIterator<Item = (&'static str, String)>,
    body: impl Into<Vec<u8>>,
) -> FixtureResponse {
    FixtureResponse::Response {
        status,
        headers: headers
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
        body: body.into(),
    }
}

pub async fn fixture_source<R>(router: R) -> (TlsListener, AtlassianSource)
where
    R: Fn(&str) -> FixtureResponse + Send + Sync + 'static,
{
    fixture_source_with_ceilings(router, HttpCeilings::default()).await
}

pub async fn fixture_source_with_ceilings<R>(
    router: R,
    ceilings: HttpCeilings,
) -> (TlsListener, AtlassianSource)
where
    R: Fn(&str) -> FixtureResponse + Send + Sync + 'static,
{
    let (listener, source, _session) = fixture_with_session(router, ceilings).await;
    (listener, source)
}

pub async fn fixture_with_session<R>(
    router: R,
    ceilings: HttpCeilings,
) -> (
    TlsListener,
    AtlassianSource,
    session_support::ScratchFixture,
)
where
    R: Fn(&str) -> FixtureResponse + Send + Sync + 'static,
{
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let dynamic_port = Arc::new(AtomicU16::new(0));
    let observed_port = Arc::clone(&dynamic_port);
    let listener = TlsListener::serve_router(loopback, 0, match_cert(), move |path| {
        let mut response = router(path);
        if let FixtureResponse::Response { body, .. } = &mut response {
            let rewritten = String::from_utf8_lossy(body).replace(
                "https://tls.invalid/",
                &format!(
                    "https://tls.invalid:{}/",
                    observed_port.load(Ordering::Acquire)
                ),
            );
            *body = rewritten.into_bytes();
        }
        response
    })
    .await;
    let port = listener.address.port();
    dynamic_port.store(port, Ordering::Release);
    let origin = AllowedOrigin::new(&format!("https://{FIXTURE_HOST}:{port}/"), true)
        .expect("fixture origin");
    let token = Secret::new("token-canary".to_owned()).expect("token");
    let credential = OriginCredential::basic(origin.clone(), "agent@example.com", &token)
        .expect("Basic credential");
    let substrate = HttpSubstrate::with_host_lookup_and_roots(
        OriginAllowlist::new(vec![origin.clone()]),
        ceilings,
        move |_host| async move { Ok::<_, std::io::Error>(vec![loopback]) },
        &[tls::fixture_ca()],
        vec![credential],
    )
    .expect("substrate");
    let site =
        AtlassianSite::new(AtlassianSiteId::new("acme").expect("Site ID"), origin).expect("site");
    let session = session_support::scratch_fixture().await;
    let source = AtlassianSourceMount::new(vec![site], Arc::new(substrate))
        .expect("mount")
        .bind(session.path_session().clone());
    (listener, source, session)
}

pub fn project(id: &str, key: &str, name: &str) -> serde_json::Value {
    serde_json::json!({"id":id,"key":key,"name":name,"self":format!("https://tls.invalid/rest/api/3/project/{id}")})
}

pub fn page(rows: Vec<serde_json::Value>, start: usize, max: usize, next: Option<usize>) -> String {
    let mut value = serde_json::json!({"values": rows, "startAt":start,"maxResults":max,"isLast":next.is_none()});
    if let Some(next) = next {
        value["nextPage"] = serde_json::json!(format!(
            "https://tls.invalid/rest/api/3/project/search?startAt={next}&maxResults={max}"
        ));
    }
    value.to_string()
}

pub async fn read(
    source: &AtlassianSource,
    reference: &str,
) -> Result<SourceResource, ResourceError> {
    source
        .read(
            &PathReference::parse(reference).expect("fixture reference"),
            &OperationGuard::new(),
        )
        .await
}

pub fn offset(path: &str) -> usize {
    let url = url::Url::parse(&format!("https://tls.invalid{path}")).expect("request URL");
    assert_eq!(url.path(), "/rest/api/3/project/search");
    assert!(!url.query_pairs().any(|(key, _)| key == "jql"));
    url.query_pairs()
        .find(|(key, _)| key == "startAt")
        .expect("native offset")
        .1
        .parse()
        .expect("numeric offset")
}
