//! Narrow typed field inverse. Never retains or restores an original UI bundle.
use anyhow::{Context, Result, ensure};
use kfc::{
    container::KFCReader,
    guid::ResourceId,
    reflection::{LookupKey, TypeRegistry},
    resource::value::Value,
};
use serde::Deserialize;
use std::{fs, path::Path};

pub(super) const GENERATION: &str =
    "1273043|^/game38/branches/release_1.0|2026-10-20T14:55:29.904125Z";
const BUNDLE: &str = "46e14df6-fb8f-4f66-9463-e23b9e19e48e";
const ROOT: &str = "0942dd71-2a74-4511-a853-349c5eed42a0";
const BUNDLE_TYPE: u32 = 4277737656;
const ROOT_TYPE: u32 = 3483826245;
const BUNDLE_NAME: &str = "keen::FbUiBundle";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Contract {
    pub registry_sha256: String,
}

pub(super) fn id() -> ResourceId {
    ResourceId::parse(BUNDLE, BUNDLE_TYPE, 0).unwrap()
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.as_struct()
        .and_then(|v| v.get(key))
        .with_context(|| format!("zoom field missing: {key}"))
}
fn field_mut<'a>(v: &'a mut Value, key: &str) -> Result<&'a mut Value> {
    v.as_struct_mut()
        .and_then(|v| v.get_mut(key))
        .with_context(|| format!("zoom field missing: {key}"))
}
fn configs(v: &Value) -> Result<&[Value]> {
    field(field(field(v, "menu")?, "map")?, "zoomConfigs")?
        .as_array()
        .context("zoomConfigs is not an array")
}
fn profile(extended: bool) -> [f32; 5] {
    if extended {
        [16., 8., 4., 1., 0.35]
    } else {
        [16., 8., 4., 2., 0.7]
    }
}
pub(super) fn transform(value: &Value, was_extended: bool, extended: bool) -> Result<Value> {
    let entries = configs(value)?;
    ensure!(entries.len() == 5, "unsupported zoom count");
    for (entry, expected) in entries.iter().zip(profile(was_extended)) {
        ensure!(
            matches!(field(entry,"zoomScale")?, Value::Float(v) if *v == expected as f64),
            "zoom scale conflicts with recorded effective state"
        );
    }
    let mut next = value.clone();
    let entries = field_mut(
        field_mut(field_mut(&mut next, "menu")?, "map")?,
        "zoomConfigs",
    )?
    .as_array_mut()
    .context("zoomConfigs is not an array")?;
    for i in [3, 4] {
        *field_mut(&mut entries[i], "zoomScale")? = Value::Float(profile(extended)[i] as f64);
    }
    Ok(next)
}

pub(super) fn registry(
    root: &Path,
    contract: &Contract,
    executable_hash: &str,
) -> Result<TypeRegistry> {
    let path = super::joined(root, "shroudforge/cache/types-enshrouded.json")?;
    ensure!(
        super::stamp(&path)?.1 == contract.registry_sha256,
        "zoom type registry changed"
    );
    let identity: serde_json::Value = super::read_json(&super::joined(
        root,
        "shroudforge/cache/types-enshrouded.identity.json",
    )?)?;
    ensure!(
        identity
            == serde_json::json!({"schemaVersion":2,"executableSha256":executable_hash,
        "reflectionExtractor":"kfc-a404b15cde40fc7bd43d812bb460ce40d2a9d3b1"}),
        "zoom registry executable identity mismatch"
    );
    let registry: TypeRegistry = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        registry.version == GENERATION,
        "zoom registry generation mismatch"
    );
    Ok(registry)
}

fn read_bundle(root: &Path, registry: &TypeRegistry) -> Result<Value> {
    read_bundle_from(KFCReader::new(root, super::STEM)?, registry)
}

fn read_bundle_from(reader: KFCReader, registry: &TypeRegistry) -> Result<Value> {
    ensure!(
        reader.file().game_version() == GENERATION,
        "zoom resource generation mismatch"
    );
    let mut reader = reader.into_cursor()?;
    let bytes = reader
        .read_resource(&ResourceId::parse(ROOT, ROOT_TYPE, 0).unwrap())?
        .context("zoom client root absent")?;
    ensure!(bytes.len() <= 32768, "oversized zoom client root");
    let meta = registry
        .get_by_name(LookupKey::Qualified("keen::Game38ClientResources"))
        .context("client root type absent")?;
    let client = Value::from_bytes(registry, meta, &bytes)?;
    ensure!(
        field(&client, "fbUiBundle")?
            .as_guid()
            .is_some_and(|v| v.to_string().to_lowercase() == BUNDLE),
        "zoom client root link mismatch"
    );
    let bytes = reader.read_resource(&id())?.context("zoom bundle absent")?;
    ensure!(bytes.len() <= 1024 * 1024, "oversized zoom bundle");
    let meta = registry
        .get_by_name(LookupKey::Qualified(BUNDLE_NAME))
        .context("zoom bundle type absent")?;
    let value = Value::from_bytes(registry, meta, &bytes)?;
    let roundtrip = value.to_bytes(registry, meta)?;
    ensure!(
        Value::from_bytes(registry, meta, &roundtrip)? == value,
        "zoom typed source roundtrip mismatch"
    );
    Ok(value)
}

pub(super) fn verify_profile(
    reader: KFCReader,
    registry: &TypeRegistry,
    extended: bool,
) -> Result<()> {
    let value = read_bundle_from(reader, registry)?;
    transform(&value, extended, extended)?;
    Ok(())
}

pub(super) struct Change {
    pub registry: TypeRegistry,
    expected: Value,
    pub bytes: Vec<u8>,
}
impl Change {
    pub fn new(root: &Path, registry: TypeRegistry, was: bool, desired: bool) -> Result<Self> {
        let before = read_bundle(root, &registry)?;
        let expected = transform(&before, was, desired)?;
        let meta = registry
            .get_by_name(LookupKey::Qualified(BUNDLE_NAME))
            .context("zoom bundle type absent")?;
        let bytes = expected.to_bytes(&registry, meta)?;
        ensure!(
            Value::from_bytes(&registry, meta, &bytes)? == expected,
            "zoom typed postimage roundtrip mismatch"
        );
        Ok(Self {
            registry,
            expected,
            bytes,
        })
    }
    pub fn verify(&self, root: &Path) -> Result<()> {
        ensure!(
            read_bundle(root, &self.registry)? == self.expected,
            "zoom full typed postimage mismatch"
        );
        Ok(())
    }
}
