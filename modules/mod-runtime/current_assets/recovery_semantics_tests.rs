//! ZOOM-RECOVERY-01: real typed pending postimages, private fixtures only.
use super::*;
use std::time::{SystemTime,UNIX_EPOCH};

const SEED: &str = "E:/Scratch/hdmap-zoom-startup-20261029-r1-60340";
const EVIDENCE: &str = "E:/Build/hdmap-zoom-startup-20261029-r2";

// Produce a real mixed-path transaction for the separately built DLL export.
// This fixture is never loaded by the game and contains synthetic loader pins.
#[test]
fn prepare_exact_dll_recovery_fixture() -> Result<()> {
    let nonce=SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root=PathBuf::from(format!("E:/Scratch/hdmap-zoom-dll-recovery-{}-{nonce}",std::process::id()));
    copy_tree(Path::new(SEED),&root)?;
    let root=fs::canonicalize(root)?;
    let config=root.join("shroudforge/config/modloader-config.json");
    let mut loader:serde_json::Value=read_json(&config)?;
    for (key,path) in [("mods","mods"),("cache","shroudforge/cache"),("runtime","shroudforge/runtime"),("state","shroudforge"),("logs","shroudforge/logs"),("exports","shroudforge/exports"),("updates","shroudforge/updates"),("ui","shroudforge/ui")] {loader["paths"][key]=path.into();}
    fs::write(config,serde_json::to_vec(&loader)?)?;
    let mut state:Prepared=read_json(&root.join(STATE))?;
    state.root=root.to_string_lossy().into();write_json(&root.join(STATE),&state)?;
    let policy:Policy=read_json(&root.join(POLICY))?;
    let registry=zoom::registry(&root,policy.zoom.as_ref().unwrap(),&policy.executable_sha256)?;
    let change=zoom::Change::new(&root,registry,state.effective_zoom.unwrap(),false)?;
    rewrite_selected(&root,&policy,false,Some(&change))?;
    let stage=transaction::directory(&root,STEM);
    let mut next:Prepared=read_json(&root.join(STATE))?;
    next.configured_extended_zoom=Some(false);next.effective_zoom=Some(false);
    next.previous_state_sha256=Some(stamp(&root.join(STATE))?.1);
    for name in ["enshrouded.kfc","enshrouded.kfc_resources"] {next.outputs.insert(name.into(),stamp(&stage.join(name))?);}
    let records:Vec<_>=["enshrouded.kfc_resources","enshrouded.kfc"].into_iter()
        .map(|name|(name,Some(&state.outputs[name]),&next.outputs[name])).collect();
    fs::write(stage.join("generated-pending.json"),serde_json::to_vec(&(1u32,&next.root,&records))?)?;
    fs::rename(stage.join("enshrouded.kfc_resources"),root.join("enshrouded.kfc_resources"))?;
    write_json(&root.join(PENDING),&next)?;
    verify_pending_selection(&root,&stage,&next)?;
    fs::write(Path::new(EVIDENCE).join("dll-fixture.json"),serde_json::to_vec_pretty(&serde_json::json!({"root":root,"stage":stage,"next":next,"boundary":"resources published; index staged"}))?)?;
    Ok(())
}

fn copy_tree(from:&Path,to:&Path)->Result<()> {
    ensure!(!plain(from)?.file_type().is_symlink());
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {let entry=entry?;plain(&entry.path())?;
        if entry.file_type()?.is_dir(){copy_tree(&entry.path(),&to.join(entry.file_name()))?;}
        else {fs::copy(entry.path(),to.join(entry.file_name()))?;}
    }
    Ok(())
}
fn snapshot(root:&Path)->Result<BTreeMap<String,Stamp>> {
    let mut rows=BTreeMap::new();
    for entry in fs::read_dir(root)? {let entry=entry?;
        let name=entry.file_name().to_string_lossy().to_string();
        if valid_output(&name) {rows.insert(name,stamp(&entry.path())?);}
    }
    for name in [STATE,PENDING] {if root.join(name).exists(){rows.insert(name.into(),stamp(&root.join(name))?);}}
    let stage=transaction::directory(root,STEM);
    if stage.exists(){for entry in fs::read_dir(stage)? {let entry=entry?;
        rows.insert(format!("stage/{}",entry.file_name().to_string_lossy()),stamp(&entry.path())?);
    }}
    Ok(rows)
}

