use std::{
    ffi::OsString,
    fmt::Write as _,
    io::Write as _,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use cap_std::fs::{Dir, File, OpenOptions, Permissions};
use resourcefs_core::{
    ErrorCategory, MAX_ARTIFACT_BYTES, MutationAccess, MutationAdapter, MutationSourceKey,
    MutationState, MutationTarget, OperationGuard, PathReference, ResourceAddress, ResourceError,
    SourceMutation, VersionTag, WorkspacePath, select_utf8,
};

use super::*;

struct OpenMutationParent {
    directory: Dir,
    name: OsString,
    canonical_path: PathBuf,
}

struct ExistingText {
    content: String,
    version_tag: VersionTag,
    permissions: Permissions,
    _file: File,
}

#[async_trait]
impl MutationAdapter for FilesystemSource {
    async fn resolve(
        &self,
        reference: &PathReference,
        access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError> {
        let authority = self.mutation_authority_guard().await?;
        let resolved = resolve_reference(active_view(&authority), reference)?;
        enforce_grant(resolved.root.grants, access)?;
        let canonical = canonical_target(&resolved)?;
        open_mutation_parent(&resolved.root, &resolved.path, canonical.requested())?;
        MutationTarget::new(
            canonical,
            MutationSourceKey::new(resolved.root.metadata.id().as_str())?,
        )
    }

    async fn load(
        &self,
        target: &MutationTarget,
        access: MutationAccess,
        operation: &OperationGuard,
    ) -> Result<MutationState, ResourceError> {
        ensure_operation_active(operation)?;
        let authority = self.mutation_authority_guard().await?;
        let resolved = resolve_target(active_view(&authority), target)?;
        enforce_grant(resolved.root.grants, access)?;
        let parent = open_mutation_parent(
            &resolved.root,
            &resolved.path,
            target.canonical_reference().requested(),
        )?;
        load_current(&parent, target.canonical_reference().requested()).map(|current| {
            current.map_or(MutationState::Missing, |current| MutationState::Text {
                content: current.content,
                version_tag: current.version_tag,
            })
        })
    }

    async fn commit(
        &self,
        mutation: SourceMutation,
        operation: &OperationGuard,
    ) -> Result<(), ResourceError> {
        if !operation.is_committing() {
            return Err(ResourceError::new(
                ErrorCategory::Cancelled,
                "filesystem mutation commit requires a committing operation guard",
            ));
        }
        let authority = self.mutation_authority_guard().await?;
        match mutation {
            SourceMutation::Create { target, content } => {
                commit_create(active_view(&authority), target, content)
            }
            SourceMutation::Replace {
                target,
                expected,
                content,
            } => commit_replace(active_view(&authority), target, expected, content),
            SourceMutation::Delete { .. } | SourceMutation::Move { .. } => Err(ResourceError::new(
                ErrorCategory::UnsupportedMutation,
                "filesystem delete and move are not enabled in this increment",
            )),
        }
    }
}

fn active_view(authority: &MutationAuthorityGuard) -> &WorkspaceView {
    match &*authority._guard {
        AuthorityState::Active { view, .. } => view,
        AuthorityState::Refreshing { .. } | AuthorityState::Disabled { .. } => {
            unreachable!("mutation authority guard holds an active state")
        }
    }
}

fn resolve_reference(
    view: &WorkspaceView,
    reference: &PathReference,
) -> Result<ResolvedAddress, ResourceError> {
    let ResourceAddress::Workspace(address) = reference.address() else {
        return Err(ResourceError::new(
            ErrorCategory::PermissionDenied,
            "only workspace Resources can use filesystem mutation",
        ));
    };
    let resolved = resolve_address(view, address)?;
    if resolved.path.is_root() {
        return Err(ResourceError::new(
            ErrorCategory::UnsupportedMutation,
            "Workspace Root directories are not mutable text Resources",
        ));
    }
    Ok(resolved)
}

fn resolve_target(
    view: &WorkspaceView,
    target: &MutationTarget,
) -> Result<ResolvedAddress, ResourceError> {
    let resolved = resolve_reference(view, target.canonical_reference())?;
    if resolved.root.metadata.id().as_str() != target.source_key().as_str() {
        return Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "mutation target source identity changed",
        ));
    }
    Ok(resolved)
}

fn canonical_target(resolved: &ResolvedAddress) -> Result<PathReference, ResourceError> {
    Ok(PathReference::canonical(
        resolved.root.metadata.id().clone(),
        resolved.path.clone(),
    ))
}

