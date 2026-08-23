//! Source-neutrality contract for the bounded HTTP substrate (rfs-g2z9 C18).
//!
//! The substrate exists so that the HTTPS, GitHub, and downstream-MCP sources
//! share one audited egress path instead of each building a client. That only
//! holds if its interface names no source-specific type — so this file is
//! written as a consumer that knows nothing about HTTPS, and its compilation
//! is the assertion.

use std::{
    io,
    net::{IpAddr, Ipv4Addr},
};

use resourcefs_core::{
    AllowedOrigin, ErrorCategory, HttpCeilings, OperationGuard, OriginAllowlist,
};
use resourcefs_sources::{BoundedHttpResponse, HttpRequest, HttpSubstrate};
use url::Url;

/// Drives the substrate using only neutral types.
///
/// The signature is the point: a caller reaches egress with a URL, a guard,
/// and nothing else. If `fetch` ever required a source-specific handle, this
/// function would stop compiling — which is exactly the regression the claim
/// forbids.
async fn neutral_consumer(
    substrate: &HttpSubstrate,
    url: Url,
) -> Result<BoundedHttpResponse, resourcefs_core::ResourceError> {
    substrate
        .fetch(HttpRequest::get(url), &OperationGuard::new())
        .await
}

#[tokio::test]
async fn substrate_is_source_neutral() {
    let allowlist = OriginAllowlist::new(vec![
        AllowedOrigin::new("https://neutral.invalid/", false).expect("origin is well formed"),
    ]);
    let substrate =
        HttpSubstrate::with_host_lookup(allowlist, HttpCeilings::default(), |_host| async {
            Ok::<_, io::Error>(vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))])
        })
        .expect("substrate builds");

    // A URL outside every declared origin is refused before egress, and the
    // neutral consumer observes that through the shared error vocabulary
    // rather than any HTTPS-specific type.
    let outside = Url::parse("https://elsewhere.invalid/doc").expect("url parses");
    let failure = neutral_consumer(&substrate, outside)
        .await
        .expect_err("an unallowlisted origin is refused");
    assert_eq!(failure.category(), ErrorCategory::PermissionDenied);

    // The ceilings the substrate enforces are core's neutral type.
    assert_eq!(
        substrate.ceilings().fetch_bytes(),
        HttpCeilings::default().fetch_bytes()
    );
}
