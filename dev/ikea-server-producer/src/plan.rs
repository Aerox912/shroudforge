//! Build the exact request from staged mod/evidence files. No game asset input.
use crate::prepare::{PRIVATE, Request, directory_chain, write_new};
use anyhow::{Context, Result, ensure};
use prepared_source_admission::{ProfilePolicy, ReadLimits, SourceLease, SourcePlan, Stamp};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

fn membership(root: &Path, path: &Path, out: &mut Vec<String>, depth: usize) -> Result<()> {
    ensure!(depth <= 16, "package traversal depth");
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        let m = fs::symlink_metadata(&path)?;
        ensure!(!m.file_type().is_symlink(), "linked package entry");
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(m.file_attributes() & 0x400 == 0, "reparse package entry");
        }
        ensure!(m.is_dir() || m.is_file(), "package entry kind");
        ensure!(out.len() < 1000, "package membership bound");
        out.push(
            path.strip_prefix(root)?
                .to_str()
                .context("package name UTF8")?
                .replace('\\', "/"),
        );
        if m.is_dir() {
            membership(root, &path, out, depth + 1)?;
        }
    }
    Ok(())
}

pub fn source_plan(root: &Path, provenance: &Value) -> Result<(SourcePlan, ReadLimits)> {
    let files: BTreeMap<String, Stamp> = provenance["recipeFiles"]
        .as_object()
        .context("recipe files")?
        .iter()
        .map(|(name, p)| {
            Ok((
                name.clone(),
                Stamp {
                    bytes: p["bytes"].as_u64().context("recipe bytes")?,
                    sha256: p["sha256"].as_str().context("recipe hash")?.into(),
                },
            ))
        })
        .collect::<Result<_>>()?;
    ensure!(
        files.len() == 63 && files.keys().all(|n| n.starts_with("mods/")),
        "exact package file set"
    );
    let packages: BTreeSet<_> = files
        .keys()
        .map(|n| n.split('/').take(2).collect::<Vec<_>>().join("/"))
        .collect();
    ensure!(packages.len() == 13, "package count");
    let mut entries = vec![String::new()];
    membership(root, root, &mut entries, 0)?;
    let mut plan = SourcePlan {
        schema: 1,
        kind: "prepared_effective_source_plan_v1".into(),
        environment_root: root.into(),
        raw_files: files,
        entries,
        package_roots: packages.into_iter().collect(),
        replacements: vec![],
        profile_policy: ProfilePolicy::RejectAmbientProfiles,
        allow_unknown_manifest_fields: false,
        raw_inventory_sha256: String::new(),
        effective_inventory_sha256: String::new(),
    };
    plan.raw_inventory_sha256 = plan.inventory_digest(false)?;
    plan.effective_inventory_sha256 = plan.inventory_digest(true)?;
    let limits = ReadLimits {
        max_bytes: 32 * 1024 * 1024,
        max_calls: 2000,
        final_reserve_bytes: plan.required_final_bytes()?,
        final_reserve_calls: plan.required_final_calls()?,
    };
    Ok((plan, limits))
}

