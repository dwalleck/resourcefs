use std::{
    collections::HashSet,
    fmt::{self, Write as _},
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};
use url::Url;

use crate::{ErrorCategory, ResourceError};

const WORKSPACE_PREFIX: &str = "rfs://workspace/";
pub(crate) const SOURCE_CATALOG_REFERENCE: &str = "rfs://";
pub(crate) const WORKSPACE_CATALOG_REFERENCE: &str = "rfs://workspace";
pub(crate) const ARTIFACT_PREFIX: &str = "artifact://";
pub(crate) const LOCAL_PREFIX: &str = "local://";
pub(crate) const HTTPS_PREFIX: &str = "https://";
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

    pub fn root() -> Self {
        Self(PathBuf::new())
    }

    pub fn is_root(&self) -> bool {
        self.0.as_os_str().is_empty()
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

/// Immutable Path Session artifact identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtifactAddress {
    session_token: String,
    object_id: u64,
}

impl ArtifactAddress {
    pub fn new(session_token: impl Into<String>, object_id: u64) -> Result<Self, ResourceError> {
        let session_token = session_token.into();
        validate_session_token(&session_token)?;
        if object_id == 0 {
            return Err(invalid_reference("artifact object ID must be positive"));
        }
        Ok(Self {
            session_token,
            object_id,
        })
    }

    pub fn session_token(&self) -> &str {
        &self.session_token
    }

    pub const fn object_id(&self) -> u64 {
        self.object_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogAddress {
    Sources,
    Workspace,
}

impl CatalogAddress {
    pub(crate) const fn canonical_reference(self) -> &'static str {
        match self {
            Self::Sources => SOURCE_CATALOG_REFERENCE,
            Self::Workspace => WORKSPACE_CATALOG_REFERENCE,
        }
    }
}

/// Maximum length of a Session Scratch name, in UTF-8 bytes after percent-decoding.
pub const MAX_LOCAL_NAME_BYTES: usize = 255;

/// Validated flat Session Scratch name.
///
/// Session Scratch has no hierarchy: a name is one filename-like segment, so a
/// scratch reference cannot address anything outside its Path Session by
/// construction. The stored value is the percent-decoded name; rendering a
/// canonical reference re-encodes it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalName(String);

impl LocalName {
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        if value.is_empty() {
            return Err(invalid_reference("Session Scratch name must not be empty"));
        }
        if value.len() > MAX_LOCAL_NAME_BYTES {
            return Err(invalid_reference(format!(
                "Session Scratch name must not exceed {MAX_LOCAL_NAME_BYTES} UTF-8 bytes"
            )));
        }
        if value == "." || value == ".." {
            return Err(invalid_reference(
                "Session Scratch name must not be '.' or '..'",
            ));
        }
        if value.contains(['/', '\\']) {
            return Err(invalid_reference(
                "Session Scratch names are flat and must not contain a path separator",
            ));
        }
        if value.chars().any(char::is_control) {
            return Err(invalid_reference(
                "Session Scratch name must not contain control characters",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Renders the canonical `local://<name>` reference for this name.
    pub(crate) fn canonical_reference(&self) -> String {
        let mut rendered = String::with_capacity(LOCAL_PREFIX.len() + self.0.len());
        rendered.push_str(LOCAL_PREFIX);
        encode_component(&self.0, &mut rendered);
        rendered
    }
}

impl fmt::Display for LocalName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Session Scratch identity: the family root or one named scratch Resource.
///
/// The bare `local://` root is the scratch family's self-describing entry point,
/// mirroring `rfs://` for mounted sources and `rfs://workspace` for Workspace
/// Roots. It addresses a synthetic read-only listing, never a stored Resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalAddress {
    Root,
    Named(LocalName),
}

impl LocalAddress {
    /// Renders the canonical reference for this scratch identity.
    pub(crate) fn canonical_reference(&self) -> String {
        match self {
            Self::Root => LOCAL_PREFIX.to_owned(),
            Self::Named(name) => name.canonical_reference(),
        }
    }

    /// Returns the scratch name, or `None` for the family root.
    pub const fn name(&self) -> Option<&LocalName> {
        match self {
            Self::Root => None,
            Self::Named(name) => Some(name),
        }
    }
}

/// An allowlistable HTTPS document identity.
///
/// Wraps a `url::Url` that is known to carry the `https` scheme and a host.
/// Parsing here establishes *syntax* only: whether the origin is reachable is a
/// Server Profile allowlist decision made by the HTTPS Source Adapter, and
/// whether a resolved address may be connected to is an address-policy decision
/// made inside the resolver. Neither belongs in the grammar.
///
/// Percent-encoded separators are preserved verbatim: `%2F` in a URL path or
/// query is meaningful to the origin, so the filesystem containment guard that
/// rejects encoded separators deliberately does not apply to this family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsAddress(Url);

impl HttpsAddress {
    /// Parses an `https://` reference, rejecting any other scheme and any URL
    /// without a host.
    fn parse(input: &str) -> Result<Self, ResourceError> {
        let url = Url::parse(input)
            .map_err(|_| invalid_reference("HTTPS reference must be a well-formed URL"))?;
        if url.scheme() != "https" {
            return Err(invalid_reference(
                "HTTPS reference must use the https scheme",
            ));
        }
        if !url.has_host() {
            return Err(invalid_reference("HTTPS reference must name a host"));
        }
        // Credentials reach an origin from the Server Profile, never from the
        // reference. Admitting userinfo would carry secret material into
        // canonical references, catalog entries, error text, and logs — the
        // exact channels the source's leak-freedom contract closes.
        if !url.username().is_empty() || url.password().is_some() {
            return Err(invalid_reference(
                "HTTPS reference must not embed credentials",
            ));
        }
        Ok(Self(url))
    }

    /// Returns the canonical serialization of this URL.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns the parsed URL.
    pub const fn url(&self) -> &Url {
        &self.0
    }
}

/// Parsed source identity independent of its optional projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceAddress {
    Catalog(CatalogAddress),
    Workspace(WorkspaceAddress),
    Artifact(ArtifactAddress),
    Local(LocalAddress),
    Https(HttpsAddress),
}

/// A Session Scratch name paired with the projection selector that follows it.
///
/// Scratch names are filename-like, so a colon is a legal name character and
/// `local://plan.md:1-5` is first a candidate *name*. This records the competing
/// selector reading, which the Local Source Adapter uses only when no Resource
/// carries the literal name — the same literal-wins rule the Workspace family
/// applies through [`SelectedWorkspaceAddress`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedLocalAddress {
    base: LocalName,
    selector: ProjectionSelector,
}

impl SelectedLocalAddress {
    pub const fn base(&self) -> &LocalName {
        &self.base
    }

