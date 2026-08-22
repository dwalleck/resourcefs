#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "strict DTO fields are consumed by serde and schemars before every adapter uses their values"
    )
)]

use crate::logging::{
    DEFAULT_RETAINED_FILES, LogConfig, LogError, LogErrorKind, LogLevel, MAX_ROTATION_BYTES,
};

use resourcefs_core::{
    DiscoveryLimitInput, ErrorCategory, ServerLimits, ServerLimitsInput, StorageLimitInput,
    TextLimitInput, WorkspaceRootId,
};
use resourcefs_sources::{
    BackingPathVisibility, ConfigurationDirectory, ConfigurationError, LaunchRoot,
    LaunchRootSource, MutationGrants, MutationSupport, SESSION_CLEANUP_TTL, SessionStorageConfig,
};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use std::{
    collections::BTreeMap,
    fmt,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Maximum accepted encoded Server Profile size.
pub const MAX_PROFILE_BYTES: usize = 1024 * 1024;
/// Maximum number of entries accepted by any profile-owned list.
pub const MAX_ALLOWLIST_ENTRIES: usize = 4_096;

/// Stable classification for failures at the Server Profile boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProfileErrorKind {
    /// The profile could not be read from its configured path.
    Io,
    /// The input exceeded a published hard ceiling.
    LimitExceeded,
    /// The JSON value does not conform to the closed profile shape.
    InvalidProfile,
    /// The profile names a schema version this binary does not implement.
    UnsupportedVersion,
}

/// A bounded, non-secret diagnostic produced while loading a Server Profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileError {
    kind: ProfileErrorKind,
    message: String,
}

impl ProfileError {
    fn new(kind: ProfileErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self::new(ProfileErrorKind::InvalidProfile, message)
    }

    /// Returns the stable failure classification.
    pub const fn kind(&self) -> ProfileErrorKind {
        self.kind
    }
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProfileError {}

/// The strict, versioned operator configuration accepted by ResourceFS.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[schemars(
    title = "ResourceFS Server Profile v1",
    description = "Strict operator configuration for one ResourceFS stdio server process."
)]
pub struct ProfileDocument {
    /// Profile schema version. Version 1 is the only supported value.
    #[schemars(schema_with = "schema_version_one")]
    schema_version: u32,
    /// Optional launch Workspace authority.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "WorkspaceProfile")]
    workspace: Option<WorkspaceProfile>,
    /// Optional lower-only runtime ceilings.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "LimitsProfile")]
    limits: Option<LimitsProfile>,
    /// Optional retained-session settings.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "SessionProfile")]
    session: Option<SessionProfile>,
    /// Optional bounded diagnostic destination.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "LoggingProfile")]
    logging: Option<LoggingProfile>,
    /// Optional catalog of configured Portable Sources.
    #[serde(default, deserialize_with = "deserialize_optional_bounded_vec")]
    #[schemars(length(max = 256))]
    #[schemars(with = "Vec<SourceProfile>")]
    sources: Option<Vec<SourceProfile>>,
    #[serde(skip)]
    #[schemars(skip)]
    configured_sources: Vec<super::convert::ConfiguredSource>,
    #[serde(skip)]
    #[schemars(skip)]
    static_sources: Vec<StaticSource>,
    #[serde(skip)]
    #[schemars(skip)]
    configuration_base: std::path::PathBuf,
    #[serde(skip)]
    #[schemars(skip)]
    server_limits: ServerLimits,
    #[serde(skip)]
    #[schemars(skip)]
    process_concurrency: usize,
    #[serde(skip)]
    #[schemars(skip)]
    session_storage_config: Option<SessionStorageConfig>,
    #[serde(skip)]
    #[schemars(skip)]
    logging_config: Option<LogConfig>,
}

pub(super) struct LaunchProfileComponents {
    pub(super) root_source: LaunchRootSource,
    pub(super) primary_selector: Option<String>,
    pub(super) visibility: BackingPathVisibility,
    pub(super) limits: ServerLimits,
    pub(super) session_storage: SessionStorageConfig,
    pub(super) logging: LogConfig,
}

