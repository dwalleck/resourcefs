//! The bounded HTTP substrate: the workspace's only HTTP client.
//!
//! Every network egress ResourceFS performs passes through this module, and
//! the security control has **two** entry points because the client reaches a
//! socket by two different routes.
//!
//! For a URL whose host is a **name**, the client is built with a
//! [`PolicyResolver`] that classifies each resolved address through core's
//! [`AddressPolicy`] and returns `Err` to deny, which prevents any socket being
//! opened. That placement is load-bearing rather than incidental —
//! `prove-it-prototype` established that this client calls the resolver once
//! per new connection and connects to exactly the addresses that call
//! returned, so for names resolution *is* the authorization point and there is
//! no "validate, then connect" window for a rebound name to slip through.
//!
//! For a URL whose host is an **IP literal** the resolver is never consulted
//! at all: the connector short-circuits, parsing the literal and connecting
//! straight away. An origin may legitimately be declared as an IP literal, so
//! that class is authorized by [`authorize_literal_host`] in the request path
//! *before* the client is invoked, and again for every redirect hop. Relying on
//! the resolver alone left `allow_private_network` silently unenforced for
//! every IP-literal origin — a socket to restricted space with the grant
//! withheld. The invariant this module owes its callers is "every address we
//! connect to was authorized", and it holds only because both routes are
//! covered. `http_policy_contract.rs` fences the literal route.
//!
//! The connection is pinned to a validated **address** while certificate
//! verification stays bound to the requested **hostname**. Both halves are
//! load-bearing and they pull in opposite directions, so the seam is easy to
//! break by accident: rewriting the request URL to the resolved address — the
//! naive reading of "connect to the address we authorized" — moves the
//! verification target onto the address and dismantles TLS identity, since any
//! certificate reachable at an authorized address would then be accepted. The
//! URL handed to the client must keep its hostname; the address is the
//! resolver's business alone. `http_tls_contract.rs` fences this.
//!
//! The request and response types are deliberately source-neutral: they name
//! no HTTPS-source type, so the GitHub and downstream-MCP sources can consume
//! the same audited egress path instead of building a second client.

mod extract;
mod read;
mod request;

pub(crate) use read::{BoundedRead, HttpReadBudget};
pub use request::HttpRequest;
use request::{HttpMethod, MAX_SOURCE_REQUEST_HEADER_BYTES, RedirectBehavior};
use reqwest::header;

use std::{
    collections::HashMap,
    fmt,
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use resourcefs_core::{
    AddressPolicy, AllowedOrigin, ErrorCategory, HttpCeilings, OperationGuard, OriginAllowlist,
    ResourceError, Secret,
};
use url::Url;

const RETRY_JITTER_MAX_MILLIS: u8 = 250;
const RETRY_JITTER_SAMPLE_BYTES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryAfter {
    Missing,
    Invalid,
    Delay(Duration),
}

impl RetryAfter {
    const fn delay(self) -> Option<Duration> {
        match self {
            Self::Delay(delay) => Some(delay),
            Self::Missing | Self::Invalid => None,
        }
    }
}

fn parse_retry_after_text(value: &str, now: SystemTime) -> RetryAfter {
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.parse::<u64>().map_or(RetryAfter::Invalid, |seconds| {
            RetryAfter::Delay(Duration::from_secs(seconds))
        });
    }
    let Ok(at) = httpdate::parse_http_date(value) else {
        return RetryAfter::Invalid;
    };
    match at.duration_since(now) {
        Ok(delay) => RetryAfter::Delay(delay),
        Err(_) => RetryAfter::Invalid,
    }
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap, now: SystemTime) -> RetryAfter {
    let mut values = headers.get_all(reqwest::header::RETRY_AFTER).iter();
    let Some(value) = values.next() else {
        return RetryAfter::Missing;
    };
    if values.next().is_some() || value.len() > MAX_SOURCE_REQUEST_HEADER_BYTES {
        return RetryAfter::Invalid;
    }
    match value.to_str() {
        Ok(value) => parse_retry_after_text(value, now),
        Err(_) => RetryAfter::Invalid,
    }
}

fn jitter_from_sample(sample: [u8; RETRY_JITTER_SAMPLE_BYTES]) -> Result<Duration, ResourceError> {
    sample
        .into_iter()
        .find(|millis| *millis <= RETRY_JITTER_MAX_MILLIS)
        .map(|millis| Duration::from_millis(u64::from(millis)))
        .ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "HTTP retry jitter could not be sampled without bias",
            )
        })
}

fn random_retry_jitter() -> Result<Duration, ResourceError> {
    let mut sample = [0_u8; RETRY_JITTER_SAMPLE_BYTES];
    getrandom::fill(&mut sample).map_err(|error| {
        ResourceError::new(
            ErrorCategory::SourceUnavailable,
            format!("HTTP retry jitter entropy was unavailable: {error}"),
        )
    })?;
    jitter_from_sample(sample)
}

fn retry_wait_fits(remaining: Duration, delay: Duration, jitter: Duration) -> bool {
    delay
        .checked_add(jitter)
        .is_some_and(|wait| wait < remaining)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LogicalDeadline(tokio::time::Instant);

impl LogicalDeadline {
    fn new(started: tokio::time::Instant, timeout: Duration) -> Result<Self, ResourceError> {
        started.checked_add(timeout).map(Self).ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::LimitExceeded,
                "HTTP logical read deadline is not representable",
            )
        })
    }

    fn remaining_at(self, now: tokio::time::Instant) -> Duration {
        self.0.saturating_duration_since(now)
    }

    fn remaining(self) -> Duration {
        self.remaining_at(tokio::time::Instant::now())
    }
}

pub(crate) struct HttpFetchFailure {
    error: ResourceError,
    retryable: bool,
    unknown_outcome: bool,
}

impl HttpFetchFailure {
    fn terminal(error: ResourceError) -> Self {
        Self {
            error,
            retryable: false,
            unknown_outcome: false,
        }
    }

