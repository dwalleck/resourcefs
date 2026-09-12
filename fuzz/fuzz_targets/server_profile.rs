#![no_main]

use libfuzzer_sys::fuzz_target;
use resourcefs_mcp::{MAX_PROFILE_BYTES, ProfileDocument, ProfileErrorKind};

fuzz_target!(|data: &[u8]| {
    let first = ProfileDocument::from_slice(data)
        .map(|_| ())
        .map_err(|error| error.kind());
    let second = ProfileDocument::from_slice(data)
        .map(|_| ())
        .map_err(|error| error.kind());
    assert_eq!(first, second, "profile parsing must be deterministic");

    if data.len() > MAX_PROFILE_BYTES {
        assert_eq!(first, Err(ProfileErrorKind::LimitExceeded));
    }
});
