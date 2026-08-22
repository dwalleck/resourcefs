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
    #[schemars(length(max = 4096))]
    #[schemars(with = "Vec<SourceProfile>")]
    sources: Option<Vec<SourceProfile>>,
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

    /// Decodes one already-bounded profile value without constructing authority.
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
        let profile: Self = serde_json::from_slice(bytes).map_err(|error| {
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
        Ok(profile)
    }

    pub(crate) const fn schema_version(&self) -> u32 {
        self.schema_version
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
struct WorkspaceProfile {
    /// Launch roots; an absent list means no launch Workspace authority.
    #[serde(default, deserialize_with = "deserialize_optional_bounded_vec")]
    #[schemars(length(max = 4096))]
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
struct WorkspaceRootProfile {
    /// Stable Workspace Root ID.
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
enum SourceProfile {
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
struct HttpsSourceProfile {
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    origins: Vec<HttpsOriginProfile>,
}

/// One authorized HTTPS base URL prefix.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct HttpsOriginProfile {
    base_url: String,
    allow_private_network: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "CredentialHeaderProfile")]
    credential: Option<CredentialHeaderProfile>,
}

/// Credential header resolved at process startup.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CredentialHeaderProfile {
    header: String,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "String")]
    scheme: Option<String>,
    secret: SecretReferenceProfile,
}

/// GitHub repository source.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GithubSourceProfile {
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
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
struct GithubRepositoryProfile {
    name: String,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
}

/// Read-only SSH source.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SshSourceProfile {
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
    command: CommandProfile,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    hosts: Vec<SshHostProfile>,
}

/// One SSH config alias and its contained remote roots.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SshHostProfile {
    alias: String,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    remote_roots: Vec<String>,
}

/// Document converter source.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DocumentsSourceProfile {
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
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
struct SkillsSourceProfile {
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
struct RulesSourceProfile {
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
struct MemorySourceProfile {
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
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
struct VaultSourceProfile {
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
struct AgentExportSourceProfile {
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    manifests: Vec<String>,
}

/// Downstream MCP Resource servers.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DownstreamMcpSourceProfile {
    id: String,
    required: bool,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "MutationGrantsProfile")]
    grants: Option<MutationGrantsProfile>,
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    servers: Vec<DownstreamServerProfile>,
}

/// One downstream server and its native URI scheme claims.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DownstreamServerProfile {
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
enum DownstreamTransportProfile {
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
struct CommandProfile {
    #[serde(deserialize_with = "deserialize_bounded_vec")]
    #[schemars(length(max = 4096))]
    argv: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schemars(with = "BTreeMap<String, EnvironmentValueProfile>")]
    environment: Option<BTreeMap<String, EnvironmentValueProfile>>,
}

/// Explicit subprocess environment mapping.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "kind", deny_unknown_fields, rename_all = "camelCase")]
enum EnvironmentValueProfile {
    Literal { value: String },
    Inherit { name: String },
    Secret { secret: SecretReferenceProfile },
}

/// Opaque environment or helper-command secret reference.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "kind", deny_unknown_fields, rename_all = "camelCase")]
enum SecretReferenceProfile {
    Environment { name: String },
    Command { command: CommandProfile },
}
