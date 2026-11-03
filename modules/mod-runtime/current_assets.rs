//! Explicit current-composition startup policy. Never falls back to baselines.
//! The installed recipe supplies original resource records from reviewed source,
//! not from a snapshot of the user's asset files. Publication recovers forward.
use anyhow::{Context, Result, ensure};
use kfc::{
    container::{KFCFile, KFCReader, KFCWriteOptions, KFCWriter},
    guid::{ContentHash, ResourceId},
    reflection::{LookupKey, TypeRegistry},
    resource::value::Value,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shroudforge_game_assets::transaction;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const STEM: &str = "enshrouded";
const POLICY: &str = "shroudforge/config/current-assets.json";
const STATE: &str = "shroudforge/config/current-assets-state.json";
const PENDING: &str = "shroudforge/config/current-assets-pending.json";
type Stamp = (u64, String);
#[path = "current_assets/layout_migration.rs"]
mod layout_migration;
#[path = "current_assets/resource_contribution.rs"]
mod resource_contribution;
#[path = "current_assets/zoom.rs"]
mod zoom;
#[path = "current_assets/zoom_demand.rs"]
mod zoom_demand;

/// Clear a stale one-use demand before any new startup preparation attempt.
pub fn clear_startup_zoom_demand() {
    zoom_demand::clear();
}

/// Consume the demand produced by the successful preparation on this thread.
pub fn take_startup_zoom_demand(root: &Path) -> u32 {
    zoom_demand::take(root)
}

/// Resolve the startup zoom result for the client bootstrap export.
///
/// This is the single root, ownership, and token-consumption decision used by
/// the export. A missing client executable or inactive current-assets plan
/// clears a stale demand. Errors are returned so the native export can fail
/// closed with its established error code.
pub fn take_startup_zoom_export_demand(root: &Path) -> Result<u32> {
    let result = (|| {
        if !root.join("enshrouded.exe").is_file() {
            zoom_demand::clear();
            return Ok(0);
        }
        let canonical_root = fs::canonicalize(root).context("could not canonicalize game root")?;
        if active(&canonical_root)? {
            Ok(take_startup_zoom_demand(&canonical_root))
        } else {
            zoom_demand::clear();
            Ok(0)
        }
    })();
    if result.is_err() {
        zoom_demand::clear();
    }
    result
}

#[cfg(test)]
#[path = "current_assets/zoom_tests.rs"]
mod zoom_tests;

#[cfg(test)]
#[path = "current_assets/zoom_recovery_tests.rs"]
mod zoom_recovery_tests;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Policy {
    schema_version: u32,
    mod_id: String,
    package: String,
    generation: String,
    executable_sha256: String,
    resources: Vec<Record>,
    retained_contents: Vec<Content>,
    composition: Composition,
    legacy_pickaxe_reach: Content,
    #[serde(default)]
    zoom: Option<zoom::Contract>,
    #[serde(default)]
    migration: Option<Migration>,
    #[serde(default)]
    layout_migration: Option<layout_migration::Contract>,
    #[serde(default)]
    resource_contribution: Option<resource_contribution::Contract>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DedicatedPolicy {
    schema_version: u32,
    target: String,
    mod_id: String,
    package: String,
    generation: String,
    executable_sha256: String,
    composition: Composition,
    resource_contribution: resource_contribution::Contract,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Migration {
    state_sha256: String,
    fingerprint: String,
    enabled: bool,
    configured_extended_zoom: bool,
    effective_zoom: bool,
    outputs: BTreeMap<String, Stamp>,
}
/// An explicitly reviewed existing composition, never an automatic capture.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Composition {
    fingerprint: String,
    outputs: BTreeMap<String, Stamp>,
    /// Exact complete source selection for a reviewed package-removal transition.
    #[serde(default)]
    source_fingerprint: Option<String>,
    /// State observed in the reviewed baseline before this recipe was installed.
    #[serde(default)]
    baseline_enabled: bool,
    /// Whether the exact legacy reach records are already at their reviewed 5 m values.
    #[serde(default)]
    baseline_legacy_pickaxe_reach_migrated: bool,
    /// Exact disabled packages that belong to the source installation but are
    /// intentionally absent from the target composition.
    #[serde(default)]
    excluded_packages: Vec<ExcludedPackage>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExcludedPackage {
    id: String,
    fingerprint: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    id: String,
    type_hash: u32,
    original: String,
    original_sha256: String,
    applied_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    file: String,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReachReference {
    schema_version: u32,
    generation: String,
    legacy_lua_sha256: String,
    reference_index_sha256: String,
    records: Vec<ReachRecord>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReachRecord {
    id: String,
    current_resource_sha256: String,
    reference_resource_sha256: String,
    original_max_distance: f64,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Prepared {
    schema_version: u32,
    root: String,
    fingerprint: String,
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    configured_extended_zoom: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effective_zoom: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_state_sha256: Option<String>,
    outputs: BTreeMap<String, Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resource_contribution: Option<resource_contribution::State>,
}

fn plain(path: &Path) -> Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        !meta.file_type().is_symlink(),
        "symlink in current-assets path"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            meta.file_attributes() & 0x400 == 0,
            "reparse point in current-assets path"
        );
    }
    Ok(meta)
}
fn joined(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    ensure!(!relative.is_empty(), "empty current-assets path");
    for part in Path::new(relative).components() {
        let Component::Normal(part) = part else {
            anyhow::bail!("invalid relative current-assets path")
        };
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                plain(&path)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn stamp(path: &Path) -> Result<Stamp> {
    ensure!(
        plain(path)?.is_file(),
        "current-assets input is not a regular file"
    );
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut count = 0;
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        hasher.update(&buf[..n]);
    }
    Ok((count, format!("{:x}", hasher.finalize())))
}
fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T> {
    ensure!(
        plain(path)?.len() <= 1024 * 1024,
        "oversized current-assets document"
    );
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    if temporary.exists() {
        plain(&temporary)?;
    }
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}
fn valid_output(name: &str) -> bool {
    valid_output_for(name, STEM)
}
fn valid_output_for(name: &str, stem: &str) -> bool {
    name == format!("{stem}.kfc")
        || name == format!("{stem}.kfc_resources")
        || name
            .strip_prefix(&format!("{stem}_"))
            .and_then(|s| s.strip_suffix(".dat"))
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
fn validate_shape(root: &Path, value: &Prepared) -> Result<()> {
    ensure!(
        matches!(value.schema_version, 1 | 2) && value.root == root.to_string_lossy(),
        "prepared-state root mismatch"
    );
    if value.schema_version == 1 {
        ensure!(
            value.configured_extended_zoom.is_none()
                && value.effective_zoom.is_none()
                && value.previous_state_sha256.is_none(),
            "legacy state contains zoom selection"
        );
    } else {
        let configured = value
            .configured_extended_zoom
            .context("missing configured zoom")?;
        ensure!(
            value.effective_zoom == Some(value.enabled && configured),
            "inconsistent effective zoom"
        );
        let previous = value
            .previous_state_sha256
            .as_deref()
            .context("missing zoom transaction predecessor")?;
        ensure!(
            valid_sha256(previous),
            "invalid zoom transaction predecessor"
        );
    }
    let stem = value.target.as_deref().unwrap_or(STEM);
    ensure!(
        matches!(stem, "enshrouded" | "enshrouded_server"),
        "prepared-state target does not match its resource contribution"
    );
    if let Some(contribution) = value.resource_contribution.as_ref() {
        ensure!(
            contribution.target() == stem,
            "prepared resource-contribution target mismatch"
        );
    } else {
        ensure!(stem == STEM, "dedicated prepared state has no contribution");
    }
    if stem == "enshrouded_server" {
        ensure!(
            value.schema_version == 1
                && value.resource_contribution.is_some()
                && value.configured_extended_zoom.is_none()
                && value.effective_zoom.is_none()
                && value.previous_state_sha256.is_none(),
            "dedicated contribution receipt contains client-only state"
        );
    }
    ensure!(
        value.outputs.contains_key(&format!("{stem}.kfc"))
            && value.outputs.contains_key(&format!("{stem}.kfc_resources")),
        "incomplete prepared-state"
    );
    ensure!(value.outputs.len() <= 512, "oversized prepared output set");
    ensure!(
        value.fingerprint.len() == 64 && value.fingerprint.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid prepared fingerprint"
    );
    for (name, expected) in &value.outputs {
        ensure!(valid_output_for(name, stem), "invalid prepared output path");
        ensure!(
            expected.1.len() == 64 && expected.1.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid prepared output digest"
        );
    }
    Ok(())
}
fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn validate_migration(policy: &Policy) -> Result<()> {
    ensure!(
        (policy.schema_version == 2 && policy.resource_contribution.is_none())
            || (policy.schema_version == 3 && policy.resource_contribution.is_some()),
        "unsupported current-assets recipe schema or resource contribution"
    );
    ensure!(
        policy.layout_migration.is_none() || policy.migration.is_some(),
        "layout migration requires the exact existing Migration admission"
    );
    let Some(migration) = policy.migration.as_ref() else {
        return Ok(());
    };
    ensure!(
        policy.zoom.is_some(),
        "migration requires explicit zoom policy"
    );
    ensure!(
        valid_sha256(&migration.state_sha256) && valid_sha256(&migration.fingerprint),
        "invalid reviewed migration identity"
    );
    if policy.layout_migration.is_some() {
        ensure!(
            migration.effective_zoom == (migration.enabled && migration.configured_extended_zoom),
            "reviewed layout migration has an inconsistent parent and ExtendedZoom selection"
        );
        layout_migration::gpc_applied_digest(policy)?;
    } else {
        ensure!(
            migration.enabled && migration.configured_extended_zoom && migration.effective_zoom,
            "reviewed migration must pin the enabled parent and ExtendedZoom selection"
        );
    }
    ensure!(
        !migration.outputs.is_empty() && migration.outputs == policy.composition.outputs,
        "reviewed migration stamps differ from complete composition"
    );
    ensure!(
        migration.outputs.contains_key("enshrouded.kfc")
            && migration.outputs.contains_key("enshrouded.kfc_resources"),
        "reviewed migration omits the required asset pair"
    );
    for (name, expected) in &migration.outputs {
        ensure!(valid_output(name), "invalid reviewed migration output path");
        ensure!(
            valid_sha256(&expected.1),
            "invalid reviewed migration output digest"
        );
    }
    Ok(())
}
fn validate_migration_source(
    root: &Path,
    state_path: &Path,
    state: &Prepared,
    policy: &Policy,
    migration: &Migration,
    enabled: bool,
    configured_zoom: bool,
    fingerprint: &str,
    old_layout_records: bool,
) -> Result<Prepared> {
    validate_shape(root, state)?;
    ensure!(
        migration.outputs == policy.composition.outputs,
        "migration stamp map differs from the reviewed full composition"
    );
    validate_migration_identity(state_path, state, migration, enabled, configured_zoom)?;
    // Rebind only in memory. The original, subset-owned state file remains the
    // transaction predecessor until publication succeeds.
    rebind_prepared_state_with_records(root, policy, state, fingerprint, old_layout_records)
}

fn validate_migration_identity(
    state_path: &Path,
    state: &Prepared,
    migration: &Migration,
    enabled: bool,
    configured_zoom: bool,
) -> Result<()> {
    ensure!(
        stamp(state_path)?.1 == migration.state_sha256
            && state.fingerprint == migration.fingerprint
            && state.enabled == migration.enabled
            && enabled == migration.enabled
            && configured_zoom == migration.configured_extended_zoom,
        "prepared state does not match the exact reviewed migration"
    );
    if state.schema_version == 2 {
        ensure!(
            state.configured_extended_zoom == Some(migration.configured_extended_zoom)
                && state.effective_zoom == Some(migration.effective_zoom),
            "typed state selection differs from reviewed migration"
        );
    } else {
        ensure!(
            state.schema_version == 1,
            "unsupported migration source state"
        );
    }
    Ok(())
}
fn verify_outputs(root: &Path, value: &Prepared) -> Result<()> {
    validate_shape(root, value)?;
    let stem = value.target.as_deref().unwrap_or(STEM);
    for (name, expected) in &value.outputs {
        ensure!(
            stamp(&joined(root, name)?)? == *expected,
            "prepared output changed: {name}"
        );
    }
    verify_inventory_for(root, &value.outputs, stem)
}
fn state_outputs_match_composition(
    state: &BTreeMap<String, Stamp>,
    composition: &BTreeMap<String, Stamp>,
) -> bool {
    !state.is_empty()
        && state.len() <= composition.len()
        && state
            .iter()
            .all(|(name, stamp)| composition.get(name) == Some(stamp))
}
fn verify_inventory(root: &Path, outputs: &BTreeMap<String, Stamp>) -> Result<()> {
    verify_inventory_for(root, outputs, STEM)
}
fn verify_inventory_for(root: &Path, outputs: &BTreeMap<String, Stamp>, stem: &str) -> Result<()> {
    let mut found = HashSet::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if let Some(name) = entry
            .file_name()
            .to_str()
            .filter(|name| valid_output_for(name, stem))
        {
            ensure!(plain(&entry.path())?.is_file(), "non-file asset output");
            found.insert(name.to_owned());
        }
    }
    ensure!(
        found == outputs.keys().cloned().collect(),
        "composition asset inventory changed"
    );
    Ok(())
}

/// Calculate the read-only pregame-package binding used by a reviewed baseline.
/// This does not assert Shroudtopia selection or that package effects are present
/// in the current assets.
pub fn composition_fingerprint(root: &Path) -> Result<String> {
    let root = fs::canonicalize(root)?;
    let environment = mod_loader::ModEnvironment::load(root.to_str().context("non-UTF8 root")?)
        .map_err(|_| anyhow::anyhow!("could not load composition environment"))?;
    composition_fingerprint_for(&root, &environment, &[])
}
/// Calculate a read-only composition fingerprint that leaves AutoLoot settings
/// bound while ignoring its enabled toggle and package payload.
pub fn composition_fingerprint_with_auto_loot_contribution(root: &Path) -> Result<String> {
    require_unlinked_directory(root)?;
    let root = fs::canonicalize(root)?;
    let mods_dir = shroudforge_loader_config::paths::mods_dir(&root);
    require_unlinked_directory(&mods_dir)?;
    let environment = mod_loader::ModEnvironment::load(root.to_str().context("non-UTF8 root")?)
        .map_err(|_| anyhow::anyhow!("could not load composition environment"))?;
    composition_fingerprint_for_with_contribution(&root, &environment, &[], true)
}
fn require_unlinked_directory(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    for ancestor in absolute.ancestors() {
        ensure!(
            plain(ancestor)?.is_dir(),
            "composition environment paths must be existing directories"
        );
    }
    Ok(())
}
fn composition_fingerprint_for(
    root: &Path,
    environment: &mod_loader::ModEnvironment,
    excluded_packages: &[ExcludedPackage],
) -> Result<String> {
    composition_fingerprint_for_with_contribution(root, environment, excluded_packages, false)
}
fn composition_fingerprint_for_with_contribution(
    root: &Path,
    environment: &mod_loader::ModEnvironment,
    excluded_packages: &[ExcludedPackage],
    allow_auto_loot_contribution: bool,
) -> Result<String> {
    ensure!(
        excluded_packages.len() <= if allow_auto_loot_contribution { 2 } else { 1 }
            && excluded_packages
                .iter()
                .all(|package| package.id == "xhl-vein-mining"
                    || (allow_auto_loot_contribution
                        && package.id == resource_contribution::PACKAGE_ID)),
        "only exact legacy XHL Vein and reviewed AutoLoot packages may be excluded from this composition"
    );
    let mut excluded_ids = HashSet::new();
    for excluded in excluded_packages {
        ensure!(
            !excluded.id.is_empty()
                && excluded.fingerprint.len() == 64
                && excluded
                    .fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                && excluded_ids.insert(excluded.id.as_str()),
            "invalid or duplicate excluded current-assets package"
        );
        if let Some(item) = environment.mod_registry().get(&excluded.id) {
            ensure!(
                item.info().requires_pregame() && !item.info().enabled,
                "excluded current-assets package must remain disabled and pregame"
            );
            ensure!(
                package_fingerprint(root, item)? == excluded.fingerprint,
                "excluded current-assets package changed"
            );
        }
    }
    let mut inputs = BTreeMap::new();
    inputs.insert("api".to_owned(), crate::API_VERSION.to_owned());
    for item in environment.mod_registry().values() {
        if item.info().id == "esm-real-map" || !item.info().requires_pregame() {
            continue;
        }
        let id = &item.info().id;
        if allow_auto_loot_contribution && id == resource_contribution::PACKAGE_ID {
            inputs.insert(
                format!("resource-contribution/settings/{id}"),
                hash(&serde_json::to_vec(&item.info().setting_values)?),
            );
            continue;
        }
        if excluded_ids.contains(id.as_str()) {
            continue;
        }
        inputs.insert(
            format!("manifest/{id}"),
            hash(&serde_json::to_vec(item.info())?),
        );
        let package = item.fs().root().as_std_path().to_path_buf();
        let relative = package
            .strip_prefix(root)
            .context("asset package outside installation")?
            .to_str()
            .context("non-UTF8 asset package")?
            .replace('\\', "/");
        tree_hashes(root, &relative, &mut inputs, None)?;
    }
    Ok(hash(&serde_json::to_vec(&inputs)?))
}

#[cfg(test)]
fn validate_composition_binding(
    root: &Path,
    environment: &mod_loader::ModEnvironment,
    composition: &Composition,
) -> Result<String> {
    validate_composition_binding_with_contribution(root, environment, composition, false)
}
fn validate_composition_binding_with_contribution(
    root: &Path,
    environment: &mod_loader::ModEnvironment,
    composition: &Composition,
    allow_auto_loot_contribution: bool,
) -> Result<String> {
    if !composition.excluded_packages.is_empty() {
        let excluded_present = composition
            .excluded_packages
            .iter()
            .any(|package| environment.mod_registry().get(&package.id).is_some());
        if excluded_present {
            let source_fingerprint = composition
                .source_fingerprint
                .as_deref()
                .context("excluded package transition is missing its source fingerprint")?;
            ensure!(
                source_fingerprint.len() == 64
                    && source_fingerprint
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit()),
                "invalid current-assets source fingerprint"
            );
            ensure!(
                composition_fingerprint_for(root, environment, &[])? == source_fingerprint,
                "current-assets source package composition changed"
            );
        }
    }
    composition_fingerprint_for_with_contribution(
        root,
        environment,
        &composition.excluded_packages,
        allow_auto_loot_contribution,
    )
}

fn validate_auto_loot_selection(
    environment: &mod_loader::ModEnvironment,
    stem: &str,
    contract: &resource_contribution::Contract,
) -> Result<bool> {
    ensure!(
        contract.target() == stem,
        "AutoLoot resource contribution belongs to another asset target"
    );
    let item = environment
        .mod_registry()
        .get(resource_contribution::PACKAGE_ID)
        .context("selected AutoLootPlus package is missing")?;
    ensure!(
        item.info().requires_pregame(),
        "AutoLootPlus contribution requires a pregame package"
    );
    if item.info().enabled {
        ensure!(
            environment
                .plan(stem == "enshrouded_server", crate::API_VERSION)
                .iter()
                .any(|planned| planned.info().id == resource_contribution::PACKAGE_ID),
            "AutoLootPlus is unavailable in the selected asset target plan"
        );
    }
    Ok(item.info().enabled)
}

fn package_fingerprint(root: &Path, item: &mod_loader::Mod) -> Result<String> {
    let mut inputs = BTreeMap::new();
    inputs.insert(
        "manifest".to_owned(),
        hash(&serde_json::to_vec(item.info())?),
    );
    let package = item.fs().root().as_std_path().to_path_buf();
    let relative = package
        .strip_prefix(root)
        .context("asset package outside installation")?
        .to_str()
        .context("non-UTF8 asset package")?
        .replace('\\', "/");
    tree_hashes(root, &relative, &mut inputs, None)?;
    Ok(hash(&serde_json::to_vec(&inputs)?))
}

fn validate_pending(root: &Path, stage: &Path, next: &Prepared) -> Result<()> {
    validate_shape(root, next)?;
    let stem = next.target.as_deref().unwrap_or(STEM);
    let index_name = format!("{stem}.kfc");
    let resources_name = format!("{stem}.kfc_resources");
    ensure!(plain(stage)?.is_dir(), "pending stage is not a directory");
    let mut staged = HashSet::new();
    for entry in fs::read_dir(stage)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid staged name"))?;
        ensure!(plain(&entry.path())?.is_file(), "non-file in pending stage");
        if valid_output_for(&name, stem) {
            let expected = next
                .outputs
                .get(&name)
                .context("staged output omitted from pending receipt")?;
            ensure!(
                stamp(&entry.path())? == *expected,
                "staged output differs from pending receipt"
            );
            staged.insert(name);
        } else {
            ensure!(
                matches!(
                    name.as_str(),
                    "generated-inputs.json" | "generated-pending.json" | "generated-journal.tmp"
                ),
                "unexpected pending stage entry"
            );
        }
    }
    let journal = stage.join("generated-pending.json");
    if journal.try_exists()? {
        type Journal = (u32, String, Vec<(String, Option<Stamp>, Stamp)>);
        let (version, identity, records): Journal = read_json(&journal)?;
        let transaction_root = fs::canonicalize(root)?.to_string_lossy().into_owned();
        ensure!(
            version == 1 && identity == transaction_root && (2..=512).contains(&records.len()),
            "invalid pending transaction identity/inventory"
        );
        let mut committed = HashSet::new();
        for (name, _, after) in records {
            ensure!(
                valid_output_for(&name, stem) && committed.insert(name.clone()),
                "invalid or duplicate journal output"
            );
            ensure!(
                next.outputs.get(&name) == Some(&after),
                "journal output differs from outer pending receipt"
            );
        }
        ensure!(
            committed.contains(&index_name) && committed.contains(&resources_name),
            "journal required pair missing"
        );
        ensure!(
            staged.is_subset(&committed),
            "staged output omitted from inner journal"
        );
        for (name, expected) in &next.outputs {
            if !committed.contains(name) {
                ensure!(
                    stamp(&joined(root, name)?)? == *expected,
                    "previous output changed during pending recovery"
                );
            }
        }
    } else {
        ensure!(
            staged.contains(&index_name) && staged.contains(&resources_name),
            "pending stage required pair missing"
        );
        for (name, expected) in &next.outputs {
            if !staged.contains(name) {
                ensure!(
                    stamp(&joined(root, name)?)? == *expected,
                    "previous output changed during pending publication"
                );
            }
        }
    }
    Ok(())
}
fn lease(root: &Path) -> Result<fs::File> {
    let path = shroudforge_loader_config::paths::startup_asset_lock(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        Ok(fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)
            .context("another asset preparation is active")?)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        anyhow::bail!("current-assets startup requires a Windows exclusive lease")
    }
}
fn recover(root: &Path) -> Result<()> {
    let stage = transaction::directory(root, STEM);
    let pending = joined(root, PENDING)?;
    if pending.exists() {
        let next: Prepared = read_json(&pending)?;
        validate_shape(root, &next)?;
        ensure!(
            next.resource_contribution.is_none(),
            "resource-contribution recovery requires its exact policy contract"
        );
        if stage.exists() {
            validate_pending(root, &stage, &next)?;
            // A deferred output becomes publishable only after its complete set
            // was validated and recorded by this coordinator.
            if !stage.join("generated-pending.json").exists() {
                transaction::commit_generated(root, STEM)?;
            } else {
                transaction::recover(root, STEM)?;
            }
        }
        verify_outputs(root, &next)?;
        fs::rename(pending, joined(root, STATE)?)?;
    } else if stage.exists() {
        let state = joined(root, STATE)?;
        if state.exists() {
            let current: Prepared = read_json(&state)?;
            ensure!(
                current.resource_contribution.is_none(),
                "resource-contribution recovery requires its exact policy contract"
            );
        }
        ensure!(
            stage.join("generated-inputs.json").exists()
                && !stage.join("generated-pending.json").exists(),
            "unowned or legacy pending asset transaction; explicit reconciliation required"
        );
        transaction::recover(root, STEM)?;
    }
    Ok(())
}

fn refuse_uncontracted_zoom_state(root: &Path) -> Result<()> {
    for name in [STATE, PENDING] {
        let path = joined(root, name)?;
        if path.exists() {
            let receipt: Prepared = read_json(&path)?;
            ensure!(
                receipt.schema_version != 2,
                "Zoom ownership receipt requires its explicit contract; refusing generic recovery"
            );
        }
    }
    Ok(())
}

fn recover_without_zoom_contract(root: &Path) -> Result<()> {
    refuse_uncontracted_zoom_state(root)?;
    recover(root)
}

fn recover_zoom(
    root: &Path,
    fingerprint: &str,
    enabled: bool,
    configured_zoom: bool,
    policy: &Policy,
    registry: &TypeRegistry,
    contribution: Option<&resource_contribution::Contract>,
) -> Result<()> {
    let stage = transaction::directory(root, STEM);
    let pending = joined(root, PENDING)?;
    if pending.exists() {
        let next: Prepared = read_json(&pending)?;
        validate_shape(root, &next)?;
        validate_pending_binding_with_contribution(
            root,
            &next,
            fingerprint,
            enabled,
            configured_zoom,
            policy,
            contribution,
        )?;
        if stage.exists() {
            validate_pending(root, &stage, &next)?;
        }
        verify_pending_selection(root, &stage, &next, policy, registry, contribution)?;
        if stage.exists() {
            if !stage.join("generated-pending.json").exists() {
                transaction::commit_generated(root, STEM)?;
            } else {
                transaction::recover(root, STEM)?;
            }
        }
        verify_outputs(root, &next)?;
        verify_root_selection(root, &next, policy, registry, contribution)?;
        let state_path = joined(root, STATE)?;
        fs::rename(pending, state_path)?;
    } else if stage.exists() {
        ensure!(
            stage.join("generated-inputs.json").exists()
                && !stage.join("generated-pending.json").exists(),
            "unowned or legacy pending asset transaction; explicit reconciliation required"
        );
        ensure!(
            contribution.is_none(),
            "resource-contribution recovery requires its current-assets pending receipt"
        );
        transaction::recover(root, STEM)?;
    }
    Ok(())
}

#[cfg(test)]
fn validate_pending_binding(
    root: &Path,
    next: &Prepared,
    fingerprint: &str,
    enabled: bool,
    configured_zoom: bool,
    policy: &Policy,
) -> Result<()> {
    validate_pending_binding_with_contribution(
        root,
        next,
        fingerprint,
        enabled,
        configured_zoom,
        policy,
        None,
    )
}

fn validate_pending_binding_with_contribution(
    root: &Path,
    next: &Prepared,
    fingerprint: &str,
    enabled: bool,
    configured_zoom: bool,
    policy: &Policy,
    contribution: Option<&resource_contribution::Contract>,
) -> Result<()> {
    ensure!(
        next.schema_version == 2 && next.fingerprint == fingerprint,
        "pending recipe changed; explicit reconciliation required"
    );
    ensure!(
        next.enabled == enabled
            && next.configured_extended_zoom == Some(configured_zoom)
            && next.effective_zoom == Some(enabled && configured_zoom),
        "pending selection changed; explicit reconciliation required"
    );
    if let Some(contract) = contribution {
        let state = next
            .resource_contribution
            .as_ref()
            .context("pending resource-contribution state missing")?;
        ensure!(
            next.target.as_deref() == Some(STEM),
            "pending resource-contribution target mismatch"
        );
        state.validate_contract(contract)?;
    } else {
        ensure!(
            next.resource_contribution.is_none(),
            "pending resource contribution has no matching policy contract"
        );
    }
    let state_path = joined(root, STATE)?;
    let current_hash = stamp(&state_path)?.1;
    let state_is_next = if current_hash == hash(&serde_json::to_vec(next)?) {
        let current: Prepared = read_json(&state_path)?;
        current == *next
    } else {
        if current_hash.as_str() != next.previous_state_sha256.as_deref().unwrap_or_default() {
            false
        } else {
            let previous: Prepared = read_json(&state_path)?;
            validate_shape(root, &previous)?;
            let stage = transaction::directory(root, STEM);
            let changed =
                pending_output_names(root, &stage, next.target.as_deref().unwrap_or(STEM))?;
            let exact_migration_source = policy
                .migration
                .as_ref()
                .is_some_and(|migration| current_hash == migration.state_sha256);
            let exact_layout_source = exact_migration_source && policy.layout_migration.is_some();
            if previous.outputs.keys().eq(next.outputs.keys()) {
                if exact_layout_source {
                    validate_migration_pending(
                        root,
                        &previous,
                        next,
                        policy,
                        &changed,
                        enabled,
                        configured_zoom,
                        fingerprint,
                    )?;
                } else {
                    ensure!(
                        !exact_migration_source,
                        "pinned migration predecessor cannot publish a subset receipt"
                    );
                    for (name, before) in &previous.outputs {
                        let after = next
                            .outputs
                            .get(name)
                            .context("pending transaction omitted a predecessor output")?;
                        ensure!(
                            changed.contains(name) || before == after,
                            "pending transaction changed an output outside its journal: {name}"
                        );
                    }
                }
            } else {
                validate_migration_pending(
                    root,
                    &previous,
                    next,
                    policy,
                    &changed,
                    enabled,
                    configured_zoom,
                    fingerprint,
                )?;
            }
            true
        }
    };
    ensure!(
        state_is_next,
        "pending transaction predecessor changed; explicit reconciliation required"
    );
    Ok(())
}

fn validate_migration_pending(
    root: &Path,
    previous: &Prepared,
    next: &Prepared,
    policy: &Policy,
    changed: &HashSet<String>,
    enabled: bool,
    configured_zoom: bool,
    fingerprint: &str,
) -> Result<()> {
    let migration = policy
        .migration
        .as_ref()
        .context("output inventory change requires an exact reviewed migration")?;
    ensure!(
        policy.zoom.is_some()
            && migration.outputs == policy.composition.outputs
            && next.outputs.keys().eq(migration.outputs.keys())
            && previous
                .outputs
                .keys()
                .all(|name| migration.outputs.contains_key(name))
            && state_outputs_match_composition(&previous.outputs, &migration.outputs),
        "migration pending inventories differ from reviewed source and full baseline"
    );
    validate_migration_inventory(
        root,
        &transaction::directory(root, STEM),
        &migration.outputs,
        &previous.outputs,
        &next.outputs,
        changed,
    )?;
    ensure!(
        stamp(&joined(root, STATE)?)?.1 == migration.state_sha256
            && previous.fingerprint == migration.fingerprint
            && previous.enabled == migration.enabled
            && enabled == migration.enabled
            && configured_zoom == migration.configured_extended_zoom
            && next.previous_state_sha256.as_deref() == Some(migration.state_sha256.as_str())
            && next.fingerprint == fingerprint
            && next.enabled == migration.enabled
            && next.configured_extended_zoom == Some(migration.configured_extended_zoom)
            && next.effective_zoom == Some(migration.effective_zoom)
            && migration.effective_zoom
                == (migration.enabled && migration.configured_extended_zoom),
        "pending migration does not match its exact predecessor and selection"
    );
    if previous.schema_version == 2 {
        ensure!(
            previous.configured_extended_zoom == Some(migration.configured_extended_zoom)
                && previous.effective_zoom == Some(migration.effective_zoom),
            "pending migration source selection differs from reviewed state"
        );
    } else {
        ensure!(
            previous.schema_version == 1,
            "unsupported pending migration source state"
        );
    }
    for (name, reviewed) in &migration.outputs {
        ensure!(
            previous
                .outputs
                .get(name)
                .map_or(true, |owned| owned == reviewed),
            "owned migration stamp differs from the full reviewed baseline: {name}"
        );
        let after = next
            .outputs
            .get(name)
            .context("pending migration omitted a reviewed output")?;
        ensure!(
            changed.contains(name) || after == reviewed,
            "pending migration changed an output outside its transaction: {name}"
        );
    }
    ensure!(
        changed
            .iter()
            .all(|name| matches!(name.as_str(), "enshrouded.kfc" | "enshrouded.kfc_resources")),
        "pending migration journal includes an unrelated output"
    );
    Ok(())
}

/// Validate the complete reviewed inventory before transaction recovery may
/// publish anything. During a journaled transaction, only the two committed
/// KFC outputs may temporarily be absent or hold either receipt after-image.
fn validate_migration_inventory(
    root: &Path,
    stage: &Path,
    reviewed: &BTreeMap<String, Stamp>,
    previous: &BTreeMap<String, Stamp>,
    next: &BTreeMap<String, Stamp>,
    changed: &HashSet<String>,
) -> Result<()> {
    if stage.exists() {
        ensure!(
            plain(stage)?.is_dir(),
            "pending migration stage is not a regular directory"
        );
    }
    let mut actual = HashSet::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str().filter(|name| valid_output(name)) {
            ensure!(plain(&entry.path())?.is_file(), "non-file asset output");
            actual.insert(name.to_owned());
        }
    }
    let reviewed_names: HashSet<_> = reviewed.keys().cloned().collect();
    ensure!(
        actual.is_subset(&reviewed_names),
        "unexpected file in reviewed migration inventory"
    );

    for (name, reviewed_stamp) in reviewed {
        let root_path = joined(root, name)?;
        let stage_path = joined(stage, name)?;
        if !changed.contains(name) {
            ensure!(
                actual.contains(name) && stamp(&root_path)? == *reviewed_stamp,
                "uncommitted migration output differs from full reviewed baseline: {name}"
            );
            continue;
        }

        ensure!(
            matches!(name.as_str(), "enshrouded.kfc" | "enshrouded.kfc_resources"),
            "pending migration changed a non-KFC output: {name}"
        );
        if actual.contains(name) {
            let current = stamp(&root_path)?;
            ensure!(
                previous.get(name) == Some(&current) || next.get(name) == Some(&current),
                "committed migration output is neither its predecessor nor after-image: {name}"
            );
        } else {
            ensure!(
                stage.is_dir()
                    && stamp(&stage_path).is_ok_and(|after| next.get(name) == Some(&after)),
                "missing committed migration output has no staged after-image: {name}"
            );
        }
        if stage_path.exists() {
            ensure!(
                stamp(&stage_path)? == *next.get(name).context("pending after-image missing")?,
                "staged migration output differs from receipt: {name}"
            );
        }
    }
    Ok(())
}

