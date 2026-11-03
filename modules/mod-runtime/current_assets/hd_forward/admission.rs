//! Read-only physical admission shared verbatim with startup.
use super::{contract::*, files};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub enum Package {
    Before,
    Mixed,
    After,
}
pub fn defaults(root: &Path) -> Result<()> {
    let path = files::checked(root, "shroudforge/config/modloader-config.json")?;
    let v: serde_json::Value = if path.try_exists()? {
        serde_json::from_str(&files::text(&path)?)?
    } else {
        serde_json::json!({})
    };
    for (key, name) in [
        ("mods", "mods"),
        ("cache", "shroudforge/cache"),
        ("runtime", "shroudforge/runtime"),
    ] {
        let want = files::checked(root, name)?;
        let actual = match v.pointer(&format!("/paths/{key}")) {
            None => want.clone(),
            Some(x) => {
                let p = PathBuf::from(x.as_str().context("invalid loader path")?);
                if p.is_absolute() { p } else { root.join(p) }
            }
        };
        ensure!(
            same_root(&actual, &want),
            "alternate loader path not qualified"
        );
    }
    Ok(())
}
pub fn no_hold(root: &Path) -> Result<()> {
    let p = files::checked(root, HOLD)?;
    ensure!(
        !p.try_exists()?,
        "HD forward publication is pending; finish Manager recovery before game launch"
    );
    Ok(())
}
pub fn inputs(r: &Request, a: &Admitted, mode: Package) -> Result<PathBuf> {
    let root = files::absolute(&r.root)?;
    ensure!(same_root(&root, &r.root), "canonical game root differs");
    defaults(&root)?;
    for name in [
        "shroudforge/config/current-assets-state.json",
        "shroudforge/config/current-assets-pending.json",
    ] {
        ensure!(
            !files::checked(&root, name)?.try_exists()?,
            "conflicting current-assets owner"
        );
    }
    for (name, s) in &a.request.inputs {
        if !asset(name) && !name.starts_with(HD) {
            files::exact(&root, name, s)?;
        }
    }
    for (name, hash) in &a.origin.preserved_loaders {
        ensure!(
            files::stamp(&files::checked(&root, name)?)?.sha256 == *hash,
            "preserved loader differs"
        );
    }
    let actual = files::hd_members(&root)?;
    match mode {
        Package::Before => ensure!(actual == r.hd_files_before, "HD before package differs"),
        Package::After => ensure!(actual == r.hd_files_after, "HD after package differs"),
        Package::Mixed => {
            let names: BTreeSet<_> = r
                .hd_files_before
                .keys()
                .chain(r.hd_files_after.keys())
                .chain(actual.keys())
                .collect();
            for n in names {
                ensure!(
                    actual.get(n) == r.hd_files_before.get(n)
                        || actual.get(n) == r.hd_files_after.get(n),
                    "HD package drift: {n}"
                );
            }
        }
    }
    let extension: serde_json::Value = serde_json::from_str(&files::text(&files::checked(
        &root,
        "mods/esm-real-map/extended.mod.json",
    )?)?)?;
    let zoom = extension
        .pointer("/settings/ExtendedZoom/value")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| {
            extension
                .pointer("/settings/ExtendedZoom")
                .and_then(serde_json::Value::as_bool)
        });
    ensure!(
        extension["enabled"] == true && zoom == Some(a.origin.extended_zoom),
        "HD raw preference/selection differs"
    );
    Ok(root)
}
pub fn assets(root: &Path, a: &Admitted, expected: &BTreeMap<String, Stamp>) -> Result<()> {
    ensure!(
        files::membership(root)?.keys().eq(expected.keys()),
        "asset membership differs"
    );
    for (name, s) in expected {
        if name.ends_with(".dat") && a.request.inputs.contains_key(name) {
            // Match established schema3 startup policy for immutable official DATs.
            let p = files::checked(root, name)?;
            files::ordinary(&p, false)?;
            ensure!(
                fs::metadata(p)?.len() == s.bytes,
                "baseline DAT size differs"
            );
        } else {
            files::exact(root, name, s)?;
        }
    }
    Ok(())
}
pub fn marker(raw: &str, r: &Request, receipt: &Receipt, source: &str) -> Result<()> {
    let m: Marker = serde_json::from_str(raw)?;
    ensure!(
        m.schema_version == 4
            && m.owner == "manager-hdmap-forward"
            && m.source_closure == source
            && same_root(&m.root, &r.root)
            && m.request_sha256 == digest(m.request_json.as_bytes())
            && m.receipt_sha256 == digest(m.receipt_json.as_bytes()),
        "forward marker differs"
    );
    let embedded: Request = serde_json::from_str(&m.request_json)?;
    ensure!(
        serde_json::to_value(&embedded)? == serde_json::to_value(r)?,
        "marker request differs"
    );
    let proof: Receipt = serde_json::from_str(&m.receipt_json)?;
    ensure!(proof == *receipt, "marker receipt differs");
    validate_receipt(&m.request_json, r, &proof, source)
}