impl ProfileDocument {
    /// Loads one profile with a hard `MAX_PROFILE_BYTES` admission ceiling.
    ///
    /// Relative local source paths resolve beneath the canonical profile
    /// directory, never the process working directory.
    pub fn load(path: &Path) -> Result<Self, ProfileError> {
        let canonical = path.canonicalize().map_err(profile_io_error)?;
        let file = File::open(&canonical).map_err(profile_io_error)?;
        let mut bytes = Vec::with_capacity(MAX_PROFILE_BYTES.saturating_add(1));
        file.take((MAX_PROFILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(profile_io_error)?;
        let parent = canonical.parent().ok_or_else(|| {
            ProfileError::new(
                ProfileErrorKind::Io,
                "could not determine the Server Profile directory",
            )
        })?;
        Self::from_slice_in(&bytes, parent)
    }

    /// Decodes and validates one already-bounded profile value, resolving
    /// local source paths beneath the current working directory.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ProfileError> {
        validate_profile_size(bytes)?;
        let base = std::env::current_dir().map_err(|error| {
            ProfileError::new(
                ProfileErrorKind::Io,
                format!("could not determine the current working directory: {error}"),
            )
        })?;
        let base = configuration_directory(&base)?;
        Self::decode(bytes, &base)
    }

    /// Decodes and validates one already-bounded profile value, resolving
    /// local source paths beneath the explicit base directory.
    pub fn from_slice_in(bytes: &[u8], base: &Path) -> Result<Self, ProfileError> {
        validate_profile_size(bytes)?;
        let base = configuration_directory(base)?;
        Self::decode(bytes, &base)
    }

    fn decode(bytes: &[u8], base: &ConfigurationDirectory) -> Result<Self, ProfileError> {
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let mut profile: Self =
            serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
                let path = error.path().to_string();
                let detail = error.inner();
                let message = if path.is_empty() {
                    format!("invalid Server Profile: {detail}")
                } else {
                    format!("invalid Server Profile field {path}: {detail}")
                };
                ProfileError::new(ProfileErrorKind::InvalidProfile, message)
            })?;
        deserializer.end().map_err(|error| {
            ProfileError::new(
                ProfileErrorKind::InvalidProfile,
                format!("invalid Server Profile: {error}"),
            )
        })?;
        if profile.schema_version != 1 {
            return Err(ProfileError::new(
                ProfileErrorKind::UnsupportedVersion,
                format!(
                    "unsupported Server Profile schemaVersion {}; expected 1",
                    profile.schema_version
                ),
            ));
        }
        let limits = profile.limits.unwrap_or_default();
        profile.server_limits = limits.server_limits()?;
        profile.process_concurrency = limits.validated_process_concurrency()?;
        profile.session_storage_config = Some(
            profile
                .session
                .take()
                .unwrap_or_default()
                .storage_config(base)?,
        );
        profile.logging_config = Some(profile.logging.take().unwrap_or_default().config(base)?);
        super::validate::validate_profile(&profile)?;
        profile.static_sources = profile
            .sources()
            .iter()
            .enumerate()
            .map(|(index, source)| StaticSource::from_profile(index, source))
            .collect();
        profile.configuration_base = base.path().to_owned();
        profile.configured_sources =
            super::convert::convert_sources(profile.sources.take().unwrap_or_default(), base)?;
        Ok(profile)
    }

    pub(crate) const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub(super) fn static_sources(&self) -> &[StaticSource] {
        &self.static_sources
    }

    pub(super) fn configuration_base(&self) -> &Path {
        &self.configuration_base
    }

    pub(super) const fn workspace(&self) -> Option<&WorkspaceProfile> {
        self.workspace.as_ref()
    }

    pub(super) fn sources(&self) -> &[SourceProfile] {
        self.sources.as_deref().unwrap_or_default()
    }
    pub const fn server_limits(&self) -> ServerLimits {
        self.server_limits
    }

    pub const fn process_concurrency(&self) -> usize {
        self.process_concurrency
    }

    pub fn session_storage_config(&self) -> &SessionStorageConfig {
        self.session_storage_config
            .as_ref()
            .expect("decoded profile contains validated session storage configuration")
    }

    pub(crate) fn logging_config(&self) -> &LogConfig {
        self.logging_config
            .as_ref()
            .expect("decoded profile contains validated logging configuration")
    }

    pub(super) fn into_launch_components(
        mut self,
    ) -> Result<LaunchProfileComponents, ProfileError> {
        let limits = self.server_limits;
        let session_storage = self.session_storage_config.take().ok_or_else(|| {
            ProfileError::invalid("decoded profile lost its session storage configuration")
        })?;
        let logging = self.logging_config.take().ok_or_else(|| {
            ProfileError::invalid("decoded profile lost its logging configuration")
        })?;
        let (roots, primary_selector, visibility) = match self.workspace.take() {
            Some(workspace) => {
                let roots = workspace
                    .roots
                    .unwrap_or_default()
                    .into_iter()
                    .map(|root| {
                        let configured = PathBuf::from(root.path);
                        if configured.as_os_str().is_empty() {
                            return Err(ProfileError::invalid(
                                "workspace root path must not be empty",
                            ));
                        }
                        let path = if configured.is_absolute() {
                            configured
                        } else {
                            self.configuration_base.join(configured)
                        };
                        let id = WorkspaceRootId::new(root.id).map_err(|error| {
                            ProfileError::invalid(format!("workspace root ID: {error}"))
                        })?;
                        Ok(LaunchRoot { id, path })
                    })
                    .collect::<Result<Vec<_>, ProfileError>>()?;
                (
                    roots,
                    workspace.primary_root,
                    workspace
                        .backing_path_visibility
                        .unwrap_or(BackingPathVisibilityProfile::Hidden)
                        .into(),
                )
            }
            None => (Vec::new(), None, BackingPathVisibility::Hidden),
        };
        Ok(LaunchProfileComponents {
            root_source: LaunchRootSource::Profile(roots),
            primary_selector,
            visibility,
            limits,
            session_storage,
            logging,
        })
    }
}

fn validate_profile_size(bytes: &[u8]) -> Result<(), ProfileError> {
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err(ProfileError::new(
            ProfileErrorKind::LimitExceeded,
            format!(
                "Server Profile is {} bytes; maximum is {MAX_PROFILE_BYTES}",
                bytes.len()
            ),
        ));
    }
    Ok(())
}

fn configuration_directory(base: &Path) -> Result<ConfigurationDirectory, ProfileError> {
    ConfigurationDirectory::new(base).map_err(|error| {
        ProfileError::new(
            ProfileErrorKind::Io,
            format!("could not establish the configuration base directory: {error}"),
        )
    })
}

fn profile_io_error(error: io::Error) -> ProfileError {
    ProfileError::new(
        ProfileErrorKind::Io,
        format!("could not read Server Profile: {error}"),
    )
}

