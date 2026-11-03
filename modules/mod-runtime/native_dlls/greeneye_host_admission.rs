//! Narrow, startup-only admission for the unchanged Greeneye Find host.
//!
//! The DLL Manager remains the authority for package selection. This module
//! only withholds Find's direct DLL entries, validates the selected original
//! payload, and lets the ordered process worker load the host from its package.

use mod_loader::dll_manager::{DllLoadEntry, DllLoadMode, DllLoadPlan, DllProfile, DllProfileFile};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use uuid::Uuid;

const HOST_PROFILE_ID: &str = "greeneye-find-host";
const HOST_PROFILE_NAME: &str = "Greeneye Find Host (Original)";
const HOST_PACKAGE_DIRECTORY: &str = "greeneye-host";
const HOST_FILE: &str = "dbgcore.dll";
const FIND_PROFILE_ID: &str = "find-mouse-alignment";
const FIND_PROFILE_NAME: &str = "Find Mouse Alignment (Original)";
const FIND_PACKAGE_DIRECTORY: &str = "find";
const FIND_MOD_ID: &str = "find";
const FIND_DLL_PATH: &str = "greeneye/find/find.dll";
const FIND_WRAPPER: &str = "find.dll";
const FIND_UPSTREAM: &str = "find_upstream.dll";
const FIND_MANIFEST: &str = "mod.json";
const ROOT_HOST_LOG: &str = "loader.log";
const LEGACY_ROOT_INI: &str = "find.ini";

const HOST_SHA256: &str = "d4a253e0158a81fdd5c463dc37622174841db9e18c7fdd8dbe443f0e55368df4";
const FIND_WRAPPER_SHA256: &str =
    "a1fe81fed6888a3d69dcf490ef59a34afaa3e7beccb330dc7b7e2b19a203b937";
const FIND_UPSTREAM_SHA256: &str =
    "82ba715080195ac8aeceac80f341cb4466991278e21b8a24ebb836a3f2be18c5";
const FIND_MANIFEST_SHA256: &str =
    "2f2fe38d2ff009faf479fdc354801737a3ede718a6e3f3a07fe83be3b5c063f6";

