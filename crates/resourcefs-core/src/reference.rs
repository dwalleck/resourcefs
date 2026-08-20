use std::{
    collections::HashSet,
    fmt::{self, Write as _},
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};
use url::Url;

use crate::{ErrorCategory, ResourceError};

const WORKSPACE_PREFIX: &str = "rfs://workspace/";
pub const MAX_PATH_REFERENCE_BYTES: usize = 64 * 1024;
pub const MAX_WORKSPACE_ROOTS: usize = 256;

/// Stable private identifier for one configured Workspace Root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorkspaceRootId(String);

impl WorkspaceRootId {
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        let mut characters = value.chars();
        let valid = characters
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
            && characters.all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            });
        if !valid {
            return Err(invalid_reference(
                "Workspace Root ID must start with an ASCII letter or digit and contain only ASCII letters, digits, '.', '_', or '-'",
            ));
        }
        Ok(Self(value))
    }

    pub fn from_client_uri(canonical_uri: &str) -> Self {
        let digest = Sha256::digest(canonical_uri.as_bytes());
        let mut id = String::with_capacity("client-".len() + digest.len() * 2);
        id.push_str("client-");
        for byte in digest {
            write!(&mut id, "{byte:02x}").expect("writing to a String cannot fail");
        }
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspaceRootId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Validated UTF-8 path below a Workspace Root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspacePath(PathBuf);

impl WorkspacePath {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, ResourceError> {
        let path = path.as_ref();
        let mut normalized = PathBuf::new();
        let mut has_component = false;
        for component in path.components() {
            match component {
                Component::Normal(value) => {
                    value
                        .to_str()
                        .ok_or_else(|| invalid_reference("Path Reference must be valid UTF-8"))?;
                    normalized.push(value);
                    has_component = true;
                }
                Component::CurDir => {
                    return Err(invalid_reference(
                        "Path Reference must not contain a '.' component",
                    ));
                }
                Component::ParentDir => {
                    return Err(permission_denied(
                        "Path Reference must not traverse outside a Workspace Root",
                    ));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(invalid_reference(
                        "WorkspacePath must be relative to a Workspace Root",
                    ));
                }
            }
        }
        if !has_component {
            return Err(invalid_reference(
                "Path Reference must address a Resource below a Workspace Root",
            ));
        }
        Ok(Self(normalized))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    fn render(&self) -> String {
        let mut rendered = String::new();
        for component in self.0.components() {
            let Component::Normal(value) = component else {
                unreachable!("WorkspacePath construction permits only normal components");
            };
            if !rendered.is_empty() {
                rendered.push('/');
            }
            let value = value
                .to_str()
                .expect("WorkspacePath construction requires UTF-8 components");
            encode_component(value, &mut rendered);
        }
        rendered
    }
}

/// Parsed workspace address before source-specific resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceAddress {
    Relative(WorkspacePath),
    Absolute(PathBuf),
    FileUri(Url),
    Canonical {
        root: WorkspaceRootId,
        path: WorkspacePath,
    },
}

/// Preserved projection spelling. Execution belongs to the selector owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionSelector(String);

impl ProjectionSelector {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Alternate interpretation when a trailing selector is syntactically valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedWorkspaceAddress {
    base: WorkspaceAddress,
    selector: ProjectionSelector,
}

impl SelectedWorkspaceAddress {
    pub fn base(&self) -> &WorkspaceAddress {
        &self.base
    }

    pub fn selector(&self) -> &ProjectionSelector {
        &self.selector
    }
}

/// Syntax-only Path Reference. Filesystem existence and root selection are source concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathReference {
    requested: String,
    literal: WorkspaceAddress,
    selector_candidate: Option<SelectedWorkspaceAddress>,
}

impl PathReference {
    pub fn parse(input: impl Into<String>) -> Result<Self, ResourceError> {
        let requested = input.into();
        validate_reference_input(&requested)?;
        let literal = parse_address(&requested)?;
        let selector_candidate = selector_split(&requested).and_then(|(base, selector)| {
            parse_address(base)
                .ok()
                .map(|base| SelectedWorkspaceAddress {
                    base,
                    selector: ProjectionSelector(selector.to_owned()),
                })
        });
        Ok(Self {
            requested,
            literal,
            selector_candidate,
        })
    }

    pub fn canonical(root: WorkspaceRootId, path: WorkspacePath) -> Self {
        let mut requested = String::with_capacity(
            WORKSPACE_PREFIX.len() + root.as_str().len() + path.as_path().as_os_str().len() + 1,
        );
        requested.push_str(WORKSPACE_PREFIX);
        requested.push_str(root.as_str());
        requested.push('/');
        requested.push_str(&path.render());
        Self {
            requested,
            literal: WorkspaceAddress::Canonical { root, path },
            selector_candidate: None,
        }
    }

    pub fn requested(&self) -> &str {
        &self.requested
    }

    pub fn literal(&self) -> &WorkspaceAddress {
        &self.literal
    }

    pub fn selector_candidate(&self) -> Option<&SelectedWorkspaceAddress> {
        self.selector_candidate.as_ref()
    }
}

/// Source-neutral identity and selector metadata for one active root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot {
    id: WorkspaceRootId,
    canonical_uri: String,
    selector_name: Option<String>,
}

