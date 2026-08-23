//! Contract fences for the source-neutral HTTP policy types (rfs-g2z9 C10, C11).
//!
//! Both tables below are hand-authored from `spec.md` and the governing RFCs.
//! Neither is computed by calling the classifier or the allowlist: the expected
//! column is the independent oracle, so an implementation that agrees with
//! itself but disagrees with the RFCs fails here.

use std::{net::IpAddr, time::Duration};

use resourcefs_core::{
    AddressClass, AddressPolicy, AllowedOrigin, ErrorCategory, HttpCeilings, HttpCeilingsInput,
    MAX_HTTP_FETCH_BYTES, MAX_HTTP_REDIRECT_DEPTH, MAX_HTTP_TIMEOUT_MILLIS, OriginAllowlist,
};
use url::Url;

/// Hand-authored address table: literal → expected class → restricted?
///
/// Sources: RFC1122 (127/8), RFC1918 (private), RFC3927 (169.254/16),
/// RFC6598 (100.64/10), RFC5771 (224/4), RFC5737 (documentation v4),
/// RFC2544 (198.18/15), RFC1112 (240/4), RFC4291 (::1, fe80::/10, ff00::/8,
/// IPv4-mapped ::ffff:0:0/96), RFC4193 (fc00::/7), RFC3849 (2001:db8::/32).
const ADDRESS_TABLE: &[(&str, AddressClass, bool)] = &[
    // Loopback.
    ("127.0.0.1", AddressClass::Loopback, true),
    ("127.255.255.254", AddressClass::Loopback, true),
    ("::1", AddressClass::Loopback, true),
    // RFC1918 private.
    ("10.0.0.1", AddressClass::Private, true),
    ("172.16.0.1", AddressClass::Private, true),
    ("172.31.255.254", AddressClass::Private, true),
    ("192.168.1.1", AddressClass::Private, true),
    // Link-local.
    ("169.254.1.1", AddressClass::LinkLocal, true),
    ("fe80::1", AddressClass::LinkLocal, true),
    // Carrier-grade NAT (RFC6598).
    ("100.64.0.1", AddressClass::SharedCgnat, true),
    ("100.127.255.254", AddressClass::SharedCgnat, true),
    // Multicast.
    ("224.0.0.1", AddressClass::Multicast, true),
    ("ff02::1", AddressClass::Multicast, true),
    // Unspecified.
    ("0.0.0.0", AddressClass::Unspecified, true),
    ("::", AddressClass::Unspecified, true),
    // Unique-local v6.
    ("fc00::1", AddressClass::UniqueLocal, true),
    ("fd12:3456::1", AddressClass::UniqueLocal, true),
    // Documentation ranges.
    ("192.0.2.1", AddressClass::Documentation, true),
    ("198.51.100.1", AddressClass::Documentation, true),
    ("203.0.113.1", AddressClass::Documentation, true),
    ("2001:db8::1", AddressClass::Documentation, true),
    // Benchmarking and reserved.
    ("198.18.0.1", AddressClass::Benchmarking, true),
    ("240.0.0.1", AddressClass::Reserved, true),
    ("255.255.255.255", AddressClass::Reserved, true),
    // IPv4-mapped IPv6 embedding a restricted v4 address. Classifying these as
    // ordinary public v6 addresses is the classic SSRF bypass, so each must
    // classify by its embedded IPv4 rules.
    ("::ffff:127.0.0.1", AddressClass::Loopback, true),
    ("::ffff:10.0.0.1", AddressClass::Private, true),
    ("::ffff:192.168.1.1", AddressClass::Private, true),
    ("::ffff:169.254.1.1", AddressClass::LinkLocal, true),
    // Ordinary public addresses.
    ("8.8.8.8", AddressClass::Public, false),
    ("1.1.1.1", AddressClass::Public, false),
    ("93.184.216.34", AddressClass::Public, false),
    ("2001:4860:4860::8888", AddressClass::Public, false),
    ("::ffff:8.8.8.8", AddressClass::Public, false),
];

