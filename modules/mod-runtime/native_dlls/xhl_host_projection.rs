//! Private, process-stable projection for the original XHL DirectInput host.
//!
//! The host scans `<module directory>/mods`. Keep that directory owned by this
//! adapter and admit each XHL package through its own DLL Manager selection.

use mod_loader::dll_manager::{DllLoadEntry, DllLoadMode, DllLoadPlan, DllProfile, DllProfileFile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use uuid::Uuid;
use walkdir::WalkDir;

const HOST_DIRECTORY: &str = "xhl-native-core";
const HOST_FILE: &str = "dinput8.dll";
const HOST_LOG_FILE: &str = "XHL-Native-Plugin-Host.log";
const HOST_SHA256: &str = "c1c49397c488120540add0b6cb8f19180d81328d069dbef99b6161dc462576fa";
const PROJECTION_MARKER: &str = ".shroudforge-xhl-projection.json";
const MAX_PACKAGE_FILES: usize = 512;
const MAX_PACKAGE_BYTES: u64 = 64 * 1024 * 1024;

const PLUGIN_SPECS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "xhl-remote-altar-close",
        "XHL-Remote-Altar-Close",
        "XHL-Remote-Altar-Close.dll",
        "c3e4a521b5a1635ea94b1fdd67f00b3009ddb9f0066107f743aac2a92812ab17",
        "8f86c8f9445cae78522c52defe043dfd3b379f47e9567c10ac7d1a9805de6f4b",
    ),
    (
        "xhl-unlimited-gifting-range",
        "XHL-Unlimited-Gifting-Range",
        "XHL-Unlimited-Gifting-Range.dll",
        "3f5fedc265ea2df70e0789dab8f76af838040016c80b69dff13321f7762b995c",
        "ad160df23635d03e45e7d5482c035c740565b4d80eec30e646980fb3dece28b5",
    ),
];

#[derive(Clone, Debug)]
pub(super) enum Disposition {
    Skip,
    Route(PathBuf),
}

#[derive(Default)]
pub(super) struct Prepared {
    pub dispositions: HashMap<String, Disposition>,
    pub issues: Vec<(String, String)>,
}

#[derive(Clone)]
struct PackageContract {
    mod_id: String,
    directory: String,
    dll: String,
    dll_sha256: String,
    ini_sha256: String,
}

#[derive(Clone)]
struct Contract {
    host_sha256: String,
    packages: Vec<PackageContract>,
}

#[derive(Clone)]
struct SnapshotFile {
    relative_path: String,
    bytes: Vec<u8>,
    sha256: String,
}

#[derive(Clone)]
struct PackageSnapshot {
    mod_id: String,
    directory: String,
    files: Vec<SnapshotFile>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProjectionManifest {
    schema_version: u32,
    fingerprint: String,
    host: PackageManifest,
    packages: Vec<PackageManifest>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PackageManifest {
    mod_id: String,
    package_directory: String,
    files: Vec<FileManifest>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FileManifest {
    path: String,
    bytes: u64,
    sha256: String,
}

struct Snapshot {
    host: PackageSnapshot,
    packages: Vec<PackageSnapshot>,
    manifest: ProjectionManifest,
}

static PROJECTION_LEASES: OnceLock<Mutex<BTreeMap<PathBuf, Vec<File>>>> = OnceLock::new();

/// Prepare XHL entries from the same resolved DLL plan the normal loader uses.
/// Known XHL payloads are always withheld from direct library loading. The core
/// is routed only when its own selection and at least one plugin selection pass.
pub(super) fn prepare(root: &Path, is_server: bool, plan: &DllLoadPlan) -> Prepared {
    let selections = mod_loader::dll_manager::read_selections(root);
    prepare_with_contract(root, is_server, plan, &selections, &production_contract())
}

fn prepare_with_contract(
    root: &Path,
    is_server: bool,
    plan: &DllLoadPlan,
    selections: &[DllProfile],
    contract: &Contract,
) -> Prepared {
    let mut prepared = Prepared::default();
    let mut host_entries = Vec::new();
    let mut plugin_entries: HashMap<String, Vec<&DllLoadEntry>> = HashMap::new();

    for entry in &plan.entries {
        if entry.package_directory.eq_ignore_ascii_case(HOST_DIRECTORY) {
            prepared
                .dispositions
                .insert(entry.status_id.clone(), Disposition::Skip);
            host_entries.push(entry);
        } else if let Some(spec) = plugin_spec(entry, contract) {
            prepared
                .dispositions
                .insert(entry.status_id.clone(), Disposition::Skip);
            plugin_entries
                .entry(spec.mod_id.clone())
                .or_default()
                .push(entry);
        }
    }

    if host_entries.is_empty() {
        return prepared;
    }
    if is_server {
        tracing::info!(target: "shroudforge::dll_manager",
            "The XHL DirectInput host is client-only and was skipped for this process");
        return prepared;
    }

    if !selected(selections, HOST_DIRECTORY, false) {
        tracing::info!(target: "shroudforge::dll_manager",
            "The XHL DirectInput host was skipped because its DLL Manager selection is not enabled and approved");
        return prepared;
    }

    if let Err(error) = validate_host_entries(root, &host_entries) {
        add_issue(&mut prepared, &host_entries, error);
        return prepared;
    }

    let mut admitted = Vec::new();
    for spec in &contract.packages {
        let entries = plugin_entries
            .get(&spec.mod_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if entries.is_empty() {
            continue;
        }
        match admit_package(root, selections, false, spec, entries) {
            Ok(Some(package)) => admitted.push(package),
            Ok(None) => {
                tracing::debug!(target: "shroudforge::dll_manager", mod_id = %spec.mod_id,
                    "XHL package was not enabled and approved in its own DLL Manager selection");
            }
            Err(error) => {
                add_issue(&mut prepared, entries, error);
            }
        }
    }

    if admitted.is_empty() {
        tracing::info!(target: "shroudforge::dll_manager",
            "The XHL DirectInput host was skipped because no XHL plugin package passed its individual admission checks");
        return prepared;
    }

    admitted.sort_by_key(|package| package.directory.to_ascii_lowercase());
    match make_snapshot(root, &admitted, contract)
        .and_then(|snapshot| publish_projection(root, &snapshot))
    {
        Ok(projected_host) => {
            for entry in host_entries {
                prepared.dispositions.insert(
                    entry.status_id.clone(),
                    Disposition::Route(projected_host.clone()),
                );
            }
            tracing::info!(target: "shroudforge::dll_manager", host = %projected_host.display(),
                plugins = admitted.len(),
                "Prepared the private XHL host projection; plugin initialization remains owned by XHL");
        }
        Err(error) => {
            add_issue(&mut prepared, &host_entries, error);
        }
    }
    prepared
}

fn production_contract() -> Contract {
    Contract {
        host_sha256: HOST_SHA256.into(),
        packages: PLUGIN_SPECS
            .iter()
            .map(
                |(mod_id, directory, dll, dll_sha256, ini_sha256)| PackageContract {
                    mod_id: (*mod_id).into(),
                    directory: (*directory).into(),
                    dll: (*dll).into(),
                    dll_sha256: (*dll_sha256).into(),
                    ini_sha256: (*ini_sha256).into(),
                },
            )
            .collect(),
    }
}

fn plugin_spec<'a>(entry: &DllLoadEntry, contract: &'a Contract) -> Option<&'a PackageContract> {
    contract.packages.iter().find(|spec| {
        entry
            .package_directory
            .eq_ignore_ascii_case(&spec.directory)
            || entry
                .mod_id
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(&spec.mod_id))
    })
}

fn selected(selections: &[DllProfile], directory: &str, is_server: bool) -> bool {
    let mut matches = selections
        .iter()
        .filter(|selection| selection.package_directory.eq_ignore_ascii_case(directory));
    let Some(selection) = matches.next() else {
        return false;
    };
    matches.next().is_none()
        && selection.enabled
        && selection.approved
        && selection.target.applies_to(is_server)
}

fn validate_host_entries(root: &Path, entries: &[&DllLoadEntry]) -> Result<(), String> {
    let expected = mod_loader::dll_manager::resolve_package_dll_path(
        root,
        HOST_DIRECTORY,
        &DllProfileFile {
            path: HOST_FILE.into(),
            mode: DllLoadMode::Dinput8,
        },
    )?;
    if entries.is_empty() {
        return Err("The XHL host is absent from the resolved DLL plan".into());
    }
    for entry in entries {
        if entry.mode != DllLoadMode::Dinput8
            || !entry
                .path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(HOST_FILE))
            || entry
                .path
                .canonicalize()
                .map_err(|error| error.to_string())?
                != expected.canonicalize().map_err(|error| error.to_string())?
        {
            return Err(format!(
                "Managed XHL host entry '{}' must select mods/{HOST_DIRECTORY}/{HOST_FILE} in DirectInput mode",
                entry.status_id
            ));
        }
    }
    Ok(())
}

