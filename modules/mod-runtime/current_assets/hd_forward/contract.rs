//! Canonical wire contract shared with the read-only startup admission module.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub const ORIGIN_CLOSURE: &str = "1f80b068c67c03e47bdde01360e7adc2886c2f69ef27fb40e3d6dfb916d199ba";
pub const HOLD: &str = ".EnshroudedClientMods/hdmap-forward/pending.json";
pub const POLICY: &str = "shroudforge/config/current-assets.json";
pub const HD: &str = "mods/esm-real-map/";
pub const REGISTRY_SHA256: &str =
    "bf14d28554fb1e217abab3ef51d3ae3077bb078a18532e1fb7ed5f1ae7d7ddd8";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub path: PathBuf,
    pub stamp: Stamp,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginRequest {
    pub root: PathBuf,
    pub selected_ids: Vec<String>,
    pub inputs: BTreeMap<String, Stamp>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginReceipt {
    pub schema: u32,
    pub source_closure: String,
    pub request_sha256: String,
    pub root: PathBuf,
    pub staged: BTreeMap<String, Stamp>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Origin {
    pub schema_version: u32,
    pub owner: String,
    pub root: PathBuf,
    pub source_closure: String,
    pub request_json: String,
    pub request_sha256: String,
    pub receipt_json: String,
    pub receipt_sha256: String,
    pub extended_zoom: bool,
    pub preserved_loaders: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub transaction_id: String,
    pub root: PathBuf,
    pub origin_marker_json: String,
    pub origin_marker_sha256: String,
    pub origin_outer_receipt_sha256: String,
    pub payload: Input,
    pub registry: Input,
    pub staging: PathBuf,
    /// Complete HD package membership before and after the outer publication.
    pub hd_files_before: BTreeMap<String, Stamp>,
    pub hd_files_after: BTreeMap<String, Stamp>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema_version: u32,
    pub kind: String,
    pub source_closure: String,
    /// SHA256 of the exact UTF-8 request file, not a second serialization.
    pub request_sha256: String,
    pub root: PathBuf,
    pub staged: BTreeMap<String, Stamp>,
    pub texture_before_sha256: String,
    pub texture_after_sha256: String,
    pub preserved_resource_count: u64,
    pub existing_content_count: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Marker {
    pub schema_version: u32,
    pub owner: String,
    pub root: PathBuf,
    pub source_closure: String,
    pub request_json: String,
    pub request_sha256: String,
    pub receipt_json: String,
    pub receipt_sha256: String,
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn asset(s: &str) -> bool {
    s == "enshrouded.kfc"
        || s == "enshrouded.kfc_resources"
        || s.strip_prefix("enshrouded_")
            .and_then(|s| s.strip_suffix(".dat"))
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
pub fn relative(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(['\\', ':'])
        && s.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with([' ', '.'])
                && !p.chars().any(|c| c.is_control() || "<>\"|?*".contains(c))
        })
}
pub fn same_root(a: &std::path::Path, b: &std::path::Path) -> bool {
    fn key(p: &std::path::Path) -> Option<String> {
        let s = p.to_str()?;
        let s = s.strip_prefix(r"\\?\").unwrap_or(s).replace('\\', "/");
        if !p.is_absolute() || s.split('/').any(|p| matches!(p, "." | "..")) {
            return None;
        }
        Some(s.trim_end_matches('/').to_ascii_lowercase())
    }
    match (key(a), key(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}
pub struct Admitted {
    pub origin: Origin,
    pub request: OriginRequest,
    pub receipt: OriginReceipt,
}
pub fn validate(request: &Request) -> Result<Admitted> {
    ensure!(
        request.schema_version == 1
            && request.root.is_absolute()
            && request.transaction_id.len() == 32
            && request
                .transaction_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid forward request identity"
    );
    ensure!(
        valid_digest(&request.origin_outer_receipt_sha256)
            && digest(request.origin_marker_json.as_bytes()) == request.origin_marker_sha256,
        "origin marker identity differs"
    );
    let origin: Origin = serde_json::from_str(&request.origin_marker_json)?;
    ensure!(
        origin.schema_version == 3
            && origin.owner == "manager-asset-generator"
            && origin.source_closure == ORIGIN_CLOSURE
            && same_root(&origin.root, &request.root)
            && digest(origin.request_json.as_bytes()) == origin.request_sha256
            && digest(origin.receipt_json.as_bytes()) == origin.receipt_sha256,
        "unsupported or changed origin"
    );
    let old_request: OriginRequest = serde_json::from_str(&origin.request_json)?;
    let receipt: OriginReceipt = serde_json::from_str(&origin.receipt_json)?;
    ensure!(
        same_root(&old_request.root, &request.root)
            && same_root(&receipt.root, &request.root)
            && receipt.schema == 1
            && receipt.source_closure == ORIGIN_CLOSURE
            && receipt.request_sha256 == digest(&serde_json::to_vec(&old_request)?),
        "origin generation identity differs"
    );
    let ids: BTreeSet<_> = old_request.selected_ids.iter().collect();
    ensure!(
        ids.len() == old_request.selected_ids.len()
            && ids.contains(&"esm-real-map".to_owned())
            && ids.iter().all(|s| !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))),
        "invalid unchanged selection"
    );
    for (files, limit) in [
        (&old_request.inputs, 10000),
        (&receipt.staged, 512),
        (&request.hd_files_before, 512),
        (&request.hd_files_after, 512),
    ] {
        ensure!(
            !files.is_empty() && files.len() <= limit,
            "invalid file inventory size"
        );
        let mut aliases = BTreeSet::new();
        for (name, stamp) in files {
            ensure!(
                relative(name)
                    && valid_digest(&stamp.sha256)
                    && aliases.insert(name.to_lowercase()),
                "invalid file identity or alias"
            );
        }
    }
    for name in [
        "enshrouded.exe",
        "enshrouded.kfc",
        "enshrouded.kfc_resources",
    ] {
        ensure!(
            old_request.inputs.contains_key(name),
            "missing origin input"
        );
    }
    for name in [
        "enshrouded.kfc",
        "enshrouded.kfc_resources",
        "generated-inputs.json",
    ] {
        ensure!(receipt.staged.contains_key(name), "missing origin output");
    }
    for name in receipt.staged.keys() {
        ensure!(
            asset(name) || name == "generated-inputs.json",
            "foreign origin output"
        );
        ensure!(
            !name.ends_with(".dat") || !old_request.inputs.contains_key(name),
            "origin generated DAT overlaps baseline"
        );
    }
    ensure!(
        origin.preserved_loaders.contains_key("winmm.dll")
            && origin
                .preserved_loaders
                .iter()
                .all(|(n, h)| matches!(n.as_str(), "winmm.dll" | "dbgcore.dll") && valid_digest(h)),
        "unsupported origin loaders"
    );
    ensure!(
        request
            .hd_files_before
            .keys()
            .chain(request.hd_files_after.keys())
            .all(|n| n.starts_with(HD)),
        "non-HD package edit"
    );
    for (name, stamp) in &old_request.inputs {
        if name.starts_with(HD) {
            ensure!(
                request.hd_files_before.get(name) == Some(stamp),
                "HD origin input differs"
            );
        }
    }
    for name in ["extended.mod.json", "src/zoom.lua", "assets/map-shader.bin"] {
        let name = format!("{HD}{name}");
        let before = request
            .hd_files_before
            .get(&name)
            .context("missing preserved HD input")?;
        ensure!(
            request.hd_files_after.get(&name) == Some(before),
            "HD setting, shader or zoom source changed"
        );
    }
    ensure!(
        request
            .hd_files_before
            .contains_key(&format!("{HD}assets/map.rgba"))
            && !request
                .hd_files_before
                .contains_key(&format!("{HD}assets/map.bc7"))
            && !request
                .hd_files_after
                .contains_key(&format!("{HD}assets/map.rgba")),
        "unsupported HD package direction"
    );
    let payload = Stamp {
        bytes: super::texture::PAYLOAD_BYTES,
        sha256: super::texture::PAYLOAD_SHA256.into(),
    };
    ensure!(
        request.payload.path.is_absolute()
            && request.payload.stamp == payload
            && request.hd_files_after.get(&format!("{HD}assets/map.bc7")) == Some(&payload),
        "unqualified BC7 payload"
    );
    ensure!(
        request.registry.path.is_absolute()
            && request.registry.stamp.sha256 == REGISTRY_SHA256
            && request.registry.stamp.bytes <= 16 * 1024 * 1024,
        "unqualified type registry"
    );
    ensure!(
        request.staging.is_absolute() && !same_root(&request.staging, &request.root),
        "invalid output stage"
    );
    Ok(Admitted {
        origin,
        request: old_request,
        receipt,
    })
}
pub fn before_assets(a: &Admitted) -> BTreeMap<String, Stamp> {
    a.request
        .inputs
        .iter()
        .chain(a.receipt.staged.iter())
        .filter(|(n, _)| asset(n))
        .map(|(n, s)| (n.clone(), s.clone()))
        .collect()
}
pub fn validate_receipt(raw: &str, r: &Request, receipt: &Receipt, source: &str) -> Result<()> {
    let admitted = validate(r)?;
    ensure!(
        valid_digest(source)
            && source != ORIGIN_CLOSURE
            && receipt.source_closure == source
            && receipt.schema_version == 1
            && receipt.kind == "hdmap-forward-8k"
            && receipt.request_sha256 == digest(raw.as_bytes())
            && same_root(&receipt.root, &r.root),
        "forward producer or request identity differs"
    );
    ensure!(
        (3..=512).contains(&receipt.staged.len())
            && receipt.preserved_resource_count > 0
            && valid_digest(&receipt.texture_before_sha256)
            && valid_digest(&receipt.texture_after_sha256)
            && receipt.texture_before_sha256 != receipt.texture_after_sha256,
        "incomplete forward receipt"
    );
    let before = before_assets(&admitted);
    for name in [
        "enshrouded.kfc",
        "enshrouded.kfc_resources",
        "generated-inputs.json",
    ] {
        ensure!(receipt.staged.contains_key(name), "missing forward output");
    }
    for (name, stamp) in &receipt.staged {
        ensure!(
            relative(name)
                && valid_digest(&stamp.sha256)
                && (asset(name) || name == "generated-inputs.json"),
            "foreign forward output"
        );
        ensure!(
            !name.ends_with(".dat") || !before.contains_key(name),
            "forward DAT overlaps existing content"
        );
    }
    ensure!(
        receipt.staged["enshrouded.kfc_resources"].bytes
            >= before["enshrouded.kfc_resources"].bytes,
        "forward resource store shrank"
    );
    Ok(())
}
pub fn effective_assets(a: &Admitted, r: &Receipt) -> BTreeMap<String, Stamp> {
    let mut assets = before_assets(a);
    assets.extend(
        r.staged
            .iter()
            .filter(|(n, _)| asset(n))
            .map(|(n, s)| (n.clone(), s.clone())),
    );
    assets
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stamp() -> Stamp {
        Stamp {
            bytes: 1,
            sha256: "a".repeat(64),
        }
    }
    fn fixture() -> Request {
        let root = PathBuf::from("E:/fixture");
        let mut inputs: BTreeMap<_, _> = [
            "enshrouded.exe",
            "enshrouded.kfc",
            "enshrouded.kfc_resources",
            "mods/esm-real-map/extended.mod.json",
            "mods/esm-real-map/src/zoom.lua",
            "mods/esm-real-map/assets/map-shader.bin",
            "mods/esm-real-map/assets/map.rgba",
        ]
        .into_iter()
        .map(|n| (n.into(), stamp()))
        .collect();
        inputs.insert("mods/other/src/mod.lua".into(), stamp());
        let old = OriginRequest {
            root: root.clone(),
            selected_ids: vec!["esm-real-map".into(), "other".into()],
            inputs,
        };
        let raw = serde_json::to_string(&old).unwrap();
        let receipt = OriginReceipt {
            schema: 1,
            source_closure: ORIGIN_CLOSURE.into(),
            request_sha256: digest(raw.as_bytes()),
            root: root.clone(),
            staged: [
                "enshrouded.kfc",
                "enshrouded.kfc_resources",
                "enshrouded_553.dat",
                "generated-inputs.json",
            ]
            .into_iter()
            .map(|n| (n.into(), stamp()))
            .collect(),
        };
        let rr = serde_json::to_string(&receipt).unwrap();
        let origin = Origin {
            schema_version: 3,
            owner: "manager-asset-generator".into(),
            root: root.clone(),
            source_closure: ORIGIN_CLOSURE.into(),
            request_sha256: digest(raw.as_bytes()),
            request_json: raw,
            receipt_sha256: digest(rr.as_bytes()),
            receipt_json: rr,
            extended_zoom: true,
            preserved_loaders: [("winmm.dll".into(), "a".repeat(64))].into(),
        };
        let origin_marker_json = serde_json::to_string(&origin).unwrap();
        let hd_files_before: BTreeMap<_, _> = old
            .inputs
            .iter()
            .filter(|(n, _)| n.starts_with(HD))
            .map(|(n, s)| (n.clone(), s.clone()))
            .collect();
        let mut hd_files_after = hd_files_before.clone();
        hd_files_after.remove("mods/esm-real-map/assets/map.rgba");
        hd_files_after.insert(
            "mods/esm-real-map/assets/map.bc7".into(),
            Stamp {
                bytes: 89_478_512,
                sha256: super::super::texture::PAYLOAD_SHA256.into(),
            },
        );
        Request {
            schema_version: 1,
            transaction_id: "1".repeat(32),
            root,
            origin_marker_sha256: digest(origin_marker_json.as_bytes()),
            origin_marker_json,
            origin_outer_receipt_sha256: "b".repeat(64),
            payload: Input {
                path: "E:/payload.bc7".into(),
                stamp: hd_files_after["mods/esm-real-map/assets/map.bc7"].clone(),
            },
            registry: Input {
                path: "E:/types.json".into(),
                stamp: Stamp {
                    bytes: 1,
                    sha256: REGISTRY_SHA256.into(),
                },
            },
            staging: "E:/output/stage".into(),
            hd_files_before,
            hd_files_after,
        }
    }
    #[test]
    fn origin_and_target_contract_is_accepted_without_rewriting_origin() {
        let r = fixture();
        let raw = r.origin_marker_json.clone();
        assert!(validate(&r).is_ok());
        assert_eq!(r.origin_marker_json, raw);
    }
    #[test]
    fn windows_canonical_receipt_root_matches_the_same_origin_root() {
        let mut r = fixture();
        let mut origin: Origin = serde_json::from_str(&r.origin_marker_json).unwrap();
        let mut receipt: OriginReceipt = serde_json::from_str(&origin.receipt_json).unwrap();
        receipt.root = PathBuf::from(r"\\?\E:\fixture");
        origin.receipt_json = serde_json::to_string(&receipt).unwrap();
        origin.receipt_sha256 = digest(origin.receipt_json.as_bytes());
        r.origin_marker_json = serde_json::to_string(&origin).unwrap();
        r.origin_marker_sha256 = digest(r.origin_marker_json.as_bytes());
        assert!(validate(&r).is_ok());
    }
    #[test]
    fn tampered_origin_and_non_hd_or_preference_edits_are_refused() {
        for mode in 0..5 {
            let mut r = fixture();
            match mode {
                0 => r.origin_marker_sha256 = "c".repeat(64),
                1 => {
                    r.hd_files_after
                        .insert("mods/other/src/mod.lua".into(), stamp());
                }
                2 => {
                    r.hd_files_after
                        .get_mut("mods/esm-real-map/extended.mod.json")
                        .unwrap()
                        .sha256 = "d".repeat(64);
                }
                3 => r.payload.stamp.bytes = 12,
                _ => {
                    r.hd_files_after
                        .insert("mods/esm-real-map/../other/a".into(), stamp());
                }
            }
            assert!(validate(&r).is_err(), "unsafe request accepted: {mode}");
        }
    }
    #[test]
    fn new_receipt_rejects_false_producer_replaced_old_dat_and_unbound_request() {
        let r = fixture();
        let raw = serde_json::to_string(&r).unwrap();
        let base = Receipt {
            schema_version: 1,
            kind: "hdmap-forward-8k".into(),
            source_closure: "c".repeat(64),
            request_sha256: digest(raw.as_bytes()),
            root: r.root.clone(),
            staged: [
                "enshrouded.kfc",
                "enshrouded.kfc_resources",
                "enshrouded_554.dat",
                "generated-inputs.json",
            ]
            .into_iter()
            .map(|n| (n.into(), stamp()))
            .collect(),
            texture_before_sha256: "d".repeat(64),
            texture_after_sha256: "e".repeat(64),
            preserved_resource_count: 10,
            existing_content_count: 5,
        };
        assert!(validate_receipt(&raw, &r, &base, &"c".repeat(64)).is_ok());
        for mode in 0..4 {
            let mut bad = base.clone();
            match mode {
                0 => bad.source_closure = ORIGIN_CLOSURE.into(),
                1 => bad.request_sha256 = "f".repeat(64),
                2 => {
                    bad.staged.insert("enshrouded_553.dat".into(), stamp());
                }
                _ => {
                    bad.staged.insert("mods/other/mod.json".into(), stamp());
                }
            };
            assert!(
                validate_receipt(&raw, &r, &bad, &"c".repeat(64)).is_err(),
                "bad receipt accepted {mode}"
            );
        }
    }
}