    pub const fn selector(&self) -> &ProjectionSelector {
        &self.selector
    }
}

/// One validated 1-indexed line range in request order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    start: u64,
    inclusive_end: Option<u64>,
}

impl LineRange {
    const fn through_eof(start: u64) -> Self {
        Self {
            start,
            inclusive_end: None,
        }
    }

    const fn bounded(start: u64, inclusive_end: u64) -> Self {
        Self {
            start,
            inclusive_end: Some(inclusive_end),
        }
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn inclusive_end(self) -> Option<u64> {
        self.inclusive_end
    }
}

/// Ordered line-selection algebra. Duplicate and overlapping ranges are retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSelector {
    raw: bool,
    ranges: Vec<LineRange>,
}

impl LineSelector {
    pub const fn is_raw(&self) -> bool {
        self.raw
    }

    pub fn ranges(&self) -> &[LineRange] {
        &self.ranges
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProjectionKind {
    Raw,
    Lines(LineSelector),
    Page(u64),
}

/// Typed projection plus the caller's accepted spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionSelector {
    spelling: String,
    kind: ProjectionKind,
}

impl ProjectionSelector {
    pub fn parse(spelling: impl Into<String>) -> Result<Self, ResourceError> {
        let spelling = spelling.into();
        if spelling == "raw" {
            return Ok(Self {
                spelling,
                kind: ProjectionKind::Raw,
            });
        }
        if let Some(offset) = spelling.strip_prefix("page:") {
            let offset = positive_integer(offset)
                .filter(|offset| *offset != 0)
                .ok_or_else(|| invalid_reference("artifact page offset must be positive"))?;
            return Ok(Self {
                spelling,
                kind: ProjectionKind::Page(offset),
            });
        }
        let (raw, ranges) = spelling
            .strip_prefix("raw:")
            .map_or((false, spelling.as_str()), |ranges| (true, ranges));
        if ranges.is_empty() {
            return Err(invalid_reference("line selector must include a range"));
        }
        let ranges = ranges
            .split(',')
            .map(parse_line_range)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            spelling,
            kind: ProjectionKind::Lines(LineSelector { raw, ranges }),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.spelling
    }

