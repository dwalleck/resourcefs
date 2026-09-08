//! Opaque, session-scoped conversation-comment continuations.
//!
//! The handle carries no credential: it names the native next page, the
//! canonical Resource it belongs to, the configured API origin and a one-way
//! digest of the issuing Path Session. Ownership is a routing constraint, not
//! a signature or an authorization capability.
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use resourcefs_core::{ErrorCategory, PathReference, ResourceError, SourceCursor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use super::super::GithubSource;

/// Bounded size guard: an encoded handle must fit the Path Reference ceiling.
fn cursor_too_large() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        "GitHub continuation exceeds the reference ceiling; read fewer records",
    )
}

fn invalid_cursor() -> ResourceError {
    ResourceError::new(
        ErrorCategory::InvalidReference,
        "GitHub continuation is malformed or belongs to another resource, origin or session",
    )
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope<'a> {
    version: u8,
    #[serde(borrow)]
    resource: std::borrow::Cow<'a, str>,
    #[serde(borrow)]
    origin: std::borrow::Cow<'a, str>,
    #[serde(borrow)]
    session: std::borrow::Cow<'a, str>,
    #[serde(borrow)]
    next: std::borrow::Cow<'a, str>,
}

/// Ownership context for one collection continuation.
pub(super) struct CursorOwner {
    canonical: PathReference,
    origin: String,
    session: String,
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

impl CursorOwner {
    pub(super) fn new(canonical: PathReference, source: &GithubSource) -> Self {
        Self {
            canonical,
            origin: digest(source.api_base.origin().ascii_serialization().as_str()),
            session: digest(source.session.token().as_str()),
        }
    }

    /// Decodes a handle before any upstream request. The returned target is
    /// still confined by the caller to the same origin and endpoint family.
    pub(super) fn decode(&self, cursor: &SourceCursor) -> Result<Url, ResourceError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor.as_str())
            .map_err(|_| invalid_cursor())?;
        let envelope: Envelope<'_> =
            serde_json::from_slice(&bytes).map_err(|_| invalid_cursor())?;
        if envelope.version != 1
            || envelope.resource != self.canonical.requested()
            || envelope.origin != self.origin
            || envelope.session != self.session
        {
            return Err(invalid_cursor());
        }
        Url::parse(&envelope.next).map_err(|_| invalid_cursor())
    }

    /// Builds the handle for a native next page, or `None` when it cannot be
    /// represented inside the reference ceiling.
    pub(super) fn continuation(&self, next: &Url) -> Result<Option<PathReference>, ResourceError> {
        let envelope = Envelope {
            version: 1,
            resource: self.canonical.requested().into(),
            origin: self.origin.as_str().into(),
            session: self.session.as_str().into(),
            next: next.as_str().into(),
        };
        let bytes = serde_json::to_vec(&envelope).map_err(|_| cursor_too_large())?;
        let encoded = URL_SAFE_NO_PAD.encode(bytes);
        let Ok(cursor) = SourceCursor::new(encoded) else {
            return Ok(None);
        };
        let reference = PathReference::parse(format!(
            "{}:cursor:{}",
            self.canonical.requested(),
            cursor.as_str()
        ));
        match reference {
            Ok(reference) => Ok(Some(reference)),
            Err(error) if error.category() == ErrorCategory::LimitExceeded => Ok(None),
            Err(error) => Err(error),
        }
    }
}
