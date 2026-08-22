#![no_main]

use libfuzzer_sys::fuzz_target;
use resourcefs_core::{ErrorCategory, HashlinePatch, MAX_HASHLINE_PATCH_BYTES};

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let first = HashlinePatch::parse(input)
        .map(|patch| format!("{patch:?}"))
        .map_err(|error| error.category());
    let second = HashlinePatch::parse(input)
        .map(|patch| format!("{patch:?}"))
        .map_err(|error| error.category());
    assert_eq!(first, second, "hashline parsing must be deterministic");
    if input.len() > MAX_HASHLINE_PATCH_BYTES {
        assert_eq!(first, Err(ErrorCategory::LimitExceeded));
    }
});
