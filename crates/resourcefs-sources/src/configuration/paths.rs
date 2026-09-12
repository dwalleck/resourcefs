use std::path::{Path, PathBuf};

use super::ConfigurationError;

/// The filesystem target kind an operator-configured local path must resolve to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationTargetKind {
    /// The canonical target must be a regular file.
    File,
    /// The canonical target must be a directory.
    Directory,
    /// The canonical target may be a regular file or a directory.
    FileOrDirectory,
}

/// Canonical base directory that owns every relative local source path.
///
/// Constructed once per profile load. Every lookup canonicalizes the
/// candidate, follows symlinks, and requires the canonical identity to remain
/// beneath this directory. Error messages are static and bounded so
/// diagnostics never disclose canonical absolute paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationDirectory {
    root: PathBuf,
}

impl ConfigurationDirectory {
    /// Canonicalizes one existing directory as the local configuration base.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, ConfigurationError> {
        // The reason is kept, not replaced. Saying only "must be an existing
        // directory" asserts a cause the errno can contradict: a directory that
        // exists but whose parent is not traversable fails here with
        // `PermissionDenied`, and an operator told it does not exist checks the
        // path, finds it, and is no further forward. `io::ErrorKind` is a
        // fieldless enum, so naming it adds no path and no credential.
        let root = std::fs::canonicalize(path.as_ref()).map_err(|error| {
            ConfigurationError::new(format!(
                "configuration directory could not be resolved ({:?}); it must be an existing directory the server can traverse",
                error.kind()
            ))
        })?;
        let root = normalize_platform_path(root);
        if !root.is_dir() {
            return Err(ConfigurationError::new(
                "configuration directory must be an existing directory",
            ));
        }
        Ok(Self { root })
    }

    /// The canonical base directory.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Resolves one operator-configured path to its canonical contained identity.
    ///
    /// Relative inputs resolve beneath the base directory; absolute inputs are
    /// admitted only when their canonical target remains beneath it. Symlinks
    /// are followed; missing, escaping, and wrong-kind targets are rejected.
    pub fn resolve(
        &self,
        input: &str,
        expected: ConfigurationTargetKind,
    ) -> Result<PathBuf, ConfigurationError> {
        if input.is_empty() {
            return Err(ConfigurationError::new(
                "configured path must contain at least one character",
            ));
        }
        let canonical = std::fs::canonicalize(self.root.join(input)).map_err(|error| {
            ConfigurationError::new(format!(
                "configured path could not be resolved ({:?}); it must name an existing filesystem entry the server can reach",
                error.kind()
            ))
        })?;
        let canonical = normalize_platform_path(canonical);
        if strip_beneath(&canonical, &self.root).is_none() {
            return Err(ConfigurationError::new(
                "configured path must remain beneath the configuration directory",
            ));
        }
        let metadata = std::fs::metadata(&canonical).map_err(|error| {
            ConfigurationError::new(format!(
                "configured path could not be read ({:?}); it must name a readable filesystem entry",
                error.kind()
            ))
        })?;
        let accepted = match expected {
            ConfigurationTargetKind::File => metadata.is_file(),
            ConfigurationTargetKind::Directory => metadata.is_dir(),
            ConfigurationTargetKind::FileOrDirectory => metadata.is_file() || metadata.is_dir(),
        };
        if !accepted {
            return Err(ConfigurationError::new(match expected {
                ConfigurationTargetKind::File => "configured path must be a file",
                ConfigurationTargetKind::Directory => "configured path must be a directory",
                ConfigurationTargetKind::FileOrDirectory => {
                    "configured path must be a file or directory"
                }
            }));
        }
        Ok(canonical)
    }
}

#[cfg(not(windows))]
pub(crate) fn strip_beneath(target: &Path, root: &Path) -> Option<PathBuf> {
    target.strip_prefix(root).ok().map(Path::to_owned)
}

#[cfg(windows)]
pub(crate) fn strip_beneath(target: &Path, root: &Path) -> Option<PathBuf> {
    let target_components = target.components().collect::<Vec<_>>();
    let root_components = root.components().collect::<Vec<_>>();
    if target_components.len() < root_components.len()
        || !target_components
            .iter()
            .zip(&root_components)
            .all(|(left, right)| {
                left.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
            })
    {
        return None;
    }
    let mut relative = PathBuf::new();
    for component in &target_components[root_components.len()..] {
        relative.push(component.as_os_str());
    }
    Some(relative)
}

#[cfg(not(windows))]
pub(crate) fn normalize_platform_path(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
pub(crate) fn normalize_platform_path(path: PathBuf) -> PathBuf {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };

    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let verbatim_unc = "\\\\?\\UNC\\".encode_utf16().collect::<Vec<_>>();
    let verbatim = "\\\\?\\".encode_utf16().collect::<Vec<_>>();
    let normalized = if wide.starts_with(&verbatim_unc) {
        let mut value = "\\\\".encode_utf16().collect::<Vec<_>>();
        value.extend_from_slice(&wide[verbatim_unc.len()..]);
        value
    } else if wide.starts_with(&verbatim) {
        wide[verbatim.len()..].to_vec()
    } else {
        wide
    };
    PathBuf::from(OsString::from_wide(&normalized))
}
