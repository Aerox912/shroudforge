//! Private XHL scan-tree ownership. Directory sharing pins names; the protected
//! DACL freezes children. This is not a sandbox against an owner using WRITE_DAC.
//! No ACL is ever changed by runtime admission.
#![allow(dead_code)]
use std::{collections::BTreeSet, fs::{self, File, OpenOptions}, io, path::{Component, Path, PathBuf}};
pub const DISABLED_CHAIN:&[u8]=b"[Options]\r\nEnabled=0\r\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryIdentity { pub relative: String, pub volume: u64, pub id: [u8;16] }
pub struct TreeLease { _directories: Vec<File>, _files: Vec<File> }
#[cfg(windows)]
pub struct PinnedDirectory {file:File,identity:DirectoryIdentity}
#[cfg(windows)]
impl PinnedDirectory {
    pub fn identity(&self)->DirectoryIdentity {self.identity.clone()}
    pub fn validate(&self)->io::Result<()> {if win::identity(&self.file,String::new())?!=self.identity{return Err(io::Error::other("pinned directory identity drift"));}Ok(())}
}
#[cfg(windows)]
pub fn pin_directory_chain(path:&Path)->io::Result<Vec<PinnedDirectory>> {
    if !path.is_absolute()||path.components().any(|c|matches!(c,Component::CurDir|Component::ParentDir)){return Err(io::Error::other("plain absolute directory required"));}
    path.ancestors().map(|p|{let file=win::open_dir(p,false)?;let identity=win::identity(&file,String::new())?;Ok(PinnedDirectory{file,identity})}).collect()
}
#[cfg(windows)]
pub fn held_file_identity(file:&File)->io::Result<(u64,[u8;16])> {let id=win::identity(file,String::new())?;Ok((id.volume,id.id))}