    fn transport(error: ResourceError) -> Self {
        Self {
            error,
            retryable: true,
            unknown_outcome: true,
        }
    }

    fn unknown(error: ResourceError) -> Self {
        Self {
            error,
            retryable: false,
            unknown_outcome: true,
        }
    }

    pub(crate) const fn is_retryable(&self) -> bool {
        self.retryable
    }

    pub(crate) const fn is_unknown_outcome(&self) -> bool {
        self.unknown_outcome
    }

    pub(crate) fn into_error(self) -> ResourceError {
        self.error
    }
}
/// Resolves one host name to its addresses.
///
/// Injectable so contract tests can drive resolution deterministically without
/// a system resolver; the policy pass around it is identical either way, so
/// there is never a second resolution path that skips authorization.
type LookupFuture = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>>;
type HostLookup = Arc<dyn Fn(String) -> LookupFuture + Send + Sync>;

fn system_lookup() -> HostLookup {
    Arc::new(|host: String| {
        Box::pin(async move {
            // Port zero: the client substitutes the URL's port, so the lookup
            // only needs to produce addresses.
            let addresses = tokio::net::lookup_host((host.as_str(), 0u16)).await?;
            Ok(addresses.map(|address| address.ip()).collect())
        }) as LookupFuture
    })
}

/// Authorizes every resolved address before the client can connect to it.
#[derive(Clone)]
struct PolicyResolver {
    policies: HashMap<String, AddressPolicy>,
    lookup: HostLookup,
}