const MAX_IMMUTABLE_FILE_BYTES: u64 = 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
const MAX_USER_INI_BYTES: u64 = 1024 * 1024;
const MAX_USER_LOG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RUNTIME_BYTES: u64 = 34 * 1024 * 1024;
const MAX_RUNTIME_FILES: usize = 6;
const HOST_STATUS_FALLBACK: &str = "dll:greeneye-host:dbgcore.dll";
const FIND_STATUS_FALLBACK: &str = "dll:find:greeneye/find/find.dll";

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProfileContract {
    host_profile_id: String,
    host_directory: String,
    find_profile_id: String,
    find_directory: String,
    find_mod_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HashPins {
    host: String,
    wrapper: String,
    upstream: String,
    manifest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AdmissionContract {
    profile: ProfileContract,
    hashes: HashPins,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceFile {
    name: String,
    bytes: Vec<u8>,
    sha256: String,
}

struct AdmissionContext {
    root: PathBuf,
    host_path: PathBuf,
    host_status_id: String,
    find_status_id: String,
    queue_permit: bool,
    contract: AdmissionContract,
    plan_identity: serde_json::Value,
    plan_issues: serde_json::Value,
    selection_identity: serde_json::Value,
    runtime_directory: PathBuf,
    // These handles deny writes and deletion of immutable runtime payload files
    // until process exit. User configuration and logs are never leased.
    _immutable_leases: Vec<File>,
}

static ADMISSIONS: OnceLock<Mutex<HashMap<String, AdmissionContext>>> = OnceLock::new();

/// Withhold Greeneye/Find entries on every route. Only the held-entry startup
/// route may stage the absent runtime directory or route the original host.
pub(super) fn prepare(
    root: &Path,
    is_server: bool,
    startup_held_entry: bool,
    plan: &DllLoadPlan,
) -> Prepared {
    let mut prepared = mark_reserved_entries(plan, None);
    let (host_entries, find_entries) = classified_entries(plan, None);
    if host_entries.is_empty() && find_entries.is_empty() {
        return prepared;
    }
    if !startup_held_entry {
        add_issues(
            &mut prepared,
            &host_entries,
            &find_entries,
            "Greeneye Find admission requires the startup-held game entrypoint. Change DLL Manager selections and restart the game".into(),
        );
        return prepared;
    }
    if is_server {
        add_issues(
            &mut prepared,
            &host_entries,
            &find_entries,
            "The original Greeneye Find host is client-only and was refused for this process"
                .into(),
        );
        return prepared;
    }

    let contract = match production_contract(root) {
        Ok(contract) => contract,
        Err(error) => {
            add_issues(&mut prepared, &host_entries, &find_entries, error);
            return prepared;
        }
    };
    let selections = mod_loader::dll_manager::read_selections(root);
    match prepare_with_contract(root, plan, &selections, &contract, &mut prepared) {
        Ok(()) => prepared,
        Err(error) => {
            add_issues(&mut prepared, &host_entries, &find_entries, error);
            prepared
        }
    }
}

/// Queue policy blocks DLL Manager or Lua registration paths from loading a
/// known Greeneye host or Find payload without the startup admission result.
pub(super) fn check_queue(root: &Path, entry: &DllLoadEntry) -> Result<(), String> {
    match reserved_kind(root, entry)? {
        ReservedKind::None => Ok(()),
        ReservedKind::FindPayload => Err(
            "The Find wrapper and upstream core are owned by Greeneye admission and cannot be loaded directly. Restart the game after approving both profiles in DLL Manager".into(),
        ),
        ReservedKind::Host => {
            let root = canonical_root(root)?;
            let key = root_key(&root);
            let mut admissions = admissions()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let context = admissions.get_mut(&key).ok_or_else(|| {
                "The Greeneye host was not admitted during startup. Change both DLL Manager profiles and restart the game".to_owned()
            })?;
            if context.host_status_id != entry.status_id
                || !same_path(&context.host_path, &entry.path)
                || !same_path(&context.root, &root)
            {
                return Err(
                    "The selected Greeneye host differs from its startup admission. Restart the game".into(),
                );
            }
            if !context.queue_permit {
                return Err(
                    "The Greeneye host may only be queued by its startup admission. Restart the game".into(),
                );
            }
            context.queue_permit = false;
            Ok(())
        }
    }
}

/// Grant a one-shot queue permit only to the route produced by startup
/// admission. Direct Lua registrations cannot create this permit.
pub(super) fn authorize_host_queue(root: &Path, entry: &DllLoadEntry) -> Result<(), String> {
    if !matches!(reserved_kind(root, entry)?, ReservedKind::Host) {
        return Err("Only the admitted Greeneye host can receive a startup queue permit".into());
    }
    let root = canonical_root(root)?;
    let key = root_key(&root);
    let mut admissions = admissions()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let context = admissions.get_mut(&key).ok_or_else(|| {
        "The Greeneye host was not admitted during startup. Restart the game".to_owned()
    })?;
    if context.host_status_id != entry.status_id
        || !same_path(&context.host_path, &entry.path)
        || !same_path(&context.root, &root)
    {
        return Err(
            "The Greeneye host differs from its startup admission. Restart the game".into(),
        );
    }
    if context.queue_permit {
        return Err("The Greeneye host already has a pending startup queue permit".into());
    }
    context.queue_permit = true;
    Ok(())
}

pub(super) fn cancel_host_queue_authorization(root: &Path, entry: &DllLoadEntry) {
    let Ok(root) = canonical_root(root) else {
        return;
    };
    let mut admissions = admissions()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(context) = admissions.get_mut(&root_key(&root)) {
        if context.host_status_id == entry.status_id && same_path(&context.host_path, &entry.path) {
            context.queue_permit = false;
        }
    }
}

/// Recheck admission immediately before LoadLibraryExW on the ordered worker.
/// Non-Greeneye entries pass through unchanged.
pub(super) fn before_host_load(root: &Path, entry: &DllLoadEntry) -> Result<(), String> {
    if !matches!(reserved_kind(root, entry)?, ReservedKind::Host) {
        return Ok(());
    }

    let root = canonical_root(root)?;
    let _lock = AdmissionLock::acquire(&root)?;
    let key = root_key(&root);
    let admissions = admissions()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let context = admissions.get(&key).ok_or_else(|| {
        "The Greeneye host has no successful startup admission. Restart the game".to_owned()
    })?;
    if context.host_status_id != entry.status_id
        || !same_path(&context.host_path, &entry.path)
        || !same_path(&context.root, &root)
    {
        return Err(
            "The Greeneye host path changed after startup admission. Restart the game".into(),
        );
    }

    ensure_root_guards(&root)?;
    ensure_no_other_game_process(&root)?;
    validate_profile_identity(&root, &context.contract.profile)?;
    let (_, find_status_id) = validate_current_settings(
        &root,
        &context.contract,
        &context.selection_identity,
        &context.plan_identity,
        &context.plan_issues,
    )?;
    if find_status_id != context.find_status_id {
        return Err(
            "The selected Find entry changed after Greeneye startup admission. Restart the game"
                .into(),
        );
    }
    validate_host_file(&root, &context.contract, &context.host_path)?;
    let source = read_find_source(&root, &context.contract)?;
    verify_runtime_tree(&root, &context.runtime_directory, &context.contract.hashes)?;
    if source.len() != 3 {
        return Err(
            "The admitted Find source no longer contains exactly its three immutable payload files"
                .into(),
        );
    }
    Ok(())
}

fn prepare_with_contract(
    root: &Path,
    plan: &DllLoadPlan,
    selections: &[DllProfile],
    contract: &AdmissionContract,
    prepared: &mut Prepared,
) -> Result<(), String> {
    let root = canonical_root(root)?;
    let _lock = AdmissionLock::acquire(&root)?;
    ensure_root_guards(&root)?;
    ensure_no_other_game_process(&root)?;
    validate_profile_identity(&root, &contract.profile)?;

    let (host_entries, find_entries) = classified_entries(plan, Some(&contract.profile));
    if host_entries.is_empty() {
        return Err(
            "The Greeneye host is not present in the canonical DLL Manager load plan".into(),
        );
    }
    let host_selection = admitted_selection(
        selections,
        &contract.profile.host_directory,
        &contract.profile.host_profile_id,
        "Greeneye host",
    )?;
    let find_selection = admitted_selection(
        selections,
        &contract.profile.find_directory,
        &contract.profile.find_profile_id,
        "Find",
    )?;
    let selection_identity = selection_identity(host_selection, find_selection)?;
    let (host_entry, find_entry) =
        validate_plan_entries(&root, &contract.profile, &host_entries, &find_entries)?;
    validate_host_file(&root, contract, &host_entry.path)?;
    let source = read_find_source(&root, contract)?;
    let plan_identity = mod_loader::dll_manager::load_plan_identity(plan);
    let plan_issues = serde_json::to_value(&plan.issues).map_err(|error| error.to_string())?;

    let runtime_directory = root.join("greeneye");
    match fs::symlink_metadata(&runtime_directory) {
        Ok(_) => verify_runtime_tree(&root, &runtime_directory, &contract.hashes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            publish_absent_tree(&root, &runtime_directory, &source, &contract.hashes, || {
                ensure_root_guards(&root)?;
                ensure_no_other_game_process(&root)?;
                validate_profile_identity(&root, &contract.profile)?;
                validate_current_settings(
                    &root,
                    contract,
                    &selection_identity,
                    &plan_identity,
                    &plan_issues,
                )?;
                validate_host_file(&root, contract, &host_entry.path)?;
                let current_source = read_find_source(&root, contract)?;
                if current_source != source {
                    return Err(
                        "The canonical Find package changed while its runtime files were staged"
                            .into(),
                    );
                }
                Ok(())
            })?;
        }
        Err(error) => {
            return Err(format!(
                "Could not inspect game/greeneye without changing it: {error}"
            ));
        }
    }

    ensure_root_guards(&root)?;
    ensure_no_other_game_process(&root)?;
    validate_current_settings(
        &root,
        contract,
        &selection_identity,
        &plan_identity,
        &plan_issues,
    )?;
    validate_host_file(&root, contract, &host_entry.path)?;
    verify_runtime_tree(&root, &runtime_directory, &contract.hashes)?;
    let immutable_leases = open_immutable_leases(&runtime_directory, &contract.hashes)?;

    let context = AdmissionContext {
        root: root.clone(),
        host_path: host_entry.path.clone(),
        host_status_id: host_entry.status_id.clone(),
        find_status_id: find_entry.status_id.clone(),
        queue_permit: false,
        contract: contract.clone(),
        plan_identity,
        plan_issues,
        selection_identity,
        runtime_directory,
        _immutable_leases: immutable_leases,
    };
    admissions()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(root_key(&root), context);

    prepared.dispositions.insert(
        host_entry.status_id.clone(),
        Disposition::Route(host_entry.path.clone()),
    );
    prepared
        .dispositions
        .insert(find_entry.status_id.clone(), Disposition::Skip);
    Ok(())
}

fn production_contract(root: &Path) -> Result<AdmissionContract, String> {
    let profiles = mod_loader::mod_profiles::read_profiles(root).map_err(|errors| {
        format!(
            "Could not read Greeneye DLL profiles: {}",
            errors.join("; ")
        )
    })?;
    let host = profiles
        .iter()
        .find(|profile| profile.id == HOST_PROFILE_ID)
        .ok_or_else(|| format!("Required DLL Manager profile '{HOST_PROFILE_ID}' is missing"))?;
    let find = profiles
        .iter()
        .find(|profile| profile.id == FIND_PROFILE_ID)
        .ok_or_else(|| format!("Required DLL Manager profile '{FIND_PROFILE_ID}' is missing"))?;
    validate_profile(
        host,
        HOST_PROFILE_ID,
        HOST_PROFILE_NAME,
        HOST_PACKAGE_DIRECTORY,
        "",
        HOST_FILE,
        &[("native_runtime/find/dbgcore.dll", HOST_FILE)],
    )?;
    validate_profile(
        find,
        FIND_PROFILE_ID,
        FIND_PROFILE_NAME,
        FIND_PACKAGE_DIRECTORY,
        FIND_MOD_ID,
        FIND_DLL_PATH,
        &[
            ("native_runtime/find/greeneye/find/find.dll", FIND_DLL_PATH),
            (
                "native_runtime/find/greeneye/find/find_upstream.dll",
                "greeneye/find/find_upstream.dll",
            ),
            (
                "native_runtime/find/greeneye/find/mod.json",
                "greeneye/find/mod.json",
            ),
        ],
    )?;
    Ok(AdmissionContract {
        profile: ProfileContract {
            host_profile_id: HOST_PROFILE_ID.into(),
            host_directory: HOST_PACKAGE_DIRECTORY.into(),
            find_profile_id: FIND_PROFILE_ID.into(),
            find_directory: FIND_PACKAGE_DIRECTORY.into(),
            find_mod_id: FIND_MOD_ID.into(),
        },
        hashes: HashPins {
            host: HOST_SHA256.into(),
            wrapper: FIND_WRAPPER_SHA256.into(),
            upstream: FIND_UPSTREAM_SHA256.into(),
            manifest: FIND_MANIFEST_SHA256.into(),
        },
    })
}

fn validate_profile(
    profile: &mod_loader::mod_profiles::ModCompatibilityProfile,
    expected_id: &str,
    expected_name: &str,
    expected_directory: &str,
    expected_mod_id: &str,
    expected_dll: &str,
    imports: &[(&str, &str)],
) -> Result<(), String> {
    let exact_client_target = profile.target.applies_to(false) && !profile.target.applies_to(true);
    let valid_file = profile.files.len() == 1
        && profile.files[0].path == expected_dll
        && profile.files[0].mode == DllLoadMode::Library;
    let valid_imports = profile.import_files.len() == imports.len()
        && imports.iter().all(|(source, destination)| {
            profile
                .import_files
                .get(*source)
                .is_some_and(|value| value == destination)
        });
    if profile.id != expected_id
        || profile.schema_version != 1
        || profile.name != expected_name
        || !profile
            .package_directory
            .eq_ignore_ascii_case(expected_directory)
        || profile.mod_id != expected_mod_id
        || !exact_client_target
        || !valid_file
        || !valid_imports
        || profile.mod_json.is_some()
        || profile.extended_mod_json.is_some()
        || !profile.package_files.is_empty()
    {
        return Err(format!(
            "DLL Manager profile '{expected_id}' does not match the fixed client-only Greeneye/Find package contract"
        ));
    }
    Ok(())
}

fn validate_profile_identity(root: &Path, expected: &ProfileContract) -> Result<(), String> {
    let actual = production_contract(root)?.profile;
    if &actual != expected {
        return Err("The installed Greeneye/Find DLL profile identity changed. Restart after restoring the reviewed profiles".into());
    }
    Ok(())
}

fn classified_entries<'a>(
    plan: &'a DllLoadPlan,
    profile: Option<&ProfileContract>,
) -> (Vec<&'a DllLoadEntry>, Vec<&'a DllLoadEntry>) {
    let host_directory = profile.map_or(HOST_PACKAGE_DIRECTORY, |profile| {
        profile.host_directory.as_str()
    });
    let find_directory = profile.map_or(FIND_PACKAGE_DIRECTORY, |profile| {
        profile.find_directory.as_str()
    });
    let find_mod_id = profile.map_or(FIND_MOD_ID, |profile| profile.find_mod_id.as_str());
    let mut hosts = Vec::new();
    let mut finds = Vec::new();
    for entry in &plan.entries {
        if entry.package_directory.eq_ignore_ascii_case(host_directory) {
            hosts.push(entry);
        }
        if entry.package_directory.eq_ignore_ascii_case(find_directory)
            || entry
                .mod_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(find_mod_id))
        {
            finds.push(entry);
        }
    }
    (hosts, finds)
}

fn mark_reserved_entries(plan: &DllLoadPlan, profile: Option<&ProfileContract>) -> Prepared {
    let (hosts, finds) = classified_entries(plan, profile);
    let mut prepared = Prepared::default();
    for entry in hosts.into_iter().chain(finds) {
        prepared
            .dispositions
            .insert(entry.status_id.clone(), Disposition::Skip);
    }
    prepared
}

fn admitted_selection<'a>(
    selections: &'a [DllProfile],
    directory: &str,
    profile_id: &str,
    label: &str,
) -> Result<&'a DllProfile, String> {
    let mut matches = selections
        .iter()
        .filter(|selection| selection.package_directory.eq_ignore_ascii_case(directory));
    let selection = matches.next().ok_or_else(|| {
        format!(
            "The {label} requires its own enabled and approved '{profile_id}' DLL Manager selection"
        )
    })?;
    if matches.next().is_some()
        || !selection.enabled
        || !selection.approved
        || !selection.target.applies_to(false)
        || selection.target.applies_to(true)
        || selection.compatibility_profile.as_deref() != Some(profile_id)
    {
        return Err(format!(
            "The {label} selection must use profile '{profile_id}', be enabled and approved, and target client only"
        ));
    }
    Ok(selection)
}

