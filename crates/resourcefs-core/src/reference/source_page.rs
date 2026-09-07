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
