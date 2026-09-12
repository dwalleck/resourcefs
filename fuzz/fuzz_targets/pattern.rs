#![no_main]
#![allow(
    dead_code,
    reason = "this target includes the pattern module standalone, without the Source Adapters that use the rest of it"
)]

//! Fuzzes the workspace's only `unsafe` FFI against caller-supplied input.
//!
//! `sources/src/pattern.rs` holds ten `unsafe` blocks over PCRE2 -- code
//! compilation, match-context and match-data allocation, the match call itself,
//! and three `Drop` impls that free them. Every one of those is reached from the
//! `rfs_search` `pattern` argument, which a caller controls up to
//! `MAX_DISCOVERY_PATTERN_BYTES`, matched against subjects a caller also
//! influences. That made it the highest-leverage coverage gap in the tree: the
//! two *safe* parse boundaries AGENTS.md names both had fuzz targets and this
//! did not.
//!
//! The module is included by `#[path]` rather than reached through the library,
//! matching `tests/pattern_contract.rs`, because `SearchMatcher` and
//! `GlobMatcher` are `pub(crate)` and fuzzing them should not widen the
//! library's public surface.
//!
//! What this proves beyond "does not crash": the sanitizer sees every PCRE2
//! allocation and free, so a leaked `pcre2_code` or a double free shows up
//! across iterations rather than only under a real workload.

#[path = "../../crates/resourcefs-sources/src/pattern.rs"]
mod pattern;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use pattern::{GlobMatcher, PatternErrorKind, SearchMatcher};
use resourcefs_core::{MAX_DISCOVERY_PATTERN_BYTES, MAX_PATH_REFERENCE_BYTES};

#[derive(Arbitrary, Debug)]
struct Input<'a> {
    pattern: &'a str,
    subject: &'a str,
    case_sensitive: bool,
    /// Exercise `GlobMatcher` as well; it shares the file and the ceiling
    /// vocabulary but not the FFI.
    glob: bool,
}

/// The finite vocabulary `pattern.rs` refuses with. A variant outside this set
/// would mean an error path grew without its contract being updated.
const KNOWN: [PatternErrorKind; 5] = [
    PatternErrorKind::InvalidPattern,
    PatternErrorKind::InputLimitExceeded,
    PatternErrorKind::MatchLimitExceeded,
    PatternErrorKind::DepthLimitExceeded,
    PatternErrorKind::EngineFailure,
];

fuzz_target!(|input: Input<'_>| {
    let Input {
        pattern,
        subject,
        case_sensitive,
        glob,
    } = input;

    if glob {
        let compiled = GlobMatcher::compile(pattern, case_sensitive);
        let repeat = GlobMatcher::compile(pattern, case_sensitive);
        assert_eq!(
            compiled.is_ok(),
            repeat.is_ok(),
            "glob compilation must be deterministic"
        );
        if let Err(error) = &compiled {
            assert!(KNOWN.contains(&error.kind()), "unknown glob error kind");
        }
        if pattern.is_empty() || pattern.len() > MAX_PATH_REFERENCE_BYTES {
            assert!(
                compiled.is_err(),
                "an empty or oversized glob must be refused"
            );
        }
        if let Ok(matcher) = compiled {
            // `GlobMatcher::is_match` is infallible by signature, so the claim
            // is only that it terminates and does not panic on any subject.
            let first = matcher.is_match(subject);
            assert_eq!(
                first,
                matcher.is_match(subject),
                "glob match must be stable"
            );
        }
        return;
    }

    let compiled = SearchMatcher::compile(pattern, case_sensitive);

    // Compiling the same pattern twice must reach the same verdict. This is the
    // invariant that would break if engine selection ever depended on
    // process state rather than the pattern: `compile` tries Rust regex first
    // and falls back to PCRE2 only on a syntax error.
    let repeat = SearchMatcher::compile(pattern, case_sensitive);
    match (&compiled, &repeat) {
        (Ok(left), Ok(right)) => assert_eq!(
            left.engine(),
            right.engine(),
            "engine selection must be deterministic"
        ),
        (Err(left), Err(right)) => assert_eq!(
            left.kind(),
            right.kind(),
            "refusal category must be deterministic"
        ),
        _ => panic!("compilation must be deterministic: {pattern:?}"),
    }

    if let Err(error) = &compiled {
        assert!(
            KNOWN.contains(&error.kind()),
            "unknown search error kind: {:?}",
            error.kind()
        );
    }

    // The two ceilings `validate_pattern` enforces before either engine sees
    // the pattern.
    if pattern.is_empty() {
        assert_eq!(
            compiled.as_ref().err().map(|error| error.kind()),
            Some(PatternErrorKind::InvalidPattern),
            "an empty pattern must be refused as invalid"
        );
    }
    if pattern.len() > MAX_DISCOVERY_PATTERN_BYTES {
        assert_eq!(
            compiled.as_ref().err().map(|error| error.kind()),
            Some(PatternErrorKind::InputLimitExceeded),
            "a pattern past the {MAX_DISCOVERY_PATTERN_BYTES}-byte ceiling must be refused"
        );
    }

    let Ok(mut matcher) = compiled else {
        return;
    };

    // `is_match` takes `&mut self` because the PCRE2 path reuses its match
    // data. Reusing it must not change the answer, and a refusal must stay a
    // refusal -- a match-limit failure that succeeded on the second call would
    // mean the work bound depends on accumulated state.
    let first = matcher.is_match(subject);
    let second = matcher.is_match(subject);
    match (&first, &second) {
        (Ok(left), Ok(right)) => assert_eq!(left, right, "match result must be stable"),
        (Err(left), Err(right)) => {
            assert_eq!(left.kind(), right.kind(), "refusal must be stable");
            // The bounded-work refusals are the whole reason PCRE2_MATCH_LIMIT
            // and PCRE2_DEPTH_LIMIT are set; a catastrophic pattern must come
            // back as one of these rather than run away.
            assert!(KNOWN.contains(&left.kind()), "unknown match error kind");
        }
        _ => panic!("match determinism violated for {pattern:?} on {subject:?}"),
    }
});
