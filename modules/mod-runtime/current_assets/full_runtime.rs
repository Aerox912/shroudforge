//! Explicit schema6 admission. This is not a generator or a native permission API.
use super::*;
use serde_json::Value;
use std::sync::{Mutex,OnceLock};

pub const HANDSHAKE_V1:u32=0x5346_0100;
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct NativeFile {pub bytes:u64,pub sha256:String}
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Contract {
    pub schema_version:u32,
    pub host:String,
    pub runtime_source_closure:String,
    pub prepared_request_sha256:String,
    pub prepared_receipt_sha256:String,
    pub boot_services:Vec<String>,
    pub overlay:bool,
    pub first_person:bool,
    pub runtime_mods:Vec<String>,
    /// Hashes of effective canonical package Lua, including profile overrides.
    /// Keys are exact `manifest-id:src/path.lua`, not native permissions.
    pub scripts:BTreeMap<String,NativeFile>,
    pub dll_plan:Value,
    pub files:BTreeMap<String,NativeFile>,
    // Exact selected paths whose independently qualified owner is Greeneye.
    // No compatibility is inferred from a basename or from file presence.
    pub greeneye_paths:Vec<String>,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Phase {Verified,Preparing,Prepared,Taken,Committed,Running,Failed}
#[derive(Clone)]
struct PackageBinding {directory:PathBuf,metadata:Value}
struct Admission {root:PathBuf,contract:Contract,phase:Phase,packages:BTreeMap<String,PackageBinding>,_files:Vec<(String,fs::File)>}
static ADMISSION:OnceLock<Mutex<Option<Admission>>>=OnceLock::new();
fn state()->&'static Mutex<Option<Admission>> {ADMISSION.get_or_init(||Mutex::new(None))}
fn lock()->std::sync::MutexGuard<'static,Option<Admission>> {state().lock().unwrap_or_else(|e|e.into_inner())}
fn valid_hash(s:&str)->bool {s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))}

