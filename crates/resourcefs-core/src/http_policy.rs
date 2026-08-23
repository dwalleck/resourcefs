//! Source-neutral HTTP policy: address classification, origin allowlisting, and
//! transport ceilings.
//!
//! These are pure decision types. They hold no client, open no socket, and add
//! no dependency beyond `url` (already a workspace dependency) and `std::net`,
//! so every security decision this source makes is unit-testable without a
//! network. The transport that consumes them lives in `resourcefs-sources`.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use url::Url;

use crate::{ErrorCategory, ResourceError, limits::validate_limit};

/// Largest response body admitted before reader-mode extraction runs.
pub const MAX_HTTP_FETCH_BYTES: usize = 8 * 1024 * 1024;

/// Largest redirect chain followed for one request.
pub const MAX_HTTP_REDIRECT_DEPTH: usize = 5;

/// Longest total duration admitted for one request, in milliseconds.
pub const MAX_HTTP_TIMEOUT_MILLIS: usize = 30_000;

/// Classification of one resolved address against the special-purpose registries.
///
/// Every variant except [`AddressClass::Public`] is restricted: reaching it
/// requires the origin's distinct private-network grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressClass {
    /// An ordinary globally routable address.
    Public,
    /// Loopback: `127.0.0.0/8`, `::1`.
    Loopback,
    /// RFC1918 private space: `10/8`, `172.16/12`, `192.168/16`.
    Private,
    /// Link-local: `169.254.0.0/16`, `fe80::/10`.
    LinkLocal,
    /// RFC6598 carrier-grade NAT space: `100.64.0.0/10`.
    SharedCgnat,
    /// Multicast: `224.0.0.0/4`, `ff00::/8`.
    Multicast,
    /// The unspecified address, and the rest of `0.0.0.0/8`.
    Unspecified,
    /// RFC4193 unique-local v6: `fc00::/7`.
    UniqueLocal,
    /// Documentation ranges reserved by RFC5737 and RFC3849.
    Documentation,
    /// RFC2544 benchmarking space: `198.18.0.0/15`.
    Benchmarking,
    /// Reserved or broadcast space with no ordinary unicast use.
    Reserved,
}

impl AddressClass {
    /// Classifies one resolved address.
    ///
    /// IPv6 addresses embedding an IPv4 address classify by their embedded
    /// IPv4 rules. Treating `::ffff:10.0.0.1` as an ordinary public v6 address
    /// is a well-known SSRF bypass, so the embedding is unwrapped before the
    /// v6-specific ranges are consulted.
    #[must_use]
    pub const fn classify(address: IpAddr) -> Self {
        match address {
            IpAddr::V4(address) => Self::classify_v4(address),
            IpAddr::V6(address) => Self::classify_v6(address),
        }
    }

    /// Returns whether reaching this address requires the private-network grant.
    #[must_use]
    pub const fn is_restricted(self) -> bool {
        !matches!(self, Self::Public)
    }