fn schema_version_one(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = u32::json_schema(generator);
    schema.insert("const".to_owned(), serde_json::Value::from(1));
    schema
}

fn configuration_id_schema(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = String::json_schema(generator);
    schema.insert("minLength".to_owned(), serde_json::Value::from(1));
    schema.insert(
        "maxLength".to_owned(),
        serde_json::Value::from(resourcefs_sources::MAX_CONFIGURATION_ID_BYTES),
    );
    schema.insert(
        "pattern".to_owned(),
        serde_json::Value::from(r"^[A-Za-z0-9][A-Za-z0-9._-]*$"),
    );
    schema
}

fn command_environment_schema(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = BTreeMap::<String, EnvironmentValueProfile>::json_schema(generator);
    schema.insert(
        "maxProperties".to_owned(),
        serde_json::Value::from(resourcefs_sources::MAX_COMMAND_ENVIRONMENT_ENTRIES),
    );
    schema
}

fn read_only_grants_schema(_generator: &mut SchemaGenerator) -> Schema {
    mutation_grants_schema([false, false, false])
}

fn github_grants_schema(_generator: &mut SchemaGenerator) -> Schema {
    mutation_grants_schema([true, true, false])
}

fn mutation_grants_schema(supported: [bool; 3]) -> Schema {
    let mut properties = serde_json::Map::new();
    for ((name, description), supported) in [
        ("create", "Permit creation when true."),
        ("update", "Permit replacement and editing when true."),
        ("delete", "Permit deletion when true."),
    ]
    .into_iter()
    .zip(supported)
    {
        let mut property = serde_json::Map::new();
        property.insert(
            "description".to_owned(),
            serde_json::Value::String(description.to_owned()),
        );
        property.insert(
            "type".to_owned(),
            serde_json::Value::String("boolean".to_owned()),
        );
        if !supported {
            property.insert("const".to_owned(), serde_json::Value::Bool(false));
        }
        properties.insert(name.to_owned(), serde_json::Value::Object(property));
    }
    let mut schema = serde_json::Map::new();
    schema.insert(
        "additionalProperties".to_owned(),
        serde_json::Value::Bool(false),
    );
    schema.insert(
        "description".to_owned(),
        serde_json::Value::String(
            "Independent create, update, and delete grants supported by this source.".to_owned(),
        ),
    );
    schema.insert(
        "properties".to_owned(),
        serde_json::Value::Object(properties),
    );
    schema.insert(
        "type".to_owned(),
        serde_json::Value::String("object".to_owned()),
    );
    Schema::from(schema)
}

fn deserialize_optional_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)?
        .ok_or_else(|| D::Error::custom("null is not allowed"))
        .map(Some)
}

fn deserialize_bounded_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let values = Vec::<T>::deserialize(deserializer)?;
    if values.len() > MAX_ALLOWLIST_ENTRIES {
        return Err(D::Error::custom(format!(
            "list has {} entries; maximum is {MAX_ALLOWLIST_ENTRIES}",
            values.len()
        )));
    }
    Ok(values)
}

fn deserialize_optional_bounded_vec<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let values = Option::<Vec<T>>::deserialize(deserializer)?
        .ok_or_else(|| D::Error::custom("null is not allowed"))?;
    if values.len() > MAX_ALLOWLIST_ENTRIES {
        return Err(D::Error::custom(format!(
            "list has {} entries; maximum is {MAX_ALLOWLIST_ENTRIES}",
            values.len()
        )));
    }
    Ok(Some(values))
}

/// Workspace Roots supplied by the operator profile.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct WorkspaceProfile {
    /// Launch roots; an absent list means no launch Workspace authority.
    #[serde(default, deserialize_with = "deserialize_optional_bounded_vec")]
    #[schemars(length(max = 256))]
    #[schemars(with = "Vec<WorkspaceRootProfile>")]
    roots: Option<Vec<WorkspaceRootProfile>>,
    /// Stable ID of the Primary Workspace Root.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "String")]
    primary_root: Option<String>,
    /// Whether normal results may expose backing file URIs.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "BackingPathVisibilityProfile")]
    backing_path_visibility: Option<BackingPathVisibilityProfile>,
}

/// One operator-granted Workspace Root.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct WorkspaceRootProfile {
    /// Stable Workspace Root ID.
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    /// Directory path, resolved relative to the profile directory.
    path: String,
    /// Independent mutation grants.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
}

/// Backing-path disclosure policy.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum BackingPathVisibilityProfile {
    /// Keep backing filesystem paths private.
    Hidden,
    /// Include backing file URI metadata.
    Visible,
}

impl From<BackingPathVisibilityProfile> for BackingPathVisibility {
    fn from(visibility: BackingPathVisibilityProfile) -> Self {
        match visibility {
            BackingPathVisibilityProfile::Hidden => Self::Hidden,
            BackingPathVisibilityProfile::Visible => Self::Visible,
        }
    }
}

/// Independent create, update, and delete grants.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MutationGrantsProfile {
    /// Permit creation when true.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "bool")]
    create: Option<bool>,
    /// Permit replacement and editing when true.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "bool")]
    update: Option<bool>,
    /// Permit deletion when true.
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "bool")]
    delete: Option<bool>,
}