/// 0 ordinary legacy startup; 1 historical current-assets mode; 2 explicit full.
/// Errors are never permission to enter the ordinary migrations/backup route.
pub fn mode(root:&Path)->Result<u32> {
    if !super::active(root)? {return Ok(0);}
    let value:Value=read_json(&joined(root,POLICY)?)?;
    match value["schemaVersion"].as_u64() {
        Some(6)=>Ok(2),Some(1..=5)=>Ok(1),_=>anyhow::bail!("unsupported current-assets startup schema"),
    }
}
fn validate_contract(c:&Contract,producer:&str,request:&str,receipt:&str)->Result<()> {
    let compiled=option_env!("SF_FULL_RUNTIME_SOURCE_CLOSURE").context("full runtime source closure not compiled")?;
    validate_contract_pair(c,producer,request,receipt,compiled,option_env!("HD_PREPARED_SOURCE_CLOSURE"))
}
fn validate_contract_pair(c:&Contract,producer:&str,request:&str,receipt:&str,compiled:&str,legacy:Option<&str>)->Result<()> {
    ensure!(valid_hash(compiled)&&valid_hash(producer)&&c.runtime_source_closure==compiled,"full runtime source pair differs");
    ensure!(Some(producer)!=legacy,"historical prepared producer cannot authorize full runtime");
    ensure!(c.schema_version==1&&c.host=="sf-winmm-startup-gate-v1","unsupported full runtime host contract");
    ensure!(c.prepared_request_sha256==request&&c.prepared_receipt_sha256==receipt,"full runtime composition binding differs");
    let mut expected=vec!["required-engine-hooks-v1"];
    if c.overlay{expected.push("graphics-boot-v1");}if c.first_person{expected.push("first-person-preparation-v1");}
    ensure!(c.boot_services.iter().map(String::as_str).collect::<Vec<_>>()==expected,"unsupported full runtime boot requirements (World services are late-bound)");
    ensure!(c.files.len()>=4&&c.files.len()<=132&&c.greeneye_paths.len()<=8,"invalid full runtime file set");
    let ids:HashSet<_>=c.runtime_mods.iter().collect();
    ensure!(ids.len()==c.runtime_mods.len()&&ids.len()<=256&&c.scripts.len()<=4096,"invalid full runtime package/source set");
    Ok(())
}
fn projected_runtime_plan<'a>(env:&'a mod_loader::ModEnvironment,c:&Contract)->Result<Vec<&'a mod_loader::Mod>> {
    let (mut candidates,_)=env.runtime_plan_report_detailed(false,crate::API_VERSION);
    let config=shroudforge_loader_config::config::read_loader(env.game_dir().as_std_path()).map_err(anyhow::Error::msg)?;
    if config.pointer("/modules/worldEditor/enabled").and_then(Value::as_bool)==Some(false){candidates.retain(|m|m.info().id!="world-editor");}
    ensure!(candidates.iter().all(|m|!m.info().enabled||c.runtime_mods.contains(&m.info().id)),"enabled runtime package is outside the prepared composition; prepare and restart");
    candidates.retain(|m|c.runtime_mods.contains(&m.info().id));
    ensure!(candidates.iter().map(|m|&m.info().id).eq(c.runtime_mods.iter()),"ordered prepared runtime plan differs; prepare and restart");
    Ok(candidates)
}
fn bind_runtime_packages(root:&Path,c:&Contract,environment:&BTreeMap<String,Stamp>)->Result<BTreeMap<String,PackageBinding>> {
    let env=mod_loader::ModEnvironment::load(root.to_str().context("nonUTF8 runtime root")?).map_err(|e|anyhow::anyhow!("{e:?}"))?;
    let plan=projected_runtime_plan(&env,c)?;let mut bindings=BTreeMap::new();
    for package in plan {
        let directory=fs::canonicalize(package.fs().root().as_std_path())?;
        ensure!(plain(&directory)?.is_dir(),"schema6 runtime package requires a bound directory");
        let relative=directory.strip_prefix(root).context("runtime package escapes prepared root")?.to_str().context("nonUTF8 runtime package")?.replace('\\',"/");
        ensure!(environment.contains_key(&format!("{relative}/mod.json")),"runtime package metadata is not in prepared environment");
        if directory.join("extended.mod.json").exists(){ensure!(environment.contains_key(&format!("{relative}/extended.mod.json")),"runtime settings are not in prepared environment");}
        ensure!(c.scripts.contains_key(&format!("{}:src/mod.lua",package.info().id)),"runtime entrypoint source is not bound");
        bindings.insert(package.info().id.clone(),PackageBinding{directory,metadata:serde_json::to_value(package.info())?});
    }
    for(key,expected)in &c.scripts {
        let(id,path)=key.split_once(':').context("invalid prepared script identity")?;
        ensure!(bindings.contains_key(id)&&path.starts_with("src/")&&path.ends_with(".lua")&&Path::new(path).components().all(|p|matches!(p,Component::Normal(_))),"unbound/unsafe prepared script");
        ensure!(valid_hash(&expected.sha256)&&expected.bytes<=8*1024*1024,"invalid prepared script stamp");
        let package=env.mod_registry().get(id).context("missing script package")?;
        let mut fs=package.fs();let reader=fs.read_file(path)?;let mut bytes=Vec::new();reader.take(8*1024*1024+1).read_to_end(&mut bytes)?;
        ensure!(bytes.len()as u64==expected.bytes&&format!("{:x}",Sha256::digest(&bytes))==expected.sha256,"effective prepared runtime source differs: {key}");
    }
    Ok(bindings)
}
fn snapshot(root:&Path)->Result<Option<(Contract,BTreeMap<String,PackageBinding>)>> {
    let slot=lock();let Some(a)=slot.as_ref()else{return Ok(None);};
    ensure!(fs::canonicalize(root)?==a.root&&a.phase==Phase::Running,"full runtime composition is not live or is closed");
    Ok(Some((a.contract.clone(),a.packages.clone())))
}
pub(crate) fn select_runtime_plan<'a>(env:&'a mod_loader::ModEnvironment,plan:&mut Vec<&'a mod_loader::Mod>)->Result<()> {
    let Some((c,bindings))=snapshot(env.game_dir().as_std_path())?else{return Ok(());};
    projected_runtime_plan(env,&c)?;
    plan.retain(|m|c.runtime_mods.contains(&m.info().id));
    ensure!(plan.iter().map(|m|&m.info().id).eq(c.runtime_mods.iter()),"effective runtime plan differs from prepared composition");
    for package in plan.iter() {
        let bound=bindings.get(&package.info().id).context("runtime package not admitted")?;
        ensure!(fs::canonicalize(package.fs().root().as_std_path())?==bound.directory,"runtime package moved");
        ensure!(crate::ingame::settings_only_manifest_change(&serde_json::to_value(package.info())?,&bound.metadata),"runtime package metadata changed; prepare and restart");
    }
    Ok(())
}
pub(crate) fn validate_runtime_manifest(root:&Path,id:&str,metadata:&Value)->Result<()> {
    let Some((_,bindings))=snapshot(root)?else{return Ok(());};
    check_runtime_manifest(&bindings,id,metadata)
}
fn check_runtime_manifest(bindings:&BTreeMap<String,PackageBinding>,id:&str,metadata:&Value)->Result<()> {
    let bound=bindings.get(id).context("runtime package is not admitted; prepare and restart")?;
    ensure!(crate::ingame::settings_only_manifest_change(metadata,&bound.metadata),"runtime metadata changed; prepare and restart");Ok(())
}
pub(crate) fn validate_script(root:&Path,id:&str,package:&Path,path:&str,bytes:&[u8])->Result<()> {
    let Some((c,bindings))=snapshot(root)?else{return Ok(());};
    check_script(&c,&bindings,id,package,path,bytes)
}
pub(crate) fn script_size_bound(root:&Path,id:&str,path:&str)->Result<Option<u64>> {
    let Some((c,_))=snapshot(root)?else{return Ok(None);};
    Ok(Some(c.scripts.get(&format!("{id}:{path}")).context("runtime source is not admitted; prepare and restart")?.bytes))
}
fn check_script(c:&Contract,bindings:&BTreeMap<String,PackageBinding>,id:&str,package:&Path,path:&str,bytes:&[u8])->Result<()> {
    let bound=bindings.get(id).context("runtime source package is not admitted")?;
    ensure!(fs::canonicalize(package)?==bound.directory,"runtime source package moved");
    let expected=c.scripts.get(&format!("{id}:{path}")).context("runtime source is not admitted; prepare and restart")?;
    ensure!(bytes.len()as u64==expected.bytes&&format!("{:x}",Sha256::digest(bytes))==expected.sha256,"runtime source changed; prepare and restart");Ok(())
}
pub(crate) fn reject_package_reload(root:&Path)->Result<()> {
    ensure!(snapshot(root)?.is_none(),"prepared full-runtime package reload requires a newly verified preparation and restart");Ok(())
}
pub(crate) fn has_verified_composition(root:&Path)->bool {
    // Only for the existing asset-plan check, never a native service readiness.
    snapshot(root).ok().flatten().is_some()
}
fn keep_readonly(path:&Path)->Result<fs::File> {
    let mut options=fs::OpenOptions::new();options.read(true);
    #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt;options.share_mode(1);}
    Ok(options.open(path)?)
}
fn verify_files(root:&Path,c:&Contract)->Result<Vec<(String,fs::File)>> {
    let mut aliases=HashSet::new();let mut files=Vec::new();
    for(name,expected)in &c.files {
        ensure!(aliases.insert(name.to_ascii_lowercase())&&valid_hash(&expected.sha256),"invalid/aliased full runtime file");
        let path=joined(root,name)?;ensure!(plain(&path)?.is_file(),"full runtime file is not regular");
        let held=keep_readonly(&path)?;
        ensure!(stamp(&path)?==(expected.bytes,expected.sha256.clone()),"full runtime file drift: {name}");files.push((name.clone(),held));
    }
    for name in ["winmm.dll","shroudforge/shroudforge-runtime.dll","shroudforge/kfc-runtime.dll","shroudforge/config/modloader-config.json"] {
        ensure!(c.files.contains_key(name),"missing full runtime owned file: {name}");
    }
    Ok(files)
}
pub(crate) fn validate_plan(root:&Path,c:&Contract)->Result<()> {
    ensure!(crate::native_dlls::manager_enabled(root).map_err(anyhow::Error::msg)?,"full runtime DLL Manager is disabled");
    let plan=mod_loader::dll_manager::resolve_load_plan(root,false);
    ensure!(plan.issues.is_empty()&&mod_loader::dll_manager::load_plan_identity(&plan)==c.dll_plan,"full runtime DLL selection differs or has unresolved issues");
    let selections=mod_loader::dll_manager::read_selections(root);
    let mut paths=HashSet::new();let mut images=HashSet::new();let mut greeneye=HashSet::new();
    for entry in &plan.entries {
        ensure!(entry.mode==mod_loader::dll_manager::DllLoadMode::Library,"schema6 DirectInput chain host is unqualified; select an approved Library route");
        ensure!(selections.iter().any(|s|s.package_directory.eq_ignore_ascii_case(&entry.package_directory)&&s.enabled&&s.approved&&s.target.applies_to(false)),"full runtime DLL lacks explicit approval");
        let path=fs::canonicalize(&entry.path)?;
        let relative=path.strip_prefix(root).context("DLL escapes full runtime root")?.to_str().context("nonUTF8 DLL")?.replace('\\',"/");
        let key=relative.to_ascii_lowercase();
        let name=path.file_name().context("missing DLL filename")?.to_string_lossy().to_ascii_lowercase();
        ensure!(!matches!(name.as_str(),"winmm.dll"|"shroudforge-runtime.dll"|"kfc-runtime.dll"|"shroudtopia.dll"),"full runtime dependency is a proxy/self/unsupported provider");
        ensure!(paths.insert(key.clone()),"duplicate canonical full runtime DLL");
        let expected=c.files.get(&relative).context("unbound full runtime DLL bytes")?;
        for file in ["mod.json","native-plugin.ini"] {
            let name=format!("mods/{}/{file}",entry.package_directory);
            if joined(root,&name)?.exists(){ensure!(c.files.contains_key(&name),"unbound native declaration: {name}");}
        }
        ensure!(images.insert(expected.sha256.clone()),"duplicate full runtime DLL image");
        for owned in ["winmm.dll","shroudforge/shroudforge-runtime.dll","shroudforge/kfc-runtime.dll"] {
            ensure!(expected.sha256!=c.files[owned].sha256,"self dependency alias");
        }
        if name=="dbgcore.dll" {ensure!(c.greeneye_paths.contains(&relative),"Greeneye requires explicit qualified selection");greeneye.insert(relative);}
    }
    ensure!(greeneye.len()==c.greeneye_paths.len(),"unknown or duplicate Greeneye selection");
    Ok(())
}
pub(super) fn verify_and_stage(root:&Path,c:Contract,request:&str,receipt:&str,producer:&str,environment:&BTreeMap<String,Stamp>)->Result<()> {
    validate_contract(&c,producer,request,receipt)?;
    let mut files=verify_files(root,&c)?;
    ensure!(environment.len()<=10000,"oversized full runtime environment");
    // Preserve the verified package/configuration bytes until start_live has
    // actually read them. Only read leases are created, never asset copies.
    for(name,expected)in environment {
        let path=joined(root,name)?;ensure!(plain(&path)?.is_file(),"invalid full runtime environment member");
        let held=keep_readonly(&path)?;ensure!(&stamp(&path)?==expected,"full runtime environment drift: {name}");files.push((name.clone(),held));
    }
    validate_plan(root,&c)?;
    let packages=bind_runtime_packages(root,&c,environment)?;
    let mut slot=lock();ensure!(slot.is_none(),"full runtime startup already attempted");
    *slot=Some(Admission{root:root.into(),contract:c,phase:Phase::Verified,packages,_files:files});Ok(())
}
fn transition(root:&Path,from:Phase,to:Phase)->Result<Contract> {
    let root=fs::canonicalize(root)?;let mut slot=lock();let a=slot.as_mut().context("full runtime admission missing")?;
    move_to(a,&root,from,to)
}
fn move_to(a:&mut Admission,root:&Path,from:Phase,to:Phase)->Result<Contract> {
    if a.root!=root||a.phase!=from {a.phase=Phase::Failed;anyhow::bail!("full runtime operation out of sequence");}
    a.phase=to;Ok(a.contract.clone())
}
pub fn prepare_native(root:&Path)->Result<()> {
    let c=transition(root,Phase::Verified,Phase::Preparing)?;
    let result=crate::native_dlls::prepare_full_startup(root,&c).map_err(anyhow::Error::msg);
    if let Err(e)=result {abort(root);return Err(e);}
    transition(root,Phase::Preparing,Phase::Prepared)?;Ok(())
}
pub fn take(root:&Path)->u32 {
    transition(root,Phase::Prepared,Phase::Taken).map(|c|HANDSHAKE_V1|u32::from(c.overlay)|(u32::from(c.first_person)<<1)).unwrap_or(u32::MAX)
}
pub fn commit(root:&Path,handshake:u32)->bool {
    let mut slot=lock();let Some(a)=slot.as_mut()else{return false;};
    if fs::canonicalize(root).ok().as_ref()!=Some(&a.root)||a.phase!=Phase::Taken||handshake!=(HANDSHAKE_V1|u32::from(a.contract.overlay)|(u32::from(a.contract.first_person)<<1)){a.phase=Phase::Failed;return false;}
    a.phase=Phase::Committed;true
}
pub fn before_create(root:&Path)->Result<()> {
    // A deleted/changed policy cannot turn an existing full attempt into legacy.
    let full=lock().is_some();
    if full||mode(root)?==2 {
        let c=transition(root,Phase::Committed,Phase::Running)?;
        validate_plan(root,&c)?;
        let env=mod_loader::ModEnvironment::load(root.to_str().context("nonUTF8 runtime root")?).map_err(|e|anyhow::anyhow!("{e:?}"))?;
        let mut plan=projected_runtime_plan(&env,&c)?;select_runtime_plan(&env,&mut plan)?;
    }
    Ok(())
}
pub fn after_create(root:&Path)->Result<()> {
    let mut slot=lock();
    if let Some(a)=slot.as_mut(){
        ensure!(fs::canonicalize(root)?==a.root&&a.phase==Phase::Running,"full runtime create completion differs");
        a._files.retain(|(name,_)|name.to_ascii_lowercase().ends_with(".dll"));
    }
    Ok(())
}
pub fn abort(_: &Path) {if let Some(a)=lock().as_mut(){a.phase=Phase::Failed;}}
#[cfg(test)]
pub(super) fn retire_fixture(){*lock()=None;}

