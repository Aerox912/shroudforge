//! One-time removal of the exact seven-field legacy XHL startup patch.
//! References contain reviewed field values and identities, never asset copies.
use anyhow::{Context, Result, ensure};
use kfc::{
    container::KFCReader,
    guid::ResourceId,
    reflection::{LookupKey, TypeRegistry},
    resource::value::Value,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path};

const LEGACY_LUA: &str = "1cbabd125b4204c1165ca0748b2fe627f4f88fa0e2ded2f31f13af42473a5d7d";
const REFERENCE_INDEX: &str = "54a8cb76b3229c486ef55a7ababcd48828fa0ffc5bb144d206f687fd0ab00a92";
const IDS: [&str; 7] = [
    "59dba0f3-81b8-4ac5-88d7-ba97f4d2928c",
    "46ea9b46-445a-4e36-bf84-20cab7585f48",
    "70ecc7fb-6ed1-416c-a51e-0362b2ebdd1a",
    "c03c05ce-4cc0-40fd-b201-58d05c730767",
    "2e4b3166-4960-48b1-ad1f-f3bb9d0d981a",
    "aaff198b-9139-47f5-8e41-b1ce0262abc8",
    "aee6b3ba-adeb-4264-b117-25f974a08cb8",
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Reference {
    schema_version: u32,
    generation: String,
    legacy_lua_sha256: String,
    reference_index_sha256: String,
    records: Vec<Record>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    id: String,
    current_resource_sha256: String,
    reference_resource_sha256: String,
    original_max_distance: f64,
}

pub(crate) fn registry(root: &Path) -> Result<TypeRegistry> {
    let cache = shroudforge_loader_config::paths::cache_dir(root);
    crate::reflection::load_type_registry::load_current_composition_registry(
        camino::Utf8Path::from_path(root).context("non-UTF8 root")?,
        camino::Utf8Path::from_path(&cache).context("non-UTF8 cache")?,
        "enshrouded",
    )
    .map(|(registry, _)| registry)
    .map_err(|()| anyhow::anyhow!("legacy reach type registry identity unavailable"))
}

fn distance(value: &mut Value) -> Result<&mut Value> {
    value
        .as_struct_mut()
        .context("ItemInfo is not a struct")?
        .get_mut("equipment")
        .context("ItemInfo equipment missing")?
        .as_struct_mut()
        .context("equipment is not a struct")?
        .get_mut("maxDistance")
        .context("equipment.maxDistance missing")
}

/// Validate every source record before returning any edit for the shared stage.
/// Later starts only verify the migrated field and never reapply the transition.
pub(crate) fn plan(
    root: &Path,
    generation: &str,
    reference: &Reference,
    migrated: bool,
) -> Result<Vec<(ResourceId, Vec<u8>)>> {
    ensure!(
        reference.schema_version == 1
            && reference.generation == generation
            && reference.legacy_lua_sha256 == LEGACY_LUA
            && reference.reference_index_sha256 == REFERENCE_INDEX,
        "unrecognized legacy reach reference identity"
    );
    ensure!(
        reference.records.len() == IDS.len(),
        "incomplete legacy reach reference"
    );
    let registry = registry(root)?;
    let ty = registry
        .get_by_name(LookupKey::Qualified("keen::ItemInfo"))
        .context("ItemInfo type unavailable")?;
    let mut reader = KFCReader::new(root, "enshrouded")?.into_cursor()?;
    let mut seen = HashSet::new();
    let mut edits = Vec::new();
    for record in &reference.records {
        ensure!(
            IDS.contains(&record.id.as_str())
                && seen.insert(record.id.as_str())
                && record.original_max_distance == 5.0
                && record.reference_resource_sha256.len() == 64
                && record
                    .reference_resource_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "invalid legacy reach field reference"
        );
        let id =
            ResourceId::parse(&record.id, ty.qualified_hash, 0).context("invalid pickaxe ID")?;
        let bytes = reader
            .read_resource(&id)?
            .context("legacy pickaxe missing")?;
        let mut value = Value::from_bytes(&registry, ty, &bytes)?;
        let field = distance(&mut value)?;
        ensure!(
            field.as_f64() == Some(if migrated { 5.0 } else { 10.0 }),
            "unexpected legacy pickaxe reach"
        );
        if migrated {
            continue;
        }
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == record.current_resource_sha256,
            "legacy pickaxe source record changed"
        );
        *field = Value::Float(record.original_max_distance);
        let mut edited = Vec::new();
        value.write_into(&registry, ty, &mut edited)?;
        // A typed round trip must preserve every other field in the current
        // composed ItemInfo, including fields that differ from stock records.
        let decoded = Value::from_bytes(&registry, ty, &edited)?;
        ensure!(
            decoded == value,
            "legacy reach serialization changed other typed fields"
        );
        edits.push((id, edited));
    }
    Ok(edits)
}