/// Hand-authored scoping table against base `https://ex.com/docs`.
const SCOPING_TABLE: &[(&str, bool)] = &[
    // Exactly the base, and everything beneath it at a segment boundary.
    ("https://ex.com/docs", true),
    ("https://ex.com/docs/", true),
    ("https://ex.com/docs/guide", true),
    ("https://ex.com/docs/guide/deep/page.html", true),
    ("https://ex.com/docs/a%2Fb", true),
    ("https://ex.com/docs?q=1", true),
    // Prefix collision: a naive string prefix would wrongly admit these.
    ("https://ex.com/docsecret", false),
    ("https://ex.com/docs-internal", false),
    ("https://ex.com/docsx/y", false),
    // Wrong path, host, or port.
    ("https://ex.com/other", false),
    ("https://ex.com/", false),
    ("https://other.com/docs", false),
    ("https://sub.ex.com/docs", false),
    ("https://ex.com:8443/docs", false),
];

fn address(literal: &str) -> IpAddr {
    literal
        .parse()
        .unwrap_or_else(|_| panic!("test table address {literal} must parse"))
}

fn url(literal: &str) -> Url {
    Url::parse(literal).unwrap_or_else(|_| panic!("test table url {literal} must parse"))
}

fn docs_origin(allow_private_network: bool) -> AllowedOrigin {
    AllowedOrigin::new("https://ex.com/docs", allow_private_network)
        .expect("static base url is well formed")
}

#[test]
fn private_network_grant() {
    // Classification agrees with the RFC-derived table.
    for (literal, expected_class, expected_restricted) in ADDRESS_TABLE {
        let observed = AddressClass::classify(address(literal));
        assert_eq!(
            observed, *expected_class,
            "classification of {literal} disagrees with the RFC table"
        );
        assert_eq!(
            observed.is_restricted(),
            *expected_restricted,
            "restriction verdict for {literal} disagrees with the RFC table"
        );
    }

    // Without the grant, every restricted address is denied and every public
    // address is admitted.
    let denying = AddressPolicy::new(false);
    for (literal, _, expected_restricted) in ADDRESS_TABLE {
        let outcome = denying.authorize(address(literal));
        if *expected_restricted {
            let error = outcome.expect_err("restricted address must be denied without the grant");
            assert_eq!(
                error.category(),
                ErrorCategory::PermissionDenied,
                "denial category for {literal}"
            );
        } else {
            outcome.unwrap_or_else(|error| {
                panic!("public address {literal} must be admitted, got {error}")
            });
        }
    }

    // The grant is a distinct per-origin permission: with it, restricted
    // addresses become reachable while classification is unchanged.
    let granting = AddressPolicy::new(true);
    for (literal, expected_class, _) in ADDRESS_TABLE {
        granting
            .authorize(address(literal))
            .unwrap_or_else(|error| {
                panic!("granted policy must admit {literal} ({expected_class:?}), got {error}")
            });
        assert_eq!(
            AddressClass::classify(address(literal)),
            *expected_class,
            "the grant must not change how {literal} classifies"
        );
    }
}

#[test]
fn base_url_scoping() {
    let allowlist = OriginAllowlist::new(vec![docs_origin(false)]);

    for (literal, expected_allowed) in SCOPING_TABLE {
        let outcome = allowlist.authorize(&url(literal));
        if *expected_allowed {
            let origin =
                outcome.unwrap_or_else(|error| panic!("{literal} must be authorized, got {error}"));
            assert_eq!(
                origin.base_url().as_str(),
                "https://ex.com/docs",
                "{literal} resolved to the wrong origin"
            );
        } else {
            let error = outcome.err().unwrap_or_else(|| {
                panic!("{literal} must be denied by base_url scoping, but was authorized")
            });
            assert_eq!(
                error.category(),
                ErrorCategory::PermissionDenied,
                "denial category for {literal}"
            );
        }
    }
}