fn admit_package(
    root: &Path,
    selections: &[DllProfile],
    is_server: bool,
    spec: &PackageContract,
    entries: &[&DllLoadEntry],
) -> Result<Option<PackageSnapshot>, String> {
    let PackageContract {
        mod_id,
        directory,
        dll,
        dll_sha256: expected_dll_sha,
        ini_sha256: expected_ini_sha,
    } = spec;
    if !selected(selections, directory, is_server) {
        return Ok(None);
    }
    if entries.len() != 1 {
        return Err(format!(
            "XHL package {directory} has {} resolved DLL entries; exactly its declared plugin DLL is required",
            entries.len()
        ));
    }
    let entry = entries[0];
    if !entry.package_directory.eq_ignore_ascii_case(directory)
        || !entry
            .mod_id
            .as_deref()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(mod_id))
        || entry.mode != DllLoadMode::Library
    {
        return Err(format!(
            "XHL package {directory} must be admitted by its own package identity using ordinary DLL selection"
        ));
    }
    let expected_path = mod_loader::dll_manager::resolve_package_dll_path(
        root,
        directory,
        &DllProfileFile {
            path: dll.into(),
            mode: DllLoadMode::Library,
        },
    )?;
    if entry
        .path
        .canonicalize()
        .map_err(|error| error.to_string())?
        != expected_path
            .canonicalize()
            .map_err(|error| error.to_string())?
    {
        return Err(format!(
            "XHL package {directory} selected a DLL other than {dll}"
        ));
    }

    let package_path = mod_loader::paths::mods_dir(root).join(directory);
    let (ini_state, declared_dll) = mod_loader::dll_manager::read_plugin_ini(&package_path);
    if ini_state != "enabled" || declared_dll.as_deref() != Some(dll) {
        return Err(format!(
            "XHL package {directory} no longer has its expected enabled native-plugin.ini declaration"
        ));
    }

    let package = read_package(root, directory, mod_id, false)?;
    require_hash(&package, dll, expected_dll_sha, "XHL plugin DLL")?;
    require_hash(
        &package,
        "native-plugin.ini",
        expected_ini_sha,
        "XHL plugin declaration",
    )?;
    verify_manifest_id(&package, mod_id)?;
    Ok(Some(package))
}

fn verify_manifest_id(package: &PackageSnapshot, expected: &str) -> Result<(), String> {
    let file = package
        .files
        .iter()
        .find(|file| file.relative_path.eq_ignore_ascii_case("mod.json"))
        .ok_or_else(|| format!("XHL package {} has no mod.json", package.directory))?;
    let manifest: serde_json::Value = serde_json::from_slice(&file.bytes)
        .map_err(|error| format!("XHL package {} mod.json: {error}", package.directory))?;
    if !manifest
        .get("id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
    {
        return Err(format!(
            "XHL package {} mod.json does not match its admitted package identity",
            package.directory
        ));
    }
    Ok(())
}

fn require_hash(
    package: &PackageSnapshot,
    path: &str,
    expected: &str,
    label: &str,
) -> Result<(), String> {
    let file = package
        .files
        .iter()
        .find(|file| file.relative_path.eq_ignore_ascii_case(path))
        .ok_or_else(|| format!("{} is missing from {}", label, package.directory))?;
    if !file.sha256.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "{} identity changed in {}",
            label, package.directory
        ));
    }
    Ok(())
}

fn make_snapshot(
    root: &Path,
    admitted: &[PackageSnapshot],
    contract: &Contract,
) -> Result<Snapshot, String> {
    let host = read_package(root, HOST_DIRECTORY, HOST_DIRECTORY, true)?;
    require_hash(&host, HOST_FILE, &contract.host_sha256, "Original XHL host")?;
    let mut packages = admitted.to_vec();
    packages.sort_by_key(|package| package.directory.to_ascii_lowercase());
    let mut manifest = ProjectionManifest {
        schema_version: 1,
        fingerprint: String::new(),
        host: package_manifest(&host),
        packages: packages.iter().map(package_manifest).collect(),
    };
    let identity = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
    manifest.fingerprint = hex_sha256(&identity);

    // Re-read every copied source after taking the in-memory snapshot. A file
    // that changed during startup cannot produce a mixed projection.
    let host_after = read_package(root, HOST_DIRECTORY, HOST_DIRECTORY, true)?;
    if !same_snapshot(&host, &host_after) {
        return Err(
            "The original XHL host package changed while its projection was prepared".into(),
        );
    }
    for package in &packages {
        let after = read_package(root, &package.directory, &package.mod_id, false)?;
        if !same_snapshot(package, &after) {
            return Err(format!(
                "XHL package {} changed while its projection was prepared",
                package.directory
            ));
        }
    }
    Ok(Snapshot {
        host,
        packages,
        manifest,
    })
}