fn relative(s: &str) -> io::Result<PathBuf> {
    if s.is_empty() || s.contains('\\') || s.contains(':') || s.contains('\0') ||
        s.split('/').any(|p|p.is_empty() || p.ends_with('.') || p.ends_with(' ')) ||
        !Path::new(s).components().all(|c|matches!(c,Component::Normal(_))) {
        return Err(io::Error::other("invalid private tree relative name"));
    }
    Ok(PathBuf::from(s))
}
fn inventory(root:&Path)->io::Result<(BTreeSet<String>,BTreeSet<String>)> {
    fn visit(root:&Path, path:&Path, dirs:&mut BTreeSet<String>,files:&mut BTreeSet<String>)->io::Result<()> {
        for item in fs::read_dir(path)? {
            let item=item?;let path=item.path();let md=fs::symlink_metadata(&path)?;
            #[cfg(windows)] {use std::os::windows::fs::MetadataExt;if md.file_attributes()&0x400!=0{return Err(io::Error::other("reparse member"));}}
            let name=path.strip_prefix(root).map_err(io::Error::other)?.to_str().ok_or_else(||io::Error::other("nonUTF8 member"))?.replace('\\',"/");
            relative(&name)?;
            if md.is_dir(){dirs.insert(name);visit(root,&path,dirs,files)?;}else if md.is_file(){files.insert(name);}else{return Err(io::Error::other("nonregular member"));}
            if dirs.len()+files.len()>256{return Err(io::Error::other("private tree is oversized"));}
        } Ok(())
    }
    let(mut dirs,mut files)=(BTreeSet::from([String::new()]),BTreeSet::new());visit(root,root,&mut dirs,&mut files)?;Ok((dirs,files))
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::{ffi::c_void, os::windows::{io::AsRawHandle,fs::OpenOptionsExt,ffi::OsStrExt}, ptr};
    type Handle=*mut c_void;
    #[repr(C)] struct FileId {volume:u64,id:[u8;16]}
    #[link(name="kernel32")] unsafe extern "system" {
        fn GetFileInformationByHandleEx(h:Handle,class:u32,data:*mut c_void,size:u32)->i32;
        fn LocalFree(p:*mut c_void)->*mut c_void;
    }
    #[link(name="advapi32")] unsafe extern "system" {
        fn GetSecurityInfo(h:Handle,kind:u32,info:u32,owner:*mut *mut c_void,group:*mut *mut c_void,dacl:*mut *mut c_void,sacl:*mut *mut c_void,sd:*mut *mut c_void)->u32;
        fn SetSecurityInfo(h:Handle,kind:u32,info:u32,owner:*mut c_void,group:*mut c_void,dacl:*mut c_void,sacl:*mut c_void)->u32;
        fn GetSecurityDescriptorControl(sd:*const c_void,control:*mut u16,revision:*mut u32)->i32;
        fn GetSecurityDescriptorDacl(sd:*const c_void,present:*mut i32,dacl:*mut *mut c_void,defaulted:*mut i32)->i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(text:*const u16,revision:u32,sd:*mut *mut c_void,size:*mut u32)->i32;
        fn OpenProcessToken(process:Handle,access:u32,token:*mut Handle)->i32;
        fn GetTokenInformation(token:Handle,class:u32,data:*mut c_void,len:u32,needed:*mut u32)->i32;
        fn ConvertSidToStringSidW(sid:*const c_void,text:*mut *mut u16)->i32;
    }
    #[link(name="kernel32")] unsafe extern "system" {fn CloseHandle(h:Handle)->i32;fn GetCurrentProcess()->Handle;}
    pub fn open_dir(path:&Path,write_dac:bool)->io::Result<File> {
        let file=OpenOptions::new().access_mode(0x20000|0x80|1|if write_dac{0x40000}else{0})
            .share_mode(3).custom_flags(0x02000000|0x00200000).open(path)?;
        use std::os::windows::fs::MetadataExt;
        let md=file.metadata()?;if !md.is_dir()||md.file_attributes()&0x400!=0{return Err(io::Error::other("not a plain directory"));} Ok(file)
    }
    pub fn identity(file:&File,relative:String)->io::Result<DirectoryIdentity> {
        let mut id=FileId{volume:0,id:[0;16]};
        if unsafe{GetFileInformationByHandleEx(file.as_raw_handle(),18,(&mut id as *mut FileId).cast(),24)}==0{return Err(io::Error::last_os_error());}
        #[cfg(test)] if super::test_faults::identity_fault(){id.id[0]^=1;}
        Ok(DirectoryIdentity{relative,volume:id.volume,id:id.id})
    }
    struct Local(*mut c_void);impl Drop for Local {fn drop(&mut self){unsafe{LocalFree(self.0);}}}
    #[derive(Clone,PartialEq,Eq)]
    pub struct Acl {pub bytes:Vec<u8>,pub protected:bool}
    fn acl_bytes(p:*const c_void)->io::Result<Vec<u8>> {
        if p.is_null(){return Err(io::Error::other("null DACL refused"));}
        let n=unsafe{ptr::read_unaligned((p as *const u8).add(2).cast::<u16>())} as usize;
        if !(8..=65535).contains(&n){return Err(io::Error::other("invalid ACL"));}
        Ok(unsafe{std::slice::from_raw_parts(p.cast::<u8>(),n)}.to_vec())
    }
    pub fn acl(file:&File)->io::Result<Acl> {
        let(mut dacl,mut sd)=(ptr::null_mut(),ptr::null_mut());
        let result=unsafe{GetSecurityInfo(file.as_raw_handle(),1,4,ptr::null_mut(),ptr::null_mut(),&mut dacl,ptr::null_mut(),&mut sd)};
        if result!=0{return Err(io::Error::from_raw_os_error(result as i32));}let _sd=Local(sd);
        let(mut control,mut revision)=(0,0);if unsafe{GetSecurityDescriptorControl(sd,&mut control,&mut revision)}==0{return Err(io::Error::last_os_error());}
        Ok(Acl{bytes:acl_bytes(dacl)?,protected:control&0x1000!=0})
    }
    pub fn set(file:&File,acl:&Acl)->io::Result<()> {
        #[cfg(test)] let fail_after=super::test_faults::before_set()?;
        set_native(file,acl)?;
        #[cfg(test)] if fail_after{return Err(io::Error::other("injected SetSecurityInfo error after write"));}
        Ok(())
    }
    // Kept separate so fault fixtures can simulate external DACL drift on their
    // own fresh directory without changing the operation's recovery record.
    pub(super) fn set_native(file:&File,acl:&Acl)->io::Result<()> {
        let r=unsafe{SetSecurityInfo(file.as_raw_handle(),1,4|if acl.protected{0x80000000}else{0x20000000},ptr::null_mut(),ptr::null_mut(),acl.bytes.as_ptr() as *mut c_void,ptr::null_mut())};
        if r!=0{return Err(io::Error::from_raw_os_error(r as i32));}Ok(())
    }
    pub fn seal_acl()->io::Result<Acl> {
        let mut token=ptr::null_mut();if unsafe{OpenProcessToken(GetCurrentProcess(),8,&mut token)}==0{return Err(io::Error::last_os_error());}
        let mut needed=0;unsafe{GetTokenInformation(token,1,ptr::null_mut(),0,&mut needed);}
        let mut data=vec![0usize;(needed as usize+std::mem::size_of::<usize>()-1)/std::mem::size_of::<usize>()];
        let ok=unsafe{GetTokenInformation(token,1,data.as_mut_ptr().cast(),needed,&mut needed)};unsafe{CloseHandle(token);}
        if ok==0{return Err(io::Error::last_os_error());}
        let sid=unsafe{ptr::read(data.as_ptr().cast::<*const c_void>())};let mut text=ptr::null_mut();
        if unsafe{ConvertSidToStringSidW(sid,&mut text)}==0{return Err(io::Error::last_os_error());}let _text=Local(text.cast());
        let mut n=0;while unsafe{*text.add(n)}!=0{n+=1;}let user=String::from_utf16(unsafe{std::slice::from_raw_parts(text,n)}).map_err(io::Error::other)?;
        // Explicit per-directory ACL, no object inheritance. Existing files are
        // protected by their own read leases, not by directory ACE inheritance.
        let sddl=format!("D:P(D;;0x46;;;{user})(A;;FA;;;{user})(A;;FA;;;SY)(A;;FA;;;BA)");
        let wide:Vec<u16>=std::ffi::OsStr::new(&sddl).encode_wide().chain(Some(0)).collect();let mut sd=ptr::null_mut();
        if unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(wide.as_ptr(),1,&mut sd,ptr::null_mut())}==0{return Err(io::Error::last_os_error());}let _sd=Local(sd);
        let(mut present,mut defaulted,mut dacl)=(0,0,ptr::null_mut());
        if unsafe{GetSecurityDescriptorDacl(sd,&mut present,&mut dacl,&mut defaulted)}==0||present==0{return Err(io::Error::last_os_error());}
        Ok(Acl{bytes:acl_bytes(dacl)?,protected:true})
    }
}

