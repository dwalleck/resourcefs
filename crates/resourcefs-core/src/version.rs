use std::fmt;

use sha2::{Digest, Sha256};

/// Content-derived identity for an authoritative Resource state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionTag(String);

impl VersionTag {
    pub fn from_content(content: &[u8]) -> Self {
        let digest = Sha256::digest(content);
        Self(format!("sha256:{digest:x}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VersionTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