fn read_package(
    root: &Path,
    directory: &str,
    mod_id: &str,
    omit_adjacent_mods: bool,
) -> Result<PackageSnapshot, String> {
    if directory.is_empty()
        || directory.contains(['/', '\\', ':'])
        || directory == "."
        || directory == ".."
    {
        return Err(format!("Invalid XHL package directory: {directory}"));
    }
    let mods = mod_loader::paths::mods_dir(root)
        .canonicalize()
        .map_err(|error| format!("Could not resolve the installed mods folder: {error}"))?;
    let source = mod_loader::paths::mods_dir(root).join(directory);
    let source_meta = fs::symlink_metadata(&source)
        .map_err(|error| format!("Could not inspect XHL package {directory}: {error}"))?;
    if !source_meta.is_dir() || link_like(&source, &source_meta, false) {
        return Err(format!(
            "XHL package {directory} is not a regular directory"
        ));
    }
    let package = source
        .canonicalize()
        .map_err(|error| format!("Could not resolve XHL package {directory}: {error}"))?;
    if package.parent() != Some(mods.as_path()) || package.file_name().is_none() {
        return Err(format!(
            "XHL package {directory} is not a direct mods child"
        ));
    }
    if omit_adjacent_mods {
        let adjacent_mods = package.join("mods");
        match fs::symlink_metadata(&adjacent_mods) {
            Ok(metadata) if !metadata.is_dir() || link_like(&adjacent_mods, &metadata, false) => {
                return Err(
                    "The original XHL host package has an invalid adjacent mods entry".into(),
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Could not inspect adjacent XHL mods folder: {error}"
                ));
            }
        }
    }

    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut total_bytes = 0u64;
    for entry in WalkDir::new(&package)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            !(omit_adjacent_mods
                && entry.depth() == 1
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("mods"))
        })
    {
        let entry =
            entry.map_err(|error| format!("Could not inspect XHL package {directory}: {error}"))?;
        if entry.depth() == 0 {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&package)
            .map_err(|error| error.to_string())?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("Could not inspect {}: {error}", entry.path().display()))?;
        if link_like(entry.path(), &metadata, entry.file_type().is_file()) {
            return Err(format!(
                "Links are not allowed in XHL package {}",
                directory
            ));
        }
        if entry.file_type().is_dir() {
            if omit_adjacent_mods
                && entry.depth() == 1
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(HOST_LOG_FILE)
            {
                return Err("The XHL host log output is not a regular file".into());
            }
            continue;
        }
        if !entry.file_type().is_file() {
            return Err(format!(
                "Unsupported file type in XHL package {}",
                directory
            ));
        }
        let relative_path = safe_relative_path(relative)?;
        if !seen.insert(relative_path.to_ascii_lowercase()) {
            return Err(format!(
                "Duplicate package path in XHL package {}",
                directory
            ));
        }
        if omit_adjacent_mods && is_host_log_output(&relative_path) {
            // This exact root sidecar is host output, not an input package file.
            continue;
        }
        if files.len() >= MAX_PACKAGE_FILES {
            return Err(format!("XHL package {} has too many files", directory));
        }
        let before = file_stamp(&metadata);
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("Could not read {}: {error}", entry.path().display()))?;
        let after_meta = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("Could not recheck {}: {error}", entry.path().display()))?;
        if link_like(entry.path(), &after_meta, true) || before != file_stamp(&after_meta) {
            return Err(format!(
                "XHL package file changed while being read: {}",
                entry.path().display()
            ));
        }
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_PACKAGE_BYTES {
            return Err(format!(
                "XHL package {} exceeds the snapshot size limit",
                directory
            ));
        }
        files.push(SnapshotFile {
            relative_path,
            sha256: hex_sha256(&bytes),
            bytes,
        });
    }
    files.sort_by_key(|file| file.relative_path.to_ascii_lowercase());
    Ok(PackageSnapshot {
        mod_id: mod_id.into(),
        directory: directory.into(),
        files,
    })
}

#[derive(PartialEq, Eq)]
struct FileStamp(u64, Option<std::time::SystemTime>);

fn file_stamp(metadata: &fs::Metadata) -> FileStamp {
    FileStamp(metadata.len(), metadata.modified().ok())
}

fn safe_relative_path(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            _ => return Err("Package file paths must stay relative to their package".into()),
        }
    }
    if parts.is_empty() {
        return Err("Package file path is empty".into());
    }
    let value = parts.join("/");
    if value.contains([':', '\\', '\0']) {
        return Err("Package file path contains an invalid character".into());
    }
    Ok(value)
}

fn link_like(_path: &Path, metadata: &fs::Metadata, file: bool) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
        return file && windows_hard_linked(_path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return file && metadata.nlink() > 1;
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = file;
        false
    }
}

#[cfg(windows)]
#[repr(C)]
#[allow(dead_code)]
#[derive(Default)]
struct WindowsFileTime {
    low: u32,
    high: u32,
}

#[cfg(windows)]
#[repr(C)]
#[allow(dead_code)]
#[derive(Default)]
struct WindowsFileInformation {
    attributes: u32,
    creation_time: WindowsFileTime,
    last_access_time: WindowsFileTime,
    last_write_time: WindowsFileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(
        file: std::os::windows::io::RawHandle,
        information: *mut WindowsFileInformation,
    ) -> i32;
}

#[cfg(windows)]
fn windows_hard_linked(path: &Path) -> bool {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};

    let file = match OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0007)
        .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
        .open(path)
    {
        Ok(file) => file,
        Err(_) => return true,
    };
    let mut information = WindowsFileInformation::default();
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    succeeded == 0 || information.number_of_links > 1
}

fn package_manifest(package: &PackageSnapshot) -> PackageManifest {
    PackageManifest {
        mod_id: package.mod_id.clone(),
        package_directory: package.directory.clone(),
        files: package
            .files
            .iter()
            .map(|file| FileManifest {
                path: file.relative_path.clone(),
                bytes: file.bytes.len() as u64,
                sha256: file.sha256.clone(),
            })
            .collect(),
    }
}

fn same_snapshot(left: &PackageSnapshot, right: &PackageSnapshot) -> bool {
    left.mod_id.eq_ignore_ascii_case(&right.mod_id)
        && left.directory.eq_ignore_ascii_case(&right.directory)
        && left.files.len() == right.files.len()
        && left.files.iter().zip(&right.files).all(|(left, right)| {
            left.relative_path
                .eq_ignore_ascii_case(&right.relative_path)
                && left.sha256 == right.sha256
                && left.bytes.len() == right.bytes.len()
        })
}