impl reqwest::dns::Resolve for PolicyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        let policy = self.policies.get(&host).copied();
        let lookup = Arc::clone(&self.lookup);
        Box::pin(async move {
            let Some(policy) = policy else {
                return Err(Box::new(ResourceError::new(
                    ErrorCategory::PermissionDenied,
                    "host is not declared by any allowlisted HTTPS origin",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            };
            let addresses = lookup(host).await.map_err(|error| {
                Box::new(ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    format!("host could not be resolved: {error}"),
                )) as Box<dyn std::error::Error + Send + Sync>
            })?;
            if addresses.is_empty() {
                return Err(Box::new(ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "host resolved to no addresses",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            // Every address must pass. A host resolving partly into restricted
            // space is the rebinding shape this control exists to stop, so the
            // whole resolution is refused rather than quietly connecting to
            // whichever address happened to be acceptable.
            let mut authorized = Vec::with_capacity(addresses.len());
            for address in addresses {
                policy
                    .authorize(address)
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?;
                authorized.push(SocketAddr::new(address, 0));
            }
            Ok(Box::new(authorized.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

impl HttpMethod {
    const fn reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Patch => reqwest::Method::PATCH,
        }
    }
}

/// One bounded response, carrying no source-specific type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedHttpResponse {
    status: u16,
    final_url: Url,
    content_type: Option<String>,
    etag: Option<String>,
    /// `Some(Err)` records a Link header that arrived but is not readable as
    /// text — present-but-corrupt, which pagination must not mistake for
    /// "no further pages".
    link: Option<Result<String, ResourceError>>,
    retry_after: RetryAfter,
    rate_limit_remaining: Option<u64>,
    body: Vec<u8>,
    truncated: bool,
}

impl BoundedHttpResponse {
    /// Returns the HTTP status code.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    /// Returns the URL the response was ultimately served from.
    #[must_use]
    pub const fn final_url(&self) -> &Url {
        &self.final_url
    }

    /// Returns the declared content type, when the response carried one.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Returns the entity validator retained for conditional reads.
    ///
    /// A validator that is not readable as text is reported as absent: one
    /// that cannot be echoed back in `If-None-Match` is no validator, and the
    /// consequence — an unconditional refetch — is the safe one. A plain read
    /// that never revalidates is therefore never failed by an odd ETag.
    #[must_use]
    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }

    /// Returns the pagination Link header exactly as received.
    ///
    /// A Link header that is present but not readable as text is an error
    /// rather than `None`: a continuation the caller cannot follow must never
    /// be mistaken for the last page. Callers that never paginate never ask,
    /// so an opaque Link costs a plain read nothing.
    pub fn link(&self) -> Result<Option<&str>, ResourceError> {
        match &self.link {
            None => Ok(None),
            Some(Ok(link)) => Ok(Some(link)),
            Some(Err(error)) => Err(error.clone()),
        }
    }

    /// Returns usable Retry-After guidance in either standard wire form.
    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        self.retry_after.delay()
    }

    /// Returns GitHub-style remaining request budget when numeric.
    #[must_use]
    pub const fn rate_limit_remaining(&self) -> Option<u64> {
        self.rate_limit_remaining
    }

    /// Returns the accepted body bytes.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Moves the accepted body into a source-owned cache without copying.
    #[must_use]
    pub fn into_body(self) -> Vec<u8> {
        self.body
    }

    /// Returns whether the body reached the accept ceiling and was cut short.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Counts extractor invocations so a contract test can prove extraction did
/// **not** run for an over-ceiling document.
///
/// Asserting only that the call failed would be satisfied by any refusal — the
/// allowlist, the resolver, a transport error — so it cannot distinguish "we
/// refused before extracting" from "we extracted and then something else went
/// wrong". Four fixtures in this change passed for the wrong reason on exactly
/// that class of assertion; this counter is the positive evidence.
/// Scoped to one substrate, deliberately, rather than to the process. A
/// process-global counter cannot be asserted for exact equality: libtest runs a
/// binary's tests on parallel threads, so a sibling test performing a legitimate
/// extraction would land between this test's `before` reading and its
/// assertion and fail it for no reason. Per-substrate makes the count a
/// property of the object under test.
#[cfg(feature = "test-support")]
type ExtractionCounter = Arc<std::sync::atomic::AtomicUsize>;

/// Extracts reader-mode Markdown, for contract tests that exercise the
/// extractor directly rather than through a network fetch.
#[cfg(feature = "test-support")]
pub fn extract_markdown_for_test(html: &str) -> String {
    extract::extract_markdown(html)
}

/// A reader-mode rendering of one complete HTML document.
#[derive(Debug, Clone)]
pub struct ReaderModeDocument {
    markdown: String,
    final_url: Url,
    status: u16,
}

impl ReaderModeDocument {
    /// Returns the extracted Markdown.
    pub fn markdown(&self) -> &str {
        &self.markdown
    }

    /// Returns the URL the document was finally read from.
    pub const fn final_url(&self) -> &Url {
        &self.final_url
    }

    /// Returns the HTTP status the document was served with.
    pub const fn status(&self) -> u16 {
        self.status
    }
}

/// Authorizes a URL whose host is an IP literal, before any connect.
///
/// The connector short-circuits DNS when the host parses as an address, so
/// [`PolicyResolver`] never sees this class and the per-origin
/// `allow_private_network` grant would go unenforced. A host that is a *name*
/// returns `Ok` here and is authorized by the resolver instead — this is a
/// second gate on a second route, never a replacement for the first.
///
/// The refusal is deliberately the same [`AddressPolicy`] message the resolver
/// produces: it is the same control refusing for the same reason, and an
/// operator reading it should not have to care which internal route reached it.
fn authorize_literal_host(
    policies: &HashMap<String, AddressPolicy>,
    url: &Url,
) -> Result<(), ResourceError> {
    let address = match url.host() {
        Some(url::Host::Ipv4(address)) => IpAddr::V4(address),
        Some(url::Host::Ipv6(address)) => IpAddr::V6(address),
        // A domain name, or no host at all: the resolver owns this route.
        _ => return Ok(()),
    };
    // Keyed by `host_str` exactly as `host_policies` builds it, so the two
    // spellings cannot drift (notably IPv6, which serializes bracketed).
    let policy = url
        .host_str()
        .and_then(|host| policies.get(host).copied())
        .ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::PermissionDenied,
                "host is not declared by any allowlisted HTTPS origin",
            )
        })?;
    policy.authorize(address)
}

/// One DER trust anchor accepted only by test-support HTTP construction.
///
/// Validation happens at construction so malformed certificate bytes cannot
/// reach either substrate client or any network operation.
#[cfg(feature = "test-support")]
pub struct TestRootCertificate {
    der: Box<[u8]>,
}

#[cfg(feature = "test-support")]
impl TestRootCertificate {
    pub fn from_der(der: &[u8]) -> Result<Self, ResourceError> {
        let certificate = parse_root_certificate(der)?;
        reqwest::Client::builder()
            .tls_certs_merge([certificate])
            .build()
            .map_err(trust_anchor_error)?;
        Ok(Self { der: der.into() })
    }

    fn as_der(&self) -> &[u8] {
        &self.der
    }
}

type RetryWallClock = Arc<dyn Fn() -> SystemTime + Send + Sync>;
type RetryJitter = Arc<dyn Fn() -> Result<Duration, ResourceError> + Send + Sync>;

struct RetryRuntime {
    wall_now: RetryWallClock,
    jitter: RetryJitter,
}

impl Default for RetryRuntime {
    fn default() -> Self {
        Self {
            wall_now: Arc::new(SystemTime::now),
            jitter: Arc::new(random_retry_jitter),
        }
    }
}

/// The workspace's single bounded HTTP egress point.
pub struct HttpSubstrate {
    client: reqwest::Client,
    mutation_client: reqwest::Client,
    allowlist: OriginAllowlist,
    ceilings: HttpCeilings,
    /// The same per-host policies the resolver holds, retained so the
    /// IP-literal route can apply them before the client is invoked.
    policies: HashMap<String, AddressPolicy>,
    /// Resolved per-origin credentials, matched by the same `authorizes`
    /// predicate the allowlist uses so two origins sharing a host cannot be
    /// confused for one another.
    credentials: Vec<OriginCredential>,
    /// Origins belonging to a source whose startup probe reported Degraded.
    ///
    /// These are *configured* — the operator declared them — so refusing them
    /// as "not allowlisted" would misdescribe the profile. They are held apart
    /// from the allowlist and refused as `source_unavailable`, which is the
    /// state that is actually true: the source exists and is unreachable.
    degraded: Vec<AllowedOrigin>,
    retry_runtime: RetryRuntime,
    #[cfg(feature = "test-support")]
    extractions: ExtractionCounter,
}

impl std::fmt::Debug for HttpSubstrate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpSubstrate")
            .field("origins", &self.allowlist.origins().len())
            .finish_non_exhaustive()
    }
}

/// One origin's resolved credential, applied at the single point of egress.
///
/// The value is a [`Secret`], which implements neither `Debug` nor `Display`,
/// so it cannot reach an observable channel without crossing `expose` at the
/// one call site that attaches it to a request.
pub struct OriginCredential {
    origin: AllowedOrigin,
    header: String,
    value: Secret,
}

