#![expect(
    dead_code,
    reason = "strict DTO fields are consumed by serde and schemars before every adapter uses their values"
)]

use resourcefs_sources::{ConfigurationError, MutationGrants, MutationSupport};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use std::{
    collections::BTreeMap,
    fmt,
    fs::File,
    io::{self, Read},
    path::Path,
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
}

impl ProfileDocument {
    /// Loads one profile with a hard `MAX_PROFILE_BYTES` admission ceiling.
    pub fn load(path: &Path) -> Result<Self, ProfileError> {
        let canonical = path.canonicalize().map_err(profile_io_error)?;
        let file = File::open(canonical).map_err(profile_io_error)?;
        let mut bytes = Vec::with_capacity(MAX_PROFILE_BYTES.saturating_add(1));
        file.take((MAX_PROFILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(profile_io_error)?;
        Self::from_slice(&bytes)
    }

    /// Decodes and validates one already-bounded profile value.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ProfileError> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(ProfileError::new(
                ProfileErrorKind::LimitExceeded,
                format!(
                    "Server Profile is {} bytes; maximum is {MAX_PROFILE_BYTES}",
                    bytes.len()
                ),
            ));
        }
        let mut profile: Self = serde_json::from_slice(bytes).map_err(|error| {
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
        super::validate::validate_profile(&profile)?;
        profile.configured_sources =
            super::convert::convert_sources(profile.sources.take().unwrap_or_default())?;
        Ok(profile)
    }

    pub(crate) const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub(super) const fn workspace(&self) -> Option<&WorkspaceProfile> {
        self.workspace.as_ref()
    }

    pub(super) fn sources(&self) -> &[SourceProfile] {
        self.sources.as_deref().unwrap_or_default()
    }
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
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LimitsProfile {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize")]
    text_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize")]
    text_lines: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize")]
    text_columns: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize")]
    image_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize")]
    object_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize")]
    session_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize")]
    listing_entries: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "usize")]
    process_concurrency: Option<usize>,
}

/// Retained Path Session configuration.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SessionProfile {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "String")]
    cache_directory: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "u64")]
    retention_ttl_seconds: Option<u64>,
}

/// Bounded off-protocol logging configuration.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LoggingProfile {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "LogLevelProfile")]
    level: Option<LogLevelProfile>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "LogDestinationProfile")]
    destination: Option<LogDestinationProfile>,
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
        path: String,
        #[serde(default, deserialize_with = "deserialize_optional_non_null")]
        #[schemars(with = "usize")]
        rotation_bytes: Option<usize>,
        #[serde(default, deserialize_with = "deserialize_optional_non_null")]
        #[schemars(with = "usize")]
        retain_files: Option<usize>,
    },
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
struct DocumentConverterProfile {
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    extensions: Vec<String>,
    input: ConverterInputProfile,
    command: CommandProfile,
}

/// Converter input delivery mode.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ConverterInputProfile {
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
struct NamedPathProfile {
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
struct VaultProfile {
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

fn grants_or_default(grants: Option<MutationGrantsProfile>) -> MutationGrants {
    grants.map_or_else(MutationGrants::default, |grants| {
        MutationGrants::new(
            grants.create.unwrap_or(false),
            grants.update.unwrap_or(false),
            grants.delete.unwrap_or(false),
        )
    })
}