#[cfg(windows)]
impl TreeLease {
    pub fn pin_container(path:&Path,expected:&DirectoryIdentity)->io::Result<Vec<File>> {
        if !expected.relative.is_empty(){return Err(io::Error::other("container identity must be relative root"));}
        let mut files=Vec::new();for p in path.ancestors(){files.push(win::open_dir(p,false)?);}
        if win::identity(&files[0],String::new())?!=*expected{return Err(io::Error::other("Core container identity differs"));}Ok(files)
    }
    /// Runtime admission: exact identity, exact membership, exact owned DACL;
    /// never repairs an existing tree. File content stamps are checked by the
    /// enclosing full-runtime contract while these read leases remain held.
    pub fn validate(root:&Path,expected_dirs:&[DirectoryIdentity],expected_files:&[String])->io::Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        if expected_dirs.is_empty()||expected_dirs.len()+expected_files.len()>256{return Err(io::Error::other("invalid tree bound"));}
        let acl=win::seal_acl()?;let mut directories=Vec::new();let mut names=BTreeSet::new();
        for expected in expected_dirs {
            let path=if expected.relative.is_empty(){root.to_owned()}else{root.join(relative(&expected.relative)?)};
            if !names.insert(expected.relative.clone()){return Err(io::Error::other("duplicate directory"));}
            let held=win::open_dir(&path,false)?;let actual=win::identity(&held,expected.relative.clone())?;
            let actual_acl=win::acl(&held)?;
            if actual!=*expected||!actual_acl.protected||actual_acl.bytes!=acl.bytes{return Err(io::Error::other("private tree identity/seal differs"));}directories.push(held);
        }
        let expected:BTreeSet<_>=expected_files.iter().cloned().collect();
        if expected.len()!=expected_files.len(){return Err(io::Error::other("duplicate file"));}
        let mut files=Vec::new();for name in &expected {files.push(OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000).open(root.join(relative(name)?))?);}
        let(actual_dirs,actual_files)=inventory(root)?;
        let mut aliases=BTreeSet::new();
        if actual_dirs!=names||actual_files!=expected||actual_dirs.iter().chain(actual_files.iter()).any(|s|!aliases.insert(s.to_ascii_lowercase())){return Err(io::Error::other("private tree membership differs"));}
        Ok(Self{_directories:directories,_files:files})
    }
}