fn selection_identity(host: &DllProfile, find: &DllProfile) -> Result<serde_json::Value, String> {
    serde_json::to_value((host, find)).map_err(|error| error.to_string())
}

fn validate_plan_entries<'a>(
    root: &Path,
    profile: &ProfileContract,
    host_entries: &[&'a DllLoadEntry],
    find_entries: &[&'a DllLoadEntry],
) -> Result<(&'a DllLoadEntry, &'a DllLoadEntry), String> {
    let host_file = DllProfileFile {
        path: HOST_FILE.into(),
        mode: DllLoadMode::Library,
    };
    let find_file = DllProfileFile {
        path: FIND_DLL_PATH.into(),
        mode: DllLoadMode::Library,
    };
    let expected_host = mod_loader::dll_manager::resolve_package_dll_path(
        root,
        &profile.host_directory,
        &host_file,
    )?;
    let expected_find = mod_loader::dll_manager::resolve_package_dll_path(
        root,
        &profile.find_directory,
        &find_file,
    )?;
    let host_candidates: Vec<_> = host_entries
        .iter()
        .copied()
        .filter(|entry| entry.source != "chain")
        .collect();
    let find_candidates: Vec<_> = find_entries
        .iter()
        .copied()
        .filter(|entry| entry.source != "chain")
        .collect();
    if host_candidates.len() != 1 {
        return Err(format!(
            "The Greeneye package has {} non-chain DLL entries. Exactly its selected original {HOST_FILE} library is required",
            host_candidates.len()
        ));
    }
    if find_candidates.len() != 1 {
        return Err(format!(
            "The Find package has {} non-chain DLL entries. Exactly its selected wrapper library is required",
            find_candidates.len()
        ));
    }
    let host = host_candidates[0];
    let find = find_candidates[0];
    if !host
        .package_directory
        .eq_ignore_ascii_case(&profile.host_directory)
        || host.mode != DllLoadMode::Library
        || !same_path(&host.path, &expected_host)
    {
        return Err(format!(
            "The Greeneye host must resolve to mods/{}/{HOST_FILE} in ordinary library mode",
            profile.host_directory
        ));
    }
    if !find
        .package_directory
        .eq_ignore_ascii_case(&profile.find_directory)
        || find
            .mod_id
            .as_deref()
            .is_some_and(|id| !id.eq_ignore_ascii_case(&profile.find_mod_id))
        || find.mode != DllLoadMode::Library
        || !same_path(&find.path, &expected_find)
    {
        return Err(format!(
            "Find must pass the canonical mod, target, dependency, filename-filter and path gates for mods/{}/{}",
            profile.find_directory, FIND_DLL_PATH
        ));
    }
    Ok((host, find))
}

fn add_issues(
    prepared: &mut Prepared,
    hosts: &[&DllLoadEntry],
    finds: &[&DllLoadEntry],
    detail: String,
) {
    let mut ids = HashSet::new();
    for entry in hosts.iter().chain(finds) {
        ids.insert(entry.status_id.clone());
    }
    if ids.is_empty() {
        ids.insert(HOST_STATUS_FALLBACK.into());
        ids.insert(FIND_STATUS_FALLBACK.into());
    }
    for id in ids {
        prepared.issues.push((id, detail.clone()));
    }
}

fn ensure_root_guards(root: &Path) -> Result<(), String> {
    require_absent(&root.join("dbgcore.dll"), "game-root dbgcore.dll")?;
    require_absent(
        &root.join(LEGACY_ROOT_INI),
        "legacy game-root find.ini; its ownership and unknown settings are not migrated",
    )?;
    Ok(())
}

fn require_absent(path: &Path, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!(
            "Greeneye Find admission refused because {label} is present. No files were changed"
        )),
        Err(error) => Err(format!("Could not verify that {label} is absent: {error}")),
    }
}