    pub const fn is_raw(&self) -> bool {
        match &self.kind {
            ProjectionKind::Raw => true,
            ProjectionKind::Lines(selection) => selection.is_raw(),
            ProjectionKind::Page(_) => false,
        }
    }

    pub const fn line_selection(&self) -> Option<&LineSelector> {
        match &self.kind {
            ProjectionKind::Lines(selection) => Some(selection),
            ProjectionKind::Raw | ProjectionKind::Page(_) => None,
        }
    }

    pub const fn page_offset(&self) -> Option<u64> {
        match &self.kind {
            ProjectionKind::Page(offset) => Some(*offset),
            ProjectionKind::Raw | ProjectionKind::Lines(_) => None,
        }
    }
}

/// Alternate workspace interpretation when a trailing selector is syntactically valid.
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
    address: ResourceAddress,
    projection: Option<ProjectionSelector>,
    selector_candidate: Option<SelectedWorkspaceAddress>,
    selector_error: Option<ResourceError>,
    local_candidate: Option<SelectedLocalAddress>,
}

impl PathReference {
    pub fn parse(input: impl Into<String>) -> Result<Self, ResourceError> {
        let mut requested = input.into();
        validate_reference_input(&requested)?;
        let catalog = match requested.as_str() {
            SOURCE_CATALOG_REFERENCE => Some(CatalogAddress::Sources),
            WORKSPACE_CATALOG_REFERENCE | "rfs://workspace/" => Some(CatalogAddress::Workspace),
            _ => None,
        };
        if let Some(address) = catalog {
            requested.clear();
            requested.push_str(address.canonical_reference());
            return Ok(Self {
                requested,
                address: ResourceAddress::Catalog(address),
                projection: None,
                selector_candidate: None,
                selector_error: None,
                local_candidate: None,
            });
        }
        if requested.starts_with(ARTIFACT_PREFIX) {
            let (base, projection) = projection_candidate_split(&requested).map_or_else(
                || Ok((requested.as_str(), None)),
                |(base, selector)| {
                    ProjectionSelector::parse(selector).map(|selector| (base, Some(selector)))
                },
            )?;
            let address = parse_artifact_address(base)?;
            return Ok(Self {
                requested,
                address: ResourceAddress::Artifact(address),
                projection,
                selector_candidate: None,
                selector_error: None,
                local_candidate: None,
            });
        }

        // Parsed ahead of the workspace fall-through, whose generic `contains("://")`
        // arm would otherwise reject every URL as an unsupported scheme.
        if requested.starts_with(HTTPS_PREFIX) {
            let address = HttpsAddress::parse(&requested)?;
            return Ok(Self {
                requested: address.as_str().to_owned(),
                address: ResourceAddress::Https(address),
                projection: None,
                selector_candidate: None,
                selector_error: None,
                local_candidate: None,
            });
        }

        if let Some(raw_name) = requested.strip_prefix(LOCAL_PREFIX) {
            // Filesystem-backed family: escapes must be validated before
            // `percent_decode`, whose contract assumes prior validation.
            validate_percent_encoding(&requested)?;
            if raw_name.is_empty() {
                return Ok(Self {
                    requested: LOCAL_PREFIX.to_owned(),
                    address: ResourceAddress::Local(LocalAddress::Root),
                    projection: None,
                    selector_candidate: None,
                    selector_error: None,
                    local_candidate: None,
                });
            }
            let decoded = percent_decode(raw_name)?;
            let name = LocalName::new(decoded.clone())?;
            // A colon is a legal scratch-name character, so the literal name is
            // the primary reading and the selector split is only a candidate.
            let local_candidate =
                projection_candidate_split(&decoded).and_then(|(base, selector)| {
                    let base = LocalName::new(base).ok()?;
                    let selector = ProjectionSelector::parse(selector).ok()?;
                    Some(SelectedLocalAddress { base, selector })
                });
            return Ok(Self {
                requested: name.canonical_reference(),
                address: ResourceAddress::Local(LocalAddress::Named(name)),
                projection: None,
                selector_candidate: None,
                selector_error: None,
                local_candidate,
            });
        }

        let literal = parse_workspace_address(&requested)?;
        let (selector_candidate, selector_error) =
            projection_candidate_split(&requested).map_or((None, None), |(base, selector)| {
                match ProjectionSelector::parse(selector).and_then(|selector| {
                    parse_workspace_address(base)
                        .map(|base| SelectedWorkspaceAddress { base, selector })
                }) {
                    Ok(candidate) => (Some(candidate), None),
                    Err(error) => (None, Some(error)),
                }
            });
        if matches!(
            &literal,
            WorkspaceAddress::Canonical { path, .. } if path.is_root()
        ) && !requested.ends_with('/')
        {
            requested.push('/');
        }
        Ok(Self {
            requested,
            address: ResourceAddress::Workspace(literal),
            projection: None,
            selector_candidate,
            selector_error,
            local_candidate: None,
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
            address: ResourceAddress::Workspace(WorkspaceAddress::Canonical { root, path }),
            projection: None,
            selector_candidate: None,
            selector_error: None,
            local_candidate: None,
        }
    }

    /// Builds the canonical reference for one Session Scratch Resource.
    pub fn local(name: impl Into<String>) -> Result<Self, ResourceError> {
        let name = LocalName::new(name)?;
        Ok(Self {
            requested: name.canonical_reference(),
            address: ResourceAddress::Local(LocalAddress::Named(name)),
            projection: None,
            selector_candidate: None,
            selector_error: None,
            local_candidate: None,
        })
    }

    /// Builds the canonical reference for the Session Scratch family root.
    pub fn local_root() -> Self {
        Self {
            requested: LOCAL_PREFIX.to_owned(),
            address: ResourceAddress::Local(LocalAddress::Root),
            projection: None,
            selector_candidate: None,
            selector_error: None,
            local_candidate: None,
        }
    }

    pub fn artifact(
        address: ArtifactAddress,
        projection: Option<ProjectionSelector>,
    ) -> Result<Self, ResourceError> {
        let mut requested = format!(
            "{ARTIFACT_PREFIX}{}-{}",
            address.session_token(),
            address.object_id()
        );
        if let Some(selector) = projection.as_ref() {
            requested.push(':');
            requested.push_str(selector.as_str());
        }
        Self::parse(requested)
    }

    pub fn requested(&self) -> &str {
        &self.requested
    }

    pub const fn address(&self) -> &ResourceAddress {
        &self.address
    }

    pub fn workspace_address(&self) -> Option<&WorkspaceAddress> {
        match &self.address {
            ResourceAddress::Workspace(address) => Some(address),
            ResourceAddress::Catalog(_)
            | ResourceAddress::Artifact(_)
            | ResourceAddress::Local(_)
            | ResourceAddress::Https(_) => None,
        }
    }

    pub const fn projection(&self) -> Option<&ProjectionSelector> {
        self.projection.as_ref()
    }

    pub fn selector_candidate(&self) -> Option<&SelectedWorkspaceAddress> {
        self.selector_candidate.as_ref()
    }

    pub const fn selector_error(&self) -> Option<&ResourceError> {
        self.selector_error.as_ref()
    }

    /// The competing selector reading of a Session Scratch reference.
    ///
    /// Present only when the literal scratch name also splits into a valid
    /// name plus projection selector. The Local Source Adapter consults it only
    /// after the literal name reports `not_found`.
    pub const fn local_selector_candidate(&self) -> Option<&SelectedLocalAddress> {
        self.local_candidate.as_ref()
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
            return Ok(Self {
                roots,
                primary: None,
            });
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
    Ok(())
}

fn parse_workspace_address(input: &str) -> Result<WorkspaceAddress, ResourceError> {
    // Filesystem containment: an encoded separator must never survive into a
    // path component, and every downstream `percent_decode` assumes escapes were
    // validated here. Non-filesystem families (`https://`, `artifact://`,
    // catalogs) never reach this function and own their own syntax.
    validate_percent_encoding(input)?;
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
        let (root, path) = match workspace_path.split_once('/') {
            Some((root, "")) => (root, WorkspacePath::root()),
            Some((root, path)) => (root, parse_workspace_path(path)?),
            None => (workspace_path, WorkspacePath::root()),
        };
        let root = WorkspaceRootId::new(root.to_owned())?;
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

fn parse_artifact_address(input: &str) -> Result<ArtifactAddress, ResourceError> {
    let body = input
        .strip_prefix(ARTIFACT_PREFIX)
        .ok_or_else(|| invalid_reference("malformed artifact reference"))?;
    if body.len() < 34 {
        return Err(invalid_reference("malformed artifact reference"));
    }
    let session_bytes = &body.as_bytes()[..32];
    if !session_bytes
        .iter()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(invalid_reference(
            "artifact session token must be 128-bit lowercase hexadecimal",
        ));
    }
    let session_token =
        std::str::from_utf8(session_bytes).expect("validated ASCII session token must be UTF-8");
    let object = body
        .get(32..)
        .ok_or_else(|| invalid_reference("malformed artifact reference"))?;
    validate_session_token(session_token)?;
    let object = object
        .strip_prefix('-')
        .ok_or_else(|| invalid_reference("artifact reference must separate token and object ID"))?;
    if object.len() > 1 && object.starts_with('0') {
        return Err(invalid_reference(
            "artifact object ID must use canonical decimal",
        ));
    }
    let object_id = positive_integer(object)
        .ok_or_else(|| invalid_reference("artifact object ID must be positive"))?;
    ArtifactAddress::new(session_token, object_id)
}

fn validate_session_token(token: &str) -> Result<(), ResourceError> {
    if token.len() != 32
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(invalid_reference(
            "artifact session token must be 128-bit lowercase hexadecimal",
        ));
    }
    Ok(())
}

fn projection_candidate_split(input: &str) -> Option<(&str, &str)> {
    for marker in [":raw", ":page:"] {
        if let Some(index) = input.rfind(marker) {
            return Some((&input[..index], &input[index + 1..]));
        }
    }
    let (base, selector) = input.rsplit_once(':')?;
    selector
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_digit)
        .then_some((base, selector))
}

fn parse_line_range(range: &str) -> Result<LineRange, ResourceError> {
    if let Some((start, count)) = range.split_once('+') {
        let start = positive_integer(start)
            .ok_or_else(|| invalid_reference("line selector start must be positive"))?;
        let count = positive_integer(count)
            .ok_or_else(|| invalid_reference("line selector count must be positive"))?;
        let inclusive_end = start
            .checked_add(count - 1)
            .ok_or_else(|| invalid_reference("line selector range overflows"))?;
        return Ok(LineRange::bounded(start, inclusive_end));
    }
    if let Some((start, end)) = range.split_once('-') {
        let start = positive_integer(start)
            .ok_or_else(|| invalid_reference("line selector start must be positive"))?;
        if end.is_empty() {
            return Ok(LineRange::through_eof(start));
        }
        let end = positive_integer(end)
            .ok_or_else(|| invalid_reference("line selector end must be positive"))?;
        if end < start {
            return Err(invalid_reference(
                "line selector end must not precede its start",
            ));
        }
        return Ok(LineRange::bounded(start, end));
    }
    let start = positive_integer(range)
        .ok_or_else(|| invalid_reference("line selector start must be positive"))?;
    Ok(LineRange::through_eof(start))
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
