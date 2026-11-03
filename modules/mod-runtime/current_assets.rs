//! Explicit current-composition startup policy. Never falls back to baselines.
//! The installed recipe supplies original resource records from reviewed source,
//! not from a snapshot of the user's asset files. Publication recovers forward.
use anyhow::{Context, Result, ensure};
use kfc::{
    container::{KFCFile, KFCReader, KFCWriteOptions, KFCWriter},
    guid::{ContentHash, ResourceId},
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
#[path = "current_assets/zoom.rs"]
mod zoom;
#[path = "current_assets/zoom_demand.rs"]
mod zoom_demand;
#[path = "current_assets/manager_generation.rs"]
mod manager_generation;
pub use manager_generation::take_loader_demand as take_startup_loader_demand;
#[path = "current_assets/layout_migration.rs"]
mod layout_migration;
pub use zoom_demand::{clear as clear_zoom_demand, take as take_zoom_demand};

#[cfg(test)]
#[path = "current_assets/tests.rs"]
mod selection_tests;

#[cfg(test)]
#[path = "current_assets/recovery_semantics_tests.rs"]
mod recovery_semantics_tests;

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
    preserved_loaders: BTreeMap<String, String>,
    #[serde(default)]
    zoom: Option<zoom::Contract>,
    #[serde(default)]
    migration: Option<Migration>,
    #[serde(default)]
    layout_migration: Option<layout_migration::Contract>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Migration {
    state_sha256: String,
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
#[derive(Serialize, Deserialize)]
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
    name == "enshrouded.kfc"
        || name == "enshrouded.kfc_resources"
        || name
            .strip_prefix("enshrouded_")
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
            "legacy state contains successor selection"
        );
    } else {
        let configured = value
            .configured_extended_zoom
            .context("missing configured zoom")?;
        ensure!(
            value.effective_zoom == Some(value.enabled && configured),
            "inconsistent effective zoom"
        );
        let before = value
            .previous_state_sha256
            .as_deref()
            .context("missing successor before-state binding")?;
        ensure!(
            before.len() == 64 && before.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid successor before-state binding"
        );
    }
    ensure!(
        value.outputs.contains_key("enshrouded.kfc")
            && value.outputs.contains_key("enshrouded.kfc_resources"),
        "incomplete prepared-state"
    );
    ensure!(value.outputs.len() <= 512, "oversized prepared output set");
    ensure!(
        value.fingerprint.len() == 64 && value.fingerprint.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid prepared fingerprint"
    );
    for (name, expected) in &value.outputs {
        ensure!(valid_output(name), "invalid prepared output path");
        ensure!(
            expected.1.len() == 64 && expected.1.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid prepared output digest"
        );
    }
    Ok(())
}
fn verify_outputs(root: &Path, value: &Prepared) -> Result<()> {
    validate_shape(root, value)?;
    for (name, expected) in &value.outputs {
        ensure!(
            stamp(&joined(root, name)?)? == *expected,
            "prepared output changed: {name}"
        );
    }
    Ok(())
}
fn validate_pending(root: &Path, stage: &Path, next: &Prepared) -> Result<()> {
    validate_shape(root, next)?;
    ensure!(plain(stage)?.is_dir(), "pending stage is not a directory");
    let mut staged = HashSet::new();
    for entry in fs::read_dir(stage)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid staged name"))?;
        ensure!(plain(&entry.path())?.is_file(), "non-file in pending stage");
        if valid_output(&name) {
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
        ensure!(
            version == 1 && identity == next.root && (2..=512).contains(&records.len()),
            "invalid pending transaction identity/inventory"
        );
        let mut committed = HashSet::new();
        for (name, _, after) in records {
            ensure!(
                valid_output(&name) && committed.insert(name.clone()),
                "invalid or duplicate journal output"
            );
            ensure!(
                next.outputs.get(&name) == Some(&after),
                "journal output differs from outer pending receipt"
            );
        }
        ensure!(
            committed.contains("enshrouded.kfc") && committed.contains("enshrouded.kfc_resources"),
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
            staged.contains("enshrouded.kfc") && staged.contains("enshrouded.kfc_resources"),
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
fn recover(root: &Path, fingerprint: &str, schema: u32) -> Result<()> {
    let stage = transaction::directory(root, STEM);
    let pending = joined(root, PENDING)?;
    if pending.exists() {
        let next: Prepared = read_json(&pending)?;
        validate_shape(root, &next)?;
        ensure!(
            next.schema_version == schema && next.fingerprint == fingerprint,
            "pending recipe changed; explicit reconciliation required"
        );
        if schema == 2 {
            let current = stamp(&joined(root, STATE)?)?.1;
            ensure!(
                Some(current.as_str()) == next.previous_state_sha256.as_deref()
                    || current == hash(&serde_json::to_vec(&next)?),
                "pending before-state mismatch; explicit reconciliation required"
            );
        }
        if stage.exists() {
            validate_pending(root, &stage, &next)?;
        }
        if schema == 2 {
            verify_pending_selection(root, &stage, &next)?;
        }
        if stage.exists() {
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
        ensure!(
            stage.join("generated-inputs.json").exists()
                && !stage.join("generated-pending.json").exists(),
            "unowned or legacy pending asset transaction; explicit reconciliation required"
        );
        transaction::recover(root, STEM)?;
    }
    Ok(())
}

// A partially published transaction can split its index and resource file
// between stage and root. Resolve only hash-verified after-images, then inspect
// that complete logical view before recovery is allowed to change any file.
fn verify_pending_selection(root: &Path, stage: &Path, next: &Prepared) -> Result<()> {
    let after_image = |name: &str| -> Result<PathBuf> {
        let staged = joined(stage, name)?;
        let path = if staged.exists() { staged } else { joined(root, name)? };
        ensure!(
            next.outputs.get(name) == Some(&stamp(&path)?),
            "pending logical after-image mismatch: {name}"
        );
        Ok(path)
    };
    let index = after_image("enshrouded.kfc")?;
    let resources = after_image("enshrouded.kfc_resources")?;
    let reader = || -> Result<KFCReader> {
        Ok(KFCReader::new_with_resource_path(
            index.parent().context("pending index parent absent")?, STEM, resources.clone())?)
    };
    let policy: Policy = read_json(&joined(root, POLICY)?)?;
    ensure!(policy.schema_version == 2, "pending selection policy mismatch");
    ensure!(reader()?.file().game_version() == policy.generation, "pending resource generation mismatch");
    verify_resource_reader(reader()?, &policy, next.enabled)?;
    let registry = zoom::registry(
        root,
        policy.zoom.as_ref().context("pending zoom contract absent")?,
        &policy.executable_sha256,
    )?;
    zoom::verify_profile(reader()?, &registry, next.effective_zoom.context("pending effective zoom absent")?)
}
fn tree_hashes(
    root: &Path,
    relative: &str,
    result: &mut BTreeMap<String, String>,
    selection: bool,
) -> Result<()> {
    let path = joined(root, relative)?;
    if plain(&path)?.is_dir() {
        for e in fs::read_dir(&path)? {
            let e = e?;
            let next = format!(
                "{relative}/{}",
                e.file_name().to_str().context("non-UTF8 package path")?
            );
            tree_hashes(root, &next, result, selection)?;
        }
    } else {
        let digest = if path.file_name().is_some_and(|n| n == "extended.mod.json")
            && (!selection || relative == "mods/esm-real-map/extended.mod.json")
        {
            let mut value: serde_json::Value = read_json(&path)?;
            value
                .as_object_mut()
                .context("invalid extended manifest")?
                .remove("enabled");
            if selection && relative == "mods/esm-real-map/extended.mod.json" {
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
        "zoom requires restart asset preparation"
    );
    // Normalize exactly the reviewed preference; metadata remains fingerprinted.
    definition.insert("value".into(), true.into());
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
    verify_resource_records(root, policy, enabled)?;
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
fn rewrite_selected(
    root: &Path,
    policy: &Policy,
    restore_map: bool,
    zoom: Option<&zoom::Change>,
) -> Result<()> {
    rewrite_selected_with_layout(root, policy, restore_map, zoom, None)
}
fn rewrite_selected_with_layout(
    root: &Path, policy: &Policy, restore_map: bool,
    zoom: Option<&zoom::Change>, predecessor: Option<&Policy>,
) -> Result<()> {
    verify_records(root, predecessor.unwrap_or(policy), true)?;
    let before = KFCFile::from_path(root.join("enshrouded.kfc"), false)?;
    let cache = shroudforge_loader_config::paths::cache_dir(root);
    let (registry, _) = crate::reflection::load_type_registry::load_type_registry(
        camino::Utf8Path::from_path(root).context("non-UTF8 root")?,
        camino::Utf8Path::from_path(&cache).context("non-UTF8 cache")?,
        STEM,
    )
    .map_err(|()| anyhow::anyhow!("inverse type registry identity unavailable"))?;
    let stage = transaction::begin_generated(root, STEM)?;
    for name in ["enshrouded.kfc", "enshrouded.kfc_resources"] {
        fs::copy(root.join(name), stage.join(name))?;
    }
    let mut writer = KFCWriter::new_incremental_with_options(
        &stage,
        STEM,
        &before,
        &registry,
        KFCWriteOptions {
            append_after_resource_file: true,
            ..Default::default()
        },
    )?;
    let mut changed = HashSet::new();
    if restore_map {
        for (r, id) in policy.resources.iter().zip(ids(policy)?) {
            writer.write_resource(&id, &original(root, &r.original, &r.original_sha256)?)?;
            changed.insert(id);
        }
    }
    if !restore_map && predecessor.is_some() {
        let id=layout_migration::gpc_id();
        let bytes=KFCReader::new(root,STEM)?.into_cursor()?.read_resource(&id)?.context("migration GPC absent")?;
        writer.write_resource(&id,&layout_migration::corrected(&bytes)?)?;
        changed.insert(id);
    }
    if let Some(zoom) = zoom {
        writer.write_resource(&zoom::id(), &zoom.bytes)?;
        changed.insert(zoom::id());
    }
    writer.finalize()?;
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
    for (r, id) in policy.resources.iter().zip(ids(policy)?) {
        ensure!(
            hash(
                &reader
                    .read_resource(&id)?
                    .context("inverse missing resource")?
            ) == if restore_map {
                &r.original_sha256
            } else {
                &r.applied_sha256
            }
            .as_str(),
            "inverse readback mismatch"
        );
    }
    if let Some(zoom) = zoom {
        zoom.verify(&stage)?;
    }
    Ok(())
}

/// Returns false only when this explicit policy is absent. Errors must stop
/// startup; callers must never recover them through the legacy backup path.
pub fn prepare(root: &Path) -> Result<bool> {
    clear_zoom_demand();
    manager_generation::clear_loader_demand();
    manager_generation::reject_forward_hold(root)?;
    let policy_path = joined(root, POLICY)?;
    if !policy_path.try_exists()? {
        ensure!(
            !active(root)?,
            "current-assets policy missing from an owned installation"
        );
        return Ok(false);
    }
    // Manager already generated the complete selected composition offline.
    // This ownership mode validates published bytes and hands off zoom demand;
    // it must never enter the map-only recipe or a baseline/backup path.
    let document: serde_json::Value = read_json(&policy_path)?;
    if document.get("schemaVersion").and_then(serde_json::Value::as_u64) == Some(5) {
        manager_generation::verify_prepared(root)?;
        return Ok(true);
    }
    if document.get("schemaVersion").and_then(serde_json::Value::as_u64) == Some(4) {
        manager_generation::verify_forward(root)?;
        return Ok(true);
    }
    if document.get("schemaVersion").and_then(serde_json::Value::as_u64) == Some(3) {
        manager_generation::verify(root)?;
        return Ok(true);
    }
    plain(root)?;
    let root = fs::canonicalize(root)?;
    let loader =
        shroudforge_loader_config::config::read_loader(&root).map_err(anyhow::Error::msg)?;
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
    let _lease = lease(&root)?;
    let policy: Policy = read_json(&policy_path)?;
    ensure!(
        matches!(policy.schema_version, 1 | 2)
            && policy.mod_id == "esm-real-map"
            && policy.package == "mods/esm-real-map"
            && policy.resources.len() == 2
            && policy.retained_contents.len() == 2,
        "unsupported current-assets recipe"
    );
    let predecessor = layout_migration::predecessor(&root, &policy)?;
    let selection = policy.schema_version == 2;
    ensure!(
        selection == policy.zoom.is_some() && selection == policy.migration.is_some(),
        "incomplete versioned zoom policy"
    );
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
        policy.preserved_loaders.len() == 4,
        "incomplete preserved loader bindings"
    );
    for name in ["winmm.dll", "shroudtopia.dll", "dbgcore.dll", "dinput8.dll"] {
        let expected = policy
            .preserved_loaders
            .get(name)
            .context("missing preserved loader binding")?;
        ensure!(
            stamp(&joined(&root, name)?)?.1 == *expected,
            "preserved native loader changed: {name}"
        );
    }
    ensure!(
        stamp(&root.join("enshrouded.exe"))?.1 == policy.executable_sha256,
        "current-assets executable mismatch"
    );
    let environment = mod_loader::ModEnvironment::load(root.to_str().context("non-UTF8 root")?)
        .map_err(|_| anyhow::anyhow!("could not load current-assets mod environment"))?;
    let item = environment
        .mod_registry()
        .get(&policy.mod_id)
        .context("selected package missing")?;
    let enabled = item.info().enabled;
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
    tree_hashes(&root, &policy.package, &mut inputs, selection)?;
    if policy.layout_migration.is_some() { layout_migration::package(&inputs)?; }
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
                .and_then(|v| v.as_bool())
                .context("ExtendedZoom must resolve to boolean")?,
        )
    } else {
        None
    };
    let desired_zoom = enabled && configured_zoom.unwrap_or(false);
    inputs.insert("settings".into(), hash(&serde_json::to_vec(&settings)?));
    for r in &policy.resources {
        original(&root, &r.original, &r.original_sha256)?;
    }
    for c in &policy.retained_contents {
        original(&root, &c.file, &c.sha256)?;
    }
    let fingerprint = hash(&serde_json::to_vec(&inputs)?);
    // Bind pending publication to immutable recipe/package identity BEFORE it
    // can write. The declared selection may change while recovery is pending.
    // A mixed asset generation is never passed to a parser.
    let registry = policy
        .zoom
        .as_ref()
        .map(|contract| zoom::registry(&root, contract, &policy.executable_sha256))
        .transpose()?;
    if let Some(contract)=&policy.layout_migration {
        layout_migration::before_recovery(&root, &fingerprint, contract)?;
    }
    recover(&root, &fingerprint, policy.schema_version)?;
    let index = KFCFile::from_path(root.join("enshrouded.kfc"), false)?;
    ensure!(
        index.game_version() == policy.generation,
        "current-assets generation mismatch"
    );
    let state_path = joined(&root, STATE)?;
    let mut migrating_layout = false;
    let previous = if state_path.exists() {
        let state: Prepared = read_json(&state_path)?;
        verify_outputs(&root, &state)?;
        if selection && state.schema_version == 1 {
            ensure!(policy.layout_migration.is_none(), "layout migration requires schema2 predecessor");
            let migration = policy
                .migration
                .as_ref()
                .context("missing explicit migration")?;
            ensure!(
                stamp(&state_path)?.1 == migration.state_sha256
                    && state.fingerprint == migration.fingerprint,
                "legacy terminal identity does not match explicit migration"
            );
        } else if state.schema_version == policy.schema_version && state.fingerprint == fingerprint {
            // Already owned by this exact successor recipe.
        } else if let Some(contract)=&policy.layout_migration {
            layout_migration::authorize(&root,&state,contract)?;
            migrating_layout = true;
        } else {
            anyhow::bail!("selected package or recipe changed; explicit migration required");
        }
        Some(state)
    } else {
        ensure!(
            !selection,
            "zoom successor requires a reviewed legacy terminal or successor state"
        );
        None
    };
    let was_enabled = previous.as_ref().is_some_and(|s| s.enabled);
    let was_zoom = previous
        .as_ref()
        .and_then(|s| s.effective_zoom)
        .unwrap_or(false);
    let before_policy = if migrating_layout { predecessor.as_ref().context("layout predecessor absent")? } else { &policy };
    verify_records(&root, before_policy, was_enabled)?;
    let zoom_change = registry
        .map(|registry| zoom::Change::new(&root, registry, was_zoom, desired_zoom))
        .transpose()?;
    if !migrating_layout && previous.as_ref().is_some_and(|s| {
        s.schema_version == policy.schema_version
            && s.enabled == enabled
            && s.configured_extended_zoom == configured_zoom
    }) {
        zoom_demand::publish(&root, desired_zoom, &policy.executable_sha256);
        return Ok(true);
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
    } else if was_enabled && (!enabled || was_zoom != desired_zoom || migrating_layout) {
        rewrite_selected_with_layout(&root, &policy, !enabled, zoom_change.as_ref(),
            if migrating_layout { predecessor.as_ref() } else { None })?;
    }
    let stage = transaction::directory(&root, STEM);
    let mut outputs = previous.map(|p| p.outputs).unwrap_or_default();
    if stage.exists() {
        verify_resource_records(&stage, &policy, enabled)?;
        if let Some(zoom) = &zoom_change {
            zoom.verify(&stage)?;
        }
        for entry in fs::read_dir(&stage)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid generated output name"))?;
            if valid_output(&name) {
                outputs.insert(name, stamp(&entry.path())?);
            }
        }
    } else {
        if let Some(zoom) = &zoom_change {
            zoom.verify(&root)?;
        }
        for name in ["enshrouded.kfc", "enshrouded.kfc_resources"] {
            outputs.insert(name.into(), stamp(&root.join(name))?);
        }
    }
    let next = Prepared {
        schema_version: policy.schema_version,
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
    };
    if stage.exists() {
        write_json(&joined(&root, PENDING)?, &next)?;
        transaction::commit_generated(&root, STEM)?;
        verify_records(&root, &policy, enabled)?;
        if let Some(zoom) = &zoom_change {
            zoom.verify(&root)?;
        }
    }
    verify_outputs(&root, &next)?;
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
    manager_generation::reject_forward_hold(root)?;
    for name in [POLICY, STATE, PENDING] {
        if joined(root, name)?.try_exists()? {
            return Ok(true);
        }
    }
    Ok(false)
}