fn pending_output_names(root: &Path, stage: &Path, stem: &str) -> Result<HashSet<String>> {
    let index_name = format!("{stem}.kfc");
    let resources_name = format!("{stem}.kfc_resources");
    if !stage.exists() {
        let previous: Prepared = read_json(&joined(root, STATE)?)?;
        let next: Prepared = read_json(&joined(root, PENDING)?)?;
        let changed: HashSet<_> = previous
            .outputs
            .iter()
            .filter_map(|(name, before)| {
                (next.outputs.get(name) != Some(before)).then(|| name.clone())
            })
            .collect();
        ensure!(
            changed
                .iter()
                .all(|name| name == &index_name || name == &resources_name),
            "completed pending transaction changed non-index outputs"
        );
        return Ok(changed);
    }
    let journal = stage.join("generated-pending.json");
    if journal.exists() {
        type Journal = (u32, String, Vec<(String, Option<Stamp>, Stamp)>);
        let (version, identity, records): Journal = read_json(&journal)?;
        let previous: Prepared = read_json(&joined(root, STATE)?)?;
        let next: Prepared = read_json(&joined(root, PENDING)?)?;
        let transaction_root = fs::canonicalize(root)?.to_string_lossy().into_owned();
        ensure!(
            version == 1 && identity == transaction_root && (2..=512).contains(&records.len()),
            "invalid pending transaction identity/inventory"
        );
        let mut names = HashSet::new();
        for (name, before, after) in records {
            ensure!(
                (name == index_name || name == resources_name) && names.insert(name.clone()),
                "invalid, duplicate, or unrelated generated transaction output"
            );
            ensure!(
                previous.outputs.get(&name) == before.as_ref()
                    && next.outputs.get(&name) == Some(&after),
                "journal predecessor or after-image differs from state receipts: {name}"
            );
        }
        Ok(names)
    } else {
        let previous: Prepared = read_json(&joined(root, STATE)?)?;
        let next: Prepared = read_json(&joined(root, PENDING)?)?;
        let mut names = HashSet::new();
        for entry in fs::read_dir(stage)? {
            let entry = entry?;
            if let Some(name) = entry
                .file_name()
                .to_str()
                .filter(|name| valid_output_for(name, stem))
            {
                ensure!(
                    (name == index_name || name == resources_name)
                        && previous.outputs.get(name) == Some(&stamp(&joined(root, name)?)?)
                        && next.outputs.get(name) == Some(&stamp(&entry.path())?),
                    "staged output is not bound to the transaction predecessor and receipt"
                );
                names.insert(name.to_owned());
            }
        }
        Ok(names)
    }
}
fn tree_hashes(
    root: &Path,
    relative: &str,
    result: &mut BTreeMap<String, String>,
    zoom_package: Option<&str>,
) -> Result<()> {
    let path = joined(root, relative)?;
    if plain(&path)?.is_dir() {
        for e in fs::read_dir(&path)? {
            let e = e?;
            let next = format!(
                "{relative}/{}",
                e.file_name().to_str().context("non-UTF8 package path")?
            );
            tree_hashes(root, &next, result, zoom_package)?;
        }
    } else {
        let digest = if path.file_name().is_some_and(|n| n == "extended.mod.json") {
            let mut value: serde_json::Value = read_json(&path)?;
            value
                .as_object_mut()
                .context("invalid extended manifest")?
                .remove("enabled");
            if zoom_package
                .is_some_and(|package| relative == format!("{package}/extended.mod.json"))
            {
                normalize_zoom_setting(&mut value)?;
            }
            hash(&serde_json::to_vec(&value)?)
        } else {
            stamp(&path)?.1
        };
        result.insert(relative.into(), digest);
    }
    Ok(())
}
fn normalize_zoom_setting(value: &mut serde_json::Value) -> Result<()> {
    let settings = value
        .get_mut("settings")
        .and_then(serde_json::Value::as_object_mut)
        .context("zoom declaration missing")?;
    ensure!(settings.len() == 1, "unknown setting in zoom package");
    let definition = settings
        .get_mut("ExtendedZoom")
        .and_then(serde_json::Value::as_object_mut)
        .context("ExtendedZoom declaration missing")?;
    ensure!(
        definition
            .get("value")
            .is_some_and(serde_json::Value::is_boolean),
        "ExtendedZoom must be boolean"
    );
    ensure!(
        definition.get("applyAt") == Some(&serde_json::json!("restart"))
            && definition.get("restartRequired") == Some(&serde_json::json!(true))
            && definition.get("assetPreparation") == Some(&serde_json::json!(true)),
        "ExtendedZoom must require restart asset preparation"
    );
    definition.remove("value");
    Ok(())
}
fn original(root: &Path, name: &str, expected: &str) -> Result<Vec<u8>> {
    let path = joined(root, &format!("shroudforge/current-assets/{name}"))?;
    ensure!(
        plain(&path)?.len() <= 1024 * 1024,
        "oversized inverse input"
    );
    let bytes = fs::read(path)?;
    ensure!(hash(&bytes) == expected, "inverse source drift");
    Ok(bytes)
}
fn ids(policy: &Policy) -> Result<Vec<ResourceId>> {
    policy
        .resources
        .iter()
        .map(|r| ResourceId::parse(&r.id, r.type_hash, 0).context("invalid resource id"))
        .collect()
}
fn verify_records(root: &Path, policy: &Policy, enabled: bool) -> Result<()> {
    verify_records_for_state(root, policy, enabled, false)
}

