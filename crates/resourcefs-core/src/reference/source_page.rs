use std::num::NonZeroU64;

use crate::ResourceError;

use super::invalid_reference;

/// Positive native source offset; the first page is the unselected collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceOffset(NonZeroU64);

impl SourceOffset {
    pub fn new(value: u64) -> Result<Self, ResourceError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or_else(|| invalid_reference("source offset must be a positive canonical u64"))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub(super) fn parse(value: &str) -> Result<Self, ResourceError> {
        if value.is_empty()
            || value.starts_with('0')
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(invalid_reference(
                "source offset must be a positive canonical u64",
            ));
        }
        let value = value
            .parse::<u64>()
            .map_err(|_| invalid_reference("source offset must be a positive canonical u64"))?;
        Self::new(value)
    }
}

/// Canonical unpadded base64url source continuation, without interpreting its payload.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceCursor(String);

impl SourceCursor {
    pub fn new(value: String) -> Result<Self, ResourceError> {
        Self::validate(&value)?;
        Ok(Self(format!("cursor:{value}")))
    }

    pub fn as_str(&self) -> &str {
        &self.0["cursor:".len()..]
    }

    pub(super) fn from_spelling(spelling: String) -> Result<Self, ResourceError> {
        let value = spelling
            .strip_prefix("cursor:")
            .ok_or_else(|| invalid_reference("source cursor requires its selector prefix"))?;
        Self::validate(value)?;
        Ok(Self(spelling))
    }

    pub(super) fn spelling(&self) -> &str {
        &self.0
    }

    fn validate(value: &str) -> Result<(), ResourceError> {
        if value.len() > super::MAX_PATH_REFERENCE_BYTES - "cursor:".len() {
            return Err(ResourceError::new(
                crate::ErrorCategory::LimitExceeded,
                "source cursor exceeds the reference byte ceiling",
            ));
        }
        let mut last = 0;
        for byte in value.bytes() {
            last = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'-' => 62,
                b'_' => 63,
                _ => {
                    return Err(invalid_reference(
                        "source cursor must be canonical base64url",
                    ));
                }
            };
        }
        if value.is_empty()
            || value.len() % 4 == 1
            || (value.len() % 4 == 2 && last & 15 != 0)
            || (value.len() % 4 == 3 && last & 3 != 0)
        {
            return Err(invalid_reference(
                "source cursor must be canonical base64url",
            ));
        }
        Ok(())
    }
}