fn enforce_grant(grants: MutationGrants, access: MutationAccess) -> Result<(), ResourceError> {
    let granted = match access {
        MutationAccess::Create => grants.create(),
        MutationAccess::Update => grants.update(),
        MutationAccess::Delete => grants.delete(),
    };
    if granted {
        Ok(())
    } else {
        Err(ResourceError::new(
            ErrorCategory::PermissionDenied,
            format!(
                "workspace {} grant is false",
                match access {
                    MutationAccess::Create => "create",
                    MutationAccess::Update => "update",
                    MutationAccess::Delete => "delete",
                }
            ),
        ))
    }
}

fn ensure_operation_active(operation: &OperationGuard) -> Result<(), ResourceError> {
    if operation.is_active() {
        Ok(())
    } else {
        Err(ResourceError::new(
            ErrorCategory::Cancelled,
            "filesystem mutation was cancelled before commit",
        ))
    }
}

fn open_mutation_parent(
    root: &FilesystemRoot,
    path: &WorkspacePath,
    identity: &str,
) -> Result<OpenMutationParent, ResourceError> {
    let relative = path.as_path();
    let name = relative.file_name().ok_or_else(|| {
        ResourceError::new(
            ErrorCategory::UnsupportedMutation,
            "Workspace Root directories are not mutable text Resources",
        )
    })?;
    let parent_path = relative.parent().unwrap_or_else(|| Path::new(""));
    let parent_workspace = if parent_path.as_os_str().is_empty() {
        WorkspacePath::root()
    } else {
        WorkspacePath::new(parent_path)?
    };
    let directory = if parent_workspace.is_root() {
        root.directory
            .try_clone()
            .map_err(|error| resource_io_error(identity, "open mutation parent", error))?
    } else {
        open_capability_directory(root, &parent_workspace, identity)?
    };
    let final_parent = normalize_handle_path(
        final_directory_path(&directory)
            .map_err(|error| resource_io_error(identity, "resolve mutation parent", error))?,
    )?;
    let expected_parent = normalize_handle_path(root.canonical_path.join(parent_path))?;
    if final_parent != expected_parent {
        return Err(ResourceError::new(
            ErrorCategory::PermissionDenied,
            "workspace mutation paths must not traverse links or reparse points",
        ));
    }
    match directory.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "workspace mutation targets must not be links or reparse points",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(resource_io_error(
                identity,
                "inspect mutation target",
                error,
            ));
        }
    }
    Ok(OpenMutationParent {
        directory,
        name: name.to_owned(),
        canonical_path: final_parent,
    })
}

fn load_current(
    parent: &OpenMutationParent,
    identity: &str,
) -> Result<Option<ExistingText>, ResourceError> {
    let mut file = match parent.directory.open(&parent.name) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(resource_io_error(identity, "open mutation target", error)),
    };
    let metadata = file
        .metadata()
        .map_err(|error| resource_io_error(identity, "inspect mutation target", error))?;
    if !metadata.is_file() {
        return Err(ResourceError::new(
            ErrorCategory::UnsupportedMutation,
            "workspace mutation target is not a regular text file",
        ));
    }
    let permissions = metadata.permissions();
    let selected = select_utf8(&mut file, None).map_err(|error| {
        if error.category() == ErrorCategory::UnsupportedProjection {
            ResourceError::new(ErrorCategory::UnsupportedMutation, error.message())
        } else {
            error
        }
    })?;
    let (content, version_tag, _) = selected.into_parts();
    Ok(Some(ExistingText {
        content,
        version_tag,
        permissions,
        _file: file,
    }))
}

fn commit_create(
    view: &WorkspaceView,
    target: MutationTarget,
    content: String,
) -> Result<(), ResourceError> {
    let resolved = resolve_target(view, &target)?;
    enforce_grant(resolved.root.grants, MutationAccess::Create)?;
    let parent = open_mutation_parent(
        &resolved.root,
        &resolved.path,
        target.canonical_reference().requested(),
    )?;
    if load_current(&parent, target.canonical_reference().requested())?.is_some() {
        return Err(ResourceError::new(
            ErrorCategory::VersionConflict,
            "create destination already exists",
        ));
    }
    let (temporary_name, temporary) = write_temporary(
        &parent,
        &content,
        None,
        target.canonical_reference().requested(),
    )?;
    commit_temporary(
        &parent,
        &temporary_name,
        &parent.name,
        &temporary,
        false,
        target.canonical_reference().requested(),
    )
}

