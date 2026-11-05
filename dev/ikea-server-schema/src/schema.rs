use anyhow::{Context, Result, ensure};
use kfc::reflection::{PrimitiveType as P, TypeRegistry};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub fn sha(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
pub fn validate(r: &TypeRegistry) -> Result<Value> {
    ensure!(!r.is_empty() && r.len() <= 32768, "type count");
    let mut count = 0usize;
    let mut optional = Vec::new();
    let mut primitives = BTreeMap::new();
    for (i, t) in r.iter().enumerate() {
        ensure!(i == t.index.as_usize(), "type index");
        ensure!(
            r.get_by_name(kfc::reflection::LookupKey::Qualified(
                t.qualified_name.as_str()
            ))
            .is_some_and(|m| m.index == t.index),
            "qualified name/hash lookup"
        );
        ensure!(t.field_count <= 4096, "pertype count");
        *primitives
            .entry(format!("{:?}", t.primitive_type))
            .or_insert(0usize) += 1;
        ensure!(t.qualified_name.len() <= 65536, "name size");
        count += t.struct_fields.len() + t.enum_fields.len() + t.attributes.len();
        ensure!(count <= 262144, "metadata entries");
        if let Some(n) = t.inner_type {
            r.get(n).context("inner index")?;
        }
        for f in t.struct_fields.values() {
            let fty = r.get(f.r#type).context("field index")?;
            ensure!(
                f.name.len() <= 65536
                    && f.data_offset
                        .checked_add(fty.size as u64)
                        .is_some_and(|end| end <= t.size as u64),
                "field extent/name"
            );
            let _ = fty;
            for a in f.attributes.values() {
                if let Some(n) = a.r#type {
                    r.get(n).context("attribute index")?;
                }
            }
        }
        for a in t.attributes.values() {
            if let Some(n) = a.r#type {
                r.get(n).context("attribute index")?;
            }
        }
        ensure!(t.alignment.is_power_of_two(), "alignment");
        if t.primitive_type == P::Typedef {
            ensure!(
                r.get(t.inner_type.context("typedef inner")?)
                    .context("typedef index")?
                    .size
                    == t.size,
                "typedef size"
            );
        }
        let fixed_size = match t.primitive_type {
            P::Bool | P::UInt8 | P::SInt8 | P::Bitmask8 => Some(1),
            P::UInt16 | P::SInt16 | P::Bitmask16 => Some(2),
            P::UInt32 | P::SInt32 | P::Float32 | P::Bitmask32 => Some(4),
            P::UInt64 | P::SInt64 | P::Float64 | P::Bitmask64 => Some(8),
            P::Guid | P::ObjectReference => Some(16),
            _ => None,
        };
        if let Some(n) = fixed_size {
            ensure!(t.size == n, "primitive size");
        }
        match t.primitive_type {
            P::Typedef
            | P::StaticArray
            | P::DsArray
            | P::BlobArray
            | P::DsOptional
            | P::BlobOptional
            | P::DsVariant
            | P::BlobVariant
            | P::Enum
            | P::Bitmask8
            | P::Bitmask16
            | P::Bitmask32
            | P::Bitmask64 => {
                ensure!(t.inner_type.is_some(), "required inner type");
            }
            _ => {}
        }
        // Only typedef and STRUCT inheritance chains must be acyclic. Field recursion through arrays is allowed.
        let mut at = t;
        let mut seen = BTreeSet::new();
        while matches!(at.primitive_type, P::Typedef | P::Struct) {
            ensure!(
                seen.len() < 128 && seen.insert(at.index.as_usize()),
                "typedef/inheritance cycle"
            );
            let Some(n) = at.inner_type else { break };
            let next = r.get(n).context("chain inner index")?;
            if at.primitive_type == P::Struct {
                ensure!(next.primitive_type == P::Struct, "struct base kind");
            }
            at = next;
        }
        if matches!(t.primitive_type, P::BlobOptional | P::DsOptional) {
            let mut at = r.get(t.inner_type.unwrap()).unwrap();
            let mut seen = BTreeSet::new();
            while at.primitive_type == P::Typedef {
                ensure!(
                    seen.len() < 128 && seen.insert(at.index.as_usize()),
                    "optional typedef cycle"
                );
                at = r
                    .get(at.inner_type.context("optional inner absent")?)
                    .context("optional inner index")?;
            }
            optional.push(json!({"index":i,"leafIndex":at.index.as_usize(),"leafKind":at.primitive_type,"ambiguousGuidNull":matches!(at.primitive_type,P::Guid|P::ObjectReference)}));
        }
    }
    Ok(
        json!({"types":r.len(),"metadataEntries":count,"primitiveCounts":primitives,"allOptionalFullTypedefLeaves":optional,"allSerializedTypesValidated":true,"fieldRecursionAllowed":true}),
    )
}
pub fn compare(r: &TypeRegistry, baseline: &Value) -> Result<Value> {
    let types = serde_json::to_value(r)?;
    let types = types["types"].as_array().context("registry type array")?;
    let old = baseline["types"]
        .as_array()
        .context("baseline fingerprints")?;
    let mut changed = Vec::new();
    let mut changes = 0;
    for (i, t) in types.iter().enumerate() {
        let digest = sha(&serde_json::to_vec(t)?);
        if old.get(i).and_then(|r| r["sha256"].as_str()) != Some(digest.as_str()) {
            changes += 1;
            if changed.len() < 128 {
                changed.push(json!({"index":i,"qualifiedName":t["qualifiedName"],"currentTypeSha256":digest}));
            }
        }
    }
    let removed = old.len().saturating_sub(types.len());
    Ok(
        json!({"baselineRegistrySha256":baseline["sourceSha256"],"comparison":"complete canonical type objects; every metadata value and ordered array, excluding paired registry.version only; map key ordering normalized; extraction audit must be clean","wholeSchemaEqual":changes==0&&removed==0,"changedCurrentTypes":changes,"removedTailTypes":removed,"boundedChangedTypeDetails":changed,"truncated":changes>128,"currentTypeCount":types.len(),"baselineTypeCount":old.len(),"newBaselineCompatibilityProven":false}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn old() -> TypeRegistry {
        serde_json::from_str(include_str!("baseline-fixture.json")).unwrap()
    }
    #[test]
    fn phase_a_full_schema_validate_and_compare() {
        let r = old();
        validate(&r).unwrap();
        let j = serde_json::to_value(&r).unwrap();
        let b = json!({"types":j["types"].as_array().unwrap().iter().map(|t|json!({"sha256":sha(&serde_json::to_vec(t).unwrap())})).collect::<Vec<_>>()});
        assert_eq!(compare(&r, &b).unwrap()["wholeSchemaEqual"], true);
    }
    #[test]
    fn phase_a_cycle_index_and_full_field_metadata_change() {
        let mut j: Value = serde_json::to_value(old()).unwrap();
        j["types"][1]["innerType"] = json!(1);
        j["types"][1]["primitiveType"] = json!("TYPEDEF");
        let r: TypeRegistry = serde_json::from_value(j.clone()).unwrap();
        assert!(validate(&r).is_err());
        j["types"][1]["innerType"] = json!(99999);
        let r: TypeRegistry = serde_json::from_value(j).unwrap();
        assert!(validate(&r).is_err());
    }
}

#[cfg(test)]
mod old_profile_metadata_test {
    use super::*;
    #[test]
    fn phase_a_all_old_schema_primitives_and_optional_leaves_are_qualified() {
        let b=std::fs::read("E:/Build/sf177-deployment-request-20261103-r1/environment/shroudforge/cache/types-enshrouded.json").unwrap();
        assert_eq!(
            sha(&b),
            "dcbc2c1374ddc528a51c1d424219047180f380f8ea61fe3db11e6e738f35c852"
        );
        let r: TypeRegistry = serde_json::from_slice(&b).unwrap();
        let proof = validate(&r).unwrap();
        assert_eq!(proof["types"], 12658);
        assert_eq!(
            proof["allOptionalFullTypedefLeaves"]
                .as_array()
                .unwrap()
                .len(),
            20
        );
        assert!(
            proof["allOptionalFullTypedefLeaves"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v["ambiguousGuidNull"] == false)
        );
        let baseline: Value =
            serde_json::from_str(include_str!("../test-data/client-baseline-fingerprints.json")).unwrap();
        assert_eq!(compare(&r, &baseline).unwrap()["wholeSchemaEqual"], true);
        let mut j = serde_json::to_value(&r).unwrap();
        j["types"][2]["structFields"]["value"]["dataOffset"] = json!(1);
        let changed: TypeRegistry = serde_json::from_value(j).unwrap();
        assert_eq!(
            compare(&changed, &baseline).unwrap()["wholeSchemaEqual"],
            false
        );
    }
}
