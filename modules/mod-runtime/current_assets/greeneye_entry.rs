//! Only the qualified Greeneye host uses this original-entry continuation.
//! Tickets bind retained original work. They are not a native permission API.
use super::*;
use mod_loader::dll_manager::{DllLoadEntry,DllLoadMode};
use crate::current_assets::greeneye_profiles::{self, Profile};
#[cfg(test)] const HOST_SHA:&str=greeneye_profiles::LEGACY.host_sha256;
const INPUTS:[(&str,u64,&str);4]=greeneye_profiles::LEGACY.scan_inputs;
#[repr(C)]
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
pub struct EntryTicketV1 {pub bytes:u32,pub version:u32,pub kind:u32,pub reserved:u32,pub binding:[u8;32]}
const _:()=assert!(std::mem::size_of::<EntryTicketV1>()==48);
pub(super) struct Work {
    pub entry:DllLoadEntry,pub ticket:EntryTicketV1,pub queued:bool,
    pub handle:Option<isize>,pub confirmed:bool,
    // These prevent rename/delete, not child creation. Admission assumes the
    // installer/update path is quiescent; no hostile-owner namespace guarantee.
    _directories:Vec<fs::File>,
}
impl Work {
    pub fn matches(&self,e:&DllLoadEntry)->bool {self.entry.path==e.path&&self.entry.status_id==e.status_id&&self.entry.mod_id==e.mod_id&&self.entry.package_directory==e.package_directory&&self.entry.mode==e.mode&&self.entry.source==e.source}
    pub fn retains(&self,name:&str)->bool {INPUTS.iter().any(|(p,_,_)|*p==name)}
    pub fn check(&self,ticket:&EntryTicketV1)->Result<()> {ensure!(*ticket==self.ticket,"original Greeneye continuation ticket differs");Ok(())}
}
fn validate_pins(c:&Contract)->Result<&'static Profile> {
    ensure!(c.greeneye_paths.len()==1,"exactly one qualified Greeneye host required");
    let host=&c.greeneye_paths[0];
    ensure!(host.starts_with("mods/")&&host.ends_with("/dbgcore.dll"),"Greeneye host must be an explicitly selected private Library");
    let pin=c.files.get(host).context("Greeneye host is unbound")?;
    let profile=greeneye_profiles::for_host(pin.bytes,&pin.sha256).context("Greeneye entrypoint profile is unsupported")?;
    for(path,bytes,hash)in profile.scan_inputs {let pin=c.files.get(path).context("Greeneye scan input is unbound")?;ensure!(pin.bytes==bytes&&pin.sha256==hash,"Greeneye scan input profile differs: {path}");}
    Ok(profile)
}
fn pin_directory(path:&Path)->Result<fs::File> {
    ensure!(plain(path)?.is_dir(),"Greeneye namespace is not a plain directory");
    #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt;Ok(fs::OpenOptions::new().read(true).share_mode(3).custom_flags(0x02000000).open(path)?)}
    #[cfg(not(windows))] {Ok(fs::File::open(path)?)}
}
pub(super) fn namespace(root:&Path)->Result<Vec<fs::File>> {
    ensure!(!root.join("dbgcore.dll").try_exists()?,"root Greeneye conflicts with the selected private host");
    let scan=root.join("greeneye");let mut handles=vec![pin_directory(&scan)?];
    for entry in fs::read_dir(&scan)? {
        let entry=entry?;let name=entry.file_name();
        ensure!(name=="find"||name=="map-teleport","unbound Greeneye scan entry or hotreload marker");
        handles.push(pin_directory(&entry.path())?);
    }
    for name in ["find","map-teleport"] {ensure!(plain(&scan.join(name))?.is_dir(),"required Greeneye directory missing");}
    Ok(handles)
}
#[cfg(windows)]
fn not_loaded(path:&Path)->Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let name:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();
    ensure!(unsafe{windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(name.as_ptr())}.is_null(),"selected Greeneye host was loaded before its original continuation");Ok(())
}
#[cfg(not(windows))]
fn not_loaded(_: &Path)->Result<()> {Ok(())}
pub(super) fn capture(root:&Path,c:&Contract,plan:&crate::native_dlls::IntendedNativePlan)->Result<Option<Work>> {
    if c.greeneye_paths.is_empty(){return Ok(None);}
    let profile=validate_pins(c)?;
    let path=fs::canonicalize(joined(root,&c.greeneye_paths[0])?)?;
    let entries:Vec<_>=plan.ordinary.iter().filter(|e|e.path==path).collect();
    ensure!(entries.len()==1&&entries[0].mode==DllLoadMode::Library,"Greeneye host must be original ordinary Library work");
    not_loaded(&path)?;
    let mut directories=namespace(root)?;
    // Pin the private host's ancestors; its parent remains writable for logs.
    let mut parent=path.parent();
    while let Some(p)=parent {if p==root{break;}ensure!(p.starts_with(root),"Greeneye host escapes root");directories.push(pin_directory(p)?);parent=p.parent();}
    let binding: [u8;32]=Sha256::digest(serde_json::to_vec(&serde_json::json!({"root":root,"plan":c.dll_plan,"files":c.files,"request":c.prepared_request_sha256,"receipt":c.prepared_receipt_sha256}))?).into();
    Ok(Some(Work{entry:entries[0].clone(),ticket:EntryTicketV1{bytes:48,version:1,kind:profile.ticket_kind,reserved:0,binding},queued:false,handle:None,confirmed:false,_directories:directories}))
}
pub(super) fn load(root:&Path,entry:&DllLoadEntry)->(Option<isize>,Result<()>) {
    let validate=(||{not_loaded(&entry.path)?;namespace(root)?;Ok(())})();
    if let Err(e)=validate{return(None,Err(e));}
    let(handle,result)=crate::native_dlls::load_library(root,entry);
    let result=result.map_err(anyhow::Error::msg).and_then(|()| {
        let h=handle.context("Greeneye loader returned no module")?;
        #[cfg(windows)] {
            use std::os::windows::ffi::OsStringExt;
            use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW,GetModuleHandleExW,GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,GET_MODULE_HANDLE_EX_FLAG_PIN};
            let mut name=vec![0u16;32768];let len=unsafe{GetModuleFileNameW(h as _,name.as_mut_ptr(),name.len()as u32)}as usize;
            ensure!(len>0&&len<name.len(),"Greeneye loaded module path unavailable");
            let actual=fs::canonicalize(PathBuf::from(std::ffi::OsString::from_wide(&name[..len])))?;
            ensure!(actual==entry.path,"Greeneye loader reused a different module");
            let mut pinned=std::ptr::null_mut();
            ensure!(unsafe{GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,h as *const u16,&mut pinned)}!=0&&pinned==h as _,"Greeneye module residency could not be pinned");
        }
        #[cfg(not(windows))] let _=h;
        Ok(())
    });
    // Never FreeLibrary: even a failed witness may already have published code.
    (handle,result)
}