fn verify_records_for_state(
    root: &Path,
    policy: &Policy,
    enabled: bool,
    old_layout_records: bool,
) -> Result<()> {
    let reader = KFCReader::new(root, STEM)?;
    if old_layout_records {
        verify_predecessor_resource_reader(reader, policy, enabled)?;
    } else {
        verify_resource_reader(reader, policy, enabled)?;
    }
    let mut reader = KFCReader::new(root, STEM)?.into_cursor()?;
    for c in &policy.retained_contents {
        let bytes = original(root, &c.file, &c.sha256)?;
        ensure!(
            reader
                .read_content(&ContentHash::from_data(&bytes))?
                .as_deref()
                == Some(bytes.as_slice()),
            "inverse content no longer available"
        );
    }
    Ok(())
}

fn verify_predecessor_resource_reader(
    reader: KFCReader,
    policy: &Policy,
    enabled: bool,
) -> Result<()> {
    layout_migration::gpc_applied_digest(policy)?;
    let mut reader = reader.into_cursor()?;
    let gpc = layout_migration::gpc_id();
    for (record, id) in policy.resources.iter().zip(ids(policy)?) {
        let bytes = reader
            .read_resource(&id)?
            .context("selected predecessor resource missing")?;
        let expected = if enabled && id == gpc {
            layout_migration::old_applied_digest()
        } else if enabled {
            &record.applied_sha256
        } else {
            &record.original_sha256
        };
        ensure!(
            hash(&bytes) == expected,
            "selected predecessor resource does not match guarded state"
        );
    }
    Ok(())
}

