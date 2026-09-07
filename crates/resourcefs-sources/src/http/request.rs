//! Request construction, source-header validation, and explicit replay intent.

use bytes::Bytes;
use resourcefs_core::{ErrorCategory, MAX_ARTIFACT_BYTES, ResourceError};
use url::Url;

use super::header;

const MAX_SOURCE_REQUEST_HEADERS: usize = 16;
pub(super) const MAX_SOURCE_REQUEST_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_MUTATION_REQUEST_BYTES: usize = MAX_ARTIFACT_BYTES * 6 + 64 * 1024;
const MAX_HTTP_READ_ONLY_REQUEST_BYTES: usize = 384 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestIntent {
    ReadOnly,
    Mutation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SourceRequestHeader {
    pub(super) name: header::HeaderName,
    pub(super) value: header::HeaderValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HttpMethod {
    Get,
    Post,
    Patch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RedirectBehavior {
    FollowReads,
    Refuse,
}

/// One source-neutral request for the substrate to perform.
#[derive(PartialEq, Eq)]
pub struct HttpRequest {
    pub(super) url: Url,
    pub(super) method: HttpMethod,
    pub(super) redirect: RedirectBehavior,
    pub(super) body: Option<Bytes>,
    pub(super) headers: Vec<SourceRequestHeader>,
    header_bytes: usize,
    intent: RequestIntent,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("redirect", &self.redirect)
            .field("intent", &self.intent)
            .field("body_bytes", &self.body.as_ref().map(Bytes::len))
            .finish_non_exhaustive()
    }
}

impl HttpRequest {
    /// Builds a GET request for one absolute URL.
    #[must_use]
    pub const fn get(url: Url) -> Self {
        Self {
            url,
            method: HttpMethod::Get,
            redirect: RedirectBehavior::FollowReads,
            body: None,
            headers: Vec::new(),
            header_bytes: 0,
            intent: RequestIntent::ReadOnly,
        }
    }

    /// Builds one bounded non-redirecting JSON POST request.
    pub fn post_json(url: Url, body: Vec<u8>) -> Result<Self, ResourceError> {
        Self::json(url, HttpMethod::Post, body)
    }

    /// Builds a nonredirecting POST whose caller guarantees read-only semantics.
    ///
    /// Callers preflight input lengths before JSON serialization. The encoded
    /// body has its own 384 KiB ceiling and transfers into shared immutable bytes
    /// once, so replay never copies or reserializes the payload.
    pub(crate) fn post_json_read_only(url: Url, body: Vec<u8>) -> Result<Self, ResourceError> {
        if body.len() > MAX_HTTP_READ_ONLY_REQUEST_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "HTTP read-only request body exceeds the {MAX_HTTP_READ_ONLY_REQUEST_BYTES}-byte encoded ceiling"
                ),
            ));
        }
        let mut request = Self::json(url, HttpMethod::Post, body)?;
        request.intent = RequestIntent::ReadOnly;
        Ok(request)
    }

    /// Builds one bounded non-redirecting JSON PATCH request.
    pub fn patch_json(url: Url, body: Vec<u8>) -> Result<Self, ResourceError> {
        Self::json(url, HttpMethod::Patch, body)
    }

    fn json(url: Url, method: HttpMethod, body: Vec<u8>) -> Result<Self, ResourceError> {
        if body.len() > MAX_HTTP_MUTATION_REQUEST_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "HTTP mutation request body exceeds the {MAX_HTTP_MUTATION_REQUEST_BYTES}-byte encoded ceiling"
                ),
            ));
        }
        Ok(Self {
            url,
            method,
            redirect: RedirectBehavior::Refuse,
            body: Some(Bytes::from(body)),
            headers: Vec::new(),
            header_bytes: 0,
            intent: RequestIntent::Mutation,
        })
    }

    /// Adds one validated non-secret end-to-end header.
    pub fn with_header(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, ResourceError> {
        if self.headers.len() >= MAX_SOURCE_REQUEST_HEADERS {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "HTTP request exceeds the {MAX_SOURCE_REQUEST_HEADERS}-header source ceiling"
                ),
            ));
        }
        let name = name.into();
        let value = value.into();
        let parsed_name = header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| invalid_source_header("HTTP request header name is invalid"))?;
        if self.body.is_some() && parsed_name == header::CONTENT_TYPE {
            return Err(invalid_source_header(
                "HTTP JSON requests own their Content-Type header",
            ));
        }
        if is_authority_or_framing_header(&parsed_name) {
            return Err(invalid_source_header(
                "HTTP source header must not control authority, cookies, host, or message framing",
            ));
        }
        if self.headers.iter().any(|header| header.name == parsed_name) {
            return Err(invalid_source_header(
                "HTTP source headers must not contain duplicate names",
            ));
        }
        let parsed_value = header::HeaderValue::from_str(&value)
            .map_err(|_| invalid_source_header("HTTP request header value is invalid"))?;
        let header_bytes = self
            .header_bytes
            .checked_add(name.len())
            .and_then(|bytes| bytes.checked_add(value.len()))
            .ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::LimitExceeded,
                    "HTTP source header byte count overflowed",
                )
            })?;
        if header_bytes > MAX_SOURCE_REQUEST_HEADER_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "HTTP request source headers exceed the {MAX_SOURCE_REQUEST_HEADER_BYTES}-byte ceiling"
                ),
            ));
        }
        self.headers.push(SourceRequestHeader {
            name: parsed_name,
            value: parsed_value,
        });
        self.header_bytes = header_bytes;
        Ok(self)
    }

    /// Returns the requested URL.
    #[must_use]
    pub const fn url(&self) -> &Url {
        &self.url
    }

    pub(super) fn retry_copy(&self) -> Option<Self> {
        if self.intent != RequestIntent::ReadOnly {
            return None;
        }
        Some(Self {
            url: self.url.clone(),
            method: self.method,
            redirect: self.redirect,
            body: self.body.clone(),
            headers: self.headers.clone(),
            header_bytes: self.header_bytes,
            intent: self.intent,
        })
    }
}

fn invalid_source_header(message: &'static str) -> ResourceError {
    ResourceError::new(ErrorCategory::InvalidReference, message)
}

fn is_authority_or_framing_header(name: &header::HeaderName) -> bool {
    matches!(
        name,
        &header::AUTHORIZATION
            | &header::PROXY_AUTHORIZATION
            | &header::COOKIE
            | &header::HOST
            | &header::CONNECTION
            | &header::TRANSFER_ENCODING
            | &header::CONTENT_LENGTH
            | &header::TE
            | &header::TRAILER
            | &header::UPGRADE
    )
}
