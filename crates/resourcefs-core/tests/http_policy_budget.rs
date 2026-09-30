//! Production-scale budget fence for the HTTP policy decision path (rfs-g2z9).
//!
//! This work runs on every request — once per resolved address and once per
//! redirect hop — so it must not become a measurable fraction of a network
//! round trip. The budget is one classification plus one full allowlist pass at
//! the profile's maximum of 256 declared origins, under 1 ms.

#[path = "support/wall_budget.rs"]
mod wall_budget;

use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

use resourcefs_core::{AddressClass, AddressPolicy, AllowedOrigin, OriginAllowlist};
use url::Url;

/// The profile cardinality limit the plan cites for declared origins.
const PRODUCTION_ORIGINS: usize = 256;

#[test]
fn classification_and_allowlist_pass_fit_the_request_budget() {
    let origins = (0..PRODUCTION_ORIGINS)
        .map(|index| {
            AllowedOrigin::new(&format!("https://host{index}.example/base{index}"), false)
                .expect("generated base url is well formed")
        })
        .collect::<Vec<_>>();
    let allowlist = OriginAllowlist::new(origins);

    // Worst case: the matching origin is last, so the scan walks all 256.
    let requested = Url::parse(&format!(
        "https://host{}.example/base{}/page",
        PRODUCTION_ORIGINS - 1,
        PRODUCTION_ORIGINS - 1
    ))
    .expect("generated url is well formed");
    let address: IpAddr = "203.0.113.7".parse().expect("static address");
    let policy = AddressPolicy::new(true);

    // One iteration is the per-request cost the budget governs; repeat to get a
    // stable measurement, then report the per-iteration figure.
    const ITERATIONS: u32 = 1_000;
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = AddressClass::classify(address);
        policy.authorize(address).expect("granted policy admits");
        allowlist
            .authorize(&requested)
            .expect("last origin authorizes its own subtree");
    }
    let per_request = started.elapsed() / ITERATIONS;

    // Strict, as the `as_micros() < 1_000` it replaced was: whole
    // microseconds below 1,000 is exactly `per_request < 1 ms`.
    wall_budget::check_wall_budget_below(
        "256-origin allowlist pass per request",
        per_request,
        Duration::from_millis(1),
    );
}