fn validate_current_settings(
    root: &Path,
    contract: &AdmissionContract,
    expected_selection: &serde_json::Value,
    expected_plan: &serde_json::Value,
    expected_issues: &serde_json::Value,
) -> Result<(String, String), String> {
    if !super::manager_enabled(root)? {
        return Err(
            "DLL Manager was disabled while Greeneye Find admission was being prepared".into(),
        );
    }
    let selections = mod_loader::dll_manager::read_selections(root);
    let host = admitted_selection(
        &selections,
        &contract.profile.host_directory,
        &contract.profile.host_profile_id,
        "Greeneye host",
    )?;
    let find = admitted_selection(
        &selections,
        &contract.profile.find_directory,
        &contract.profile.find_profile_id,
        "Find",
    )?;
    if selection_identity(host, find)? != *expected_selection {
        return Err("DLL Manager settings changed while Greeneye Find admission was being prepared. Restart after reviewing both selections".into());
    }
    let current = mod_loader::dll_manager::resolve_load_plan(root, false);
    let identity = mod_loader::dll_manager::load_plan_identity(&current);
    let issues = serde_json::to_value(&current.issues).map_err(|error| error.to_string())?;
    if identity != *expected_plan || issues != *expected_issues {
        return Err("The canonical DLL Manager load plan changed while Greeneye Find admission was being prepared. Restart after reviewing package dependencies and filters".into());
    }
    let (host_entries, find_entries) = classified_entries(&current, Some(&contract.profile));
    let (host, find) =
        validate_plan_entries(root, &contract.profile, &host_entries, &find_entries)?;
    if mod_loader::dll_manager::is_excluded_dll_name(root, FIND_UPSTREAM) {
        return Err(format!(
            "Find's required sidecar {FIND_UPSTREAM} is excluded by the DLL Manager filename filter"
        ));
    }
    Ok((host.status_id.clone(), find.status_id.clone()))
}

fn validate_host_file(
    root: &Path,
    contract: &AdmissionContract,
    path: &Path,
) -> Result<(), String> {
    let expected = mod_loader::dll_manager::resolve_package_dll_path(
        root,
        &contract.profile.host_directory,
        &DllProfileFile {
            path: HOST_FILE.into(),
            mode: DllLoadMode::Library,
        },
    )?;
    if !same_path(path, &expected) {
        return Err("The selected Greeneye host path is outside its admitted package".into());
    }
    let file = read_checked_file(path, false, MAX_IMMUTABLE_FILE_BYTES)?;
    if file.sha256 != contract.hashes.host {
        return Err("The original Greeneye host hash differs from the reviewed payload. The host was not loaded".into());
    }
    Ok(())
}

fn read_find_source(root: &Path, contract: &AdmissionContract) -> Result<Vec<SourceFile>, String> {
    let mods = mod_loader::paths::mods_dir(root);
    let package = mods.join(&contract.profile.find_directory);
    validate_real_directory(root, &mods)?;
    validate_real_directory(root, &package)?;
    let greeneye = package.join("greeneye");
    validate_real_directory(root, &greeneye)?;
    exact_names(&greeneye, &["find"])?;
    let source = greeneye.join("find");
    validate_real_directory(root, &source)?;
    exact_names(&source, &[FIND_WRAPPER, FIND_UPSTREAM, FIND_MANIFEST])?;

    let expected = [
        (FIND_WRAPPER, contract.hashes.wrapper.as_str()),
        (FIND_UPSTREAM, contract.hashes.upstream.as_str()),
        (FIND_MANIFEST, contract.hashes.manifest.as_str()),
    ];
    let mut files = Vec::with_capacity(expected.len());
    let mut total = 0u64;
    for (name, hash) in expected {
        let file = read_checked_file(&source.join(name), false, MAX_IMMUTABLE_FILE_BYTES)?;
        total = total.saturating_add(file.bytes.len() as u64);
        if file.sha256 != hash {
            return Err(format!(
                "Canonical Find source file {name} does not match its reviewed original hash"
            ));
        }
        files.push(SourceFile {
            name: name.into(),
            bytes: file.bytes,
            sha256: file.sha256,
        });
    }
    if total > MAX_SOURCE_BYTES {
        return Err("Canonical Find source exceeds the bounded 1 MiB payload limit".into());
    }
    Ok(files)
}

fn publish_absent_tree<F>(
    root: &Path,
    destination: &Path,
    source: &[SourceFile],
    hashes: &HashPins,
    recheck: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    if path_exists(destination)? {
        return Err("game/greeneye appeared during admission. It was left untouched".into());
    }
    let staging = root.join(format!(
        ".shroudforge-greeneye-find-staging-{}",
        Uuid::new_v4()
    ));
    fs::create_dir(&staging)
        .map_err(|error| format!("Could not create private Greeneye staging directory: {error}"))?;
    let mut cleanup = StagingDirectory::new(staging.clone());
    let find_dir = staging.join("find");
    fs::create_dir(&find_dir)
        .map_err(|error| format!("Could not create the bounded Find staging folder: {error}"))?;
    for file in source {
        write_new_file(&find_dir.join(&file.name), &file.bytes)?;
    }
    verify_runtime_tree(root, &staging, hashes)?;
    recheck()?;
    if path_exists(destination)? {
        return Err(
            "game/greeneye appeared before atomic publication. It was left untouched".into(),
        );
    }
    rename_absent(&staging, destination)?;
    cleanup.disarm();
    verify_runtime_tree(root, destination, hashes)
}