    const fn classify_v4(address: Ipv4Addr) -> Self {
        let octets = address.octets();
        if octets[0] == 0 {
            return Self::Unspecified;
        }
        if address.is_loopback() {
            return Self::Loopback;
        }
        if address.is_private() {
            return Self::Private;
        }
        if address.is_link_local() {
            return Self::LinkLocal;
        }
        // RFC6598 100.64.0.0/10.
        if octets[0] == 100 && octets[1] >= 64 && octets[1] <= 127 {
            return Self::SharedCgnat;
        }
        if address.is_multicast() {
            return Self::Multicast;
        }
        if address.is_documentation() {
            return Self::Documentation;
        }
        // RFC2544 198.18.0.0/15.
        if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
            return Self::Benchmarking;
        }
        // RFC1112 240.0.0.0/4, which subsumes the broadcast address.
        if octets[0] >= 240 {
            return Self::Reserved;
        }
        Self::Public
    }

    const fn classify_v6(address: Ipv6Addr) -> Self {
        // Checked before the embedded-IPv4 unwrap so these keep accurate labels:
        // `::1` and `::` are otherwise indistinguishable from IPv4-compatible
        // forms of `0.0.0.1` and `0.0.0.0`.
        if address.is_unspecified() {
            return Self::Unspecified;
        }
        if address.is_loopback() {
            return Self::Loopback;
        }
        let segments = address.segments();
        // IPv4-mapped (`::ffff:a.b.c.d`) and the deprecated IPv4-compatible
        // (`::a.b.c.d`) forms both carry an embedded v4 address.
        if segments[0] == 0
            && segments[1] == 0
            && segments[2] == 0
            && segments[3] == 0
            && segments[4] == 0
            && (segments[5] == 0 || segments[5] == 0xffff)
        {
            let octets = address.octets();
            return Self::classify_v4(Ipv4Addr::new(
                octets[12], octets[13], octets[14], octets[15],
            ));
        }
        if address.is_multicast() {
            return Self::Multicast;
        }
        // RFC4193 fc00::/7.
        if segments[0] & 0xfe00 == 0xfc00 {
            return Self::UniqueLocal;
        }
        // RFC4291 fe80::/10.
        if segments[0] & 0xffc0 == 0xfe80 {
            return Self::LinkLocal;
        }
        // RFC3849 2001:db8::/32.
        if segments[0] == 0x2001 && segments[1] == 0x0db8 {
            return Self::Documentation;
        }
        Self::Public
    }
}

/// Whether one origin may reach restricted addresses.
///
/// The grant is per-origin and distinct from being allowlisted: declaring an
/// origin never implies permission to reach private space through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressPolicy {
    allow_private_network: bool,
}

impl AddressPolicy {
    /// Builds a policy for one origin's private-network grant.
    #[must_use]
    pub const fn new(allow_private_network: bool) -> Self {
        Self {
            allow_private_network,
        }
    }

    /// Authorizes one resolved address, or refuses it.
    ///
    /// The message names the class that refused, never the address itself:
    /// resolved addresses are among the values the source's leak-freedom
    /// contract keeps off every observable channel.
    pub fn authorize(self, address: IpAddr) -> Result<(), ResourceError> {
        let class = AddressClass::classify(address);
        if !class.is_restricted() || self.allow_private_network {
            return Ok(());
        }
        Err(ResourceError::new(
            ErrorCategory::PermissionDenied,
            format!(
                "resolved address is {class:?} space; this origin does not grant private-network access"
            ),
        ))
    }
}

/// One allowlisted HTTPS origin and its grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedOrigin {
    base_url: Url,
    allow_private_network: bool,
}

impl AllowedOrigin {
    /// Declares an origin from its configured `base_url`.
    pub fn new(base_url: &str, allow_private_network: bool) -> Result<Self, ResourceError> {
        let base_url = Url::parse(base_url).map_err(|_| {
            ResourceError::new(
                ErrorCategory::InvalidReference,
                "origin base_url must be a well-formed URL",
            )
        })?;
        if base_url.scheme() != "https" {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "origin base_url must use the https scheme",
            ));
        }
        if !base_url.has_host() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "origin base_url must name a host",
            ));
        }
        Ok(Self {
            base_url,
            allow_private_network,
        })
    }

    /// Returns the configured base URL.
    #[must_use]
    pub const fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Returns whether this origin may reach restricted addresses.
    #[must_use]
    pub const fn allow_private_network(&self) -> bool {
        self.allow_private_network
    }

    /// Returns the address policy this origin's grant implies.
    #[must_use]
    pub const fn address_policy(&self) -> AddressPolicy {
        AddressPolicy::new(self.allow_private_network)
    }

    /// Returns whether this origin's `base_url` prefixes `requested`.
    ///
    /// Public so the transport can ask the same question the allowlist asks —
    /// notably to keep an origin's credential from following a redirect out of
    /// the origin that owns it.
    #[must_use]
    pub fn authorizes(&self, requested: &Url) -> bool {
        self.base_url.scheme() == requested.scheme()
            && self.base_url.host() == requested.host()
            && self.base_url.port_or_known_default() == requested.port_or_known_default()
            && path_is_within(self.base_url.path(), requested.path())
    }
}