impl OriginCredential {
    /// Resolves one origin's credential into the value sent on the wire.
    ///
    /// An auth scheme is folded into the value here (`Bearer <secret>`), so the
    /// composed string is itself a `Secret` and never exists as a plain
    /// `String` a caller could log.
    pub fn new(
        origin: AllowedOrigin,
        header: impl Into<String>,
        scheme: Option<&str>,
        secret: &Secret,
    ) -> Result<Self, ResourceError> {
        let composed = match scheme {
            Some(scheme) => format!("{scheme} {}", secret.expose()),
            None => secret.expose().to_owned(),
        };
        let value = Secret::new(composed).map_err(|_| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "origin credential could not be composed",
            )
        })?;
        Ok(Self {
            origin,
            header: header.into(),
            value,
        })
    }

    /// Composes an RFC 7617 Basic credential without exposing its value outside
    /// the audited credential boundary.
    pub fn basic(
        origin: AllowedOrigin,
        username: &str,
        secret: &Secret,
    ) -> Result<Self, ResourceError> {
        if username.is_empty() || username.contains(':') || username.chars().any(char::is_control) {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "Basic credential username must be non-empty and contain neither ':' nor control characters",
            ));
        }
        let raw_len = username
            .len()
            .checked_add(1)
            .and_then(|length| length.checked_add(secret.expose().len()))
            .ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::LimitExceeded,
                    "Basic credential exceeds the bounded request-header ceiling",
                )
            })?;
        let encoded_len = raw_len
            .checked_add(2)
            .and_then(|length| length.checked_div(3))
            .and_then(|length| length.checked_mul(4))
            .ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::LimitExceeded,
                    "Basic credential exceeds the bounded request-header ceiling",
                )
            })?;
        if encoded_len
            .checked_add("Basic ".len())
            .is_none_or(|length| length > MAX_SOURCE_REQUEST_HEADER_BYTES)
        {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                "Basic credential exceeds the bounded request-header ceiling",
            ));
        }

        let mut raw = Vec::with_capacity(raw_len);
        raw.extend_from_slice(username.as_bytes());
        raw.push(b':');
        raw.extend_from_slice(secret.expose().as_bytes());
        let mut composed = String::with_capacity("Basic ".len() + encoded_len);
        composed.push_str("Basic ");
        STANDARD.encode_string(&raw, &mut composed);
        raw.fill(0);
        let value = Secret::new(composed).map_err(|_| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "Basic origin credential could not be composed",
            )
        })?;
        Ok(Self {
            origin,
            header: "Authorization".to_owned(),
            value,
        })
    }

    /// Returns the origin this credential belongs to.
    #[must_use]
    pub const fn origin(&self) -> &AllowedOrigin {
        &self.origin
    }
}

impl fmt::Debug for OriginCredential {
    /// Names the header but never the value.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OriginCredential")
            .field("header", &self.header)
            .finish_non_exhaustive()
    }
}

fn trust_anchor_error(error: impl fmt::Display) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!("trust anchor could not be parsed: {error}"),
    )
}

fn parse_root_certificate(root: &[u8]) -> Result<reqwest::Certificate, ResourceError> {
    reqwest::Certificate::from_der(root).map_err(trust_anchor_error)
}

fn build_http_client(
    resolver: PolicyResolver,
    redirect: reqwest::redirect::Policy,
    ceilings: HttpCeilings,
    roots: &[&[u8]],
) -> Result<reqwest::Client, ResourceError> {
    let mut builder = reqwest::Client::builder()
        .dns_resolver(Arc::new(resolver))
        .redirect(redirect)
        .timeout(ceilings.timeout());
    for root in roots {
        builder = builder.tls_certs_merge([parse_root_certificate(root)?]);
    }
    builder.build().map_err(|error| {
        ResourceError::new(
            ErrorCategory::SourceUnavailable,
            format!("HTTP client could not be constructed: {error}"),
        )
    })
}

impl HttpSubstrate {
    /// Builds the substrate over the system resolver.
    pub fn new(
        allowlist: OriginAllowlist,
        ceilings: HttpCeilings,
        credentials: Vec<OriginCredential>,
    ) -> Result<Self, ResourceError> {
        Self::build(allowlist, ceilings, system_lookup(), &[], credentials)
    }

    /// Builds the policy-preserving substrate over the system resolver with
    /// one additional, already-validated fixture root.
    #[cfg(feature = "test-support")]
    pub fn with_system_lookup_and_root(
        allowlist: OriginAllowlist,
        ceilings: HttpCeilings,
        root: TestRootCertificate,
        credentials: Vec<OriginCredential>,
    ) -> Result<Self, ResourceError> {
        Self::build(
            allowlist,
            ceilings,
            system_lookup(),
            &[root.as_der()],
            credentials,
        )
    }

    /// Records origins whose source reported Degraded at startup.
    ///
    /// Builder-style rather than a constructor parameter: degradation is a
    /// launch-time observation, not part of what the substrate *is*, and every
    /// existing construction site describes a profile with no degraded source.
    #[must_use]
    pub fn with_degraded_origins(mut self, origins: Vec<AllowedOrigin>) -> Self {
        self.degraded = origins;
        self
    }