/// Rebind an older ownership receipt only when every recorded output is a
/// matching subset of the reviewed baseline, the full inventory matches that
/// baseline, and the typed resource records still prove its enabled state.
fn rebind_prepared_state(
    root: &Path,
    policy: &Policy,
    previous: &Prepared,
    fingerprint: &str,
) -> Result<Prepared> {
    rebind_prepared_state_with_records(root, policy, previous, fingerprint, false)
}

fn rebind_prepared_state_with_records(
    root: &Path,
    policy: &Policy,
    previous: &Prepared,
    fingerprint: &str,
    old_layout_records: bool,
) -> Result<Prepared> {
    validate_shape(root, previous)?;
    let baseline = reviewed_migration_baseline(root, policy, previous)?;
    verify_records_for_state(root, policy, previous.enabled, old_layout_records)?;
    let mut rebound = baseline_with_fingerprint(baseline, fingerprint);
    rebound.target = previous.target.clone();
    rebound.resource_contribution = previous.resource_contribution.clone();
    Ok(rebound)
}

fn reviewed_migration_baseline(
    root: &Path,
    policy: &Policy,
    previous: &Prepared,
) -> Result<Prepared> {
    validate_shape(root, previous)?;
    ensure!(
        state_outputs_match_composition(&previous.outputs, &policy.composition.outputs),
        "prepared outputs do not match the reviewed current composition"
    );
    let baseline = Prepared {
        schema_version: 1,
        root: root.to_string_lossy().into(),
        fingerprint: policy.composition.fingerprint.clone(),
        enabled: previous.enabled,
        configured_extended_zoom: None,
        effective_zoom: None,
        previous_state_sha256: None,
        outputs: policy.composition.outputs.clone(),
        target: None,
        resource_contribution: None,
    };
    verify_outputs(root, &baseline)?;
    Ok(baseline)
}

fn baseline_with_fingerprint(mut baseline: Prepared, fingerprint: &str) -> Prepared {
    baseline.fingerprint = fingerprint.to_owned();
    baseline
}
fn verify_resource_records(root: &Path, policy: &Policy, enabled: bool) -> Result<()> {
    verify_resource_reader(KFCReader::new(root, STEM)?, policy, enabled)
}

fn verify_resource_reader(reader: KFCReader, policy: &Policy, enabled: bool) -> Result<()> {
    let mut reader = reader.into_cursor()?;
    for (r, id) in policy.resources.iter().zip(ids(policy)?) {
        let bytes = reader
            .read_resource(&id)?
            .context("selected resource missing")?;
        ensure!(
            hash(&bytes).as_str()
                == if enabled {
                    r.applied_sha256.as_str()
                } else {
                    r.original_sha256.as_str()
                },
            "selected resource does not match guarded state"
        );
    }
    Ok(())
}

fn afterimage_reader(root: &Path, stage: &Path, next: &Prepared) -> Result<KFCReader> {
    let (index, resources) = afterimage_paths(root, stage, next)?;
    KFCReader::new_with_resource_path(
        index.parent().context("pending index parent absent")?,
        STEM,
        resources,
    )
    .map_err(anyhow::Error::from)
}

fn afterimage_paths(root: &Path, stage: &Path, next: &Prepared) -> Result<(PathBuf, PathBuf)> {
    let index_stage = joined(stage, "enshrouded.kfc")?;
    let index = if index_stage.exists() {
        index_stage
    } else {
        joined(root, "enshrouded.kfc")?
    };
    let resources_stage = joined(stage, "enshrouded.kfc_resources")?;
    let resources = if resources_stage.exists() {
        resources_stage
    } else {
        joined(root, "enshrouded.kfc_resources")?
    };
    for (name, path) in [
        ("enshrouded.kfc", &index),
        ("enshrouded.kfc_resources", &resources),
    ] {
        ensure!(
            next.outputs.get(name) == Some(&stamp(path)?),
            "pending logical after-image mismatch: {name}"
        );
    }
    Ok((index, resources))
}

fn verify_reach_profile(
    root: &Path,
    reader: KFCReader,
    registry: &TypeRegistry,
    policy: &Policy,
) -> Result<()> {
    const LEGACY_LUA: &str = "1cbabd125b4204c1165ca0748b2fe627f4f88fa0e2ded2f31f13af42473a5d7d";
    const REFERENCE_INDEX: &str =
        "54a8cb76b3229c486ef55a7ababcd48828fa0ffc5bb144d206f687fd0ab00a92";
    const IDS: [&str; 7] = [
        "59dba0f3-81b8-4ac5-88d7-ba97f4d2928c",
        "46ea9b46-445a-4e36-bf84-20cab7585f48",
        "70ecc7fb-6ed1-416c-a51e-0362b2ebdd1a",
        "c03c05ce-4cc0-40fd-b201-58d05c730767",
        "2e4b3166-4960-48b1-ad1f-f3bb9d0d981a",
        "aaff198b-9139-47f5-8e41-b1ce0262abc8",
        "aee6b3ba-adeb-4264-b117-25f974a08cb8",
    ];
    let reference = &policy.legacy_pickaxe_reach;
    let bytes = original(root, &reference.file, &reference.sha256)?;
    let reference: ReachReference = serde_json::from_slice(&bytes)?;
    ensure!(
        reference.schema_version == 1
            && reference.generation == policy.generation
            && reference.legacy_lua_sha256 == LEGACY_LUA
            && reference.reference_index_sha256 == REFERENCE_INDEX
            && reference.records.len() == IDS.len(),
        "unrecognized legacy reach reference identity"
    );
    let ty = registry
        .get_by_name(LookupKey::Qualified("keen::ItemInfo"))
        .context("ItemInfo type unavailable for pending recovery")?;
    let mut cursor = reader.into_cursor()?;
    let mut seen = HashSet::new();
    for record in &reference.records {
        ensure!(
            IDS.contains(&record.id.as_str())
                && seen.insert(record.id.as_str())
                && record.original_max_distance == 5.0
                && valid_sha256(&record.reference_resource_sha256),
            "invalid legacy reach after-image reference"
        );
        let id = ResourceId::parse(&record.id, ty.qualified_hash, 0)
            .context("invalid legacy reach after-image ID")?;
        let bytes = cursor
            .read_resource(&id)?
            .context("legacy reach after-image missing")?;
        let value = Value::from_bytes(registry, ty, &bytes)?;
        let equipment = value
            .as_struct()
            .and_then(|value| value.get("equipment"))
            .and_then(Value::as_struct)
            .context("legacy reach after-image equipment missing")?;
        ensure!(
            equipment.get("maxDistance").and_then(Value::as_f64) == Some(5.0),
            "legacy reach after-image is not migrated"
        );
    }
    ensure!(
        seen.len() == IDS.len(),
        "incomplete legacy reach after-image"
    );
    Ok(())
}

fn verify_pending_selection(
    root: &Path,
    stage: &Path,
    next: &Prepared,
    policy: &Policy,
    registry: &TypeRegistry,
    contribution: Option<&resource_contribution::Contract>,
) -> Result<()> {
    let reader = || afterimage_reader(root, stage, next);
    ensure!(
        reader()?.file().game_version() == policy.generation,
        "pending resource generation mismatch"
    );
    verify_resource_reader(reader()?, policy, next.enabled)?;
    zoom::verify_profile(
        reader()?,
        registry,
        next.effective_zoom
            .context("pending effective zoom absent")?,
    )?;
    verify_reach_profile(root, reader()?, registry, policy)?;
    verify_pending_contribution(root, stage, next, contribution)
}

fn verify_root_selection(
    root: &Path,
    next: &Prepared,
    policy: &Policy,
    registry: &TypeRegistry,
    contribution: Option<&resource_contribution::Contract>,
) -> Result<()> {
    let reader = || KFCReader::new(root, STEM).map_err(anyhow::Error::from);
    ensure!(
        reader()?.file().game_version() == policy.generation,
        "published resource generation mismatch"
    );
    verify_resource_records(root, policy, next.enabled)?;
    zoom::verify_profile(
        reader()?,
        registry,
        next.effective_zoom
            .context("published effective zoom absent")?,
    )?;
    verify_reach_profile(root, reader()?, registry, policy)?;
    if let Some(contract) = contribution {
        let state = next
            .resource_contribution
            .as_ref()
            .context("prepared resource-contribution state missing")?;
        state.verify_current(root, STEM, contract, state.enabled())?;
    }
    Ok(())
}

fn contribution_afterimage_reader(root: &Path, stage: &Path, stem: &str) -> Result<KFCReader> {
    let index_name = format!("{stem}.kfc");
    let resource_name = format!("{stem}.kfc_resources");
    let staged_index = joined(stage, &index_name)?;
    let index = if staged_index.exists() {
        staged_index
    } else {
        joined(root, &index_name)?
    };
    let staged_resources = joined(stage, &resource_name)?;
    let resources = if staged_resources.exists() {
        staged_resources
    } else {
        joined(root, &resource_name)?
    };
    KFCReader::new_with_resource_path(
        index
            .parent()
            .context("contribution index parent missing")?,
        stem,
        resources,
    )
    .map_err(anyhow::Error::from)
}

