use std::collections::{BTreeMap, HashSet};

use super::{
    ConfigurationError, MAX_COMMAND_ARGUMENT_BYTES, MAX_COMMAND_ARGUMENTS,
    MAX_COMMAND_ENVIRONMENT_ENTRIES, SecretReference,
};

/// One validated direct-argv subprocess definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    argv: Vec<String>,
    environment: ChildEnvironment,
}

impl CommandSpec {
    /// Validates a command without resolving or executing its program.
    pub fn new(
        argv: Vec<String>,
        environment: ChildEnvironment,
    ) -> Result<Self, ConfigurationError> {
        if argv.is_empty() || argv.len() > MAX_COMMAND_ARGUMENTS {
            return Err(ConfigurationError::new(format!(
                "command argv must contain 1–{MAX_COMMAND_ARGUMENTS} elements"
            )));
        }
        if argv[0].is_empty() {
            return Err(ConfigurationError::new(
                "command executable must not be empty",
            ));
        }
        let mut total_bytes = 0usize;
        for argument in &argv {
            if argument.contains('\0') {
                return Err(ConfigurationError::new(
                    "command argv elements must not contain NUL",
                ));
            }
            total_bytes = total_bytes
                .checked_add(argument.len())
                .filter(|total| *total <= MAX_COMMAND_ARGUMENT_BYTES)
                .ok_or_else(|| {
                    ConfigurationError::new(format!(
                        "command argv must contain at most {MAX_COMMAND_ARGUMENT_BYTES} bytes"
                    ))
                })?;
        }
        Ok(Self { argv, environment })
    }

    pub(super) fn argv(&self) -> &[String] {
        &self.argv
    }

    pub(super) const fn environment(&self) -> &ChildEnvironment {
        &self.environment
    }
}

/// Explicit environment entries for one cleared child environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChildEnvironment {
    entries: BTreeMap<String, EnvironmentValue>,
}

impl ChildEnvironment {
    /// Validates destination names without reading the parent environment.
    ///
    /// Case-fold collisions are rejected on every platform so one profile has
    /// the same child-environment meaning on case-insensitive Windows hosts.
    pub fn new(entries: BTreeMap<String, EnvironmentValue>) -> Result<Self, ConfigurationError> {
        if entries.len() > MAX_COMMAND_ENVIRONMENT_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "child environment must contain at most {MAX_COMMAND_ENVIRONMENT_ENTRIES} entries"
            )));
        }
        let mut folded_names = HashSet::with_capacity(entries.len());
        for (name, value) in &entries {
            validate_environment_name(name)?;
            if !folded_names.insert(name.to_uppercase()) {
                return Err(ConfigurationError::new(
                    "child environment names must be unique ignoring case",
                ));
            }
            value.validate()?;
        }
        Ok(Self { entries })
    }

    pub(super) fn contains_secret(&self) -> bool {
        self.entries
            .values()
            .any(|value| matches!(value, EnvironmentValue::Secret(_)))
    }
}

/// One explicit child-environment value source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentValue {
    Literal(String),
    Inherit(String),
    Secret(SecretReference),
}

impl EnvironmentValue {
    pub fn literal(value: impl Into<String>) -> Result<Self, ConfigurationError> {
        let value = value.into();
        if value.contains('\0') {
            return Err(ConfigurationError::new(
                "literal environment values must not contain NUL",
            ));
        }
        Ok(Self::Literal(value))
    }

    pub fn inherit(name: impl Into<String>) -> Result<Self, ConfigurationError> {
        let name = name.into();
        validate_environment_name(&name)?;
        Ok(Self::Inherit(name))
    }

    pub const fn secret(reference: SecretReference) -> Self {
        Self::Secret(reference)
    }

    fn validate(&self) -> Result<(), ConfigurationError> {
        match self {
            Self::Literal(value) => {
                if value.contains('\0') {
                    Err(ConfigurationError::new(
                        "literal environment values must not contain NUL",
                    ))
                } else {
                    Ok(())
                }
            }
            Self::Inherit(name) => validate_environment_name(name),
            Self::Secret(_) => Ok(()),
        }
    }
}

pub(super) fn validate_environment_name(name: &str) -> Result<(), ConfigurationError> {
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(ConfigurationError::new(
            "environment variable names must be non-empty and contain neither '=' nor NUL",
        ));
    }
    Ok(())
}