/// Lower-only server ceilings.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LimitsProfile {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "TextLimitsProfile")]
    text: Option<TextLimitsProfile>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "DiscoveryLimitsProfile")]
    discovery: Option<DiscoveryLimitsProfile>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 5_242_880))]
    image_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "StorageLimitsProfile")]
    storage: Option<StorageLimitsProfile>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 32))]
    process_concurrency: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TextLimitsProfile {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 49_152))]
    bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 3_000))]
    lines: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 512))]
    columns: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveryLimitsProfile {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 1_000))]
    search_matches: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 1_000))]
    glob_entries: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 1_000))]
    listing_entries: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StorageLimitsProfile {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 67_108_864))]
    object_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize", range(min = 1, max = 268_435_456))]
    session_bytes: Option<usize>,
}

impl LimitsProfile {
    fn server_limits(self) -> Result<ServerLimits, ProfileError> {
        let text = self.text.unwrap_or_default();
        let discovery = self.discovery.unwrap_or_default();
        let storage = self.storage.unwrap_or_default();
        ServerLimits::new(ServerLimitsInput {
            text: TextLimitInput {
                bytes: text.bytes,
                lines: text.lines,
                columns: text.columns,
            },
            discovery: DiscoveryLimitInput {
                search_matches: discovery.search_matches,
                glob_entries: discovery.glob_entries,
                listing_entries: discovery.listing_entries,
            },
            image_bytes: self.image_bytes,
            storage: StorageLimitInput {
                object_bytes: storage.object_bytes,
                session_bytes: storage.session_bytes,
            },
        })
        .map_err(|error| {
            ProfileError::new(
                ProfileErrorKind::LimitExceeded,
                format!("invalid limits: {}", error.message()),
            )
        })
    }

    fn validated_process_concurrency(self) -> Result<usize, ProfileError> {
        let value = self
            .process_concurrency
            .unwrap_or(resourcefs_sources::MAX_LIVE_COMMAND_TREES);
        if value == 0 || value > resourcefs_sources::MAX_LIVE_COMMAND_TREES {
            return Err(ProfileError::new(
                ProfileErrorKind::LimitExceeded,
                format!(
                    "invalid limits: processConcurrency must be between 1 and {}",
                    resourcefs_sources::MAX_LIVE_COMMAND_TREES
                ),
            ));
        }
        Ok(value)
    }
}

/// Retained Path Session configuration.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SessionProfile {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "String", length(min = 1))]
    cache_directory: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "u64", range(max = 86_400))]
    retention_ttl_seconds: Option<u64>,
}

impl SessionProfile {
    fn storage_config(
        self,
        base: &ConfigurationDirectory,
    ) -> Result<SessionStorageConfig, ProfileError> {
        let retention_ttl_seconds = self
            .retention_ttl_seconds
            .unwrap_or(SESSION_CLEANUP_TTL.as_secs());
        let retention_ttl_seconds =
            i64::try_from(retention_ttl_seconds).map_err(|_| session_ttl_profile_error())?;
        let config = match self.cache_directory {
            Some(cache_directory) => {
                if cache_directory.is_empty() {
                    return Err(ProfileError::new(
                        ProfileErrorKind::InvalidProfile,
                        "invalid session configuration: session.cacheDirectory must not be empty",
                    ));
                }
                let configured = std::path::PathBuf::from(cache_directory);
                let cache_root = if configured.is_absolute() {
                    configured
                } else {
                    base.path().join(configured)
                };
                SessionStorageConfig::new(cache_root, retention_ttl_seconds)
            }
            None => SessionStorageConfig::for_current_user(retention_ttl_seconds),
        };
        config.map_err(session_storage_profile_error)
    }
}

fn session_ttl_profile_error() -> ProfileError {
    ProfileError::new(
        ProfileErrorKind::LimitExceeded,
        format!(
            "invalid session configuration: session.retentionTtlSeconds must be between 0 and {}",
            SESSION_CLEANUP_TTL.as_secs()
        ),
    )
}

fn session_storage_profile_error(error: resourcefs_core::ResourceError) -> ProfileError {
    let kind = match error.category() {
        ErrorCategory::LimitExceeded => ProfileErrorKind::LimitExceeded,
        ErrorCategory::SourceUnavailable => ProfileErrorKind::Io,
        _ => ProfileErrorKind::InvalidProfile,
    };
    ProfileError::new(
        kind,
        format!("invalid session configuration: {}", error.message()),
    )
}

/// Bounded off-protocol logging configuration.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LoggingProfile {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "LogLevelProfile")]
    level: Option<LogLevelProfile>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "LogDestinationProfile")]
    destination: Option<LogDestinationProfile>,
}

impl LoggingProfile {
    fn config(self, base: &ConfigurationDirectory) -> Result<LogConfig, ProfileError> {
        let level = self.level.unwrap_or(LogLevelProfile::Info).into();
        let config = match self.destination.unwrap_or(LogDestinationProfile::Stderr) {
            LogDestinationProfile::Stderr => Ok(LogConfig::stderr(level)),
            LogDestinationProfile::File {
                path,
                rotation_bytes,
                retain_files,
            } => {
                if path.is_empty() {
                    return Err(ProfileError::new(
                        ProfileErrorKind::InvalidProfile,
                        "invalid logging configuration: logging.destination.path must not be empty",
                    ));
                }
                let configured = std::path::PathBuf::from(path);
                let path = if configured.is_absolute() {
                    configured
                } else {
                    base.path().join(configured)
                };
                let rotation_bytes = rotation_bytes.unwrap_or(MAX_ROTATION_BYTES as i64);
                let retain_files = retain_files.unwrap_or(DEFAULT_RETAINED_FILES as i64);
                LogConfig::file(level, path, rotation_bytes, retain_files)
            }
        };
        config.map_err(logging_profile_error)
    }
}

