//! Authenticated, bounded conversation-comment continuations.
//!
//! A handle contains the native next-page target and its routing context, but
//! carries no credential.  The envelope is authenticated with a private key
//! generated when one `GithubSource` is bound to a `PathSession`; clones share
//! that key while independently bound sources do not.
use std::{
    borrow::Cow,
    io::{self, Write},
    sync::Arc,
};

use aws_lc_rs::hmac;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use resourcefs_core::{
    ErrorCategory, MAX_PATH_REFERENCE_BYTES, PathReference, ProjectionSelector, ResourceAddress,
    ResourceError, SourceCursor,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::super::GithubSource;
use crate::http::reject_url_userinfo;

const CURSOR_VERSION: u8 = 2;
const CURSOR_KEY_BYTES: usize = 32;
const CURSOR_TAG_BYTES: usize = 32;
const CURSOR_SELECTOR_PREFIX: &str = ":cursor:";

fn invalid_cursor() -> ResourceError {
    ResourceError::new(
        ErrorCategory::InvalidReference,
        "GitHub continuation is malformed or belongs to another resource, source, API base or session",
    )
}

fn cursor_serialization_error() -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        "GitHub continuation could not be serialized",
    )
}

/// A bounded writer used for the unsigned portion of the cursor envelope.
///
/// `serde_json::to_writer` is deliberately fed a writer whose allocation and
/// accepted bytes are capped before any provider-controlled target can cause a
/// larger buffer to be reserved.
struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl BoundedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(required) = self.bytes.len().checked_add(bytes.len()) else {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "cursor envelope exceeds its bounded encoding budget",
            ));
        };
        if required > self.limit {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "cursor envelope exceeds its bounded encoding budget",
            ));
        }
        if required > self.bytes.capacity() {
            self.bytes.reserve_exact(required - self.bytes.len());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn base64_len(raw: usize) -> Option<usize> {
    raw.checked_div(3)?
        .checked_mul(4)?
        .checked_add(match raw % 3 {
            0 => 0,
            1 => 2,
            _ => 3,
        })
}
fn raw_limit(encoded_limit: usize) -> Option<usize> {
    let quotient = encoded_limit / 4;
    let remainder = encoded_limit % 4;
    quotient.checked_mul(3)?.checked_add(match remainder {
        2 => 1,
        3 => 2,
        _ => 0,
    })
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope<'a> {
    version: u8,
    #[serde(borrow)]
    source: Cow<'a, str>,
    #[serde(borrow)]
    api: Cow<'a, str>,
    #[serde(borrow)]
    resource: Cow<'a, str>,
    #[serde(borrow)]
    target: Cow<'a, str>,
}

/// Generates the independent key for one source/session binding.
pub(in crate::github) fn new_session_key() -> Result<Arc<[u8; CURSOR_KEY_BYTES]>, ResourceError> {
    let mut key = [0_u8; CURSOR_KEY_BYTES];
    getrandom::fill(&mut key).map_err(|error| {
        ResourceError::new(
            ErrorCategory::SourceUnavailable,
            format!("GitHub continuation key could not be generated: {error}"),
        )
    })?;
    Ok(Arc::new(key))
}

/// Ownership context for one collection continuation.
pub(super) struct CursorOwner<'a> {
    canonical: PathReference,
    source: &'a str,
    api: &'a Url,
    key: &'a [u8; CURSOR_KEY_BYTES],
}

impl<'a> CursorOwner<'a> {
    pub(super) fn new(canonical: PathReference, source: &'a GithubSource) -> Self {
        Self {
            canonical,
            source: source.config.id(),
            api: &source.api_base,
            key: source.cursor_key.as_ref(),
        }
    }

    fn encoded_limit(&self) -> Option<usize> {
        let prefix = self
            .canonical
            .requested()
            .len()
            .checked_add(CURSOR_SELECTOR_PREFIX.len())?;
        MAX_PATH_REFERENCE_BYTES.checked_sub(prefix)
    }