pub fn staged() -> Result<Value> {
    let root = PathBuf::from(PRIVATE);
    let environment = root.join("environment");
    let evidence = root.join("evidence");
    let mut directories = directory_chain(&environment)?;
    directories.extend(directory_chain(&evidence)?);
    directories.extend(directory_chain(&root.join("controls"))?);
    let pin = |path: &str, bytes, sha256: &str| shroudforge_api::EvidencePin {
        path: path.into(),
        bytes,
        sha256: sha256.into(),
    };
    let input = shroudforge_api::ServerRegistryInput {
        manifest: pin(
            "MANIFEST.json",
            802,
            "20f9e014ab4ad2f220134a6d59d40537c9620152b537c397b29b38957bc9fecd",
        ),
        canonical_review: pin(
            "CANONICAL-REVIEW.json",
            3896,
            "673f8cf8178e6cc35c3bc2ee2e8d478cae119e2119ca695cad1c6039d4fe133c",
        ),
        main_acceptance: pin(
            "MAIN-ACCEPTANCE.json",
            743,
            "08fb8baf49cc2033a214daa39bca0f35e1c8a6bc753f993f362fe1b51981efe5",
        ),
    };
    let registry = shroudforge_api::admit_ikea_server_registry(&evidence, &input)?;
    let (sources, limits) = source_plan(&environment, registry.provenance())?;
    let mut lease = SourceLease::admit_ikea_server(sources.clone(), limits)?;
    let env = shroudforge_package::ModEnvironment::from_prepared_packages(
        environment.to_str().unwrap().into(),
        environment
            .join("shroudforge/cache")
            .to_str()
            .unwrap()
            .into(),
        environment.join("mods").to_str().unwrap().into(),
        lease.package_views()?,
        lease.manifest_policy(),
    )
    .map_err(|e| anyhow::anyhow!("staged package environment: {e:?}"))?;
    let (mods, issues) = env.plan_report(true, shroudforge_api::API_VERSION);
    let ids: Vec<_> = mods
        .iter()
        .filter(|m| m.info().requires_pregame())
        .map(|m| m.info().id.clone())
        .collect();
    let finalization = lease.finalize()?;
    ensure!(
        issues.is_empty()
            && serde_json::to_value(ids)? == registry.provenance()["plannedIds"]
            && finalization.verified,
        "staged package plan differs"
    );
    let request = Request {
        schema: 1,
        kind: "ikea_private_server_preparation_v1".into(),
        source_root: PathBuf::from("D:/GameServers/staging/ikea-game-update-20261105-r1/official"),
        evidence_root: evidence,
        output_root: root.join("generated"),
        operation_root: root.join("operation-r1"),
        sources,
        source_read_limits: limits,
        registry: input,
    };
    write_new(
        &root.join("controls/REQUEST.json"),
        &serde_json::to_value(request)?,
    )?;
    let report = json!({"status":"SERVER_PREPARATION_REQUEST_STAGED_NOT_SELECTED","sourceFinalization":finalization,"originalGameReads":0,"luaExecuted":false,"liveChanges":false});
    write_new(&root.join("controls/PLAN-RESULT.json"), &report)?;
    Ok(report)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn exact_retained_recipe_projects_and_finalizes_without_game_inputs() {
        let raw = fs::read("E:/Build/ikea-server-admission-tests-20261105-r3/schema.stdout.json")
            .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&raw)),
            "e08017b6a6a0da98e92e1df2622a0a309acc687b1351d8bae06bdcbb4134158e"
        );
        let provenance: Value = serde_json::from_slice(&raw).unwrap();
        let root = Path::new("E:/Build/ikea-server-ported-recipe-20261105-r2/environment");
        let (plan, limits) = source_plan(root, &provenance).unwrap();
        assert_eq!(plan.raw_files.len(), 63);
        assert_eq!(plan.package_roots.len(), 13);
        let mut lease = SourceLease::admit_ikea_server(plan, limits).unwrap();
        let env = shroudforge_package::ModEnvironment::from_prepared_packages(
            root.to_str().unwrap().into(),
            root.join("shroudforge/cache").to_str().unwrap().into(),
            root.join("mods").to_str().unwrap().into(),
            lease.package_views().unwrap(),
            lease.manifest_policy(),
        )
        .unwrap();
        let (mods, issues) = env.plan_report(true, shroudforge_api::API_VERSION);
        assert!(issues.is_empty());
        let ids: Vec<_> = mods
            .iter()
            .filter(|m| m.info().requires_pregame())
            .map(|m| m.info().id.clone())
            .collect();
        assert_eq!(serde_json::to_value(ids).unwrap(), provenance["plannedIds"]);
        let controls = shroudforge_api::PreparedControls::from_admitted_bytes(Some(
            br#"{"schemaVersion":1,"modules":{"modDetection":{"enabled":false}}}"#,
        ))
        .unwrap();
        assert_eq!(controls.manifest_policy(), lease.manifest_policy());
        assert!(lease.finalize().unwrap().verified);
    }
}
