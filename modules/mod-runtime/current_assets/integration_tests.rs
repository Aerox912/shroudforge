//! Real startup/codec/settings/WAL tests in a synthetic private asset composition.
//! Reuses the retained synthetic typed fixture as read-only input. No live
//! installation reads or writes; all generated evidence belongs to R2.
use super::super::*;
use kfc::{
    reflection::{LookupKey, TypeRegistry},
    resource::value::Value,
};

const BUILD: &str = "E:/Build/hdmap-zoom-startup-20261029-r2";
const PACKAGES: &str = "E:/Build/hdmap-zoom-startup-20261029-r1";
const SEED: &str = "E:/Scratch/hdmap-zoom-startup-20261029-r1-60340";
const OLD: &str = "E:/Build/hdmap-r9-main-startup-compatibility-20261028-r1";

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let out = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &out)?;
        } else {
            fs::copy(e.path(), out)?;
        }
    }
    Ok(())
}
fn resource(root: &Path, id: &ResourceId) -> Result<Vec<u8>> {
    Ok(KFCReader::new(root, STEM)?
        .into_cursor()?
        .read_resource(id)?
        .context("resource absent")?
        .to_vec())
}
fn native_fixture(root: &Path) -> Result<(serde_json::Value, Value)> {
    ensure!(
        root.to_string_lossy()
            .starts_with("E:/Scratch/hdmap-zoom-startup-20261029-r2-")
    );
    ensure!(!root.exists());
    fs::create_dir_all(root)?;
    let m: serde_json::Value = serde_json::from_slice(&fs::read(
        "E:/Build/esm-realmap-startup-r9-20261028/manifest.json",
    )?)?;
    let input = |key: &str| -> Result<Vec<u8>> {
        let row = &m["files"][key];
        let b = fs::read(row["path"].as_str().unwrap())?;
        ensure!(
            hash(&b) == row["sha256"].as_str().unwrap(),
            "input drift {key}"
        );
        Ok(b)
    };
    let types = input("registry")?;
    let registry: TypeRegistry = serde_json::from_slice(&types)?;
    let seed = Path::new(SEED);
    ensure!(stamp(&seed.join(STATE))?.1 == "3d777887368db430bf8b2c42f6b558475e2a6e53a364c69838d506223c3bb52a");
    let terminal: Prepared = read_json(&seed.join(STATE))?;
    verify_outputs(&fs::canonicalize(seed)?, &terminal)?;
    ensure!(terminal.enabled && terminal.effective_zoom == Some(true));
    let root_id = ResourceId::parse("0942dd71-2a74-4511-a853-349c5eed42a0", 3483826245, 0).unwrap();
    let root_bytes = resource(seed, &root_id)?;
    ensure!(
        hash(&root_bytes) == "53cb974ce0d79c666e83412ce49ab0e7d71ec6673404daffd58b6b27b760a69e"
    );
    let bundle_bytes = resource(seed, &zoom::id())?;
    let meta = registry
        .get_by_name(LookupKey::Qualified("keen::FbUiBundle"))
        .unwrap();
    let retained = Value::from_bytes(&registry, meta, &bundle_bytes)?;
    // The retained fixture already has the non-native isoline canary. Only
    // the two selected scales are inverted in memory to seed native tests.
    let synthetic = zoom::transform(&retained, true, false)?;
    let synthetic_bytes = synthetic.to_bytes(&registry, meta)?;
    ensure!(Value::from_bytes(&registry, meta, &synthetic_bytes)? == synthetic);
    drop(bundle_bytes);
    fs::File::create_new(root.join("enshrouded.kfc"))?;
    fs::File::create_new(root.join("enshrouded.kfc_resources"))?;
    let mut w = KFCWriter::<KFCFile, _>::new(root, STEM, &registry, zoom::GENERATION)?;
    let policy: serde_json::Value = serde_json::from_slice(&fs::read(
        Path::new(OLD).join("transition-proof-r1/main-current-assets-2k.json"),
    )?)?;
    for r in policy["resources"].as_array().unwrap() {
        w.write_resource(
            &ResourceId::parse(
                r["id"].as_str().unwrap(),
                r["typeHash"].as_u64().unwrap() as u32,
                0,
            )
            .unwrap(),
            &input(r["original"].as_str().unwrap())?,
        )?;
    }
    w.write_resource(&root_id, &root_bytes)?;
    w.write_resource(&zoom::id(), &synthetic_bytes)?;
    let unrelated =
        ResourceId::parse("11111111-1111-4111-8111-111111111111", 0xf9e86997, 0).unwrap();
    w.write_resource(&unrelated, &input("texture_resource")?)?;
    for key in ["gradient", "original_code"] {
        let b = input(key)?;
        w.write_content(&ContentHash::from_data(&b), &b)?;
    }
    w.write_content(
        &ContentHash::from_data(b"unrelated content canary"),
        b"unrelated content canary",
    )?;
    w.finalize()?;
    fs::write(root.join("enshrouded.exe"), input("executable")?)?;
    shroudforge_loader_config::config::initialize_loader_config(root)
        .map_err(anyhow::Error::msg)?;
    let cache = root.join("shroudforge/cache");
    fs::create_dir_all(&cache)?;
    fs::write(cache.join("types-enshrouded.json"), &types)?;
    fs::write(
        cache.join("types-enshrouded.identity.json"),
        serde_json::to_vec(
            &serde_json::json!({"schemaVersion":2,"executableSha256":m["files"]["executable"]["sha256"],"reflectionExtractor":"kfc-a404b15cde40fc7bd43d812bb460ce40d2a9d3b1"}),
        )?,
    )?;
    let recipe = root.join("shroudforge/current-assets");
    fs::create_dir_all(&recipe)?;
    for key in ["texture_resource", "gpc", "gradient", "original_code"] {
        fs::write(recipe.join(key), input(key)?)?;
    }
    copy_tree(
        &Path::new(PACKAGES).join("legacy-package"),
        &root.join("mods/esm-real-map"),
    )?;
    let mut policy = policy;
    for name in ["winmm.dll", "shroudtopia.dll", "dbgcore.dll", "dinput8.dll"] {
        let b = format!("synthetic loader pin {name}");
        fs::write(root.join(name), b.as_bytes())?;
        policy["preservedLoaders"][name] = hash(b.as_bytes()).into();
    }
    fs::write(root.join(POLICY), serde_json::to_vec(&policy)?)?;
    Ok((policy, synthetic))
}
fn select(root: &Path, parent: bool, configured: bool) -> Result<()> {
    let manifest = mod_loader::config::read_manifest_path(root, &root.join("mods/esm-real-map"))
        .map_err(anyhow::Error::msg)?;
    let rev = mod_loader::config_revision(
        &serde_json::json!({"enabled":manifest.enabled,"settings":manifest.setting_values}),
    );
    mod_loader::save_mod_settings(
        root,
        "esm-real-map",
        &serde_json::json!({"ExtendedZoom":configured}),
        &rev,
    )
    .map_err(anyhow::Error::msg)?;
    let rev = mod_loader::config_revision(
        &serde_json::json!({"enabled":manifest.enabled,"settings":{"ExtendedZoom":configured}}),
    );
    mod_loader::set_mod_enabled(root, "esm-real-map", parent, &rev).map_err(anyhow::Error::msg)?;
    Ok(())
}
fn verify(root: &Path, native: &Value, parent: bool, configured: bool) -> Result<()> {
    let state: Prepared = read_json(&root.join(STATE))?;
    ensure!(
        state.schema_version == 2
            && state.enabled == parent
            && state.configured_extended_zoom == Some(configured)
            && state.effective_zoom == Some(parent && configured)
    );
    let policy: Policy = read_json(&root.join(POLICY))?;
    verify_outputs(&fs::canonicalize(root)?, &state)?;
    verify_records(root, &policy, parent)?;
    let reg = zoom::registry(
        root,
        policy.zoom.as_ref().unwrap(),
        &policy.executable_sha256,
    )?;
    let meta = reg
        .get_by_name(LookupKey::Qualified("keen::FbUiBundle"))
        .unwrap();
    let actual = Value::from_bytes(&reg, meta, &resource(root, &zoom::id())?)?;
    ensure!(
        actual == zoom::transform(native, false, parent && configured)?,
        "full tree preservation failed"
    );
    let c = KFCReader::new(root, STEM)?
        .into_cursor()?
        .read_content(&ContentHash::from_data(b"unrelated content canary"))?
        .unwrap();
    ensure!(c == b"unrelated content canary");
    Ok(())
}