fn commit_replace(
    view: &WorkspaceView,
    target: MutationTarget,
    expected: VersionTag,
    content: String,
) -> Result<(), ResourceError> {
    let resolved = resolve_target(view, &target)?;
    enforce_grant(resolved.root.grants, MutationAccess::Update)?;
    let parent = open_mutation_parent(
        &resolved.root,
        &resolved.path,
        target.canonical_reference().requested(),
    )?;
    let current =
        load_current(&parent, target.canonical_reference().requested())?.ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::VersionConflict,
                "replacement target no longer exists",
            )
        })?;
    if current.version_tag != expected {
        return Err(ResourceError::new(
            ErrorCategory::VersionConflict,
            "replacement Version Tag no longer matches authoritative content",
        ));
    }
    let (temporary_name, temporary) = write_temporary(
        &parent,
        &content,
        Some(&current),
        target.canonical_reference().requested(),
    )?;
    commit_temporary(
        &parent,
        &temporary_name,
        &parent.name,
        &temporary,
        true,
        target.canonical_reference().requested(),
    )
}

fn write_temporary(
    parent: &OpenMutationParent,
    content: &str,
    current: Option<&ExistingText>,
    identity: &str,
) -> Result<(OsString, File), ResourceError> {
    if content.len() > MAX_ARTIFACT_BYTES {
        return Err(ResourceError::new(
            ErrorCategory::LimitExceeded,
            format!("mutation content exceeds the {MAX_ARTIFACT_BYTES}-byte ceiling"),
        ));
    }
    for _ in 0..32 {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("failed to generate mutation temporary name: {error}"),
            )
        })?;
        let mut rendered_name = String::with_capacity(45);
        rendered_name.push_str(".resourcefs-");
        for byte in random {
            write!(&mut rendered_name, "{byte:02x}")
                .expect("writing hexadecimal bytes to String cannot fail");
        }
        rendered_name.push_str(".tmp");
        let name = OsString::from(rendered_name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = match parent.directory.open_with(&name, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(resource_io_error(identity, "create temporary file", error)),
        };
        if let Err(error) = file
            .write_all(content.as_bytes())
            .and_then(|()| file.flush())
        {
            return cleanup_error(parent, &name, identity, "write temporary file", error);
        }
        if let Some(current) = current {
            preserve_permissions(parent, &name, &file, current, identity)?;
        }
        return Ok((name, file));
    }
    Err(ResourceError::new(
        ErrorCategory::SourceUnavailable,
        "could not allocate a unique mutation temporary name",
    ))
}

fn preserve_permissions(
    parent: &OpenMutationParent,
    temporary: &OsString,
    _temporary_file: &File,
    current: &ExistingText,
    identity: &str,
) -> Result<(), ResourceError> {
    if let Err(error) = parent
        .directory
        .set_permissions(temporary, current.permissions.clone())
    {
        return cleanup_error(
            parent,
            temporary,
            identity,
            "preserve file permissions",
            error,
        );
    }
    #[cfg(windows)]
    if let Err(error) = copy_windows_dacl(&current._file, _temporary_file) {
        return cleanup_error(parent, temporary, identity, "preserve Windows DACL", error);
    }
    Ok(())
}

fn cleanup_error<T>(
    parent: &OpenMutationParent,
    temporary: &OsString,
    identity: &str,
    operation: &str,
    error: io::Error,
) -> Result<T, ResourceError> {
    let failure = if operation == "commit mutation" {
        mutation_commit_error(identity, &error)
    } else {
        resource_io_error(identity, operation, error)
    };
    match parent.directory.remove_file(temporary) {
        Ok(()) => Err(failure),
        Err(cleanup) => Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            format!(
                "{failure}; temporary cleanup also failed for Resource '{identity}': {cleanup}"
            ),
        )),
    }
}

fn mutation_commit_error(identity: &str, error: &io::Error) -> ResourceError {
    let category = match error.kind() {
        io::ErrorKind::AlreadyExists => ErrorCategory::VersionConflict,
        io::ErrorKind::CrossesDevices => ErrorCategory::UnsupportedMutation,
        _ => ErrorCategory::SourceUnavailable,
    };
    ResourceError::new(
        category,
        format!("Resource '{identity}' failed atomic commit: {error}"),
    )
}