    /// Replaces wall-clock and jitter inputs for deterministic retry contracts.
    #[cfg(feature = "test-support")]
    pub fn with_retry_control_for_test(
        mut self,
        wall_now: SystemTime,
        jitter: Duration,
    ) -> Result<Self, ResourceError> {
        if jitter > Duration::from_millis(u64::from(RETRY_JITTER_MAX_MILLIS)) {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "test retry jitter exceeds the 250-millisecond production bound",
            ));
        }
        self.retry_runtime = RetryRuntime {
            wall_now: Arc::new(move || wall_now),
            jitter: Arc::new(move || Ok(jitter)),
        };
        Ok(self)
    }

    /// Builds the substrate over an injected host lookup, for contract tests.
    ///
    /// The address policy is applied identically to the production path; only
    /// the name-to-address step is substituted.
    #[cfg(feature = "test-support")]
    pub fn with_host_lookup<F, R>(
        allowlist: OriginAllowlist,
        ceilings: HttpCeilings,
        lookup: F,
    ) -> Result<Self, ResourceError>
    where
        F: Fn(String) -> R + Send + Sync + 'static,
        R: Future<Output = io::Result<Vec<IpAddr>>> + Send + 'static,
    {
        Self::with_host_lookup_and_roots(allowlist, ceilings, lookup, &[], Vec::new())
    }

    /// Builds the substrate over an injected host lookup that additionally
    /// trusts `roots`, for contract tests that terminate TLS locally.
    ///
    /// Each entry is one DER-encoded certificate added to the client's trust
    /// anchors. This exists so a fixture can present a certificate the client
    /// genuinely validates: trusting a fixture root keeps *chain* verification
    /// real, which is what makes a name-mismatch rejection attributable to the
    /// hostname rather than to an untrusted issuer. Certificate verification
    /// itself is never relaxed — there is deliberately no hook here for
    /// accepting invalid certificates or invalid hostnames, because the
    /// property the TLS contracts assert is exactly the one such a hook would
    /// disable.
    #[cfg(feature = "test-support")]
    pub fn with_host_lookup_and_roots<F, R>(
        allowlist: OriginAllowlist,
        ceilings: HttpCeilings,
        lookup: F,
        roots: &[&[u8]],
        credentials: Vec<OriginCredential>,
    ) -> Result<Self, ResourceError>
    where
        F: Fn(String) -> R + Send + Sync + 'static,
        R: Future<Output = io::Result<Vec<IpAddr>>> + Send + 'static,
    {
        Self::build(
            allowlist,
            ceilings,
            Arc::new(move |host| Box::pin(lookup(host)) as LookupFuture),
            roots,
            credentials,
        )
    }

    fn build(
        allowlist: OriginAllowlist,
        ceilings: HttpCeilings,
        lookup: HostLookup,
        roots: &[&[u8]],
        credentials: Vec<OriginCredential>,
    ) -> Result<Self, ResourceError> {
        let policies = host_policies(&allowlist);
        let resolver = PolicyResolver {
            policies: policies.clone(),
            lookup,
        };
        let client = build_http_client(
            resolver.clone(),
            redirect_policy(
                allowlist.clone(),
                policies.clone(),
                credentials
                    .iter()
                    .map(|credential| credential.origin().clone())
                    .collect(),
                ceilings.redirect_depth(),
            ),
            ceilings,
            roots,
        )?;
        let mutation_client =
            build_http_client(resolver, reqwest::redirect::Policy::none(), ceilings, roots)?;
        Ok(Self {
            client,
            mutation_client,
            allowlist,
            ceilings,
            policies,
            credentials,
            degraded: Vec::new(),
            retry_runtime: RetryRuntime::default(),
            #[cfg(feature = "test-support")]
            extractions: ExtractionCounter::default(),
        })
    }

    /// Refuses a URL belonging to a source that reported Degraded at startup.
    ///
    /// Uses the origin's own `authorizes` predicate, the same matcher the
    /// allowlist and the credential lookup use, so a degraded origin sharing a
    /// host with a healthy one is distinguished by path prefix rather than by
    /// host alone — a healthy source keeps serving when a sibling degrades.
    fn refuse_degraded(&self, url: &Url) -> Result<(), ResourceError> {
        if self.degraded.iter().any(|origin| origin.authorizes(url)) {
            return Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!(
                    "'{url}' belongs to a configured HTTPS source that was unreachable at startup"
                ),
            ));
        }
        Ok(())
    }

    /// Builds one request, attaching validated source headers and then the
    /// credential of the origin that owns this URL, if any.
    ///
    /// Source headers cannot name authority or framing fields, so only this
    /// method can cross `Secret::expose` and attach a credential.
    fn credentialed(&self, request: HttpRequest) -> reqwest::RequestBuilder {
        let HttpRequest {
            url,
            method,
            redirect,
            body,
            headers,
            ..
        } = request;
        let client = match redirect {
            RedirectBehavior::FollowReads => &self.client,
            RedirectBehavior::Refuse => &self.mutation_client,
        };
        let mut builder = client.request(method.reqwest(), url.clone());
        if let Some(body) = body {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        for header in headers {
            builder = builder.header(header.name, header.value);
        }
        for credential in &self.credentials {
            if credential.origin.authorizes(&url) {
                builder = builder.header(&credential.header, credential.value.expose());
                break;
            }
        }
        builder
    }

    /// Records one extractor invocation against this substrate.
    #[cfg(feature = "test-support")]
    fn record_extraction(&self) {
        self.extractions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(not(feature = "test-support"))]
    #[expect(
        clippy::unused_self,
        reason = "mirrors the test-support signature so the call site is identical"
    )]
    const fn record_extraction(&self) {}

    /// Returns how many times **this substrate** has run the extractor.
    #[cfg(feature = "test-support")]
    pub fn extraction_count(&self) -> usize {
        self.extractions.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Returns the ceilings this substrate enforces.
    #[must_use]
    pub const fn ceilings(&self) -> HttpCeilings {
        self.ceilings
    }

    /// Starts one bounded logical read using this substrate's configured timeout.
    pub(crate) fn begin_read<'a>(
        &'a self,
        operation: &'a OperationGuard,
    ) -> Result<BoundedRead<'a>, ResourceError> {
        Ok(BoundedRead {
            substrate: self,
            operation,
            deadline: LogicalDeadline::new(tokio::time::Instant::now(), self.ceilings.timeout())?,
        })
    }

    /// Performs one bounded logical fetch.
    ///
    /// A GET may make exactly one follow-up for a conclusively retryable
    /// transport failure or a 429/503 carrying usable Retry-After guidance.
    /// POST and PATCH remain single-attempt.
    pub async fn fetch(
        &self,
        request: HttpRequest,
        operation: &OperationGuard,
    ) -> Result<BoundedHttpResponse, ResourceError> {
        self.begin_read(operation)?.fetch(request).await
    }

    pub(crate) async fn fetch_attempt(
        &self,
        request: HttpRequest,
        operation: &OperationGuard,
    ) -> Result<BoundedHttpResponse, HttpFetchFailure> {
        if !operation.is_active() && !operation.is_committing() {
            return Err(HttpFetchFailure::terminal(ResourceError::new(
                ErrorCategory::Cancelled,
                "request was cancelled before egress",
            )));
        }
        let request_url = request.url().clone();
        self.refuse_degraded(&request_url)
            .map_err(HttpFetchFailure::terminal)?;
        self.allowlist
            .authorize(&request_url)
            .map_err(HttpFetchFailure::terminal)?;
        authorize_literal_host(&self.policies, &request_url).map_err(HttpFetchFailure::terminal)?;

        let response = tokio::select! {
            biased;
            () = operation.cancelled() => {
                return Err(HttpFetchFailure::unknown(cancelled_mid_request()));
            }
            sent = self.credentialed(request).send() => {
                sent.map_err(classify_reqwest_failure)?
            }
        };

        let status = response.status().as_u16();
        let final_url = Url::parse(response.url().as_str()).unwrap_or(request_url);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        // Retained metadata has a hard byte ceiling on every fetch, but text
        // strictness is per header: see `etag()` and `link()` for why one
        // degrades to absence and the other to an error at the point of use.
        let etag = match Self::header_within_ceiling(response.headers(), &reqwest::header::ETAG) {
            Ok(value) => value
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            // ETag is optional optimization, never response authority. A value
            // too large to echo safely is therefore the same as no validator.
            Err(error) if error.category() == ErrorCategory::LimitExceeded => None,
            Err(error) => return Err(HttpFetchFailure::unknown(error)),
        };
        let link = Self::header_within_ceiling(response.headers(), &reqwest::header::LINK)
            .map_err(HttpFetchFailure::unknown)?
            .map(|value| {
                value.to_str().map(str::to_owned).map_err(|_| {
                    ResourceError::new(
                        ErrorCategory::SourceUnavailable,
                        "HTTP response header 'link' is not valid text",
                    )
                })
            });
        let retry_after = if response
            .headers()
            .contains_key(reqwest::header::RETRY_AFTER)
        {
            parse_retry_after(response.headers(), (self.retry_runtime.wall_now)())
        } else {
            RetryAfter::Missing
        };
        let rate_limit_remaining = response
            .headers()
            .get(reqwest::header::HeaderName::from_static(
                "x-ratelimit-remaining",
            ))
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());

        let (body, truncated) = self
            .read_bounded(response, operation)
            .await
            .map_err(HttpFetchFailure::unknown)?;
        Ok(BoundedHttpResponse {
            status,
            final_url,
            content_type,
            etag,
            link,
            retry_after,
            rate_limit_remaining,
            body,
            truncated,
        })
    }

    /// Returns a response header the substrate may retain, refusing one over
    /// the metadata ceiling before any caller can copy it.
    fn header_within_ceiling<'a>(
        headers: &'a reqwest::header::HeaderMap,
        name: &reqwest::header::HeaderName,
    ) -> Result<Option<&'a reqwest::header::HeaderValue>, ResourceError> {
        let Some(value) = headers.get(name) else {
            return Ok(None);
        };
        if value.len() > MAX_SOURCE_REQUEST_HEADER_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "HTTP response header '{name}' exceeds the {MAX_SOURCE_REQUEST_HEADER_BYTES}-byte retained metadata ceiling"
                ),
            ));
        }
        Ok(Some(value))
    }

    /// Performs one authorized request and returns its reader-mode Markdown.
    ///
    /// The ceiling check runs **strictly before** extraction. An over-ceiling
    /// document is refused outright rather than extracted from what arrived,
    /// because extraction over truncated markup can silently drop or mangle
    /// structure — an unclosed element swallows the visible content — and the
    /// caller has no way to tell a faithful rendering from a mangled one. The
    /// refusal names `:raw` with a selector, which remains available for a
    /// bounded slice of an oversized document.
    pub async fn fetch_reader_mode(
        &self,
        request: HttpRequest,
        operation: &OperationGuard,
    ) -> Result<ReaderModeDocument, ResourceError> {
        let response = self.fetch(request, operation).await?;
        if response.truncated() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "document exceeds the {}-byte fetch ceiling; read it with :raw and a \
                     selector for a bounded slice",
                    self.ceilings.fetch_bytes()
                ),
            ));
        }
        let html = reader_mode_source(&response)?;
        self.record_extraction();
        Ok(ReaderModeDocument {
            markdown: extract::extract_markdown(&html),
            final_url: response.final_url,
            status: response.status,
        })
    }

    /// Reads a response body, stopping once the accept ceiling is reached or
    /// the operation is cancelled.
    ///
    /// The ceiling bounds what ResourceFS accepts and retains, not what the
    /// peer transmits — socket and client buffers sit in between, and the
    /// probe measured a peer flushing well past a smaller client ceiling.
    /// `http_bounds_contract.rs` fences the retained bound (C16) and records
    /// the server's flushed count as an observation rather than asserting the
    /// stronger guarantee. Reader mode adds its own rule on top: `fetch_reader_mode`
    /// refuses an over-ceiling document outright, so extraction never runs over
    /// a truncated one.
    ///
    /// Cancellation is checked per chunk rather than once at the end, so a
    /// caller who gave up stops paying for a body still arriving. Dropping the
    /// response mid-stream is what tells the peer to stop, which the fixtures
    /// observe as a server that flushed only part of what it meant to send.
    /// Unlike a mutation, a read has no commit to mask (rfs-73dz): HTTPS is
    /// read-only, so cancellation is simply honoured wherever it lands.
    async fn read_bounded(
        &self,
        mut response: reqwest::Response,
        operation: &OperationGuard,
    ) -> Result<(Vec<u8>, bool), ResourceError> {
        let ceiling = self.ceilings.fetch_bytes();
        let mut body = Vec::new();
        let mut truncated = false;
        loop {
            let chunk = tokio::select! {
                biased;
                () = operation.cancelled() => return Err(cancelled_mid_request()),
                chunk = response.chunk() => chunk.map_err(policy_error_or)?,
            };
            let Some(chunk) = chunk else { break };
            // Strictly greater, not `>=`: a body of exactly the ceiling was
            // fully accepted and is not truncated. Treating it as truncated
            // would refuse a document the signed spec says must succeed, and
            // the ceiling/ceiling+1 boundary is precisely what C8 fences.
            if body.len() + chunk.len() > ceiling {
                let remaining = ceiling.saturating_sub(body.len());
                body.extend_from_slice(&chunk[..remaining]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        Ok((body, truncated))
    }
}

/// Returns a response's body as HTML source, or refuses reader mode for it.
///
/// Reader mode is a rendering of an HTML document, and three inputs are not
/// that: a declared non-HTML media type, a declared non-UTF-8 charset, and
/// bytes that are not valid UTF-8. Each was previously run through
/// `String::from_utf8_lossy` and fed to the tokenizer, so a PDF or an
/// ISO-8859-1 page produced replacement-character Markdown that returned `Ok`
/// and was indistinguishable from a faithful rendering — and that output is
/// what a content-derived Version Tag hashes.
///
/// Refusing is the same choice the signed spec already made for an over-ceiling
/// document: fail loudly rather than hand back something plausible and wrong.
/// `:raw` remains available and returns the original bytes, which is the
/// supported way to read any of these.
///
/// A response with no declared media type is treated as HTML. Servers omit the
/// header routinely and reader mode is the default projection, so refusing on
/// absence would break ordinary pages; a body that is not UTF-8 is still caught
/// by the final check.
fn reader_mode_source(response: &BoundedHttpResponse) -> Result<String, ResourceError> {
    if let Some(content_type) = response.content_type() {
        let mut parts = content_type.split(';').map(str::trim);
        let media = parts.next().unwrap_or_default().to_ascii_lowercase();
        if !media.is_empty() && !matches!(media.as_str(), "text/html" | "application/xhtml+xml") {
            return Err(unsupported_reader_mode(&format!(
                "response declares media type {media}, which reader mode does not render"
            )));
        }
        for parameter in parts {
            let Some((name, value)) = parameter.split_once('=') else {
                continue;
            };
            if !name.trim().eq_ignore_ascii_case("charset") {
                continue;
            }
            let charset = value.trim().trim_matches('"').to_ascii_lowercase();
            if !matches!(charset.as_str(), "utf-8" | "utf8" | "us-ascii" | "ascii") {
                return Err(unsupported_reader_mode(&format!(
                    "response declares charset {charset}, which reader mode does not decode"
                )));
            }
        }
    }
    String::from_utf8(response.body().to_vec())
        .map_err(|_| unsupported_reader_mode("response body is not valid UTF-8"))
}

/// The refusal shared by every reader-mode input that is not an HTML document.
///
/// One constructor so the three rejection paths cannot drift into naming
/// different recoveries, and so the message always carries the `:raw` escape
/// hatch the signed spec provides.
fn unsupported_reader_mode(detail: &str) -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        format!("{detail}; read it with :raw for the original bytes"),
    )
}