#[test]
fn actual_schema_migration_settings_lua_codec_all_transitions_and_guards() -> Result<()> {
    let root = PathBuf::from(format!(
        "E:/Scratch/hdmap-zoom-startup-20261029-r2-transitions-{}",
        std::process::id()
    ));
    let (mut policy, native) = native_fixture(&root)?;
    // Produce the old schema using the actual accepted recipe/runtime path.
    let ext = root.join("mods/esm-real-map/extended.mod.json");
    let mut old: serde_json::Value = read_json(&ext)?;
    old["enabled"] = true.into();
    fs::write(&ext, serde_json::to_vec(&old)?)?;
    assert!(prepare(&root)?);
    let old_state: Prepared = read_json(&root.join(STATE))?;
    ensure!(old_state.schema_version == 1 && old_state.enabled);
    let legacy_hash = stamp(&root.join(STATE))?.1;
    copy_tree(
        &Path::new(PACKAGES).join("new-package"),
        &root.join("mods/esm-real-map"),
    )?;
    select(&root, true, true)?;
    policy["schemaVersion"] = 2.into();
    policy["zoom"] = serde_json::json!({"registrySha256":"bf14d28554fb1e217abab3ef51d3ae3077bb078a18532e1fb7ed5f1ae7d7ddd8"});
    policy["migration"] =
        serde_json::json!({"stateSha256":legacy_hash,"fingerprint":old_state.fingerprint});
    fs::write(root.join(POLICY), serde_json::to_vec(&policy)?)?;
    let assets_before = (
        stamp(&root.join("enshrouded.kfc"))?,
        stamp(&root.join("enshrouded.kfc_resources"))?,
    );
    // Wrong migration terminal refuses before changing outputs.
    let mut bad = policy.clone();
    bad["migration"]["stateSha256"] = "0".repeat(64).into();
    fs::write(root.join(POLICY), serde_json::to_vec(&bad)?)?;
    assert!(prepare(&root).is_err());
    assert_eq!(
        assets_before,
        (
            stamp(&root.join("enshrouded.kfc"))?,
            stamp(&root.join("enshrouded.kfc_resources"))?
        )
    );
    fs::write(root.join(POLICY), serde_json::to_vec(&policy)?)?;
    assert!(prepare(&root)?);
    verify(&root, &native, true, true)?;
    let counts = KFCFile::from_path(root.join("enshrouded.kfc"), false)?
        .containers()
        .len();
    for a in [(false, false), (false, true), (true, false), (true, true)] {
        for b in [(false, false), (false, true), (true, false), (true, true)] {
            select(&root, a.0, a.1)?;
            assert!(prepare(&root)?);
            verify(&root, &native, a.0, a.1)?;
            select(&root, b.0, b.1)?;
            assert!(prepare(&root)?);
            verify(&root, &native, b.0, b.1)?;
            let state = fs::read(root.join(STATE))?;
            assert!(prepare(&root)?);
            assert_eq!(
                fs::read(root.join(STATE))?,
                state,
                "no-op receipt rewritten"
            );
        }
    }
    assert_eq!(
        KFCFile::from_path(root.join("enshrouded.kfc"), false)?
            .containers()
            .len(),
        counts,
        "zoom allocated content"
    );
    // Existing persistence rejects invalid values/stale revision without changing extension.
    let saved = fs::read(&ext)?;
    assert!(
        mod_loader::save_mod_settings(
            &root,
            "esm-real-map",
            &serde_json::json!({"ExtendedZoom":"true"}),
            "invalid"
        )
        .is_err()
    );
    assert_eq!(fs::read(&ext)?, saved);
    for (label, mutated) in [
        ("metadata", {
            let mut v: serde_json::Value = serde_json::from_slice(&saved)?;
            v["settings"]["ExtendedZoom"]["label"] = "changed".into();
            v
        }),
        ("unknown-setting", {
            let mut v: serde_json::Value = serde_json::from_slice(&saved)?;
            v["settings"]["Other"] = true.into();
            v
        }),
    ] {
        fs::write(&ext, serde_json::to_vec(&mutated)?)?;
        assert!(prepare(&root).is_err(), "guard {label}");
        fs::write(&ext, &saved)?;
    }
    verify(&root, &native, true, true)?;
    // Native pin/code drift remains a hard refusal with assets and state untouched.
    let state = fs::read(root.join(STATE))?;
    for path in [
        root.join("winmm.dll"),
        root.join("mods/esm-real-map/src/zoom.lua"),
    ] {
        let saved = fs::read(&path)?;
        fs::write(&path, b"drift")?;
        assert!(prepare(&root).is_err());
        fs::write(&path, saved)?;
    }
    assert_eq!(fs::read(root.join(STATE))?, state);
    println!(
        "PASS schema1->2, actual Lua+typed codec and saved settings, all16 transitions/repeats, full canary tree, fixed content inventory, drift refusal; fixture={}",
        root.display()
    );
    fs::write(
        Path::new(BUILD).join("fixture-path.txt"),
        root.to_string_lossy().as_bytes(),
    )?;
    Ok(())
}

