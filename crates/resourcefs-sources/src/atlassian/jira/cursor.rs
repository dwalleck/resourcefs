use std::{borrow::Cow, io::Write};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use resourcefs_core::{
    ErrorCategory, JiraAddress, MAX_PATH_REFERENCE_BYTES, PathReference, ProjectionSelector,
    ResourceError, SourceCursor,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{super::wire::collections::NativeIssueToken, AtlassianSite};

/// Ownership is a routing constraint, not a signature or authorization capability.
pub(super) struct CursorOwner<'a> {
    address: &'a JiraAddress,
    canonical: PathReference,
    origin: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope<'a> {
    version: u8,
    #[serde(borrow)]
    owner: Cow<'a, str>,
    #[serde(borrow)]
    origin: Cow<'a, str>,
    #[serde(borrow)]
    token: Cow<'a, str>,
}

impl<'a> CursorOwner<'a> {
    pub(super) fn new(
        address: &'a JiraAddress,
        site: &AtlassianSite,
    ) -> Result<Self, ResourceError> {
        if !matches!(
            address,
            JiraAddress::Issues { .. }
                | JiraAddress::ProjectIssues { .. }
                | JiraAddress::Query { .. }
        ) || address.site() != site.id()
        {
            return Err(invalid_cursor());
        }
        Ok(Self {
            address,
            canonical: PathReference::jira(address.clone(), None)?,
            origin: format!(
                "{:x}",
                Sha256::digest(site.origin().base_url().as_str().as_bytes())
            ),
        })
    }

    pub(super) fn decode(&self, cursor: &SourceCursor) -> Result<NativeIssueToken, ResourceError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor.as_str())
            .map_err(|_| invalid_cursor())?;
        let envelope: Envelope<'_> =
            serde_json::from_slice(&bytes).map_err(|_| invalid_cursor())?;
        if envelope.version != 1
            || envelope.owner != self.canonical.requested()
            || envelope.origin != self.origin
        {
            return Err(invalid_cursor());
        }
        let token =
            NativeIssueToken::new(envelope.token.into_owned()).map_err(|_| invalid_cursor())?;
        self.validate_continuation(&token)
            .map_err(|_| invalid_cursor())?;
        Ok(token)
    }

    pub(super) fn continuation(
        &self,
        token: NativeIssueToken,
    ) -> Result<PathReference, ResourceError> {
        let bytes = self.serialize_token(&token, Vec::new())?;
        let selector =
            SourceCursor::new(URL_SAFE_NO_PAD.encode(bytes)).map_err(|_| cursor_too_large())?;
        PathReference::jira(
            self.address.clone(),
            Some(ProjectionSelector::from_source_cursor(selector)),
        )
        .map_err(|_| cursor_too_large())
    }

    pub(super) fn validate_continuation(
        &self,
        token: &NativeIssueToken,
    ) -> Result<(), ResourceError> {
        self.serialize_token(token, std::io::sink()).map(|_| ())
    }

    fn serialize_token<W: Write>(
        &self,
        token: &NativeIssueToken,
        writer: W,
    ) -> Result<W, ResourceError> {
        // Bound escaped JSON before allocation or native-token forwarding. Measuring an internal
        // page's continuation uses a sink, avoiding an unused encoded cursor allocation.
        let encoded_limit = MAX_PATH_REFERENCE_BYTES
            .checked_sub(self.canonical.requested().len() + ":cursor:".len())
            .ok_or_else(cursor_too_large)?;
        let limit = encoded_limit * 3 / 4;
        if token.as_str().len() > limit {
            return Err(cursor_too_large());
        }
        let mut writer = CursorBytes {
            writer,
            remaining: limit,
        };
        let envelope = Envelope {
            version: 1,
            owner: Cow::Borrowed(self.canonical.requested()),
            origin: Cow::Borrowed(&self.origin),
            token: Cow::Borrowed(token.as_str()),
        };
        serde_json::to_writer(&mut writer, &envelope).map_err(|_| cursor_too_large())?;
        Ok(writer.writer)
    }
}

struct CursorBytes<W> {
    writer: W,
    remaining: usize,
}

impl<W: Write> Write for CursorBytes<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other("cursor exceeds reference ceiling"));
        }
        let written = self.writer.write(bytes)?;
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn invalid_cursor() -> ResourceError {
    ResourceError::new(
        ErrorCategory::InvalidReference,
        "Jira cursor is malformed or belongs to another collection or mount",
    )
}

fn cursor_too_large() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        "Jira continuation exceeds the reference ceiling; narrow the collection before reading",
    )
}
