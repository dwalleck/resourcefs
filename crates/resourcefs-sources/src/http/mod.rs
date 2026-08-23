//! The bounded HTTP substrate: the workspace's only HTTP client.
//!
//! Every network egress ResourceFS performs passes through this module, and
//! the security control lives in one specific place: the DNS resolver. The
//! client is built with a [`PolicyResolver`] that classifies each resolved
//! address through core's [`AddressPolicy`] and returns `Err` to deny, which
//! prevents any socket being opened. That placement is load-bearing rather
//! than incidental — `prove-it-prototype` established that this client calls
//! the resolver once per new connection and connects to exactly the addresses
//! that call returned, so resolution *is* the authorization point and there is
//! no "validate, then connect" window for a rebound name to slip through.
//!
//! The request and response types are deliberately source-neutral: they name
//! no HTTPS-source type, so the GitHub and downstream-MCP sources can consume
//! the same audited egress path instead of building a second client.

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

/// The workspace's single bounded HTTP egress point.
pub struct HttpSubstrate {
    client: reqwest::Client,
    allowlist: OriginAllowlist,
    ceilings: HttpCeilings,
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
        Self::build(allowlist, ceilings, system_lookup())
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
        Self::build(
            allowlist,
            ceilings,
            Arc::new(move |host| Box::pin(lookup(host)) as LookupFuture),
        )
    }

    fn build(
        allowlist: OriginAllowlist,
        ceilings: HttpCeilings,
        lookup: HostLookup,
    ) -> Result<Self, ResourceError> {
        let resolver = PolicyResolver {
            policies: host_policies(&allowlist),
            lookup,
        };
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(resolver))
            // Redirect authorization is rfs-g2z9 Slice 4's claim (C4/C5).
            // Until it lands the substrate follows no redirect at all, so the
            // interim posture is deny rather than an unauthorized hop.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(ceilings.timeout())
            .build()
            .map_err(|error| {
                ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    format!("HTTP client could not be constructed: {error}"),
                )
            })?;
        Ok(Self {
            client,
            allowlist,
            ceilings,
        })
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

        let response = self
            .client
            .get(request.url().clone())
            .send()
            .await
            .map_err(policy_error_or)?;

        let status = response.status().as_u16();
        let final_url = Url::parse(response.url().as_str()).unwrap_or_else(|_| request.url.clone());
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let (body, truncated) = self.read_bounded(response).await?;
        Ok(BoundedHttpResponse {
            status,
            final_url,
            content_type,
            body,
            truncated,
        })
    }

    /// Reads a response body, stopping once the accept ceiling is reached.
    ///
    /// The ceiling bounds what ResourceFS accepts and retains, not what the
    /// peer transmits — socket and client buffers sit in between, and the
    /// probe measured a peer flushing well past a smaller client ceiling. The
    /// ceiling's own fence (C8, including the rule that extraction never runs
    /// over a truncated document) belongs to rfs-g2z9 Slice 7; this bound exists so the
    /// substrate never performs an unbounded read in the meantime.
    async fn read_bounded(
        &self,
        mut response: reqwest::Response,
    ) -> Result<(Vec<u8>, bool), ResourceError> {
        let ceiling = self.ceilings.fetch_bytes();
        let mut body = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = response.chunk().await.map_err(policy_error_or)? {
            let remaining = ceiling.saturating_sub(body.len());
            if chunk.len() >= remaining {
                body.extend_from_slice(&chunk[..remaining]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        Ok((body, truncated))
    }
}

/// Recovers a policy refusal from a transport error, or reports the transport.
///
/// A denial raised inside the resolver reaches the caller wrapped in the
/// client's own error type, so the chain is walked for the original
/// [`ResourceError`]; without this a `permission_denied` would surface as an
/// opaque connection failure and the operator would not learn which policy
/// refused.
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
