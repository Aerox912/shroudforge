//! Package planning only. No asset runtime, game input or native mod execution.
use anyhow::{Context, Result, ensure};
use prepared_source_admission::{SourcePlan, SourceLease, ReadLimits, ProfilePolicy, Stamp};
use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};

fn entries(root: &Path, path: &Path, out: &mut Vec<String>) -> Result<()> {
    for item in fs::read_dir(path)? {
        let path = item?.path(); let metadata = fs::symlink_metadata(&path)?;
        ensure!(!metadata.file_type().is_symlink(), "linked member");
        #[cfg(windows)] {
            use std::os::windows::fs::MetadataExt;
            ensure!(metadata.file_attributes() & 0x400 == 0, "reparse member");
        }
        ensure!(out.len() < 1000, "membership bound");
        out.push(path.strip_prefix(root)?.to_str().context("UTF8")?.replace('\\', "/"));
        if metadata.is_dir() { entries(root, &path, out)?; }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(args.len() == 3, "usage: recipe-check RECIPE.json REPORT.json");
    let receipt: serde_json::Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    ensure!(receipt["status"] == "PORTED_SERVER_RECIPE_PREPARED_NOT_QUALIFIED", "recipe kind");
    let root = Path::new(receipt["environmentRoot"].as_str().context("root")?);
    ensure!(root.to_str().is_some_and(|s| s.starts_with("E:\\Build\\ikea-server-ported-recipe-")), "private recipe root");
    let files: BTreeMap<String, Stamp> = receipt["effectiveFiles"].as_object().context("files")?.iter()
        .map(|(name,pin)| Ok((name.clone(), Stamp {
            bytes: pin["bytes"].as_u64().context("bytes")?,
            sha256: pin["sha256"].as_str().context("sha")?.into(),
        }))).collect::<Result<_>>()?;
    let mut membership = vec![String::new()]; entries(root, root, &mut membership)?;
    let packages: BTreeSet<_> = files.keys().map(|n| n.split('/').take(2).collect::<Vec<_>>().join("/")).collect();
    ensure!(packages.len() == 13 && files.len() == 63, "recipe membership");
    let mut plan = SourcePlan { schema:1, kind:"prepared_effective_source_plan_v1".into(),
        environment_root:root.into(), raw_files:files, entries:membership,
        package_roots:packages.into_iter().collect(), replacements:vec![],
        profile_policy:ProfilePolicy::RejectAmbientProfiles, allow_unknown_manifest_fields:false,
        raw_inventory_sha256:String::new(), effective_inventory_sha256:String::new() };
    plan.raw_inventory_sha256 = plan.inventory_digest(false)?;
    plan.effective_inventory_sha256 = plan.inventory_digest(true)?;
    let limits = ReadLimits { max_bytes:32*1024*1024, max_calls:2000,
        final_reserve_bytes:plan.required_final_bytes()?, final_reserve_calls:plan.required_final_calls()? };
    let mut lease = SourceLease::admit(plan.clone(), limits)?;
    let env = shroudforge_package::ModEnvironment::from_prepared_packages(
        root.to_str().unwrap().into(), root.join("shroudforge/cache").to_str().unwrap().into(),
        root.join("mods").to_str().unwrap().into(), lease.package_views()?, lease.manifest_policy())
        .map_err(|e| anyhow::anyhow!("package admission: {e:?}"))?;
    // API version comes from workspace.package.version, not the release tag.
    let (mods, issues) = env.plan_report(true, "1.0.0");
    let ids: Vec<_> = mods.iter().filter(|m|m.info().requires_pregame()).map(|m|m.info().id.clone()).collect();
    let finalization = lease.finalize()?;
    let passed = issues.is_empty() && ids.len() == 13 && finalization.verified;
    let report = serde_json::json!({"status":if passed{"SERVER_PACKAGE_PLAN_ACCEPTED_ONLY"}else{"SERVER_PACKAGE_PLAN_REFUSED"},
        "plannedIds":ids,"issues":issues,"sourcePlan":plan,"sourceFinalization":finalization,
        "gameInputsRead":false,"luaExecuted":false,"assetCompatibilityAccepted":false,"liveChanges":false});
    let mut output = fs::OpenOptions::new().write(true).create_new(true).open(&args[2])?;
    serde_json::to_writer_pretty(&mut output, &report)?;
    ensure!(passed, "server plan failed; inspect report");
    println!("13 server asset packages planned; held source finalization passed; no game inputs read");
    Ok(())
}
