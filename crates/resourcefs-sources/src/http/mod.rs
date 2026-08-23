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

use std::{
    collections::HashMap,
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
};

use resourcefs_core::{
    AddressPolicy, ErrorCategory, HttpCeilings, OperationGuard, OriginAllowlist, ResourceError,
};
use url::Url;

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

/// One source-neutral request for the substrate to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    url: Url,
}

impl HttpRequest {
    /// Builds a GET request for one absolute URL.
    #[must_use]
    pub const fn get(url: Url) -> Self {
        Self { url }
    }

    /// Returns the requested URL.
    #[must_use]
    pub const fn url(&self) -> &Url {
        &self.url
    }
}

/// One bounded response, carrying no source-specific type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedHttpResponse {
    status: u16,
    final_url: Url,
    content_type: Option<String>,
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

    /// Returns the accepted body bytes.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
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

/// The workspace's single bounded HTTP egress point.
pub struct HttpSubstrate {
    client: reqwest::Client,
    allowlist: OriginAllowlist,
    ceilings: HttpCeilings,
    /// The same per-host policies the resolver holds, retained so the
    /// IP-literal route can apply them before the client is invoked.
    policies: HashMap<String, AddressPolicy>,
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

impl HttpSubstrate {
    /// Builds the substrate over the system resolver.
    pub fn new(allowlist: OriginAllowlist, ceilings: HttpCeilings) -> Result<Self, ResourceError> {
        Self::build(allowlist, ceilings, system_lookup(), &[])
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
        Self::with_host_lookup_and_roots(allowlist, ceilings, lookup, &[])
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
        )
    }

    fn build(
        allowlist: OriginAllowlist,
        ceilings: HttpCeilings,
        lookup: HostLookup,
        roots: &[&[u8]],
    ) -> Result<Self, ResourceError> {
        let policies = host_policies(&allowlist);
        let resolver = PolicyResolver {
            policies: policies.clone(),
            lookup,
        };
        let mut builder = reqwest::Client::builder()
            .dns_resolver(Arc::new(resolver))
            .redirect(redirect_policy(
                allowlist.clone(),
                policies.clone(),
                ceilings.redirect_depth(),
            ))
            .timeout(ceilings.timeout());
        for root in roots {
            let certificate = reqwest::Certificate::from_der(root).map_err(|error| {
                ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    format!("trust anchor could not be parsed: {error}"),
                )
            })?;
            builder = builder.tls_certs_merge([certificate]);
        }
        let client = builder.build().map_err(|error| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("HTTP client could not be constructed: {error}"),
            )
        })?;
        Ok(Self {
            client,
            allowlist,
            ceilings,
            policies,
            #[cfg(feature = "test-support")]
            extractions: ExtractionCounter::default(),
        })
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

    /// Performs one authorized request and returns its bounded response.
    pub async fn fetch(
        &self,
        request: HttpRequest,
        operation: &OperationGuard,
    ) -> Result<BoundedHttpResponse, ResourceError> {
        if !operation.is_active() {
            return Err(ResourceError::new(
                ErrorCategory::Cancelled,
                "request was cancelled before egress",
            ));
        }
        // Origin scoping runs before any egress: a URL outside every declared
        // `base_url` never reaches the resolver, let alone a socket.
        self.allowlist.authorize(request.url())?;
        // The IP-literal route bypasses the resolver entirely, so the address
        // policy is applied here instead. Names fall through untouched and are
        // authorized inside the resolver.
        authorize_literal_host(&self.policies, request.url())?;

        // Cancellation races the connect the same way it races the body: a
        // caller who gave up should not wait on a handshake to a slow peer.
        let response = tokio::select! {
            biased;
            () = operation.cancelled() => return Err(cancelled_mid_request()),
            sent = self.client.get(request.url().clone()).send() => {
                sent.map_err(policy_error_or)?
            }
        };

        let status = response.status().as_u16();
        let final_url = Url::parse(response.url().as_str()).unwrap_or_else(|_| request.url.clone());
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let (body, truncated) = self.read_bounded(response, operation).await?;
        Ok(BoundedHttpResponse {
            status,
            final_url,
            content_type,
            body,
            truncated,
        })
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
    depth: usize,
) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
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