#[test]
fn final_feature_codec_and_persisted_selection() -> Result<()> {
    let root=PathBuf::from(format!("E:/Scratch/hdmap-zoom-startup-20261029-r2-feature-{}",std::process::id()));
    let (mut policy,native)=native_fixture(&root)?;
    assert!(prepare(&root)?); // Actual schema1 native/disabled terminal.
    let old:Prepared=read_json(&root.join(STATE))?;
    ensure!(old.schema_version==1&&!old.enabled);
    let legacy_hash=stamp(&root.join(STATE))?.1;
    copy_tree(&Path::new(PACKAGES).join("new-package"),&root.join("mods/esm-real-map"))?;
    let manifest=mod_loader::config::read_manifest_path(&root,&root.join("mods/esm-real-map")).map_err(anyhow::Error::msg)?;
    ensure!(!manifest.enabled&&manifest.setting_values["ExtendedZoom"]==true,"archive defaults changed");
    policy["schemaVersion"]=2.into();
    policy["zoom"]=serde_json::json!({"registrySha256":"bf14d28554fb1e217abab3ef51d3ae3077bb078a18532e1fb7ed5f1ae7d7ddd8"});
    policy["migration"]=serde_json::json!({"stateSha256":legacy_hash,"fingerprint":old.fingerprint});
    fs::write(root.join(POLICY),serde_json::to_vec(&policy)?)?;
    // Both actual Lua selection branches after the owner's validation-order fix.
    select(&root,true,false)?;assert!(prepare(&root)?);verify(&root,&native,true,false)?;
    select(&root,false,true)?;assert!(prepare(&root)?);verify(&root,&native,false,true)?;
    select(&root,true,true)?;assert!(prepare(&root)?);verify(&root,&native,true,true)?;
    let receipt=fs::read(root.join(STATE))?;assert!(prepare(&root)?);assert_eq!(fs::read(root.join(STATE))?,receipt);
    fs::write(Path::new(BUILD).join("fixture-path.txt"),root.to_string_lossy().as_bytes())?;
    println!("PASS final frozen feature real Lua native/extended branches, full typed canary preservation, persisted false/true and default true; fixture={}",root.display());
    Ok(())
}
