use std::{
    fmt,
    path::{Component, Path, PathBuf},
};

use crate::{ErrorCategory, ResourceError};

const WORKSPACE_PREFIX: &str = "rfs://workspace/";

/// Validated name of a configured Workspace Root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RootName(String);

impl RootName {
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
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "Workspace Root name must start with an ASCII letter or digit and contain only ASCII letters, digits, '.', '_', or '-'",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RootName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A contained workspace Path Reference in the founding grammar subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathReference {
    root: RootName,
    relative_path: PathBuf,
    canonical: String,
}

impl PathReference {
    pub fn parse(input: &str, primary_root: &RootName) -> Result<Self, ResourceError> {
        if input.is_empty() {
            return Err(invalid_reference("Path Reference must not be empty"));
        }

        let relative = if let Some(workspace_path) = input.strip_prefix(WORKSPACE_PREFIX) {
            let (root, path) = workspace_path.split_once('/').ok_or_else(|| {
                invalid_reference("canonical workspace reference must include a root and path")
            })?;
            let root = RootName::new(root)?;
            if &root != primary_root {
                return Err(invalid_reference(format!(
                    "Workspace Root '{root}' is not the configured Primary Workspace Root"
                )));
            }
            path
        } else {
            if input.contains("://") {
                return Err(invalid_reference("unsupported Path Reference scheme"));
            }
            input
        };

        if looks_like_windows_path(relative) || Path::new(relative).is_absolute() {
            return Err(invalid_reference(
                "absolute filesystem references are not supported by this server slice",
            ));
        }

        let mut normalized = PathBuf::new();
        let mut canonical = String::with_capacity(
            WORKSPACE_PREFIX.len() + primary_root.as_str().len() + relative.len() + 1,
        );
        canonical.push_str(WORKSPACE_PREFIX);
        canonical.push_str(primary_root.as_str());
        canonical.push('/');
        let mut has_component = false;
        for component in Path::new(relative).components() {
            match component {
                Component::Normal(value) => {
                    let value = value
                        .to_str()
                        .ok_or_else(|| invalid_reference("Path Reference must be valid UTF-8"))?;
                    if has_component {
                        canonical.push('/');
                    }
                    normalized.push(value);
                    canonical.push_str(value);
                    has_component = true;
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(ResourceError::new(
                        ErrorCategory::PermissionDenied,
                        "Path Reference escapes the Primary Workspace Root",
                    ));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(invalid_reference(
                        "absolute filesystem references are not supported by this server slice",
                    ));
                }
            }
        }

        if !has_component {
            return Err(invalid_reference(
                "Path Reference must address a Resource below the Workspace Root",
            ));
        }
        Ok(Self {
            root: primary_root.clone(),
            relative_path: normalized,
            canonical,
        })
    }

    pub fn root(&self) -> &RootName {
        &self.root
    }

    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub fn canonical(&self) -> &str {
        &self.canonical
    }
}

fn looks_like_windows_path(input: &str) -> bool {
    let bytes = input.as_bytes();
    input.starts_with("\\\\")
        || bytes.get(1) == Some(&b':') && bytes.first().is_some_and(u8::is_ascii_alphabetic)
}

fn invalid_reference(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::InvalidReference, message)
}
