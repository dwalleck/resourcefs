//! Credential transmission, leak-freedom, and read cancellation for the
//! mounted HTTPS Source Adapter (rfs-g2z9 C20 and C21).
//!
//! **Why this file could not exist before.** C12 originally claimed both "no
//! resolved address leaks" and "no credential leaks". The credential half was
//! unfenceable: nothing transmitted a credential, so a sentinel scan would have
//! found nothing and passed *unconditionally* — green while asserting nothing,
//! and green afterwards no matter how transmission was later wired up. C12 was
//! narrowed to addresses, and C20 records the real pair: the credential must
//! reach the wire **and** must reach no observable channel. Transmission is
//! what makes the leak scan meaningful, which is why both halves are one claim.
//!
//! Two shapes recur here, and both are deliberate:
//!
//! - **Unique explanation.** An absent header is produced just as readily by a
//!   request that never happened as by a credential that was never attached, so
//!   the transmission row is paired against a no-credential row on the same
//!   listener: only the configured credential differs between them.
//! - **Positive control for absence.** "The value appears on no channel" and
//!   "the server stopped short" are absence claims. Each channel is asserted
//!   non-empty before it is scanned, and the cancelled read is measured against
//!   an uncancelled row that transfers the whole body.

#[path = "support/tls.rs"]
mod tls;

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use resourcefs_core::{
    AllowedOrigin, ErrorCategory, OperationGuard, OriginAllowlist, PathReference, Secret,
    ServerLimits, SourceAdapter, WorkspaceRootId,
};
use resourcefs_sources::{
    ArtifactSource, BackingPathVisibility, CompiledSources, FilesystemSource, HttpsSource,
    LaunchRoot, LaunchRootSource, LocalSource, OriginCredential, SESSION_CLEANUP_TTL,
    SessionStorageConfig, SessionStore,
};
use tempfile::TempDir;
use tls::{
    FIXTURE_HOST, FixtureResponse, MATCH_CERT, TlsListener, fixture_allowlist, settle,
    tls_substrate_with_credentials,
};

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// The credential's secret value.
///
/// Deliberately unique: a scan for something like `"token"` would match
/// incidental prose in an error message and report a leak that is not one, or
/// match nothing and report safety it did not establish.
const SENTINEL: &str = "rfs-credential-sentinel-9d41f7a2";
const HEADER: &str = "x-rfs-authorization";
const SCHEME: &str = "Bearer";
const PAGE: &str = "<html><body><p>fixture page</p></body></html>";

/// A trickled body large enough that a read is still in flight when
/// cancellation lands, and small enough to stay far under the fetch ceiling.
const TRICKLE_LEN: usize = 256 * 1024;
const TRICKLE_CHUNK: usize = 32 * 1024;
const TRICKLE_DELAY: Duration = Duration::from_millis(50);

fn origin(port: u16) -> AllowedOrigin {
    AllowedOrigin::new(&format!("https://{FIXTURE_HOST}:{port}/"), true)
        .expect("the fixture origin is well formed")
}

fn credential(port: u16) -> OriginCredential {
    let secret = Secret::new(SENTINEL.to_owned()).expect("the sentinel is a valid secret");
    OriginCredential::new(origin(port), HEADER, Some(SCHEME), &secret)
        .expect("the credential composes")
}

fn source(port: u16, credentials: Vec<OriginCredential>) -> HttpsSource {
    HttpsSource::new(Arc::new(tls_substrate_with_credentials(
        fixture_allowlist(port, true),
        vec![LOOPBACK],
        credentials,
    )))
}

fn reference(port: u16, path: &str) -> PathReference {
    PathReference::parse(format!("https://{FIXTURE_HOST}:{port}{path}"))
        .expect("the https reference parses")
}