fn commit_temporary(
    parent: &OpenMutationParent,
    temporary_name: &OsString,
    destination: &OsString,
    temporary: &File,
    replace: bool,
    identity: &str,
) -> Result<(), ResourceError> {
    let result = platform_rename(
        &parent.directory,
        temporary_name,
        destination,
        &parent.canonical_path.join(destination),
        temporary,
        replace,
    );
    match result {
        Ok(()) => Ok(()),
        Err(error) => cleanup_error(parent, temporary_name, identity, "commit mutation", error),
    }
}

#[cfg(unix)]
fn platform_rename(
    parent: &Dir,
    source: &OsString,
    destination: &OsString,
    _destination_path: &Path,
    _source_file: &File,
    replace: bool,
) -> io::Result<()> {
    if replace {
        rustix::fs::renameat(parent, source, parent, destination).map_err(io::Error::from)
    } else {
        rustix::fs::renameat_with(
            parent,
            source,
            parent,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(io::Error::from)
    }
}

#[cfg(windows)]
fn platform_rename(
    _parent: &Dir,
    _source: &OsString,
    _destination: &OsString,
    destination_path: &Path,
    source_file: &File,
    replace: bool,
) -> io::Result<()> {
    use std::{
        mem::offset_of,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
        ptr,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileRenameInfoEx,
        ReOpenFile, SetFileInformationByHandle,
    };

    const DELETE_ACCESS: u32 = 0x0001_0000;
    const FILE_RENAME_REPLACE_IF_EXISTS: u32 = 0x0000_0001;
    const FILE_RENAME_POSIX_SEMANTICS: u32 = 0x0000_0002;
    let reopened = unsafe {
        ReOpenFile(
            source_file.as_raw_handle(),
            DELETE_ACCESS,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            0,
        )
    };
    if reopened == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        return Err(io::Error::new(
            error.kind(),
            format!("ReOpenFile failed: {error}"),
        ));
    }
    let reopened = unsafe { OwnedHandle::from_raw_handle(reopened) };
    let destination = destination_path
        .as_os_str()
        .encode_wide()
        .collect::<Vec<_>>();
    let bytes = offset_of!(FILE_RENAME_INFO, FileName)
        .checked_add(destination.len() * std::mem::size_of::<u16>())
        .ok_or_else(|| io::Error::other("rename buffer overflow"))?;
    let words = bytes.div_ceil(std::mem::size_of::<usize>());
    let mut storage = vec![0_usize; words];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.Flags = FILE_RENAME_POSIX_SEMANTICS
            | if replace {
                FILE_RENAME_REPLACE_IF_EXISTS
            } else {
                0
            };
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = u32::try_from(destination.len() * std::mem::size_of::<u16>())
            .map_err(|_| io::Error::other("destination is too long"))?;
        ptr::copy_nonoverlapping(
            destination.as_ptr(),
            (*info).FileName.as_mut_ptr(),
            destination.len(),
        );
    }
    let renamed = unsafe {
        SetFileInformationByHandle(
            reopened.as_raw_handle(),
            FileRenameInfoEx,
            info.cast(),
            u32::try_from(bytes).map_err(|_| io::Error::other("rename buffer is too large"))?,
        )
    };
    if renamed == 0 {
        let error = io::Error::last_os_error();
        Err(io::Error::new(
            error.kind(),
            format!("FileRenameInfoEx failed: {error}"),
        ))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn copy_windows_dacl(source: &File, destination: &File) -> io::Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Security::{DACL_SECURITY_INFORMATION, GetKernelObjectSecurity, SetKernelObjectSecurity},
        Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, ReOpenFile},
    };

    const READ_CONTROL_ACCESS: u32 = 0x0002_0000;
    const WRITE_DAC_ACCESS: u32 = 0x0004_0000;
    let destination_security = unsafe {
        ReOpenFile(
            destination.as_raw_handle(),
            READ_CONTROL_ACCESS | WRITE_DAC_ACCESS,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            0,
        )
    };
    if destination_security == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let destination_security = unsafe { OwnedHandle::from_raw_handle(destination_security) };
    let mut required = 0_u32;
    unsafe {
        GetKernelObjectSecurity(
            source.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut required,
        );
    }
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let words = usize::try_from(required)
        .map_err(|_| io::Error::other("security descriptor is too large"))?
        .div_ceil(std::mem::size_of::<usize>());
    let mut descriptor = vec![0_usize; words];
    let loaded = unsafe {
        GetKernelObjectSecurity(
            source.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    };
    if loaded == 0 {
        return Err(io::Error::last_os_error());
    }
    let stored = unsafe {
        SetKernelObjectSecurity(
            destination_security.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_mut_ptr().cast(),
        )
    };
    if stored == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
