//! Exact, reversible AutoLootPlus resource-map contributions.
//!
//! The contract records the reviewed preimage for every resource the pinned
//! Lua package is expected to write. The state records the generated mapping
//! and digest. Inverse publication only rebinds old prefix mappings or removes
//! originally absent IDs; it never copies old asset payloads into a new chunk.

use anyhow::{Context, Result, ensure};
use kfc::{
    container::{KFCCursor, KFCFile, KFCReader},
    guid::ResourceId,
    reflection::{LookupKey, TypeRegistry},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{borrow::Borrow, collections::HashSet, fs::File, io::Read, path::Path};

pub(super) const PACKAGE_ID: &str = "auto-loot-plus";
const PACKAGE_PATH: &str = "mods/auto-loot-plus";
const CONFIG_PATH: &str = "mods/auto-loot-plus/config.txt";
const LUA_PATH: &str = "mods/auto-loot-plus/src/mod.lua";
const PROGRAM_LUA_PATH: &str = "mods/auto-loot-plus/src/autosmash_program.lua";
const CLIENT_DLL_PATH: &str = "mods/auto-loot-plus/native/dinput8.dll";
const SOURCE_MANIFEST_SHA256: &str =
    "4c0b3205e1ca1e139952294b91b3d91555ddb89480565c3fcf643c24cddf9452";
const LUA_SHA256: &str = "7d524e03a845d11ca58eff2ab26bfb182eecce7639a10f10253d60f88ad7344e";
const PROGRAM_LUA_SHA256: &str = "aa9992e2af2ca505acf869f299846db3a0e0fd375692bd8c11b614a8ffb14e17";
const CONFIG_SHA256: &str = "d3229deb5e522bed0f4df372f032107144841757ad1a68e84fa2a80741f60569";
const CLIENT_DLL_SHA256: &str = "f885ac8032109838cfefa9fb34307e4dbbc396472e3cf465b656915559b89c46";
const PROGRAM_GUID: &str = "a5c0de5a-1e00-4d0d-9a11-0000000a5000";
const AURA_GUID: &str = "a5c0de5a-1e00-4d0d-9a11-0000000a5002";
const MAX_RESOURCES: usize = 16_384;
const ALLOWED_TYPES: [&str; 3] = [
    "keen::ImpactProgram",
    "keen::ImpactRegistryResource",
    "keen::ecs::TemplateResource",
];

#[cfg(test)]
thread_local! {
    static DEDICATED_REGISTRY_FACTORY: std::cell::Cell<Option<fn() -> TypeRegistry>> =
        const { std::cell::Cell::new(None) };
}

fn load_dedicated_registry(executable: std::path::PathBuf) -> Result<TypeRegistry> {
    #[cfg(test)]
    if let Some(factory) = DEDICATED_REGISTRY_FACTORY.with(std::cell::Cell::get) {
        return Ok(factory());
    }
    TypeRegistry::load_from_executable(executable)
        .context("dedicated resource type registry unavailable")
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Contract {
    schema_version: u32,
    target: String,
    package_tree_sha256: String,
    source_manifest_sha256: String,
    config_sha256: String,
    resources: Vec<ResourceContract>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceContract {
    guid: String,
    type_hash: u32,
    part_index: u32,
    type_name: String,
    before: BeforeImage,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BeforeImage {
    present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mapping: Option<Mapping>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Mapping {
    offset: u64,
    size: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AfterImage {
    mapping: Mapping,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceState {
    resource: ResourceContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    after: Option<AfterImage>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct State {
    schema_version: u32,
    target: String,
    contract_sha256: String,
    package_tree_sha256: String,
    config_sha256: String,
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_state_sha256: Option<String>,
    resources: Vec<ResourceState>,
}

impl Contract {
    pub(super) fn target(&self) -> &str {
        &self.target
    }

    pub(super) fn validate(
        &self,
        root: &Path,
        target: &str,
        registry: &TypeRegistry,
    ) -> Result<String> {
        ensure!(
            self.schema_version == 1 && self.target == target,
            "unsupported resource-contribution target or schema"
        );
        ensure!(
            matches!(target, "enshrouded" | "enshrouded_server"),
            "unsupported resource-contribution asset target"
        );
        ensure!(
            self.source_manifest_sha256 == SOURCE_MANIFEST_SHA256
                && self.config_sha256 == CONFIG_SHA256
                && super::valid_sha256(&self.package_tree_sha256),
            "resource-contribution source or config pin is not the reviewed AutoLootPlus input"
        );
        ensure!(
            !self.resources.is_empty() && self.resources.len() <= MAX_RESOURCES,
            "resource-contribution write set is empty or oversized"
        );

        for (path, expected, label) in [
            (LUA_PATH, LUA_SHA256, "AutoLootPlus Lua"),
            (
                PROGRAM_LUA_PATH,
                PROGRAM_LUA_SHA256,
                "AutoLootPlus impact-program Lua",
            ),
            (CONFIG_PATH, CONFIG_SHA256, "AutoLootPlus asset config"),
        ] {
            ensure!(
                super::stamp(&super::joined(root, path)?)?.1 == expected,
                "{label} differs from the reviewed source input"
            );
        }
        let mut package_tree = std::collections::BTreeMap::new();
        super::tree_hashes(root, PACKAGE_PATH, &mut package_tree, None)?;
        ensure!(
            super::hash(&serde_json::to_vec(&package_tree)?) == self.package_tree_sha256,
            "AutoLootPlus package tree changed"
        );
        let dll = super::joined(root, CLIENT_DLL_PATH)?;
        if target == "enshrouded_server" {
            ensure!(
                !dll.try_exists()?,
                "server AutoLootPlus package contains the client-only native DLL"
            );
        } else {
            ensure!(
                super::stamp(&dll)?.1 == CLIENT_DLL_SHA256,
                "client AutoLootPlus native library differs from the reviewed input"
            );
        }

        let mut previous = None;
        let mut seen = HashSet::new();
        for resource in &self.resources {
            let id = parse_id(resource)?;
            let order_key = (
                resource.guid.as_str(),
                resource.type_hash,
                resource.part_index,
            );
            ensure!(
                previous.is_none_or(|value| value < order_key),
                "resource-contribution records are unsorted or duplicated"
            );
            previous = Some(order_key);
            ensure!(seen.insert(id), "duplicate resource-contribution ID");
            ensure!(
                ALLOWED_TYPES.contains(&resource.type_name.as_str()),
                "resource-contribution type is outside the reviewed source write set"
            );
            let metadata = registry
                .get_by_name(LookupKey::Qualified(resource.type_name.as_str()))
                .context("resource-contribution type is unavailable")?;
            ensure!(
                metadata.qualified_hash == resource.type_hash,
                "resource-contribution resource type hash mismatch"
            );
            validate_before(&resource.before)?;
            if !resource.before.present {
                ensure!(
                    is_known_created_resource(resource),
                    "AutoLootPlus cannot add an unreviewed resource ID"
                );
            }
        }

        Ok(self.digest()?)
    }

    pub(super) fn digest(&self) -> Result<String> {
        Ok(super::hash(&serde_json::to_vec(self)?))
    }

    pub(super) fn initial_state(&self, enabled: bool) -> Result<State> {
        Ok(State {
            schema_version: 1,
            target: self.target.clone(),
            contract_sha256: self.digest()?,
            package_tree_sha256: self.package_tree_sha256.clone(),
            config_sha256: self.config_sha256.clone(),
            enabled,
            previous_state_sha256: None,
            resources: self
                .resources
                .iter()
                .cloned()
                .map(|resource| ResourceState {
                    resource,
                    after: None,
                })
                .collect(),
        })
    }

    pub(super) fn ids(&self, registry: &TypeRegistry) -> Result<Vec<ResourceId>> {
        contract_ids(self, registry)
    }
}

impl State {
    pub(super) fn target(&self) -> &str {
        &self.target
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn previous_state_sha256(&self) -> Option<&str> {
        self.previous_state_sha256.as_deref()
    }

    pub(super) fn set_previous_state_sha256(&mut self, digest: Option<String>) {
        self.previous_state_sha256 = digest;
    }

    pub(super) fn validate_contract(&self, contract: &Contract) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.target == contract.target
                && self.contract_sha256 == contract.digest()?
                && self.package_tree_sha256 == contract.package_tree_sha256
                && self.config_sha256 == contract.config_sha256
                && self.resources.len() == contract.resources.len(),
            "resource-contribution state does not match its contract"
        );
        ensure!(
            self.previous_state_sha256
                .as_deref()
                .is_none_or(super::valid_sha256),
            "invalid resource-contribution predecessor state digest"
        );
        for (state, expected) in self.resources.iter().zip(&contract.resources) {
            ensure!(
                state.resource == *expected,
                "resource-contribution state write set changed"
            );
            if let Some(after) = &state.after {
                ensure!(
                    super::valid_sha256(&after.sha256),
                    "invalid resource-contribution after-image digest"
                );
            }
        }
        Ok(())
    }

    pub(super) fn verify_current(
        &self,
        root: &Path,
        stem: &str,
        contract: &Contract,
        enabled: bool,
    ) -> Result<()> {
        self.validate_contract(contract)?;
        ensure!(
            contract.target == stem,
            "resource-contribution reader target mismatch"
        );
        let reader = KFCReader::new(root, stem)?;
        verify_state_reader(reader, self, contract, enabled)
    }

    pub(super) fn verify_afterimage(
        &self,
        reader: KFCReader,
        stem: &str,
        contract: &Contract,
        enabled: bool,
    ) -> Result<()> {
        self.validate_contract(contract)?;
        ensure!(
            contract.target == stem,
            "resource-contribution after-image target mismatch"
        );
        verify_state_reader(reader, self, contract, enabled)
    }

    pub(super) fn write_inverse<F, T>(
        &self,
        writer: &mut kfc::container::KFCWriter<F, T>,
        contract: &Contract,
    ) -> Result<()>
    where
        F: Borrow<KFCFile>,
        T: Borrow<TypeRegistry>,
    {
        self.validate_contract(contract)?;
        ensure!(
            self.enabled,
            "resource-contribution inverse requires an enabled receipt"
        );
        for entry in &self.resources {
            let after = entry
                .after
                .as_ref()
                .context("resource-contribution after-image is missing")?;
            let id = parse_id(&entry.resource)?;
            writer.restore_resource_mapping(
                &id,
                Some((after.mapping.offset, after.mapping.size)),
                entry
                    .resource
                    .before
                    .mapping
                    .as_ref()
                    .map(|mapping| (mapping.offset, mapping.size)),
            )?;
        }
        Ok(())
    }
}

/// Apply one AutoLoot selection transition through the shared generated-asset
/// writer. The selected-package callback is never called for a repeat state;
/// disable only restores the receipt's exact resource mappings.
pub(super) fn apply_selection<F>(
    root: &Path,
    stem: &str,
    contract: &Contract,
    state: &State,
    registry: &TypeRegistry,
    selected: bool,
    run_selected: F,
) -> Result<State>
where
    F: FnOnce() -> Result<()>,
{
    ensure!(
        contract.target == stem,
        "resource-contribution target mismatch"
    );
    state.verify_current(root, stem, contract, state.enabled())?;
    if selected == state.enabled() {
        return Ok(state.clone());
    }
    if selected {
        run_selected()?;
        let stage = super::transaction::directory(root, stem);
        capture_after(root, &stage, stem, contract, state, registry)
    } else {
        super::append_resource_mapping_inverse(root, stem, state, contract, registry)
    }
}

pub(super) fn capture_after(
    root: &Path,
    stage: &Path,
    stem: &str,
    contract: &Contract,
    state: &State,
    registry: &TypeRegistry,
) -> Result<State> {
    state.validate_contract(contract)?;
    ensure!(
        contract.target == stem,
        "resource-contribution generation target mismatch"
    );
    ensure!(
        !state.enabled,
        "resource-contribution after-image capture requires a disabled predecessor"
    );
    let before_file = KFCFile::from_path(root.join(format!("{stem}.kfc")), false)?;
    let after_file = KFCFile::from_path(stage.join(format!("{stem}.kfc")), false)?;
    let ids = contract_ids(contract, registry)?;
    let allowed: HashSet<_> = ids.iter().copied().collect();
    validate_structure(&before_file, &after_file, &allowed)?;
    validate_resource_prefix(
        &root.join(format!("{stem}.kfc_resources")),
        &stage.join(format!("{stem}.kfc_resources")),
    )?;

    let mut changed = HashSet::new();
    for (id, before) in before_file.resources().iter() {
        match after_file.resources().get(id) {
            Some(after) if (before.offset, before.size) == (after.offset, after.size) => {}
            _ => {
                changed.insert(*id);
            }
        }
    }
    for id in after_file.resources().keys() {
        if !before_file.resources().contains_key(id) {
            changed.insert(*id);
        }
    }
    ensure!(
        changed == allowed,
        "generated AutoLootPlus resource write set differs from its exact reviewed contract"
    );

    let mut before_reader = KFCReader::new(root, stem)?.into_cursor()?;
    let mut after_reader = KFCReader::new(stage, stem)?.into_cursor()?;
    let mut next = state.clone();
    next.enabled = true;
    for entry in &mut next.resources {
        let id = parse_id(&entry.resource)?;
        verify_before_record(&mut before_reader, &before_file, &entry.resource)?;
        let mapping = after_file
            .resources()
            .get(&id)
            .context("reviewed AutoLootPlus resource missing from generated output")?;
        let bytes = after_reader
            .read_resource(&id)?
            .context("reviewed AutoLootPlus output resource unreadable")?;
        let digest = digest(&bytes);
        if let Some(previous) = &entry.after {
            ensure!(
                previous.sha256 == digest,
                "re-enabled AutoLootPlus produced a different resource payload"
            );
        }
        entry.after = Some(AfterImage {
            mapping: Mapping {
                offset: mapping.offset,
                size: mapping.size,
            },
            sha256: digest,
        });
    }
    Ok(next)
}

pub(super) fn verify_inverse(
    before: &KFCFile,
    after: &KFCFile,
    before_reader: &mut impl ResourceReader,
    after_reader: &mut impl ResourceReader,
    contract: &Contract,
    state: &State,
    registry: &TypeRegistry,
) -> Result<State> {
    state.validate_contract(contract)?;
    ensure!(
        contract.target == "enshrouded" || contract.target == "enshrouded_server",
        "unsupported resource-contribution inverse target"
    );
    ensure!(
        state.enabled,
        "resource-contribution inverse has no enabled predecessor"
    );
    let ids = contract_ids(contract, registry)?;
    let allowed: HashSet<_> = ids.iter().copied().collect();
    validate_structure(before, after, &allowed)?;
    let mut changed = HashSet::new();
    for (id, mapping) in before.resources().iter() {
        match after.resources().get(id) {
            Some(next) if (mapping.offset, mapping.size) == (next.offset, next.size) => {}
            _ => {
                changed.insert(*id);
            }
        }
    }
    for id in after.resources().keys() {
        if !before.resources().contains_key(id) {
            changed.insert(*id);
        }
    }
    ensure!(
        changed == allowed,
        "inverse resource write set differs from its exact reviewed contract"
    );

    let mut next = state.clone();
    next.enabled = false;
    for entry in &mut next.resources {
        let id = parse_id(&entry.resource)?;
        let after_mapping = after.resources().get(&id);
        let after_image = entry
            .after
            .as_ref()
            .context("enabled resource contribution has no after-image")?;
        let before_image = BeforeImage {
            present: true,
            mapping: Some(after_image.mapping.clone()),
            sha256: Some(after_image.sha256.clone()),
        };
        verify_image(before_reader, before, &id, &before_image)?;
        verify_image(after_reader, after, &id, &entry.resource.before)?;
        let expected_before = &entry.resource.before;
        match (expected_before.present, after_mapping) {
            (true, Some(mapping)) => ensure!(
                expected_before.mapping.as_ref().is_some_and(|expected| {
                    (expected.offset, expected.size) == (mapping.offset, mapping.size)
                }),
                "inverse did not rebind the original resource mapping"
            ),
            (false, None) => {}
            _ => anyhow::bail!("inverse resource presence differs from its reviewed preimage"),
        }
    }
    Ok(next)
}

pub(super) fn validate_structure(
    before: &KFCFile,
    after: &KFCFile,
    mutable_ids: &HashSet<ResourceId>,
) -> Result<()> {
    ensure!(
        before.game_version() == after.game_version(),
        "asset generation changed"
    );
    ensure!(
        before.containers().len() == after.containers().len()
            && before
                .containers()
                .iter()
                .zip(after.containers())
                .all(|(a, b)| (a.size, a.count) == (b.size, b.count)),
        "resource contribution changed content-container inventory"
    );
    ensure!(
        after.resource_chunks().len() >= before.resource_chunks().len()
            && before
                .resource_chunks()
                .iter()
                .zip(after.resource_chunks())
                .all(|(a, b)| {
                    (
                        a.offset,
                        a.size,
                        a.compressed_size,
                        a.uncompressed_offset,
                        a.uncompressed_size,
                    ) == (
                        b.offset,
                        b.size,
                        b.compressed_size,
                        b.uncompressed_offset,
                        b.uncompressed_size,
                    )
                }),
        "resource contribution changed the retained resource prefix"
    );
    ensure!(
        before.contents().len() == after.contents().len(),
        "resource contribution changed content inventory"
    );
    for (id, prior) in before.contents().iter() {
        let next = after
            .contents()
            .get(id)
            .context("resource contribution removed a content mapping")?;
        ensure!(
            (prior.offset, prior.flags, prior.container_index)
                == (next.offset, next.flags, next.container_index),
            "resource contribution changed a content mapping"
        );
    }
    for (id, prior) in before.resources().iter() {
        if !mutable_ids.contains(id) {
            let next = after
                .resources()
                .get(id)
                .context("resource contribution removed an undeclared resource")?;
            ensure!(
                (prior.offset, prior.size) == (next.offset, next.size),
                "resource contribution changed an undeclared resource"
            );
        }
    }
    for id in after.resources().keys() {
        ensure!(
            before.resources().contains_key(id) || mutable_ids.contains(id),
            "resource contribution added an undeclared resource"
        );
    }
    let mut membership = std::collections::HashMap::<u32, HashSet<ResourceId>>::new();
    for id in after.resources().keys() {
        membership.entry(id.type_hash()).or_default().insert(*id);
    }
    for (type_hash, expected) in membership {
        ensure!(
            after
                .resources_by_type(type_hash)
                .copied()
                .collect::<HashSet<_>>()
                == expected,
            "resource contribution broke KFC type-bundle membership"
        );
    }
    Ok(())
}

fn verify_state_reader(
    reader: KFCReader,
    state: &State,
    contract: &Contract,
    enabled: bool,
) -> Result<()> {
    state.validate_contract(contract)?;
    ensure!(
        state.enabled == enabled,
        "resource-contribution selection differs from its prepared state"
    );
    let file = reader.file().clone();
    let mut reader = reader.into_cursor()?;
    for entry in &state.resources {
        let id = parse_id(&entry.resource)?;
        if enabled {
            let image = entry
                .after
                .as_ref()
                .context("enabled resource contribution has no after-image")?;
            let expected = BeforeImage {
                present: true,
                mapping: Some(image.mapping.clone()),
                sha256: Some(image.sha256.clone()),
            };
            verify_image(&mut reader, &file, &id, &expected)?;
        } else {
            verify_image(&mut reader, &file, &id, &entry.resource.before)?;
        }
    }
    Ok(())
}

pub(super) trait ResourceReader {
    fn read_resource(&mut self, id: &ResourceId) -> Result<Option<Vec<u8>>>;
}

impl<T> ResourceReader for KFCCursor<T>
where
    T: std::borrow::Borrow<KFCReader>,
{
    fn read_resource(&mut self, id: &ResourceId) -> Result<Option<Vec<u8>>> {
        Ok(KFCCursor::read_resource(self, id)?)
    }
}

fn verify_before_record(
    reader: &mut impl ResourceReader,
    file: &KFCFile,
    resource: &ResourceContract,
) -> Result<()> {
    verify_image(reader, file, &parse_id(resource)?, &resource.before)
}

fn verify_image(
    reader: &mut impl ResourceReader,
    file: &KFCFile,
    id: &ResourceId,
    image: &BeforeImage,
) -> Result<()> {
    let mapping = file.resources().get(id).map(|entry| Mapping {
        offset: entry.offset,
        size: entry.size,
    });
    ensure!(
        image.present == mapping.is_some() && image.mapping == mapping,
        "resource mapping differs from its reviewed preimage"
    );
    let bytes = reader.read_resource(id)?;
    match (&image.sha256, bytes) {
        (Some(expected), Some(bytes)) => ensure!(
            digest(&bytes) == *expected,
            "resource payload differs from its reviewed digest"
        ),
        (None, None) => {}
        _ => anyhow::bail!("resource presence differs from its reviewed digest"),
    }
    Ok(())
}

fn validate_before(image: &BeforeImage) -> Result<()> {
    match (image.present, &image.mapping, &image.sha256) {
        (true, Some(mapping), Some(digest)) => {
            ensure!(
                mapping.offset.checked_add(mapping.size).is_some() && super::valid_sha256(digest),
                "invalid resource-contribution before-image"
            );
        }
        (false, None, None) => {}
        _ => anyhow::bail!("inconsistent resource-contribution before-image"),
    }
    Ok(())
}

fn is_known_created_resource(resource: &ResourceContract) -> bool {
    resource.part_index == 0
        && ((resource.guid == PROGRAM_GUID && resource.type_name == "keen::ImpactProgram")
            || (resource.guid == AURA_GUID && resource.type_name == "keen::ecs::TemplateResource"))
}

fn parse_id(resource: &ResourceContract) -> Result<ResourceId> {
    let id = ResourceId::parse(&resource.guid, resource.type_hash, resource.part_index)
        .context("invalid resource-contribution GUID")?;
    ensure!(
        id.to_string() == resource.guid,
        "resource-contribution GUID must use canonical lowercase formatting"
    );
    Ok(id)
}

fn contract_ids(contract: &Contract, registry: &TypeRegistry) -> Result<Vec<ResourceId>> {
    contract
        .resources
        .iter()
        .map(|resource| {
            let metadata = registry
                .get_by_name(LookupKey::Qualified(resource.type_name.as_str()))
                .context("resource-contribution type is unavailable")?;
            ensure!(
                metadata.qualified_hash == resource.type_hash,
                "resource-contribution resource type hash mismatch"
            );
            parse_id(resource)
        })
        .collect()
}

fn validate_resource_prefix(before: &Path, after: &Path) -> Result<()> {
    let mut before = File::open(before)?;
    let mut after = File::open(after)?;
    let mut left = [0u8; 65536];
    let mut right = [0u8; 65536];
    loop {
        let count = before.read(&mut left)?;
        if count == 0 {
            return Ok(());
        }
        after
            .read_exact(&mut right[..count])
            .context("resource contribution truncated the retained resource prefix")?;
        ensure!(
            left[..count] == right[..count],
            "resource contribution changed the retained resource prefix"
        );
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn prepare_dedicated(root: &Path, policy_path: &Path) -> Result<bool> {
    const STEM: &str = "enshrouded_server";
    let policy: super::DedicatedPolicy = super::read_json(policy_path)?;
    ensure!(
        policy.schema_version == 1
            && policy.target == STEM
            && policy.mod_id == PACKAGE_ID
            && policy.package == PACKAGE_PATH
            && policy.composition.source_fingerprint.is_none()
            && !policy.composition.baseline_enabled
            && !policy.composition.baseline_legacy_pickaxe_reach_migrated
            && policy.composition.excluded_packages.is_empty(),
        "unsupported dedicated AutoLoot contribution policy"
    );
    ensure!(
        super::valid_sha256(&policy.composition.fingerprint)
            && !policy.composition.outputs.is_empty()
            && policy.composition.outputs.len() <= 512
            && policy
                .composition
                .outputs
                .contains_key("enshrouded_server.kfc")
            && policy
                .composition
                .outputs
                .contains_key("enshrouded_server.kfc_resources"),
        "invalid dedicated predecessor composition"
    );
    ensure!(
        super::valid_sha256(&policy.executable_sha256)
            && super::stamp(&root.join("enshrouded_server.exe"))?.1 == policy.executable_sha256,
        "dedicated current-assets executable mismatch"
    );
    for (name, expected) in &policy.composition.outputs {
        ensure!(
            super::valid_output_for(name, STEM) && super::valid_sha256(&expected.1),
            "invalid dedicated predecessor output stamp"
        );
    }

    let environment =
        mod_loader::ModEnvironment::load(root.to_str().context("non-UTF8 dedicated root")?)
            .map_err(|_| anyhow::anyhow!("could not load dedicated AutoLoot environment"))?;
    let selected =
        super::validate_auto_loot_selection(&environment, STEM, &policy.resource_contribution)?;
    let actual_composition =
        super::composition_fingerprint_for_with_contribution(root, &environment, &[], true)?;
    ensure!(
        actual_composition == policy.composition.fingerprint,
        "dedicated pregame composition changed; explicit current-assets reconciliation required"
    );

    let registry_path = super::joined(root, "enshrouded_server.exe")?;
    let registry = load_dedicated_registry(registry_path)?;
    policy
        .resource_contribution
        .validate(root, STEM, &registry)?;
    let mut inputs = std::collections::BTreeMap::new();
    inputs.insert("target".to_owned(), STEM.to_owned());
    inputs.insert("composition".to_owned(), actual_composition);
    inputs.insert(
        "contribution".to_owned(),
        policy.resource_contribution.digest()?,
    );
    inputs.insert("generation".to_owned(), policy.generation.clone());
    inputs.insert("executable".to_owned(), policy.executable_sha256.clone());
    inputs.insert("policy".to_owned(), super::stamp(policy_path)?.1);
    let fingerprint = super::hash(&serde_json::to_vec(&inputs)?);

    super::recover_contribution_transaction(
        root,
        STEM,
        &fingerprint,
        selected,
        None,
        &policy.resource_contribution,
        None,
        None,
    )?;
    ensure!(
        KFCFile::from_path(root.join("enshrouded_server.kfc"), false)?.game_version()
            == policy.generation,
        "dedicated resource generation mismatch"
    );

    let state_path = super::joined(root, super::STATE)?;
    let previous = if state_path.exists() {
        let state: super::Prepared = super::read_json(&state_path)?;
        super::validate_shape(root, &state)?;
        ensure!(
            state.target.as_deref() == Some(STEM) && state.fingerprint == fingerprint,
            "dedicated prepared state changed; explicit reconciliation required"
        );
        super::verify_outputs(root, &state)?;
        let contribution = state
            .resource_contribution
            .as_ref()
            .context("dedicated prepared contribution state missing")?;
        contribution.validate_contract(&policy.resource_contribution)?;
        contribution.verify_current(
            root,
            STEM,
            &policy.resource_contribution,
            contribution.enabled(),
        )?;
        Some(state)
    } else {
        let baseline = super::Prepared {
            schema_version: 1,
            root: root.to_string_lossy().into(),
            fingerprint: fingerprint.clone(),
            enabled: false,
            configured_extended_zoom: None,
            effective_zoom: None,
            previous_state_sha256: None,
            outputs: policy.composition.outputs.clone(),
            target: Some(STEM.to_owned()),
            resource_contribution: Some(policy.resource_contribution.initial_state(false)?),
        };
        super::verify_outputs(root, &baseline)?;
        super::verify_inventory_for(root, &baseline.outputs, STEM)?;
        None
    };

    let mut contribution = if let Some(state) = previous
        .as_ref()
        .and_then(|state| state.resource_contribution.as_ref())
    {
        state.clone()
    } else {
        ensure!(
            previous.is_none(),
            "dedicated prepared state has no contribution receipt"
        );
        policy.resource_contribution.initial_state(false)?
    };
    contribution.verify_current(
        root,
        STEM,
        &policy.resource_contribution,
        contribution.enabled(),
    )?;
    let changed = contribution.enabled() != selected;
    if !changed && previous.is_some() {
        return Ok(true);
    }

    if changed {
        let updated = apply_selection(
            root,
            STEM,
            &policy.resource_contribution,
            &contribution,
            &registry,
            selected,
            || {
                crate::run_with_local_schema(
                    &environment,
                    crate::RunArgs {
                        file_name: STEM.to_owned(),
                        options: crate::RunOptions {
                            current_asset_mods: Some(vec![PACKAGE_ID.to_owned()]),
                            defer_current_commit: true,
                            skip_cache: true,
                            force_patch: true,
                            patch: true,
                            export: false,
                            is_server: Some(true),
                            ..Default::default()
                        },
                    },
                )
            },
        )?;
        let predecessor = if state_path.exists() {
            Some(super::stamp(&state_path)?.1)
        } else {
            None
        };
        contribution = updated;
        contribution.set_previous_state_sha256(predecessor);
    }

    let stage = super::transaction::directory(root, STEM);
    let mut outputs = previous
        .map(|state| state.outputs)
        .unwrap_or_else(|| policy.composition.outputs.clone());
    if stage.exists() {
        for entry in std::fs::read_dir(&stage)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid dedicated staged output name"))?;
            if super::valid_output_for(&name, STEM) {
                outputs.insert(name, super::stamp(&entry.path())?);
            }
        }
    }
    let next = super::Prepared {
        schema_version: 1,
        root: root.to_string_lossy().into(),
        fingerprint,
        enabled: false,
        configured_extended_zoom: None,
        effective_zoom: None,
        previous_state_sha256: None,
        outputs,
        target: Some(STEM.to_owned()),
        resource_contribution: Some(contribution),
    };
    if stage.exists() {
        super::validate_pending(root, &stage, &next)?;
        super::verify_pending_contribution(
            root,
            &stage,
            &next,
            Some(&policy.resource_contribution),
        )?;
        super::write_json(&super::joined(root, super::PENDING)?, &next)?;
        super::transaction::commit_generated(root, STEM)?;
    }
    super::verify_outputs(root, &next)?;
    next.resource_contribution
        .as_ref()
        .context("dedicated resource-contribution state missing")?
        .verify_current(root, STEM, &policy.resource_contribution, selected)?;
    let pending_path = super::joined(root, super::PENDING)?;
    if pending_path.exists() {
        std::fs::rename(pending_path, state_path)?;
    } else {
        super::write_json(&state_path, &next)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kfc::{
        container::KFCWriter,
        guid::ContentHash,
        hash::fnv,
        reflection::{PrimitiveType, TypeFlags, TypeIndex, TypeMetadata},
    };
    use std::{
        collections::BTreeMap,
        fs::{self, File, OpenOptions},
        io::Write,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    const TEMPLATE_TYPE: &str = "keen::ecs::TemplateResource";
    const PROGRAM_TYPE: &str = "keen::ImpactProgram";

    struct Fixture {
        root: PathBuf,
        stem: String,
        registry: TypeRegistry,
        contract: Contract,
        existing: ResourceId,
        referrer: ResourceId,
        program: ResourceId,
        original_existing: Vec<u8>,
        original_referrer: Vec<u8>,
    }

    struct DedicatedRegistryGuard(Option<fn() -> TypeRegistry>);

    impl DedicatedRegistryGuard {
        fn install(factory: fn() -> TypeRegistry) -> Self {
            let previous = DEDICATED_REGISTRY_FACTORY.with(|slot| slot.replace(Some(factory)));
            Self(previous)
        }
    }

    impl Drop for DedicatedRegistryGuard {
        fn drop(&mut self) {
            DEDICATED_REGISTRY_FACTORY.with(|slot| slot.set(self.0));
        }
    }

    impl Fixture {
        fn new(stem: &str) -> Result<Self> {
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let root = std::env::temp_dir().join(format!(
                "shroudforge-resource-contribution-{stem}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&root)?;

            let registry = test_registry()?;
            let template_hash = fnv(TEMPLATE_TYPE);
            let program_hash = fnv(PROGRAM_TYPE);
            let existing =
                ResourceId::parse("00000001-0000-0000-0000-000000000001", template_hash, 0)
                    .unwrap();
            let referrer =
                ResourceId::parse("00000002-0000-0000-0000-000000000002", template_hash, 0)
                    .unwrap();
            let program = ResourceId::parse(PROGRAM_GUID, program_hash, 0).unwrap();
            let original_existing = b"fixture original program template".to_vec();
            let original_referrer = format!("fixture references {program}").into_bytes();

            File::create(root.join(format!("{stem}.kfc")))?;
            File::create(root.join(format!("{stem}.kfc_resources")))?;
            let mut writer = KFCWriter::<KFCFile, _>::new(&root, stem, &registry, "fixture")?;
            writer.write_resource(&existing, &original_existing)?;
            writer.write_resource(&referrer, &original_referrer)?;
            let retained_content = b"fixture retained container content";
            writer.write_content(&ContentHash::from_data(retained_content), retained_content)?;
            writer.finalize()?;
            OpenOptions::new()
                .append(true)
                .open(root.join(format!("{stem}.kfc_resources")))?
                .write_all(b"retained unreferenced prefix")?;

            let file = KFCFile::from_path(root.join(format!("{stem}.kfc")), false)?;
            let mut reader = KFCReader::new(&root, stem)?.into_cursor()?;
            let mut resources = vec![
                resource_contract(
                    existing,
                    TEMPLATE_TYPE,
                    image(&file, &mut reader, existing)?,
                ),
                resource_contract(
                    referrer,
                    TEMPLATE_TYPE,
                    image(&file, &mut reader, referrer)?,
                ),
                resource_contract(program, PROGRAM_TYPE, image(&file, &mut reader, program)?),
            ];
            resources.sort_by(|left, right| {
                (&left.guid, left.type_hash, left.part_index).cmp(&(
                    &right.guid,
                    right.type_hash,
                    right.part_index,
                ))
            });
            let contract = Contract {
                schema_version: 1,
                target: stem.to_owned(),
                package_tree_sha256: "a".repeat(64),
                source_manifest_sha256: SOURCE_MANIFEST_SHA256.to_owned(),
                config_sha256: CONFIG_SHA256.to_owned(),
                resources,
            };
            Ok(Self {
                root,
                stem: stem.to_owned(),
                registry,
                contract,
                existing,
                referrer,
                program,
                original_existing,
                original_referrer,
            })
        }

        fn write_selected(&self, extra: Option<ResourceId>, add_content: bool) -> Result<()> {
            let stage = super::super::transaction::begin_generated(&self.root, &self.stem)?;
            let before = KFCFile::from_path(self.root.join(format!("{}.kfc", self.stem)), false)?;
            let mut writer = KFCWriter::new_generated_incremental(
                &self.root,
                &stage,
                &self.stem,
                &before,
                &self.registry,
            )?;
            writer.write_resource(&self.existing, b"selected program template")?;
            writer.write_resource(
                &self.referrer,
                format!("selected reference to {}", self.program).as_bytes(),
            )?;
            writer.write_resource(&self.program, b"selected impact program")?;
            if let Some(extra) = extra {
                writer.write_resource(&extra, b"undeclared added resource")?;
            }
            if add_content {
                let bytes = b"unreviewed container content";
                writer.write_content(&ContentHash::from_data(bytes), bytes)?;
            }
            writer.finalize()?;
            Ok(())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    const SERVER_STEM: &str = "enshrouded_server";

    fn dedicated_test_registry() -> TypeRegistry {
        test_registry().expect("synthetic reflection registry")
    }

    fn install_reviewed_source_projection(root: &Path, enabled: bool) -> Result<String> {
        let source_root = std::env::var_os("SHROUDFORGE_AUTOLOOT_SOURCE_ROOT")
            .map(PathBuf::from)
            .context("SHROUDFORGE_AUTOLOOT_SOURCE_ROOT must identify the reviewed AutoLoot handoff package")?;
        ensure!(
            source_root.is_dir(),
            "reviewed AutoLoot source directory is missing"
        );
        let package = root.join(PACKAGE_PATH);
        for relative in [
            "mod.json",
            "config.txt",
            "src/mod.lua",
            "src/autosmash_program.lua",
        ] {
            let destination = package.join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source_root.join(relative), destination)?;
        }
        fs::write(
            package.join("extended.mod.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "enabled": enabled,
                "targets": ["server"],
                "settings": {}
            }))?,
        )?;
        ensure!(
            !package.join("native/dinput8.dll").exists(),
            "server fixture must not include the client-only AutoLoot DLL"
        );
        let mut package_tree = BTreeMap::new();
        super::super::tree_hashes(root, PACKAGE_PATH, &mut package_tree, None)?;
        Ok(super::super::hash(&serde_json::to_vec(&package_tree)?))
    }

    struct DedicatedPolicyFixture {
        fixture: Fixture,
        contract: Contract,
        fingerprint: String,
    }

    impl DedicatedPolicyFixture {
        fn new() -> Result<Self> {
            let fixture = Fixture::new(SERVER_STEM)?;
            shroudforge_loader_config::config::initialize_loader_config(&fixture.root)
                .map_err(anyhow::Error::msg)?;
            let executable = b"synthetic dedicated executable for offline current-assets tests";
            fs::write(fixture.root.join("enshrouded_server.exe"), executable)?;
            let package_tree_sha256 = install_reviewed_source_projection(&fixture.root, false)?;
            let mut contract = fixture.contract.clone();
            contract.package_tree_sha256 = package_tree_sha256;

            let environment = mod_loader::ModEnvironment::load(
                fixture.root.to_str().context("non-UTF8 fixture path")?,
            )
            .map_err(anyhow::Error::msg)?;
            let composition_fingerprint =
                super::super::composition_fingerprint_for_with_contribution(
                    &fixture.root,
                    &environment,
                    &[],
                    true,
                )?;
            let mut outputs = BTreeMap::new();
            for entry in fs::read_dir(&fixture.root)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if super::super::valid_output_for(&name, SERVER_STEM) {
                    outputs.insert(name, super::super::stamp(&entry.path())?);
                }
            }
            let policy_path = super::super::joined(&fixture.root, super::super::POLICY)?;
            fs::create_dir_all(policy_path.parent().context("policy parent missing")?)?;
            let executable_sha256 = super::super::hash(executable);
            let generation =
                KFCFile::from_path(fixture.root.join(format!("{SERVER_STEM}.kfc")), false)?
                    .game_version()
                    .to_owned();
            super::super::write_json(
                &policy_path,
                &serde_json::json!({
                    "schemaVersion": 1,
                    "target": SERVER_STEM,
                    "modId": PACKAGE_ID,
                    "package": PACKAGE_PATH,
                    "generation": generation,
                    "executableSha256": executable_sha256,
                    "composition": {
                        "fingerprint": composition_fingerprint,
                        "outputs": outputs,
                        "sourceFingerprint": null,
                        "baselineEnabled": false,
                        "baselineLegacyPickaxeReachMigrated": false,
                        "excludedPackages": []
                    },
                    "resourceContribution": contract
                }),
            )?;

            let mut fingerprint_inputs = BTreeMap::new();
            fingerprint_inputs.insert("target".to_owned(), SERVER_STEM.to_owned());
            fingerprint_inputs.insert("composition".to_owned(), composition_fingerprint);
            fingerprint_inputs.insert("contribution".to_owned(), contract.digest()?);
            fingerprint_inputs.insert("generation".to_owned(), generation);
            fingerprint_inputs.insert("executable".to_owned(), executable_sha256);
            fingerprint_inputs.insert("policy".to_owned(), super::super::stamp(&policy_path)?.1);
            let fingerprint = super::super::hash(&serde_json::to_vec(&fingerprint_inputs)?);
            Ok(Self {
                fixture,
                contract,
                fingerprint,
            })
        }

        fn set_enabled(&self, enabled: bool) -> Result<()> {
            let path = self
                .fixture
                .root
                .join(PACKAGE_PATH)
                .join("extended.mod.json");
            let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            manifest["enabled"] = serde_json::Value::Bool(enabled);
            fs::write(path, serde_json::to_vec(&manifest)?)?;
            Ok(())
        }

        fn stage_interrupted_selection(&self, with_journal: bool) -> Result<()> {
            let root = &self.fixture.root;
            let state_path = super::super::joined(root, super::super::STATE)?;
            let previous: super::super::Prepared = super::super::read_json(&state_path)?;
            let initial = previous
                .resource_contribution
                .as_ref()
                .context("initial dedicated state has no contribution")?;
            self.set_enabled(true)?;
            let mut updated = apply_selection(
                root,
                SERVER_STEM,
                &self.contract,
                initial,
                &self.fixture.registry,
                true,
                || self.fixture.write_selected(None, false),
            )?;
            updated.set_previous_state_sha256(Some(super::super::stamp(&state_path)?.1));

            let stage = super::super::transaction::directory(root, SERVER_STEM);
            let index_name = format!("{SERVER_STEM}.kfc");
            let resources_name = format!("{SERVER_STEM}.kfc_resources");
            if with_journal {
                let records = vec![
                    (
                        resources_name.clone(),
                        Some(super::super::stamp(&root.join(&resources_name))?),
                        super::super::stamp(&stage.join(&resources_name))?,
                    ),
                    (
                        index_name.clone(),
                        Some(super::super::stamp(&root.join(&index_name))?),
                        super::super::stamp(&stage.join(&index_name))?,
                    ),
                ];
                super::super::write_json(
                    &stage.join("generated-pending.json"),
                    &(
                        1u32,
                        fs::canonicalize(root)?.to_string_lossy().into_owned(),
                        records,
                    ),
                )?;
            }

            let mut outputs = previous.outputs;
            for name in [&index_name, &resources_name] {
                outputs.insert(name.clone(), super::super::stamp(&stage.join(name))?);
            }
            let pending = super::super::Prepared {
                schema_version: 1,
                root: fs::canonicalize(root)?.to_string_lossy().into_owned(),
                fingerprint: self.fingerprint.clone(),
                enabled: false,
                configured_extended_zoom: None,
                effective_zoom: None,
                previous_state_sha256: None,
                outputs,
                target: Some(SERVER_STEM.to_owned()),
                resource_contribution: Some(updated),
            };
            super::super::write_json(
                &super::super::joined(root, super::super::PENDING)?,
                &pending,
            )?;
            Ok(())
        }
    }

    fn test_registry() -> Result<TypeRegistry> {
        fn metadata(index: usize, qualified_name: &str) -> TypeMetadata {
            let name = qualified_name.rsplit("::").next().unwrap();
            TypeMetadata {
                index: TypeIndex::new(index),
                name: name.to_owned(),
                impact_name: name.to_owned(),
                qualified_name: qualified_name.to_owned(),
                namespace: qualified_name
                    .split("::")
                    .take_while(|part| *part != name)
                    .map(str::to_owned)
                    .collect(),
                inner_type: None,
                size: 0,
                alignment: 0,
                element_alignment: 0,
                field_count: 0,
                primitive_type: PrimitiveType::Struct,
                flags: TypeFlags::HAS_DS,
                name_hash: fnv(name),
                impact_hash: fnv(qualified_name),
                qualified_hash: fnv(qualified_name),
                internal_hash: index as u32 + 1,
                struct_fields: Default::default(),
                enum_fields: Default::default(),
                default_value: None,
                attributes: Default::default(),
            }
        }

        Ok(serde_json::from_value(serde_json::json!({
            "version": "fixture",
            "types": [metadata(0, TEMPLATE_TYPE), metadata(1, PROGRAM_TYPE)]
        }))?)
    }

    fn resource_contract(id: ResourceId, type_name: &str, before: BeforeImage) -> ResourceContract {
        ResourceContract {
            guid: id.to_string(),
            type_hash: id.type_hash(),
            part_index: id.part_index(),
            type_name: type_name.to_owned(),
            before,
        }
    }

    fn image(
        file: &KFCFile,
        reader: &mut impl ResourceReader,
        id: ResourceId,
    ) -> Result<BeforeImage> {
        match (file.resources().get(&id), reader.read_resource(&id)?) {
            (Some(entry), Some(bytes)) => Ok(BeforeImage {
                present: true,
                mapping: Some(Mapping {
                    offset: entry.offset,
                    size: entry.size,
                }),
                sha256: Some(digest(&bytes)),
            }),
            (None, None) => Ok(BeforeImage {
                present: false,
                mapping: None,
                sha256: None,
            }),
            _ => anyhow::bail!("synthetic resource index and reader disagree"),
        }
    }

    fn commit(fixture: &Fixture) -> Result<()> {
        super::super::transaction::commit_generated(&fixture.root, &fixture.stem)?;
        Ok(())
    }

    #[test]
    fn client_and_server_contribution_apply_repeat_disable_and_reenable() -> Result<()> {
        for stem in ["enshrouded", "enshrouded_server"] {
            let fixture = Fixture::new(stem)?;
            let initial = fixture.contract.initial_state(false)?;
            initial.verify_current(&fixture.root, stem, &fixture.contract, false)?;
            let mut selected_runs = 0;
            let applied = apply_selection(
                &fixture.root,
                stem,
                &fixture.contract,
                &initial,
                &fixture.registry,
                true,
                || {
                    selected_runs += 1;
                    fixture.write_selected(None, false)
                },
            )?;
            assert!(applied.enabled());
            commit(&fixture)?;
            applied.verify_current(&fixture.root, stem, &fixture.contract, true)?;
            let first_after: Vec<_> = applied
                .resources
                .iter()
                .map(|entry| entry.after.as_ref().unwrap().sha256.clone())
                .collect();

            let repeated = apply_selection(
                &fixture.root,
                stem,
                &fixture.contract,
                &applied,
                &fixture.registry,
                true,
                || {
                    selected_runs += 1;
                    Ok(())
                },
            )?;
            assert_eq!(repeated, applied);
            assert_eq!(selected_runs, 1, "repeat selection reran AutoLoot");

            let disabled = apply_selection(
                &fixture.root,
                stem,
                &fixture.contract,
                &applied,
                &fixture.registry,
                false,
                || {
                    selected_runs += 1;
                    Ok(())
                },
            )?;
            assert!(!disabled.enabled());
            commit(&fixture)?;
            disabled.verify_current(&fixture.root, stem, &fixture.contract, false)?;
            assert_eq!(selected_runs, 1, "disable executed AutoLoot Lua");

            let restored = KFCFile::from_path(fixture.root.join(format!("{stem}.kfc")), false)?;
            assert_eq!(
                restored
                    .resources()
                    .get(&fixture.existing)
                    .map(|entry| (entry.offset, entry.size)),
                fixture
                    .contract
                    .resources
                    .iter()
                    .find(|entry| entry.guid == fixture.existing.to_string())
                    .and_then(|entry| entry.before.mapping.as_ref())
                    .map(|mapping| (mapping.offset, mapping.size))
            );
            assert_eq!(
                restored
                    .resources()
                    .get(&fixture.referrer)
                    .map(|entry| (entry.offset, entry.size)),
                fixture
                    .contract
                    .resources
                    .iter()
                    .find(|entry| entry.guid == fixture.referrer.to_string())
                    .and_then(|entry| entry.before.mapping.as_ref())
                    .map(|mapping| (mapping.offset, mapping.size))
            );
            assert!(!restored.resources().contains_key(&fixture.program));
            let mut reader = KFCReader::new(&fixture.root, stem)?.into_cursor()?;
            assert_eq!(
                reader.read_resource(&fixture.existing)?,
                Some(fixture.original_existing.clone())
            );
            assert_eq!(
                reader.read_resource(&fixture.referrer)?,
                Some(fixture.original_referrer.clone())
            );

            let reenabled = apply_selection(
                &fixture.root,
                stem,
                &fixture.contract,
                &disabled,
                &fixture.registry,
                true,
                || {
                    selected_runs += 1;
                    fixture.write_selected(None, false)
                },
            )?;
            commit(&fixture)?;
            reenabled.verify_current(&fixture.root, stem, &fixture.contract, true)?;
            assert_eq!(selected_runs, 2);
            assert_eq!(
                reenabled
                    .resources
                    .iter()
                    .map(|entry| entry.after.as_ref().unwrap().sha256.clone())
                    .collect::<Vec<_>>(),
                first_after
            );
        }
        Ok(())
    }

    #[test]
    fn undeclared_resource_and_content_changes_are_refused() -> Result<()> {
        for add_content in [false, true] {
            let fixture = Fixture::new("enshrouded")?;
            let state = fixture.contract.initial_state(false)?;
            let extra = ResourceId::parse(
                "00000003-0000-0000-0000-000000000003",
                fnv(TEMPLATE_TYPE),
                0,
            );
            fixture.write_selected((!add_content).then_some(extra.unwrap()), add_content)?;
            let stage = super::super::transaction::directory(&fixture.root, "enshrouded");
            assert!(
                capture_after(
                    &fixture.root,
                    &stage,
                    "enshrouded",
                    &fixture.contract,
                    &state,
                    &fixture.registry,
                )
                .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn interrupted_generated_publication_recovers_from_the_shared_pending_receipt() -> Result<()> {
        let fixture = Fixture::new("enshrouded")?;
        let fingerprint = "f".repeat(64);
        let initial = fixture.contract.initial_state(false)?;
        let applied = apply_selection(
            &fixture.root,
            "enshrouded",
            &fixture.contract,
            &initial,
            &fixture.registry,
            true,
            || fixture.write_selected(None, false),
        )?;
        let stage = super::super::transaction::directory(&fixture.root, "enshrouded");
        let mut outputs = BTreeMap::new();
        for entry in fs::read_dir(&fixture.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if super::super::valid_output_for(&name, "enshrouded") {
                outputs.insert(name, super::super::stamp(&entry.path())?);
            }
        }
        for entry in fs::read_dir(&stage)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if super::super::valid_output_for(&name, "enshrouded") {
                outputs.insert(name, super::super::stamp(&entry.path())?);
            }
        }
        let next = super::super::Prepared {
            schema_version: 1,
            root: fixture.root.to_string_lossy().into_owned(),
            fingerprint: fingerprint.clone(),
            enabled: false,
            configured_extended_zoom: None,
            effective_zoom: None,
            previous_state_sha256: None,
            outputs,
            target: Some("enshrouded".into()),
            resource_contribution: Some(applied),
        };
        let index_name = "enshrouded.kfc";
        let resources_name = "enshrouded.kfc_resources";
        let records = vec![
            (
                resources_name.to_owned(),
                Some(super::super::stamp(&fixture.root.join(resources_name))?),
                super::super::stamp(&stage.join(resources_name))?,
            ),
            (
                index_name.to_owned(),
                Some(super::super::stamp(&fixture.root.join(index_name))?),
                super::super::stamp(&stage.join(index_name))?,
            ),
        ];
        fs::write(
            stage.join("generated-pending.json"),
            serde_json::to_vec(&(
                1u32,
                fs::canonicalize(&fixture.root)?
                    .to_string_lossy()
                    .to_string(),
                records,
            ))?,
        )?;
        let pending_path = super::super::joined(&fixture.root, super::super::PENDING)?;
        fs::create_dir_all(pending_path.parent().unwrap())?;
        super::super::write_json(&pending_path, &next)?;

        super::super::recover_contribution_transaction(
            &fixture.root,
            "enshrouded",
            &fingerprint,
            true,
            None,
            &fixture.contract,
            None,
            None,
        )?;

        assert!(!stage.exists());
        assert!(!pending_path.exists());
        let state_path = super::super::joined(&fixture.root, super::super::STATE)?;
        let recovered: super::super::Prepared = super::super::read_json(&state_path)?;
        assert!(recovered == next);
        recovered
            .resource_contribution
            .as_ref()
            .unwrap()
            .verify_current(&fixture.root, "enshrouded", &fixture.contract, true)?;
        Ok(())
    }

    #[test]
    #[ignore = "requires the private reviewed AutoLoot source projection"]
    fn dedicated_public_prepare_initializes_the_server_receipt() -> Result<()> {
        let prepared = DedicatedPolicyFixture::new()?;
        let _registry = DedicatedRegistryGuard::install(dedicated_test_registry);

        assert!(super::super::prepare_for(
            &prepared.fixture.root,
            SERVER_STEM
        )?);
        let state_path = super::super::joined(&prepared.fixture.root, super::super::STATE)?;
        let state: super::super::Prepared = super::super::read_json(&state_path)?;
        assert_eq!(state.target.as_deref(), Some(SERVER_STEM));
        let contribution = state
            .resource_contribution
            .as_ref()
            .context("successful dedicated prepare omitted contribution state")?;
        assert!(!contribution.enabled());
        contribution.verify_current(
            &prepared.fixture.root,
            SERVER_STEM,
            &prepared.contract,
            false,
        )?;
        assert!(!super::super::joined(&prepared.fixture.root, super::super::PENDING)?.exists());
        assert!(
            !super::super::transaction::directory(&prepared.fixture.root, SERVER_STEM).exists()
        );
        assert!(
            !prepared
                .fixture
                .root
                .join("enshrouded_server.kfc.bak")
                .exists()
        );
        super::super::clear_startup_zoom_demand();
        Ok(())
    }

    #[test]
    #[ignore = "requires the private reviewed AutoLoot source projection"]
    fn public_dedicated_prepare_recovers_server_publication_with_and_without_inner_journal()
    -> Result<()> {
        for with_journal in [false, true] {
            let prepared = DedicatedPolicyFixture::new()?;
            let _registry = DedicatedRegistryGuard::install(dedicated_test_registry);
            let root = &prepared.fixture.root;

            assert!(super::super::prepare_for(root, SERVER_STEM)?);
            prepared.stage_interrupted_selection(with_journal)?;
            assert!(super::super::prepare_for(root, SERVER_STEM)?);

            let state_path = super::super::joined(root, super::super::STATE)?;
            let state: super::super::Prepared = super::super::read_json(&state_path)?;
            assert_eq!(state.target.as_deref(), Some(SERVER_STEM));
            let contribution = state
                .resource_contribution
                .as_ref()
                .context("recovered dedicated state omitted contribution")?;
            assert!(contribution.enabled());
            contribution.verify_current(root, SERVER_STEM, &prepared.contract, true)?;
            assert!(!super::super::joined(root, super::super::PENDING)?.exists());
            assert!(!super::super::transaction::directory(root, SERVER_STEM).exists());
            assert!(!root.join("enshrouded_server.kfc.bak").exists());
            super::super::clear_startup_zoom_demand();
        }
        Ok(())
    }

    #[test]
    fn source_file_pins_match_the_reviewed_owner_manifest_projection() {
        let production = [
            ("sourceManifest", SOURCE_MANIFEST_SHA256),
            ("mods/auto-loot-plus/src/mod.lua", LUA_SHA256),
            (
                "mods/auto-loot-plus/src/autosmash_program.lua",
                PROGRAM_LUA_SHA256,
            ),
            ("mods/auto-loot-plus/config.txt", CONFIG_SHA256),
            ("mods/auto-loot-plus/native/dinput8.dll", CLIENT_DLL_SHA256),
        ];
        let reviewed = [
            (
                "sourceManifest",
                "4c0b3205e1ca1e139952294b91b3d91555ddb89480565c3fcf643c24cddf9452",
            ),
            (
                "mods/auto-loot-plus/src/mod.lua",
                "7d524e03a845d11ca58eff2ab26bfb182eecce7639a10f10253d60f88ad7344e",
            ),
            (
                "mods/auto-loot-plus/src/autosmash_program.lua",
                "aa9992e2af2ca505acf869f299846db3a0e0fd375692bd8c11b614a8ffb14e17",
            ),
            (
                "mods/auto-loot-plus/config.txt",
                "d3229deb5e522bed0f4df372f032107144841757ad1a68e84fa2a80741f60569",
            ),
            (
                "mods/auto-loot-plus/native/dinput8.dll",
                "f885ac8032109838cfefa9fb34307e4dbbc396472e3cf465b656915559b89c46",
            ),
        ];
        assert_eq!(production, reviewed);
    }

    #[test]
    fn source_config_and_cross_target_contract_changes_fail_closed() -> Result<()> {
        let fixture = Fixture::new("enshrouded")?;
        let mut changed_source = fixture.contract.clone();
        changed_source.source_manifest_sha256 = "0".repeat(64);
        assert!(
            changed_source
                .validate(&fixture.root, "enshrouded", &fixture.registry)
                .is_err()
        );

        let mut changed_config = fixture.contract.clone();
        changed_config.config_sha256 = "0".repeat(64);
        assert!(
            changed_config
                .validate(&fixture.root, "enshrouded", &fixture.registry)
                .is_err()
        );

        let initial = fixture.contract.initial_state(false)?;
        let mut wrong_target = fixture.contract.clone();
        wrong_target.target = "enshrouded_server".into();
        let mut callback_called = false;
        assert!(
            apply_selection(
                &fixture.root,
                "enshrouded",
                &wrong_target,
                &initial,
                &fixture.registry,
                true,
                || {
                    callback_called = true;
                    Ok(())
                },
            )
            .is_err()
        );
        assert!(!callback_called);
        Ok(())
    }
}
