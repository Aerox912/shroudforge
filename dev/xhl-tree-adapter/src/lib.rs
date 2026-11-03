//! One private install transaction. No loading, launching, deletion or daemon.
#![cfg(windows)]
use std::{collections::{BTreeMap,BTreeSet},fs::{self,File,OpenOptions},io::{Read,Seek,SeekFrom},path::{Component,Path,PathBuf}};
use std::os::windows::fs::{OpenOptionsExt,MetadataExt};
use serde::{Serialize,Deserialize};
use sha2::{Sha256,Digest};
#[path="../../../modules/mod-runtime/current_assets/xhl_tree.rs"] pub mod tree;
#[path="../../../modules/mod-runtime/current_assets/xhl_pins.rs"] pub mod pins;
type Result<T> = std::result::Result<T,String>;
fn hash(bytes:&[u8])->String {format!("{:x}",Sha256::digest(bytes))}
fn valid_hash(s:&str)->bool {s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))}
fn relative(s:&str)->Result<PathBuf> {
    if s.is_empty()||s.len()>1024||s.split('/').count()>32||s.contains(['\\',':','\0'])||s.split('/').any(|p|p.is_empty()||p.ends_with('.')||p.ends_with(' '))||!Path::new(s).components().all(|c|matches!(c,Component::Normal(_))){return Err("invalid relative path".into());}Ok(s.into())
}
#[derive(Clone,Debug,PartialEq,Eq,PartialOrd,Ord,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Identity {pub volume:u64,pub id:[u8;16]}
impl Identity {pub fn directory(id:&tree::DirectoryIdentity)->Self {Self{volume:id.volume,id:id.id}}}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct InputFile {pub source:Option<String>,pub source_identity:Option<Identity>,pub bytes:u64,pub sha256:String}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct ApprovedPlan {
    pub schema_version:u32,pub transaction:String,
    pub source_root:PathBuf,pub source_root_identity:Identity,
    pub target_parent:PathBuf,pub target_parent_identity:Identity,
    /// Relative to the exclusively new mods/xhl-core container. The adjacent
    /// disabled chain is generated only from the canonical helper bytes.
    pub files:BTreeMap<String,InputFile>,
}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct DirectoryPin {pub relative:String,pub volume:u64,pub id:[u8;16]}
impl From<tree::DirectoryIdentity> for DirectoryPin {fn from(id:tree::DirectoryIdentity)->Self {Self{relative:id.relative,volume:id.volume,id:id.id}}}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct FilePin {pub bytes:u64,pub sha256:String,pub identity:Identity}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct PreparedReceipt {
    pub schema_version:u32,pub transaction:String,pub approved_plan_sha256:String,pub target:PathBuf,
    pub container:DirectoryPin,pub directories:Vec<DirectoryPin>,
    /// Runtime-contract names, including mods/xhl-core prefix.
    pub files:BTreeMap<String,FilePin>,
}
impl PreparedReceipt {pub fn digest(&self)->String {hash(&serde_json::to_vec(self).unwrap())}}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct CommitAck {pub transaction:String,pub approved_plan_sha256:String,pub prepared_receipt_sha256:String}
#[derive(Debug,Serialize)]
#[serde(rename_all="camelCase")]
pub struct OwnedDirectory {pub path:PathBuf,pub volume:u64,pub id:[u8;16]}
#[derive(Debug,Serialize)]
#[serde(rename_all="camelCase")]
pub struct RollbackReceipt {pub outcome:&'static str,pub target:PathBuf,pub directories:Vec<OwnedDirectory>,pub unresolved_created_paths:Vec<PathBuf>,pub unresolved_write_attempts:Vec<String>}
enum Scan {None,Fresh(tree::FreshTree),Sealed(tree::SealedCandidate),Recovery(tree::RecoveryFailure)}
#[derive(PartialEq,Eq)] enum Phase {Building,Prepared,Committed,RolledBack}
pub struct Install {
    plan:ApprovedPlan,plan_sha:String,target:PathBuf,
    parents:Vec<tree::PinnedDirectory>,sources:BTreeMap<String,File>,
    container:Option<tree::FreshTree>,scan:Scan,creation:Vec<tree::CreationFailure>,
    target_files:BTreeMap<String,File>,write_attempts:Vec<String>,prepared:Option<PreparedReceipt>,
    phase:Phase,rollback_directories:BTreeMap<PathBuf,Identity>,rollback_unresolved:Vec<PathBuf>,
    acl_started:bool,
}
pub struct PrepareFailure {pub error:String,pub owner:Option<Box<Install>>}
impl std::fmt::Debug for Install {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct("Install").field("target",&self.target).field("prepared",&self.prepared.is_some()).finish()}}
impl std::fmt::Debug for PrepareFailure {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct("PrepareFailure").field("error",&self.error).field("owner_retained",&self.owner.is_some()).finish()}}
fn open_input(path:&Path)->Result<File> {
    let f=OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000).open(path).map_err(|e|e.to_string())?;
    let md=f.metadata().map_err(|e|e.to_string())?;
    if !md.is_file()||md.file_attributes()&0x400!=0{return Err("nonregular/reparse input".into());}Ok(f)
}
fn file_identity(f:&File)->Result<Identity> {tree::held_file_identity(f).map(|(volume,id)|Identity{volume,id}).map_err(|e|e.to_string())}
fn file_stamp(f:&mut File)->Result<(u64,String)> {
    f.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;let mut digest=Sha256::new();let mut total=0u64;let mut buf=[0;65536];
    loop {let n=f.read(&mut buf).map_err(|e|e.to_string())?;if n==0{break;}total+=n as u64;if total>32*1024*1024{return Err("input exceeds bound".into());}digest.update(&buf[..n]);}
    Ok((total,format!("{:x}",digest.finalize())))
}
fn validate_plan(p:&ApprovedPlan)->Result<()> {
    if p.schema_version!=1||p.transaction.len()<16||p.transaction.len()>128||!p.transaction.bytes().all(|b|b.is_ascii_alphanumeric()||b"-_".contains(&b))||p.files.len()>128||p.files.len()<5{return Err("invalid approved plan schema/bounds".into());}
    if p.target_parent.file_name().is_none_or(|n|n!="mods"){return Err("target parent must be the explicit mods directory".into());}
    let(mut aliases,mut total)=(BTreeSet::new(),0u64);
    for(name,row)in &p.files {
        relative(name)?;if !aliases.insert(name.to_ascii_lowercase())||!valid_hash(&row.sha256)||row.bytes>32*1024*1024{return Err("invalid file map".into());}
        total=total.checked_add(row.bytes).ok_or("file bound overflow")?;
        if name.contains('/')&&!name.starts_with("mods/"){return Err("only the private scan subtree may have nested files".into());}
        if name.eq_ignore_ascii_case("XHL-Native-Plugin-Host.log"){return Err("Core log is a writable output, not an input".into());}
        if name=="dinput8-chain.ini" {
            if row.source.is_some()||row.source_identity.is_some()||row.bytes!=tree::DISABLED_CHAIN.len()as u64||row.sha256!=hash(tree::DISABLED_CHAIN){return Err("disabled chain must use the exact canonical generated bytes".into());}
        }else{relative(row.source.as_deref().ok_or("missing source path")?)?;if row.source_identity.is_none(){return Err("missing source file identity".into());}}
        if name.to_ascii_lowercase().ends_with(".dll")&&name!="xhl-core.dll"&&!pins::PLUGINS.iter().any(|(n,_)|name==&format!("mods/{n}")){return Err("unselected DLL in file map".into());}
    }
    if total>64*1024*1024{return Err("oversized approved plan".into());}
    let core=p.files.get("xhl-core.dll").ok_or("missing Core")?;if core.sha256!=pins::CORE_SHA||core.bytes!=588800{return Err("Core image differs".into());}
    if !p.files.contains_key("dinput8-chain.ini"){return Err("missing disabled chain".into());}
    for(name,sha)in pins::PLUGINS {let row=p.files.get(&format!("mods/{name}")).ok_or("missing selected plugin")?;
        if row.sha256!=sha||(name==pins::GIFTING_PATH&&row.bytes!=pins::GIFTING_BYTES){return Err("selected plugin image differs".into());}
        if !p.files.contains_key(&format!("mods/{}/native-plugin.ini",name.rsplit_once('/').unwrap().0)){return Err("missing plugin declaration".into());}
    }Ok(())
}
impl Install {
    pub fn prepare(plan_bytes:&[u8],approved_sha:&str)->std::result::Result<Self,PrepareFailure> {
        let before=(||{if plan_bytes.len()>1024*1024||!valid_hash(approved_sha)||hash(plan_bytes)!=approved_sha{return Err("approved plan digest mismatch".into());}
            let plan:ApprovedPlan=serde_json::from_slice(plan_bytes).map_err(|e|e.to_string())?;validate_plan(&plan)?;
            let mut parents=tree::pin_directory_chain(&plan.source_root).map_err(|e|e.to_string())?;
            if Identity::directory(&parents[0].identity())!=plan.source_root_identity{return Err("source root identity differs".into());}
            let target_parents=tree::pin_directory_chain(&plan.target_parent).map_err(|e|e.to_string())?;
            if Identity::directory(&target_parents[0].identity())!=plan.target_parent_identity||plan.source_root_identity==plan.target_parent_identity{return Err("target parent identity/alias differs".into());}
            let source=fs::canonicalize(&plan.source_root).map_err(|e|e.to_string())?;let target_parent=fs::canonicalize(&plan.target_parent).map_err(|e|e.to_string())?;
            if source.starts_with(&target_parent)||target_parent.starts_with(&source)||target_parents.iter().any(|p|Identity::directory(&p.identity())==plan.source_root_identity)||parents.iter().any(|p|Identity::directory(&p.identity())==plan.target_parent_identity){return Err("source/target roots overlap".into());}
            parents.extend(target_parents);let target=target_parent.join("xhl-core");
            if fs::symlink_metadata(&target).is_ok(){return Err("target already exists; ownership cannot be adopted".into());}
            let mut sources=BTreeMap::new();let mut ids=BTreeSet::new();
            for(name,row)in &plan.files {if let Some(relative_source)=&row.source{
                let path=source.join(relative_source);parents.extend(tree::pin_directory_chain(path.parent().unwrap()).map_err(|e|e.to_string())?);
                let mut file=open_input(&path)?;let id=file_identity(&file)?;
                if Some(&id)!=row.source_identity.as_ref()||!ids.insert(id){return Err("source file identity/alias differs".into());}
                if file_stamp(&mut file)?!=(row.bytes,row.sha256.clone()){return Err("source digest/size differs".into());}sources.insert(name.clone(),file);
            }}
            Ok(Self{plan,plan_sha:approved_sha.into(),target,parents,sources,container:None,scan:Scan::None,creation:vec![],target_files:BTreeMap::new(),write_attempts:vec![],prepared:None,phase:Phase::Building,rollback_directories:BTreeMap::new(),rollback_unresolved:vec![],acl_started:false})
        })();
        let mut owner=match before{Ok(owner)=>owner,Err(error)=>return Err(PrepareFailure{error,owner:None})};
        if let Err(error)=owner.populate(){return Err(PrepareFailure{error,owner:Some(Box::new(owner))});}Ok(owner)
    }
    fn populate(&mut self)->Result<()> {
        self.container=match tree::FreshTree::create(&self.target,&[]){Ok(t)=>Some(t),Err(e)=>{let text=e.to_string();self.creation.push(e);return Err(text);}};
        let mut dirs=BTreeSet::new();for name in self.plan.files.keys().filter_map(|s|s.strip_prefix("mods/")) {
            let mut parent=Path::new(name).parent();while let Some(p)=parent {if p.as_os_str().is_empty(){break;}dirs.insert(p.to_str().ok_or("nonUTF8 relative directory")?.replace('\\',"/"));parent=p.parent();}
        }
        self.scan=match tree::FreshTree::create(&self.target.join("mods"),&dirs.into_iter().collect::<Vec<_>>()){Ok(t)=>Scan::Fresh(t),Err(e)=>{let text=e.to_string();self.creation.push(e);return Err(text);}};
        for(name,row)in &self.plan.files {
            self.write_attempts.push(name.clone());
            let(target_tree,relative)=if let Some(rel)=name.strip_prefix("mods/"){let Scan::Fresh(scan)=&self.scan else{unreachable!()};(scan,rel)}else{(self.container.as_ref().unwrap(),name.as_str())};
            if name=="dinput8-chain.ini"{target_tree.write_disabled_chain_new().map_err(|e|e.to_string())?;}else{
                let source=self.sources.get_mut(name).unwrap();source.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
                let mut bytes=Vec::with_capacity(row.bytes as usize);source.read_to_end(&mut bytes).map_err(|e|e.to_string())?;
                target_tree.write_new(relative,&bytes).map_err(|e|e.to_string())?;
            }
            let file=open_input(&self.target.join(name))?;self.target_files.insert(name.clone(),file);self.write_attempts.pop();
        }
        let scan=std::mem::replace(&mut self.scan,Scan::None);let Scan::Fresh(scan)=scan else{unreachable!()};
        self.acl_started=true;
        self.scan=match scan.seal(){Ok(s)=>Scan::Sealed(s),Err(e)=>{let text=e.to_string();self.scan=Scan::Recovery(e);return Err(text);}};
        self.prepared=Some(self.readback()?);self.phase=Phase::Prepared;Ok(())
    }
    pub fn receipt(&self)->Option<&PreparedReceipt>{self.prepared.as_ref()}
    fn readback(&mut self)->Result<PreparedReceipt> {
        for parent in &self.parents{parent.validate().map_err(|e|e.to_string())?;}
        let container=self.container.as_ref().ok_or("container absent")?.identity().map_err(|e|e.to_string())?;
        let Scan::Sealed(scan)=&self.scan else{return Err("scan is not fully sealed".into());};
        let scan_files=self.plan.files.keys().filter_map(|s|s.strip_prefix("mods/").map(str::to_owned)).collect::<Vec<_>>();
        let _guard=tree::TreeLease::validate(&self.target.join("mods"),&scan.identities,&scan_files).map_err(|e|e.to_string())?;
        let allowed:BTreeSet<_>=self.plan.files.keys().filter(|n|!n.contains('/')).cloned().chain(["mods".into(),"XHL-Native-Plugin-Host.log".into()]).collect();
        for item in fs::read_dir(&self.target).map_err(|e|e.to_string())?{let item=item.map_err(|e|e.to_string())?;let name=item.file_name().into_string().map_err(|_|"nonUTF8 container member")?;
            let md=fs::symlink_metadata(item.path()).map_err(|e|e.to_string())?;
            if !allowed.contains(&name)||md.file_attributes()&0x400!=0||(name=="XHL-Native-Plugin-Host.log"&&!md.is_file()){return Err("unexpected container member/reparse".into());}}
        let mut files=BTreeMap::new();let mut ids=BTreeSet::new();
        for(name,row)in &self.plan.files {let file=self.target_files.get_mut(name).ok_or("target lease absent")?;let identity=file_identity(file)?;
            if !ids.insert(identity.clone())||file_stamp(file)?!=(row.bytes,row.sha256.clone()){return Err("target digest/identity differs".into());}
            // Verify the pathname still names the held original file.
            if file_identity(&open_input(&self.target.join(name))?)?!=identity{return Err("target path binding differs".into());}
            files.insert(format!("mods/xhl-core/{name}"),FilePin{bytes:row.bytes,sha256:row.sha256.clone(),identity});
        }
        Ok(PreparedReceipt{schema_version:1,transaction:self.plan.transaction.clone(),approved_plan_sha256:self.plan_sha.clone(),target:self.target.clone(),container:container.into(),directories:scan.identities.clone().into_iter().map(Into::into).collect(),files})
    }
    /// The caller's explicit ack follows its entire install transaction readback.
    /// No error consumes ownership; only successful validation releases leases.
    pub fn commit(&mut self,ack:&CommitAck)->Result<PreparedReceipt> {
        if self.phase!=Phase::Prepared{return Err("transaction is not prepared".into());}
        let prepared=self.prepared.clone().ok_or("no prepared receipt")?;
        if ack.transaction!=prepared.transaction||ack.approved_plan_sha256!=self.plan_sha||ack.prepared_receipt_sha256!=prepared.digest(){return Err("commit ack does not match this prepared transaction".into());}
        if self.readback()?!=prepared{return Err("prepared readback drift".into());}
        let Scan::Sealed(scan)=std::mem::replace(&mut self.scan,Scan::None)else{return Err("no sealed owner".into());};
        if let Err(failure)=scan.commit_installed(){self.scan=Scan::Sealed(failure.candidate);return Err(failure.source.to_string());}
        self.phase=Phase::Committed;self.container.take();self.target_files.clear();self.sources.clear();self.parents.clear();self.prepared=None;Ok(prepared)
    }
    fn save_created(&mut self,root:&Path,ids:Vec<tree::DirectoryIdentity>){for id in ids{self.rollback_directories.insert(root.join(&id.relative),Identity::directory(&id));}}
    pub fn rollback(&mut self)->Result<RollbackReceipt> {
        if matches!(self.phase,Phase::Committed|Phase::RolledBack){return Err("transaction already terminal".into());}
        let scan_root=self.target.join("mods");
        // Complete canonical recovery before releasing any transaction leases.
        match std::mem::replace(&mut self.scan,Scan::None){
            Scan::Sealed(scan)=>{self.save_created(&scan_root,scan.identities.clone());if let Err(e)=scan.restore_unlaunched(){let text=e.to_string();self.scan=Scan::Recovery(e);return Err(text);}},
            Scan::Recovery(mut recovery)=>{self.save_created(&scan_root,recovery.captured_identities());if let Err(e)=recovery.resume_restore(){self.scan=Scan::Recovery(recovery);return Err(e.to_string());}},
            Scan::Fresh(scan)=>{match scan.created_receipt(){Ok(receipt)=>self.save_created(&receipt.root,receipt.known),Err(e)=>{self.scan=Scan::Fresh(scan);return Err(e.to_string());}}},
            Scan::None=>{},
        }
        while let Some(failure)=self.creation.pop(){match failure.release_unmodified(){Ok(receipt)=>{self.save_created(&receipt.root,receipt.known);self.rollback_unresolved.extend(receipt.unresolved_created_paths);},Err(e)=>{let text=e.to_string();self.creation.push(e);return Err(text);}}}
        if let Some(container)=self.container.as_ref(){let receipt=container.created_receipt().map_err(|e|e.to_string())?;self.save_created(&receipt.root,receipt.known);}
        self.container.take();self.target_files.clear();self.sources.clear();self.parents.clear();self.prepared=None;
        self.phase=Phase::RolledBack;
        Ok(RollbackReceipt{outcome:if self.acl_started{"ACL_RESTORED_NEW_TREE_RETAINED"}else{"UNSEALED_NEW_PATHS_RETAINED"},target:self.target.clone(),directories:self.rollback_directories.iter().map(|(path,id)|OwnedDirectory{path:path.clone(),volume:id.volume,id:id.id}).collect(),unresolved_created_paths:self.rollback_unresolved.clone(),unresolved_write_attempts:self.write_attempts.clone()})
    }
}
impl Drop for Install {
    fn drop(&mut self){if !matches!(self.phase,Phase::Committed|Phase::RolledBack){
        // Abandonment is not an implicit commit or lease release. Keep the whole
        // original transaction record in-process; process exit/kill is explicitly
        // outside the recovery guarantee and may leave new paths for maintenance.
        let _=Box::leak(Box::new((self.plan.clone(),self.plan_sha.clone(),self.target.clone(),
            std::mem::take(&mut self.parents),std::mem::take(&mut self.sources),self.container.take(),
            std::mem::replace(&mut self.scan,Scan::None),std::mem::take(&mut self.creation),
            std::mem::take(&mut self.target_files),std::mem::take(&mut self.write_attempts),self.prepared.take(),
            std::mem::take(&mut self.rollback_directories),std::mem::take(&mut self.rollback_unresolved))));
    }}
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {base:PathBuf,plan:ApprovedPlan}
    impl Fixture {
        fn new()->Self {
            tree::test_faults::reset(&[],&[]);
            let base_root=PathBuf::from(std::env::var_os("SF_XHL_ADAPTER_FIXTURE_ROOT").expect("explicit E fixture root required"));
            assert!(base_root.is_absolute()&&base_root.to_string_lossy().starts_with("E:\\Scratch\\sf177-xhl-adapter-"));
            let base=base_root.join(format!("case-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
            fs::create_dir_all(base.join("source")).unwrap();fs::create_dir_all(base.join("game/mods")).unwrap();
            let source=fs::canonicalize(base.join("source")).unwrap();let parent=fs::canonicalize(base.join("game/mods")).unwrap();
            let mut files=BTreeMap::new();
            let inputs=[("xhl-core.dll",r"E:\Scratch\sf177-xhl-core-probe-20261103\static\dinput8.dll"),
                ("mods/XHL-Remote-Altar-Close/XHL-Remote-Altar-Close.dll",r"E:\Scratch\sf177-xhl-core-probe-20261103\cases\rb44-real-plugins\game\core\mods\XHL-Remote-Altar-Close\XHL-Remote-Altar-Close.dll"),
                ("mods/XHL-Grappling-Hook-Pull-2x/XHL-Grappling-Hook-Pull-2x.dll",r"E:\Scratch\sf177-xhl-core-probe-20261103\cases\rb44-real-plugins\game\core\mods\XHL-Grappling-Hook-Pull-2x\XHL-Grappling-Hook-Pull-2x.dll"),
                ("mods/XHL-Unlimited-Gifting-Range/XHL-Unlimited-Gifting-Range.dll",r"E:\Build\gifting-current-test-20261103\Release\XHL-Unlimited-Gifting-Range.dll")];
            for(name,input)in inputs {let path=source.join(name);fs::create_dir_all(path.parent().unwrap()).unwrap();fs::copy(input,&path).unwrap();}
            for(name,_)in pins::PLUGINS {let ini=format!("mods/{}/native-plugin.ini",name.rsplit_once('/').unwrap().0);fs::write(source.join(&ini),format!("[Plugin]\r\nEnabled=true\r\nDll={}\r\n",Path::new(name).file_name().unwrap().to_str().unwrap())).unwrap();}
            for item in inputs.iter().map(|(n,_)|n.to_string()).chain(pins::PLUGINS.iter().map(|(n,_)|format!("mods/{}/native-plugin.ini",n.rsplit_once('/').unwrap().0))){
                let mut f=open_input(&source.join(&item)).unwrap();let(bytes,sha256)=file_stamp(&mut f).unwrap();files.insert(item.clone(),InputFile{source:Some(item),source_identity:Some(file_identity(&f).unwrap()),bytes,sha256});
            }
            files.insert("dinput8-chain.ini".into(),InputFile{source:None,source_identity:None,bytes:tree::DISABLED_CHAIN.len()as u64,sha256:hash(tree::DISABLED_CHAIN)});
            let plan=ApprovedPlan{schema_version:1,transaction:"rb65-inert-transaction".into(),source_root_identity:Identity::directory(&tree::pin_directory_chain(&source).unwrap()[0].identity()),target_parent_identity:Identity::directory(&tree::pin_directory_chain(&parent).unwrap()[0].identity()),source_root:source,target_parent:parent,files};
            Self{base,plan}
        }
        fn bytes(&self)->Vec<u8>{serde_json::to_vec(&self.plan).unwrap()}
        fn prepare(&self)->Install{let bytes=self.bytes();Install::prepare(&bytes,&hash(&bytes)).unwrap()}
        fn target(&self)->PathBuf{self.plan.target_parent.join("xhl-core")}
    }
    impl Drop for Fixture {fn drop(&mut self){tree::test_faults::reset(&[],&[]);let _=fs::remove_dir_all(&self.base);}}
    fn ack(receipt:&PreparedReceipt)->CommitAck{CommitAck{transaction:receipt.transaction.clone(),approved_plan_sha256:receipt.approved_plan_sha256.clone(),prepared_receipt_sha256:receipt.digest()}}
    #[test] fn prepared_ids_ack_commit_and_runtime_drift_boundary(){
        let f=Fixture::new();let mut install=f.prepare();let receipt=install.receipt().unwrap().clone();assert_eq!(receipt.files.len(),8);assert_eq!(receipt.directories.len(),4);
        assert_eq!(receipt.container.id,tree::pin_directory_chain(&f.target()).unwrap()[0].identity().id);
        assert!(fs::write(f.target().join("xhl-core.dll"),b"blocked until ack").is_err());
        assert!(fs::write(f.plan.source_root.join("xhl-core.dll"),b"source pinned").is_err());
        fs::write(f.target().join("XHL-Native-Plugin-Host.log"),b"parent remains writable").unwrap();
        let mut bad=ack(&receipt);bad.transaction="different-transaction".into();assert!(install.commit(&bad).is_err());assert!(install.receipt().is_some());
        tree::test_faults::set_identity_fault(true);assert!(install.commit(&ack(&receipt)).is_err());tree::test_faults::set_identity_fault(false);assert!(install.receipt().is_some());
        if let Scan::Sealed(scan)=&install.scan {scan.test_set_seal(0,false).unwrap();}else{panic!("owner lost");}
        assert!(install.commit(&ack(&receipt)).is_err());
        if let Scan::Sealed(scan)=&install.scan {scan.test_set_seal(0,true).unwrap();}
        assert_eq!(install.commit(&ack(&receipt)).unwrap(),receipt);assert!(install.rollback().is_err());
        assert!(fs::write(f.target().join("mods/injected.dll"),b"child creation still sealed").is_err());
        let plugin=format!("mods/{}",pins::GIFTING_PATH);fs::write(f.target().join(&plugin),b"postcommit drift possible").unwrap();
        assert_ne!(hash(&fs::read(f.target().join(&plugin)).unwrap()),receipt.files[&format!("mods/xhl-core/{plugin}")].sha256);
    }
    #[test] fn rollback_restores_acl_but_explicitly_retains_new_tree(){
        let f=Fixture::new();let mut install=f.prepare();let receipt=install.rollback().unwrap();
        assert_eq!(receipt.outcome,"ACL_RESTORED_NEW_TREE_RETAINED");assert!(receipt.unresolved_created_paths.is_empty());assert_eq!(receipt.directories.len(),5);
        assert!(f.target().is_dir());fs::write(f.target().join("mods/maintenance"),b"restored").unwrap();fs::write(f.target().join("xhl-core.dll"),b"leases released").unwrap();assert!(install.rollback().is_err());
    }
    #[test] fn partial_creation_failure_keeps_owner_and_reports_unresolved_without_adoption(){
        let f=Fixture::new();tree::test_faults::fail_creation(None,Some(2));let bytes=f.bytes();let failure=Install::prepare(&bytes,&hash(&bytes)).unwrap_err();
        let mut owner=failure.owner.expect("partial owner");assert!(owner.receipt().is_none());
        fs::write(f.target().join("mods/sentinel"),b"do not modify or adopt").unwrap();tree::test_faults::reset(&[],&[]);
        let receipt=owner.rollback().unwrap();assert_eq!(receipt.outcome,"UNSEALED_NEW_PATHS_RETAINED");assert_eq!(receipt.unresolved_created_paths,vec![f.target().join("mods")]);
        assert_eq!(fs::read(f.target().join("mods/sentinel")).unwrap(),b"do not modify or adopt");
    }
    #[test] fn digest_parent_source_alias_and_existing_target_refuse_before_writes(){
        let mut f=Fixture::new();let bytes=f.bytes();assert!(Install::prepare(&bytes,&"0".repeat(64)).unwrap_err().owner.is_none());assert!(!f.target().exists());
        f.plan.target_parent_identity.id[0]^=1;let bytes=f.bytes();assert!(Install::prepare(&bytes,&hash(&bytes)).unwrap_err().owner.is_none());f.plan.target_parent_identity.id[0]^=1;
        let source=f.plan.files["mods/XHL-Remote-Altar-Close/native-plugin.ini"].clone();let duplicate=f.plan.files.get_mut("mods/XHL-Grappling-Hook-Pull-2x/native-plugin.ini").unwrap();*duplicate=source;
        let bytes=f.bytes();assert!(Install::prepare(&bytes,&hash(&bytes)).unwrap_err().owner.is_none());assert!(!f.target().exists());
        fs::create_dir(f.target()).unwrap();fs::write(f.target().join("sentinel"),b"existing ownership").unwrap();let bytes=f.bytes();assert!(Install::prepare(&bytes,&hash(&bytes)).unwrap_err().owner.is_none());assert_eq!(fs::read(f.target().join("sentinel")).unwrap(),b"existing ownership");
    }
}
