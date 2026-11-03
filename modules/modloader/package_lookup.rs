use crate::package_files::{package_has_file, read_package_file};
use std::{
    fs::{self, Metadata},
    path::{Path, PathBuf},
};

/// Immutable identity for the installed package admitted to a runtime owner.
/// The package path alone is insufficient because another directory can be
/// moved into its place while a native plugin remains alive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModPackageBinding {
    id: String,
    mods_root: PathBuf,
    package_root: PathBuf,
    identity: PackageDirectoryIdentity,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PackageDirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PackageDirectoryIdentity {
    volume_serial: u32,
    file_index: u64,
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PackageDirectoryIdentity;

impl ModPackageBinding {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn package_root(&self) -> &Path {
        &self.package_root
    }

    pub fn matches_admitted_package(&self, id: &str, package_root: &Path) -> bool {
        self.id == id && self.package_root == package_root
    }
}

/// Capture directory identity while ModRegistry is admitting the package.
/// Uniqueness is checked by ModRegistry and rechecked whenever a binding is
/// used for installed settings access.
pub(crate) fn capture_admitted_mod_package(
    root: &Path,
    id: &str,
    admitted_package_root: &Path,
) -> Result<ModPackageBinding, String> {
    if !valid_mod_id(id) {
        return Err("invalid mod package ID".into());
    }
    let mods_root = canonical_mods_root(root)?;
    let package_root = canonical_package_directory(&mods_root, admitted_package_root)?;
    let package_id = read_package_id_without_redirection(&package_root)?
        .ok_or_else(|| "admitted package has no safe mod.json identity".to_owned())?;
    if package_id != id {
        return Err("admitted package ID changed while it was being loaded".into());
    }
    ensure_manifest_file_is_within_package(&package_root, "mod.json", true)?;
    ensure_manifest_file_is_within_package(&package_root, "extended.mod.json", false)?;
    let identity = package_directory_identity(&package_root)?;
    Ok(ModPackageBinding {
        id: id.to_owned(),
        mods_root,
        package_root,
        identity,
    })
}

pub(crate) fn is_admitted_package_directory(root: &Path, package_root: &Path) -> bool {
    canonical_mods_root(root)
        .and_then(|mods_root| canonical_package_directory(&mods_root, package_root))
        .and_then(|package| {
            ensure_manifest_file_is_within_package(&package, "mod.json", false)?;
            ensure_manifest_file_is_within_package(&package, "extended.mod.json", false)?;
            Ok(package)
        })
        .is_ok()
}

/// Revalidate an owner's pinned package identity. Call while holding the
/// installation lock before reading or writing its settings.
pub fn validate_mod_package_binding(
    root: &Path,
    binding: &ModPackageBinding,
) -> Result<PathBuf, String> {
    let _installation_lock = shroudforge_loader_config::config::installation_lock(root)?;
    validate_mod_package_binding_locked(root, binding)
}

pub(crate) fn validate_mod_package_binding_locked(
    root: &Path,
    binding: &ModPackageBinding,
) -> Result<PathBuf, String> {
    let mods_root = canonical_mods_root(root)?;
    if mods_root != binding.mods_root {
        return Err("native package installation root changed".into());
    }
    let resolved = resolve_mod_package_in_mods_root(&mods_root, &binding.id)?
        .ok_or_else(|| "native package is no longer installed".to_owned())?;
    if resolved != binding.package_root {
        return Err("native package moved or was rebound".into());
    }
    if package_directory_identity(&resolved)? != binding.identity {
        return Err("native package directory identity changed".into());
    }
    Ok(resolved)
}

/// Resolve only a unique, installed, non-reparse package directory whose
/// canonical location is an immediate child of this installation's mods/.
pub fn resolve_mod_package(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_mod_id(id) {
        return Err("invalid mod package ID".into());
    }
    let mods_root = canonical_mods_root(root)?;
    resolve_mod_package_in_mods_root(&mods_root, id)?.ok_or_else(|| "mod package not found".into())
}

pub fn find_mod_package(root: &Path, id: &str) -> Option<PathBuf> {
    resolve_mod_package(root, id).ok()
}

fn canonical_mods_root(root: &Path) -> Result<PathBuf, String> {
    let installation_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let mods_path = shroudforge_loader_config::paths::mods_dir(&installation_root);
    let metadata = fs::symlink_metadata(&mods_path).map_err(|error| error.to_string())?;
    if metadata_is_reparse_point(&metadata) || !metadata.is_dir() {
        return Err("installation mods path must be a regular directory".into());
    }
    let canonical_mods = fs::canonicalize(&mods_path).map_err(|error| error.to_string())?;
    if canonical_mods.parent() != Some(installation_root.as_path()) {
        return Err("installation mods path resolves outside the installation root".into());
    }
    Ok(canonical_mods)
}

fn resolve_mod_package_in_mods_root(mods_root: &Path, id: &str) -> Result<Option<PathBuf>, String> {
    let entries = fs::read_dir(mods_root).map_err(|error| error.to_string())?;
    let mut found = None;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with('.') {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let package_root = match canonical_package_directory(mods_root, &entry.path()) {
            Ok(path) => path,
            Err(_error) if is_reparse_or_not_directory(&entry.path()) => continue,
            Err(error) => return Err(error),
        };
        let Some(package_id) = read_package_id_without_redirection(&package_root)? else {
            continue;
        };
        if package_id != id {
            continue;
        }
        ensure_manifest_file_is_within_package(&package_root, "mod.json", true)?;
        ensure_manifest_file_is_within_package(&package_root, "extended.mod.json", false)?;
        if found.is_some() {
            return Err(format!("duplicate installed mod ID '{id}'"));
        }
        found = Some(package_root);
    }
    Ok(found)
}

fn canonical_package_directory(mods_root: &Path, package_root: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(package_root).map_err(|error| error.to_string())?;
    if metadata_is_reparse_point(&metadata) || !metadata.is_dir() {
        return Err("mod package must be a regular directory".into());
    }
    let canonical = fs::canonicalize(package_root).map_err(|error| error.to_string())?;
    if canonical.parent() != Some(mods_root) {
        return Err("mod package resolves outside the installation mods directory".into());
    }
    Ok(canonical)
}

fn read_package_id_without_redirection(package_root: &Path) -> Result<Option<String>, String> {
    if !ensure_manifest_file_is_within_package(package_root, "mod.json", false)? {
        return Ok(None);
    }
    let bytes = match fs::read(package_root.join("mod.json")) {
        Ok(bytes) => bytes,
        Err(error) => return Err(error.to_string()),
    };
    let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    Ok(value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| valid_mod_id(id))
        .map(str::to_owned))
}