/// Builds the redirect policy that authorizes every hop before it is sent.
///
/// Each hop is re-checked against the same [`OriginAllowlist`] the initial
/// request passed, so a redirect cannot walk out of the allowlisted origin —
/// the classic way an allowlisted host is used as an open redirector to reach
/// something it was never permitted to. The check runs inside the policy, which
/// the client consults *before* transmitting the next request, so a refused
/// target is never put on the wire at all rather than being requested and
/// discarded.
///
/// Refusal is expressed with `attempt.error(..)` rather than `attempt.stop()`.
/// The two differ in what the caller observes: `stop()` hands the caller the
/// redirect response itself, so a refused hop would surface as a successful
/// `302`, while the approved behavior is a `permission_denied` failure. The
/// error is carried through the client's own error type and recovered by
/// [`policy_error_or`].
///
/// Depth uses the same boundary as the client's built-in limit — `previous`
/// holds the original URL plus every hop already followed, so `> depth` admits
/// exactly `depth` redirects and refuses the next.
fn redirect_policy(
    allowlist: OriginAllowlist,
    policies: HashMap<String, AddressPolicy>,
    credentialed: Vec<AllowedOrigin>,
    depth: usize,
) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        // A credential is attached to the request builder, and the client
        // re-sends builder headers on every hop. Following a redirect out of
        // the origin the credential belongs to would therefore hand that
        // origin's secret to a different server — allowlisted or not. The hop
        // is refused rather than stripped, because a silently de-authenticated
        // request usually fails in a way that reads as a server fault.
        if let Some(first) = attempt.previous().first() {
            for origin in &credentialed {
                if origin.authorizes(first) && !origin.authorizes(attempt.url()) {
                    return attempt.error(ResourceError::new(
                        ErrorCategory::PermissionDenied,
                        "redirect was not followed: it would carry this origin's \
                         credential outside the origin that owns it",
                    ));
                }
            }
        }
        if attempt.previous().len() > depth {
            return attempt.error(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!("redirect chain exceeded the {depth}-hop ceiling"),
            ));
        }
        // A hop onto an IP literal reaches the connector's short-circuit just
        // as an initial request does, so the address policy is applied to the
        // hop here. Without this an allowlisted origin could redirect into
        // restricted space with the grant withheld.
        if let Err(refusal) = authorize_literal_host(&policies, attempt.url()) {
            let category = refusal.category();
            let detail = refusal.message().to_owned();
            return attempt.error(ResourceError::new(
                category,
                format!("redirect was not followed: {detail}"),
            ));
        }
        if let Err(refusal) = allowlist.authorize(attempt.url()) {
            // The underlying reason is preserved, but prefixed so the message
            // names the control that fired. Origin scoping refuses the initial
            // URL with the same category, and a caller that could not tell the
            // two apart would not know whether to fix the reference or the
            // profile.
            let category = refusal.category();
            let detail = refusal.message().to_owned();
            return attempt.error(ResourceError::new(
                category,
                format!("redirect was not followed: {detail}"),
            ));
        }
        attempt.follow()
    })
}

