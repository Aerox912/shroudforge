//! Read-only startup admission for an offline Manager-generated composition.
//! No Lua runner, asset writer, baseline restore, or publication call is reachable.
use super::*;
use std::collections::BTreeSet;

const GENERATOR_CLOSURE: &str = "4acf93eb4753c9663d557e14d6738fcfc853b0031f76c4391bffe6d989a97f98";
const MAP_EXTENSION: &str = "mods/esm-real-map/extended.mod.json";
thread_local! {static LOADERS:std::cell::RefCell<Option<(PathBuf,u32)>>=const{std::cell::RefCell::new(None)};}
pub(super) fn clear_loader_demand(){LOADERS.with(|slot|*slot.borrow_mut()=None);}
/// 0 legacy policy; high bit marks Manager ownership with optional winmm(1)/dbgcore(2).
/// A wrong-root consumption is invalid and cannot become a legacy fallback.
pub fn take_loader_demand(root:&Path)->u32 {
    match (LOADERS.with(|slot|slot.borrow_mut().take()),fs::canonicalize(root)) {
        (Some((expected,mask)),Ok(root)) if root==expected=>0x80000000|mask,
        (None,Ok(_))=>0,
        _=>u32::MAX,
    }
}

#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct FileStamp {bytes:u64,sha256:String}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {root:PathBuf,selected_ids:Vec<String>,inputs:BTreeMap<String,FileStamp>}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {schema:u32,source_closure:String,request_sha256:String,root:PathBuf,staged:BTreeMap<String,FileStamp>}
#[derive(Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Policy {
    schema_version:u32,owner:String,root:PathBuf,source_closure:String,
    request_json:String,request_sha256:String,receipt_json:String,receipt_sha256:String,
    extended_zoom:bool,preserved_loaders:BTreeMap<String,String>,
}
fn digest(bytes:&[u8])->String {format!("{:x}",Sha256::digest(bytes))}
fn valid_digest(value:&str)->bool {value.len()==64&&value.bytes().all(|c|c.is_ascii_digit()||(b'a'..=b'f').contains(&c))}
fn asset(name:&str)->bool {
    name=="enshrouded.kfc"||name=="enshrouded.kfc_resources"||name.strip_prefix("enshrouded_")
        .and_then(|s|s.strip_suffix(".dat")).is_some_and(|s|!s.is_empty()&&s.bytes().all(|c|c.is_ascii_digit()))
}
fn checked(root:&Path,name:&str)->Result<PathBuf> {
    ensure!(!name.is_empty()&&!name.contains(['\\',':'])&&!name.starts_with('/')
        &&name.split('/').all(|s|!s.is_empty()&&s!="."&&s!=".."&&!s.ends_with([' ','.'])
            &&!s.chars().any(|c|c.is_control()||"<>\"|?*".contains(c))),"unsafe Manager input path");
    joined(root,name)
}
fn exact(root:&Path,name:&str,value:&FileStamp)->Result<()> {
    ensure!(valid_digest(&value.sha256)&&stamp(&checked(root,name)?)?==(value.bytes,value.sha256.clone()),"Manager committed file differs: {name}");Ok(())
}
fn manager_lease(root:&Path)->Result<fs::File> {
    let path=checked(root,".EnshroudedClientMods/operation.lock")?;
    ensure!(plain(&path)?.is_file(),"Manager operation lock missing");
    let mut options=fs::OpenOptions::new();options.read(true);
    #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt;options.share_mode(0);}
    Ok(options.open(path).context("Manager transaction still active; finish recovery before game launch")?)
}
pub(super) fn verify(root:&Path)->Result<()> {
    for parent in root.ancestors(){ensure!(plain(parent)?.is_dir(),"invalid Manager game root");}
    let root=fs::canonicalize(root)?;
    let _outer=manager_lease(&root)?;let _startup=lease(&root)?;
    ensure!(!transaction::directory(&root,STEM).exists(),"Manager asset publication is still pending");
    for name in [STATE,PENDING] {ensure!(!joined(&root,name)?.exists(),"conflicting current-assets owner state");}
    let policy:Policy=read_json(&joined(&root,POLICY)?)?;
    ensure!(policy.schema_version==3&&policy.owner=="manager-asset-generator"
        &&policy.source_closure==GENERATOR_CLOSURE&&fs::canonicalize(&policy.root)?==root,"Manager startup owner identity differs");
    ensure!(valid_digest(&policy.request_sha256)&&digest(policy.request_json.as_bytes())==policy.request_sha256
        &&valid_digest(&policy.receipt_sha256)&&digest(policy.receipt_json.as_bytes())==policy.receipt_sha256,"Manager startup evidence digest differs");
    let request:Request=serde_json::from_str(&policy.request_json)?;
    let receipt:Receipt=serde_json::from_str(&policy.receipt_json)?;
    ensure!(receipt.schema==1&&receipt.source_closure==GENERATOR_CLOSURE
        &&receipt.request_sha256==digest(&serde_json::to_vec(&request)?)
        &&fs::canonicalize(&request.root)?==root&&receipt.root==root,"Manager generation receipt identity differs");
    let ids:BTreeSet<_>=request.selected_ids.iter().cloned().collect();
    ensure!(!ids.is_empty()&&ids.len()==request.selected_ids.len(),"invalid Manager selection");
    ensure!(request.inputs.len()<=10000&&receipt.staged.len()<=512,"oversized Manager composition");
    for name in ["enshrouded.exe","enshrouded.kfc","enshrouded.kfc_resources"] {ensure!(request.inputs.contains_key(name),"incomplete Manager baseline");}
    for name in ["enshrouded.kfc","enshrouded.kfc_resources","generated-inputs.json"] {ensure!(receipt.staged.contains_key(name),"incomplete Manager generated receipt");}
    for (name,value) in &receipt.staged {
        ensure!(asset(name)||name=="generated-inputs.json","unexpected generated receipt member");
        if asset(name) {
            if name.ends_with(".dat"){ensure!(!request.inputs.contains_key(name),"generated DAT overlaps baseline");}
            exact(&root,name,value)?;
        }
    }
    let mut aliases=BTreeSet::new();
    for (name,value) in &request.inputs {
        ensure!(aliases.insert(name.to_ascii_lowercase()),"Manager input alias");
        let path=checked(&root,name)?;
        if asset(name)&&name.ends_with(".dat") {
            // Official baseline content was verified when Manager generated the
            // composition. Do not rehash all official DATs at every game launch.
            ensure!(plain(&path)?.is_file()&&fs::metadata(path)?.len()==value.bytes,"baseline DAT membership/size differs");
        } else if !receipt.staged.contains_key(name) {exact(&root,name,value)?;}
    }
    let expected:BTreeSet<_>=request.inputs.keys().chain(receipt.staged.keys()).filter(|n|asset(n)).cloned().collect();
    let mut actual=BTreeSet::new();
    for entry in fs::read_dir(&root)? {let name=entry?.file_name().into_string().map_err(|_|anyhow::anyhow!("non-UTF8 game asset"))?;
        if asset(&name.to_ascii_lowercase()){ensure!(name==name.to_ascii_lowercase(),"game asset alias");actual.insert(name);}}
    ensure!(actual==expected,"Manager asset membership differs");
    ensure!(policy.preserved_loaders.keys().all(|name|matches!(name.as_str(),"dbgcore.dll"|"winmm.dll")),"preserved loader contract differs");
    for (name,hash) in &policy.preserved_loaders {ensure!(valid_digest(hash)&&stamp(&checked(&root,name)?)?.1==*hash,"preserved loader differs: {name}");}
    let map_selected=ids.contains("esm-real-map");
    if map_selected {
        ensure!(policy.preserved_loaders.contains_key("winmm.dll"),"HD Map requires its selected Shroudtopia loader");
        ensure!(request.inputs.contains_key(MAP_EXTENSION),"HD Map configuration was not generation-pinned");
        let extension:serde_json::Value=read_json(&checked(&root,MAP_EXTENSION)?)?;
        let configured=extension.pointer("/settings/ExtendedZoom/value").and_then(serde_json::Value::as_bool)
            .or_else(||extension.pointer("/settings/ExtendedZoom").and_then(serde_json::Value::as_bool)).context("missing HD Map ExtendedZoom selection")?;
        ensure!(extension["enabled"].as_bool()==Some(true)&&configured==policy.extended_zoom,"HD Map selection differs from committed generation");
    } else {ensure!(!policy.extended_zoom,"zoom requires selected HD Map");}
    zoom_demand::publish(&root,policy.extended_zoom,&request.inputs["enshrouded.exe"].sha256);
    let mask=u32::from(policy.preserved_loaders.contains_key("winmm.dll"))
        |(u32::from(policy.preserved_loaders.contains_key("dbgcore.dll"))<<1);
    LOADERS.with(|slot|*slot.borrow_mut()=Some((root,mask)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(PathBuf,Policy) {
        let root=std::env::temp_dir().join(format!("sf-manager-startup-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(root.join(".EnshroudedClientMods")).unwrap();
        fs::create_dir_all(root.join("shroudforge/config")).unwrap();
        fs::create_dir_all(root.join("mods/esm-real-map")).unwrap();
        let root=fs::canonicalize(root).unwrap();
        for name in ["enshrouded.exe","enshrouded.kfc","enshrouded.kfc_resources","enshrouded_000.dat","enshrouded_001.dat","winmm.dll","dbgcore.dll",".EnshroudedClientMods/operation.lock"] {
            fs::write(root.join(name),format!("fixture-{name}")).unwrap();
        }
        fs::write(root.join(MAP_EXTENSION),br#"{"schemaVersion":1,"enabled":true,"settings":{"ExtendedZoom":{"value":false}}}"#).unwrap();
        let value=|name:&str|{let (bytes,sha256)=stamp(&root.join(name)).unwrap();FileStamp{bytes,sha256}};
        let inputs=["enshrouded.exe","enshrouded.kfc","enshrouded.kfc_resources","enshrouded_000.dat",MAP_EXTENSION]
            .into_iter().map(|name|(name.into(),value(name))).collect();
        let request=Request{root:root.clone(),selected_ids:vec!["esm-real-map".into()],inputs};
        let mut staged:BTreeMap<_,_>=["enshrouded.kfc","enshrouded.kfc_resources","enshrouded_001.dat"].into_iter().map(|name|(name.into(),value(name))).collect();
        staged.insert("generated-inputs.json".into(),FileStamp{bytes:0,sha256:"0".repeat(64)});
        let receipt=Receipt{schema:1,source_closure:GENERATOR_CLOSURE.into(),request_sha256:digest(&serde_json::to_vec(&request).unwrap()),root:root.clone(),staged};
        let request_json=serde_json::to_string(&request).unwrap();let receipt_json=serde_json::to_string(&receipt).unwrap();
        let policy=Policy{schema_version:3,owner:"manager-asset-generator".into(),root:root.clone(),source_closure:GENERATOR_CLOSURE.into(),
            request_sha256:digest(request_json.as_bytes()),request_json,receipt_sha256:digest(receipt_json.as_bytes()),receipt_json,extended_zoom:false,
            preserved_loaders:["winmm.dll","dbgcore.dll"].into_iter().map(|n|(n.into(),value(n).sha256)).collect()};
        (root,policy)
    }
    fn write(root:&Path,policy:&Policy){fs::write(root.join(POLICY),serde_json::to_vec(policy).unwrap()).unwrap();}
    fn files(root:&Path)->BTreeMap<String,Stamp> {fs::read_dir(root).unwrap().map(|e|e.unwrap()).filter(|e|e.path().is_file()).map(|e|(e.file_name().to_str().unwrap().into(),stamp(&e.path()).unwrap())).collect()}
    #[test] fn valid_manager_generation_never_rewrites_assets_or_creates_state() {
        let (root,policy)=fixture();write(&root,&policy);let before=files(&root);
        assert!(super::super::prepare(&root).unwrap());assert_eq!(zoom_demand::take(&root),0);
        assert_eq!(files(&root),before);assert!(!root.join(STATE).exists());assert!(!root.join(PENDING).exists());
        assert!(!transaction::directory(&root,STEM).exists());assert!(!root.join("enshrouded.kfc.bak").exists());
    }
    #[test] fn tampered_outputs_inputs_loaders_and_unknown_assets_fail_without_repair() {
        for target in ["enshrouded.kfc","enshrouded_001.dat",MAP_EXTENSION,"winmm.dll","enshrouded.exe"] {
            let (root,policy)=fixture();write(&root,&policy);fs::write(root.join(target),"tampered").unwrap();let before=files(&root);
            assert!(super::super::prepare(&root).is_err(),"{target}");assert_eq!(zoom_demand::take(&root),2);assert_eq!(files(&root),before);
        }
        let (root,policy)=fixture();write(&root,&policy);fs::write(root.join("enshrouded_999.dat"),"foreign").unwrap();let before=files(&root);
        assert!(super::super::prepare(&root).is_err());assert_eq!(files(&root),before);
    }
    #[test] fn raw_evidence_zoom_and_owner_changes_never_fall_back_to_recipe() {
        for mode in 0..4 {let (root,mut policy)=fixture();match mode {0=>policy.request_json.push(' '),1=>policy.receipt_json.push(' '),2=>policy.extended_zoom=true,_=>policy.owner="other".into()};
            write(&root,&policy);let before=files(&root);assert!(super::super::prepare(&root).is_err());assert_eq!(zoom_demand::take(&root),2);assert_eq!(files(&root),before);
        }
    }
    #[test] fn baseline_dat_bytes_are_not_rehashed_but_membership_and_size_are_required() {
        let (root,policy)=fixture();write(&root,&policy);let name=root.join("enshrouded_000.dat");let old=fs::read(&name).unwrap();fs::write(&name,vec![b'x';old.len()]).unwrap();
        assert!(super::super::prepare(&root).unwrap());fs::write(&name,"truncated").unwrap();assert!(super::super::prepare(&root).is_err());
    }
    #[cfg(windows)] #[test] fn active_manager_transaction_prevents_startup_admission() {
        use std::os::windows::fs::OpenOptionsExt;
        let (root,policy)=fixture();write(&root,&policy);
        let _manager=fs::OpenOptions::new().read(true).write(true).share_mode(0).open(root.join(".EnshroudedClientMods/operation.lock")).unwrap();
        assert!(super::super::prepare(&root).is_err());assert_eq!(zoom_demand::take(&root),2);
    }
    #[test] fn optional_dbgcore_and_eml_only_composition_do_not_require_unselected_loaders() {
        let (root,mut policy)=fixture();policy.preserved_loaders.remove("dbgcore.dll");fs::remove_file(root.join("dbgcore.dll")).unwrap();write(&root,&policy);
        assert!(super::super::prepare(&root).unwrap());assert_eq!(take_loader_demand(&root),0x80000001);
        let (root,mut policy)=fixture();let mut request:Request=serde_json::from_str(&policy.request_json).unwrap();request.selected_ids=vec!["EMBER".into()];
        let mut receipt:Receipt=serde_json::from_str(&policy.receipt_json).unwrap();receipt.request_sha256=digest(&serde_json::to_vec(&request).unwrap());
        policy.request_json=serde_json::to_string(&request).unwrap();policy.request_sha256=digest(policy.request_json.as_bytes());
        policy.receipt_json=serde_json::to_string(&receipt).unwrap();policy.receipt_sha256=digest(policy.receipt_json.as_bytes());
        policy.preserved_loaders.clear();fs::remove_file(root.join("winmm.dll")).unwrap();fs::remove_file(root.join("dbgcore.dll")).unwrap();write(&root,&policy);
        assert!(super::super::prepare(&root).unwrap());assert_eq!(take_loader_demand(&root),0x80000000);assert_eq!(zoom_demand::take(&root),0);
    }
}
