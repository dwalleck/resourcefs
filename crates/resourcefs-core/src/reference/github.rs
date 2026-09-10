use super::{
    GithubRepositoryIdentity, ProjectionSelector, encode_rfc3986_segment, invalid_reference,
    percent_decode, projection_candidate_split,
};
use crate::ResourceError;

pub(crate) const GITHUB_PREFIX: &str = "github://";
const FACTS_SUFFIX: &str = "/facts";

/// A complete lowercase hexadecimal Git commit object identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GithubCommitId(String);

impl GithubCommitId {
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        if value.len() != 40
            || !value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(invalid_reference(
                "GitHub commit ID must be exactly 40 lowercase hexadecimal characters",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated Git path represented by decoded UTF-8 segments.
///
/// Unlike filesystem paths, Git names preserve backslashes and control bytes;
/// canonical rendering percent-encodes every byte outside the RFC3986
/// unreserved alphabet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GithubSourcePath {
    segments: Vec<String>,
    canonical: String,
}

impl GithubSourcePath {
    pub fn new(segments: Vec<String>) -> Result<Self, ResourceError> {
        if segments.is_empty() {
            return Err(invalid_reference(
                "GitHub source path must contain at least one segment",
            ));
        }
        for segment in &segments {
            validate_decoded_segment(segment)?;
        }
        let mut canonical = String::new();
        for (index, segment) in segments.iter().enumerate() {
            if index != 0 {
                canonical.push('/');
            }
            encode_rfc3986_segment(segment, &mut canonical);
        }
        Ok(Self {
            segments,
            canonical,
        })
    }

    pub fn parse(encoded: &str) -> Result<Self, ResourceError> {
        if encoded.is_empty() {
            return Err(invalid_reference(
                "GitHub source path must contain at least one segment",
            ));
        }
        let segments = encoded
            .split('/')
            .map(parse_encoded_segment)
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(segments)
    }

    /// Exact decoded UTF-8 names; percent-looking bytes remain literal bytes.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Canonical percent-encoded path, without resource operands or selectors.
    pub fn as_str(&self) -> &str {
        &self.canonical
    }
}

/// An immutable GitHub commit or source identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GithubAddress {
    Commit {
        repository: GithubRepositoryIdentity,
        commit: GithubCommitId,
    },
    Source {
        repository: GithubRepositoryIdentity,
        commit: GithubCommitId,
        path: GithubSourcePath,
    },
}

impl GithubAddress {
    pub const fn repository(&self) -> &GithubRepositoryIdentity {
        match self {
            Self::Commit { repository, .. } | Self::Source { repository, .. } => repository,
        }
    }

    pub const fn commit(&self) -> &GithubCommitId {
        match self {
            Self::Commit { commit, .. } | Self::Source { commit, .. } => commit,
        }
    }

    pub const fn path(&self) -> Option<&GithubSourcePath> {
        match self {
            Self::Commit { .. } => None,
            Self::Source { path, .. } => Some(path),
        }
    }

    pub fn canonical_reference(&self) -> String {
        match self {
            Self::Commit { repository, commit } => format!(
                "{GITHUB_PREFIX}{}/commits/{}/facts",
                repository.as_str(),
                commit.as_str()
            ),
            Self::Source {
                repository,
                commit,
                path,
            } => format!(
                "{GITHUB_PREFIX}{}/source/{}/{}/facts",
                repository.as_str(),
                commit.as_str(),
                path.as_str()
            ),
        }
    }
}

pub(crate) fn parse_github_reference(
    input: &str,
) -> Result<(GithubAddress, Option<ProjectionSelector>), ResourceError> {
    let (base, projection) = projection_candidate_split(input).map_or_else(
        || Ok((input, None)),
        |(base, selector)| {
            ProjectionSelector::parse(selector).map(|selector| (base, Some(selector)))
        },
    )?;
    let address = parse_github_address(base)?;
    Ok((address, projection))
}

fn parse_github_address(input: &str) -> Result<GithubAddress, ResourceError> {
    let body = input
        .strip_prefix(GITHUB_PREFIX)
        .ok_or_else(|| invalid_reference("malformed GitHub reference"))?;
    let body = body
        .strip_suffix(FACTS_SUFFIX)
        .ok_or_else(|| invalid_reference("GitHub immutable references must end with /facts"))?;
    if body.is_empty() {
        return Err(invalid_reference("malformed GitHub reference"));
    }
    let mut segments = body.splitn(5, '/');
    let (Some(owner), Some(repository), Some(kind), Some(commit)) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return Err(invalid_reference("malformed GitHub immutable reference"));
    };
    match (kind, segments.next()) {
        ("commits", None) => Ok(GithubAddress::Commit {
            repository: GithubRepositoryIdentity::new(owner, repository)?,
            commit: GithubCommitId::new(commit)?,
        }),
        ("source", Some(path)) if !path.is_empty() => Ok(GithubAddress::Source {
            repository: GithubRepositoryIdentity::new(owner, repository)?,
            commit: GithubCommitId::new(commit)?,
            path: GithubSourcePath::parse(path)?,
        }),
        _ => Err(invalid_reference("malformed GitHub immutable reference")),
    }
}

fn parse_encoded_segment(encoded: &str) -> Result<String, ResourceError> {
    // Every byte outside an escape must already be an RFC3986 unreserved ASCII byte.
    if encoded
        .bytes()
        .any(|byte| byte != b'%' && !is_unreserved(byte))
    {
        return Err(invalid_reference(
            "GitHub source path segments must use unreserved bytes or percent escapes",
        ));
    }
    percent_decode(encoded)
}

fn validate_decoded_segment(segment: &str) -> Result<(), ResourceError> {
    if segment.is_empty() || matches!(segment, "." | "..") || segment.contains('/') {
        return Err(invalid_reference(
            "GitHub source path segments must be nonempty and must not be dot or slash aliases",
        ));
    }
    if segment.contains('\0') {
        return Err(invalid_reference("GitHub source path must not contain NUL"));
    }
    Ok(())
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}