/// Installer-side builder, usable only for an exclusively new root. Existing
/// directories are rejected, never adopted or ACL-modified. Populate before seal.
#[cfg(windows)]
pub struct FreshTree {root:PathBuf,dirs:Vec<(String,File)>,created_ids:Vec<Option<DirectoryIdentity>>,_ancestors:Vec<File>}
#[cfg(windows)]
#[derive(Clone,Debug)]
pub struct CreationReceipt {pub root:PathBuf,pub known:Vec<DirectoryIdentity>,pub unresolved_created_paths:Vec<PathBuf>}
#[cfg(windows)]
struct CreationOwnership {tree:FreshTree,unresolved:Vec<PathBuf>}
#[cfg(windows)]
pub struct CreationFailure {pub source:io::Error,ownership:Option<Box<CreationOwnership>>}
#[cfg(windows)]
impl CreationFailure {
    fn new(source:io::Error,tree:FreshTree,unresolved:Vec<PathBuf>)->Self {Self{source,ownership:Some(Box::new(CreationOwnership{tree,unresolved}))}}
    pub fn receipt(&self)->CreationReceipt {let o=self.ownership.as_ref().unwrap();CreationReceipt{root:o.tree.root.clone(),known:o.tree.created_ids.iter().filter_map(Clone::clone).collect(),unresolved_created_paths:o.unresolved.clone()}}
    /// Creation never changed a directory ACL. Validate only original acquired
    /// handles; unresolved just-created paths are reported, never reopened/adopted.
    pub fn release_unmodified(mut self)->Result<CreationReceipt,Self> {
        if let Err(e)=self.ownership.as_ref().unwrap().tree.validate_created(){self.source=e;return Err(self);}
        let receipt=self.receipt();self.ownership.take();Ok(receipt)
    }
}
#[cfg(windows)]
impl std::fmt::Debug for CreationFailure {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct("CreationFailure").field("source",&self.source).field("created",&self.receipt()).finish()}}
#[cfg(windows)]
impl std::fmt::Display for CreationFailure {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{write!(f,"{}; partial creation ownership retained",self.source)}}
#[cfg(windows)]
impl std::error::Error for CreationFailure {}
#[cfg(windows)]
impl Drop for CreationFailure {fn drop(&mut self){if let Some(o)=self.ownership.take(){if !o.tree.dirs.is_empty()||!o.unresolved.is_empty(){let _=Box::leak(o);}}}}
#[cfg(windows)]
impl FreshTree {
    pub fn create(root:&Path,children:&[String])->Result<Self,CreationFailure> {
        let mut tree=Self{root:root.into(),dirs:Vec::new(),created_ids:Vec::new(),_ancestors:Vec::new()};
        let validated=(||{if !root.is_absolute()||root.components().any(|c|matches!(c,Component::ParentDir|Component::CurDir))||children.len()>255{return Err(io::Error::other("invalid fresh candidate bounds/path"));}
            let mut seen=BTreeSet::new();for name in children{relative(name)?;if !seen.insert(name.to_ascii_lowercase()){return Err(io::Error::other("directory alias"));}}Ok(())})();
        if let Err(e)=validated{return Err(CreationFailure::new(e,tree,vec![]));}
        for p in root.ancestors().skip(1){match win::open_dir(p,false){Ok(f)=>tree._ancestors.push(f),Err(e)=>return Err(CreationFailure::new(e,tree,vec![]))}}
        let mut names=children.to_vec();names.sort_by_key(|p|p.matches('/').count());names.insert(0,String::new());
        for name in names {
            let path=if name.is_empty(){root.into()}else{root.join(&name)};
            #[cfg(test)] if let Err(e)=test_faults::before_create(){return Err(CreationFailure::new(e,tree,vec![]));}
            if let Err(e)=fs::create_dir(&path){return Err(CreationFailure::new(e,tree,vec![]));}
            #[cfg(test)] if let Err(e)=test_faults::after_create(){return Err(CreationFailure::new(e,tree,vec![path]));}
            let file=match win::open_dir(&path,true){Ok(file)=>file,Err(e)=>return Err(CreationFailure::new(e,tree,vec![path]))};
            tree.dirs.push((name.clone(),file));tree.created_ids.push(None);
            match win::identity(&tree.dirs.last().unwrap().1,name){Ok(id)=>*tree.created_ids.last_mut().unwrap()=Some(id),Err(e)=>return Err(CreationFailure::new(e,tree,vec![path]))}
        }Ok(tree)
    }
    pub fn validate_created(&self)->io::Result<()> {
        for(i,(name,file))in self.dirs.iter().enumerate(){let actual=win::identity(file,name.clone())?;
            if self.created_ids[i].as_ref().is_some_and(|expected|*expected!=actual){return Err(io::Error::other("created directory identity drift"));}}
        Ok(())
    }
    pub fn created_receipt(&self)->io::Result<CreationReceipt> {self.validate_created()?;Ok(CreationReceipt{root:self.root.clone(),known:self.created_ids.iter().filter_map(Clone::clone).collect(),unresolved_created_paths:vec![]})}
    pub fn write_new(&self,name:&str,bytes:&[u8])->io::Result<()> {
        self.validate_created()?;
        use std::{io::Write,os::windows::fs::OpenOptionsExt};
        let mut f=OpenOptions::new().write(true).access_mode(0x40000000|0x20000|0x40000).create_new(true).open(self.root.join(relative(name)?))?;
        f.write_all(bytes)?;
        // SetSecurityInfo propagates inheritance changes to children. A fresh
        // input file receives its own protected ACL before any parent is sealed;
        // do not strip its read permissions when the scan-directory ACL changes.
        let mut original=win::acl(&f)?;original.protected=true;win::set(&f,&original)
    }
    /// Installer operation on this newly created private container only. The
    /// container itself remains unsealed so Core can write its adjacent log.
    pub fn write_disabled_chain_new(&self)->io::Result<()> {self.write_new("dinput8-chain.ini",DISABLED_CHAIN)}
    pub fn identity(&self)->io::Result<DirectoryIdentity> {win::identity(&self.dirs[0].1,String::new())}
    pub fn seal(self)->Result<SealedCandidate,RecoveryFailure> {
        let mut recovery=Recovery::new(self);
        if let Err(source)=recovery.capture(){return Err(RecoveryFailure{source,rollback_error:None,recovery});}
        if let Err(source)=recovery.install(){
            let rollback_error=recovery.restore().err();
            return Err(RecoveryFailure{source,rollback_error,recovery});
        }
        let identities=recovery.state().entries.iter().map(|e|e.identity.as_ref().unwrap().clone()).collect();
        Ok(SealedCandidate{recovery,identities})
    }
}
#[cfg(windows)]
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum DirectoryProgress {Original,SealPending,Sealed,RestorePending,Restored}
#[cfg(windows)]
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum RecoveryPhase {Capturing,Sealing,Sealed,Restoring,Complete,Installed}
#[cfg(windows)]
struct RecoveryEntry {identity:Option<DirectoryIdentity>,original:Option<win::Acl>,progress:DirectoryProgress}
#[cfg(windows)]
struct RecoveryState {tree:FreshTree,entries:Vec<RecoveryEntry>,installed:Option<win::Acl>,phase:RecoveryPhase}
#[cfg(windows)]
struct Recovery {state:Option<Box<RecoveryState>>}
#[cfg(windows)]
impl Recovery {
    fn new(tree:FreshTree)->Self {
        let entries=tree.dirs.iter().map(|_|RecoveryEntry{identity:None,original:None,progress:DirectoryProgress::Original}).collect();
        Self{state:Some(Box::new(RecoveryState{tree,entries,installed:None,phase:RecoveryPhase::Capturing}))}
    }
    fn state(&self)->&RecoveryState {self.state.as_ref().unwrap()}
    fn state_mut(&mut self)->&mut RecoveryState {self.state.as_mut().unwrap()}
    // Capture every original descriptor/ID before the first mutation. A capture
    // error retains the partial snapshots and all leases; only missing snapshots
    // may be filled on an unmodified operation's later recovery attempt.
    fn capture(&mut self)->io::Result<()> {
        let s=self.state_mut();
        for(i,(name,file))in s.tree.dirs.iter().enumerate(){
            let actual=win::identity(file,name.clone())?;
            if let Some(expected)=s.entries[i].identity.as_ref(){if actual!=*expected{return Err(io::Error::other("recovery directory identity drift"));}}
            else{s.entries[i].identity=Some(actual);}
            let acl=win::acl(file)?;
            if let Some(original)=s.entries[i].original.as_ref(){if acl!=*original{return Err(io::Error::other("unmodified recovery DACL drift"));}}
            else{s.entries[i].original=Some(acl);}
        }
        if s.installed.is_none(){s.installed=Some(win::seal_acl()?);}Ok(())
    }
    fn reconcile_one(s:&mut RecoveryState,i:usize)->io::Result<()> {
        let(name,file)=&s.tree.dirs[i];let entry=&mut s.entries[i];
        if win::identity(file,name.clone())?!=*entry.identity.as_ref().unwrap(){return Err(io::Error::other("recovery directory identity drift"));}
        let actual=win::acl(file)?;let original=entry.original.as_ref().unwrap();let installed=s.installed.as_ref().unwrap();
        entry.progress=match entry.progress {
            DirectoryProgress::Original if actual==*original=>DirectoryProgress::Original,
            DirectoryProgress::Sealed if actual==*installed=>DirectoryProgress::Sealed,
            DirectoryProgress::Restored if actual==*original=>DirectoryProgress::Restored,
            // A failed write may have applied. Only the two exact endpoints of
            // that recorded original write are admissible, never an arbitrary ACL.
            DirectoryProgress::SealPending if actual==*original=>DirectoryProgress::Original,
            DirectoryProgress::SealPending if actual==*installed=>DirectoryProgress::Sealed,
            DirectoryProgress::RestorePending if actual==*original=>DirectoryProgress::Restored,
            DirectoryProgress::RestorePending if actual==*installed=>DirectoryProgress::Sealed,
            _=>return Err(io::Error::other("unexpected recovery DACL drift")),
        };Ok(())
    }
    fn reconcile_all(&mut self)->io::Result<()> {
        let s=self.state_mut();for i in 0..s.entries.len(){Self::reconcile_one(s,i)?;}Ok(())
    }
    fn install(&mut self)->io::Result<()> {
        self.state_mut().phase=RecoveryPhase::Sealing;
        // FreshTree stores parents before children. Protect descendants first,
        // preventing a parent's ACL propagation from changing their snapshots.
        for i in (0..self.state().entries.len()).rev(){
            let s=self.state_mut();Self::reconcile_one(s,i)?;
            s.entries[i].progress=DirectoryProgress::SealPending;
            win::set(&s.tree.dirs[i].1,s.installed.as_ref().unwrap())?;
            Self::reconcile_one(s,i)?;
            if s.entries[i].progress!=DirectoryProgress::Sealed{return Err(io::Error::other("seal write did not verify"));}
        }
        self.reconcile_all()?;self.state_mut().phase=RecoveryPhase::Sealed;Ok(())
    }
    fn restore(&mut self)->io::Result<()> {
        if self.state().phase==RecoveryPhase::Capturing{self.capture()?;}
        // Even a repeated call after completion must revalidate before claiming
        // success. A refusal retains the complete record rather than dropping it.
        self.state_mut().phase=RecoveryPhase::Restoring;self.reconcile_all()?;
        // Restore parents while their descendants remain protected. Verified
        // original/restored entries are never rewritten on a retry.
        for i in 0..self.state().entries.len(){
            let s=self.state_mut();Self::reconcile_one(s,i)?;
            match s.entries[i].progress {
                DirectoryProgress::Original|DirectoryProgress::Restored=>{s.entries[i].progress=DirectoryProgress::Restored;continue;},
                DirectoryProgress::Sealed=>{},
                _=>return Err(io::Error::other("unreconciled restoration operation")),
            }
            s.entries[i].progress=DirectoryProgress::RestorePending;
            win::set(&s.tree.dirs[i].1,s.entries[i].original.as_ref().unwrap())?;
            Self::reconcile_one(s,i)?;
            if s.entries[i].progress!=DirectoryProgress::Restored{return Err(io::Error::other("original DACL write did not verify"));}
        }
        self.reconcile_all()?;
        if self.state().entries.iter().any(|e|e.progress!=DirectoryProgress::Restored){return Err(io::Error::other("restoration incomplete"));}
        self.state_mut().phase=RecoveryPhase::Complete;Ok(())
    }
    #[cfg(test)] fn drift_acl(&self,i:usize,restore:bool)->io::Result<()> {
        let s=self.state();let mut acl=s.entries[i].original.as_ref().unwrap().clone();if !restore{acl.protected=!acl.protected;}
        win::set_native(&s.tree.dirs[i].1,&acl)
    }
}
#[cfg(windows)]
impl Drop for Recovery {
    fn drop(&mut self){if let Some(record)=self.state.take(){
        if !matches!(record.phase,RecoveryPhase::Complete|RecoveryPhase::Installed) {
            // Deliberately retain one complete structured record on abandonment,
            // including descriptors and leases. No implicit cleanup or success.
            // The caller must keep its returned failure object to resume it.
            let _=Box::leak(record);
        }
    }}
}
#[cfg(windows)]
pub struct RecoveryFailure {pub source:io::Error,pub rollback_error:Option<io::Error>,recovery:Recovery}
#[cfg(windows)]
impl RecoveryFailure {
    pub fn resume_restore(&mut self)->io::Result<()> {self.recovery.restore()}
    pub fn restoration_complete(&self)->bool {self.recovery.state().phase==RecoveryPhase::Complete}
    pub fn progress(&self)->Vec<DirectoryProgress> {self.recovery.state().entries.iter().map(|e|e.progress).collect()}
    pub fn captured_identities(&self)->Vec<DirectoryIdentity> {self.recovery.state().entries.iter().filter_map(|e|e.identity.clone()).collect()}
    #[cfg(test)] pub fn test_drift_acl(&self,i:usize,restore:bool)->io::Result<()> {self.recovery.drift_acl(i,restore)}
}
#[cfg(windows)]
impl std::fmt::Debug for RecoveryFailure {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.debug_struct("RecoveryFailure").field("source",&self.source).field("rollback_error",&self.rollback_error).field("phase",&self.recovery.state().phase).field("progress",&self.progress()).finish()}
}
#[cfg(windows)]
impl std::fmt::Display for RecoveryFailure {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {write!(f,"{}; recovery ownership retained (rollback error: {:?})",self.source,self.rollback_error)}}
#[cfg(windows)]
impl std::error::Error for RecoveryFailure {fn source(&self)->Option<&(dyn std::error::Error+'static)>{Some(&self.source)}}
#[cfg(windows)]
pub struct SealedCandidate {recovery:Recovery,pub identities:Vec<DirectoryIdentity>}
#[cfg(windows)]
pub struct InstallCommitFailure {pub source:io::Error,pub candidate:SealedCandidate}
#[cfg(windows)]
impl std::fmt::Debug for InstallCommitFailure {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct("InstallCommitFailure").field("source",&self.source).finish()}}
#[cfg(windows)]
impl SealedCandidate {
    #[cfg(test)] pub fn test_set_seal(&self,i:usize,installed:bool)->io::Result<()> {let s=self.recovery.state();win::set_native(&s.tree.dirs[i].1,if installed{s.installed.as_ref().unwrap()}else{s.entries[i].original.as_ref().unwrap()})}
    /// Installer-only explicit ownership transfer after its complete transaction
    /// readback/ack. Preserve the installed ACL, release handles, never label it
    /// restored or permanently immutable. A refusal returns the original owner.
    pub fn commit_installed(mut self)->Result<Vec<DirectoryIdentity>,InstallCommitFailure> {
        let result=(||{if self.recovery.state().phase!=RecoveryPhase::Sealed{return Err(io::Error::other("candidate is not sealed"));}
            self.recovery.reconcile_all()?;
            if self.recovery.state().entries.iter().any(|e|e.progress!=DirectoryProgress::Sealed){return Err(io::Error::other("candidate seal incomplete"));}Ok(())})();
        if let Err(source)=result{return Err(InstallCommitFailure{source,candidate:self});}
        let ids=self.recovery.state().entries.iter().map(|e|e.identity.as_ref().unwrap().clone()).collect();
        self.recovery.state_mut().phase=RecoveryPhase::Installed;Ok(ids)
    }
    pub fn bind_process(self,child:std::process::Child)->RunningCandidate {RunningCandidate{seal:Some(self),child}}
    /// Only valid before a process has been attached; binding consumes this type.
    pub fn restore_unlaunched(mut self)->Result<(),RecoveryFailure> {
        match self.recovery.restore(){Ok(())=>Ok(()),Err(source)=>Err(RecoveryFailure{source,rollback_error:None,recovery:self.recovery})}
    }
}
#[cfg(windows)]
pub struct RunningCandidate {seal:Option<SealedCandidate>,child:std::process::Child}
#[cfg(windows)]
impl RunningCandidate {
    pub fn restore_when_exited(&mut self)->io::Result<()> {
        if self.child.try_wait()?.is_none(){return Err(io::Error::other("owned process is still running"));}
        self.seal.as_mut().ok_or_else(||io::Error::other("seal already retired"))?.recovery.restore()?;
        // Only a fully verified restoration may relinquish the record/leases.
        self.seal.take();Ok(())
    }
    pub fn recovery_progress(&self)->Option<Vec<DirectoryProgress>> {self.seal.as_ref().map(|s|s.recovery.state().entries.iter().map(|e|e.progress).collect())}
    #[cfg(test)] pub fn test_drift_acl(&self,i:usize,restore:bool)->io::Result<()> {self.seal.as_ref().unwrap().recovery.drift_acl(i,restore)}
}

/// Thread-local injection exists only in standalone test builds. Each injected
/// failure still exercises real ACL writes/readback on fresh fixture directories.
#[cfg(test)]
pub mod test_faults {
    use std::{cell::RefCell,io};
    #[derive(Default)] struct Faults {calls:usize,before:Vec<usize>,after:Vec<usize>,identity:bool,create_calls:usize,create_before:Option<usize>,create_after:Option<usize>}
    thread_local! {static FAULTS:RefCell<Faults>=RefCell::new(Faults::default());}
    pub fn reset(before:&[usize],after:&[usize]) {FAULTS.with(|f|*f.borrow_mut()=Faults{before:before.to_vec(),after:after.to_vec(),..Default::default()});}
    pub fn clear_failures(){FAULTS.with(|f|{let mut f=f.borrow_mut();f.before.clear();f.after.clear();f.identity=false;});}
    pub fn set_identity_fault(value:bool){FAULTS.with(|f|f.borrow_mut().identity=value);}
    pub fn calls()->usize {FAULTS.with(|f|f.borrow().calls)}
    pub fn fail_creation(before:Option<usize>,after:Option<usize>){FAULTS.with(|f|{let mut f=f.borrow_mut();f.create_calls=0;f.create_before=before;f.create_after=after;});}
    pub(super) fn before_create()->io::Result<()> {FAULTS.with(|f|{let mut f=f.borrow_mut();f.create_calls+=1;if f.create_before==Some(f.create_calls){return Err(io::Error::other("injected create error"));}Ok(())})}
    pub(super) fn after_create()->io::Result<()> {FAULTS.with(|f|{let f=f.borrow();if f.create_after==Some(f.create_calls){return Err(io::Error::other("injected post-create open error"));}Ok(())})}
    pub(super) fn identity_fault()->bool {FAULTS.with(|f|f.borrow().identity)}
    pub(super) fn before_set()->io::Result<bool> {FAULTS.with(|f|{let mut f=f.borrow_mut();f.calls+=1;let n=f.calls;
        if f.before.contains(&n){return Err(io::Error::other("injected SetSecurityInfo error before write"));}Ok(f.after.contains(&n))})}
}
#[cfg(windows)]
impl Drop for RunningCandidate {
    fn drop(&mut self){
        // Losing an installer-side record must not release directory pins while
        // its owned child is still alive. No implicit ACL restoration on Drop.
        if let Some(seal)=self.seal.take(){std::mem::forget(seal);}
    }
}
