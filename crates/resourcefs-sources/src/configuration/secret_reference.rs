use super::{CommandSpec, ConfigurationError, command::validate_environment_name};

/// A profile-declared secret lookup that carries no resolved credential value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretReference(SecretReferenceKind);

#[derive(Debug, Clone, PartialEq, Eq)]
enum SecretReferenceKind {
    Environment(String),
    Command(CommandSpec),
}

impl SecretReference {
    /// References one variable in the ResourceFS parent environment.
    pub fn environment(name: impl Into<String>) -> Result<Self, ConfigurationError> {
        let name = name.into();
        validate_environment_name(&name)?;
        Ok(Self(SecretReferenceKind::Environment(name)))
    }

    /// References one helper command whose own environment cannot contain secret references.
    pub fn command(command: CommandSpec) -> Result<Self, ConfigurationError> {
        if command.environment().contains_secret() {
            return Err(ConfigurationError::new(
                "secret helper environments cannot contain secret references",
            ));
        }
        Ok(Self(SecretReferenceKind::Command(command)))
    }
}