/// C20 — a configured origin's credential reaches the wire, and an origin
/// without one sends no credential header.
///
/// Both rows run against the same listener, so the only difference between them
/// is the configured credential. That is what makes the control row load
/// bearing: an assertion that the header is present would also be satisfied by
/// an empty log if no request were ever made, and the absence assertion in the
/// control row would pass for exactly that reason too. Requiring the log to
/// grow between the rows rules that explanation out.
#[tokio::test]
async fn credential_reaches_the_wire_only_when_configured() {
    let listener = TlsListener::serve(LOOPBACK, 0, MATCH_CERT, PAGE).await;
    let port = listener.address.port();

    source(port, vec![credential(port)])
        .read(&reference(port, "/doc"), &OperationGuard::new())
        .await
        .expect("a credentialed read succeeds");
    settle().await;

    let credentialed = listener.heads();
    assert_eq!(
        credentialed.len(),
        1,
        "the listener must have recorded exactly one request, or the header \
         assertions below describe a request that never happened: {credentialed:?}"
    );
    let head = credentialed[0].to_ascii_lowercase();
    assert!(
        head.contains(HEADER),
        "the configured credential header must reach the wire: {head}"
    );
    assert!(
        head.contains(&SENTINEL.to_ascii_lowercase()),
        "the credential value must reach the wire under its header: {head}"
    );
    assert!(
        head.contains(&format!("{}: {SCHEME} {SENTINEL}", HEADER).to_ascii_lowercase()),
        "the configured auth scheme must prefix the value on one header line: {head}"
    );

    // POSITIVE CONTROL — same listener, same origin, no credential configured.
    source(port, Vec::new())
        .read(&reference(port, "/doc"), &OperationGuard::new())
        .await
        .expect("an uncredentialed read succeeds");
    settle().await;

    let both = listener.heads();
    assert_eq!(
        both.len(),
        2,
        "the control row must have produced its own request, or its empty \
         header set proves nothing: {both:?}"
    );
    let control = both[1].to_ascii_lowercase();
    assert!(
        !control.contains(HEADER),
        "an origin with no configured credential must send no credential header: {control}"
    );
    assert!(
        !control.contains(&SENTINEL.to_ascii_lowercase()),
        "the credential value must not reach an origin that did not configure it: {control}"
    );
}

/// C20 — the credential value appears on no channel a caller can observe.
///
/// The refusal exercised here is the cross-origin redirect guard: the fixture
/// redirects to a *second allowlisted* origin, so the request is refused by the
/// credential guard rather than by the allowlist. That is the one code path
/// that has both the credential and a rendered error in hand, which makes it
/// the likeliest place for the value to escape.
///
/// Every channel is asserted non-empty before it is scanned. Scanning an empty
/// string finds no sentinel and reports success — the exact vacuous shape this
/// fence exists to rule out.
#[tokio::test]
async fn credential_never_reaches_an_observable_channel() {
    let elsewhere = TlsListener::serve(LOOPBACK, 0, MATCH_CERT, PAGE).await;
    let elsewhere_port = elsewhere.address.port();
    let redirect = format!("https://{FIXTURE_HOST}:{elsewhere_port}/landing");

    let listener = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, {
        let redirect = redirect.clone();
        move |path: &str| {
            if path.starts_with("/hop") {
                FixtureResponse::Redirect(redirect.clone())
            } else {
                FixtureResponse::Body(PAGE.to_owned())
            }
        }
    })
    .await;
    let port = listener.address.port();

    // Both origins are allowlisted, so a refusal is the credential guard's
    // doing and not the allowlist's.
    let allowlist = OriginAllowlist::new(vec![origin(port), origin(elsewhere_port)]);
    let source = HttpsSource::new(Arc::new(tls_substrate_with_credentials(
        allowlist,
        vec![LOOPBACK],
        vec![credential(port)],
    )));

    let mut channels: Vec<(&str, String)> = Vec::new();

    // Channel: a successful credentialed read's projection.
    let resource = source
        .read(&reference(port, "/doc"), &OperationGuard::new())
        .await
        .expect("a credentialed read succeeds");
    channels.push(("resource-debug", format!("{resource:?}")));

    // Channel: the refusal rendered when the credentialed origin redirects
    // across an origin boundary.
    let refusal = source
        .read(&reference(port, "/hop"), &OperationGuard::new())
        .await
        .expect_err("a credentialed cross-origin redirect is refused");
    channels.push(("error-message", refusal.message().to_owned()));
    channels.push(("error-display", refusal.to_string()));
    channels.push(("error-debug", format!("{refusal:?}")));

    // Channel: the credential's own rendering, the type that holds the value.
    channels.push(("credential-debug", format!("{:?}", credential(port))));

    for (name, value) in &channels {
        assert!(
            !value.is_empty(),
            "channel `{name}` rendered nothing, so scanning it for the sentinel \
             would report safety it did not establish"
        );
        assert!(
            !value.contains(SENTINEL),
            "channel `{name}` leaked the credential value: {value}"
        );
    }

    // The redirect target must not have been reached at all: refusing the hop
    // is what keeps the credential from crossing the origin boundary.
    settle().await;
    let crossed = elsewhere.heads();
    assert!(
        crossed.is_empty(),
        "the credentialed request must not have reached the second origin: {crossed:?}"
    );
}

