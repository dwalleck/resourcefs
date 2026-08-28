use std::{fmt, io::Cursor, sync::Arc};

use async_trait::async_trait;
use resourcefs_core::{
    DiscoveryAdapter, ErrorCategory, GlobOptions, GlobTarget, HttpsAddress, OperationGuard,
    PathReference, ResourceAddress, ResourceError, SearchOptions, SearchSourceResult, SearchTarget,
    SourceAdapter, SourceGlobResult, SourceResource, VersionTag, select_utf8,
};

use crate::{
    catalog::{SourceCatalogEntry, SourceCatalogMetadata},
    http::{HttpRequest, HttpSubstrate},
    pattern::search_document,
};

/// Read and discovery adapter for allowlisted `https://` Resources.
///
/// Read-only by construction: this type implements no `MutationAdapter`, so
/// there is no code path through which an `https://` reference could be
/// written. `CompiledSources` turns every HTTPS mutation into
/// `unsupported_mutation` rather than routing it anywhere.
///
/// Every request goes through the one bounded [`HttpSubstrate`], which owns
/// origin scoping, address authorization, redirect vetting, and the ceilings.
/// This adapter adds only projection selection and the common result shape.
#[derive(Clone)]
pub struct HttpsSource {
    substrate: Arc<HttpSubstrate>,
}

impl HttpsSource {
    #[must_use]
    pub const fn new(substrate: Arc<HttpSubstrate>) -> Self {
        Self { substrate }
    }

    /// Reads one HTTPS Resource under its projection.
    ///
    /// Reader-mode Markdown is the default projection; `:raw` returns the
    /// bounded original body. A line selector applies to whichever of the two
    /// the reference selected, so `:raw:1-5` slices the original body while a
    /// bare `:1-5` slices the extracted Markdown.
    async fn read_https(
        &self,
        reference: &PathReference,
        address: &HttpsAddress,
        operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        let projection = reference.projection();
        let raw = projection.is_some_and(resourcefs_core::ProjectionSelector::is_raw);
        let request = HttpRequest::get(address.url().clone());

        // Identity is the canonical URL without its selector: a later read of
        // the same document under a different selector is the same Resource.
        let canonical = PathReference::parse(address.as_str().to_owned())?;

        let (content, version_tag) = if raw {
            let response = self.substrate.fetch(request, operation).await?;
            classify_status(response.status())?;
            let body = String::from_utf8(response.body().to_vec()).map_err(|_| {
                ResourceError::new(
                    ErrorCategory::UnsupportedProjection,
                    "response body is not valid UTF-8; :raw returns text only",
                )
            })?;
            let version_tag = VersionTag::from_content(body.as_bytes());
            (body, version_tag)
        } else {
            let document = self.substrate.fetch_reader_mode(request, operation).await?;
            classify_status(document.status())?;
            let markdown = document.markdown().to_owned();
            let version_tag = VersionTag::from_content(markdown.as_bytes());
            (markdown, version_tag)
        };

        // A `:raw` marker with no line selection selects the whole body, so the
        // selector is applied only when it actually narrows the content.
        let has_selection = projection.is_some_and(|selector| {
            selector.line_selection().is_some() || selector.page_offset().is_some()
        });
        if !has_selection {
            return SourceResource::text_projection(canonical, content, version_tag);
        }
        let selected = select_utf8(Cursor::new(content), projection)?;
        let (selected_content, selected_tag, _) = selected.into_parts();
        SourceResource::text_projection(canonical, selected_content, selected_tag)
    }
}

/// Maps an upstream status onto the Behavior Contract before any body is
/// treated as content (rfs-0ox5; rfs-g2z9: "an upstream 404 is `not_found`").
///
/// Only the numeric status is consulted, never body prose, and an error page
/// is never a Resource: rendering one would hand the caller a "not found"
/// article carrying its own Version Tag, searchable and retainable as if it
/// were the document. The vocabulary matches the GitHub adapter's
/// machine-signal mapping so one status means one category across sources.
fn classify_status(status: u16) -> Result<(), ResourceError> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(ResourceError::new(
            ErrorCategory::PermissionDenied,
            format!("upstream denied the request with HTTP status {status}"),
        )),
        404 | 410 => Err(ResourceError::new(
            ErrorCategory::NotFound,
            format!("upstream reports no document at this reference (HTTP status {status})"),
        )),
        status => Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            format!("upstream returned HTTP status {status}"),
        )),
    }
}

impl fmt::Debug for HttpsSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpsSource")
            .finish_non_exhaustive()
    }
}

impl SourceCatalogMetadata for HttpsSource {
    fn catalog_entries(&self) -> Result<Vec<SourceCatalogEntry>, ResourceError> {
        Ok(vec![SourceCatalogEntry::new(
            "https://",
            "https://<host>/<path>[:raw][:selector] (allowlisted origins only; reader-mode Markdown by default, :raw for the original body)",
            "https://example.com/doc",
            None,
        )?])
    }
}

#[async_trait]
impl SourceAdapter for HttpsSource {
    async fn read(
        &self,
        reference: &PathReference,
        operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        let ResourceAddress::Https(address) = reference.address() else {
            return Err(unsupported_https_target());
        };
        // The caller's guard reaches the substrate, so a cancelled `rfs_read`
        // abandons the request instead of running to the full request timeout.
        self.read_https(reference, address, operation).await
    }
}

#[async_trait]
impl DiscoveryAdapter for HttpsSource {
    async fn search(
        &self,
        target: &SearchTarget,
        pattern: &str,
        options: SearchOptions,
        operation: &OperationGuard,
    ) -> Result<SearchSourceResult, ResourceError> {
        let reference = target.reference().ok_or_else(unsupported_https_target)?;
        let ResourceAddress::Https(address) = reference.address() else {
            return Err(unsupported_https_target());
        };
        // Search reads the reader-mode rendering, so a match line is the line a
        // caller would see from an unselected read of the same reference.
        let document = self
            .substrate
            .fetch_reader_mode(HttpRequest::get(address.url().clone()), operation)
            .await?;
        classify_status(document.status())?;
        let canonical = PathReference::parse(address.as_str().to_owned())?;
        let markdown = document.markdown().to_owned();
        let pattern = pattern.to_owned();
        let case_sensitive = options.case_sensitive();

        // A compiled PCRE2 matcher is not `Send`, so matching runs on a
        // blocking worker — the same structure the Artifact and Local adapters
        // use.
        tokio::task::spawn_blocking(move || {
            search_document(&markdown, &canonical, &pattern, case_sensitive)
        })
        .await
        .map_err(discovery_worker_error)?
    }

    async fn glob(
        &self,
        _target: &GlobTarget,
        _options: GlobOptions,
        _operation: &OperationGuard,
    ) -> Result<SourceGlobResult, ResourceError> {
        // A remote origin exposes no enumerable namespace: there is nothing to
        // walk without crawling, which this source deliberately does not do.
        Err(ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            "https:// Resources cannot be enumerated by glob; read or search one URL",
        ))
    }
}

fn discovery_worker_error(error: tokio::task::JoinError) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!("HTTPS discovery worker failed: {error}"),
    )
}

fn unsupported_https_target() -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        "HTTPS Source Adapter requires one https:// Resource",
    )
}