/// Diagnostic severity floor.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum LogLevelProfile {
    Error,
    Warn,
    Info,
    Debug,
}

impl From<LogLevelProfile> for LogLevel {
    fn from(level: LogLevelProfile) -> Self {
        match level {
            LogLevelProfile::Error => Self::Error,
            LogLevelProfile::Warn => Self::Warn,
            LogLevelProfile::Info => Self::Info,
            LogLevelProfile::Debug => Self::Debug,
        }
    }
}

/// Diagnostic output destination.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    deny_unknown_fields,
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum LogDestinationProfile {
    Stderr,
    File {
        #[schemars(length(min = 1))]
        path: String,
        #[serde(default, deserialize_with = "deserialize_optional_non_null")]
        #[schemars(with = "i64", range(min = 0, max = 10_485_760))]
        rotation_bytes: Option<i64>,
        #[serde(default, deserialize_with = "deserialize_optional_non_null")]
        #[schemars(with = "i64", range(min = 1, max = 10))]
        retain_files: Option<i64>,
    },
}

fn logging_profile_error(error: LogError) -> ProfileError {
    let kind = match error.kind() {
        LogErrorKind::InvalidConfig => ProfileErrorKind::InvalidProfile,
        LogErrorKind::LimitExceeded => ProfileErrorKind::LimitExceeded,
        LogErrorKind::Io => ProfileErrorKind::Io,
    };
    ProfileError::new(kind, format!("invalid logging configuration: {error}"))
}

/// One strict Portable Source configuration.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(super) enum SourceProfile {
    #[serde(rename = "https")]
    Https(HttpsSourceProfile),
    #[serde(rename = "github")]
    Github(GithubSourceProfile),
    #[serde(rename = "ssh")]
    Ssh(SshSourceProfile),
    #[serde(rename = "documents")]
    Documents(DocumentsSourceProfile),
    #[serde(rename = "skills")]
    Skills(SkillsSourceProfile),
    #[serde(rename = "rules")]
    Rules(RulesSourceProfile),
    #[serde(rename = "memory")]
    Memory(MemorySourceProfile),
    #[serde(rename = "vault")]
    Vault(VaultSourceProfile),
    #[serde(rename = "agentExport")]
    AgentExport(AgentExportSourceProfile),
    #[serde(rename = "downstreamMcp")]
    DownstreamMcp(DownstreamMcpSourceProfile),
}

/// HTTPS allowlist source.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct HttpsSourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(schema_with = "read_only_grants_schema")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    origins: Vec<HttpsOriginProfile>,
}

/// One authorized HTTPS base URL prefix.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct HttpsOriginProfile {
    base_url: String,
    allow_private_network: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "CredentialHeaderProfile")]
    credential: Option<CredentialHeaderProfile>,
}

/// Credential header resolved at process startup.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct CredentialHeaderProfile {
    header: String,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "String")]
    scheme: Option<String>,
    secret: SecretReferenceProfile,
}

/// GitHub repository source.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct GithubSourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(schema_with = "github_grants_schema")]
    grants: Option<MutationGrantsProfile>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "String")]
    api_base_url: Option<String>,
    allow_private_network: bool,
    credential: SecretReferenceProfile,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    repositories: Vec<GithubRepositoryProfile>,
}

/// One allowlisted GitHub repository.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct GithubRepositoryProfile {
    name: String,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(schema_with = "github_grants_schema")]
    grants: Option<MutationGrantsProfile>,
}

/// Read-only SSH source.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SshSourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(schema_with = "read_only_grants_schema")]
    grants: Option<MutationGrantsProfile>,
    command: CommandProfile,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    hosts: Vec<SshHostProfile>,
}

/// One SSH config alias and its contained remote roots.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SshHostProfile {
    alias: String,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    remote_roots: Vec<String>,
}

/// Document converter source.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct DocumentsSourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(schema_with = "read_only_grants_schema")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    converters: Vec<DocumentConverterProfile>,
}

/// One extension-owned direct converter.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct DocumentConverterProfile {
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    extensions: Vec<String>,
    input: ConverterInputProfile,
    command: CommandProfile,
}

/// Converter input delivery mode.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) enum ConverterInputProfile {
    Stdin,
    Path,
}

/// Agent Skills roots.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SkillsSourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    roots: Vec<String>,
}

/// Native rules manifests.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct RulesSourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    manifests: Vec<String>,
}

/// Named read-only Memory roots.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MemorySourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(schema_with = "read_only_grants_schema")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    roots: Vec<NamedPathProfile>,
}

/// A stable Resource name mapped to a contained path.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct NamedPathProfile {
    name: String,
    path: String,
}

/// Named Obsidian vaults.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct VaultSourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    vaults: Vec<VaultProfile>,
}

/// One named vault with optional narrower grants.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct VaultProfile {
    name: String,
    path: String,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
}

/// Imported public Agent Export manifests.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct AgentExportSourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(schema_with = "read_only_grants_schema")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    manifests: Vec<String>,
}

/// Downstream MCP Resource servers.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct DownstreamMcpSourceProfile {
    #[schemars(schema_with = "configuration_id_schema")]
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(schema_with = "read_only_grants_schema")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    servers: Vec<DownstreamServerProfile>,
}

/// One downstream server and its native URI scheme claims.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct DownstreamServerProfile {
    id: String,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    schemes: Vec<String>,
    transport: DownstreamTransportProfile,
}