fn verify_runtime_tree(root: &Path, directory: &Path, hashes: &HashPins) -> Result<(), String> {
    validate_real_directory(root, directory)?;
    exact_names(directory, &["find"])?;
    let root_names = entry_names(directory)?;
    if root_names.len() != 1 && root_names.len() != 2 {
        return Err(
            "game/greeneye contains unexpected files or directories. Its bytes were left untouched"
                .into(),
        );
    }
    if root_names
        .iter()
        .any(|name| name != "find" && name != ROOT_HOST_LOG)
    {
        return Err(
            "game/greeneye contains an unapproved child. Its bytes were left untouched".into(),
        );
    }
    let find_dir = directory.join("find");
    validate_real_directory(root, &find_dir)?;
    let find_names = entry_names(&find_dir)?;
    let required: HashSet<_> = [FIND_WRAPPER, FIND_UPSTREAM, FIND_MANIFEST]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let allowed: HashSet<_> = [
        FIND_WRAPPER,
        FIND_UPSTREAM,
        FIND_MANIFEST,
        "find.ini",
        "find.log",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if !required.is_subset(&find_names) || !find_names.is_subset(&allowed) {
        return Err("game/greeneye/find contains a missing original payload or an unapproved file. Its bytes were left untouched".into());
    }
    let root_file_count = if root_names.contains(ROOT_HOST_LOG) {
        1
    } else {
        0
    };
    if find_names.len() > 5 || root_file_count + find_names.len() > MAX_RUNTIME_FILES {
        return Err(
            "game/greeneye exceeds the bounded file count. Its bytes were left untouched".into(),
        );
    }

    let expected = [
        (FIND_WRAPPER, hashes.wrapper.as_str()),
        (FIND_UPSTREAM, hashes.upstream.as_str()),
        (FIND_MANIFEST, hashes.manifest.as_str()),
    ];
    let mut total = 0u64;
    for (name, hash) in expected {
        let file = read_checked_file(&find_dir.join(name), false, MAX_IMMUTABLE_FILE_BYTES)?;
        if file.sha256 != hash {
            return Err(format!(
                "game/greeneye/find/{name} differs from its reviewed original hash. Its bytes were left untouched"
            ));
        }
        total = total.saturating_add(file.bytes.len() as u64);
    }
    for name in [&ROOT_HOST_LOG, &"find.ini", &"find.log"] {
        let path = if *name == ROOT_HOST_LOG {
            directory.join(name)
        } else {
            find_dir.join(name)
        };
        if path_exists(&path)? {
            let limit = if *name == "find.ini" {
                MAX_USER_INI_BYTES
            } else {
                MAX_USER_LOG_BYTES
            };
            let file = inspect_regular_file(&path, limit)?;
            total = total.saturating_add(file.len());
        }
    }
    if total > MAX_RUNTIME_BYTES {
        return Err("game/greeneye exceeds the bounded 34 MiB file-size limit. Its bytes were left untouched".into());
    }
    Ok(())
}

fn open_immutable_leases(directory: &Path, hashes: &HashPins) -> Result<Vec<File>, String> {
    let find_dir = directory.join("find");
    let mut leases = Vec::with_capacity(3);
    for (name, hash) in [
        (FIND_WRAPPER, hashes.wrapper.as_str()),
        (FIND_UPSTREAM, hashes.upstream.as_str()),
        (FIND_MANIFEST, hashes.manifest.as_str()),
    ] {
        let (file, digest) = open_immutable(&find_dir.join(name))?;
        if digest != hash {
            return Err(format!(
                "Could not retain the immutable Greeneye payload file {name}: its hash changed"
            ));
        }
        leases.push(file);
    }
    Ok(leases)
}

fn read_checked_file(path: &Path, retain: bool, max_bytes: u64) -> Result<SourceFile, String> {
    let (mut file, metadata) = open_checked(path, retain, max_bytes)?;
    let capacity = usize::try_from(metadata.len()).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    if bytes.len() as u64 != metadata.len() {
        return Err(format!(
            "{} changed while it was being read",
            path.display()
        ));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has an invalid file name", path.display()))?
        .to_owned();
    Ok(SourceFile {
        name,
        sha256: sha256(&bytes),
        bytes,
    })
}

fn inspect_regular_file(path: &Path, max_bytes: u64) -> Result<fs::Metadata, String> {
    let (file, metadata) = open_checked(path, false, max_bytes)?;
    drop(file);
    Ok(metadata)
}

fn open_immutable(path: &Path) -> Result<(File, String), String> {
    let (mut file, metadata) = open_checked(path, true, MAX_IMMUTABLE_FILE_BYTES)?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("Could not retain {}: {error}", path.display()))?;
    if bytes.len() as u64 != metadata.len() {
        return Err(format!(
            "{} changed while its immutable lease was opened",
            path.display()
        ));
    }
    Ok((file, sha256(&bytes)))
}

fn open_checked(path: &Path, lease: bool, max_bytes: u64) -> Result<(File, fs::Metadata), String> {
    validate_path_chain(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(if lease { 0x0000_0001 } else { 0x0000_0007 })
            .custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(|error| {
        format!(
            "Could not open {} without following links: {error}",
            path.display()
        )
    })?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err(format!(
            "{} is not a regular, non-reparse file",
            path.display()
        ));
    }
    if metadata.len() > max_bytes {
        return Err(format!(
            "{} exceeds the {max_bytes}-byte file limit",
            path.display()
        ));
    }
    if !single_link(&file, &metadata)? {
        return Err(format!(
            "{} is hard-linked. Admission was refused without changing it",
            path.display()
        ));
    }
    Ok((file, metadata))
}

fn validate_real_directory(root: &Path, path: &Path) -> Result<(), String> {
    validate_path_chain(path)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect directory {}: {error}", path.display()))?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err(format!(
            "{} is not a regular, non-reparse directory",
            path.display()
        ));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Could not resolve {}: {error}", path.display()))?;
    if !canonical.starts_with(root) {
        return Err(format!(
            "{} resolves outside the game folder",
            path.display()
        ));
    }
    Ok(())
}

fn exact_names(directory: &Path, required: &[&str]) -> Result<(), String> {
    let names = entry_names(directory)?;
    if required.iter().any(|name| !names.contains(*name)) {
        return Err(format!(
            "{} is missing an expected Greeneye payload path",
            directory.display()
        ));
    }
    Ok(())
}

fn entry_names(directory: &Path) -> Result<HashSet<String>, String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("Could not enumerate {}: {error}", directory.display()))?;
    let mut names = HashSet::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("Could not enumerate {}: {error}", directory.display()))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| format!("{} contains a non-UTF-8 entry", directory.display()))?;
        if !names.insert(name) {
            return Err(format!(
                "{} contains duplicate entry names",
                directory.display()
            ));
        }
    }
    Ok(names)
}

fn path_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Could not inspect {}: {error}", path.display())),
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Could not create staged file {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("Could not write staged file {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("Could not flush staged file {}: {error}", path.display()))
}

fn rename_absent(from: &Path, to: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileW(existing: *const u16, new: *const u16) -> i32;
            fn GetLastError() -> u32;
        }
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) } == 0 {
            return Err(format!(
                "Could not atomically publish game/greeneye without replacing an existing destination (Windows error {})",
                unsafe { GetLastError() }
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if path_exists(to)? {
            return Err("The Greeneye destination exists and will not be replaced".into());
        }
        fs::rename(from, to)
            .map_err(|error| format!("Could not atomically publish game/greeneye: {error}"))
    }
}

struct StagingDirectory {
    path: PathBuf,
    armed: bool,
}

impl StagingDirectory {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        // The staging path was atomically moved to the final absent destination.
        self.armed = false;
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    validate_path_chain(root)?;
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("Could not resolve game folder {}: {error}", root.display()))?;
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("Could not inspect game folder {}: {error}", root.display()))?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err(
            "Greeneye admission requires a regular game folder without reparse points".into(),
        );
    }
    Ok(canonical)
}

