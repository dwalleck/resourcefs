use std::collections::HashSet;

use super::{
    CommandSpec, ConfigurationError, MAX_CONFIGURATION_ENTRIES, MutationGrants, MutationSupport,
    validate_configuration_id,
};

/// Maximum encoded length of one converter extension claim.
pub const MAX_EXTENSION_BYTES: usize = 64;

/// How one converter receives the backing document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConverterInput {
    /// The document bytes are written to the child's standard input.
    Stdin,
    /// Exactly one contained canonical backing path is appended to argv.
    Path,
}

/// One extension-owned direct converter, validated by [`DocumentsConfig::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentConverter {
    extensions: Vec<String>,
    input: ConverterInput,
    command: CommandSpec,
}

impl DocumentConverter {
    /// Validates the claimed extensions, the input mode, and the command shape.
    ///
    /// No argv placeholder mechanism exists: `Path` mode always appends exactly
    /// one contained canonical backing path, so brace spellings such as
    /// `{path}` would otherwise be passed to the child literally.
    pub fn new(
        extensions: Vec<String>,
        input: ConverterInput,
        command: CommandSpec,
    ) -> Result<Self, ConfigurationError> {
        if extensions.is_empty() || extensions.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "converter extensions must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }
        for extension in &extensions {
            validate_extension(extension)?;
        }
        if command
            .argv()
            .iter()
            .any(|argument| argument.contains('{') || argument.contains('}'))
        {
            return Err(ConfigurationError::new(
                "converter command argv must not contain placeholder spellings",
            ));
        }
        Ok(Self {
            extensions,
            input,
            command,
        })
    }
}

/// Validated static authority for the read-only Documents Source Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentsConfig {
    id: String,
    required: bool,
    converters: Vec<DocumentConverter>,
}

impl DocumentsConfig {
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        converters: Vec<DocumentConverter>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::READ_ONLY.validate(grants)?;
        if converters.is_empty() || converters.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "document converters must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }

        let mut claimed = HashSet::new();
        for converter in &converters {
            for extension in &converter.extensions {
                if !claimed.insert(extension.as_str()) {
                    return Err(ConfigurationError::new(
                        "converter extensions must be globally unique",
                    ));
                }
            }
        }

        Ok(Self {
            id,
            required,
            converters,
        })
    }
}

/// Extension claims are 1–64 bytes of lowercase ASCII alphanumerics, `_`, `+`, or `-`.
///
/// The alphabet excludes dots, separators, and wildcards, so a claim can never
/// spell a leading dot, an internal dot, a path, or a glob.
fn validate_extension(extension: &str) -> Result<(), ConfigurationError> {
    let bytes = extension.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_EXTENSION_BYTES {
        return Err(ConfigurationError::new(format!(
            "converter extension must contain 1–{MAX_EXTENSION_BYTES} bytes"
        )));
    }
    if !bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'+' | b'-')
    }) {
        return Err(ConfigurationError::new(
            "converter extension may contain only lowercase ASCII letters, digits, '_', '+', or '-'",
        ));
    }
    Ok(())
}