/// Supported downstream MCP transport.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    deny_unknown_fields,
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(super) enum DownstreamTransportProfile {
    Stdio {
        command: CommandProfile,
    },
    Http {
        endpoint: String,
        allow_private_network: bool,
        #[serde(default, deserialize_with = "deserialize_optional_non_null")]
        #[schemars(with = "CredentialHeaderProfile")]
        credential: Option<CredentialHeaderProfile>,
    },
}

/// Strict direct-argv subprocess definition.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct CommandProfile {
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 256))]
    argv: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(schema_with = "command_environment_schema")]
    environment: Option<BTreeMap<String, EnvironmentValueProfile>>,
}

/// Explicit subprocess environment mapping.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "kind", deny_unknown_fields, rename_all = "camelCase")]
pub(super) enum EnvironmentValueProfile {
    Literal { value: String },
    Inherit { name: String },
    Secret { secret: SecretReferenceProfile },
}

/// Opaque environment or helper-command secret reference.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "kind", deny_unknown_fields, rename_all = "camelCase")]
pub(super) enum SecretReferenceProfile {
    Environment { name: String },
    Command { command: CommandProfile },
}

impl HttpsSourceProfile {
    pub(super) fn into_parts(self) -> (String, bool, MutationGrants, Vec<HttpsOriginProfile>) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.origins,
        )
    }
}

impl HttpsOriginProfile {
    pub(super) fn into_parts(self) -> (String, bool, Option<CredentialHeaderProfile>) {
        (self.base_url, self.allow_private_network, self.credential)
    }
}

impl CredentialHeaderProfile {
    pub(super) fn into_parts(self) -> (String, Option<String>, SecretReferenceProfile) {
        (self.header, self.scheme, self.secret)
    }
}

impl GithubSourceProfile {
    pub(super) fn into_parts(
        self,
    ) -> (
        String,
        bool,
        MutationGrants,
        Option<String>,
        bool,
        SecretReferenceProfile,
        Vec<GithubRepositoryProfile>,
    ) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.api_base_url,
            self.allow_private_network,
            self.credential,
            self.repositories,
        )
    }
}

impl GithubRepositoryProfile {
    pub(super) fn into_parts(self) -> (String, MutationGrants) {
        (self.name, grants_or_default(self.grants))
    }
}

impl SshSourceProfile {
    pub(super) fn into_parts(
        self,
    ) -> (
        String,
        bool,
        MutationGrants,
        CommandProfile,
        Vec<SshHostProfile>,
    ) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.command,
            self.hosts,
        )
    }
}

impl SshHostProfile {
    pub(super) fn into_parts(self) -> (String, Vec<String>) {
        (self.alias, self.remote_roots)
    }
}

impl DocumentsSourceProfile {
    pub(super) fn into_parts(
        self,
    ) -> (String, bool, MutationGrants, Vec<DocumentConverterProfile>) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.converters,
        )
    }
}

impl DocumentConverterProfile {
    pub(super) fn into_parts(self) -> (Vec<String>, ConverterInputProfile, CommandProfile) {
        (self.extensions, self.input, self.command)
    }
}

impl SkillsSourceProfile {
    pub(super) fn into_parts(self) -> (String, bool, MutationGrants, Vec<String>) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.roots,
        )
    }
}

impl RulesSourceProfile {
    pub(super) fn into_parts(self) -> (String, bool, MutationGrants, Vec<String>) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.manifests,
        )
    }
}

impl MemorySourceProfile {
    pub(super) fn into_parts(self) -> (String, bool, MutationGrants, Vec<NamedPathProfile>) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.roots,
        )
    }
}

impl NamedPathProfile {
    pub(super) fn into_parts(self) -> (String, String) {
        (self.name, self.path)
    }
}

impl VaultSourceProfile {
    pub(super) fn into_parts(self) -> (String, bool, MutationGrants, Vec<VaultProfile>) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.vaults,
        )
    }
}

impl VaultProfile {
    pub(super) fn into_parts(self) -> (String, String, MutationGrants) {
        (self.name, self.path, grants_or_default(self.grants))
    }
}

impl AgentExportSourceProfile {
    pub(super) fn into_parts(self) -> (String, bool, MutationGrants, Vec<String>) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.manifests,
        )
    }
}

impl DownstreamMcpSourceProfile {
    pub(super) fn into_parts(self) -> (String, bool, MutationGrants, Vec<DownstreamServerProfile>) {
        (
            self.id,
            self.required,
            grants_or_default(self.grants),
            self.servers,
        )
    }
}

impl DownstreamServerProfile {
    pub(super) fn into_parts(self) -> (String, Vec<String>, DownstreamTransportProfile) {
        (self.id, self.schemes, self.transport)
    }
}

impl CommandProfile {
    pub(super) fn into_parts(self) -> (Vec<String>, BTreeMap<String, EnvironmentValueProfile>) {
        (self.argv, self.environment.unwrap_or_default())
    }
}

impl WorkspaceProfile {
    pub(super) fn roots(&self) -> &[WorkspaceRootProfile] {
        self.roots.as_deref().unwrap_or_default()
    }
}

impl WorkspaceRootProfile {
    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn grants(&self) -> MutationGrants {
        grants_or_default(self.grants)
    }
}