impl WorkspaceRoot {
    pub fn new(
        id: WorkspaceRootId,
        canonical_uri: impl Into<String>,
        selector_name: Option<String>,
    ) -> Result<Self, ResourceError> {
        let canonical_uri = canonical_uri.into();
        parse_file_uri(&canonical_uri).map_err(|_| {
            invalid_reference("Workspace Root canonical URI must be a local file URI")
        })?;
        if selector_name
            .as_ref()
            .is_some_and(|name| WorkspaceRootId::new(name.clone()).is_err())
        {
            return Err(invalid_reference(
                "Workspace Root selector name must use Workspace Root ID grammar",
            ));
        }
        Ok(Self {
            id,
            canonical_uri,
            selector_name,
        })
    }

    pub fn id(&self) -> &WorkspaceRootId {
        &self.id
    }

    pub fn canonical_uri(&self) -> &str {
        &self.canonical_uri
    }

    pub fn selector_name(&self) -> Option<&str> {
        self.selector_name.as_deref()
    }
}

/// Immutable source-neutral view of active root identities and primary selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRootSet {
    roots: Vec<WorkspaceRoot>,
    primary: Option<WorkspaceRootId>,
}

impl WorkspaceRootSet {
    pub fn new(
        roots: Vec<WorkspaceRoot>,
        primary_selector: Option<&str>,
    ) -> Result<Self, ResourceError> {
        if roots.is_empty() {
            return Err(invalid_reference(
                "Workspace Root set must contain at least one root",
            ));
        }
        if roots.len() > MAX_WORKSPACE_ROOTS {
            return Err(limit_exceeded("Workspace Root count exceeds 256"));
        }
        let mut ids = HashSet::with_capacity(roots.len());
        let mut uris = HashSet::with_capacity(roots.len());
        for root in &roots {
            if !ids.insert(root.id.clone()) {
                return Err(invalid_reference("Workspace Root IDs must be unique"));
            }
            if !uris.insert(root.canonical_uri.clone()) {
                return Err(invalid_reference(
                    "Workspace Root canonical URIs must be unique",
                ));
            }
        }

        let primary = if roots.len() == 1 {
            Some(roots[0].id.clone())
        } else {
            primary_selector.and_then(|selector| {
                let mut matches = roots
                    .iter()
                    .filter(|root| root.selector_name() == Some(selector));
                let selected = matches.next()?;
                matches.next().is_none().then(|| selected.id.clone())
            })
        };
        Ok(Self { roots, primary })
    }

    pub fn roots(&self) -> &[WorkspaceRoot] {
        &self.roots
    }

    pub fn primary(&self) -> Option<&WorkspaceRootId> {
        self.primary.as_ref()
    }

    pub fn get(&self, id: &WorkspaceRootId) -> Option<&WorkspaceRoot> {
        self.roots.iter().find(|root| root.id() == id)
    }

    pub fn equivalent_to(&self, other: &Self) -> bool {
        if self.primary != other.primary || self.roots.len() != other.roots.len() {
            return false;
        }
        let mut left = self
            .roots
            .iter()
            .map(|root| (root.id.as_str(), root.canonical_uri.as_str()))
            .collect::<Vec<_>>();
        let mut right = other
            .roots
            .iter()
            .map(|root| (root.id.as_str(), root.canonical_uri.as_str()))
            .collect::<Vec<_>>();
        left.sort_unstable();
        right.sort_unstable();
        left == right
    }
}

fn validate_reference_input(input: &str) -> Result<(), ResourceError> {
    if input.len() > MAX_PATH_REFERENCE_BYTES {
        return Err(limit_exceeded("Path Reference exceeds 64 KiB"));
    }
    if input.is_empty() {
        return Err(invalid_reference("Path Reference must not be empty"));
    }
    if input.contains('\0') {
        return Err(invalid_reference("Path Reference must not contain NUL"));
    }
    validate_percent_encoding(input)?;
    Ok(())
}