fn verify_pending_contribution(
    root: &Path,
    stage: &Path,
    next: &Prepared,
    contract: Option<&resource_contribution::Contract>,
) -> Result<()> {
    match contract {
        Some(contract) => {
            let stem = next.target.as_deref().unwrap_or(STEM);
            ensure!(
                contract.target() == stem,
                "pending resource-contribution target mismatch"
            );
            let state = next
                .resource_contribution
                .as_ref()
                .context("pending resource-contribution state missing")?;
            state.verify_afterimage(
                contribution_afterimage_reader(root, stage, stem)?,
                stem,
                contract,
                state.enabled(),
            )
        }
        None => {
            ensure!(
                next.resource_contribution.is_none(),
                "pending resource contribution has no matching policy contract"
            );
            Ok(())
        }
    }
}

/// Recover one exact resource contribution from the shared outer receipt and
/// generated transaction. A contribution stage without that receipt is never
/// forwarded by the generic game-asset recovery path.
fn recover_contribution_transaction(
    root: &Path,
    stem: &str,
    fingerprint: &str,
    selected: bool,
    parent_enabled: Option<bool>,
    contract: &resource_contribution::Contract,
    policy: Option<&Policy>,
    registry: Option<&TypeRegistry>,
) -> Result<()> {
    ensure!(
        contract.target() == stem,
        "resource-contribution target mismatch"
    );
    let stage = transaction::directory(root, stem);
    let pending_path = joined(root, PENDING)?;
    if !pending_path.exists() {
        ensure!(
            !stage.exists(),
            "resource-contribution recovery requires its current-assets pending receipt"
        );
        return Ok(());
    }

    let next: Prepared = read_json(&pending_path)?;
    validate_shape(root, &next)?;
    ensure!(
        next.target.as_deref() == Some(stem)
            && next.fingerprint == fingerprint
            && parent_enabled.is_none_or(|enabled| next.enabled == enabled),
        "pending contribution target, recipe or parent selection changed"
    );
    let next_state = next
        .resource_contribution
        .as_ref()
        .context("pending resource-contribution state missing")?;
    next_state.validate_contract(contract)?;
    ensure!(
        next_state.enabled() == selected,
        "pending resource-contribution selection changed"
    );

    let state_path = joined(root, STATE)?;
    let next_hash = hash(&serde_json::to_vec(&next)?);
    if state_path.exists() {
        let current_hash = stamp(&state_path)?.1;
        ensure!(
            current_hash == next_hash
                || next_state
                    .previous_state_sha256()
                    .is_some_and(|previous| previous == current_hash),
            "resource-contribution pending predecessor changed"
        );
    } else {
        ensure!(
            next_state.previous_state_sha256().is_none(),
            "resource-contribution pending predecessor receipt is missing"
        );
    }

    if stage.exists() {
        validate_pending(root, &stage, &next)?;
    }
    verify_pending_contribution(root, &stage, &next, Some(contract))?;
    if let Some(policy) = policy {
        let registry = registry.context("current-assets recovery registry missing")?;
        let reader = contribution_afterimage_reader(root, &stage, stem)?;
        ensure!(
            reader.file().game_version() == policy.generation,
            "pending resource generation mismatch"
        );
        verify_resource_reader(reader, policy, next.enabled)?;
        verify_reach_profile(
            root,
            contribution_afterimage_reader(root, &stage, stem)?,
            registry,
            policy,
        )?;
    }

    if stage.exists() {
        if !stage.join("generated-pending.json").exists() {
            transaction::commit_generated(root, stem)?;
        } else {
            transaction::recover(root, stem)?;
        }
    }
    verify_outputs(root, &next)?;
    next_state.verify_current(root, stem, contract, selected)?;
    if let Some(policy) = policy {
        let registry = registry.context("current-assets recovery registry missing")?;
        verify_resource_records(root, policy, next.enabled)?;
        verify_reach_profile(root, KFCReader::new(root, stem)?, registry, policy)?;
    }

    if state_path.exists() && stamp(&state_path)?.1 == next_hash {
        fs::remove_file(pending_path)?;
    } else {
        fs::rename(pending_path, state_path)?;
    }
    Ok(())
}

fn inverse(
    root: &Path,
    policy: &Policy,
    old_layout_records: bool,
    zoom_change: Option<&zoom::Change>,
    preferred_registry: Option<&TypeRegistry>,
) -> Result<()> {
    verify_records_for_state(root, policy, true, old_layout_records)?;
    let before = KFCFile::from_path(root.join("enshrouded.kfc"), false)?;
    let fallback_registry;
    let registry = if let Some(registry) = preferred_registry {
        registry
    } else {
        fallback_registry = crate::startup::remove_legacy_pickaxe_reach::registry(root)?;
        &fallback_registry
    };
    let stage = transaction::begin_generated(root, STEM)?;
    let mut writer = KFCWriter::new_generated_incremental(root, &stage, STEM, &before, registry)?;
    let mut expected = Vec::new();
    for (r, id) in policy.resources.iter().zip(ids(policy)?) {
        let bytes = original(root, &r.original, &r.original_sha256)?;
        writer.write_resource(&id, &bytes)?;
        expected.push((id, bytes));
    }
    if let Some(change) = zoom_change {
        writer.write_resource(&zoom::id(), &change.bytes)?;
        expected.push((zoom::id(), change.bytes.clone()));
    }
    writer.finalize()?;
    validate_resource_edits(&stage, &before, &expected)?;
    if let Some(change) = zoom_change {
        change.verify(&stage, registry)?;
    }
    Ok(())
}

fn validate_resource_edits(
    stage: &Path,
    before: &KFCFile,
    expected: &[(ResourceId, Vec<u8>)],
) -> Result<()> {
    let changed: HashSet<_> = expected.iter().map(|(id, _)| *id).collect();
    let after = KFCFile::from_path(stage.join("enshrouded.kfc"), false)?;
    ensure!(
        before.containers().len() == after.containers().len(),
        "inverse changed container inventory"
    );
    for (a, b) in before.containers().iter().zip(after.containers()) {
        ensure!(
            (a.size, a.count) == (b.size, b.count),
            "inverse changed existing container metadata"
        );
    }
    ensure!(
        after.resource_chunks().len() >= before.resource_chunks().len(),
        "inverse removed chunks"
    );
    for (a, b) in before.resource_chunks().iter().zip(after.resource_chunks()) {
        ensure!(
            (
                a.offset,
                a.size,
                a.compressed_size,
                a.uncompressed_offset,
                a.uncompressed_size
            ) == (
                b.offset,
                b.size,
                b.compressed_size,
                b.uncompressed_offset,
                b.uncompressed_size
            ),
            "inverse changed existing chunk metadata"
        );
    }
    let mut membership: std::collections::HashMap<_, HashSet<_>> = std::collections::HashMap::new();
    for id in after.resources().keys() {
        membership.entry(id.type_hash()).or_default().insert(*id);
    }
    for (kind, expected) in membership {
        ensure!(
            after
                .resources_by_type(kind)
                .copied()
                .collect::<HashSet<_>>()
                == expected,
            "inverse resource bundle mismatch"
        );
    }
    ensure!(
        before.game_version() == after.game_version()
            && before.resources().len() == after.resources().len()
            && before.contents().len() == after.contents().len(),
        "inverse changed unrelated asset inventory"
    );
    for (id, a) in before.resources().iter() {
        let b = after
            .resources()
            .get(id)
            .context("inverse removed resource")?;
        ensure!(
            changed.contains(id) || (a.offset, a.size) == (b.offset, b.size),
            "inverse changed unrelated resource"
        );
    }
    for (id, a) in before.contents().iter() {
        let b = after
            .contents()
            .get(id)
            .context("inverse removed content")?;
        ensure!(
            (a.offset, a.flags, a.container_index) == (b.offset, b.flags, b.container_index),
            "inverse changed content mapping"
        );
    }
    let mut reader = KFCReader::new(&stage, STEM)?.into_cursor()?;
    for (id, bytes) in expected {
        ensure!(
            reader
                .read_resource(id)?
                .context("edited resource missing")?
                == bytes.as_slice(),
            "resource edit readback mismatch"
        );
    }
    Ok(())
}

fn append_resource_edits(
    root: &Path,
    expected: &[(ResourceId, Vec<u8>)],
    preferred_registry: Option<&TypeRegistry>,
) -> Result<()> {
    let stage = transaction::directory(root, STEM);
    let creating = !stage.exists();
    if creating {
        transaction::begin_generated(root, STEM)?;
    }
    let before = KFCFile::from_path(
        if creating { root } else { &stage }.join("enshrouded.kfc"),
        false,
    )?;
    let fallback_registry;
    let registry = if let Some(registry) = preferred_registry {
        registry
    } else {
        fallback_registry = crate::startup::remove_legacy_pickaxe_reach::registry(root)?;
        &fallback_registry
    };
    let mut writer = if creating {
        KFCWriter::new_generated_incremental(root, &stage, STEM, &before, registry)?
    } else {
        // This is already a validated edited output owned by this transaction.
        KFCWriter::new_incremental_with_options(
            &stage,
            STEM,
            &before,
            registry,
            KFCWriteOptions {
                append_after_resource_file: true,
                ..Default::default()
            },
        )?
    };
    for (id, bytes) in expected {
        writer.write_resource(id, bytes)?;
    }
    writer.finalize()?;
    validate_resource_edits(&stage, &before, expected)
}

fn append_resource_mapping_inverse(
    root: &Path,
    stem: &str,
    state: &resource_contribution::State,
    contract: &resource_contribution::Contract,
    registry: &TypeRegistry,
) -> Result<resource_contribution::State> {
    state.verify_current(root, stem, contract, true)?;
    let stage = transaction::directory(root, stem);
    let creating = !stage.exists();
    if creating {
        transaction::begin_generated(root, stem)?;
    }
    let index = if creating {
        root.join(format!("{stem}.kfc"))
    } else {
        stage.join(format!("{stem}.kfc"))
    };
    let before = KFCFile::from_path(index, false)?;
    let mut writer = if creating {
        KFCWriter::new_generated_incremental(root, &stage, stem, &before, registry)?
    } else {
        KFCWriter::new_incremental_with_options(
            &stage,
            stem,
            &before,
            registry,
            KFCWriteOptions {
                append_after_resource_file: true,
                ..Default::default()
            },
        )?
    };
    state.write_inverse(&mut writer, contract)?;
    writer.finalize_without_resource_data()?;

    let after = KFCFile::from_path(stage.join(format!("{stem}.kfc")), false)?;
    let mut before_reader = KFCReader::new(root, stem)?.into_cursor()?;
    let mut after_reader = KFCReader::new(&stage, stem)?.into_cursor()?;
    let next = resource_contribution::verify_inverse(
        &before,
        &after,
        &mut before_reader,
        &mut after_reader,
        contract,
        state,
        registry,
    )?;
    next.verify_current(&stage, stem, contract, false)?;
    Ok(next)
}

/// Returns false only when this explicit policy is absent. Errors must stop
/// startup; callers must never recover them through the legacy backup path.
pub fn prepare(root: &Path) -> Result<bool> {
    prepare_for(root, STEM)
}

/// Prepare the exact client or dedicated-server target using the shared
/// current-assets lease and transaction coordinator.
pub fn prepare_for(root: &Path, stem: &str) -> Result<bool> {
    zoom_demand::clear();
    ensure!(
        matches!(stem, "enshrouded" | "enshrouded_server"),
        "unsupported current-assets asset target"
    );
    let policy_path = joined(root, POLICY)?;
    if !policy_path.try_exists()? {
        ensure!(
            !active_for(root, stem)?,
            "current-assets policy missing from an owned installation"
        );
        return Ok(false);
    }
    plain(root)?;
    let root = fs::canonicalize(root)?;
    let policy_path = joined(&root, POLICY)?;
    let loader =
        shroudforge_loader_config::config::read_loader(&root).map_err(anyhow::Error::msg)?;
    validate_owned_loader_paths(&root, &loader)?;
    let _lease = lease(&root)?;
    if stem == "enshrouded_server" {
        let handled = resource_contribution::prepare_dedicated(&root, &policy_path)?;
        return Ok(publish_dedicated_contribution_demand(&root, handled));
    }
    ensure!(stem == STEM, "client current-assets target mismatch");
    prepare_client(&root, &policy_path)
}

