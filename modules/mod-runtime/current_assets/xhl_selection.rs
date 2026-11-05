//! Typed loader-owned Core selection. Native SelectedBootPlanV2 is unchanged.
use super::*;
pub const CORE:&str="mods/xhl-core/xhl-core.dll";
pub const SCAN:&str="mods/xhl-core/mods";
pub const CHAIN:&str="mods/xhl-core/dinput8-chain.ini";
#[path="xhl_pins.rs"] mod pins;
pub use pins::{CORE_SHA,GIFTING_PATH,GIFTING_SHA,GIFTING_BYTES,PLUGINS};
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct DirectoryPin {pub relative:String,pub volume:u64,pub id:[u8;16]}
impl DirectoryPin {fn native(&self)->xhl_tree::DirectoryIdentity {xhl_tree::DirectoryIdentity{relative:self.relative.clone(),volume:self.volume,id:self.id}}}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Selection {
    pub service:String,
    /// Installed container identity; it stays writable for Core's own log.
    pub container:DirectoryPin,
    /// Every scan directory, including relative "" root, has the owned seal.
    pub directories:Vec<DirectoryPin>,
    /// Exact scan-tree files including declarations, config inputs and notices.
    /// Bytes/stamps reside once in Contract.files under SCAN/name.
    pub files:Vec<String>,
}
impl Selection {
    pub fn validate_read_input(name:&str)->Result<()> {
        let key=name.to_ascii_lowercase();
        ensure!(key!="mods/xhl-core/xhl-native-plugin-host.log"&&!key.starts_with("xhl/config/"),"XHL writable output cannot be a prepared read-leased input");Ok(())
    }
    pub fn validate(&self,c:&Contract)->Result<()> {
        ensure!(c.schema_version==3&&c.host=="sf-winmm-selected-services-v3"&&self.service=="xhl-core-registration-v1","unsupported XHL loader contract");
        ensure!(self.container.relative.is_empty()&&!self.directories.is_empty()&&self.directories.len()+self.files.len()<=256,"invalid XHL tree bounds");
        ensure!(c.files.get(CORE).is_some_and(|f|f.sha256==CORE_SHA),"Core2.0.6 exact image is not bound");
        let chain_hash=format!("{:x}",Sha256::digest(xhl_tree::DISABLED_CHAIN));
        ensure!(c.files.get(CHAIN).is_some_and(|f|f.bytes==xhl_tree::DISABLED_CHAIN.len()as u64&&f.sha256==chain_hash),"Core auxiliary chain must be explicitly disabled with exact bound bytes");
        ensure!(c.files.contains_key("dinput8.dll"),"matching SF DirectInput host must be bound");
        for name in c.files.keys(){Self::validate_read_input(name)?;}
        let mut names=HashSet::new();let mut dlls=HashSet::new();
        for name in &self.files {
            ensure!(names.insert(name.to_ascii_lowercase())&&name.split('/').all(|s|!s.is_empty()&&s!="."&&s!=".."&&!s.ends_with('.')&&!s.ends_with(' '))&&!name.contains(['\\',':']),"invalid XHL member name/alias");
            ensure!(c.files.contains_key(&format!("{SCAN}/{name}")),"unbound XHL scan input");
            if name.to_ascii_lowercase().ends_with(".dll"){dlls.insert(name.as_str());}
        }
        ensure!(dlls.len()==3,"only the three selected private plugin DLLs may be scanned");
        for(path,hash)in PLUGINS {
            ensure!(dlls.contains(path)&&c.files[&format!("{SCAN}/{path}")].sha256==hash,"selected XHL plugin image differs");
            if path==GIFTING_PATH {ensure!(c.files[&format!("{SCAN}/{path}")].bytes==GIFTING_BYTES,"selected current private Gifting image size differs");}
            let parent=path.rsplit_once('/').unwrap().0;
            ensure!(self.files.contains(&format!("{parent}/native-plugin.ini")),"XHL declaration is not bound");
        }
        // No scan-tree member may be outside this namespace declaration.
        ensure!(c.files.keys().filter_map(|s|s.strip_prefix(&format!("{SCAN}/"))).all(|s|self.files.iter().any(|v|v==s)),"unlisted XHL code-tree input");
        Ok(())
    }
    pub fn validate_chain_leases(files:&[(String,fs::File)])->Result<()> {
        let chain=files.iter().find(|(name,_)|name==CHAIN).context("missing permanent Core chain input lease")?;
        let id=selected_services::file_identity(&chain.1)?;
        for(name,file)in files {ensure!(name==CHAIN||selected_services::file_identity(file)?!=id,"Core chain input has an admitted alias");}Ok(())
    }
    pub fn is_core(root:&Path,path:&Path)->bool {fs::canonicalize(root).is_ok_and(|r|path==r.join(CORE))}
    pub fn delegated(&self,root:&Path,path:&Path)->Result<bool> {
        let root=fs::canonicalize(root)?;
        let Ok(member)=path.strip_prefix(root.join(SCAN))else{return Ok(false);};
        let name=member.to_str().context("nonUTF8 XHL delegated path")?.replace('\\',"/");
        ensure!(PLUGINS.iter().any(|(p,_)|*p==name)&&self.files.contains(&name),"unexpected DLL in Core-owned subtree");Ok(true)
    }
    #[cfg(windows)]
    pub fn admit(&self,root:&Path)->Result<(Vec<fs::File>,xhl_tree::TreeLease)> {
        let container=xhl_tree::TreeLease::pin_container(&root.join("mods/xhl-core"),&self.container.native())?;
        let dirs:Vec<_>=self.directories.iter().map(DirectoryPin::native).collect();
        let lease=xhl_tree::TreeLease::validate(&root.join(SCAN),&dirs,&self.files)?;
        Ok((container,lease))
    }
}