fn parse_address(input: &str) -> Result<WorkspaceAddress, ResourceError> {
    let delimiter_input = input.strip_prefix("\\\\?\\").unwrap_or(input);
    if delimiter_input.contains('?') || delimiter_input.contains('#') {
        return Err(invalid_reference(
            "literal '?' and '#' characters must be percent-encoded",
        ));
    }
    if let Some(kind) = windows_path_kind(input) {
        return match kind {
            WindowsPathKind::Absolute if valid_absolute_windows_path(input) => {
                validate_path_segments(input, true)?;
                Ok(WorkspaceAddress::Absolute(PathBuf::from(percent_decode(
                    input,
                )?)))
            }
            WindowsPathKind::Absolute
            | WindowsPathKind::DriveRelative
            | WindowsPathKind::RootRelative => Err(invalid_reference(
                "Windows path must be fully qualified with a drive root or UNC share",
            )),
        };
    }

    if let Some(workspace_path) = input.strip_prefix(WORKSPACE_PREFIX) {
        let (root, path) = workspace_path.split_once('/').ok_or_else(|| {
            invalid_reference("canonical workspace reference must include a root and path")
        })?;
        let root = WorkspaceRootId::new(root.to_owned())?;
        let path = parse_workspace_path(path)?;
        return Ok(WorkspaceAddress::Canonical { root, path });
    }
    if input.starts_with("rfs:") {
        return Err(invalid_reference("malformed canonical workspace reference"));
    }

    if input.starts_with("file:") {
        return parse_file_uri(input).map(WorkspaceAddress::FileUri);
    }

    if input.contains("://") {
        return Err(invalid_reference("unsupported Path Reference scheme"));
    }

    #[cfg(windows)]
    if input.starts_with('/') {
        return Err(invalid_reference(
            "root-relative Windows paths are not supported",
        ));
    }
    #[cfg(not(windows))]
    if input.starts_with('/') {
        validate_path_segments(input.trim_start_matches('/'), false)?;
        return Ok(WorkspaceAddress::Absolute(PathBuf::from(decode_path(
            input, true,
        )?)));
    }

    parse_workspace_path(input).map(WorkspaceAddress::Relative)
}

fn parse_file_uri(input: &str) -> Result<Url, ResourceError> {
    validate_path_segments(input, true)?;
    let uri = Url::parse(input).map_err(|_| invalid_reference("malformed file URI"))?;
    if uri.scheme() != "file"
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.port().is_some()
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        return Err(invalid_reference(
            "file URI must not contain credentials, a port, query, or fragment",
        ));
    }

    #[cfg(not(windows))]
    if !matches!(uri.host_str(), None | Some("") | Some("localhost")) {
        return Err(invalid_reference("file URI host is not local"));
    }

    #[cfg(windows)]
    if uri.to_file_path().is_err() {
        return Err(invalid_reference(
            "file URI must identify a local drive or UNC path",
        ));
    }

    Ok(uri)
}

fn parse_workspace_path(input: &str) -> Result<WorkspacePath, ResourceError> {
    validate_path_segments(input, cfg!(windows))?;
    WorkspacePath::new(PathBuf::from(decode_path(input, false)?))
}

fn validate_path_segments(input: &str, backslash_is_separator: bool) -> Result<(), ResourceError> {
    for component in
        input.split(|character| character == '/' || backslash_is_separator && character == '\\')
    {
        if component.is_empty() {
            continue;
        }
        let decoded = percent_decode(component)?;
        if decoded == "." || decoded == ".." && component.contains('%') {
            return Err(invalid_reference(
                "Path Reference must not encode '.' or '..' components",
            ));
        }
        if decoded == ".." {
            return Err(permission_denied(
                "Path Reference must not traverse outside a Workspace Root",
            ));
        }
    }
    Ok(())
}

fn decode_path(input: &str, preserve_leading_slash: bool) -> Result<String, ResourceError> {
    let mut decoded = String::with_capacity(input.len());
    if preserve_leading_slash && input.starts_with('/') {
        decoded.push('/');
    }
    let mut first = true;
    for component in input.split('/').filter(|component| !component.is_empty()) {
        if (!first || preserve_leading_slash) && !decoded.ends_with('/') {
            decoded.push('/');
        }
        decoded.push_str(&percent_decode(component)?);
        first = false;
    }
    Ok(decoded)
}

// `url` deliberately normalizes URI escapes. Workspace paths need stricter
// single-pass decoding that rejects encoded separators before normalization.
fn validate_percent_encoding(input: &str) -> Result<(), ResourceError> {
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        let high = *bytes
            .get(index + 1)
            .ok_or_else(|| invalid_reference("malformed percent escape"))?;
        let low = *bytes
            .get(index + 2)
            .ok_or_else(|| invalid_reference("malformed percent escape"))?;
        let value = decode_hex(high)
            .zip(decode_hex(low))
            .map(|(high, low)| high << 4 | low)
            .ok_or_else(|| invalid_reference("malformed percent escape"))?;
        if matches!(value, b'/' | b'\\') {
            return Err(invalid_reference(
                "Path Reference must not contain an encoded separator",
            ));
        }
        index += 3;
    }
    Ok(())
}

