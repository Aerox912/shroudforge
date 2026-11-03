//! Explicit schema6 admission. This is not a generator or a native permission API.
use super::*;
use serde_json::Value;
use std::sync::{Mutex,OnceLock};

#[path="selected_services.rs"]
pub mod selected_services;
#[path="xhl_selection.rs"]
pub(crate) mod xhl_selection;
#[path="xhl_tree.rs"]
mod xhl_tree;
#[path="greeneye_entry.rs"]
mod greeneye_entry;
pub use greeneye_entry::EntryTicketV1;
pub use selected_services::{SelectedBootPlanV2,SelectedDemand};
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
    pub selection:SelectedDemand,
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
enum Phase {Verified,Preparing,Prepared,Taken,NativePrepared,EntryPending,EntryTaken,EntryLoading,EntryLoaded,Committed,Running,Failed}
#[derive(Clone)]
struct PackageBinding {directory:PathBuf,metadata:Value}
struct Admission {root:PathBuf,contract:Contract,phase:Phase,packages:BTreeMap<String,PackageBinding>,
    #[cfg(windows)] _xhl:Option<(Vec<fs::File>,xhl_tree::TreeLease)>,
    intended_native:Option<crate::native_dlls::IntendedNativePlan>,greeneye:Option<greeneye_entry::Work>,entry_taken:bool,_files:Vec<(String,fs::File)>}
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
    match (c.schema_version,c.host.as_str(),c.selection.xhl.as_ref()) {
        (2,"sf-winmm-selected-services-v2",None)=>{},
        (3,"sf-winmm-selected-services-v3",Some(xhl))=>xhl.validate(c)?,
        _=>anyhow::bail!("unsupported full runtime host contract"),
    }
    ensure!(c.prepared_request_sha256==request&&c.prepared_receipt_sha256==receipt,"full runtime composition binding differs");
    selected_services::plan(c)?;
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
fn exact_metadata_binding<'a,T>(files:&'a BTreeMap<String,T>,name:&str)->Result<Option<&'a T>> {
    let mut binding=None;
    for (path,value) in files {
        if path.eq_ignore_ascii_case(name) {
            ensure!(path==name,"aliased runtime package metadata path: {name}");
            ensure!(binding.is_none(),"duplicate runtime package metadata path: {name}");
            binding=Some(value);
        }
    }
    Ok(binding)
}
fn runtime_metadata_file(directory:&Path,name:&str,required:bool)->Result<Option<PathBuf>> {
    let mut found=None;
    for entry in fs::read_dir(directory)? {
        let entry=entry?;let file_name=entry.file_name();
        if file_name.to_string_lossy().eq_ignore_ascii_case(name) {
            ensure!(file_name==std::ffi::OsStr::new(name),"aliased runtime package metadata file: {name}");
            ensure!(found.is_none(),"duplicate runtime package metadata file: {name}");
            let path=entry.path();ensure!(plain(&path)?.is_file(),"runtime package metadata is not a regular file: {name}");
            found=Some(path);
        }
    }
    ensure!(!required||found.is_some(),"runtime package metadata is missing: {name}");
    Ok(found)
}
fn bind_runtime_packages(root:&Path,c:&Contract,environment:&BTreeMap<String,Stamp>)->Result<BTreeMap<String,PackageBinding>> {
    let env=mod_loader::ModEnvironment::load(root.to_str().context("nonUTF8 runtime root")?).map_err(|e|anyhow::anyhow!("{e:?}"))?;
    let plan=projected_runtime_plan(&env,c)?;let mut bindings=BTreeMap::new();
    for package in plan {
        let directory=fs::canonicalize(package.fs().root().as_std_path())?;
        ensure!(plain(&directory)?.is_dir(),"schema6 runtime package requires a bound directory");
        let relative=directory.strip_prefix(root).context("runtime package escapes prepared root")?.to_str().context("nonUTF8 runtime package")?.replace('\\',"/");
        ensure!(relative.starts_with("mods/")&&Path::new(&relative).components().all(|part|matches!(part,Component::Normal(_))),"runtime package path is not canonical");
        let mut fully_prepared_bound=true;let mut fully_contract_bound=true;
        for (file,required) in [("mod.json",true),("extended.mod.json",false)] {
            let name=format!("{relative}/{file}");
            let path=runtime_metadata_file(&directory,file,required)?;
            let prepared=exact_metadata_binding(environment,&name)?;
            let contract=exact_metadata_binding(&c.files,&name)?;
            if let Some(path)=path {
                let actual=stamp(&path)?;
                if let Some(expected)=prepared {ensure!(*expected==actual,"prepared runtime metadata drift: {name}");}
                if let Some(expected)=contract {ensure!((expected.bytes,expected.sha256.clone())==actual,"runtime contract metadata drift: {name}");}
                if let (Some(prepared),Some(contract))=(prepared,contract) {
                    ensure!(prepared.0==contract.bytes&&prepared.1==contract.sha256,"runtime contract conflicts with prepared metadata: {name}");
                }
                fully_prepared_bound&=prepared.is_some();fully_contract_bound&=contract.is_some();
            } else {
                ensure!(prepared.is_none()&&contract.is_none(),"runtime metadata binding names a missing file: {name}");
            }
        }
        // Partial generator coverage cannot authorize metadata-only runtime admission.
        if !fully_prepared_bound {
            ensure!(fully_contract_bound,"runtime-only package metadata is not fully bound by the contract");
            ensure!(!package.info().capabilities.iter().any(|capability|capability.requires_pregame()),"runtime-only package has a pregame capability");
        }
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
        selected_services::validate_live_settings(&c,&package.info().id,&serde_json::to_value(package.info())?)?;
    }
    Ok(())
}
pub(crate) fn validate_runtime_manifest(root:&Path,id:&str,metadata:&Value)->Result<()> {
    let Some((c,bindings))=snapshot(root)?else{return Ok(());};
    check_runtime_manifest(&bindings,id,metadata)?;
    selected_services::validate_live_settings(&c,id,metadata)
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
    if c.selection.vein_selected {
        ensure!(c.files.contains_key(selected_services::VEIN_INI),"selected Vein INI identity is missing");
    }
    if selected_services::plan(c)?.services&selected_services::FLIGHT_SCALAR!=0 {
        ensure!(c.files.contains_key("shroudforge/shroudweave-native-scalar-sdk.dll"),"selected Flight scalar SDK dependency is not bound");
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
        let core=c.selection.xhl.as_ref().is_some_and(|_|xhl_selection::Selection::is_core(root,&entry.path));
        if let Some(xhl)=c.selection.xhl.as_ref(){if xhl.delegated(root,&entry.path)?{continue;}}
        ensure!(if core{entry.mode==mod_loader::dll_manager::DllLoadMode::Dinput8}else{entry.mode==mod_loader::dll_manager::DllLoadMode::Library},"schema6 DirectInput chain host is unqualified; select an approved Library route");
        ensure!(selections.iter().any(|s|s.package_directory.eq_ignore_ascii_case(&entry.package_directory)&&s.enabled&&s.approved&&s.target.applies_to(false)),"full runtime DLL lacks explicit approval");
        let path=fs::canonicalize(&entry.path)?;
        let relative=path.strip_prefix(root).context("DLL escapes full runtime root")?.to_str().context("nonUTF8 DLL")?.replace('\\',"/");
        let key=relative.to_ascii_lowercase();
        let name=path.file_name().context("missing DLL filename")?.to_string_lossy().to_ascii_lowercase();
        ensure!(!matches!(name.as_str(),"winmm.dll"|"shroudforge-runtime.dll"|"kfc-runtime.dll"|"shroudtopia.dll"),"full runtime dependency is a proxy/self/unsupported provider");
        ensure!(paths.insert(key.clone()),"duplicate canonical full runtime DLL");
        let expected=c.files.get(&relative).context("unbound full runtime DLL bytes")?;
        if !core&&c.selection.xhl.is_some(){ensure!(!xhl_selection::PLUGINS.iter().any(|(_,hash)|expected.sha256==*hash)&&expected.sha256!=xhl_selection::CORE_SHA,"XHL image selected outside its owned subtree");}
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
    if let Some(xhl)=c.selection.xhl.as_ref(){
        xhl.validate(c)?;
        ensure!(plan.entries.iter().filter(|e|xhl_selection::Selection::is_core(root,&e.path)).count()==1,"exactly one Core selection required");
    }
    Ok(())
}
pub(super) fn verify_and_stage(root:&Path,c:Contract,request:&str,receipt:&str,producer:&str,environment:&BTreeMap<String,Stamp>)->Result<()> {
    validate_contract(&c,producer,request,receipt)?;
    let mut files=verify_files(root,&c)?;
    ensure!(environment.len()<=10000,"oversized full runtime environment");
    // Preserve the verified package/configuration bytes until start_live has
    // actually read them. Only read leases are created, never asset copies.
    for(name,expected)in environment {
        if c.selection.xhl.is_some(){xhl_selection::Selection::validate_read_input(name)?;}
        let path=joined(root,name)?;ensure!(plain(&path)?.is_file(),"invalid full runtime environment member");
        let held=keep_readonly(&path)?;ensure!(&stamp(&path)?==expected,"full runtime environment drift: {name}");files.push((name.clone(),held));
    }
    validate_plan(root,&c)?;
    if c.selection.xhl.is_some(){xhl_selection::Selection::validate_chain_leases(&files)?;}
    #[cfg(windows)] let xhl=match c.selection.xhl.as_ref(){Some(xhl)=>Some(xhl.admit(root)?),None=>None};
    #[cfg(not(windows))] ensure!(c.selection.xhl.is_none(),"XHL tree admission requires Windows");
    let packages=bind_runtime_packages(root,&c,environment)?;
    let env=mod_loader::ModEnvironment::load(root.to_str().context("nonUTF8 selected root")?).map_err(|e|anyhow::anyhow!("{e:?}"))?;
    let intended_native=selected_services::validate_environment(&env,&c)?;
    if c.selection.vein_selected {
        ensure!(files.iter().filter(|(name,_)|name.eq_ignore_ascii_case(selected_services::VEIN_INI)).all(|(name,_)|name==selected_services::VEIN_INI),"Vein INI alias refused");
        selected_services::validate_ini_handles(&files)?;
    }
    let greeneye=greeneye_entry::capture(root,&c,&intended_native)?;
    let mut slot=lock();ensure!(slot.is_none(),"full runtime startup already attempted");
    *slot=Some(Admission{root:root.into(),contract:c,phase:Phase::Verified,packages,intended_native:Some(intended_native),greeneye,entry_taken:false,_files:files,#[cfg(windows)] _xhl:xhl});Ok(())
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
    let result=(|| {
        let expected=verified_intended_native_plan(root)?;
        let env=mod_loader::ModEnvironment::load(root.to_str().context("nonUTF8 selected root")?).map_err(|e|anyhow::anyhow!("{e:?}"))?;
        let current=selected_services::validate_environment(&env,&c)?;
        ensure!(expected.matches(&current),"native demand/declaration changed after full admission");
        crate::native_dlls::prepare_full_startup(root,&c).map_err(anyhow::Error::msg)
    })();
    if let Err(e)=result {abort(root);return Err(e);}
    transition(root,Phase::Preparing,Phase::Prepared)?;Ok(())
}
pub(crate) fn verified_intended_native_plan(root:&Path)->Result<crate::native_dlls::IntendedNativePlan> {
    let slot=lock();let a=slot.as_ref().context("verified native plan missing")?;
    ensure!(a.root==fs::canonicalize(root)?&&a.phase==Phase::Preparing,"verified native plan belongs to another operation");
    a.intended_native.clone().context("verified native plan binding missing")
}
pub fn take_v2(root:&Path)->Result<SelectedBootPlanV2> {
    let c=transition(root,Phase::Prepared,Phase::Taken)?;
    selected_services::plan(&c)
}
// Old schema6 handshake cannot silently authorize the version2 contract.
pub fn take(_: &Path)->u32 {u32::MAX}
pub fn commit(_: &Path,_:u32)->bool {false}
pub fn release_vein_lease(root:&Path,plan:&SelectedBootPlanV2,prepared:u32)->bool {
    let mut slot=lock();let Some(a)=slot.as_mut()else{return false;};
    let Ok(root)=fs::canonicalize(root)else{a.phase=Phase::Failed;return false;};
    release_ini(a,&root,plan,prepared)
}
fn release_ini(a:&mut Admission,root:&Path,plan:&SelectedBootPlanV2,prepared:u32)->bool {
    if root!=a.root.as_path()||a.phase!=Phase::Taken||
        selected_services::plan(&a.contract).ok().as_ref()!=Some(plan)||prepared!=plan.prepared_services(){a.phase=Phase::Failed;return false;}
    // The native prepare result covers the actual INI read/parse while these
    // handles were held. Drop only this admitted input before native activation.
    if plan.services&selected_services::VEIN!=0 {a._files.retain(|(name,_)|name!=selected_services::VEIN_INI);}
    a.phase=Phase::NativePrepared;true
}
pub fn commit_v2(root:&Path,plan:&SelectedBootPlanV2,completed:u32)->bool {
    let mut slot=lock();let Some(a)=slot.as_mut()else{return false;};
    let Ok(root)=fs::canonicalize(root)else{a.phase=Phase::Failed;return false;};
    complete_selected(a,&root,plan,completed)
}
fn complete_selected(a:&mut Admission,root:&Path,plan:&SelectedBootPlanV2,completed:u32)->bool {
    if root!=a.root.as_path()||a.phase!=Phase::NativePrepared||
        selected_services::plan(&a.contract).ok().as_ref()!=Some(plan)||completed!=plan.services {a.phase=Phase::Failed;return false;}
    if a.greeneye.as_ref().is_some_and(|g|!g.queued){a.phase=Phase::Failed;return false;}
    a.phase=if a.greeneye.is_some(){Phase::EntryPending}else{Phase::Committed};true
}
// Native queue observes and consumes only the original verified host entry.
pub(crate) fn defer_greeneye(root:&Path,entry:&mod_loader::dll_manager::DllLoadEntry)->Result<bool> {
    let mut slot=lock();let Some(a)=slot.as_mut()else{return Ok(false);};
    let Some(g)=a.greeneye.as_mut()else{return Ok(false);};
    if g.entry.path!=entry.path{return Ok(false);}
    if a.root!=fs::canonicalize(root)?||a.phase!=Phase::Preparing||g.queued||!g.matches(entry){a.phase=Phase::Failed;anyhow::bail!("Greeneye queue cannot adopt or repeat original work");}
    g.queued=true;Ok(true)
}
pub(crate) fn greeneye_status()->Option<crate::native_dlls::NativeDllStatus> {
    let slot=lock();let a=slot.as_ref()?;let g=a.greeneye.as_ref()?;if !g.queued{return None;}
    Some(crate::native_dlls::NativeDllStatus{id:g.entry.status_id.clone(),source:g.entry.source.clone(),
        state:if a.phase==Phase::Failed{"error"}else if g.confirmed{"loaded-entrypoint-hooked-scan-pending"}else{"deferred-entrypoint"},
        detail:"Qualified host entrypoint continuation; plugin scan/Start/gameplay are not witnessed".into()})
}
fn take_entry(a:&mut Admission,root:&Path)->Result<Option<EntryTicketV1>> {
    if a.root!=root||a.entry_taken||!matches!(a.phase,Phase::Committed|Phase::EntryPending){a.phase=Phase::Failed;anyhow::bail!("entry continuation take is out of sequence");}
    a.entry_taken=true;
    if let Some(g)=a.greeneye.as_ref(){ensure!(g.queued,"Greeneye original queue missing");a.phase=Phase::EntryTaken;Ok(Some(g.ticket))}else{Ok(None)}
}
pub fn take_entry_v1(root:&Path)->Result<Option<EntryTicketV1>> {
    let root=fs::canonicalize(root)?;let mut slot=lock();take_entry(slot.as_mut().context("full admission missing")?,&root)
}
fn begin_entry_load(a:&mut Admission,root:&Path,ticket:&EntryTicketV1)->Result<mod_loader::dll_manager::DllLoadEntry> {
    if a.root!=root||a.phase!=Phase::EntryTaken||a.greeneye.as_ref().is_none_or(|g|g.check(ticket).is_err()){a.phase=Phase::Failed;anyhow::bail!("original entry load operation differs");}
    a.phase=Phase::EntryLoading;Ok(a.greeneye.as_ref().unwrap().entry.clone())
}
fn finish_entry_load(a:&mut Admission,ticket:&EntryTicketV1,handle:Option<isize>,result:Result<()>)->Result<usize> {
    if let Some(g)=a.greeneye.as_mut(){g.handle=handle;} // Retain even after refusal.
    if a.phase!=Phase::EntryLoading||a.greeneye.as_ref().is_none_or(|g|g.check(ticket).is_err()){a.phase=Phase::Failed;anyhow::bail!("entry load was cancelled or repeated");}
    match result.and_then(|()|handle.filter(|h|*h!=0).context("entry load has no module")) {
        Ok(h)=>{a.phase=Phase::EntryLoaded;Ok(h as usize)},
        Err(e)=>{a.phase=Phase::Failed;Err(e)}
    }
}
pub fn load_entry_v1(root:&Path,ticket:&EntryTicketV1)->Result<usize> {
    load_entry_using(root,ticket,greeneye_entry::load)
}
fn load_entry_using(root:&Path,ticket:&EntryTicketV1,load:impl FnOnce(&Path,&mod_loader::dll_manager::DllLoadEntry)->(Option<isize>,Result<()>))->Result<usize> {
    let root=fs::canonicalize(root)?;
    let entry={let mut slot=lock();begin_entry_load(slot.as_mut().context("full admission missing")?,&root,ticket)?};
    // The native loader executes DllMain. No admission/executor mutex is held.
    let(handle,result)=load(&root,&entry);
    let mut slot=lock();finish_entry_load(slot.as_mut().context("full admission missing")?,ticket,handle,result)
}
fn confirm_entry(a:&mut Admission,root:&Path,ticket:&EntryTicketV1,handle:usize,witness:bool)->bool {
    if a.root!=root||a.phase!=Phase::EntryLoaded||!witness||a.greeneye.as_ref().is_none_or(|g|g.check(ticket).is_err()||g.handle!=Some(handle as isize)||handle==0){a.phase=Phase::Failed;return false;}
    a.greeneye.as_mut().unwrap().confirmed=true;a.phase=Phase::Committed;true
}
pub fn confirm_entry_v1(root:&Path,ticket:&EntryTicketV1,handle:usize,witness:bool)->bool {
    let Ok(root)=fs::canonicalize(root)else{return false;};let mut slot=lock();let Some(a)=slot.as_mut()else{return false;};confirm_entry(a,&root,ticket,handle,witness)
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
        retain_create_leases(a);
    }
    Ok(())
}
fn retain_create_leases(a:&mut Admission) {
    a._files.retain(|(name,_)|name.to_ascii_lowercase().ends_with(".dll")||
        (a.contract.selection.xhl.is_some()&&name==xhl_selection::CHAIN)||
        a.greeneye.as_ref().is_some_and(|g|g.retains(name)));
}
pub fn abort(_: &Path) {if let Some(a)=lock().as_mut(){a.phase=Phase::Failed;}}
#[cfg(test)]
pub(super) fn retire_fixture(){*lock()=None;}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn contract()->Contract {
        Contract{schema_version:2,host:"sf-winmm-selected-services-v2".into(),runtime_source_closure:"a".repeat(64),prepared_request_sha256:"b".repeat(64),prepared_receipt_sha256:"c".repeat(64),boot_services:vec!["metadata-init-v1".into()],overlay:false,first_person:false,selection:SelectedDemand::default(),runtime_mods:vec![],scripts:BTreeMap::new(),dll_plan:Value::Array(vec![]),files:["winmm.dll","shroudforge/shroudforge-runtime.dll","shroudforge/kfc-runtime.dll","shroudforge/config/modloader-config.json"].into_iter().map(|n|(n.into(),NativeFile{bytes:1,sha256:"d".repeat(64)})).collect(),greeneye_paths:vec![]}
    }
    #[test] fn compiled_full_pair_must_exist_and_match() {
        let mut c=contract();c.runtime_source_closure="0".repeat(64);
        assert!(validate_contract(&c,&"d".repeat(64),&"b".repeat(64),&"c".repeat(64)).is_err());
    }
    #[test] fn greeneye_entry_original_queue_restore_load_confirm_and_failure_paths() {
        use mod_loader::dll_manager::{DllLoadEntry,DllLoadMode};
        let fixture=PathBuf::from("E:/Build/sf177-release-rebase-20261103/greeneye-entry-r1/fixtures").join(format!("rust-{}",std::process::id()));
        fs::create_dir_all(fixture.join("mods/qualified-host")).unwrap();
        let root=fs::canonicalize(&fixture).unwrap();
        let path=root.join("mods/qualified-host/dbgcore.dll");fs::write(&path,b"inert fixture, never loaded").unwrap();
        let entry=DllLoadEntry{status_id:"fixture-greeneye".into(),mod_id:None,package_directory:"qualified-host".into(),path:path.canonicalize().unwrap(),mode:DllLoadMode::Library,source:"profile".into()};
        let reset=||{*lock()=Some(Admission{root:root.clone(),contract:contract(),phase:Phase::Preparing,packages:BTreeMap::new(),intended_native:None,greeneye:Some(greeneye_entry::fixture(entry.clone())),entry_taken:false,_files:vec![],#[cfg(windows)] _xhl:None});};
        // Run the real DLL Manager queue entry, not a second queue implementation.
        let cfg=root.join("shroudforge/config");fs::create_dir_all(&cfg).unwrap();
        let mut config=shroudforge_loader_config::config::read_loader_defaults().unwrap();
        config["modules"]["dllManager"]["enabled"]=true.into();
        config["modules"]["dllManager"]["profiles"]=serde_json::json!([{"id":"host","packageDirectory":"qualified-host","name":"Inert host","enabled":true,"approved":true,"target":"client","files":[{"path":"dbgcore.dll","mode":"library"}]}]);
        fs::write(cfg.join("modloader-config.json"),serde_json::to_vec(&config).unwrap()).unwrap();
        reset();crate::native_dlls::queue(&root,entry.clone()).unwrap();
        assert_eq!(crate::native_dlls::statuses().iter().find(|s|s.id==entry.status_id).unwrap().state,"deferred-entrypoint");
        {let mut slot=lock();let a=slot.as_mut().unwrap();a.phase=Phase::NativePrepared;let plan=selected_services::plan(&a.contract).unwrap();assert!(complete_selected(a,&root,&plan,plan.services));assert_eq!(a.phase,Phase::EntryPending);}
        assert!(before_create(&root).is_err()); // Early create permanently refuses.
        reset();assert!(defer_greeneye(&root,&entry).unwrap());lock().as_mut().unwrap().phase=Phase::EntryPending;
        let ticket=take_entry_v1(&root).unwrap().unwrap();
        let module=load_entry_using(&root,&ticket,|original_root,original_entry|{
            assert_eq!(original_root,root);assert_eq!(original_entry.path,entry.path);
            assert_eq!(greeneye_status().unwrap().state,"deferred-entrypoint"); // Reentrant read proves mutex is not held.
            (Some(1234),Ok(()))
        }).unwrap();
        assert!(confirm_entry_v1(&root,&ticket,module,true));assert_eq!(lock().as_ref().unwrap().phase,Phase::Committed);
        assert_eq!(greeneye_status().unwrap().state,"loaded-entrypoint-hooked-scan-pending");
        assert!(!confirm_entry_v1(&root,&ticket,module,true)); // Exactly once.
        reset();defer_greeneye(&root,&entry).unwrap();assert!(defer_greeneye(&root,&entry).is_err());
        reset();defer_greeneye(&root,&entry).unwrap();lock().as_mut().unwrap().phase=Phase::EntryPending;
        let ticket=take_entry_v1(&root).unwrap().unwrap();let mut wrong=ticket;wrong.binding[0]^=1;
        assert!(load_entry_using(&root,&wrong,|_,_|panic!("mismatched ticket must not load")).is_err());
        reset();defer_greeneye(&root,&entry).unwrap();lock().as_mut().unwrap().phase=Phase::EntryPending;
        let ticket=take_entry_v1(&root).unwrap().unwrap();load_entry_using(&root,&ticket,|_,_|(Some(1234),Ok(()))).unwrap();
        assert!(!confirm_entry_v1(&root,&ticket,1234,false));assert_eq!(lock().as_ref().unwrap().greeneye.as_ref().unwrap().handle,Some(1234));
        reset();defer_greeneye(&root,&entry).unwrap();lock().as_mut().unwrap().phase=Phase::EntryPending;
        let ticket=take_entry_v1(&root).unwrap().unwrap();
        assert!(load_entry_using(&root,&ticket,|_,_|{abort(&root);(Some(5678),Ok(()))}).is_err());
        assert_eq!(lock().as_ref().unwrap().greeneye.as_ref().unwrap().handle,Some(5678)); // No revival after reentry/cancellation.
        *lock()=None;
        // Fixture files are deliberately retained under the task evidence path.
    }
    #[test] fn exact_new_contract_accepts_only_supported_boot_requirements_and_new_pair() {
        let c=contract();let producer="e".repeat(64);
        let verify=|c:&Contract|validate_contract_pair(c,&producer,&"b".repeat(64),&"c".repeat(64),&"a".repeat(64),Some(&"f".repeat(64)));
        verify(&c).unwrap();
        for edit in 0..5 {let mut c=c.clone();match edit {0=>c.host="external-winmm".into(),1=>c.runtime_source_closure="f".repeat(64),2=>c.prepared_request_sha256="f".repeat(64),3=>c.boot_services.push("World-ready".into()),_=>c.schema_version=1};assert!(verify(&c).is_err());}
        assert!(validate_contract_pair(&c,&producer,&"b".repeat(64),&"c".repeat(64),&"a".repeat(64),Some(&producer)).is_err());
    }
    #[test] fn full_sequence_is_one_shot_and_wrong_root_cannot_refresh_it() {
        let root=PathBuf::from("E:/owned/synthetic-root");
        let mut a=Admission{root:root.clone(),contract:contract(),phase:Phase::Verified,packages:BTreeMap::new(),intended_native:None,greeneye:None,entry_taken:false,_files:vec![],#[cfg(windows)] _xhl:None};
        for (from,to) in [(Phase::Verified,Phase::Preparing),(Phase::Preparing,Phase::Prepared),(Phase::Prepared,Phase::Taken),(Phase::Taken,Phase::NativePrepared),(Phase::NativePrepared,Phase::Committed),(Phase::Committed,Phase::Running)] {move_to(&mut a,&root,from,to).unwrap();}
        assert!(move_to(&mut a,&root,Phase::Committed,Phase::Running).is_err());assert_eq!(a.phase,Phase::Failed);
        a.phase=Phase::Prepared;assert!(move_to(&mut a,&root.join("other"),Phase::Prepared,Phase::Taken).is_err());assert_eq!(a.phase,Phase::Failed);
    }
    #[test] fn selected_live_controls_refuse_unprepared_services_without_resetting_values() {
        let mut c=contract();
        let mut metadata=serde_json::json!({"settingValues":{"turnSpeed":100,"moveSpeed":100,"liftSpeed":100,"descendSpeed":100,"unlimitedFlying":false}});
        selected_services::validate_live_settings(&c,"sf-unlimited-flight",&metadata).unwrap();
        metadata["settingValues"]["moveSpeed"]=150.into();let original=metadata.clone();
        assert!(selected_services::validate_live_settings(&c,"sf-unlimited-flight",&metadata).unwrap_err().to_string().contains("unsupported"));
        assert_eq!(metadata,original); // The saved choice is not rewritten to100.
        c.boot_services.push("required-engine-hooks-v1".into());
        selected_services::validate_live_settings(&c,"sf-unlimited-flight",&metadata).unwrap();
        metadata["settingValues"]["unlimitedFlying"]=true.into();
        assert!(selected_services::validate_live_settings(&c,"sf-unlimited-flight",&metadata).is_err());
        for invalid in [Value::Null,Value::String("100".into())] {
            metadata["settingValues"]["moveSpeed"]=invalid;
            assert!(selected_services::validate_live_settings(&c,"sf-unlimited-flight",&metadata).is_err());
        }
    }
    #[test] fn selected_ini_release_is_exact_once_after_native_prepare_and_keeps_other_leases() {
        let mut f=Fixture::new(&[]);
        let ini=f.root.join(selected_services::VEIN_INI);fs::write(&ini,b"synthetic admitted existing INI").unwrap();
        let alias=f.root.join("mods/aliased-input.ini");fs::hard_link(&ini,&alias).unwrap();
        f.c.boot_services.extend(["vein-flight-provider-v1".into(),"vein-activation-v1".into()]);
        f.c.selection.vein_selected=true;f.c.selection.dedicated_guest_qualified=true;f.c.selection.allow_unverified_cursor_gate=true;
        f.c.selection.vein_settings_path=Some(selected_services::VEIN_INI.into());
        let plan=selected_services::plan(&f.c).unwrap();
        let other="shroudforge/config/modloader-config.json";
        let mut a=Admission{root:f.root.clone(),contract:f.c.clone(),phase:Phase::Taken,packages:BTreeMap::new(),intended_native:None,greeneye:None,entry_taken:false,
            #[cfg(windows)] _xhl:None,
            _files:vec![(selected_services::VEIN_INI.into(),keep_readonly(&ini).unwrap()),
                (selected_services::VEIN_INI.into(),keep_readonly(&ini).unwrap()),(other.into(),keep_readonly(&f.root.join(other)).unwrap())]};
        selected_services::validate_ini_handles(&a._files).unwrap();
        a._files.push(("mods/aliased-input.ini".into(),keep_readonly(&alias).unwrap()));
        assert!(selected_services::validate_ini_handles(&a._files).is_err());a._files.pop();
        #[cfg(windows)] assert!(fs::OpenOptions::new().write(true).open(&ini).is_err());
        assert!(!complete_selected(&mut a,&f.root,&plan,plan.services));
        a.phase=Phase::Taken;
        assert!(!release_ini(&mut a,&f.root,&plan,plan.services)); // Vein was not activated under the lease.
        assert_eq!(a._files.len(),3);a.phase=Phase::Taken;
        assert!(release_ini(&mut a,&f.root,&plan,plan.prepared_services()));
        assert_eq!(a._files.len(),1);assert_eq!(a._files[0].0,other);
        assert!(fs::OpenOptions::new().write(true).open(&ini).is_ok());
        #[cfg(windows)] assert!(fs::OpenOptions::new().write(true).open(f.root.join(other)).is_err());
        assert!(!complete_selected(&mut a,&f.root,&plan,plan.services|256));
        a.phase=Phase::NativePrepared;assert!(complete_selected(&mut a,&f.root,&plan,plan.services));
        assert!(!release_ini(&mut a,&f.root,&plan,plan.prepared_services()));
        assert_eq!(a.phase,Phase::Failed);
        let c=contract();let p=selected_services::plan(&c).unwrap();
        let mut unselected=Admission{root:f.root.clone(),contract:c,phase:Phase::Taken,packages:BTreeMap::new(),intended_native:None,greeneye:None,entry_taken:false,
            #[cfg(windows)] _xhl:None,
            _files:vec![(selected_services::VEIN_INI.into(),keep_readonly(&ini).unwrap())]};
        assert!(release_ini(&mut unselected,&f.root,&p,p.prepared_services()));
        assert_eq!(unselected._files.len(),1); // No selected Vein input, no lease drop.
    }
    #[test] fn xhl_disabled_chain_lease_survives_create_and_aliases_refuse() {
        let mut f=Fixture::new(&[]);let chain=f.root.join(xhl_selection::CHAIN);fs::create_dir_all(chain.parent().unwrap()).unwrap();fs::write(&chain,xhl_tree::DISABLED_CHAIN).unwrap();
        let alias=f.root.join("mods/alias.ini");fs::hard_link(&chain,&alias).unwrap();
        f.c.selection.xhl=Some(xhl_selection::tests::fixture());
        let mut a=Admission{root:f.root.clone(),contract:f.c.clone(),phase:Phase::Running,packages:BTreeMap::new(),intended_native:None,greeneye:None,entry_taken:false,
            #[cfg(windows)] _xhl:None,
            _files:vec![(xhl_selection::CHAIN.into(),keep_readonly(&chain).unwrap()),("mods/alias.ini".into(),keep_readonly(&alias).unwrap()),
                ("shroudforge/config/modloader-config.json".into(),keep_readonly(&f.root.join("shroudforge/config/modloader-config.json")).unwrap())]};
        assert!(xhl_selection::Selection::validate_chain_leases(&a._files).is_err());a._files.remove(1);xhl_selection::Selection::validate_chain_leases(&a._files).unwrap();
        retain_create_leases(&mut a);assert_eq!(a._files.len(),1);assert_eq!(a._files[0].0,xhl_selection::CHAIN);
        #[cfg(windows)] assert!(fs::OpenOptions::new().write(true).open(&chain).is_err());
        fs::write(f.root.join("mods/xhl-core/XHL-Native-Plugin-Host.log"),b"writable log").unwrap();
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
    fn runtime_package(f:&mut Fixture,capabilities:&[&str])->(PathBuf,BTreeMap<String,Stamp>) {
        let package=f.root.join("mods/fixture-runtime");fs::create_dir_all(package.join("src")).unwrap();
        let manifest=serde_json::json!({"id":"fixture-runtime","name":"Fixture","version":"1.0.0","capabilities":capabilities});
        fs::write(package.join("mod.json"),serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::write(package.join("extended.mod.json"),br#"{"schemaVersion":1,"enabled":true,"targets":["client"]}"#).unwrap();
        fs::write(package.join("src/mod.lua"),b"return {}\n").unwrap();
        let environment=["mod.json","extended.mod.json"].into_iter().map(|file|{
            (format!("mods/fixture-runtime/{file}"),stamp(&package.join(file)).unwrap())
        }).collect();
        let(bytes,sha256)=stamp(&package.join("src/mod.lua")).unwrap();
        f.c.runtime_mods=vec!["fixture-runtime".into()];
        f.c.scripts.insert("fixture-runtime:src/mod.lua".into(),NativeFile{bytes,sha256});
        (package,environment)
    }
    fn contract_package_metadata(f:&mut Fixture,package:&Path,files:&[&str]) {
        for file in files {
            let(bytes,sha256)=stamp(&package.join(file)).unwrap();
            f.c.files.insert(format!("mods/fixture-runtime/{file}"),NativeFile{bytes,sha256});
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
    #[test] fn runtime_package_binding_accepts_exact_contract_files_without_environment_metadata() {
        let mut f=Fixture::new(&[]);let(package,_)=runtime_package(&mut f,&["runtime"]);
        contract_package_metadata(&mut f,&package,&["mod.json","extended.mod.json"]);
        let leases=verify_files(&f.root,&f.c).unwrap();
        let bindings=bind_runtime_packages(&f.root,&f.c,&BTreeMap::new()).unwrap();
        assert_eq!(bindings["fixture-runtime"].directory,fs::canonicalize(&package).unwrap());
        check_script(&f.c,&bindings,"fixture-runtime",&package,"src/mod.lua",b"return {}\n").unwrap();
        drop(leases);
    }
    #[test] fn runtime_package_binding_rejects_missing_metadata_and_source_drift() {
        let mut missing=Fixture::new(&[]);let(package,environment)=runtime_package(&mut missing,&["runtime"]);
        contract_package_metadata(&mut missing,&package,&["mod.json"]);
        assert!(bind_runtime_packages(&missing.root,&missing.c,&BTreeMap::new()).is_err());
        assert!(bind_runtime_packages(&missing.root,&missing.c,&environment).is_ok());

        let mut drift=Fixture::new(&[]);let(package,_)=runtime_package(&mut drift,&["runtime"]);
        contract_package_metadata(&mut drift,&package,&["mod.json","extended.mod.json"]);
        fs::write(package.join("extended.mod.json"),br#"{"schemaVersion":1,"enabled":false,"targets":["client"]}"#).unwrap();
        assert!(verify_files(&drift.root,&drift.c).is_err());
        assert!(bind_runtime_packages(&drift.root,&drift.c,&BTreeMap::new()).is_err());
    }
    #[test] fn runtime_package_binding_rejects_aliases_conflicts_and_unlisted_packages() {
        let mut aliased=Fixture::new(&[]);let(package,_)=runtime_package(&mut aliased,&["runtime"]);
        contract_package_metadata(&mut aliased,&package,&["mod.json","extended.mod.json"]);
        let value=aliased.c.files.remove("mods/fixture-runtime/mod.json").unwrap();
        aliased.c.files.insert("mods/fixture-runtime/MOD.JSON".into(),value);
        assert!(bind_runtime_packages(&aliased.root,&aliased.c,&BTreeMap::new()).is_err());

        let mut prepared_alias=Fixture::new(&[]);let(package,mut environment)=runtime_package(&mut prepared_alias,&["runtime"]);
        contract_package_metadata(&mut prepared_alias,&package,&["mod.json","extended.mod.json"]);
        let value=environment.remove("mods/fixture-runtime/mod.json").unwrap();
        environment.insert("mods/fixture-runtime/MOD.JSON".into(),value);
        assert!(bind_runtime_packages(&prepared_alias.root,&prepared_alias.c,&environment).is_err());

        let mut conflict=Fixture::new(&[]);let(package,environment)=runtime_package(&mut conflict,&["runtime"]);
        contract_package_metadata(&mut conflict,&package,&["mod.json","extended.mod.json"]);
        let mut conflicting=environment.clone();let value=conflicting.get_mut("mods/fixture-runtime/mod.json").unwrap();
        value.0+=1;assert!(bind_runtime_packages(&conflict.root,&conflict.c,&conflicting).is_err());

        let mut unlisted=Fixture::new(&[]);let(_,environment)=runtime_package(&mut unlisted,&["runtime"]);
        unlisted.c.runtime_mods.clear();
        assert!(bind_runtime_packages(&unlisted.root,&unlisted.c,&environment).is_err());
    }
    #[test] fn runtime_package_binding_keeps_pregame_mixed_packages_generator_bound() {
        for capability in ["patch","export"] {
            let mut runtime_only=Fixture::new(&[]);let(package,_)=runtime_package(&mut runtime_only,&["runtime",capability]);
            contract_package_metadata(&mut runtime_only,&package,&["mod.json","extended.mod.json"]);
            assert!(bind_runtime_packages(&runtime_only.root,&runtime_only.c,&BTreeMap::new()).is_err());
        }

        let mut prepared=Fixture::new(&[]);let(_package,environment)=runtime_package(&mut prepared,&["runtime","patch"]);
        assert!(bind_runtime_packages(&prepared.root,&prepared.c,&environment).is_ok());
    }
}