fn ensure_manifest_file_is_within_package(
    package_root: &Path,
    name: &str,
    required: bool,
) -> Result<bool, String> {
    let path = package_root.join(name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => {
            return Ok(false);
        }
        Err(error) => return Err(error.to_string()),
    };
    if metadata_is_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!("{name} must be a regular file inside its package"));
    }
    if metadata_has_multiple_links(&path, &metadata)? {
        return Err(format!(
            "{name} must be a single-link file inside its package"
        ));
    }
    let canonical = fs::canonicalize(&path).map_err(|error| error.to_string())?;
    if canonical.parent() != Some(package_root) {
        return Err(format!("{name} resolves outside its package"));
    }
    Ok(true)
}

fn is_reparse_or_not_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata_is_reparse_point(&metadata) || !metadata.is_dir())
        .unwrap_or(false)
}

fn metadata_is_reparse_point(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(unix)]
fn metadata_has_multiple_links(_path: &Path, metadata: &Metadata) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    Ok(metadata.nlink() > 1)
}

#[cfg(windows)]
fn metadata_has_multiple_links(path: &Path, _metadata: &Metadata) -> Result<bool, String> {
    let info = read_windows_file_information(path)?;
    Ok(info.nNumberOfLinks > 1)
}

#[cfg(not(any(unix, windows)))]
fn metadata_has_multiple_links(_path: &Path, _metadata: &Metadata) -> Result<bool, String> {
    Ok(false)
}

#[cfg(windows)]
fn read_windows_file_information(
    path: &Path,
) -> Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION, String> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            GetFileInformationByHandle, OPEN_EXISTING,
        },
    };

    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let read_ok = unsafe { GetFileInformationByHandle(handle, &mut info) } != 0;
    let read_error = (!read_ok).then(std::io::Error::last_os_error);
    unsafe { CloseHandle(handle) };
    if let Some(error) = read_error {
        return Err(error.to_string());
    }
    Ok(info)
}

#[cfg(unix)]
fn package_directory_identity(package_root: &Path) -> Result<PackageDirectoryIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(package_root).map_err(|error| error.to_string())?;
    if metadata_is_reparse_point(&metadata) || !metadata.is_dir() {
        return Err("native package is no longer a regular directory".into());
    }
    Ok(PackageDirectoryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn package_directory_identity(package_root: &Path) -> Result<PackageDirectoryIdentity, String> {
    let metadata = fs::symlink_metadata(package_root).map_err(|error| error.to_string())?;
    if metadata_is_reparse_point(&metadata) || !metadata.is_dir() {
        return Err("native package is no longer a regular directory".into());
    }
    let info = read_windows_file_information(package_root)?;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
    {
        return Err("native package is no longer a regular directory".into());
    }
    Ok(PackageDirectoryIdentity {
        volume_serial: info.dwVolumeSerialNumber,
        file_index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

#[cfg(not(any(unix, windows)))]
fn package_directory_identity(_package_root: &Path) -> Result<PackageDirectoryIdentity, String> {
    Err("stable native package identity is unsupported on this platform".into())
}

pub fn mod_ecosystem(root: &Path, id: &str) -> Option<&'static str> {
    let package = find_mod_package(root, id)?;
    Some(mod_ecosystem_for_package(&package))
}

pub fn mod_ecosystem_for_package(package: &Path) -> &'static str {
    // ShroudForge mods always ship an extension. A package that only has
    // mod.json is an EML mod until ShroudForge creates the extension when the
    // user first changes its state.
    if !package_has_file(package, "extended.mod.json").unwrap_or(true) {
        return "EML";
    }
    let launcher = read_package_file(package, "extended.mod.json")
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("launcher")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    match launcher.as_deref() {
        Some("EML") => "EML",
        // Existing extensions without EML provenance belong to SF.
        _ => "SF",
    }
}

pub fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn valid_mod_id(value: &str) -> bool {
    valid_identifier(value) && value != "." && value != ".."
}