fn publish_projection(root: &Path, snapshot: &Snapshot) -> Result<PathBuf, String> {
    let runtime = mod_loader::paths::runtime_dir(root);
    let parent = runtime.join("xhl-host-projections");
    fs::create_dir_all(&parent)
        .map_err(|error| format!("Could not create the private XHL projection folder: {error}"))?;
    reject_reparse(&parent)?;
    let final_path = parent.join(&snapshot.manifest.fingerprint);
    let marker = serde_json::to_vec(&snapshot.manifest).map_err(|error| error.to_string())?;
    if final_path.exists() {
        validate_projection(&final_path, snapshot, &marker)?;
        retain_projection(&final_path, snapshot)?;
        return Ok(final_path.join(HOST_FILE));
    }
    let staging = parent.join(format!(".staging-{}", Uuid::new_v4()));
    fs::create_dir(&staging)
        .map_err(|error| format!("Could not create the XHL projection staging folder: {error}"))?;
    let cleanup = StagingDirectory(staging.clone());
    write_package(&staging, &snapshot.host)?;
    fs::create_dir(staging.join("mods"))
        .map_err(|error| format!("Could not create the mandatory adjacent mods folder: {error}"))?;
    for package in &snapshot.packages {
        let destination = staging.join("mods").join(&package.directory);
        fs::create_dir(&destination)
            .map_err(|error| format!("Could not create the XHL package projection: {error}"))?;
        write_package(&destination, package)?;
    }
    let marker_path = staging.join(PROJECTION_MARKER);
    write_new_file(&marker_path, &marker)?;
    validate_projection(&staging, snapshot, &marker)?;

    // Recheck source package bytes immediately before publication. The copied
    // files are then independent of later writes to the installed packages.
    let host_after = read_package(root, HOST_DIRECTORY, HOST_DIRECTORY, true)?;
    if !same_snapshot(&snapshot.host, &host_after) {
        return Err("The original XHL host package changed before projection publication".into());
    }
    for package in &snapshot.packages {
        let after = read_package(root, &package.directory, &package.mod_id, false)?;
        if !same_snapshot(package, &after) {
            return Err(format!(
                "XHL package {} changed before projection publication",
                package.directory
            ));
        }
    }

    match fs::rename(&staging, &final_path) {
        Ok(()) => std::mem::forget(cleanup),
        Err(_error) if final_path.exists() => {
            validate_projection(&final_path, snapshot, &marker)?;
        }
        Err(error) => {
            return Err(format!(
                "Could not publish the XHL projection atomically: {error}"
            ));
        }
    }
    validate_projection(&final_path, snapshot, &marker)?;
    retain_projection(&final_path, snapshot)?;
    Ok(final_path.join(HOST_FILE))
}