#[cfg(test)] pub(super) mod tests {
    use super::*;
    pub(crate) fn fixture()->Selection {
        Selection{service:"xhl-core-registration-v1".into(),container:DirectoryPin{relative:String::new(),volume:1,id:[1;16]},
            directories:vec![DirectoryPin{relative:String::new(),volume:1,id:[2;16]}],
            files:PLUGINS.iter().flat_map(|(p,_)|[p.to_string(),format!("{}/native-plugin.ini",p.rsplit_once('/').unwrap().0)]).collect()}
    }
    pub(crate) fn contract_fixture()->Contract {
        let mut c=super::super::tests::contract();c.schema_version=3;c.host="sf-winmm-selected-services-v3".into();
        let x=fixture();c.selection.xhl=Some(x.clone());c.files.insert(CORE.into(),NativeFile{bytes:588800,sha256:CORE_SHA.into()});c.files.insert("dinput8.dll".into(),NativeFile{bytes:1,sha256:"a".repeat(64)});
        c.files.insert(CHAIN.into(),NativeFile{bytes:xhl_tree::DISABLED_CHAIN.len()as u64,sha256:format!("{:x}",Sha256::digest(xhl_tree::DISABLED_CHAIN))});
        for name in &x.files{c.files.insert(format!("{SCAN}/{name}"),NativeFile{bytes:if name==GIFTING_PATH{GIFTING_BYTES}else{1},sha256:PLUGINS.iter().find(|(p,_)|p==name).map(|(_,s)|s.to_string()).unwrap_or_else(||"b".repeat(64))});}
        c
    }
    #[test] fn exact_loader_contract_keeps_native_v2_and_refuses_other_images_or_members(){
        let c=contract_fixture();let x=c.selection.xhl.clone().unwrap();
        x.validate(&c).unwrap();let native=selected_services::plan(&c).unwrap();assert_eq!(native.version,2);assert_eq!(native.struct_bytes,24);
        let producer="e".repeat(64);validate_contract_pair(&c,&producer,&"b".repeat(64),&"c".repeat(64),&"a".repeat(64),None).unwrap();
        let mut bad=c.clone();bad.schema_version=2;bad.host="sf-winmm-selected-services-v2".into();assert!(x.validate(&bad).is_err());
        bad=c.clone();bad.files.get_mut(CORE).unwrap().sha256="0".repeat(64);assert!(x.validate(&bad).is_err());
        for stale in ["3f5fedc265ea2df70e0789dab8f76af838040016c80b69dff13321f7762b995c","6049751069281134704447909cb7a34e13607309c5fa09b9ba63ea3bb8509280"] {
            bad=c.clone();bad.files.get_mut(&format!("{SCAN}/{GIFTING_PATH}")).unwrap().sha256=stale.into();assert!(x.validate(&bad).is_err());
        }
        bad=c.clone();bad.files.get_mut(&format!("{SCAN}/{GIFTING_PATH}")).unwrap().bytes=GIFTING_BYTES-1;assert!(x.validate(&bad).is_err());
        bad=c.clone();bad.files.remove(CHAIN);assert!(x.validate(&bad).is_err());
        bad=c.clone();bad.files.get_mut(CHAIN).unwrap().sha256=format!("{:x}",Sha256::digest(b"[Options]\r\nEnabled=1\r\n"));assert!(x.validate(&bad).is_err());
        for output in ["mods/xhl-core/XHL-Native-Plugin-Host.log","XHL/config/xhl-remote-altar-close.json"] {
            let mut bad=c.clone();bad.files.insert(output.into(),NativeFile{bytes:1,sha256:"d".repeat(64)});assert!(x.validate(&bad).is_err());
        }
        let mut bad=x.clone();bad.files.push("injected.dll".into());assert!(bad.validate(&c).is_err());
        let tmp=std::env::temp_dir().join(format!("sf-xhl-semantic-{}",std::process::id()));fs::create_dir(&tmp).unwrap();let root=fs::canonicalize(&tmp).unwrap();
        assert!(x.delegated(&root,&root.join(SCAN).join(PLUGINS[0].0)).unwrap());
        assert!(x.delegated(&root,&root.join(SCAN).join("unknown.dll")).is_err());
        assert!(!x.delegated(&root,&root.join("mods/unrelated/XHL-Remote-Altar-Close.dll")).unwrap());fs::remove_dir(tmp).unwrap();
    }
}