fn validate_path_chain(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                continue;
            }
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err(format!(
                    "{} contains a parent path component",
                    path.display()
                ));
            }
            Component::Normal(name) => current.push(name),
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_reparse(&metadata) => {
                return Err(format!(
                    "{} traverses a symlink or reparse point",
                    path.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Remaining components may be an absent destination or stage.
                break;
            }
            Err(error) => {
                return Err(format!(
                    "Could not inspect path component {}: {error}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn is_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return metadata.file_attributes() & 0x0000_0400 != 0; // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn single_link(file: &File, metadata: &fs::Metadata) -> Result<bool, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let _ = metadata;
        #[repr(C)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        #[repr(C)]
        struct ByHandleFileInformation {
            attributes: u32,
            creation_time: FileTime,
            last_access_time: FileTime,
            last_write_time: FileTime,
            volume_serial: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandle(
                file: std::os::windows::io::RawHandle,
                information: *mut ByHandleFileInformation,
            ) -> i32;
        }
        let mut info = ByHandleFileInformation {
            attributes: 0,
            creation_time: FileTime { low: 0, high: 0 },
            last_access_time: FileTime { low: 0, high: 0 },
            last_write_time: FileTime { low: 0, high: 0 },
            volume_serial: 0,
            size_high: 0,
            size_low: 0,
            links: 0,
            index_high: 0,
            index_low: 0,
        };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err("Could not verify Greeneye payload hard-link count".into());
        }
        return Ok(info.links == 1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = file;
        return Ok(metadata.nlink() == 1);
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = file;
        let _ = metadata;
        Err("Hard-link validation is unavailable on this platform".into())
    }
}

fn ensure_no_other_game_process(root: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        type Handle = *mut std::ffi::c_void;
        const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const WAIT_UNUSED: u32 = 0;
        const ERROR_NO_MORE_FILES: u32 = 18;
        #[repr(C)]
        struct ProcessEntry32W {
            size: u32,
            usage: u32,
            process_id: u32,
            default_heap_id: usize,
            module_id: u32,
            threads: u32,
            parent_process_id: u32,
            priority_base: i32,
            flags: u32,
            exe_file: [u16; 260],
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
            fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
            fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
            fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> Handle;
            fn QueryFullProcessImageNameW(
                process: Handle,
                flags: u32,
                path: *mut u16,
                size: *mut u32,
            ) -> i32;
            fn CloseHandle(handle: Handle) -> i32;
            fn GetLastError() -> u32;
            fn GetCurrentProcessId() -> u32;
        }
        struct SnapshotHandle(Handle);
        impl Drop for SnapshotHandle {
            fn drop(&mut self) {
                unsafe { CloseHandle(self.0) };
            }
        }
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, WAIT_UNUSED) };
        if snapshot as isize == -1 || snapshot.is_null() {
            return Err(format!(
                "Could not enumerate game-folder processes (Windows error {})",
                unsafe { GetLastError() }
            ));
        }
        let snapshot = SnapshotHandle(snapshot);
        let mut entry = ProcessEntry32W {
            size: std::mem::size_of::<ProcessEntry32W>() as u32,
            usage: 0,
            process_id: 0,
            default_heap_id: 0,
            module_id: 0,
            threads: 0,
            parent_process_id: 0,
            priority_base: 0,
            flags: 0,
            exe_file: [0; 260],
        };
        if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
            return Err(format!(
                "Could not read game-folder process inventory (Windows error {})",
                unsafe { GetLastError() }
            ));
        }
        let current_pid = unsafe { GetCurrentProcessId() };
        loop {
            let end = entry
                .exe_file
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(entry.exe_file.len());
            let name = OsString::from_wide(&entry.exe_file[..end])
                .to_string_lossy()
                .into_owned();
            let game_name = name.eq_ignore_ascii_case("enshrouded.exe")
                || name.eq_ignore_ascii_case("enshrouded_server.exe");
            if game_name && entry.process_id != current_pid {
                let process =
                    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.process_id) };
                if process.is_null() {
                    return Err(format!(
                        "Could not verify another {name} process before Greeneye admission (Windows error {})",
                        unsafe { GetLastError() }
                    ));
                }
                let process = SnapshotHandle(process);
                let mut path = vec![0u16; 32768];
                let mut size = path.len() as u32;
                if unsafe { QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut size) }
                    == 0
                {
                    return Err(format!(
                        "Could not resolve another {name} process path (Windows error {})",
                        unsafe { GetLastError() }
                    ));
                }
                path.truncate(size as usize);
                let image = PathBuf::from(OsString::from_wide(&path));
                let image_root = image.parent().ok_or_else(|| {
                    format!("Could not resolve the game folder for another {name} process")
                })?;
                let image_root = image_root.canonicalize().map_err(|error| {
                    format!("Could not canonicalize another {name} process folder: {error}")
                })?;
                if same_path(&image_root, root) {
                    return Err(format!(
                        "Another {name} process is using this game folder. Greeneye admission requires a restart after that process exits"
                    ));
                }
            }
            entry.size = std::mem::size_of::<ProcessEntry32W>() as u32;
            if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_NO_MORE_FILES {
                    return Err(format!(
                        "Could not finish game-folder process inventory (Windows error {error})"
                    ));
                }
                break;
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Ok(())
    }
}

fn reserved_kind(root: &Path, entry: &DllLoadEntry) -> Result<ReservedKind, String> {
    if entry
        .package_directory
        .eq_ignore_ascii_case(HOST_PACKAGE_DIRECTORY)
    {
        return Ok(ReservedKind::Host);
    }
    if entry
        .package_directory
        .eq_ignore_ascii_case(FIND_PACKAGE_DIRECTORY)
        || entry
            .mod_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(FIND_MOD_ID))
    {
        return Ok(ReservedKind::FindPayload);
    }
    let name = entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let known_name = [HOST_FILE, FIND_WRAPPER, FIND_UPSTREAM]
        .iter()
        .any(|expected| name.eq_ignore_ascii_case(expected));
    if !known_name
        && !entry
            .path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
    {
        return Ok(ReservedKind::None);
    }
    let path = match entry.path.canonicalize() {
        Ok(path) => path,
        Err(error) if known_name => {
            return Err(format!("Could not verify reserved DLL identity: {error}"));
        }
        Err(_) => return Ok(ReservedKind::None),
    };
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if known_name => {
            return Err(format!("Could not inspect reserved DLL candidate: {error}"));
        }
        Err(_) => return Ok(ReservedKind::None),
    };
    if metadata.len() > MAX_IMMUTABLE_FILE_BYTES {
        if known_name {
            return Err(
                "A known Greeneye DLL filename exceeds the safe identity-check size limit".into(),
            );
        }
        return Ok(ReservedKind::None);
    }
    let hash = match read_checked_file(&path, false, MAX_IMMUTABLE_FILE_BYTES) {
        Ok(file) => file.sha256,
        Err(error) if known_name => {
            return Err(format!("Could not verify reserved DLL identity: {error}"));
        }
        Err(_) => return Ok(ReservedKind::None),
    };
    if hash == HOST_SHA256 {
        return Ok(ReservedKind::Host);
    }
    if hash == FIND_WRAPPER_SHA256 || hash == FIND_UPSTREAM_SHA256 {
        return Ok(ReservedKind::FindPayload);
    }
    let _ = root;
    Ok(ReservedKind::None)
}

pub(crate) fn reject_reserved_plugin_registration(
    root: &Path,
    mod_id: &str,
    package_directory: &str,
    path: &Path,
) -> Result<(), String> {
    let entry = DllLoadEntry {
        status_id: format!("plugin:{package_directory}:{mod_id}"),
        mod_id: Some(mod_id.to_owned()),
        package_directory: package_directory.to_owned(),
        path: path.to_path_buf(),
        mode: DllLoadMode::Library,
        source: "Lua native plugin registration".into(),
    };
    match reserved_kind(root, &entry)? {
        ReservedKind::None => Ok(()),
        ReservedKind::Host => Err(
            "The original Greeneye host can load only through its separately approved DLL Manager profile during startup".into(),
        ),
        ReservedKind::FindPayload => Err(
            "Find wrapper and upstream binaries are host-owned and cannot be loaded directly as native plugins".into(),
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReservedKind {
    None,
    Host,
    FindPayload,
}

fn admissions() -> &'static Mutex<HashMap<String, AdmissionContext>> {
    ADMISSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn root_key(root: &Path) -> String {
    root.to_string_lossy().to_lowercase()
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct AdmissionLock {
    _handle: AdmissionLockHandle,
}

impl AdmissionLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        AdmissionLockHandle::acquire(root).map(|handle| Self { _handle: handle })
    }
}

#[cfg(windows)]
struct AdmissionLockHandle(*mut std::ffi::c_void);

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        attributes: *const std::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn ReleaseMutex(handle: *mut std::ffi::c_void) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    fn GetLastError() -> u32;
}

#[cfg(windows)]
impl AdmissionLockHandle {
    fn acquire(root: &Path) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        const WAIT_OBJECT_0: u32 = 0;
        const WAIT_ABANDONED: u32 = 0x0000_0080;
        const WAIT_TIMEOUT: u32 = 0x0000_0102;
        let digest = sha256(root_key(root).as_bytes());
        let name = format!("Local\\ShroudForge.GreeneyeFind.{digest}");
        let name: Vec<u16> = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "Could not create Greeneye admission mutex (Windows error {})",
                unsafe { GetLastError() }
            ));
        }
        let state = unsafe { WaitForSingleObject(handle, 30_000) };
        if state == WAIT_OBJECT_0 || state == WAIT_ABANDONED {
            return Ok(Self(handle));
        }
        unsafe { CloseHandle(handle) };
        if state == WAIT_TIMEOUT {
            Err("Another process is preparing this game's Greeneye directory. Admission timed out without changing it".into())
        } else {
            Err(format!(
                "Could not acquire Greeneye admission mutex (Windows error {})",
                unsafe { GetLastError() }
            ))
        }
    }
}

