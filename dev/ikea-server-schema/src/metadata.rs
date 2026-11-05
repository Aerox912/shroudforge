use anyhow::{Context, Result, ensure};
use kfc::container::KFCFile;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Cursor;
// Pinned SDK KFCHeader is private. This bounded preflight reads only its16 relative-offset/count pairs; actual parsing remains SDK KFCFile::from_reader.
struct Location {
    offset: u64,
    count: usize,
}
struct Header {
    version: Location,
    containers: Location,
    unused0: Location,
    unused1: Location,
    resource_locations: Location,
    resource_indices: Location,
    content_buckets: Location,
    content_keys: Location,
    content_values: Location,
    resource_buckets: Location,
    resource_keys: Location,
    resource_values: Location,
    resource_bundle_buckets: Location,
    resource_bundle_keys: Location,
    resource_bundle_values: Location,
    resource_chunks: Location,
}
impl Header {
    fn read(raw: &[u8]) -> Result<Self> {
        ensure!(raw.len() >= 144 && &raw[..4] == b"KFC4", "KFC header");
        let mut at = 16;
        let mut loc = || {
            let offset = at as u64 + u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()) as u64;
            let count = u32::from_le_bytes(raw[at + 4..at + 8].try_into().unwrap()) as usize;
            at += 8;
            Location { offset, count }
        };
        Ok(Self {
            version: loc(),
            containers: loc(),
            unused0: loc(),
            unused1: loc(),
            resource_locations: loc(),
            resource_indices: loc(),
            content_buckets: loc(),
            content_keys: loc(),
            content_values: loc(),
            resource_buckets: loc(),
            resource_keys: loc(),
            resource_values: loc(),
            resource_bundle_buckets: loc(),
            resource_bundle_keys: loc(),
            resource_bundle_values: loc(),
            resource_chunks: loc(),
        })
    }
}
pub fn parse(raw: &[u8]) -> Result<KFCFile> {
    ensure!(raw.len() <= 9396224, "KFC length");
    let h = Header::read(raw)?;
    let fields = [
        (&h.version, 1, 4096),
        (&h.containers, 16, 256),
        (&h.unused0, 1, 0),
        (&h.unused1, 1, 0),
        (&h.resource_locations, 12, 256),
        (&h.resource_indices, 4, 250000),
        (&h.content_buckets, 8, 250000),
        (&h.content_keys, 16, 250000),
        (&h.content_values, 16, 250000),
        (&h.resource_buckets, 8, 250000),
        (&h.resource_keys, 32, 250000),
        (&h.resource_values, 8, 250000),
        (&h.resource_bundle_buckets, 8, 32768),
        (&h.resource_bundle_keys, 4, 32768),
        (&h.resource_bundle_values, 12, 32768),
        (&h.resource_chunks, 20, 250000),
    ];
    let mut total = 0;
    for (l, width, max) in fields {
        ensure!(l.count <= max, "KFC count");
        total += l.count;
        ensure!(
            l.offset
                .checked_add(
                    (l.count as u64)
                        .checked_mul(width)
                        .context("table product")?
                )
                .is_some_and(|end| end <= raw.len() as u64),
            "KFC extent"
        );
    }
    ensure!(total <= 2000000, "KFC total entries");
    ensure!(
        h.content_keys.count == h.content_values.count
            && h.resource_keys.count == h.resource_values.count,
        "KFC parallel metadata"
    );
    let read32 = |p: usize| -> u32 { u32::from_le_bytes(raw[p..p + 4].try_into().unwrap()) };
    for i in 0..h.resource_indices.count {
        ensure!(
            (read32(h.resource_indices.offset as usize + i * 4) as usize) < h.resource_keys.count,
            "resource index"
        );
    }
    for i in 0..h.resource_bundle_values.count {
        let p = h.resource_bundle_values.offset as usize + i * 12;
        ensure!(
            (read32(p + 4) as usize)
                .checked_add(read32(p + 8) as usize)
                .is_some_and(|end| end <= h.resource_indices.count),
            "bundle index"
        );
    }
    Ok(KFCFile::from_reader(&mut Cursor::new(raw), false)?)
}
pub fn summary(f: &KFCFile, registry: &kfc::reflection::TypeRegistry) -> Result<Value> {
    let mut h = Sha256::new();
    h.update(b"hdmap-container-prefix-v1");
    h.update((f.containers().len() as u64).to_le_bytes());
    for c in f.containers() {
        h.update(c.size.to_le_bytes());
        h.update((c.count as u64).to_le_bytes());
    }
    let container = format!("{:x}", h.finalize());
    let mut contents: Vec<_> = f.contents().iter().collect();
    contents.sort_by_key(|(k, _)| **k);
    let mut h = Sha256::new();
    h.update(b"hdmap-content-map-v1");
    h.update((contents.len() as u64).to_le_bytes());
    let mut content_keys = std::collections::BTreeSet::new();
    for (k, v) in &contents {
        ensure!(content_keys.insert(**k), "duplicate content hash");
        let looked = f
            .contents()
            .get(k)
            .context("content key absent from SDK lookup")?;
        ensure!(
            looked.offset == v.offset
                && looked.flags == v.flags
                && looked.container_index == v.container_index,
            "content SDK lookup/key mismatch"
        );
        ensure!(
            v.container_index < f.containers().len(),
            "content container index"
        );
        ensure!(
            v.offset
                .checked_add(k.size() as u64)
                .is_some_and(|end| end <= f.containers()[v.container_index].size),
            "SDK raw content metadata range"
        );
        for n in [k.size(), k.hash0(), k.hash1(), k.hash2()] {
            h.update(n.to_le_bytes());
        }
        h.update(v.offset.to_le_bytes());
        h.update(v.flags.to_le_bytes());
        h.update((v.container_index as u64).to_le_bytes());
    }
    let content = format!("{:x}", h.finalize());
    let roots: [&str; 21] = [
        "keen::ItemInfo",
        "keen::SceneResource",
        "keen::BuffTypeCollectionResource",
        "keen::ecs::TemplateResource",
        "keen::RecipeRegistryResource",
        "keen::actor::ActorSequenceResource",
        "keen::LocaTagCollectionResource",
        "keen::ItemRegistryResource",
        "keen::ItemKnowledgeResource",
        "keen::GameKnowledgeResource",
        "keen::FbUiBundle",
        "keen::ActorSequenceCollectionResource",
        "keen::ImpactRegistryResource",
        "keen::ImpactProgram",
        "keen::UiTextureResource",
        "keen::VfxResource",
        "keen::VfxDefinition",
        "keen::SoundContainerResource",
        "keen::SoundResource",
        "keen::Game38ClientResources",
        "keen::GpcResource",
    ];
    let hashes: std::collections::BTreeMap<_, _> = roots
        .iter()
        .filter_map(|n| {
            registry
                .get_by_name(kfc::reflection::LookupKey::Qualified(*n))
                .map(|t| (t.qualified_hash, *n))
        })
        .collect();
    let mut selected = std::collections::BTreeMap::<&str, Vec<Value>>::new();
    let chunks: Vec<_> = f
        .resource_chunks()
        .iter()
        .map(|c| {
            ensure!(
                c.offset
                    .checked_add(c.compressed_size)
                    .is_some_and(|n| n <= 215052288),
                "compressed resource metadata extent"
            );
            Ok(json!([
                c.offset,
                c.size,
                c.compressed_size,
                c.uncompressed_offset,
                c.uncompressed_size
            ]))
        })
        .collect::<Result<_>>()?;
    let chunks_sha = crate::schema::sha(&serde_json::to_vec(&chunks)?);
    let mut bundle_keys = std::collections::BTreeSet::new();
    for (k, v) in f.resource_bundles().iter() {
        ensure!(bundle_keys.insert(*k), "duplicate bundle semantic key");
        let looked = f
            .resource_bundles()
            .get(k)
            .context("bundle key absent from SDK lookup")?;
        ensure!(
            looked.internal_hash == v.internal_hash
                && looked.index == v.index
                && looked.count == v.count,
            "bundle SDK lookup/key mismatch"
        );
    }
    let mut all = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (id, v) in f.resources().iter() {
        let looked = f
            .resources()
            .get(id)
            .context("resource key absent from SDK lookup")?;
        ensure!(
            looked.offset == v.offset && looked.size == v.size,
            "resource SDK lookup/key mismatch"
        );
        let key = format!("{}/{}/{}", id.type_hash(), id.guid(), id.part_index());
        ensure!(seen.insert(key.clone()), "duplicate resource identity");
        all.push(json!([key, v.offset, v.size]));
    }
    let mut native_order = std::collections::BTreeMap::new();
    for (hash, name) in &hashes {
        let expected: std::collections::BTreeSet<_> = f
            .resources()
            .keys()
            .iter()
            .filter(|id| id.type_hash() == *hash)
            .copied()
            .collect();
        let actual: Vec<_> = f.resources_by_type(*hash).copied().collect();
        let unique: std::collections::BTreeSet<_> = actual.iter().copied().collect();
        ensure!(
            unique == expected && unique.len() == actual.len(),
            "selected SDK typed iterator/key set mismatch"
        );
        let mut order = Vec::new();
        let mut rows = Vec::new();
        for (ordinal, id) in actual.iter().enumerate() {
            let value = f
                .resources()
                .get(id)
                .context("selected SDK ID lookup absent")?;
            order.push(json!([id.guid().to_string(), id.part_index()]));
            rows.push(json!({"guid":id.guid().to_string(),"partIndex":id.part_index(),"resourceSize":value.size,"resourceOffset":value.offset,"nativeSdkOrdinal":ordinal}));
        }
        native_order.insert(*name, crate::schema::sha(&serde_json::to_vec(&order)?));
        selected.insert(*name, rows);
    }
    ensure!(
        seen.len() <= 250000 && selected.values().map(Vec::len).sum::<usize>() <= 150000,
        "resource census limit"
    );
    all.sort_by_key(|v| v[0].as_str().unwrap().to_owned());
    for a in selected.values_mut() {
        a.sort_by_key(|v| {
            (
                v["guid"].as_str().unwrap().to_owned(),
                v["partIndex"].as_u64().unwrap(),
            )
        });
    }
    Ok(
        json!({"gameVersion":f.game_version(),"containerCount":f.containers().len(),"containerPrefixSha256":container,"contentEntryCount":contents.len(),"contentMappingSha256":content,"resourceEntryCount":all.len(),"resourceChunkMetadataSha256":chunks_sha,"resourceChunkCount":chunks.len(),"resourceSizes":"raw SDK table sizes may be24bit truncated; no payload read or uncompressed physical bound claimed","resourceLookupSha256":crate::schema::sha(&serde_json::to_vec(&all)?),"selectedQualifiedResourceIdentities":selected,"selectedNativeSdkOrderSha256":native_order,"lookupKeyAgreementChecked":true,"nativeOrderScope":"current pinned SDK iterator order only; not producer mutable-cache/history proof","resourcePayloadsDecoded":false,"contentPayloadsRead":false,"nativeContentHashesVerified":false,"contentMetadataBoundsMethod":"SDK rawContentHash.size range only; compressed resource chunks use compressed_size separately; no payload proof","originalMetadataOnly":true}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_a_kfc_metadata_roundtrip_and_count_guard() {
        let mut b = fixture();
        assert!(parse(&b).is_ok());
        b[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&b).is_err());
    }
}

#[cfg(test)]
pub(crate) fn fixture() -> Vec<u8> {
    let mut b = vec![0u8; 153];
    b[..4].copy_from_slice(b"KFC4");
    for at in (16..144).step_by(8) {
        b[at..at + 4].copy_from_slice(&((153 - at) as u32).to_le_bytes());
    }
    b[16..20].copy_from_slice(&128u32.to_le_bytes());
    b[20..24].copy_from_slice(&9u32.to_le_bytes());
    b[144..153].copy_from_slice(b"synthetic");
    b
}

#[cfg(test)]
mod lookup_tests {
    use super::*;
    use kfc::container::StaticHash;
    fn location(b: &mut [u8], at: usize, offset: usize, count: u32) {
        b[at..at + 4].copy_from_slice(&((offset - at) as u32).to_le_bytes());
        b[at + 4..at + 8].copy_from_slice(&count.to_le_bytes());
    }
    fn append(b: &mut Vec<u8>, at: usize, count: u32, raw: &[u8]) {
        let n = b.len();
        b.extend(raw);
        location(b, at, n, count);
    }
    fn registry() -> kfc::reflection::TypeRegistry {
        let mut j: Value = serde_json::from_str(include_str!("baseline-fixture.json")).unwrap();
        j["types"][0]["qualifiedName"] = json!("keen::ItemInfo");
        j["types"][0]["qualifiedHash"] = json!(kfc::hash::fnv("keen::ItemInfo"));
        serde_json::from_value(j).unwrap()
    }
    fn content(wrong: bool) -> Vec<u8> {
        let mut b = fixture();
        let mut container = vec![0; 16];
        container[8..16].copy_from_slice(&1u64.to_le_bytes());
        append(&mut b, 24, 1, &container);
        let h = kfc::guid::ContentHash::from_data(b"");
        let bucket = (h.static_hash() % 2) as usize;
        let mut buckets = vec![0; 16];
        let bucket = if wrong { 1 - bucket } else { bucket };
        buckets[bucket * 8 + 4..bucket * 8 + 8].copy_from_slice(&1u32.to_le_bytes());
        append(&mut b, 64, 2, &buckets);
        let mut key = Vec::new();
        for n in [h.size(), h.hash0(), h.hash1(), h.hash2()] {
            key.extend(n.to_le_bytes());
        }
        append(&mut b, 72, 1, &key);
        append(&mut b, 80, 1, &[0; 16]);
        b
    }
    fn resource(missing_from_native_iterator: bool) -> Vec<u8> {
        let mut b = fixture();
        let hash = kfc::hash::fnv("keen::ItemInfo");
        let id = kfc::guid::ResourceId::parse("11111111-1111-1111-1111-111111111111", hash, 207)
            .unwrap();
        let mut key = Vec::new();
        id.write(&mut key).unwrap();
        append(&mut b, 88, 1, &[0, 0, 0, 0, 1, 0, 0, 0]);
        append(&mut b, 96, 1, &key);
        append(&mut b, 104, 1, &[0; 8]);
        append(&mut b, 56, 1, &[0; 4]);
        append(&mut b, 112, 1, &[0, 0, 0, 0, 1, 0, 0, 0]);
        append(&mut b, 120, 1, &hash.to_le_bytes());
        let mut value = vec![0; 12];
        value[8..12].copy_from_slice(
            &(if missing_from_native_iterator {
                0u32
            } else {
                1u32
            })
            .to_le_bytes(),
        );
        append(&mut b, 128, 1, &value);
        b
    }
    #[test]
    fn phase_a_real_sdk_content_lookup_rejects_wrong_bucket_membership() {
        assert!(summary(&parse(&content(false)).unwrap(), &registry()).is_ok());
        assert!(
            summary(&parse(&content(true)).unwrap(), &registry()).is_err(),
            "SDKlookup lost source-visible content key despite accepted lengths/ranges"
        );
    }
    #[test]
    fn phase_a_real_sdk_typed_iterator_must_match_all_selected_native_parts() {
        let p = summary(&parse(&resource(false)).unwrap(), &registry()).unwrap();
        assert_eq!(
            p["selectedQualifiedResourceIdentities"]["keen::ItemInfo"][0]["partIndex"],
            207
        );
        assert!(
            summary(&parse(&resource(true)).unwrap(), &registry()).is_err(),
            "rawkey census was mistaken for nativeSDK enumeration"
        );
    }
    #[test]
    fn phase_a_duplicate_identical_bundle_keys_refuse_without_relaxing_contract() {
        let mut b = resource(false);
        let hash = kfc::hash::fnv("keen::ItemInfo");
        append(&mut b, 112, 1, &[0, 0, 0, 0, 2, 0, 0, 0]);
        let mut keys = Vec::new();
        keys.extend(hash.to_le_bytes());
        keys.extend(hash.to_le_bytes());
        append(&mut b, 120, 2, &keys);
        let mut values = vec![0; 24];
        values[8..12].copy_from_slice(&1u32.to_le_bytes());
        values[20..24].copy_from_slice(&1u32.to_le_bytes());
        append(&mut b, 128, 2, &values);
        assert!(summary(&parse(&b).unwrap(), &registry()).is_err());
    }
}
