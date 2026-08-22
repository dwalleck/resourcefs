use crate::{
    ErrorCategory, MAX_ARTIFACT_BYTES, MAX_DISCOVERY_RESULTS, MAX_SESSION_BYTES, MAX_TEXT_BYTES,
    MAX_TEXT_COLUMNS, MAX_TEXT_LINES, ResourceError, TextLimits,
};

/// Maximum decoded image payload admitted by the ResourceFS 1.0 contract.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Optional text-result limit overrides supplied by an operator profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextLimitInput {
    pub bytes: Option<usize>,
    pub lines: Option<usize>,
    pub columns: Option<usize>,
}

/// Optional discovery-result limit overrides supplied by an operator profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiscoveryLimitInput {
    pub search_matches: Option<usize>,
    pub glob_entries: Option<usize>,
    pub listing_entries: Option<usize>,
}

/// Optional session-storage limit overrides supplied by an operator profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StorageLimitInput {
    pub object_bytes: Option<usize>,
    pub session_bytes: Option<usize>,
}

/// Source-neutral operator overrides for ResourceFS server ceilings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServerLimitsInput {
    pub text: TextLimitInput,
    pub discovery: DiscoveryLimitInput,
    pub image_bytes: Option<usize>,
    pub storage: StorageLimitInput,
}

/// Validated lower-only server ceilings shared by every engine in one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerLimits {
    text_bytes: usize,
    text_lines: usize,
    text_columns: usize,
    search_matches: usize,
    glob_entries: usize,
    listing_entries: usize,
    image_bytes: usize,
    object_bytes: usize,
    session_bytes: usize,
}

impl ServerLimits {
    pub fn new(input: ServerLimitsInput) -> Result<Self, ResourceError> {
        let text_bytes = validate_limit("text.bytes", input.text.bytes, MAX_TEXT_BYTES)?;
        let text_lines = validate_limit("text.lines", input.text.lines, MAX_TEXT_LINES)?;
        let text_columns = validate_limit("text.columns", input.text.columns, MAX_TEXT_COLUMNS)?;
        let search_matches = validate_limit(
            "discovery.searchMatches",
            input.discovery.search_matches,
            MAX_DISCOVERY_RESULTS,
        )?;
        let glob_entries = validate_limit(
            "discovery.globEntries",
            input.discovery.glob_entries,
            MAX_DISCOVERY_RESULTS,
        )?;
        let listing_entries = validate_limit(
            "discovery.listingEntries",
            input.discovery.listing_entries,
            MAX_DISCOVERY_RESULTS,
        )?;
        let image_bytes = validate_limit("imageBytes", input.image_bytes, MAX_IMAGE_BYTES)?;
        let object_bytes = validate_limit(
            "storage.objectBytes",
            input.storage.object_bytes,
            MAX_ARTIFACT_BYTES,
        )?;
        let session_bytes = validate_limit(
            "storage.sessionBytes",
            input.storage.session_bytes,
            MAX_SESSION_BYTES,
        )?;
        if object_bytes > session_bytes {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                "storage.objectBytes must not exceed storage.sessionBytes",
            ));
        }
        Ok(Self {
            text_bytes,
            text_lines,
            text_columns,
            search_matches,
            glob_entries,
            listing_entries,
            image_bytes,
            object_bytes,
            session_bytes,
        })
    }

    pub const fn text_bytes(self) -> usize {
        self.text_bytes
    }

    pub const fn text_lines(self) -> usize {
        self.text_lines
    }

    pub const fn text_columns(self) -> usize {
        self.text_columns
    }

    pub const fn search_matches(self) -> usize {
        self.search_matches
    }

    pub const fn glob_entries(self) -> usize {
        self.glob_entries
    }

    pub const fn listing_entries(self) -> usize {
        self.listing_entries
    }

    pub const fn image_bytes(self) -> usize {
        self.image_bytes
    }

    pub const fn object_bytes(self) -> usize {
        self.object_bytes
    }

    pub const fn session_bytes(self) -> usize {
        self.session_bytes
    }

    pub(crate) const fn text_limits(self) -> TextLimits {
        TextLimits::from_validated(self.text_bytes, self.text_lines, self.text_columns)
    }
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            text_bytes: MAX_TEXT_BYTES,
            text_lines: MAX_TEXT_LINES,
            text_columns: MAX_TEXT_COLUMNS,
            search_matches: MAX_DISCOVERY_RESULTS,
            glob_entries: MAX_DISCOVERY_RESULTS,
            listing_entries: MAX_DISCOVERY_RESULTS,
            image_bytes: MAX_IMAGE_BYTES,
            object_bytes: MAX_ARTIFACT_BYTES,
            session_bytes: MAX_SESSION_BYTES,
        }
    }
}

fn validate_limit(
    field: &'static str,
    value: Option<usize>,
    maximum: usize,
) -> Result<usize, ResourceError> {
    let value = value.unwrap_or(maximum);
    if value == 0 || value > maximum {
        return Err(ResourceError::new(
            ErrorCategory::LimitExceeded,
            format!("{field} must be between 1 and {maximum}"),
        ));
    }
    Ok(value)
}