#[cfg(test)]
pub(super) fn fixture(entry:DllLoadEntry)->Work {
    Work{entry,ticket:EntryTicketV1{bytes:48,version:1,kind:1,reserved:0,binding:[7;32]},queued:false,handle:None,confirmed:false,_directories:vec![]}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn greeneye_latest_and_legacy_sets_cannot_mix() {
        for profile in [&greeneye_profiles::LEGACY,&greeneye_profiles::LATEST] {
            let mut c=super::super::tests::contract();
            c.greeneye_paths=vec!["mods/qualified-host/dbgcore.dll".into()];
            c.files.insert(c.greeneye_paths[0].clone(),NativeFile{bytes:profile.host_bytes,sha256:profile.host_sha256.into()});
            for(p,b,h)in profile.scan_inputs{c.files.insert(p.into(),NativeFile{bytes:b,sha256:h.into()});}
            assert_eq!(validate_pins(&c).unwrap().ticket_kind,profile.ticket_kind);
            for(p,_,_)in profile.scan_inputs {
                let mut changed=c.clone();changed.files.get_mut(p).unwrap().sha256="0".repeat(64);
                assert!(validate_pins(&changed).is_err());
                let mut changed=c.clone();changed.files.get_mut(p).unwrap().bytes+=1;
                assert!(validate_pins(&changed).is_err());
            }
            let other=if profile.ticket_kind==1{&greeneye_profiles::LATEST}else{&greeneye_profiles::LEGACY};
            c.files.insert(c.greeneye_paths[0].clone(),NativeFile{bytes:other.host_bytes,sha256:other.host_sha256.into()});
            assert!(validate_pins(&c).is_err());
        }
    }
    #[test] fn greeneye_entry_namespace_refuses_alternate_scan_and_root_host() {
        let root=PathBuf::from("E:/Build/sf177-release-rebase-20261103/greeneye-entry-r1/fixtures").join(format!("namespace-{}",std::process::id()));
        for dir in ["find","map-teleport"]{fs::create_dir_all(root.join("greeneye").join(dir)).unwrap();}
        let held=namespace(&root).unwrap();
        // A directory read handle pins its identity, but does not block new children.
        fs::write(root.join("greeneye/hotreload.enable"),b"inert").unwrap();
        assert!(namespace(&root).is_err());drop(held);
        fs::remove_file(root.join("greeneye/hotreload.enable")).unwrap();
        fs::create_dir(root.join("greeneye/unbound-plugin")).unwrap();assert!(namespace(&root).is_err());
        fs::remove_dir(root.join("greeneye/unbound-plugin")).unwrap();
        fs::write(root.join("dbgcore.dll"),b"inert root host").unwrap();assert!(namespace(&root).is_err());
    }
    #[test] fn greeneye_entry_exact_profile_and_scan_inputs_only() {
        let mut c=super::super::tests::contract();c.greeneye_paths=vec!["mods/qualified-host/dbgcore.dll".into()];
        c.files.insert(c.greeneye_paths[0].clone(),NativeFile{bytes:288256,sha256:HOST_SHA.into()});
        for(p,b,h)in INPUTS{c.files.insert(p.into(),NativeFile{bytes:b,sha256:h.into()});}
        validate_pins(&c).unwrap();
        let original=c.clone();c.files.get_mut(&c.greeneye_paths[0]).unwrap().sha256="d".repeat(64);assert!(validate_pins(&c).is_err());
        let mut c=original.clone();c.files.remove(INPUTS[0].0);assert!(validate_pins(&c).is_err());
        let mut c=original;c.greeneye_paths.push(c.greeneye_paths[0].clone());assert!(validate_pins(&c).is_err());
    }
}