fn publish_dedicated_contribution_demand(root: &Path, handled: bool) -> bool {
    if handled {
        zoom_demand::publish(root, false, "");
    }
    handled
}

fn validate_owned_loader_paths(root: &Path, loader: &serde_json::Value) -> Result<()> {
    for (key, relative) in [
        ("mods", "mods"),
        ("cache", "shroudforge/cache"),
        ("runtime", "shroudforge/runtime"),
    ] {
        let expected = joined(&root, relative)?;
        let configured = loader
            .pointer(&format!("/paths/{key}"))
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| expected.clone());
        let configured = if configured.is_absolute() {
            configured
        } else {
            root.join(configured)
        };
        let configured = if configured.exists() {
            fs::canonicalize(configured)?
        } else {
            configured
        };
        ensure!(
            configured == expected,
            "current-assets {key} path belongs to another installation"
        );
    }
    Ok(())
}

fn validate_contribution_disjoint(
    root: &Path,
    policy: &Policy,
    contract: &resource_contribution::Contract,
    registry: &TypeRegistry,
) -> Result<()> {
    let mut protected: HashSet<ResourceId> = ids(policy)?.into_iter().collect();
    if policy.zoom.is_some() {
        protected.insert(zoom::id());
    }
    let reference_bytes = original(
        root,
        &policy.legacy_pickaxe_reach.file,
        &policy.legacy_pickaxe_reach.sha256,
    )?;
    let reference: ReachReference = serde_json::from_slice(&reference_bytes)?;
    let item_info = registry
        .get_by_name(LookupKey::Qualified("keen::ItemInfo"))
        .context("ItemInfo type unavailable for contribution overlap checks")?;
    for record in reference.records {
        protected.insert(
            ResourceId::parse(&record.id, item_info.qualified_hash, 0)
                .context("invalid legacy reach resource ID")?,
        );
    }
    let contribution_ids: HashSet<_> = contract.ids(registry)?.into_iter().collect();
    ensure!(
        contribution_ids.is_disjoint(&protected),
        "AutoLoot resource contribution overlaps a selected HD, zoom or legacy-reach resource"
    );
    Ok(())
}

fn prepare_client(root: &Path, policy_path: &Path) -> Result<bool> {
    let policy: Policy = read_json(&policy_path)?;
    ensure!(
        policy.schema_version
            == if policy.resource_contribution.is_some() {
                3
            } else {
                2
            }
            && policy.mod_id == "esm-real-map"
            && policy.package == "mods/esm-real-map"
            && policy.resources.len() == 2
            && policy.retained_contents.len() == 2,
        "unsupported current-assets recipe"
    );
    validate_migration(&policy)?;
    let selection = policy.zoom.is_some();
    let contribution_contract = policy.resource_contribution.as_ref();
    if selection {
        ensure!(
            policy.generation == zoom::GENERATION,
            "unsupported zoom generation"
        );
        let actual: HashSet<_> = ids(&policy)?.into_iter().collect();
        let expected: HashSet<_> = [
            ResourceId::parse("01bcbd07-bdbf-41c0-9999-5d58fb1a3aa1", 0xf9e86997, 0).unwrap(),
            ResourceId::parse("7df551f7-ddd1-52b0-8ba6-d09be35dbccc", 0x647628d6, 0).unwrap(),
        ]
        .into_iter()
        .collect();
        ensure!(actual == expected, "unsupported map resource selection");
    }
    ensure!(
        stamp(&root.join("enshrouded.exe"))?.1 == policy.executable_sha256,
        "current-assets executable mismatch"
    );
    let environment = mod_loader::ModEnvironment::load(root.to_str().context("non-UTF8 root")?)
        .map_err(|_| anyhow::anyhow!("could not load current-assets mod environment"))?;
    ensure!(
        validate_composition_binding_with_contribution(
            &root,
            &environment,
            &policy.composition,
            policy.resource_contribution.is_some(),
        )? == policy.composition.fingerprint,
        "non-HD asset mod selection, settings or package changed; explicit composition migration required"
    );
    // Preserve the legacy recovery and validation order byte-for-byte when no
    // explicit zoom contract is present. Zoom recovery waits for the normalized
    // recipe and current selection so a stale transaction cannot outrun them.
    let legacy_index = if selection || contribution_contract.is_some() {
        None
    } else {
        recover_without_zoom_contract(&root)?;
        Some(KFCFile::from_path(root.join("enshrouded.kfc"), false)?)
    };
    if let Some(index) = &legacy_index {
        ensure!(
            index.game_version() == policy.generation,
            "current-assets generation mismatch"
        );
    }
    let item = environment
        .mod_registry()
        .get(&policy.mod_id)
        .context("selected package missing")?;
    let enabled = item.info().enabled;
    let contribution_enabled = contribution_contract
        .map(|contract| validate_auto_loot_selection(&environment, STEM, contract))
        .transpose()?;
    if enabled {
        ensure!(
            environment
                .plan(false, crate::API_VERSION)
                .iter()
                .any(|m| m.info().id == policy.mod_id),
            "selected package is unavailable in client plan"
        );
    }
    let mut inputs = BTreeMap::new();
    tree_hashes(
        &root,
        &policy.package,
        &mut inputs,
        selection.then_some(policy.package.as_str()),
    )?;
    if policy.layout_migration.is_some() {
        layout_migration::package(&inputs)?;
    }
    let layout_contract = policy.layout_migration.is_some();
    inputs.insert("policy".into(), stamp(&policy_path)?.1);
    let mut settings = item.info().setting_values.clone();
    let configured_zoom = if selection {
        let settings = settings
            .as_object_mut()
            .context("invalid resolved zoom settings")?;
        ensure!(settings.len() == 1, "unknown resolved zoom setting");
        Some(
            settings
                .remove("ExtendedZoom")
                .and_then(|value| value.as_bool())
                .context("ExtendedZoom must resolve to boolean")?,
        )
    } else {
        None
    };
    let desired_zoom = enabled && configured_zoom.unwrap_or(false);
    inputs.insert("settings".into(), hash(&serde_json::to_vec(&settings)?));
    if let Some(contract) = contribution_contract {
        inputs.insert("resource-contribution".into(), contract.digest()?);
    }
    for r in &policy.resources {
        original(&root, &r.original, &r.original_sha256)?;
    }
    for c in &policy.retained_contents {
        original(&root, &c.file, &c.sha256)?;
    }
    let fingerprint = hash(&serde_json::to_vec(&inputs)?);
    let registry = if let Some(contract) = policy.zoom.as_ref() {
        Some(zoom::registry(&root, contract, &policy.executable_sha256)?)
    } else if contribution_contract.is_some() {
        Some(crate::startup::remove_legacy_pickaxe_reach::registry(
            &root,
        )?)
    } else {
        None
    };
    if let Some(contract) = contribution_contract {
        contract.validate(
            &root,
            STEM,
            registry
                .as_ref()
                .context("resource type registry missing")?,
        )?;
    }
    if let Some(registry) = registry.as_ref() {
        if selection {
            recover_zoom(
                &root,
                &fingerprint,
                enabled,
                configured_zoom.context("zoom selection missing")?,
                &policy,
                registry,
                contribution_contract,
            )?;
        } else if let (Some(contract), Some(selected)) =
            (contribution_contract, contribution_enabled)
        {
            recover_contribution_transaction(
                &root,
                STEM,
                &fingerprint,
                selected,
                Some(enabled),
                contract,
                Some(&policy),
                Some(registry),
            )?;
        }
    }
    let game_version = if let Some(index) = legacy_index {
        index.game_version().to_owned()
    } else {
        KFCFile::from_path(root.join("enshrouded.kfc"), false)?
            .game_version()
            .to_owned()
    };
    ensure!(
        game_version == policy.generation,
        "current-assets generation mismatch"
    );
    let state_path = joined(&root, STATE)?;
    let mut state_rebound = false;
    let previous = if state_path.exists() {
        let state: Prepared = read_json(&state_path)?;
        ensure!(
            contribution_contract.is_some() || state.resource_contribution.is_none(),
            "prepared AutoLoot state has no matching resource-contribution policy"
        );
        if selection {
            if state.schema_version == 2 && state.fingerprint == fingerprint {
                verify_outputs(&root, &state)?;
                Some(state)
            } else {
                let migration = policy
                    .migration
                    .as_ref()
                    .context("zoom successor requires an exact reviewed migration")?;
                let rebound = validate_migration_source(
                    &root,
                    &state_path,
                    &state,
                    &policy,
                    migration,
                    enabled,
                    configured_zoom.context("zoom selection missing")?,
                    &fingerprint,
                    layout_contract,
                )?;
                state_rebound = true;
                Some(rebound)
            }
        } else if state.fingerprint == fingerprint {
            verify_outputs(&root, &state)?;
            Some(state)
        } else {
            let rebound = rebind_prepared_state(&root, &policy, &state, &fingerprint)
                .context("prepared state differs from its recipe and cannot be safely rebound")?;
            state_rebound = true;
            Some(rebound)
        }
    } else {
        ensure!(
            !selection,
            "zoom successor requires an exact reviewed state; automatic capture is disabled"
        );
        let baseline = Prepared {
            schema_version: 1,
            root: root.to_string_lossy().into(),
            fingerprint: policy.composition.fingerprint.clone(),
            enabled: policy.composition.baseline_enabled,
            configured_extended_zoom: None,
            effective_zoom: None,
            previous_state_sha256: None,
            outputs: policy.composition.outputs.clone(),
            target: None,
            resource_contribution: None,
        };
        verify_outputs(&root, &baseline)?;
        verify_inventory(&root, &baseline.outputs)?;
        None
    };
    let mut contribution_state = if let Some(contract) = contribution_contract {
        let state = if let Some(previous) = previous.as_ref() {
            if let Some(state) = previous.resource_contribution.as_ref() {
                state.validate_contract(contract)?;
                state.clone()
            } else {
                ensure!(
                    previous.outputs == policy.composition.outputs,
                    "AutoLoot contribution requires the exact reviewed predecessor output inventory"
                );
                contract.initial_state(false)?
            }
        } else {
            contract.initial_state(false)?
        };
        state.verify_current(&root, STEM, contract, state.enabled())?;
        Some(state)
    } else {
        None
    };
    if let (Some(contract), Some(registry)) = (contribution_contract, registry.as_ref()) {
        validate_contribution_disjoint(&root, &policy, contract, registry)?;
    }
    let was_enabled = previous
        .as_ref()
        .map(|state| state.enabled)
        .unwrap_or(policy.composition.baseline_enabled);
    let layout_transition = state_rebound && layout_contract;
    let was_zoom = if state_rebound && selection {
        policy
            .migration
            .as_ref()
            .context("reviewed migration missing")?
            .effective_zoom
    } else {
        previous
            .as_ref()
            .and_then(|state| state.effective_zoom)
            .unwrap_or(false)
    };
    let reference = &policy.legacy_pickaxe_reach;
    let bytes = original(&root, &reference.file, &reference.sha256)?;
    let reference = serde_json::from_slice(&bytes)?;
    let reach_already_migrated = if state_rebound {
        policy.composition.baseline_legacy_pickaxe_reach_migrated
    } else {
        previous.is_some() || policy.composition.baseline_legacy_pickaxe_reach_migrated
    };
    let reach_edits = crate::startup::remove_legacy_pickaxe_reach::plan(
        &root,
        &policy.generation,
        &reference,
        reach_already_migrated,
    )?;
    verify_records_for_state(&root, &policy, was_enabled, layout_transition)?;
    let zoom_change = registry
        .as_ref()
        .map(|registry| zoom::Change::new(&root, registry, was_zoom, desired_zoom))
        .transpose()?;
    let zoom_changed = selection && was_zoom != desired_zoom;
    let contribution_was_enabled = contribution_state
        .as_ref()
        .is_some_and(resource_contribution::State::enabled);
    let contribution_changed =
        contribution_enabled.is_some_and(|selected| selected != contribution_was_enabled);
    ensure!(
        !contribution_changed || (reach_edits.is_empty() && !zoom_changed && !layout_transition),
        "AutoLoot selection cannot transition in the same transaction as reach, zoom or layout migration"
    );
    let already_current = previous.as_ref().is_some_and(|state| {
        state.schema_version == if selection { 2 } else { 1 }
            && state.fingerprint == fingerprint
            && state.enabled == enabled
            && state.configured_extended_zoom == configured_zoom
            && state.resource_contribution == contribution_state
    });
    ensure!(
        !contribution_changed || enabled == was_enabled,
        "AutoLoot selection cannot transition in the same transaction as the parent HD Map"
    );
    if !selection
        && was_enabled == enabled
        && reach_edits.is_empty()
        && contribution_contract.is_none()
    {
        if previous.is_none() {
            let baseline = Prepared {
                schema_version: 1,
                root: root.to_string_lossy().into(),
                fingerprint: fingerprint.clone(),
                enabled,
                configured_extended_zoom: None,
                effective_zoom: None,
                previous_state_sha256: None,
                outputs: policy.composition.outputs.clone(),
                target: None,
                resource_contribution: None,
            };
            verify_outputs(&root, &baseline)?;
            write_json(&state_path, &baseline)?;
        } else if state_rebound {
            write_json(
                &state_path,
                previous.as_ref().expect("rebound state exists"),
            )?;
        }
        zoom_demand::publish(&root, false, &policy.executable_sha256);
        return Ok(true);
    }
    if already_current
        && was_enabled == enabled
        && reach_edits.is_empty()
        && !zoom_changed
        && !contribution_changed
    {
        if let (Some(registry), Some(state)) = (registry.as_ref(), previous.as_ref()) {
            zoom::verify_profile(
                KFCReader::new(&root, STEM)?,
                registry,
                state
                    .effective_zoom
                    .context("prepared effective zoom absent")?,
            )?;
        }
        zoom_demand::publish(&root, desired_zoom, &policy.executable_sha256);
        return Ok(true);
    }
    if contribution_changed {
        let contract = contribution_contract.context("resource-contribution contract missing")?;
        let current = contribution_state
            .as_ref()
            .context("resource-contribution state missing")?;
        let selected = contribution_enabled.context("resource-contribution selection missing")?;
        let updated = resource_contribution::apply_selection(
            &root,
            STEM,
            contract,
            current,
            registry
                .as_ref()
                .context("resource type registry missing")?,
            selected,
            || {
                crate::run_with_local_schema(
                    &environment,
                    crate::RunArgs {
                        file_name: STEM.into(),
                        options: crate::RunOptions {
                            current_asset_mods: Some(vec![
                                resource_contribution::PACKAGE_ID.into(),
                            ]),
                            defer_current_commit: true,
                            patch: true,
                            force_patch: true,
                            export: false,
                            is_server: Some(false),
                            ..Default::default()
                        },
                    },
                )
            },
        )?;
        let predecessor = if state_path.exists() {
            Some(stamp(&state_path)?.1)
        } else {
            None
        };
        let mut updated = updated;
        updated.set_previous_state_sha256(predecessor);
        contribution_state = Some(updated);
    }
    if enabled && !was_enabled {
        crate::run_with_local_schema(
            &environment,
            crate::RunArgs {
                file_name: STEM.into(),
                options: crate::RunOptions {
                    current_asset_mods: Some(vec![policy.mod_id.clone()]),
                    defer_current_commit: true,
                    patch: true,
                    force_patch: true,
                    is_server: Some(false),
                    ..Default::default()
                },
            },
        )?;
    } else if !enabled && was_enabled {
        inverse(
            &root,
            &policy,
            layout_transition,
            if zoom_changed {
                zoom_change.as_ref()
            } else {
                None
            },
            registry.as_ref(),
        )?;
    }
    let mut resource_edits = reach_edits;
    if layout_transition && was_enabled && enabled {
        resource_edits.push((
            layout_migration::gpc_id(),
            layout_migration::correction(&root)?,
        ));
    }
    if zoom_changed {
        let change = zoom_change
            .as_ref()
            .context("zoom transition is missing its typed change")?;
        resource_edits.push((zoom::id(), change.bytes.clone()));
    }
    if !resource_edits.is_empty() {
        append_resource_edits(&root, &resource_edits, registry.as_ref())?;
    }
    let stage = transaction::directory(&root, STEM);
    let mut outputs = previous
        .map(|p| p.outputs)
        .unwrap_or_else(|| policy.composition.outputs.clone());
    if stage.exists() {
        verify_resource_records(&stage, &policy, enabled)?;
        if zoom_changed {
            zoom_change
                .as_ref()
                .context("zoom transition is missing its typed change")?
                .verify(&stage, registry.as_ref().context("zoom registry missing")?)?;
        }
        for entry in fs::read_dir(&stage)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid generated output name"))?;
            if valid_output_for(&name, STEM) {
                outputs.insert(name, stamp(&entry.path())?);
            }
        }
    } else {
        for name in ["enshrouded.kfc", "enshrouded.kfc_resources"] {
            outputs.insert(name.into(), stamp(&root.join(name))?);
        }
    }
    let next = Prepared {
        schema_version: if selection { 2 } else { 1 },
        root: root.to_string_lossy().into(),
        fingerprint,
        enabled,
        configured_extended_zoom: configured_zoom,
        effective_zoom: selection.then_some(desired_zoom),
        previous_state_sha256: if selection {
            Some(stamp(&state_path)?.1)
        } else {
            None
        },
        outputs,
        target: contribution_contract.map(|_| STEM.to_owned()),
        resource_contribution: contribution_state,
    };
    if stage.exists() {
        if let Some(contract) = contribution_contract {
            validate_pending(&root, &stage, &next)?;
            verify_pending_contribution(&root, &stage, &next, Some(contract))?;
        }
        write_json(&joined(&root, PENDING)?, &next)?;
        transaction::commit_generated(&root, STEM)?;
        verify_records(&root, &policy, enabled)?;
    }
    verify_outputs(&root, &next)?;
    if selection {
        let registry = registry.as_ref().context("zoom registry missing")?;
        verify_root_selection(&root, &next, &policy, registry, contribution_contract)?;
    } else if let Some(contract) = contribution_contract {
        next.resource_contribution
            .as_ref()
            .context("prepared resource-contribution state missing")?
            .verify_current(
                &root,
                STEM,
                contract,
                contribution_enabled.context("resource-contribution selection missing")?,
            )?;
    }
    write_json(&state_path, &next)?;
    let pending = joined(&root, PENDING)?;
    if pending.exists() {
        fs::remove_file(pending)?;
    }
    zoom_demand::publish(&root, desired_zoom, &policy.executable_sha256);
    Ok(true)
}