#[cfg(test)]
mod tests {
    use super::*;
    fn contract()->Contract {
        Contract{schema_version:1,host:"sf-winmm-startup-gate-v1".into(),runtime_source_closure:"a".repeat(64),prepared_request_sha256:"b".repeat(64),prepared_receipt_sha256:"c".repeat(64),boot_services:vec!["required-engine-hooks-v1".into()],overlay:false,first_person:false,runtime_mods:vec![],scripts:BTreeMap::new(),dll_plan:Value::Array(vec![]),files:["winmm.dll","shroudforge/shroudforge-runtime.dll","shroudforge/kfc-runtime.dll","shroudforge/config/modloader-config.json"].into_iter().map(|n|(n.into(),NativeFile{bytes:1,sha256:"d".repeat(64)})).collect(),greeneye_paths:vec![]}
    }
    #[test] fn compiled_full_pair_must_exist_and_match() {
        let mut c=contract();c.runtime_source_closure="0".repeat(64);
        assert!(validate_contract(&c,&"d".repeat(64),&"b".repeat(64),&"c".repeat(64)).is_err());
    }
    #[test] fn exact_new_contract_accepts_only_supported_boot_requirements_and_new_pair() {
        let c=contract();let producer="e".repeat(64);
        let verify=|c:&Contract|validate_contract_pair(c,&producer,&"b".repeat(64),&"c".repeat(64),&"a".repeat(64),Some(&"f".repeat(64)));
        verify(&c).unwrap();
        for edit in 0..5 {let mut c=c.clone();match edit {0=>c.host="external-winmm".into(),1=>c.runtime_source_closure="f".repeat(64),2=>c.prepared_request_sha256="f".repeat(64),3=>c.boot_services.push("World-ready".into()),_=>c.schema_version=2};assert!(verify(&c).is_err());}
        assert!(validate_contract_pair(&c,&producer,&"b".repeat(64),&"c".repeat(64),&"a".repeat(64),Some(&producer)).is_err());
    }
    #[test] fn full_sequence_is_one_shot_and_wrong_root_cannot_refresh_it() {
        let root=PathBuf::from("E:/owned/synthetic-root");
        let mut a=Admission{root:root.clone(),contract:contract(),phase:Phase::Verified,packages:BTreeMap::new(),_files:vec![]};
        for (from,to) in [(Phase::Verified,Phase::Preparing),(Phase::Preparing,Phase::Prepared),(Phase::Prepared,Phase::Taken),(Phase::Taken,Phase::Committed),(Phase::Committed,Phase::Running)] {move_to(&mut a,&root,from,to).unwrap();}
        assert!(move_to(&mut a,&root,Phase::Committed,Phase::Running).is_err());assert_eq!(a.phase,Phase::Failed);
        a.phase=Phase::Prepared;assert!(move_to(&mut a,&root.join("other"),Phase::Prepared,Phase::Taken).is_err());assert_eq!(a.phase,Phase::Failed);
    }
    struct Fixture{root:PathBuf,c:Contract}
    impl Fixture {
        fn new(names:&[&str])->Self {
            let root=std::env::temp_dir().join(format!("sf-full-route-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
            fs::create_dir_all(root.join("shroudforge/config")).unwrap();fs::create_dir_all(root.join("mods/fixture")).unwrap();
            let root=fs::canonicalize(root).unwrap();
            let mut config=shroudforge_loader_config::config::read_loader_defaults().unwrap();
            config["modules"]["dllManager"]["enabled"]=true.into();
            config["modules"]["dllManager"]["profiles"]=serde_json::json!([{"id":"fixture","packageDirectory":"fixture","name":"Fixture","target":"client","enabled":true,"approved":true,"files":names.iter().map(|n|serde_json::json!({"path":n,"mode":"library"})).collect::<Vec<_>>()}]);
            fs::write(root.join("shroudforge/config/modloader-config.json"),serde_json::to_vec(&config).unwrap()).unwrap();
            for name in names{fs::write(root.join("mods/fixture").join(name),b"owned synthetic DLL bytes; never executed").unwrap();}
            let mut c=contract();
            for name in ["winmm.dll","shroudforge/shroudforge-runtime.dll","shroudforge/kfc-runtime.dll"] {fs::write(root.join(name),name).unwrap();}
            for name in c.files.keys().cloned().chain(names.iter().map(|n|format!("mods/fixture/{n}"))).collect::<Vec<_>>() {
                let(bytes,sha256)=stamp(&root.join(&name)).unwrap();c.files.insert(name,NativeFile{bytes,sha256});
            }
            c.dll_plan=mod_loader::dll_manager::load_plan_identity(&mod_loader::dll_manager::resolve_load_plan(&root,false));
            Self{root,c}
        }
    }
    impl Drop for Fixture {fn drop(&mut self){if self.root.starts_with(std::env::temp_dir())&&self.root.file_name().unwrap().to_string_lossy().starts_with("sf-full-route-"){let _=fs::remove_dir_all(&self.root);}}}
    #[test] fn approved_plan_accepts_without_loading_and_disabled_or_changed_plan_refuses() {
        let f=Fixture::new(&["plugin.dll"]);assert_eq!(f.c.dll_plan.as_array().unwrap().len(),1);validate_plan(&f.root,&f.c).unwrap();
        let mut config=shroudforge_loader_config::config::read_loader(&f.root).unwrap();config["modules"]["dllManager"]["enabled"]=false.into();
        fs::write(f.root.join("shroudforge/config/modloader-config.json"),serde_json::to_vec(&config).unwrap()).unwrap();assert!(validate_plan(&f.root,&f.c).is_err());
    }
    #[test] fn duplicate_images_self_proxy_and_implicit_greeneye_refuse() {
        let f=Fixture::new(&["plugin.dll","alias.dll"]);assert!(validate_plan(&f.root,&f.c).is_err());
        let f=Fixture::new(&["winmm.dll"]);assert!(validate_plan(&f.root,&f.c).is_err());
        let mut f=Fixture::new(&["dbgcore.dll"]);assert!(validate_plan(&f.root,&f.c).is_err());
        f.c.greeneye_paths.push("mods/fixture/dbgcore.dll".into());validate_plan(&f.root,&f.c).unwrap();
    }
    #[test] fn directinput_mode_refuses_even_when_declared_plan_matches() {
        let mut f=Fixture::new(&["plugin.dll"]);
        let path=f.root.join("shroudforge/config/modloader-config.json");
        let mut config=shroudforge_loader_config::config::read_loader(&f.root).unwrap();
        config["modules"]["dllManager"]["profiles"][0]["files"][0]["mode"]="dinput8".into();
        fs::write(path,serde_json::to_vec(&config).unwrap()).unwrap();
        f.c.dll_plan=mod_loader::dll_manager::load_plan_identity(&mod_loader::dll_manager::resolve_load_plan(&f.root,false));
        assert_eq!(f.c.dll_plan.as_array().unwrap().len(),1);
        assert!(validate_plan(&f.root,&f.c).unwrap_err().to_string().contains("DirectInput"));
    }
    #[test] fn runtime_plan_and_materialized_script_binding_exclude_unlisted_and_changed_source() {
        let mut f=Fixture::new(&[]);let package=f.root.join("mods/fixture");fs::create_dir_all(package.join("src")).unwrap();
        fs::write(package.join("mod.json"),br#"{"id":"fixture-runtime","name":"Fixture","version":"1.0.0","capabilities":["runtime"]}"#).unwrap();
        fs::write(package.join("extended.mod.json"),br#"{"schemaVersion":1,"enabled":true,"targets":["client"]}"#).unwrap();
        fs::write(package.join("src/mod.lua"),b"return {}\n").unwrap();
        let env=mod_loader::ModEnvironment::load(f.root.to_str().unwrap()).unwrap();
        assert!(env.mod_registry().contains_key("fixture-runtime"));
        assert_eq!(env.runtime_plan_report_detailed(false,crate::API_VERSION).0.len(),1);
        assert!(projected_runtime_plan(&env,&f.c).is_err());
        f.c.runtime_mods.push("fixture-runtime".into());
        let(bytes,sha256)=stamp(&package.join("src/mod.lua")).unwrap();f.c.scripts.insert("fixture-runtime:src/mod.lua".into(),NativeFile{bytes,sha256});
        let environment=["mod.json","extended.mod.json","src/mod.lua"].into_iter().map(|n|(format!("mods/fixture/{n}"),stamp(&package.join(n)).unwrap())).collect();
        let bindings=bind_runtime_packages(&f.root,&f.c,&environment).unwrap();
        check_script(&f.c,&bindings,"fixture-runtime",&package,"src/mod.lua",b"return {}\n").unwrap();
        assert!(check_script(&f.c,&bindings,"fixture-runtime",&package,"src/mod.lua",b"return changed\n").is_err());
        assert!(check_script(&f.c,&bindings,"fixture-runtime",&package,"src/new.lua",b"return {}\n").is_err());
        assert!(check_runtime_manifest(&bindings,"unlisted",&Value::Null).is_err());
        let mut metadata=bindings["fixture-runtime"].metadata.clone();metadata["settingValues"]=serde_json::json!({"distance":4});
        check_runtime_manifest(&bindings,"fixture-runtime",&metadata).unwrap();
        metadata["capabilities"]=serde_json::json!(["runtime-register-dll"]);assert!(check_runtime_manifest(&bindings,"fixture-runtime",&metadata).is_err());
        fs::write(package.join("extended.mod.json"),br#"{"schemaVersion":1,"enabled":false,"targets":["client"]}"#).unwrap();
        let env=mod_loader::ModEnvironment::load(f.root.to_str().unwrap()).unwrap();let mut none=f.c.clone();none.runtime_mods.clear();none.scripts.clear();
        assert!(projected_runtime_plan(&env,&none).unwrap().is_empty());
    }
}