struct StagingDirectory(PathBuf);

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_package(destination: &Path, package: &PackageSnapshot) -> Result<(), String> {
    for file in &package.files {
        let path = destination.join(
            file.relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        write_new_file(&path, &file.bytes)?;
        let written = fs::read(&path).map_err(|error| error.to_string())?;
        if hex_sha256(&written) != file.sha256 {
            return Err(format!(
                "Copied XHL package file failed hash verification: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("Could not flush {}: {error}", path.display()))
}

fn validate_projection(root: &Path, snapshot: &Snapshot, marker: &[u8]) -> Result<(), String> {
    reject_reparse(root)?;
    if fs::read(root.join(PROJECTION_MARKER)).map_err(|error| error.to_string())? != marker {
        return Err("Existing XHL projection marker does not match the requested snapshot".into());
    }
    let mut expected_files = HashMap::<String, &SnapshotFile>::new();
    add_expected_files(&mut expected_files, "", &snapshot.host)?;
    for package in &snapshot.packages {
        add_expected_files(
            &mut expected_files,
            &format!("mods/{}", package.directory),
            package,
        )?;
    }
    let marker_relative = PROJECTION_MARKER.to_ascii_lowercase();
    let mut actual_files = HashSet::new();
    let mut actual_directories = HashSet::from([String::new()]);

    for entry in WalkDir::new(root).follow_links(false).sort_by_file_name() {
        let entry = entry.map_err(|error| format!("Could not validate XHL projection: {error}"))?;
        if entry.depth() == 0 {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if link_like(entry.path(), &metadata, entry.file_type().is_file()) {
            return Err("Links are not allowed in the generated XHL projection".into());
        }
        let relative = safe_relative_path(
            entry
                .path()
                .strip_prefix(root)
                .map_err(|error| error.to_string())?,
        )?;
        let key = relative.to_ascii_lowercase();
        if entry.file_type().is_dir() {
            actual_directories.insert(key);
            continue;
        }
        if !entry.file_type().is_file() {
            return Err("Unsupported file type in the generated XHL projection".into());
        }
        if !actual_files.insert(key.clone()) {
            return Err("Duplicate path in the generated XHL projection".into());
        }
        if key == marker_relative || is_host_log_output(&relative) {
            continue;
        }
        if let Some(expected) = expected_files.get(&key) {
            let bytes = fs::read(entry.path()).map_err(|error| error.to_string())?;
            if hex_sha256(&bytes) != expected.sha256 {
                return Err(format!("XHL projection input changed: {relative}"));
            }
        } else {
            return Err(format!(
                "Unexpected stale file in XHL projection: {relative}"
            ));
        }
    }

    for path in expected_files.keys() {
        if !actual_files.contains(path) {
            return Err(format!("Required XHL projection file is missing: {path}"));
        }
    }
    if !actual_directories.contains("mods") {
        return Err("The XHL projection is missing its adjacent mods folder".into());
    }
    let mut required_directories = HashSet::from(["mods".to_owned()]);
    for path in expected_files.keys() {
        let mut parent = Path::new(path).parent();
        while let Some(path) = parent {
            let value = path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            if !value.is_empty() {
                required_directories.insert(value);
            }
            parent = path.parent();
        }
    }
    for directory in &actual_directories {
        if !directory.is_empty() && !required_directories.contains(directory) {
            return Err(format!(
                "Unexpected stale directory in XHL projection: {directory}"
            ));
        }
    }
    Ok(())
}

fn add_expected_files<'a>(
    output: &mut HashMap<String, &'a SnapshotFile>,
    prefix: &str,
    package: &'a PackageSnapshot,
) -> Result<(), String> {
    for file in &package.files {
        let path = if prefix.is_empty() {
            file.relative_path.clone()
        } else {
            format!("{prefix}/{}", file.relative_path)
        };
        if output.insert(path.to_ascii_lowercase(), file).is_some() {
            return Err(format!("Duplicate path in XHL projection plan: {path}"));
        }
    }
    Ok(())
}

fn is_host_log_output(path: &str) -> bool {
    path.eq_ignore_ascii_case(HOST_LOG_FILE)
}

fn retain_projection(root: &Path, snapshot: &Snapshot) -> Result<(), String> {
    let mut handles = Vec::new();
    handles.push(hold_directory(&root.join("mods"))?);
    for file in &snapshot.host.files {
        if immutable_through_scan(&file.relative_path) {
            let path = root.join(
                file.relative_path
                    .replace('/', std::path::MAIN_SEPARATOR_STR),
            );
            handles.push(hold_file(&path)?);
        }
    }
    for package in &snapshot.packages {
        let package_root = root.join("mods").join(&package.directory);
        for file in &package.files {
            if immutable_through_scan(&file.relative_path) {
                let path = package_root.join(
                    file.relative_path
                        .replace('/', std::path::MAIN_SEPARATOR_STR),
                );
                handles.push(hold_file(&path)?);
            }
        }
    }
    let mut leases = PROJECTION_LEASES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    leases.entry(root.to_path_buf()).or_insert(handles);
    Ok(())
}

fn immutable_through_scan(path: &str) -> bool {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    leaf.eq_ignore_ascii_case("native-plugin.ini")
        || leaf.eq_ignore_ascii_case("mod.json")
        || path.to_ascii_lowercase().ends_with(".dll")
        || path.to_ascii_lowercase().ends_with(".lua")
}

fn hold_file(path: &Path) -> Result<File, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .share_mode(0x0000_0001) // FILE_SHARE_READ: deny replacement/write through scan.
            .open(path)
            .map_err(|error| {
                format!(
                    "Could not retain immutable XHL input {}: {error}",
                    path.display()
                )
            })
    }
    #[cfg(not(windows))]
    {
        File::open(path)
            .map_err(|error| format!("Could not retain XHL input {}: {error}", path.display()))
    }
}

fn hold_directory(path: &Path) -> Result<File, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .share_mode(0x0000_0001) // FILE_SHARE_READ: keep the admitted child set stable.
            .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS
            .open(path)
            .map_err(|error| {
                format!(
                    "Could not retain the XHL projection directory {}: {error}",
                    path.display()
                )
            })
    }
    #[cfg(not(windows))]
    {
        File::open(path).map_err(|error| {
            format!(
                "Could not retain XHL projection directory {}: {error}",
                path.display()
            )
        })
    }
}

fn reject_reparse(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || link_like(path, &metadata, false) {
        return Err(format!(
            "XHL projection path is not a regular directory: {}",
            path.display()
        ));
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn add_issue(prepared: &mut Prepared, entries: &[&DllLoadEntry], detail: String) {
    let id = entries
        .first()
        .map(|entry| entry.status_id.clone())
        .unwrap_or_else(|| format!("dll:{HOST_DIRECTORY}"));
    prepared.issues.push((id, detail));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!("sf-xhl-projection-{}", Uuid::new_v4()))
    }

    fn remove_test_root(root: &Path) {
        let projection_root = root.join("shroudforge/runtime/xhl-host-projections");
        if let Some(leases) = PROJECTION_LEASES.get() {
            leases
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .retain(|path, _| !path.starts_with(&projection_root));
        }
        let _ = fs::remove_dir_all(root);
    }

    fn contract(host: &[u8], plugins: &[(&str, &str, &str, &[u8], &[u8])]) -> Contract {
        Contract {
            host_sha256: hex_sha256(host),
            packages: plugins
                .iter()
                .map(
                    |(id, directory, dll, dll_bytes, ini_bytes)| PackageContract {
                        mod_id: (*id).into(),
                        directory: (*directory).into(),
                        dll: (*dll).into(),
                        dll_sha256: hex_sha256(dll_bytes),
                        ini_sha256: hex_sha256(ini_bytes),
                    },
                )
                .collect(),
        }
    }

    fn fixture(root: &Path) -> (Contract, Vec<DllProfile>, DllLoadPlan) {
        let mods = root.join("mods");
        let host = mods.join(HOST_DIRECTORY);
        fs::create_dir_all(&host).unwrap();
        let host_bytes = b"fake xhl host; never loaded";
        fs::write(host.join(HOST_FILE), host_bytes).unwrap();

        let mut selection_list = vec![DllProfile {
            id: "xhl-core-selection".into(),
            package_directory: HOST_DIRECTORY.into(),
            name: "XHL host".into(),
            target: mod_loader::dll_manager::DllProfileTarget::Client,
            enabled: true,
            approved: true,
            compatibility_profile: None,
            files: vec![DllProfileFile {
                path: HOST_FILE.into(),
                mode: DllLoadMode::Dinput8,
            }],
        }];
        let mut plan = DllLoadPlan::default();
        plan.entries.push(DllLoadEntry {
            status_id: "dll:xhl-native-core:dinput8.dll".into(),
            mod_id: None,
            package_directory: HOST_DIRECTORY.into(),
            path: host.join(HOST_FILE),
            mode: DllLoadMode::Dinput8,
            source: "manual".into(),
        });

        let definitions = [
            (
                "xhl-remote-altar-close",
                "XHL-Remote-Altar-Close",
                "XHL-Remote-Altar-Close.dll",
                b"fake remote altar payload".as_slice(),
                b"[Plugin]\r\nEnabled=1\r\nDll=XHL-Remote-Altar-Close.dll\r\n".as_slice(),
            ),
            (
                "xhl-unlimited-gifting-range",
                "XHL-Unlimited-Gifting-Range",
                "XHL-Unlimited-Gifting-Range.dll",
                b"fake unlimited gifting payload".as_slice(),
                b"[Plugin]\r\nEnabled=1\r\nDll=XHL-Unlimited-Gifting-Range.dll\r\n".as_slice(),
            ),
        ];
        let mut test_packages = Vec::new();
        for (id, directory, dll, dll_bytes, ini_bytes) in definitions {
            let package = mods.join(directory);
            fs::create_dir_all(package.join("src")).unwrap();
            fs::write(
                package.join("mod.json"),
                format!(r#"{{"id":"{id}","name":"Fixture","version":"1.0.0"}}"#),
            )
            .unwrap();
            fs::write(package.join("native-plugin.ini"), ini_bytes).unwrap();
            fs::write(package.join(dll), dll_bytes).unwrap();
            fs::write(package.join("src/mod.lua"), b"print('fixture')").unwrap();
            selection_list.push(DllProfile {
                id: format!("{id}-selection"),
                package_directory: directory.into(),
                name: id.into(),
                target: mod_loader::dll_manager::DllProfileTarget::Client,
                enabled: true,
                approved: true,
                compatibility_profile: None,
                files: vec![DllProfileFile {
                    path: dll.into(),
                    mode: DllLoadMode::Library,
                }],
            });
            plan.entries.push(DllLoadEntry {
                status_id: format!("dll:{directory}:{dll}"),
                mod_id: Some(id.into()),
                package_directory: directory.into(),
                path: package.join(dll),
                mode: DllLoadMode::Library,
                source: "manual".into(),
            });
            test_packages.push((id, directory, dll, dll_bytes, ini_bytes));
        }
        let references: Vec<_> = test_packages
            .iter()
            .map(|(id, directory, dll, dll_bytes, ini_bytes)| {
                (*id, *directory, *dll, *dll_bytes, *ini_bytes)
            })
            .collect();
        (contract(host_bytes, &references), selection_list, plan)
    }

    fn prepare_fixture(
        root: &Path,
        selections: &[DllProfile],
        plan: &DllLoadPlan,
        contract: &Contract,
    ) -> Prepared {
        prepare_with_contract(root, false, plan, selections, contract)
    }

    fn write_active_mod_manifests(
        root: &Path,
        disabled: &[&str],
        missing_gifting_dependency: bool,
    ) {
        for (id, directory) in [
            ("xhl-remote-altar-close", "XHL-Remote-Altar-Close"),
            ("xhl-unlimited-gifting-range", "XHL-Unlimited-Gifting-Range"),
        ] {
            let package = root.join("mods").join(directory);
            let dependencies = if missing_gifting_dependency && id == "xhl-unlimited-gifting-range"
            {
                serde_json::json!([{"id":"missing-fixture-dependency","version":">=1.0.0"}])
            } else {
                serde_json::json!([])
            };
            fs::write(
                package.join("mod.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id": id,
                    "name": "Fixture",
                    "version": "1.0.0",
                    "dependencies": dependencies,
                }))
                .unwrap(),
            )
            .unwrap();
            fs::write(
                package.join("extended.mod.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schemaVersion": 1,
                    "enabled": !disabled.contains(&id),
                    "targets": ["client"],
                }))
                .unwrap(),
            )
            .unwrap();
        }
    }

    fn resolve_public_plan(
        root: &Path,
        selections: &[DllProfile],
        manager_enabled: bool,
    ) -> DllLoadPlan {
        let config_path = shroudforge_loader_config::paths::loader_config(root);
        if !config_path.is_file() {
            shroudforge_loader_config::config::initialize_loader_config(root).unwrap();
        }
        let mut config = shroudforge_loader_config::config::read_loader(root).unwrap();
        config["modules"]["dllManager"]["enabled"] = serde_json::json!(manager_enabled);
        config["modules"]["dllManager"]["profiles"] = serde_json::to_value(selections).unwrap();
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        mod_loader::dll_manager::resolve_load_plan(root, false)
    }

    fn public_fixture(root: &Path) -> (Contract, Vec<DllProfile>) {
        let (contract, selections, _) = fixture(root);
        write_active_mod_manifests(root, &[], false);
        (contract, selections)
    }

    #[test]
    fn private_projection_copies_only_admitted_packages_and_preserves_input_bytes() {
        let root = test_root();
        let (contract, selections, plan) = fixture(&root);
        let result = prepare_fixture(&root, &selections, &plan, &contract);
        let core_id = "dll:xhl-native-core:dinput8.dll";
        let Disposition::Route(host) = result.dispositions.get(core_id).unwrap() else {
            panic!("expected projected host route");
        };
        assert_eq!(host.file_name().unwrap(), HOST_FILE);
        let adjacent_mods = host.parent().unwrap().join("mods");
        assert!(adjacent_mods.is_dir());
        assert!(
            adjacent_mods
                .join("XHL-Remote-Altar-Close/native-plugin.ini")
                .is_file()
        );
        assert!(
            adjacent_mods
                .join("XHL-Unlimited-Gifting-Range/native-plugin.ini")
                .is_file()
        );
        assert_eq!(
            fs::read(adjacent_mods.join("XHL-Remote-Altar-Close/native-plugin.ini")).unwrap(),
            b"[Plugin]\r\nEnabled=1\r\nDll=XHL-Remote-Altar-Close.dll\r\n"
        );
        assert_eq!(
            fs::read(adjacent_mods.join("XHL-Remote-Altar-Close/XHL-Remote-Altar-Close.dll"))
                .unwrap(),
            b"fake remote altar payload"
        );
        assert_eq!(
            fs::read(adjacent_mods.join("XHL-Unlimited-Gifting-Range/src/mod.lua")).unwrap(),
            b"print('fixture')"
        );
        remove_test_root(&root);
    }

    #[test]
    fn unapproved_plugin_is_not_admitted_even_when_the_host_is_approved() {
        let root = test_root();
        let (contract, mut selections, plan) = fixture(&root);
        let gifting = selections
            .iter_mut()
            .find(|selection| selection.package_directory == "XHL-Unlimited-Gifting-Range")
            .unwrap();
        gifting.enabled = false;
        let result = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(host) = result
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("the independently approved package should still route");
        };
        let adjacent_mods = host.parent().unwrap().join("mods");
        assert!(adjacent_mods.join("XHL-Remote-Altar-Close").is_dir());
        assert!(!adjacent_mods.join("XHL-Unlimited-Gifting-Range").exists());
        assert!(matches!(
            result
                .dispositions
                .get("dll:XHL-Unlimited-Gifting-Range:XHL-Unlimited-Gifting-Range.dll"),
            Some(Disposition::Skip)
        ));
        remove_test_root(&root);
    }

    #[test]
    fn core_is_skipped_when_no_plugin_has_individual_admission() {
        let root = test_root();
        let (contract, mut selections, plan) = fixture(&root);
        for selection in &mut selections {
            if selection.package_directory != HOST_DIRECTORY {
                selection.approved = false;
            }
        }
        let result = prepare_fixture(&root, &selections, &plan, &contract);
        assert!(matches!(
            result.dispositions.get("dll:xhl-native-core:dinput8.dll"),
            Some(Disposition::Skip)
        ));
        assert!(
            !root
                .join("shroudforge/runtime/xhl-host-projections")
                .exists()
        );
        remove_test_root(&root);
    }

    #[test]
    fn canonical_package_edits_create_a_new_snapshot() {
        let root = test_root();
        let (contract, selections, plan) = fixture(&root);
        let first = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(host) = first
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("expected first route");
        };
        let source_lua = root.join("mods/XHL-Unlimited-Gifting-Range/src/mod.lua");
        let updated_lua = b"print('canonical update')";
        fs::write(&source_lua, updated_lua).unwrap();
        let second = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(new_host) = second
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("expected route after canonical source edit");
        };
        assert_ne!(host.parent(), new_host.parent());
        assert_eq!(
            fs::read(
                new_host
                    .parent()
                    .unwrap()
                    .join("mods/XHL-Unlimited-Gifting-Range/src/mod.lua")
            )
            .unwrap(),
            updated_lua
        );
        assert_eq!(fs::read(&source_lua).unwrap(), updated_lua);
        remove_test_root(&root);
    }

    #[test]
    fn unknown_ini_file_is_not_carried_or_admitted_as_projection_input() {
        let root = test_root();
        let (contract, selections, plan) = fixture(&root);
        let first = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(host) = first
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("expected route");
        };
        let stale = host
            .parent()
            .unwrap()
            .join("mods/XHL-Remote-Altar-Close/settings.ini");
        fs::write(stale, b"[Plugin]\r\nEnabled=1\r\nDll=rogue.dll\r\n").unwrap();
        let second = prepare_fixture(&root, &selections, &plan, &contract);
        assert!(matches!(
            second.dispositions.get("dll:xhl-native-core:dinput8.dll"),
            Some(Disposition::Skip)
        ));
        assert!(!second.issues.is_empty());
        remove_test_root(&root);
    }

    #[test]
    fn changed_plugin_enabled_and_dll_fields_are_never_admitted() {
        for changed_ini in [
            b"[Plugin]\r\nEnabled=0\r\nDll=XHL-Remote-Altar-Close.dll\r\n".as_slice(),
            b"[Plugin]\r\nEnabled=1\r\nDll=rogue.dll\r\n".as_slice(),
            b"[Plugin]\r\nEnabled=1\r\nDll=..\\rogue.dll\r\n".as_slice(),
        ] {
            let root = test_root();
            let (contract, selections, plan) = fixture(&root);
            let remote_ini = root.join("mods/XHL-Remote-Altar-Close/native-plugin.ini");
            fs::write(&remote_ini, changed_ini).unwrap();
            let result = prepare_fixture(&root, &selections, &plan, &contract);
            let Disposition::Route(host) = result
                .dispositions
                .get("dll:xhl-native-core:dinput8.dll")
                .unwrap()
            else {
                panic!("the unchanged sibling should retain its independent admission");
            };
            let adjacent_mods = host.parent().unwrap().join("mods");
            assert!(!adjacent_mods.join("XHL-Remote-Altar-Close").exists());
            assert!(adjacent_mods.join("XHL-Unlimited-Gifting-Range").is_dir());
            assert_eq!(fs::read(&remote_ini).unwrap(), changed_ini);
            assert!(result.issues.iter().any(|(id, _)| {
                id.eq_ignore_ascii_case("dll:XHL-Remote-Altar-Close:XHL-Remote-Altar-Close.dll")
            }));
            assert!(matches!(
                result
                    .dispositions
                    .get("dll:XHL-Remote-Altar-Close:XHL-Remote-Altar-Close.dll"),
                Some(Disposition::Skip)
            ));
            remove_test_root(&root);
        }
    }

    #[test]
    fn public_load_plan_routes_host_and_withholds_admitted_plugin_entries() {
        let root = test_root();
        let (contract, selections) = public_fixture(&root);
        let plan = resolve_public_plan(&root, &selections, true);
        assert_eq!(plan.entries.len(), 3, "host plus two enabled XHL packages");
        let identity_before = mod_loader::dll_manager::load_plan_identity(&plan);
        let result = prepare_fixture(
            &root,
            &mod_loader::dll_manager::read_selections(&root),
            &plan,
            &contract,
        );
        assert_eq!(
            mod_loader::dll_manager::load_plan_identity(&plan),
            identity_before,
            "projection must not replace canonical plan paths"
        );
        let Disposition::Route(host) = result
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("expected the original host to route through the projection");
        };
        assert_ne!(host, &root.join("mods/xhl-native-core/dinput8.dll"));
        let projected_mods = host.parent().unwrap().join("mods");
        assert!(projected_mods.join("XHL-Remote-Altar-Close").is_dir());
        assert!(projected_mods.join("XHL-Unlimited-Gifting-Range").is_dir());
        for entry in plan.entries.iter().filter(|entry| {
            entry
                .package_directory
                .eq_ignore_ascii_case("XHL-Remote-Altar-Close")
                || entry
                    .package_directory
                    .eq_ignore_ascii_case("XHL-Unlimited-Gifting-Range")
        }) {
            assert!(matches!(
                result.dispositions.get(&entry.status_id),
                Some(Disposition::Skip)
            ));
        }
        remove_test_root(&root);
    }

    #[test]
    fn public_load_plan_requires_individual_approval_and_manager_enablement() {
        for (enabled, approved) in [(false, true), (true, false)] {
            let root = test_root();
            let (contract, mut selections) = public_fixture(&root);
            let gifting = selections
                .iter_mut()
                .find(|selection| {
                    selection
                        .package_directory
                        .eq_ignore_ascii_case("XHL-Unlimited-Gifting-Range")
                })
                .unwrap();
            gifting.enabled = enabled;
            gifting.approved = approved;
            let plan = resolve_public_plan(&root, &selections, true);
            assert!(!plan.entries.iter().any(|entry| {
                entry
                    .package_directory
                    .eq_ignore_ascii_case("XHL-Unlimited-Gifting-Range")
            }));
            let result = prepare_fixture(
                &root,
                &mod_loader::dll_manager::read_selections(&root),
                &plan,
                &contract,
            );
            let Disposition::Route(host) = result
                .dispositions
                .get("dll:xhl-native-core:dinput8.dll")
                .unwrap()
            else {
                panic!("the independently admitted Remote Altar package should route");
            };
            let projected_mods = host.parent().unwrap().join("mods");
            assert!(projected_mods.join("XHL-Remote-Altar-Close").is_dir());
            assert!(!projected_mods.join("XHL-Unlimited-Gifting-Range").exists());
            remove_test_root(&root);
        }

        let root = test_root();
        let (contract, selections) = public_fixture(&root);
        let plan = resolve_public_plan(&root, &selections, false);
        assert!(plan.entries.is_empty());
        let result = prepare_fixture(
            &root,
            &mod_loader::dll_manager::read_selections(&root),
            &plan,
            &contract,
        );
        assert!(result.dispositions.is_empty());
        assert!(
            !root
                .join("shroudforge/runtime/xhl-host-projections")
                .exists()
        );
        remove_test_root(&root);
    }

    #[test]
    fn public_load_plan_respects_target_manifest_and_dependency_gates() {
        for gate in ["target", "manifest", "dependency"] {
            let root = test_root();
            let (contract, mut selections) = public_fixture(&root);
            if gate == "target" {
                selections
                    .iter_mut()
                    .find(|selection| {
                        selection
                            .package_directory
                            .eq_ignore_ascii_case("XHL-Unlimited-Gifting-Range")
                    })
                    .unwrap()
                    .target = mod_loader::dll_manager::DllProfileTarget::Server;
            } else if gate == "manifest" {
                write_active_mod_manifests(&root, &["xhl-unlimited-gifting-range"], false);
            } else {
                write_active_mod_manifests(&root, &[], true);
            }
            let plan = resolve_public_plan(&root, &selections, true);
            assert!(
                !plan.entries.iter().any(|entry| {
                    entry
                        .package_directory
                        .eq_ignore_ascii_case("XHL-Unlimited-Gifting-Range")
                }),
                "gifting entry should be filtered by the {gate} gate"
            );
            let result = prepare_fixture(
                &root,
                &mod_loader::dll_manager::read_selections(&root),
                &plan,
                &contract,
            );
            let Disposition::Route(host) = result
                .dispositions
                .get("dll:xhl-native-core:dinput8.dll")
                .unwrap()
            else {
                panic!("Remote Altar should remain independently admitted for {gate}");
            };
            let projected_mods = host.parent().unwrap().join("mods");
            assert!(projected_mods.join("XHL-Remote-Altar-Close").is_dir());
            assert!(!projected_mods.join("XHL-Unlimited-Gifting-Range").exists());
            remove_test_root(&root);
        }
    }

    #[test]
    fn changed_canonical_plugin_ini_is_rejected_and_identical_snapshot_reuses_path() {
        let root = test_root();
        let (contract, selections) = public_fixture(&root);
        let ini = root.join("mods/XHL-Unlimited-Gifting-Range/native-plugin.ini");
        let changed_ini = b"[Plugin]\r\nEnabled=0\r\nDll=XHL-Unlimited-Gifting-Range.dll\r\n";
        fs::write(&ini, changed_ini).unwrap();
        let plan = resolve_public_plan(&root, &selections, true);
        assert!(
            plan.entries.iter().any(|entry| {
                entry
                    .package_directory
                    .eq_ignore_ascii_case("XHL-Unlimited-Gifting-Range")
            }),
            "manual selection is observable before projection validates package config"
        );
        let result = prepare_fixture(
            &root,
            &mod_loader::dll_manager::read_selections(&root),
            &plan,
            &contract,
        );
        let Disposition::Route(host) = result
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("the unchanged Remote Altar sibling should remain admitted");
        };
        let projected_mods = host.parent().unwrap().join("mods");
        assert!(projected_mods.join("XHL-Remote-Altar-Close").is_dir());
        assert!(!projected_mods.join("XHL-Unlimited-Gifting-Range").exists());
        assert_eq!(fs::read(&ini).unwrap(), changed_ini);
        assert!(result.issues.iter().any(|(id, _)| {
            id.eq_ignore_ascii_case(
                "dll:xhl-unlimited-gifting-range:xhl-unlimited-gifting-range.dll",
            )
        }));
        remove_test_root(&root);

        let root = test_root();
        let (contract, selections) = public_fixture(&root);
        let plan = resolve_public_plan(&root, &selections, true);
        let identity = mod_loader::dll_manager::load_plan_identity(&plan);
        let first = prepare_fixture(
            &root,
            &mod_loader::dll_manager::read_selections(&root),
            &plan,
            &contract,
        );
        let second = prepare_fixture(
            &root,
            &mod_loader::dll_manager::read_selections(&root),
            &plan,
            &contract,
        );
        let (Disposition::Route(first_host), Disposition::Route(second_host)) = (
            first
                .dispositions
                .get("dll:xhl-native-core:dinput8.dll")
                .unwrap(),
            second
                .dispositions
                .get("dll:xhl-native-core:dinput8.dll")
                .unwrap(),
        ) else {
            panic!("identical canonical inputs should keep a valid host projection");
        };
        assert_eq!(first_host, second_host);
        assert_eq!(mod_loader::dll_manager::load_plan_identity(&plan), identity);
        remove_test_root(&root);
    }

    #[test]
    fn canonical_settings_ini_is_hash_bound_and_projection_edits_do_not_carry_forward() {
        let root = test_root();
        let (contract, selections, plan) = fixture(&root);
        let source_ini = root.join("mods/XHL-Remote-Altar-Close/settings.ini");
        let canonical_initial = b"[UserSettings]\r\nValue=canonical-initial\r\n";
        let generated_edit = b"[UserSettings]\r\nValue=projection-output\r\n";
        let canonical_updated = b"[UserSettings]\r\nValue=canonical-updated\r\n";
        fs::write(&source_ini, canonical_initial).unwrap();
        let first = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(first_host) = first
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("the admitted package with a canonical settings file should route");
        };
        let first_projection = first_host.parent().unwrap().to_path_buf();
        let projected_ini = first_projection.join("mods/XHL-Remote-Altar-Close/settings.ini");
        assert_eq!(fs::read(&projected_ini).unwrap(), canonical_initial);
        assert_eq!(fs::read(&source_ini).unwrap(), canonical_initial);

        fs::write(&projected_ini, generated_edit).unwrap();
        let reused = prepare_fixture(&root, &selections, &plan, &contract);
        assert!(matches!(
            reused.dispositions.get("dll:xhl-native-core:dinput8.dll"),
            Some(Disposition::Skip)
        ));
        assert!(!reused.issues.is_empty());
        assert_eq!(fs::read(&source_ini).unwrap(), canonical_initial);

        fs::write(&source_ini, canonical_updated).unwrap();
        let updated = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(updated_host) = updated
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            remove_test_root(&root);
            panic!("an updated canonical source should create a fresh snapshot");
        };
        let updated_projection = updated_host.parent().unwrap();
        assert_ne!(updated_projection, first_projection.as_path());
        assert_eq!(
            fs::read(updated_projection.join("mods/XHL-Remote-Altar-Close/settings.ini")).unwrap(),
            canonical_updated
        );
        assert_eq!(fs::read(&source_ini).unwrap(), canonical_updated);
        remove_test_root(&root);
    }

    #[test]
    fn host_root_log_is_output_only_across_reuse_and_new_snapshots() {
        let root = test_root();
        let (contract, selections, plan) = fixture(&root);
        let canonical_log = root.join("mods").join(HOST_DIRECTORY).join(HOST_LOG_FILE);
        fs::write(&canonical_log, b"old canonical host log").unwrap();

        let first = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(host) = first
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("expected route");
        };
        let first_host = host.clone();
        let first_projection = first_host.parent().unwrap().to_path_buf();
        let projected_log = first_projection.join(HOST_LOG_FILE);
        assert!(
            !projected_log.exists(),
            "canonical log output is not copied"
        );

        fs::write(&projected_log, b"first host output").unwrap();
        let reused = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(reused_host) = reused
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("the exact root host log should not invalidate a reused projection");
        };
        assert_eq!(reused_host, &first_host);

        fs::write(&projected_log, b"second host output with different bytes").unwrap();
        let updated_log = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(updated_log_host) = updated_log
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("log bytes must not alter the canonical snapshot identity");
        };
        assert_eq!(updated_log_host, &first_host);
        assert_eq!(
            fs::read(&projected_log).unwrap(),
            b"second host output with different bytes"
        );

        let source_lua = root.join("mods/XHL-Unlimited-Gifting-Range/src/mod.lua");
        fs::write(&source_lua, b"print('canonical source update')").unwrap();
        let new_snapshot = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(new_host) = new_snapshot
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("canonical source changes should produce a new projection");
        };
        assert_ne!(new_host.parent(), Some(first_projection.as_path()));
        assert!(
            !new_host.parent().unwrap().join(HOST_LOG_FILE).exists(),
            "a prior projection log must not enter a new snapshot"
        );
        assert!(
            !fs::read_to_string(new_host.parent().unwrap().join(PROJECTION_MARKER))
                .unwrap()
                .contains(HOST_LOG_FILE),
            "host log output must not enter the snapshot manifest or fingerprint"
        );
        assert_eq!(fs::read(&canonical_log).unwrap(), b"old canonical host log");
        remove_test_root(&root);
    }

    #[test]
    fn host_log_lookalikes_and_nested_outputs_are_rejected() {
        for relative in [
            format!("{HOST_LOG_FILE}.bak"),
            format!("mods/XHL-Remote-Altar-Close/{HOST_LOG_FILE}"),
        ] {
            let root = test_root();
            let (contract, selections, plan) = fixture(&root);
            let first = prepare_fixture(&root, &selections, &plan, &contract);
            let Disposition::Route(host) = first
                .dispositions
                .get("dll:xhl-native-core:dinput8.dll")
                .unwrap()
            else {
                panic!("expected first route");
            };
            let extra = host
                .parent()
                .unwrap()
                .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Some(parent) = extra.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&extra, b"not the recognized root output").unwrap();

            let second = prepare_fixture(&root, &selections, &plan, &contract);
            assert!(matches!(
                second.dispositions.get("dll:xhl-native-core:dinput8.dll"),
                Some(Disposition::Skip)
            ));
            assert!(!second.issues.is_empty());
            remove_test_root(&root);
        }
    }

    #[test]
    fn hardlinked_root_host_log_is_rejected() {
        let root = test_root();
        let (contract, selections, plan) = fixture(&root);
        let first = prepare_fixture(&root, &selections, &plan, &contract);
        let Disposition::Route(host) = first
            .dispositions
            .get("dll:xhl-native-core:dinput8.dll")
            .unwrap()
        else {
            panic!("expected first route");
        };
        let projection = host.parent().unwrap();
        let source = root.join("host-log-hardlink-source.bin");
        let linked_log = projection.join(HOST_LOG_FILE);
        fs::write(&source, b"linked output").unwrap();
        fs::hard_link(&source, &linked_log).unwrap();

        let second = prepare_fixture(&root, &selections, &plan, &contract);
        assert!(matches!(
            second.dispositions.get("dll:xhl-native-core:dinput8.dll"),
            Some(Disposition::Skip)
        ));
        assert!(!second.issues.is_empty());
        remove_test_root(&root);
    }
}