/// Presence also includes ownership receipts, so deleting the policy cannot
/// silently re-enable the baseline path on a previously composed installation.
pub fn active(root: &Path) -> Result<bool> {
    active_for(root, STEM)
}

/// Detect explicit ownership or selected AutoLoot inputs for one executable
/// target before the ordinary baseline and full-plan preparation path.
pub fn active_for(root: &Path, stem: &str) -> Result<bool> {
    ensure!(
        matches!(stem, "enshrouded" | "enshrouded_server"),
        "unsupported current-assets asset target"
    );
    for name in [POLICY, STATE, PENDING] {
        if joined(root, name)?.try_exists()? {
            return Ok(true);
        }
    }
    let mods = shroudforge_loader_config::paths::mods_dir(root);
    let hd_map = stem == STEM && mods.join("esm-real-map").try_exists()?;
    let auto_loot = mods.join(resource_contribution::PACKAGE_ID).try_exists()?;
    if hd_map || auto_loot {
        let environment = mod_loader::ModEnvironment::load(root.to_str().context("non-UTF8 root")?)
            .map_err(|_| anyhow::anyhow!("could not inspect current-assets package activation"))?;
        if hd_map
            && environment
                .mod_registry()
                .get("esm-real-map")
                .is_some_and(|item| item.info().enabled)
        {
            return Ok(true);
        }
        if auto_loot
            && environment
                .mod_registry()
                .get(resource_contribution::PACKAGE_ID)
                .is_some_and(|item| item.info().enabled)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "shroudforge-current-assets-{label}-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn startup_zoom_export_decision_handles_mixed_server_root_and_consumes_once() {
        let temp = TempRoot::new("zoom-export-mixed-server");
        let root = &temp.0;
        fs::write(root.join("enshrouded_server.exe"), []).unwrap();
        let canonical_root = fs::canonicalize(root).unwrap();

        zoom_demand::publish(&canonical_root, false, "");
        assert_eq!(take_startup_zoom_export_demand(root).unwrap(), 0);

        fs::write(root.join("enshrouded.exe"), []).unwrap();
        fs::create_dir_all(root.join("shroudforge/config")).unwrap();
        fs::write(
            root.join("shroudforge/config/current-assets-state.json"),
            br#"{"target":"enshrouded_server","resourceContribution":{"target":"enshrouded_server"}}"#,
        )
        .unwrap();
        zoom_demand::publish(&canonical_root, false, "");
        assert_eq!(take_startup_zoom_export_demand(root).unwrap(), 0);
        assert_eq!(
            take_startup_zoom_export_demand(root).unwrap(),
            2,
            "the mixed-root disabled result is consumed exactly once"
        );
    }

    #[test]
    fn startup_zoom_export_without_client_executable_clears_positive_demand() {
        const SUPPORTED_CLIENT_SHA256: &str =
            "d64966e23b1a458ee6f147bcbceac0b577fa7e39a212f371287d8cc119817882";

        let temp = TempRoot::new("zoom-export-missing-client-clears-token");
        let root = &temp.0;
        fs::write(root.join("enshrouded_server.exe"), []).unwrap();
        fs::create_dir_all(root.join("shroudforge/config")).unwrap();
        fs::write(
            root.join("shroudforge/config/current-assets-state.json"),
            br#"{"target":"enshrouded_server","resourceContribution":{"target":"enshrouded_server"}}"#,
        )
        .unwrap();
        let canonical_root = fs::canonicalize(root).unwrap();
        zoom_demand::publish(&canonical_root, true, SUPPORTED_CLIENT_SHA256);

        assert_eq!(take_startup_zoom_export_demand(root).unwrap(), 0);
        fs::write(root.join("enshrouded.exe"), []).unwrap();
        assert_eq!(
            take_startup_zoom_export_demand(root).unwrap(),
            2,
            "the missing-client decision must clear the stale positive token"
        );
    }

    fn write_package(root: &Path, id: &str, enabled: bool, setting: i64) {
        let package = root.join("mods").join(id);
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(
            package.join("mod.json"),
            serde_json::to_vec(&json!({
                "id": id,
                "name": id,
                "version": "1.0.0",
                "capabilities": ["patch"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            package.join("extended.mod.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "enabled": enabled,
                "settings": {"power": {"value": setting, "label": "Power"}}
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(package.join("src/mod.lua"), "return {}\n").unwrap();
    }

    #[test]
    fn prepared_state_rebind_requires_exact_reviewed_output_subset() {
        let composition = BTreeMap::from([
            ("enshrouded.kfc".into(), (3, "a".repeat(64))),
            ("enshrouded.kfc_resources".into(), (4, "b".repeat(64))),
            ("enshrouded_001.dat".into(), (5, "c".repeat(64))),
        ]);
        let state = BTreeMap::from([
            ("enshrouded.kfc".into(), (3, "a".repeat(64))),
            ("enshrouded.kfc_resources".into(), (4, "b".repeat(64))),
        ]);

        assert!(state_outputs_match_composition(&state, &composition));

        let changed = BTreeMap::from([
            ("enshrouded.kfc".into(), (3, "d".repeat(64))),
            ("enshrouded.kfc_resources".into(), (4, "b".repeat(64))),
        ]);
        assert!(!state_outputs_match_composition(&changed, &composition));

        let unexpected = BTreeMap::from([
            ("enshrouded.kfc".into(), (3, "a".repeat(64))),
            ("enshrouded.kfc_resources".into(), (4, "b".repeat(64))),
            ("enshrouded_999.dat".into(), (5, "c".repeat(64))),
        ]);
        assert!(!state_outputs_match_composition(&unexpected, &composition));
    }

    #[test]
    fn composition_fingerprint_tracks_selected_settings_and_ignores_map_payload() {
        let temp = TempRoot::new("fingerprint");
        let root = &temp.0;
        write_package(root, "selected-eml", true, 1);
        let selected = composition_fingerprint(root).unwrap();

        write_package(root, "selected-eml", true, 2);
        let changed_setting = composition_fingerprint(root).unwrap();
        assert_ne!(selected, changed_setting);

        write_package(root, "selected-eml", false, 2);
        let disabled = composition_fingerprint(root).unwrap();
        assert_ne!(changed_setting, disabled);

        write_package(root, "esm-real-map", true, 1);
        let map_assets = root.join("mods/esm-real-map/assets");
        fs::create_dir_all(&map_assets).unwrap();
        fs::write(map_assets.join("map.rgba"), b"R9 payload").unwrap();
        let with_map = composition_fingerprint(root).unwrap();

        write_package(root, "esm-real-map", false, 99);
        fs::write(map_assets.join("map.rgba"), b"R10 payload").unwrap();
        let changed_map = composition_fingerprint(root).unwrap();
        assert_eq!(with_map, changed_map);
    }

    fn directory_snapshot(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
        let mut entries = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let metadata = fs::symlink_metadata(&path).unwrap();
                if metadata.is_dir() {
                    entries.push((relative, None));
                    pending.push(path);
                } else {
                    entries.push((relative, Some(fs::read(path).unwrap())));
                }
            }
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries
    }

    #[test]
    fn auto_loot_contribution_fingerprint_ignores_only_its_enabled_toggle() {
        let temp = TempRoot::new("auto-loot-composition-toggle");
        let root = &temp.0;
        write_package(root, resource_contribution::PACKAGE_ID, true, 3);

        let enabled = composition_fingerprint_with_auto_loot_contribution(root).unwrap();
        write_package(root, resource_contribution::PACKAGE_ID, false, 3);
        let disabled = composition_fingerprint_with_auto_loot_contribution(root).unwrap();

        assert_eq!(enabled, disabled);
    }

    #[test]
    fn auto_loot_contribution_fingerprint_tracks_retained_disabled_package_drift() {
        let temp = TempRoot::new("auto-loot-retained-disabled-drift");
        let root = &temp.0;
        write_package(root, resource_contribution::PACKAGE_ID, true, 3);
        write_package(root, "retained-disabled", false, 1);

        let original = composition_fingerprint_with_auto_loot_contribution(root).unwrap();
        write_package(root, "retained-disabled", false, 2);
        let changed = composition_fingerprint_with_auto_loot_contribution(root).unwrap();

        assert_ne!(original, changed);
    }

    #[test]
    fn auto_loot_contribution_fingerprint_refuses_missing_environment_without_writes() {
        let temp = TempRoot::new("auto-loot-missing-environment");
        let existing_root = temp.0.join("existing-root");
        fs::create_dir_all(&existing_root).unwrap();
        fs::write(existing_root.join("sentinel.txt"), b"unchanged").unwrap();
        let before = directory_snapshot(&existing_root);

        assert!(composition_fingerprint_with_auto_loot_contribution(&existing_root).is_err());
        assert_eq!(before, directory_snapshot(&existing_root));
        assert!(!existing_root.join("mods").exists());

        let missing_root = temp.0.join("missing-root");
        assert!(composition_fingerprint_with_auto_loot_contribution(&missing_root).is_err());
        assert!(!missing_root.exists());
    }

    #[test]
    fn composition_excludes_only_an_exact_disabled_pregame_package() {
        let candidate = TempRoot::new("target-composition");
        write_package(&candidate.0, "selected-eml", true, 1);
        let target_fingerprint = composition_fingerprint(&candidate.0).unwrap();

        let source = TempRoot::new("source-composition");
        write_package(&source.0, "selected-eml", true, 1);
        write_package(&source.0, "xhl-vein-mining", false, 1);
        let source_root = fs::canonicalize(&source.0).unwrap();
        let environment = mod_loader::ModEnvironment::load(source_root.to_str().unwrap()).unwrap();
        let item = environment
            .mod_registry()
            .get("xhl-vein-mining")
            .expect("the pinned legacy package is registered");
        let excluded_fingerprint = package_fingerprint(&source_root, item).unwrap();
        let source_fingerprint =
            composition_fingerprint_for(&source_root, &environment, &[]).unwrap();
        let excluded = ExcludedPackage {
            id: "xhl-vein-mining".into(),
            fingerprint: excluded_fingerprint,
        };
        let composition = Composition {
            fingerprint: target_fingerprint.clone(),
            outputs: BTreeMap::new(),
            source_fingerprint: Some(source_fingerprint),
            baseline_enabled: true,
            baseline_legacy_pickaxe_reach_migrated: false,
            excluded_packages: vec![excluded],
        };

        assert_eq!(
            validate_composition_binding(&source_root, &environment, &composition).unwrap(),
            target_fingerprint
        );
        assert!(
            composition_fingerprint_for(
                &source_root,
                &environment,
                &[ExcludedPackage {
                    id: "unrelated-package".into(),
                    fingerprint: "a".repeat(64),
                }]
            )
            .is_err()
        );

        let mut missing_source_binding = Composition {
            fingerprint: composition.fingerprint.clone(),
            outputs: BTreeMap::new(),
            source_fingerprint: None,
            baseline_enabled: true,
            baseline_legacy_pickaxe_reach_migrated: false,
            excluded_packages: vec![ExcludedPackage {
                id: "xhl-vein-mining".into(),
                fingerprint: package_fingerprint(
                    &source_root,
                    environment.mod_registry().get("xhl-vein-mining").unwrap(),
                )
                .unwrap(),
            }],
        };
        assert!(
            validate_composition_binding(&source_root, &environment, &missing_source_binding)
                .is_err()
        );
        missing_source_binding.source_fingerprint = Some("f".repeat(64));
        assert!(
            validate_composition_binding(&source_root, &environment, &missing_source_binding)
                .is_err()
        );

        fs::write(
            source.0.join("mods/xhl-vein-mining/src/mod.lua"),
            "return { changed = true }\n",
        )
        .unwrap();
        let changed_environment =
            mod_loader::ModEnvironment::load(source_root.to_str().unwrap()).unwrap();
        assert!(
            validate_composition_binding(&source_root, &changed_environment, &composition).is_err()
        );

        fs::write(
            source.0.join("mods/xhl-vein-mining/extended.mod.json"),
            br#"{"schemaVersion":1,"enabled":true}"#,
        )
        .unwrap();
        let enabled_environment =
            mod_loader::ModEnvironment::load(source_root.to_str().unwrap()).unwrap();
        assert!(
            validate_composition_binding(&source_root, &enabled_environment, &composition).is_err()
        );
    }

    #[test]
    fn pending_publication_keeps_unstaged_outputs_bound_to_previous_state() {
        let temp = TempRoot::new("pending");
        let root = &temp.0;
        let kfc = b"old kfc";
        let resources = b"old resources";
        let dat = b"unmodified data";
        fs::write(root.join("enshrouded.kfc"), kfc).unwrap();
        fs::write(root.join("enshrouded.kfc_resources"), resources).unwrap();
        fs::write(root.join("enshrouded_001.dat"), dat).unwrap();
        let stage = root.join("stage");
        fs::create_dir_all(&stage).unwrap();
        let next_kfc = b"new kfc";
        let next_resources = b"new resources";
        fs::write(stage.join("enshrouded.kfc"), next_kfc).unwrap();
        fs::write(stage.join("enshrouded.kfc_resources"), next_resources).unwrap();
        let next = Prepared {
            schema_version: 1,
            root: root.to_string_lossy().into(),
            fingerprint: "e".repeat(64),
            enabled: false,
            configured_extended_zoom: None,
            effective_zoom: None,
            previous_state_sha256: None,
            outputs: BTreeMap::from([
                (
                    "enshrouded.kfc".into(),
                    (next_kfc.len() as u64, hash(next_kfc)),
                ),
                (
                    "enshrouded.kfc_resources".into(),
                    (next_resources.len() as u64, hash(next_resources)),
                ),
                ("enshrouded_001.dat".into(), (dat.len() as u64, hash(dat))),
            ]),
            target: None,
            resource_contribution: None,
        };

        validate_pending(root, &stage, &next).unwrap();
        fs::write(root.join("enshrouded_001.dat"), b"changed while pending").unwrap();
        assert!(validate_pending(root, &stage, &next).is_err());
    }

    #[test]
    fn composition_baseline_flags_default_to_the_schema_two_disabled_state() {
        let prior: Composition = serde_json::from_value(json!({
            "fingerprint": "a".repeat(64),
            "outputs": {}
        }))
        .unwrap();
        assert!(!prior.baseline_enabled);
        assert!(!prior.baseline_legacy_pickaxe_reach_migrated);

        let r10: Composition = serde_json::from_value(json!({
            "fingerprint": "b".repeat(64),
            "outputs": {},
            "baselineEnabled": true,
            "baselineLegacyPickaxeReachMigrated": true
        }))
        .unwrap();
        assert!(r10.baseline_enabled);
        assert!(r10.baseline_legacy_pickaxe_reach_migrated);
    }

    #[test]
    fn prepare_rejects_noncanonical_loader_paths_before_lock_or_staging() {
        for key in ["mods", "cache", "runtime"] {
            let temp = TempRoot::new(&format!("path-gate-{key}"));
            let root = &temp.0;
            shroudforge_loader_config::config::initialize_loader_config(root).unwrap();

            let config_path = shroudforge_loader_config::paths::loader_config(root);
            let mut loader = shroudforge_loader_config::config::read_loader(root).unwrap();
            loader["paths"][key] = root
                .join(format!("redirected-{key}"))
                .to_string_lossy()
                .into_owned()
                .into();
            fs::write(&config_path, serde_json::to_vec(&loader).unwrap()).unwrap();

            let policy_path = root.join(POLICY);
            fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
            // The path check runs before policy parsing, executable reads, or
            // the startup asset lease. A malformed policy makes that order
            // observable without creating any game assets.
            fs::write(&policy_path, b"not a parsed recipe yet").unwrap();

            let error = prepare(root).unwrap_err().to_string();
            let expected = format!("current-assets {key} path belongs to another installation");
            assert!(
                error.contains(expected.as_str()),
                "unexpected refusal for {key}: {error}"
            );
            assert!(
                !shroudforge_loader_config::paths::startup_asset_lock(root).exists(),
                "path mismatch acquired the startup asset lease for {key}"
            );
            assert!(
                !transaction::directory(root, STEM).exists(),
                "path mismatch created a generated transaction for {key}"
            );
        }
    }

    #[test]
    fn enabled_autoloot_without_pregame_capability_stays_on_the_guarded_route() {
        let temp = TempRoot::new("autoloot-invalid-capability");
        let root = &temp.0;
        write_package(root, resource_contribution::PACKAGE_ID, true, 1);
        let manifest_path = root.join("mods/auto-loot-plus/mod.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["capabilities"] = json!([]);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        assert!(active_for(root, "enshrouded_server").unwrap());
        let error = prepare_for(root, "enshrouded_server").unwrap_err();
        assert!(error.to_string().contains("current-assets policy missing"));
        assert!(!transaction::directory(root, "enshrouded_server").exists());
        assert!(!root.join("enshrouded_server.kfc.bak").exists());
    }

    #[test]
    fn wrong_dedicated_target_contract_fails_before_asset_backup_or_stage() {
        let temp = TempRoot::new("wrong-server-target");
        let root = &temp.0;
        shroudforge_loader_config::config::initialize_loader_config(root).unwrap();
        let policy_path = joined(root, POLICY).unwrap();
        fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
        let type_hash = kfc::hash::fnv("keen::ImpactProgram");
        let policy = json!({
            "schemaVersion": 1,
            "target": "enshrouded",
            "modId": "auto-loot-plus",
            "package": "mods/auto-loot-plus",
            "generation": "fixture",
            "executableSha256": "a".repeat(64),
            "composition": {
                "fingerprint": "b".repeat(64),
                "outputs": {
                    "enshrouded_server.kfc": [0, "c".repeat(64)],
                    "enshrouded_server.kfc_resources": [0, "d".repeat(64)]
                }
            },
            "resourceContribution": {
                "schemaVersion": 1,
                "target": "enshrouded",
                "packageTreeSha256": "e".repeat(64),
                "sourceManifestSha256": "4c0b3205e1ca1e139952294b91b3d91555ddb89480565c3fcf643c24cddf9452",
                "configSha256": "d3229deb5e522bed0f4df372f032107144841757ad1a68e84fa2a80741f60569",
                "resources": [{
                    "guid": "a5c0de5a-1e00-4d0d-9a11-0000000a5000",
                    "typeHash": type_hash,
                    "partIndex": 0,
                    "typeName": "keen::ImpactProgram",
                    "before": {"present": false}
                }]
            }
        });
        fs::write(&policy_path, serde_json::to_vec(&policy).unwrap()).unwrap();

        let error = prepare_for(root, "enshrouded_server").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported dedicated AutoLoot contribution policy")
        );
        assert!(!transaction::directory(root, "enshrouded_server").exists());
        assert!(!root.join("enshrouded_server.kfc.bak").exists());
        assert!(!root.join("enshrouded_server.exe").exists());
    }

    #[test]
    fn inverse_recipe_paths_cannot_escape_current_assets_directory() {
        let temp = TempRoot::new("inverse-path-escape");
        let expected_hash = "0".repeat(64);
        assert!(
            original(
                &temp.0,
                "../config/current-assets.json",
                expected_hash.as_str()
            )
            .is_err()
        );
        assert!(joined(&temp.0, "shroudforge/current-assets/../../outside.bin").is_err());
    }
}