    /// Decodes and authenticates a handle before any upstream request.  The
    /// returned target is still confined by the caller to its endpoint family.
    pub(super) fn decode(&self, cursor: &SourceCursor) -> Result<Url, ResourceError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor.as_str())
            .map_err(|_| invalid_cursor())?;
        if bytes.len() <= CURSOR_TAG_BYTES {
            return Err(invalid_cursor());
        }
        let payload_len = bytes.len() - CURSOR_TAG_BYTES;
        let (payload, tag) = bytes.split_at(payload_len);
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.key.as_ref());
        hmac::verify(&key, payload, tag).map_err(|_| invalid_cursor())?;
        let envelope: Envelope<'_> =
            serde_json::from_slice(payload).map_err(|_| invalid_cursor())?;
        if envelope.version != CURSOR_VERSION
            || envelope.source.as_ref() != self.source
            || envelope.api.as_ref() != self.api.as_str()
            || envelope.resource.as_ref() != self.canonical.requested()
        {
            return Err(invalid_cursor());
        }
        let target = Url::parse(&envelope.target).map_err(|_| invalid_cursor())?;
        let same_api = target.scheme() == self.api.scheme()
            && target.host_str() == self.api.host_str()
            && target.port_or_known_default() == self.api.port_or_known_default()
            && target.path().starts_with(self.api.path());
        if !same_api {
            return Err(invalid_cursor());
        }
        reject_url_userinfo(&target).map_err(|_| invalid_cursor())?;
        Ok(target)
    }

    /// Builds an authenticated handle for a native next page, or `None` when
    /// the final typed reference cannot fit its intentional size ceiling.
    pub(super) fn continuation(&self, next: &Url) -> Result<Option<PathReference>, ResourceError> {
        let Some(encoded_limit) = self.encoded_limit() else {
            return Ok(None);
        };
        let Some(max_raw) = raw_limit(encoded_limit) else {
            return Ok(None);
        };
        if max_raw <= CURSOR_TAG_BYTES {
            return Ok(None);
        }
        let payload_limit = max_raw - CURSOR_TAG_BYTES;
        let envelope = Envelope {
            version: CURSOR_VERSION,
            source: self.source.into(),
            api: self.api.as_str().into(),
            resource: self.canonical.requested().into(),
            target: next.as_str().into(),
        };
        let mut writer = BoundedWriter::new(payload_limit);
        let serialized = serde_json::to_writer(&mut writer, &envelope);
        if writer.exceeded {
            return Ok(None);
        }
        serialized.map_err(|_| cursor_serialization_error())?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.key.as_ref());
        let tag = hmac::sign(&key, &writer.bytes);
        writer.bytes.reserve_exact(CURSOR_TAG_BYTES);
        let mut raw = writer.bytes;
        raw.extend_from_slice(tag.as_ref());
        let Some(encoded_len) = base64_len(raw.len()) else {
            return Ok(None);
        };
        if encoded_len > encoded_limit {
            return Ok(None);
        }
        let mut encoded = String::with_capacity(encoded_len);
        URL_SAFE_NO_PAD.encode_string(&raw, &mut encoded);
        let cursor = match SourceCursor::new(encoded) {
            Ok(cursor) => cursor,
            Err(error) if error.category() == ErrorCategory::LimitExceeded => return Ok(None),
            Err(error) => return Err(error),
        };
        let ResourceAddress::PullRequest(address) = self.canonical.address() else {
            return Err(invalid_cursor());
        };
        let selector = ProjectionSelector::from_source_cursor(cursor);
        match PathReference::pull_request(address.clone(), Some(selector)) {
            Ok(reference) => Ok(Some(reference)),
            Err(error) if error.category() == ErrorCategory::LimitExceeded => Ok(None),
            Err(error) => Err(error),
        }
    }
}