impl SourceProfile {
    pub(super) const fn kind(&self) -> &'static str {
        match self {
            Self::Https(_) => "https",
            Self::Github(_) => "github",
            Self::Ssh(_) => "ssh",
            Self::Documents(_) => "documents",
            Self::Skills(_) => "skills",
            Self::Rules(_) => "rules",
            Self::Memory(_) => "memory",
            Self::Vault(_) => "vault",
            Self::AgentExport(_) => "agentExport",
            Self::DownstreamMcp(_) => "downstreamMcp",
        }
    }

    pub(super) fn id(&self) -> &str {
        match self {
            Self::Https(source) => &source.id,
            Self::Github(source) => &source.id,
            Self::Ssh(source) => &source.id,
            Self::Documents(source) => &source.id,
            Self::Skills(source) => &source.id,
            Self::Rules(source) => &source.id,
            Self::Memory(source) => &source.id,
            Self::Vault(source) => &source.id,
            Self::AgentExport(source) => &source.id,
            Self::DownstreamMcp(source) => &source.id,
        }
    }

    pub(super) const fn required(&self) -> bool {
        match self {
            Self::Https(source) => source.required,
            Self::Github(source) => source.required,
            Self::Ssh(source) => source.required,
            Self::Documents(source) => source.required,
            Self::Skills(source) => source.required,
            Self::Rules(source) => source.required,
            Self::Memory(source) => source.required,
            Self::Vault(source) => source.required,
            Self::AgentExport(source) => source.required,
            Self::DownstreamMcp(source) => source.required,
        }
    }

    pub(super) fn grants(&self) -> MutationGrants {
        let grants = match self {
            Self::Https(source) => source.grants,
            Self::Github(source) => source.grants,
            Self::Ssh(source) => source.grants,
            Self::Documents(source) => source.grants,
            Self::Skills(source) => source.grants,
            Self::Rules(source) => source.grants,
            Self::Memory(source) => source.grants,
            Self::Vault(source) => source.grants,
            Self::AgentExport(source) => source.grants,
            Self::DownstreamMcp(source) => source.grants,
        };
        grants_or_default(grants)
    }

    pub(super) const fn mutation_support(&self) -> MutationSupport {
        match self {
            Self::Github(_) => MutationSupport::GITHUB,
            Self::Skills(_) | Self::Rules(_) | Self::Vault(_) => MutationSupport::FULL,
            Self::Https(_)
            | Self::Ssh(_)
            | Self::Documents(_)
            | Self::Memory(_)
            | Self::AgentExport(_)
            | Self::DownstreamMcp(_) => MutationSupport::READ_ONLY,
        }
    }

    pub(super) fn validate_grants(&self) -> Result<(), ConfigurationError> {
        let support = self.mutation_support();
        let parent = self.grants();
        support.validate(parent)?;
        match self {
            Self::Github(source) => {
                for repository in &source.repositories {
                    support.validate_nested(parent, grants_or_default(repository.grants))?;
                }
            }
            Self::Vault(source) => {
                for vault in &source.vaults {
                    support.validate_nested(parent, grants_or_default(vault.grants))?;
                }
            }
            Self::Https(_)
            | Self::Ssh(_)
            | Self::Documents(_)
            | Self::Skills(_)
            | Self::Rules(_)
            | Self::Memory(_)
            | Self::AgentExport(_)
            | Self::DownstreamMcp(_) => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) struct StaticSource {
    pub(super) id: String,
    pub(super) kind: &'static str,
    pub(super) required: bool,
    pub(super) requirements: Vec<StaticRequirement>,
    pub(super) probe: StaticProbe,
}

#[derive(Debug, Clone)]
pub(super) enum StaticProbe {
    ValidatedLocal,
    Network(Vec<String>),
    Unsupported,
}

#[derive(Debug, Clone)]
pub(super) enum StaticRequirement {
    Secret(StaticSecret),
    Command(StaticCommand),
}

#[derive(Debug, Clone)]
pub(super) struct StaticSecret {
    pub(super) field: String,
    pub(super) kind: StaticSecretKind,
}

#[derive(Debug, Clone)]
pub(super) enum StaticSecretKind {
    Environment(String),
    Command(StaticCommand),
}

#[derive(Debug, Clone)]
pub(super) struct StaticCommand {
    pub(super) field: String,
    pub(super) argv: Vec<String>,
    pub(super) environment: Vec<StaticEnvironment>,
}

#[derive(Debug, Clone)]
pub(super) struct StaticEnvironment {
    pub(super) field: String,
    pub(super) destination: String,
    pub(super) value: StaticEnvironmentValue,
}

#[derive(Debug, Clone)]
pub(super) enum StaticEnvironmentValue {
    Literal(String),
    Inherit(String),
    Secret(StaticSecret),
}

impl StaticSource {
    fn from_profile(index: usize, source: &SourceProfile) -> Self {
        let base = format!("sources[{index}]");
        let mut requirements = Vec::new();
        match source {
            SourceProfile::Https(source) => {
                for (origin_index, origin) in source.origins.iter().enumerate() {
                    if let Some(credential) = &origin.credential {
                        requirements.push(StaticRequirement::Secret(StaticSecret::from_profile(
                            format!("{base}.origins[{origin_index}].credential.secret"),
                            &credential.secret,
                        )));
                    }
                }
            }
            SourceProfile::Github(source) => {
                requirements.push(StaticRequirement::Secret(StaticSecret::from_profile(
                    format!("{base}.credential"),
                    &source.credential,
                )));
            }
            SourceProfile::Ssh(source) => {
                requirements.push(StaticRequirement::Command(StaticCommand::from_profile(
                    format!("{base}.command"),
                    &source.command,
                )));
            }
            SourceProfile::Documents(source) => {
                requirements.extend(source.converters.iter().enumerate().map(
                    |(converter_index, converter)| {
                        StaticRequirement::Command(StaticCommand::from_profile(
                            format!("{base}.converters[{converter_index}].command"),
                            &converter.command,
                        ))
                    },
                ));
            }
            SourceProfile::DownstreamMcp(source) => {
                for (server_index, server) in source.servers.iter().enumerate() {
                    match &server.transport {
                        DownstreamTransportProfile::Stdio { command } => {
                            requirements.push(StaticRequirement::Command(
                                StaticCommand::from_profile(
                                    format!("{base}.servers[{server_index}].transport.command"),
                                    command,
                                ),
                            ));
                        }
                        DownstreamTransportProfile::Http { credential, .. } => {
                            if let Some(credential) = credential {
                                requirements.push(StaticRequirement::Secret(
                                    StaticSecret::from_profile(
                                        format!(
                                            "{base}.servers[{server_index}].transport.credential.secret"
                                        ),
                                        &credential.secret,
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
            SourceProfile::Skills(_)
            | SourceProfile::Rules(_)
            | SourceProfile::Memory(_)
            | SourceProfile::Vault(_)
            | SourceProfile::AgentExport(_) => {}
        }
        let probe = match source {
            SourceProfile::Https(source) => StaticProbe::Network(
                source
                    .origins
                    .iter()
                    .map(|origin| origin.base_url.clone())
                    .collect(),
            ),
            SourceProfile::Github(source) => StaticProbe::Network(vec![
                source
                    .api_base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.github.com".to_owned()),
            ]),
            SourceProfile::Documents(_)
            | SourceProfile::Skills(_)
            | SourceProfile::Rules(_)
            | SourceProfile::Memory(_)
            | SourceProfile::Vault(_)
            | SourceProfile::AgentExport(_) => StaticProbe::ValidatedLocal,
            SourceProfile::Ssh(_) | SourceProfile::DownstreamMcp(_) => StaticProbe::Unsupported,
        };
        Self {
            id: source.id().to_owned(),
            kind: source.kind(),
            required: source.required(),
            requirements,
            probe,
        }
    }
}

impl StaticSecret {
    fn from_profile(field: String, secret: &SecretReferenceProfile) -> Self {
        let kind = match secret {
            SecretReferenceProfile::Environment { name } => {
                StaticSecretKind::Environment(name.clone())
            }
            SecretReferenceProfile::Command { command } => StaticSecretKind::Command(
                StaticCommand::from_profile(format!("{field}.command"), command),
            ),
        };
        Self { field, kind }
    }
}

impl StaticCommand {
    fn from_profile(field: String, command: &CommandProfile) -> Self {
        let environment = command
            .environment
            .as_ref()
            .into_iter()
            .flat_map(BTreeMap::iter)
            .enumerate()
            .map(|(index, (destination, value))| {
                let environment_field = format!("{field}.environment[{index}]");
                let value = match value {
                    EnvironmentValueProfile::Literal { value } => {
                        StaticEnvironmentValue::Literal(value.clone())
                    }
                    EnvironmentValueProfile::Inherit { name } => {
                        StaticEnvironmentValue::Inherit(name.clone())
                    }
                    EnvironmentValueProfile::Secret { secret } => StaticEnvironmentValue::Secret(
                        StaticSecret::from_profile(format!("{environment_field}.secret"), secret),
                    ),
                };
                StaticEnvironment {
                    field: environment_field,
                    destination: destination.clone(),
                    value,
                }
            })
            .collect();
        Self {
            field,
            argv: command.argv.clone(),
            environment,
        }
    }
}

fn grants_or_default(grants: Option<MutationGrantsProfile>) -> MutationGrants {
    grants.map_or_else(MutationGrants::default, |grants| {
        MutationGrants::new(
            grants.create.unwrap_or(false),
            grants.update.unwrap_or(false),
            grants.delete.unwrap_or(false),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::ProfileDocument;
    use crate::logging::{LogDestinationKind, LogLevel};

    #[test]
    fn logging_paths_resolve_from_the_profile_directory() {
        let fixture = tempfile::tempdir().expect("profile directory");
        let default = ProfileDocument::from_slice_in(br#"{"schemaVersion":1}"#, fixture.path())
            .expect("default logging profile");
        assert_eq!(default.logging_config().level(), LogLevel::Info);
        assert_eq!(
            default.logging_config().destination_kind(),
            LogDestinationKind::Stderr
        );

        let relative = ProfileDocument::from_slice_in(
            br#"{"schemaVersion":1,"logging":{"level":"debug","destination":{"kind":"file","path":"logs/resourcefs.log","rotationBytes":0,"retainFiles":10}}}"#,
            fixture.path(),
        )
        .expect("relative logging profile");
        let relative_config = relative.logging_config();
        assert_eq!(relative_config.level(), LogLevel::Debug);
        assert_eq!(relative_config.destination_kind(), LogDestinationKind::File);
        assert_eq!(
            relative_config.path(),
            Some(fixture.path().join("logs/resourcefs.log").as_path())
        );
        assert_eq!(relative_config.rotation_bytes(), Some(0));
        assert_eq!(relative_config.retained_files(), Some(10));

        let absolute_path = fixture.path().join("absolute.log");
        let encoded = serde_json::to_vec(&serde_json::json!({
            "schemaVersion":1,
            "logging":{"destination":{"kind":"file","path":absolute_path}}
        }))
        .expect("absolute profile bytes");
        let absolute = ProfileDocument::from_slice_in(&encoded, fixture.path())
            .expect("absolute logging profile");
        assert_eq!(
            absolute.logging_config().path(),
            Some(absolute_path.as_path())
        );
    }
}