/// Recovers a policy refusal from a transport error, or reports the transport.
///
/// A denial raised inside the resolver reaches the caller wrapped in the
/// client's own error type, so the chain is walked for the original
/// [`ResourceError`]; without this a `permission_denied` would surface as an
/// opaque connection failure and the operator would not learn which policy
/// refused.
/// The refusal for an operation cancelled after egress began.
///
/// Named rather than inlined so the connect and body paths cannot drift into
/// reporting the same event differently, and so the message identifies
/// cancellation specifically: a timeout and a transport failure share this
/// call's failure position, and a fixture asserting only a category could not
/// tell them apart.
fn cancelled_mid_request() -> ResourceError {
    ResourceError::new(
        ErrorCategory::Cancelled,
        "request was cancelled before its response was accepted",
    )
}

fn classify_reqwest_failure(error: reqwest::Error) -> HttpFetchFailure {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = source {
        if let Some(found) = current.downcast_ref::<ResourceError>() {
            return HttpFetchFailure::terminal(found.clone());
        }
        source = current.source();
    }
    HttpFetchFailure::transport(policy_error_or(error))
}

fn policy_error_or(error: reqwest::Error) -> ResourceError {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = source {
        if let Some(found) = current.downcast_ref::<ResourceError>() {
            return found.clone();
        }
        source = current.source();
    }
    if error.is_timeout() {
        return ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "request exceeded the configured timeout",
        );
    }
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!("request failed: {error}"),
    )
}