#[test]
fn coherent_selection_mismatch_refuses_before_each_typed_publication_boundary()->Result<()> {
    let nonce=SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root=PathBuf::from(format!("E:/Scratch/hdmap-zoom-semantic-recovery-{}-{nonce}",std::process::id()));
    copy_tree(Path::new(SEED),&root)?;
    let root=fs::canonicalize(root)?;
    let config=root.join("shroudforge/config/modloader-config.json");
    let mut loader:serde_json::Value=read_json(&config)?;
    for (key,path) in [("mods","mods"),("cache","shroudforge/cache"),("runtime","shroudforge/runtime"),("state","shroudforge"),("logs","shroudforge/logs"),("exports","shroudforge/exports"),("updates","shroudforge/updates"),("ui","shroudforge/ui")] {loader["paths"][key]=path.into();}
    fs::write(config,serde_json::to_vec(&loader)?)?;
    let mut state:Prepared=read_json(&root.join(STATE))?;
    state.root=root.to_string_lossy().into();write_json(&root.join(STATE),&state)?;
    let policy:Policy=read_json(&root.join(POLICY))?;
    let mut results=Vec::new();
    for boundary in -1..=3 {
        let previous:Prepared=read_json(&root.join(STATE))?;
        verify_outputs(&root,&previous)?;
        let before_receipt=stamp(&root.join(STATE))?.1;
        let old_effective=previous.effective_zoom.unwrap();
        let target=!old_effective;
        let registry=zoom::registry(&root,policy.zoom.as_ref().unwrap(),&policy.executable_sha256)?;
        let change=zoom::Change::new(&root,registry,old_effective,target)?;
        rewrite_selected(&root,&policy,false,Some(&change))?;
        let stage=transaction::directory(&root,STEM);
        let mut next:Prepared=read_json(&root.join(STATE))?;
        next.configured_extended_zoom=Some(target);next.effective_zoom=Some(target);
        next.previous_state_sha256=Some(before_receipt);
        for name in ["enshrouded.kfc","enshrouded.kfc_resources"] {next.outputs.insert(name.into(),stamp(&stage.join(name))?);}
        write_json(&root.join(PENDING),&next)?;
        if boundary>=0 {
            let records:Vec<_>=["enshrouded.kfc_resources","enshrouded.kfc"].into_iter()
                .map(|name|(name,Some(&previous.outputs[name]),&next.outputs[name])).collect();
            fs::write(stage.join("generated-pending.json"),serde_json::to_vec(&(1u32,&next.root,&records))?)?;
            for (name,_,_) in records.iter().take(boundary.min(2) as usize) {fs::rename(stage.join(name),root.join(name))?;}
            if boundary==3 {transaction::recover(&root,STEM)?;ensure!(!stage.exists());}
        }
        let receipt=fs::read(root.join(STATE))?;
        fs::write(root.join(STATE),b"foreign receipt")?;
        let before=snapshot(&root)?;
        assert!(recover(&root,&next.fingerprint,2).is_err());
        assert_eq!(snapshot(&root)?,before);
        fs::write(root.join(STATE),receipt)?;
        let before=snapshot(&root)?;
        assert!(recover(&root,&"b".repeat(64),2).is_err());
        assert_eq!(snapshot(&root)?,before);
        if boundary<2 {
            let path=stage.join("enshrouded.kfc");let bytes=fs::read(&path)?;
            fs::write(&path,b"corrupt stage")?;
            let before=snapshot(&root)?;
            assert!(recover(&root,&next.fingerprint,2).is_err());
            assert_eq!(snapshot(&root)?,before);
            fs::write(path,bytes)?;
        }
        // Flip both booleans coherently. Every output hash, recipe, root and
        // before-state binding remains valid, but the typed profile disagrees.
        let mut wrong:serde_json::Value=serde_json::to_value(&next)?;
        wrong["configuredExtendedZoom"]=(!target).into();wrong["effectiveZoom"]=(!target).into();
        fs::write(root.join(PENDING),serde_json::to_vec(&wrong)?)?;
        let before=snapshot(&root)?;
        let refusal=recover(&root,&next.fingerprint,2);
        assert!(refusal.is_err(),"boundary {boundary}: coherent zoom mismatch was published");
        assert_eq!(snapshot(&root)?,before,"boundary {boundary}: refusal changed assets/state/journals");
        let zoom_error=format!("{:#}",refusal.unwrap_err());
        // A coherent parent-Off state must not describe applied HD records.
        let mut wrong:serde_json::Value=serde_json::to_value(&next)?;
        wrong["enabled"]=false.into();wrong["effectiveZoom"]=false.into();
        fs::write(root.join(PENDING),serde_json::to_vec(&wrong)?)?;
        let before=snapshot(&root)?;
        let refusal=recover(&root,&next.fingerprint,2);
        assert!(refusal.is_err(),"boundary {boundary}: coherent map mismatch was published");
        assert_eq!(snapshot(&root)?,before,"boundary {boundary}: map refusal changed files");
        let map_error=format!("{:#}",refusal.unwrap_err());
        // Repair only this fixture's deliberately changed receipt and prove
        // the unchanged postimage recovers forward, including mixed pairs.
        write_json(&root.join(PENDING),&next)?;
        recover(&root,&next.fingerprint,2)?;
        verify_outputs(&root,&next)?;verify_records(&root,&policy,true)?;change.verify(&root)?;
        assert_eq!(fs::read(root.join(STATE))?,serde_json::to_vec(&next)?);
        ensure!(!root.join(PENDING).exists()&&!stage.exists());
        // Receipt replacement completed, outer pending deletion interrupted.
        write_json(&root.join(PENDING),&next)?;
        recover(&root,&next.fingerprint,2)?;
        assert_eq!(fs::read(root.join(STATE))?,serde_json::to_vec(&next)?);
        ensure!(!root.join(PENDING).exists());
        ensure!(!root.join("enshrouded.kfc.bak").exists());
        results.push(serde_json::json!({"boundary":boundary,"zoomMismatchRefused":zoom_error,"mapMismatchRefused":map_error,"allFilesAndJournalsPreservedOnRefusal":true,"validForwardRecovery":true}));
    }
    fs::write(Path::new(EVIDENCE).join("TYPED-RECOVERY.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"PASS","fixture":root,"boundaries":results,"liveAccess":false}))?)?;
    println!("PASS real typed selection/postimage mismatch refusal and valid recovery at all5 boundaries; fixture={}",root.display());
    Ok(())
}
