use crate::{PathReference, VersionTag};

pub const BEHAVIOR_CONTRACT_VERSION: &str = "1.0.0";
pub const MAX_TEXT_BYTES: usize = 48 * 1024;
pub const MAX_TEXT_LINES: usize = 3_000;
pub const MAX_TEXT_COLUMNS: usize = 512;
pub const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// Complete source-neutral result of reading one UTF-8 Resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadResource {
    canonical_reference: String,
    content_type: &'static str,
    version_tag: VersionTag,
    mutable: bool,
    bounded: bool,
    content: String,
}

impl ReadResource {
    pub fn text(reference: PathReference, content: String) -> Self {
        let version_tag = VersionTag::from_content(content.as_bytes());
        Self {
            canonical_reference: reference.canonical().to_owned(),
            content_type: TEXT_CONTENT_TYPE,
            version_tag,
            mutable: false,
            bounded: false,
            content,
        }
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

    pub fn content(&self) -> &str {
        &self.content
    }
}