/// Returns whether `requested` lies at or beneath `base` on a segment boundary.
///
/// Segment awareness is the point: a naive string prefix would let a base of
/// `/docs` authorize `/docsecret`, silently widening the operator's grant to a
/// sibling path they never declared.
fn path_is_within(base: &str, requested: &str) -> bool {
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        // A base URL carrying no path scopes to the whole host.
        return true;
    }
    if requested == base {
        return true;
    }
    requested
        .strip_prefix(base)
        .is_some_and(|remainder| remainder.starts_with('/'))
}

/// The declared set of allowlisted origins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OriginAllowlist {
    origins: Vec<AllowedOrigin>,
}

impl OriginAllowlist {
    /// Builds an allowlist from declared origins.
    #[must_use]
    pub const fn new(origins: Vec<AllowedOrigin>) -> Self {
        Self { origins }
    }

    /// Returns the declared origins.
    #[must_use]
    pub fn origins(&self) -> &[AllowedOrigin] {
        &self.origins
    }

    /// Authorizes one requested URL against the declared origins.
    ///
    /// The first origin whose scheme, host, port, and path prefix all match
    /// wins. A URL matching none is refused; the message names the reference
    /// and the policy that refused it, never another origin's configuration.
    pub fn authorize(&self, requested: &Url) -> Result<&AllowedOrigin, ResourceError> {
        self.origins
            .iter()
            .find(|origin| origin.authorizes(requested))
            .ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::PermissionDenied,
                    format!(
                        "'{requested}' is not within any allowlisted HTTPS origin declared by the Server Profile"
                    ),
                )
            })
    }
}

/// Optional operator overrides for the HTTP transport ceilings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HttpCeilingsInput {
    pub fetch_bytes: Option<usize>,
    pub redirect_depth: Option<usize>,
    pub timeout_millis: Option<usize>,
}

/// Validated lower-only HTTP transport ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpCeilings {
    fetch_bytes: usize,
    redirect_depth: usize,
    timeout_millis: usize,
}

impl HttpCeilings {
    /// Validates operator overrides against the contract maxima.
    ///
    /// Every value may only be lowered, matching the `ServerLimits` idiom: an
    /// absent override takes the contract maximum, and zero or above-maximum
    /// is refused rather than clamped.
    pub fn new(input: HttpCeilingsInput) -> Result<Self, ResourceError> {
        Ok(Self {
            fetch_bytes: validate_limit(
                "http.fetchBytes",
                input.fetch_bytes,
                MAX_HTTP_FETCH_BYTES,
            )?,
            redirect_depth: validate_limit(
                "http.redirectDepth",
                input.redirect_depth,
                MAX_HTTP_REDIRECT_DEPTH,
            )?,
            timeout_millis: validate_limit(
                "http.timeoutMillis",
                input.timeout_millis,
                MAX_HTTP_TIMEOUT_MILLIS,
            )?,
        })
    }

    /// Returns the largest body admitted before extraction.
    #[must_use]
    pub const fn fetch_bytes(self) -> usize {
        self.fetch_bytes
    }

    /// Returns the largest redirect chain followed.
    #[must_use]
    pub const fn redirect_depth(self) -> usize {
        self.redirect_depth
    }

    /// Returns the total per-request timeout.
    #[must_use]
    pub const fn timeout(self) -> Duration {
        Duration::from_millis(self.timeout_millis as u64)
    }
}

impl Default for HttpCeilings {
    fn default() -> Self {
        Self {
            fetch_bytes: MAX_HTTP_FETCH_BYTES,
            redirect_depth: MAX_HTTP_REDIRECT_DEPTH,
            timeout_millis: MAX_HTTP_TIMEOUT_MILLIS,
        }
    }
}