#[test]
fn allowlist_selects_the_matching_origin() {
    let allowlist = OriginAllowlist::new(vec![
        AllowedOrigin::new("https://a.example/one", false).expect("well formed"),
        AllowedOrigin::new("https://b.example/two", true).expect("well formed"),
    ]);

    let first = allowlist
        .authorize(&url("https://a.example/one/x"))
        .expect("first origin authorizes its own subtree");
    assert!(!first.allow_private_network());

    let second = allowlist
        .authorize(&url("https://b.example/two/y"))
        .expect("second origin authorizes its own subtree");
    assert!(second.allow_private_network());

    let empty = OriginAllowlist::new(Vec::new());
    assert_eq!(
        empty
            .authorize(&url("https://a.example/one"))
            .expect_err("an empty allowlist authorizes nothing")
            .category(),
        ErrorCategory::PermissionDenied
    );
}

#[test]
fn whole_host_base_url_scopes_to_that_host() {
    let allowlist = OriginAllowlist::new(vec![
        AllowedOrigin::new("https://ex.com", false).expect("valid"),
    ]);

    for literal in ["https://ex.com/", "https://ex.com/anything/deep"] {
        allowlist
            .authorize(&url(literal))
            .unwrap_or_else(|error| panic!("{literal} must be authorized, got {error}"));
    }
    assert!(allowlist.authorize(&url("https://other.com/")).is_err());
}

#[test]
fn origin_rejects_a_base_url_that_is_not_https() {
    for literal in ["http://ex.com/docs", "ftp://ex.com/docs", "not a url"] {
        let error = AllowedOrigin::new(literal, false)
            .expect_err("only https base urls are configurable origins");
        assert_eq!(
            error.category(),
            ErrorCategory::InvalidReference,
            "{literal}"
        );
    }
}

#[test]
fn ceilings_default_to_the_contract_values_and_lower_only() {
    let defaults = HttpCeilings::default();
    assert_eq!(defaults.fetch_bytes(), MAX_HTTP_FETCH_BYTES);
    assert_eq!(defaults.redirect_depth(), MAX_HTTP_REDIRECT_DEPTH);
    assert_eq!(defaults.timeout(), Duration::from_millis(30_000));
    assert_eq!(MAX_HTTP_FETCH_BYTES, 8 * 1024 * 1024);
    assert_eq!(MAX_HTTP_REDIRECT_DEPTH, 5);
    assert_eq!(MAX_HTTP_TIMEOUT_MILLIS, 30_000);

    // Lowering is accepted.
    let lowered = HttpCeilings::new(HttpCeilingsInput {
        fetch_bytes: Some(1024),
        redirect_depth: Some(2),
        timeout_millis: Some(5_000),
    })
    .expect("lowering every ceiling is accepted");
    assert_eq!(lowered.fetch_bytes(), 1024);
    assert_eq!(lowered.redirect_depth(), 2);
    assert_eq!(lowered.timeout(), Duration::from_millis(5_000));

    // Raising above the hard ceiling, or to zero, is refused.
    for input in [
        HttpCeilingsInput {
            fetch_bytes: Some(MAX_HTTP_FETCH_BYTES + 1),
            ..HttpCeilingsInput::default()
        },
        HttpCeilingsInput {
            redirect_depth: Some(MAX_HTTP_REDIRECT_DEPTH + 1),
            ..HttpCeilingsInput::default()
        },
        HttpCeilingsInput {
            timeout_millis: Some(MAX_HTTP_TIMEOUT_MILLIS + 1),
            ..HttpCeilingsInput::default()
        },
        HttpCeilingsInput {
            fetch_bytes: Some(0),
            ..HttpCeilingsInput::default()
        },
    ] {
        assert_eq!(
            HttpCeilings::new(input)
                .expect_err("a ceiling above the contract maximum, or zero, is refused")
                .category(),
            ErrorCategory::LimitExceeded
        );
    }
}
