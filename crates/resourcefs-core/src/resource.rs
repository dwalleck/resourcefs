use url::Url;

use crate::{
    ErrorCategory, PathReference, ResourceAddress, ResourceError, VersionTag, WorkspaceAddress,
};

pub const BEHAVIOR_CONTRACT_VERSION: &str = "1.0.0";
pub const MAX_TEXT_BYTES: usize = 48 * 1024;
pub const MAX_TEXT_LINES: usize = 3_000;
pub const MAX_TEXT_COLUMNS: usize = 512;
pub const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
pub const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// Complete source-neutral result of reading one UTF-8 Resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadResource {
    canonical_reference: String,
    content_type: &'static str,
    version_tag: VersionTag,
    mutable: bool,
    bounded: bool,
    backing_file_uri: Option<String>,
    content: String,
}

impl ReadResource {
    pub fn text(reference: PathReference, content: String) -> Result<Self, ResourceError> {
        if !matches!(
            reference.address(),
            ResourceAddress::Workspace(WorkspaceAddress::Canonical { .. })
        ) {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "ReadResource identity must be a canonical workspace reference",
            ));
        }
        let version_tag = VersionTag::from_content(content.as_bytes());
        Ok(Self {
            canonical_reference: reference.requested().to_owned(),
            content_type: TEXT_CONTENT_TYPE,
            version_tag,
            mutable: false,
            bounded: false,
            backing_file_uri: None,
            content,
        })
    }

    pub fn canonical_reference(&self) -> &str {
        &self.canonical_reference
    }

    pub const fn content_type(&self) -> &'static str {
        self.content_type
    }

    pub const fn version_tag(&self) -> &VersionTag {
        &self.version_tag
    }

    pub const fn is_mutable(&self) -> bool {
        self.mutable
    }

    pub const fn is_bounded(&self) -> bool {
        self.bounded
    }

    pub fn with_backing_file_uri(
        mut self,
        backing_file_uri: impl Into<String>,
    ) -> Result<Self, ResourceError> {
        let backing_file_uri = backing_file_uri.into();
        let parsed = Url::parse(&backing_file_uri).map_err(|_| {
            ResourceError::new(
                ErrorCategory::InvalidReference,
                "backing-file metadata must be a valid local file URI",
            )
        })?;
        if parsed.scheme() != "file"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.to_file_path().is_err()
        {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "backing-file metadata must be a valid local file URI",
            ));
        }
        self.backing_file_uri = Some(parsed.into());
        Ok(self)
    }

    pub fn backing_file_uri(&self) -> Option<&str> {
        self.backing_file_uri.as_deref()
    }

    pub fn content(&self) -> &str {
        &self.content
    }
}