fn percent_decode(input: &str) -> Result<String, ResourceError> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = decode_hex(bytes[index + 1]).expect("percent escapes were validated");
            let low = decode_hex(bytes[index + 2]).expect("percent escapes were validated");
            decoded.push(high << 4 | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(decoded)
        .map_err(|_| invalid_reference("percent-decoded Path Reference must be UTF-8"))?;
    if decoded.contains('\0') {
        return Err(invalid_reference("Path Reference must not decode to NUL"));
    }
    Ok(decoded)
}

fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn encode_component(component: &str, output: &mut String) {
    for character in component.chars() {
        if matches!(character, '%' | ':' | '?' | '#' | '/') {
            for byte in character.to_string().bytes() {
                write!(output, "%{byte:02X}").expect("writing to a String cannot fail");
            }
        } else {
            output.push(character);
        }
    }
}

fn selector_split(input: &str) -> Option<(&str, &str)> {
    if let Some(index) = input.rfind(":raw") {
        let selector = &input[index + 1..];
        if valid_selector(selector) {
            return Some((&input[..index], selector));
        }
    }
    let (base, selector) = input.rsplit_once(':')?;
    valid_selector(selector).then_some((base, selector))
}

fn valid_selector(selector: &str) -> bool {
    if selector == "raw" {
        return true;
    }
    let ranges = selector.strip_prefix("raw:").unwrap_or(selector);
    !ranges.is_empty() && ranges.split(',').all(valid_range)
}

fn valid_range(range: &str) -> bool {
    if let Some((start, count)) = range.split_once('+') {
        return positive_integer(start).is_some() && positive_integer(count).is_some();
    }
    if let Some((start, end)) = range.split_once('-') {
        let Some(start) = positive_integer(start) else {
            return false;
        };
        return end.is_empty() || positive_integer(end).is_some_and(|end| end >= start);
    }
    positive_integer(range).is_some()
}

fn positive_integer(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|value| *value != 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsPathKind {
    Absolute,
    DriveRelative,
    RootRelative,
}

fn windows_path_kind(input: &str) -> Option<WindowsPathKind> {
    let bytes = input.as_bytes();
    if input.starts_with("\\\\") {
        return Some(WindowsPathKind::Absolute);
    }
    if input.starts_with('\\') {
        return Some(WindowsPathKind::RootRelative);
    }
    if bytes.get(1) == Some(&b':') && bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return Some(if matches!(bytes.get(2), Some(b'\\' | b'/')) {
            WindowsPathKind::Absolute
        } else {
            WindowsPathKind::DriveRelative
        });
    }
    None
}

fn valid_absolute_windows_path(input: &str) -> bool {
    if let Some(tail) = input.strip_prefix("\\\\?\\UNC\\") {
        return valid_unc_tail(tail);
    }
    if let Some(tail) = input.strip_prefix("\\\\?\\") {
        return valid_drive_absolute(tail);
    }
    if let Some(tail) = input.strip_prefix("\\\\") {
        return !tail.starts_with(['\\', '/']) && !tail.starts_with(".\\") && valid_unc_tail(tail);
    }
    valid_drive_absolute(input)
}

fn valid_unc_tail(tail: &str) -> bool {
    let mut components = tail.split(['\\', '/']);
    components.next().is_some_and(|server| !server.is_empty())
        && components.next().is_some_and(|share| !share.is_empty())
}

fn valid_drive_absolute(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && matches!(bytes.get(2), Some(b'\\' | b'/'))
}

fn invalid_reference(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::InvalidReference, message)
}

fn permission_denied(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::PermissionDenied, message)
}

fn limit_exceeded(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::LimitExceeded, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_root_id_is_rejected() {
        let id = WorkspaceRootId::new("same").expect("id");
        let first = WorkspaceRoot::new(id.clone(), "file:///one", None).expect("root");
        let second = WorkspaceRoot::new(id, "file:///two", None).expect("root");
        let error = WorkspaceRootSet::new(vec![first, second], None).expect_err("duplicate id");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }

    #[cfg(not(windows))]
    #[test]
    fn workspace_root_rejects_nonlocal_file_uri() {
        let error = WorkspaceRoot::new(
            WorkspaceRootId::new("remote").expect("id"),
            "file://example.com/workspace",
            None,
        )
        .expect_err("remote file URI must fail");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }
}
