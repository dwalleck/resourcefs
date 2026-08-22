use std::fmt::{self, Write as _};

use sha2::{Digest, Sha256};

/// Content-derived identity for an authoritative Resource state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionTag(String);

impl VersionTag {
    pub fn parse(value: impl Into<String>) -> Result<Self, crate::ResourceError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(crate::ResourceError::new(
                crate::ErrorCategory::InvalidReference,
                "Version Tag must begin with sha256:",
            ));
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(crate::ResourceError::new(
                crate::ErrorCategory::InvalidReference,
                "Version Tag must contain exactly 64 lowercase hexadecimal digest characters",
            ));
        }
        Ok(Self(value))
    }
    pub fn from_content(content: &[u8]) -> Self {
        let digest = Sha256::digest(content);
        Self(format!("sha256:{digest:x}"))
    }
    pub(crate) fn from_sha256_digest(digest: [u8; 32]) -> Self {
        let mut tag = String::with_capacity("sha256:".len() + digest.len() * 2);
        tag.push_str("sha256:");
        for byte in digest {
            write!(tag, "{byte:02x}").expect("writing to a String cannot fail");
        }
        Self(tag)
    }
    pub(crate) fn from_sha256_hex(hex: &str) -> Self {
        debug_assert_eq!(hex.len(), 64);
        debug_assert!(hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
        Self(format!("sha256:{hex}"))
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