/// C21 — a cancelled read abandons the request instead of running to the
/// request timeout.
///
/// Before this claim `SourceAdapter::read` took no guard at all, so
/// `HttpsSource` built a fresh always-active one internally and an `rfs_read`
/// could not be interrupted by any means. Threading the caller's guard through
/// the trait is what makes this row expressible.
///
/// The control row proves the trickling fixture completes when nothing cancels
/// it, so the short transfer in the cancelled row is attributable to
/// cancellation rather than to a fixture that never worked. The refusal's
/// **message** is asserted, not only its category, because a request timeout
/// occupies the same call position and would otherwise be indistinguishable.
#[tokio::test]
async fn cancelled_read_abandons_the_request() {
    // POSITIVE CONTROL — the same body, uncancelled, transferred whole.
    let control_listener = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let control_port = control_listener.address.port();
    let control = source(control_port, Vec::new())
        .read(&reference(control_port, "/doc:raw"), &OperationGuard::new())
        .await
        .expect("an uncancelled trickled read completes");
    settle().await;

    let control_flushed = control_listener.flushed();
    assert_eq!(
        control_flushed, TRICKLE_LEN,
        "the control must transfer the whole body, or `stopped short` below \
         measures nothing"
    );
    assert_eq!(
        control.content().len(),
        TRICKLE_LEN,
        "the control must retain the whole body it was served"
    );

    // The cancelled row — same fixture, cancelled while the body is arriving.
    let listener = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let port = listener.address.port();

    let guard = OperationGuard::new();
    let canceller = guard.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        canceller.cancel();
    });

    let refusal = source(port, Vec::new())
        .read(&reference(port, "/doc:raw"), &guard)
        .await
        .expect_err("a cancelled read must not return a document");
    settle().await;

    assert_eq!(
        refusal.category(),
        ErrorCategory::Cancelled,
        "a cancelled read reports `cancelled`: {}",
        refusal.message()
    );
    assert!(
        refusal.message().to_ascii_lowercase().contains("cancel"),
        "the refusal must name cancellation, since the request timeout occupies \
         the same call position: {}",
        refusal.message()
    );
    let flushed = listener.flushed();
    assert!(
        flushed < control_flushed,
        "the cancelled read must stop the server short of the body it served \
         uncancelled ({flushed} of {control_flushed} bytes)"
    );
}

/// C21 — the guard survives the hop `rfs_read` actually takes.
///
/// The fence above drives [`HttpsSource`] directly, which leaves the delegation
/// in [`CompiledSources::read`] unmeasured: a dispatcher that forwarded a fresh
/// always-active guard on the `Https` arm would keep every other test in the
/// workspace green while `rfs_read` silently ran to the full request timeout.
/// That is not hypothetical — it was verified by mutation, and nothing caught
/// it. The read engine calls this dispatcher, so fencing the hop here is what
/// ties the claim to the tool the claim is about.
#[tokio::test]
async fn cancellation_survives_source_dispatch() {
    let listener = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let port = listener.address.port();

    let workspace = TempDir::new().expect("workspace");
    let filesystem = FilesystemSource::new(
        LaunchRootSource::Cli(vec![LaunchRoot::read_only(
            WorkspaceRootId::new("workspace").expect("root id"),
            workspace.path().to_owned(),
        )]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");

    let cache = TempDir::new().expect("session cache");
    let store = SessionStore::open_with(
        SessionStorageConfig::new(cache.path(), SESSION_CLEANUP_TTL.as_secs() as i64)
            .expect("session storage config"),
    )
    .await
    .expect("session store");
    let session = store
        .create_session(ServerLimits::default())
        .await
        .expect("stored session");

    let compiled = CompiledSources::new(
        filesystem,
        ArtifactSource::new(session.path_session().clone()),
        LocalSource::new(session.path_session().clone()),
        Some(source(port, Vec::new())),
        None,
    )
    .await
    .expect("compiled sources");

    let guard = OperationGuard::new();
    let canceller = guard.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        canceller.cancel();
    });

    let refusal = compiled
        .read(&reference(port, "/doc:raw"), &guard)
        .await
        .expect_err("a cancelled dispatched read must not return a document");
    settle().await;

    assert_eq!(
        refusal.category(),
        ErrorCategory::Cancelled,
        "the dispatcher must forward the caller's guard, not a fresh one: {}",
        refusal.message()
    );
    assert!(
        listener.flushed() < TRICKLE_LEN,
        "the dispatched read must stop the server short of the whole body \
         ({} of {TRICKLE_LEN} bytes)",
        listener.flushed()
    );
}