#[cfg(windows)]
impl Drop for AdmissionLockHandle {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

#[cfg(not(windows))]
struct AdmissionLockHandle(std::sync::MutexGuard<'static, ()>);

#[cfg(not(windows))]
static NON_WINDOWS_ADMISSION_LOCK: Mutex<()> = Mutex::new(());

#[cfg(not(windows))]
impl AdmissionLockHandle {
    fn acquire(_root: &Path) -> Result<Self, String> {
        NON_WINDOWS_ADMISSION_LOCK
            .lock()
            .map(Self)
            .map_err(|_| "Greeneye admission mutex is poisoned".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixtureRoot(PathBuf);

    impl FixtureRoot {
        fn new() -> Self {
            let base = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let path = base.join(format!("greeneye-admission-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }

    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            let key = self.0.canonicalize().map(|root| root_key(&root)).ok();
            if let Some(key) = key {
                admissions()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(&key);
            }
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_hashes() -> HashPins {
        HashPins {
            host: sha256(b"inert test host"),
            wrapper: sha256(b"inert test wrapper"),
            upstream: sha256(b"inert test upstream"),
            manifest: sha256(b"inert test manifest"),
        }
    }

    fn save_dll_selections(root: &Path, host_enabled: bool, find_enabled: bool) {
        let mut config = shroudforge_loader_config::config::read_loader(root).unwrap();
        config["modules"]["dllManager"]["profiles"] = serde_json::json!([
            {
                "id": "greeneye-host-selection",
                "packageDirectory": HOST_PACKAGE_DIRECTORY,
                "name": "Greeneye host",
                "target": "client",
                "enabled": host_enabled,
                "approved": true,
                "compatibilityProfile": HOST_PROFILE_ID,
                "files": []
            },
            {
                "id": "find-selection",
                "packageDirectory": FIND_PACKAGE_DIRECTORY,
                "name": "Find",
                "target": "client",
                "enabled": find_enabled,
                "approved": true,
                "compatibilityProfile": FIND_PROFILE_ID,
                "files": []
            }
        ]);
        fs::write(
            shroudforge_loader_config::paths::loader_config(root),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
    }

    fn public_plan_fixture() -> FixtureRoot {
        let root = FixtureRoot::new();
        let host_package = root.0.join("mods").join(HOST_PACKAGE_DIRECTORY);
        let find_source = root.0.join("mods/find/greeneye/find");
        fs::create_dir_all(&host_package).unwrap();
        fs::create_dir_all(&find_source).unwrap();
        fs::write(host_package.join(HOST_FILE), b"inert test host").unwrap();
        fs::write(find_source.join(FIND_WRAPPER), b"inert test wrapper").unwrap();
        fs::write(find_source.join(FIND_UPSTREAM), b"inert test upstream").unwrap();
        fs::write(
            find_source.join(FIND_MANIFEST),
            b"inert nested original manifest",
        )
        .unwrap();
        let profiles = shroudforge_loader_config::paths::mod_profiles_dir(&root.0);
        fs::create_dir_all(&profiles).unwrap();
        fs::write(
            profiles.join("greeneye-find-host.json"),
            include_str!("../../modloader/mod-profiles/greeneye-find-host.json"),
        )
        .unwrap();
        fs::write(
            profiles.join("find-mouse-alignment.json"),
            include_str!("../../modloader/mod-profiles/find-mouse-alignment.json"),
        )
        .unwrap();
        shroudforge_loader_config::config::initialize_loader_config(&root.0).unwrap();
        save_dll_selections(&root.0, true, true);
        root
    }

    #[test]
    fn shipped_profiles_match_the_fixed_greeneye_import_contract() {
        let host: mod_loader::mod_profiles::ModCompatibilityProfile = serde_json::from_str(
            include_str!("../../modloader/mod-profiles/greeneye-find-host.json"),
        )
        .unwrap();
        let find: mod_loader::mod_profiles::ModCompatibilityProfile = serde_json::from_str(
            include_str!("../../modloader/mod-profiles/find-mouse-alignment.json"),
        )
        .unwrap();
        validate_profile(
            &host,
            HOST_PROFILE_ID,
            HOST_PROFILE_NAME,
            HOST_PACKAGE_DIRECTORY,
            "",
            HOST_FILE,
            &[("native_runtime/find/dbgcore.dll", HOST_FILE)],
        )
        .unwrap();
        validate_profile(
            &find,
            FIND_PROFILE_ID,
            FIND_PROFILE_NAME,
            FIND_PACKAGE_DIRECTORY,
            FIND_MOD_ID,
            FIND_DLL_PATH,
            &[
                ("native_runtime/find/greeneye/find/find.dll", FIND_DLL_PATH),
                (
                    "native_runtime/find/greeneye/find/find_upstream.dll",
                    "greeneye/find/find_upstream.dll",
                ),
                (
                    "native_runtime/find/greeneye/find/mod.json",
                    "greeneye/find/mod.json",
                ),
            ],
        )
        .unwrap();
    }

    fn fixture_source(hashes: &HashPins) -> Vec<SourceFile> {
        [
            (
                FIND_WRAPPER,
                b"inert test wrapper".as_slice(),
                &hashes.wrapper,
            ),
            (
                FIND_UPSTREAM,
                b"inert test upstream".as_slice(),
                &hashes.upstream,
            ),
            (
                FIND_MANIFEST,
                b"inert test manifest".as_slice(),
                &hashes.manifest,
            ),
        ]
        .into_iter()
        .map(|(name, bytes, hash)| SourceFile {
            name: name.into(),
            bytes: bytes.to_vec(),
            sha256: hash.clone(),
        })
        .collect()
    }

    fn write_runtime_tree(root: &Path, hashes: &HashPins) -> PathBuf {
        let directory = root.join("greeneye");
        let find = directory.join("find");
        fs::create_dir_all(&find).unwrap();
        for source in fixture_source(hashes) {
            fs::write(find.join(source.name), source.bytes).unwrap();
        }
        directory
    }

    fn plan_entry(
        status_id: &str,
        package_directory: &str,
        mod_id: Option<&str>,
        path: PathBuf,
    ) -> DllLoadEntry {
        DllLoadEntry {
            status_id: status_id.into(),
            mod_id: mod_id.map(str::to_owned),
            package_directory: package_directory.into(),
            path,
            mode: DllLoadMode::Library,
            source: "test fixture".into(),
        }
    }

    #[test]
    fn existing_tree_accepts_only_pinned_payload_and_preserves_userdata() {
        let root = FixtureRoot::new();
        let hashes = fixture_hashes();
        let directory = write_runtime_tree(&root.0, &hashes);
        let user_files = [
            (directory.join("loader.log"), b"host log bytes\0".as_slice()),
            (
                directory.join("find/find.ini"),
                b"unknown=keep\r\n".as_slice(),
            ),
            (
                directory.join("find/find.log"),
                b"find log bytes\n".as_slice(),
            ),
        ];
        for (path, bytes) in &user_files {
            fs::write(path, bytes).unwrap();
        }
        verify_runtime_tree(&root.0, &directory, &hashes).unwrap();
        for (path, bytes) in user_files {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn mismatched_tree_is_refused_without_changing_its_bytes() {
        let root = FixtureRoot::new();
        let hashes = fixture_hashes();
        let directory = write_runtime_tree(&root.0, &hashes);
        let extra = directory.join("rootloader.log");
        fs::write(&extra, b"preserve me").unwrap();
        assert!(verify_runtime_tree(&root.0, &directory, &hashes).is_err());
        assert_eq!(fs::read(extra).unwrap(), b"preserve me");
        assert_eq!(
            fs::read(directory.join("find/find.dll")).unwrap(),
            b"inert test wrapper"
        );
    }

    #[test]
    fn absent_tree_publishes_complete_original_shape_atomically() {
        let root = FixtureRoot::new();
        let hashes = fixture_hashes();
        let destination = root.0.join("greeneye");
        publish_absent_tree(
            &root.0,
            &destination,
            &fixture_source(&hashes),
            &hashes,
            || Ok(()),
        )
        .unwrap();
        verify_runtime_tree(&root.0, &destination, &hashes).unwrap();
        assert_eq!(
            entry_names(&destination).unwrap(),
            HashSet::from(["find".to_owned()])
        );
        assert_eq!(
            entry_names(&root.0).unwrap(),
            HashSet::from(["greeneye".to_owned()])
        );
    }

    #[test]
    fn late_plan_withholds_greeneye_and_requires_restart_without_staging() {
        let root = FixtureRoot::new();
        let plan = DllLoadPlan {
            entries: vec![
                plan_entry("host", HOST_PACKAGE_DIRECTORY, None, PathBuf::from("host")),
                plan_entry(
                    "find",
                    FIND_PACKAGE_DIRECTORY,
                    Some(FIND_MOD_ID),
                    PathBuf::from("find"),
                ),
            ],
            issues: Vec::new(),
        };
        let prepared = prepare(&root.0, false, false, &plan);
        assert!(matches!(
            prepared.dispositions.get("host"),
            Some(Disposition::Skip)
        ));
        assert!(matches!(
            prepared.dispositions.get("find"),
            Some(Disposition::Skip)
        ));
        assert!(
            prepared
                .issues
                .iter()
                .any(|(_, detail)| detail.contains("restart"))
        );
        assert!(!root.0.join("greeneye").exists());
    }

    #[test]
    fn find_profile_plan_accepts_original_package_without_a_root_manifest() {
        let root = FixtureRoot::new();
        let host_path = root.0.join("mods/greeneye-host/dbgcore.dll");
        let find_path = root.0.join("mods/find/greeneye/find/find.dll");
        fs::create_dir_all(host_path.parent().unwrap()).unwrap();
        fs::create_dir_all(find_path.parent().unwrap()).unwrap();
        fs::write(&host_path, b"inert host").unwrap();
        fs::write(&find_path, b"inert wrapper").unwrap();
        let profile = ProfileContract {
            host_profile_id: HOST_PROFILE_ID.into(),
            host_directory: HOST_PACKAGE_DIRECTORY.into(),
            find_profile_id: FIND_PROFILE_ID.into(),
            find_directory: FIND_PACKAGE_DIRECTORY.into(),
            find_mod_id: FIND_MOD_ID.into(),
        };
        let host = plan_entry(
            "host",
            HOST_PACKAGE_DIRECTORY,
            None,
            host_path.canonicalize().unwrap(),
        );
        let find = plan_entry(
            "find",
            FIND_PACKAGE_DIRECTORY,
            None,
            find_path.canonicalize().unwrap(),
        );
        validate_plan_entries(&root.0, &profile, &[&host], &[&find]).unwrap();
    }

    #[test]
    fn public_dll_plan_includes_both_approved_client_profiles_and_honors_disable() {
        let root = public_plan_fixture();
        let plan = mod_loader::dll_manager::resolve_load_plan(&root.0, false);
        assert!(
            plan.issues.is_empty(),
            "unexpected plan issues: {:?}",
            plan.issues
        );
        assert_eq!(plan.entries.len(), 2);
        let host = plan
            .entries
            .iter()
            .find(|entry| entry.package_directory == HOST_PACKAGE_DIRECTORY)
            .unwrap();
        let find = plan
            .entries
            .iter()
            .find(|entry| entry.package_directory == FIND_PACKAGE_DIRECTORY)
            .unwrap();
        assert_eq!(
            host.path,
            root.0
                .join("mods/greeneye-host/dbgcore.dll")
                .canonicalize()
                .unwrap()
        );
        assert_eq!(
            find.path,
            root.0
                .join("mods/find/greeneye/find/find.dll")
                .canonicalize()
                .unwrap()
        );
        assert_eq!(host.mode, DllLoadMode::Library);
        assert_eq!(find.mode, DllLoadMode::Library);
        assert_eq!(
            find.mod_id, None,
            "the original manifest is nested under greeneye/find"
        );

        save_dll_selections(&root.0, true, false);
        let without_find = mod_loader::dll_manager::resolve_load_plan(&root.0, false);
        assert!(
            without_find
                .entries
                .iter()
                .any(|entry| entry.package_directory == HOST_PACKAGE_DIRECTORY)
        );
        assert!(
            !without_find
                .entries
                .iter()
                .any(|entry| entry.package_directory == FIND_PACKAGE_DIRECTORY)
        );

        save_dll_selections(&root.0, false, true);
        let without_host = mod_loader::dll_manager::resolve_load_plan(&root.0, false);
        assert!(
            !without_host
                .entries
                .iter()
                .any(|entry| entry.package_directory == HOST_PACKAGE_DIRECTORY)
        );
        assert!(
            without_host
                .entries
                .iter()
                .any(|entry| entry.package_directory == FIND_PACKAGE_DIRECTORY)
        );
    }

    #[test]
    fn lua_plugin_registration_cannot_load_find_package_or_host_profile() {
        let root = FixtureRoot::new();
        assert!(
            reject_reserved_plugin_registration(
                &root.0,
                FIND_MOD_ID,
                "some-folder",
                Path::new("arbitrary.dll"),
            )
            .is_err()
        );
        assert!(
            reject_reserved_plugin_registration(
                &root.0,
                "unrelated",
                HOST_PACKAGE_DIRECTORY,
                Path::new("arbitrary.dll"),
            )
            .is_err()
        );
    }

    #[test]
    fn admitted_host_queue_permit_is_one_shot() {
        let root = FixtureRoot::new();
        let host_path = root.0.join("mods/greeneye-host/dbgcore.dll");
        fs::create_dir_all(host_path.parent().unwrap()).unwrap();
        fs::write(&host_path, b"inert test host").unwrap();
        let host_path = host_path.canonicalize().unwrap();
        let canonical_root = root.0.canonicalize().unwrap();
        let entry = plan_entry(
            "host-status",
            HOST_PACKAGE_DIRECTORY,
            None,
            host_path.clone(),
        );
        let context = AdmissionContext {
            root: canonical_root.clone(),
            host_path,
            host_status_id: entry.status_id.clone(),
            find_status_id: "find-status".into(),
            queue_permit: false,
            contract: AdmissionContract {
                profile: ProfileContract {
                    host_profile_id: HOST_PROFILE_ID.into(),
                    host_directory: HOST_PACKAGE_DIRECTORY.into(),
                    find_profile_id: FIND_PROFILE_ID.into(),
                    find_directory: FIND_PACKAGE_DIRECTORY.into(),
                    find_mod_id: FIND_MOD_ID.into(),
                },
                hashes: fixture_hashes(),
            },
            plan_identity: serde_json::Value::Null,
            plan_issues: serde_json::Value::Null,
            selection_identity: serde_json::Value::Null,
            runtime_directory: canonical_root.join("greeneye"),
            _immutable_leases: Vec::new(),
        };
        admissions()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(root_key(&canonical_root), context);
        assert!(check_queue(&canonical_root, &entry).is_err());
        authorize_host_queue(&canonical_root, &entry).unwrap();
        check_queue(&canonical_root, &entry).unwrap();
        assert!(check_queue(&canonical_root, &entry).is_err());
    }
}

#[cfg(not(windows))]
impl Drop for AdmissionLockHandle {
    fn drop(&mut self) {
        let _ = &self.0;
    }
}