/// Collapses the allowlist into one address policy per host.
///
/// When several origins share a host, the most restrictive grant wins: a
/// permissive sibling path must not silently widen a stricter one, because the
/// resolver sees only the host and cannot tell the two apart.
fn host_policies(allowlist: &OriginAllowlist) -> HashMap<String, AddressPolicy> {
    let mut policies: HashMap<String, bool> = HashMap::new();
    for origin in allowlist.origins() {
        if let Some(host) = origin.base_url().host_str() {
            let entry = policies.entry(host.to_owned()).or_insert(true);
            *entry &= origin.allow_private_network();
        }
    }
    policies
        .into_iter()
        .map(|(host, allow)| (host, AddressPolicy::new(allow)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        RetryAfter, jitter_from_sample, parse_retry_after, parse_retry_after_text, retry_wait_fits,
    };

    #[test]
    fn retry_after_delta_and_http_date_have_one_typed_delay() {
        let before_known_date = UNIX_EPOCH + Duration::from_secs(784_111_776);

        assert_eq!(
            parse_retry_after_text("0", before_known_date),
            RetryAfter::Delay(Duration::ZERO)
        );
        assert_eq!(
            parse_retry_after_text("7", before_known_date),
            RetryAfter::Delay(Duration::from_secs(7))
        );
        assert_eq!(
            parse_retry_after_text("Sun, 06 Nov 1994 08:49:37 GMT", before_known_date),
            RetryAfter::Delay(Duration::from_secs(1))
        );
    }

    #[test]
    fn unusable_retry_after_never_becomes_zero_delay() {
        let known_date = UNIX_EPOCH + Duration::from_secs(784_111_777);
        for value in [
            "",
            "-1",
            "18446744073709551616",
            "+1",
            " 1",
            "1 ",
            "not-a-date",
            "Sun, 06 Nov 1994 08:49:36 GMT",
            "0, 1",
        ] {
            assert_eq!(
                parse_retry_after_text(value, known_date),
                RetryAfter::Invalid,
                "{value:?}"
            );
        }
    }

    #[test]
    fn retry_jitter_is_bounded_unbiased_and_deadline_checked() {
        assert_eq!(
            jitter_from_sample([0, 251, 252, 253, 254, 255, 251, 252])
                .expect("zero is an accepted sample"),
            Duration::ZERO
        );
        assert_eq!(
            jitter_from_sample([251, 252, 137, 253, 254, 255, 251, 252])
                .expect("the first in-range byte is accepted"),
            Duration::from_millis(137)
        );
        assert_eq!(
            jitter_from_sample([251, 252, 253, 254, 255, 250, 251, 252]).expect("250 is included"),
            Duration::from_millis(250)
        );
        let failure = jitter_from_sample([251; 8]).expect_err("no biased fallback");
        assert_eq!(failure.category(), ErrorCategory::SourceUnavailable);

        assert!(retry_wait_fits(
            Duration::from_millis(251),
            Duration::ZERO,
            Duration::from_millis(250)
        ));
        assert!(!retry_wait_fits(
            Duration::from_millis(250),
            Duration::ZERO,
            Duration::from_millis(250)
        ));
        assert!(!retry_wait_fits(
            Duration::MAX,
            Duration::MAX,
            Duration::from_millis(1)
        ));
    }

    #[test]
    fn logical_read_never_refreshes_its_deadline() {
        let started = tokio::time::Instant::now();
        let deadline = super::LogicalDeadline::new(started, Duration::from_secs(5))
            .expect("representable deadline");

        assert_eq!(
            deadline.remaining_at(started + Duration::from_secs(2)),
            Duration::from_secs(3)
        );
        assert_eq!(
            deadline.remaining_at(started + Duration::from_secs(4)),
            Duration::from_secs(1)
        );
    }

    use resourcefs_core::ErrorCategory;

    #[test]
    fn retry_after_equal_http_date_is_zero_delay() {
        let known_date = SystemTime::UNIX_EPOCH + Duration::from_secs(784_111_777);
        assert_eq!(
            parse_retry_after_text("Sun, 06 Nov 1994 08:49:37 GMT", known_date),
            RetryAfter::Delay(Duration::ZERO)
        );
    }

    #[test]
    #[ignore = "checkpointed-build production-scale budget"]
    fn retry_policy_budget() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"),
        );
        let now = UNIX_EPOCH + Duration::from_secs(784_111_776);
        let iterations = 100_000_u32;
        let started = std::time::Instant::now();
        for _ in 0..iterations {
            let guidance = parse_retry_after(&headers, now);
            let jitter = jitter_from_sample([251, 252, 250, 253, 254, 255, 251, 252])
                .expect("sample contains 250");
            std::hint::black_box(retry_wait_fits(
                Duration::from_secs(2),
                guidance.delay().expect("known date is one second ahead"),
                jitter,
            ));
        }
        let average = started.elapsed() / iterations;
        assert!(
            average <= Duration::from_millis(1),
            "worst-case retry policy took {average:?}"
        );
    }
}
