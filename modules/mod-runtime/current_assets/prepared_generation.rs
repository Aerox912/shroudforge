//! Admission for the exact isolated preparation worker, not a new writer.
//! Original schema3/4 receipts and their producer identities remain unchanged.
use super::*;

#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedRequest {
    schema:u32,target_root:PathBuf,asset_input_root:PathBuf,
    environment_root:PathBuf,output_root:PathBuf,selected_ids:Vec<String>,
    inputs:BTreeMap<String,FileStamp>,environment_files:BTreeMap<String,FileStamp>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    external_registry:Option<crate::ExternalRegistryInput>,
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedReceipt {
    schema:u32,source_closure:String,request_sha256:String,target_root:PathBuf,
    #[serde(default,skip_serializing_if="Option::is_none")]
    external_registry:Option<crate::ExternalRegistryInput>,
    output_stage:PathBuf,staged:BTreeMap<String,FileStamp>,prepared_marker:FileStamp,
    #[serde(default,skip_serializing_if="Option::is_none")]
    source_dat_membership:Option<BTreeMap<String,FileStamp>>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    source_dat_membership_sha256:Option<String>,
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct PreparedPolicy {
    schema_version:u32,owner:String,root:PathBuf,source_closure:String,
    request_json:String,request_sha256:String,receipt_json:String,receipt_sha256:String,
    extended_zoom:bool,preserved_loaders:BTreeMap<String,String>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    runtime:Option<super::super::full_runtime::Contract>,
}

fn path_key(path:&Path)->Result<String> {
    use std::path::Component;
    ensure!(path.is_absolute()&&!path.components().any(|c|matches!(c,Component::CurDir|Component::ParentDir)),"noncanonical prepared provenance path");
    let text=path.to_str().context("nonUTF8 preparation path")?;
    let text=text.strip_prefix("\\\\?\\").unwrap_or(text).replace('\\',"/").to_ascii_lowercase();
    ensure!(text.len()>=3&&text.as_bytes()[0].is_ascii_alphabetic()&&&text[1..3]==":/","native drive path required");
    Ok(text.trim_end_matches('/').into())
}
fn overlaps(a:&str,b:&str)->bool {a==b||a.starts_with(&format!("{b}/"))||b.starts_with(&format!("{a}/"))}
fn same(a:&FileStamp,b:&FileStamp)->bool {a.bytes==b.bytes&&a.sha256==b.sha256}
fn is_dat_asset(name:&str)->bool {let lower=name.to_ascii_lowercase();lower.ends_with(".dat")&&asset(&lower)}
fn valid_selected_mod_id(id:&str)->bool {
    mod_loader::config::valid_id(id)
        && !id.to_ascii_lowercase().replace(['-', '_', '.'], "").contains("autoloot")
}

fn validate_receipt_schema<'a>(request:&PreparedRequest,receipt:&'a PreparedReceipt,full:bool)->Result<Option<&'a BTreeMap<String,FileStamp>>> {
    let external=request.external_registry.is_some();
    ensure!(full||!external,"external registry requires full prepared provenance");
    ensure!(request.external_registry==receipt.external_registry,"prepared registry provenance differs");
    let expected_schema=if external {3}else if full {2}else{1};
    ensure!((request.schema==if external{2}else{1})&&receipt.schema==expected_schema,"prepared request/receipt schema differs");
    if !full {
        ensure!(receipt.source_dat_membership.is_none()&&receipt.source_dat_membership_sha256.is_none(),"legacy prepared receipt contains schema2 DAT membership");
        return Ok(None);
    }
    let membership=receipt.source_dat_membership.as_ref().context("schema2 prepared DAT membership missing")?;
    let membership_sha256=receipt.source_dat_membership_sha256.as_deref().context("schema2 prepared DAT membership digest missing")?;
    ensure!(membership.len()<=10000&&valid_digest(membership_sha256)&&digest(&serde_json::to_vec(membership)?)==membership_sha256,"schema2 prepared DAT membership digest differs");
    let mut request_dat=BTreeMap::new();
    for (name,value) in &request.inputs {
        if is_dat_asset(name) {
            ensure!(name==&name.to_ascii_lowercase(),"noncanonical source DAT input");
            request_dat.insert(name.clone(),value);
        }
    }
    ensure!(request_dat.len()==membership.len(),"schema2 source DAT membership/input count differs");
    for (name,value) in membership {
        ensure!(name==&name.to_ascii_lowercase()&&is_dat_asset(name)&&valid_digest(&value.sha256),"invalid schema2 source DAT identity");
        ensure!(request_dat.get(name).is_some_and(|source|same(*source,value)),"schema2 source DAT identity differs from request");
    }
    Ok(Some(membership))
}

fn validate_schema2_dat_union(actual_assets:&BTreeSet<String>,source:&BTreeMap<String,FileStamp>,staged:&BTreeMap<String,FileStamp>)->Result<()> {
    let mut expected:BTreeSet<_>=source.keys().cloned().collect();
    for name in staged.keys().filter(|name|is_dat_asset(name)) {
        ensure!(!source.contains_key(name),"schema2 generated DAT overlaps source membership");
        expected.insert(name.clone());
    }
    let actual:BTreeSet<_>=actual_assets.iter().filter(|name|is_dat_asset(name)).cloned().collect();
    ensure!(actual==expected,"schema2 installed DAT union differs");
    Ok(())
}

pub(super) fn verify(root:&Path)->Result<()> {
    verify_mode(root,false)
}
pub(super) fn verify_full(root:&Path)->Result<()> {
    verify_mode(root,true)
}
fn verify_mode(root:&Path,full:bool)->Result<()> {
    // Optional at compile time; absence leaves only this new mode closed.
    let producer=if full {option_env!("SF_FULL_PREPARED_SOURCE_CLOSURE")}else{option_env!("HD_PREPARED_SOURCE_CLOSURE")}.context("prepared producer not compiled into runtime")?;
    ensure!(valid_digest(producer),"invalid compiled preparation identity");
    for parent in root.ancestors(){ensure!(plain(parent)?.is_dir(),"invalid prepared target root");}
    let root=fs::canonicalize(root)?;
    let _outer=manager_lease(&root)?;let _startup=lease(&root)?;
    ensure!(!transaction::directory(&root,STEM).exists(),"asset publication remains pending");
    for name in [STATE,PENDING] {ensure!(!joined(&root,name)?.exists(),"conflicting native asset owner");}
    let policy_document:serde_json::Value=read_json(&joined(&root,POLICY)?)?;
    ensure!(full||policy_document.get("runtime").is_none(),"historical prepared policy cannot contain a runtime field");
    let policy:PreparedPolicy=serde_json::from_value(policy_document)?;
    ensure!(policy.schema_version==if full {6}else{5},"prepared startup schema differs");
    ensure!(policy.owner==if full {"shroudforge-full-prepared"}else{"hdmap-prepared-generator"},"prepared startup owner differs");
    ensure!(policy.source_closure==producer&&fs::canonicalize(&policy.root)?==root,"prepared startup producer/root differs");
    ensure!(full==policy.runtime.is_some(),"runtime contract is exclusive to schema6");
    ensure!(valid_digest(&policy.request_sha256)&&digest(policy.request_json.as_bytes())==policy.request_sha256&&valid_digest(&policy.receipt_sha256)&&digest(policy.receipt_json.as_bytes())==policy.receipt_sha256,"prepared evidence digest differs");
    let request:PreparedRequest=serde_json::from_str(&policy.request_json)?;
    let receipt:PreparedReceipt=serde_json::from_str(&policy.receipt_json)?;
    ensure!(receipt.source_closure==producer&&receipt.request_sha256==digest(&serde_json::to_vec(&request)?),"prepared receipt producer/request differs");
    let source_dat_membership=validate_receipt_schema(&request,&receipt,full)?;
    ensure!(fs::canonicalize(&request.target_root)?==root&&fs::canonicalize(&request.asset_input_root)?==root&&receipt.target_root==root,"prepared receipt retargeting rejected");
    let source=path_key(&root)?;let environment=path_key(&request.environment_root)?;let output=path_key(&request.output_root)?;
    ensure!(environment.starts_with("e:/")&&output.starts_with("e:/")&&!overlaps(&source,&environment)&&!overlaps(&source,&output)&&!overlaps(&environment,&output),"prepared root ownership differs");
    ensure!(path_key(&receipt.output_stage)?==format!("{output}/.enshrouded.shroudforge-stage"),"prepared stage binding differs");
    ensure!(valid_digest(&receipt.prepared_marker.sha256)&&receipt.prepared_marker.bytes>0,"prepared marker identity missing");
    ensure!(!request.inputs.is_empty()&&request.inputs.len()<=10000&&request.environment_files.len()<=10000&&receipt.staged.len()<=512,"oversized prepared composition");
    let ids:BTreeSet<_>=request.selected_ids.iter().cloned().collect();
    ensure!(!ids.is_empty()&&ids.len()==request.selected_ids.len(),"invalid prepared selection");
    for id in &ids {ensure!(valid_selected_mod_id(id),"unqualified prepared selection");}
    for name in ["enshrouded.exe","enshrouded.kfc","enshrouded.kfc_resources"] {ensure!(request.inputs.contains_key(name),"incomplete prepared baseline");}
    for name in ["enshrouded.kfc","enshrouded.kfc_resources","generated-inputs.json"] {ensure!(receipt.staged.contains_key(name),"incomplete prepared output");}
    let mut aliases=BTreeSet::new();
    for (name,value) in &request.environment_files {
        checked(&root,name)?;
        ensure!(aliases.insert(name.to_ascii_lowercase())&&valid_digest(&value.sha256),"invalid prepared environment identity");
        if name.starts_with("mods/") {ensure!(request.inputs.get(name).is_some_and(|v|same(v,value)),"unbound prepared package");}
    }
    aliases.clear();
    for (name,value) in &request.inputs {
        ensure!(aliases.insert(name.to_ascii_lowercase())&&valid_digest(&value.sha256),"invalid prepared input");
        if !(asset(name)||name=="enshrouded.exe") {ensure!(name.starts_with("mods/")&&request.environment_files.get(name).is_some_and(|v|same(v,value)),"prepared package input differs");}
        let path=checked(&root,name)?;
        if asset(name)&&name.ends_with(".dat") {
            // Deployment preflight verifies schema2 source DAT names and exact stamps before commit.
            // Startup retains the legacy presence/size check for these large files.
            ensure!(plain(&path)?.is_file()&&fs::metadata(path)?.len()==value.bytes,"baseline DAT membership/size differs");
        }
        else if !receipt.staged.contains_key(name) {exact(&root,name,value)?;}
    }
    aliases.clear();
    for (name,value) in &receipt.staged {
        ensure!(aliases.insert(name.to_ascii_lowercase())&&valid_digest(&value.sha256)&&(asset(name)||name=="generated-inputs.json"),"invalid prepared output member");
        if asset(name) {
            if name.ends_with(".dat") {
                ensure!(!request.inputs.contains_key(name),"prepared DAT overlaps baseline");
                if let Some(source)=source_dat_membership {ensure!(!source.contains_key(name),"prepared DAT overlaps schema2 source membership");}
            }
            exact(&root,name,value)?;
        }
    }
    let expected:BTreeSet<_>=request.inputs.keys().chain(receipt.staged.keys()).filter(|n|asset(n)).cloned().collect();
    let mut actual=BTreeSet::new();
    for entry in fs::read_dir(&root)? {let name=entry?.file_name().into_string().map_err(|_|anyhow::anyhow!("nonUTF8 target asset"))?;if asset(&name.to_ascii_lowercase()){ensure!(name==name.to_ascii_lowercase(),"target asset alias");actual.insert(name);}}
    ensure!(actual==expected,"prepared asset membership differs");
    if let Some(source)=source_dat_membership {validate_schema2_dat_union(&actual,source,&receipt.staged)?;}
    ensure!(policy.preserved_loaders.keys().all(|n|matches!(n.as_str(),"winmm.dll"|"dbgcore.dll")),"prepared loader contract differs");
    for (name,hash) in &policy.preserved_loaders {ensure!(valid_digest(hash)&&stamp(&checked(&root,name)?)?.1==*hash,"prepared loader drift");}
    if ids.contains("esm-real-map") {
        ensure!((full||policy.preserved_loaders.contains_key("winmm.dll"))&&request.inputs.contains_key(MAP_EXTENSION),"prepared HD dependency missing");
        let extension:serde_json::Value=read_json(&checked(&root,MAP_EXTENSION)?)?;
        let configured=extension.pointer("/settings/ExtendedZoom/value").and_then(serde_json::Value::as_bool).or_else(||extension.pointer("/settings/ExtendedZoom").and_then(serde_json::Value::as_bool)).context("missing prepared ExtendedZoom setting")?;
        ensure!(extension["enabled"].as_bool()==Some(true)&&configured==policy.extended_zoom,"prepared HD settings differ");
    } else {ensure!(!policy.extended_zoom,"prepared zoom requires HD selection");}
    if let Some(contract)=policy.runtime {
        ensure!(policy.preserved_loaders.is_empty(),"full runtime cannot activate implicit preserved loaders");
        let environment=request.environment_files.iter().map(|(name,value)|(name.clone(),(value.bytes,value.sha256.clone()))).collect();
        super::super::full_runtime::verify_and_stage(&root,contract,&policy.request_sha256,&policy.receipt_sha256,producer,&environment)?;
    }
    zoom_demand::publish(&root,policy.extended_zoom,&request.inputs["enshrouded.exe"].sha256);
    let mask=u32::from(policy.preserved_loaders.contains_key("winmm.dll"))|(u32::from(policy.preserved_loaders.contains_key("dbgcore.dll"))<<1);
    LOADERS.with(|slot|*slot.borrow_mut()=Some((root,mask)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {root:PathBuf,policy:PreparedPolicy}
    impl Fixture {
        fn new()->Self {Self::new_with_id("esm-real-map")}
        fn new_with_id(mod_id:&str)->Self {
            let parent=PathBuf::from("E:/Scratch/hdmap-prepared-admission-tests");fs::create_dir_all(&parent).unwrap();
            let root=parent.join(format!("case-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
            for name in [".EnshroudedClientMods","shroudforge/config"] {fs::create_dir_all(root.join(name)).unwrap();}
            let mod_directory=format!("mods/{mod_id}");fs::create_dir_all(root.join(&mod_directory)).unwrap();
            let root=fs::canonicalize(root).unwrap();
            let mod_lua=format!("{mod_directory}/mod.lua");
            let map_lua="mods/esm-real-map/mod.lua";
            for name in ["enshrouded.exe","enshrouded.kfc","enshrouded.kfc_resources","enshrouded_000.dat","enshrouded_001.dat","winmm.dll",".EnshroudedClientMods/operation.lock"] {fs::write(root.join(name),format!("synthetic-{name}")).unwrap();}
            fs::write(root.join(&mod_lua),format!("synthetic-{mod_lua}")).unwrap();
            fs::create_dir_all(root.join(MAP_EXTENSION).parent().unwrap()).unwrap();
            if mod_lua!=map_lua {fs::write(root.join(map_lua),b"synthetic-HD-map-script").unwrap();}
            fs::write(root.join(MAP_EXTENSION),br#"{"schemaVersion":1,"enabled":true,"settings":{"ExtendedZoom":{"value":false}}}"#).unwrap();
            let value=|name:&str|{let (bytes,sha256)=stamp(&root.join(name)).unwrap();FileStamp{bytes,sha256}};
            let mut inputs:BTreeMap<_,_>=["enshrouded.exe","enshrouded.kfc","enshrouded.kfc_resources","enshrouded_000.dat",MAP_EXTENSION].into_iter().map(|n|(n.into(),value(n))).collect();
            inputs.insert(mod_lua.clone(),value(&mod_lua));
            if mod_lua!=map_lua {inputs.insert(map_lua.into(),value(map_lua));}
            let mut environment_files=[MAP_EXTENSION].into_iter().map(|n|(n.into(),value(n))).collect::<BTreeMap<_,_>>();
            environment_files.insert(mod_lua.clone(),value(&mod_lua));
            if mod_lua!=map_lua {environment_files.insert(map_lua.into(),value(map_lua));}
            let environment_root=root.with_file_name(format!("{}-environment",root.file_name().unwrap().to_string_lossy()));
            let output_root=root.with_file_name(format!("{}-output",root.file_name().unwrap().to_string_lossy()));
            let request=PreparedRequest {schema:1,target_root:root.clone(),asset_input_root:root.clone(),environment_root,output_root:output_root.clone(),selected_ids:vec![mod_id.into()],inputs,environment_files,external_registry:None};
            let mut staged:BTreeMap<_,_>=["enshrouded.kfc","enshrouded.kfc_resources","enshrouded_001.dat"].into_iter().map(|n|(n.into(),value(n))).collect();
            staged.insert("generated-inputs.json".into(),FileStamp{bytes:123,sha256:digest(b"synthetic durable generation preimages")});
            let receipt=PreparedReceipt {schema:1,source_closure:option_env!("HD_PREPARED_SOURCE_CLOSURE").unwrap().into(),request_sha256:digest(&serde_json::to_vec(&request).unwrap()),target_root:root.clone(),output_stage:output_root.join(".enshrouded.shroudforge-stage"),staged,prepared_marker:FileStamp{bytes:100,sha256:digest(b"synthetic prepared marker")},source_dat_membership:None,source_dat_membership_sha256:None,external_registry:None};
            let request_json=serde_json::to_string(&request).unwrap();let receipt_json=serde_json::to_string(&receipt).unwrap();
            let policy=PreparedPolicy {schema_version:5,owner:"hdmap-prepared-generator".into(),root:root.clone(),source_closure:receipt.source_closure.clone(),request_sha256:digest(request_json.as_bytes()),receipt_sha256:digest(receipt_json.as_bytes()),request_json,receipt_json,extended_zoom:false,runtime:None,preserved_loaders:[("winmm.dll".into(),value("winmm.dll").sha256)].into_iter().collect()};
            Self {root,policy}
        }
        fn write(&self){fs::write(self.root.join(POLICY),serde_json::to_vec(&self.policy).unwrap()).unwrap();}
        fn asset_hashes(&self)->BTreeMap<String,(u64,String)> {fs::read_dir(&self.root).unwrap().map(|e|e.unwrap()).filter_map(|e|{let name=e.file_name().to_string_lossy().into_owned();asset(&name).then(||(name,stamp(&e.path()).unwrap()))}).collect()}
        fn edit_request(&mut self,edit:impl FnOnce(&mut PreparedRequest)) {
            let mut request:PreparedRequest=serde_json::from_str(&self.policy.request_json).unwrap();edit(&mut request);
            self.policy.request_json=serde_json::to_string(&request).unwrap();self.policy.request_sha256=digest(self.policy.request_json.as_bytes());
            self.edit_receipt(|r|r.request_sha256=digest(&serde_json::to_vec(&request).unwrap()));
        }
        fn edit_receipt(&mut self,edit:impl FnOnce(&mut PreparedReceipt)) {
            let mut receipt:PreparedReceipt=serde_json::from_str(&self.policy.receipt_json).unwrap();edit(&mut receipt);
            self.policy.receipt_json=serde_json::to_string(&receipt).unwrap();self.policy.receipt_sha256=digest(self.policy.receipt_json.as_bytes());
        }
        fn source_dat_membership(&self)->BTreeMap<String,FileStamp> {
            let request:PreparedRequest=serde_json::from_str(&self.policy.request_json).unwrap();
            request.inputs.into_iter().filter(|(name,_)|is_dat_asset(name)).collect()
        }
        fn set_schema2_receipt(&mut self) {
            let membership=self.source_dat_membership();
            let membership_sha256=digest(&serde_json::to_vec(&membership).unwrap());
            self.edit_receipt(|receipt|{
                receipt.schema=2;
                receipt.source_dat_membership=Some(membership);
                receipt.source_dat_membership_sha256=Some(membership_sha256);
            });
        }
    }
    impl Drop for Fixture {fn drop(&mut self){if let (Ok(path),Ok(parent))=(fs::canonicalize(&self.root),fs::canonicalize("E:/Scratch/hdmap-prepared-admission-tests")){if path.parent()==Some(parent.as_path())&&plain(&path).is_ok(){let _=fs::remove_dir_all(path);}}}}

    #[test]
    fn synthetic_prepared_receipt_admits_committed_bytes_without_asset_writes() {
        let f=Fixture::new();f.write();let before=f.asset_hashes();
        assert!(super::super::super::prepare(&f.root).unwrap());
        assert_eq!(f.asset_hashes(),before);
        assert_eq!(take_loader_demand(&f.root),0x80000001);
        assert_eq!(zoom_demand::take(&f.root),0);
        assert!(!transaction::directory(&f.root,STEM).exists());
        assert!(!f.root.join("enshrouded.kfc.bak").exists());
    }
    #[test]
    fn selected_ids_admit_canonical_dotted_mod_without_asset_writes() {
        let f=Fixture::new_with_id("esm.stack-size");f.write();let before=f.asset_hashes();
        assert!(super::super::super::prepare(&f.root).unwrap());
        assert_eq!(f.asset_hashes(),before);
        assert_eq!(take_loader_demand(&f.root),0x80000001);
        assert_eq!(zoom_demand::take(&f.root),0);
        assert!(!transaction::directory(&f.root,STEM).exists());
        assert!(!f.root.join("enshrouded.kfc.bak").exists());
    }
    #[test]
    fn schema5_runtime_field_even_null_does_not_reinterpret_legacy_receipt() {
        let f=Fixture::new();let before=f.asset_hashes();
        let mut document=serde_json::to_value(&f.policy).unwrap();document["runtime"]=serde_json::Value::Null;
        fs::write(f.root.join(POLICY),serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(super::super::super::prepare(&f.root).is_err());assert_eq!(before,f.asset_hashes());
    }
    #[test]
    fn schema2_receipt_binds_exact_source_dat_input_subset() {
        let mut f=Fixture::new();
        let request:PreparedRequest=serde_json::from_str(&f.policy.request_json).unwrap();
        let legacy:PreparedReceipt=serde_json::from_str(&f.policy.receipt_json).unwrap();
        assert!(validate_receipt_schema(&request,&legacy,false).unwrap().is_none());
        assert!(validate_receipt_schema(&request,&legacy,true).is_err());

        f.set_schema2_receipt();
        let valid:PreparedReceipt=serde_json::from_str(&f.policy.receipt_json).unwrap();
        let membership=validate_receipt_schema(&request,&valid,true).unwrap().unwrap();
        assert_eq!(membership.keys().cloned().collect::<BTreeSet<_>>(),BTreeSet::from(["enshrouded_000.dat".to_owned()]));

        let mut missing=valid.clone();missing.source_dat_membership=None;
        assert!(validate_receipt_schema(&request,&missing,true).is_err());
        let mut wrong_digest=valid.clone();wrong_digest.source_dat_membership_sha256=Some("0".repeat(64));
        assert!(validate_receipt_schema(&request,&wrong_digest,true).is_err());
        let mut wrong_stamp=valid.clone();
        let mut changed=wrong_stamp.source_dat_membership.clone().unwrap();changed.get_mut("enshrouded_000.dat").unwrap().bytes+=1;
        wrong_stamp.source_dat_membership_sha256=Some(digest(&serde_json::to_vec(&changed).unwrap()));wrong_stamp.source_dat_membership=Some(changed);
        assert!(validate_receipt_schema(&request,&wrong_stamp,true).is_err());
        let mut alias=valid;let mut changed=alias.source_dat_membership.clone().unwrap();
        let stamp=changed.remove("enshrouded_000.dat").unwrap();changed.insert("Enshrouded_000.dat".into(),stamp);
        alias.source_dat_membership_sha256=Some(digest(&serde_json::to_vec(&changed).unwrap()));alias.source_dat_membership=Some(changed);
        assert!(validate_receipt_schema(&request,&alias,true).is_err());
    }
    #[test]
    fn schema2_installed_dat_set_is_source_union_generated_outputs() {
        let stamp=FileStamp{bytes:1,sha256:"a".repeat(64)};
        let source=[("enshrouded_000.dat".to_owned(),stamp.clone())].into_iter().collect();
        let staged=[
            ("enshrouded.kfc".to_owned(),stamp.clone()),
            ("enshrouded.kfc_resources".to_owned(),stamp.clone()),
            ("enshrouded_001.dat".to_owned(),stamp.clone()),
        ].into_iter().collect();
        let actual=BTreeSet::from([
            "enshrouded.kfc".to_owned(),"enshrouded.kfc_resources".to_owned(),
            "enshrouded_000.dat".to_owned(),"enshrouded_001.dat".to_owned(),
        ]);
        validate_schema2_dat_union(&actual,&source,&staged).unwrap();
        let mut missing=actual.clone();missing.remove("enshrouded_000.dat");
        assert!(validate_schema2_dat_union(&missing,&source,&staged).is_err());
        let mut extra=actual.clone();extra.insert("enshrouded_002.dat".into());
        assert!(validate_schema2_dat_union(&extra,&source,&staged).is_err());
        let mut overlap=staged.clone();overlap.insert("enshrouded_000.dat".into(),stamp);
        assert!(validate_schema2_dat_union(&actual,&source,&overlap).is_err());
    }
    #[test]
    #[ignore="requires explicitly synthetic SF_FULL_* test-build closures; not a production producer receipt"]
    fn schema6_readonly_admission_uses_receipt2_and_selected_service_phases() {
        let mut f=Fixture::new_with_id("esm.stack-size");let root=f.root.clone();
        let producer=option_env!("SF_FULL_PREPARED_SOURCE_CLOSURE").unwrap();
        let runtime=option_env!("SF_FULL_RUNTIME_SOURCE_CLOSURE").unwrap();
        f.policy.schema_version=6;f.policy.owner="shroudforge-full-prepared".into();f.policy.source_closure=producer.into();f.policy.preserved_loaders.clear();
        f.set_schema2_receipt();
        f.edit_receipt(|r|r.source_closure=producer.into());
        let package=root.join("mods/fixture-runtime");fs::create_dir_all(package.join("src")).unwrap();
        fs::write(package.join("mod.json"),br#"{"id":"fixture-runtime","name":"Fixture","version":"1.0.0","capabilities":["runtime"]}"#).unwrap();
        fs::write(package.join("extended.mod.json"),br#"{"schemaVersion":1,"enabled":true,"targets":["client"]}"#).unwrap();
        fs::write(package.join("src/mod.lua"),b"return {}\n").unwrap();
        let entries:Vec<_>=["mod.json","extended.mod.json","src/mod.lua"].into_iter().map(|name|{let(bytes,sha256)=stamp(&package.join(name)).unwrap();(format!("mods/fixture-runtime/{name}"),FileStamp{bytes,sha256})}).collect();
        f.edit_request(|r|for(name,value)in entries{r.inputs.insert(name.clone(),value.clone());r.environment_files.insert(name,value);});
        fs::write(root.join("enshrouded.kfc"),b"committed KFC postimage").unwrap();
        fs::write(root.join("enshrouded.kfc_resources"),b"committed resource postimage").unwrap();
        f.edit_receipt(|r|for name in ["enshrouded.kfc","enshrouded.kfc_resources"] {
            let(bytes,sha256)=stamp(&root.join(name)).unwrap();r.staged.insert(name.into(),FileStamp{bytes,sha256});
        });
        fs::create_dir_all(root.join("shroudforge")).unwrap();
        fs::write(root.join("shroudforge/shroudforge-runtime.dll"),b"synthetic runtime; never loaded").unwrap();
        fs::write(root.join("shroudforge/kfc-runtime.dll"),b"synthetic provider; never loaded").unwrap();
        let config=shroudforge_loader_config::config::read_loader_defaults().unwrap();
        fs::write(root.join("shroudforge/config/modloader-config.json"),serde_json::to_vec(&config).unwrap()).unwrap();
        let files=["winmm.dll","shroudforge/shroudforge-runtime.dll","shroudforge/kfc-runtime.dll","shroudforge/config/modloader-config.json"].into_iter().map(|name|{let(bytes,sha256)=stamp(&root.join(name)).unwrap();(name.into(),super::super::super::full_runtime::NativeFile{bytes,sha256})}).collect();
        let plan=mod_loader::dll_manager::resolve_load_plan(&root,false);assert!(plan.issues.is_empty());assert!(plan.entries.is_empty());
        let(bytes,sha256)=stamp(&package.join("src/mod.lua")).unwrap();
        let scripts=[("fixture-runtime:src/mod.lua".into(),super::super::super::full_runtime::NativeFile{bytes,sha256})].into_iter().collect();
        f.policy.runtime=Some(super::super::super::full_runtime::Contract{schema_version:2,host:"sf-winmm-selected-services-v2".into(),runtime_source_closure:runtime.into(),prepared_request_sha256:f.policy.request_sha256.clone(),prepared_receipt_sha256:f.policy.receipt_sha256.clone(),boot_services:vec!["metadata-init-v1".into()],overlay:false,first_person:false,selection:Default::default(),runtime_mods:vec!["fixture-runtime".into()],scripts,dll_plan:mod_loader::dll_manager::load_plan_identity(&plan),files,greeneye_paths:vec![]});
        f.write();let before=f.asset_hashes();
        assert!(super::super::super::prepare(&root).unwrap());assert_eq!(f.asset_hashes(),before);
        super::super::super::full_runtime::prepare_native(&root).unwrap();
        assert!(crate::native_dlls::prepare_full_startup(&root,f.policy.runtime.as_ref().unwrap()).is_err());
        let selected=super::super::super::full_runtime::take_v2(&root).unwrap();
        assert!(selected.valid());
        assert_eq!(selected.services,super::super::super::full_runtime::selected_services::METADATA);
        assert!(super::super::super::full_runtime::release_vein_lease(&root,&selected,selected.prepared_services()));
        assert!(super::super::super::full_runtime::commit_v2(&root,&selected,selected.services));
        super::super::super::full_runtime::before_create(&root).unwrap();
        #[cfg(windows)]assert!(fs::write(root.join("mods/esm-real-map/mod.lua"),b"must stay immutable through create").is_err());
        super::super::super::full_runtime::after_create(&root).unwrap();
        assert!(super::super::super::full_runtime::reject_package_reload(&root).is_err());
        fs::write(package.join("src/mod.lua"),b"changed after create").unwrap();
        assert!(super::super::super::full_runtime::validate_script(&root,"fixture-runtime",&package,"src/mod.lua",b"changed after create").is_err());
        super::super::super::full_runtime::validate_script(&root,"fixture-runtime",&package,"src/mod.lua",b"return {}\n").unwrap();
        fs::write(root.join("mods/esm-real-map/mod.lua"),b"settings/source may change after create").unwrap();
        #[cfg(windows)]assert!(fs::write(root.join("shroudforge/kfc-runtime.dll"),b"resident native image remains held").is_err());
        assert!(super::super::super::full_runtime::before_create(&root).is_err());
        assert_eq!(f.asset_hashes(),before);assert!(!root.join("enshrouded.kfc.bak").exists());
        super::super::super::full_runtime::retire_fixture();
    }
    #[test]
    fn rehashed_wrong_producer_roots_selection_outputs_and_dependencies_still_refuse() {
        for case in 0..10 {
            let mut f=Fixture::new();
            match case {
                0=>f.policy.source_closure="0".repeat(64),
                1=>f.edit_receipt(|r|r.source_closure="0".repeat(64)),
                2=>f.edit_request(|r|r.asset_input_root=r.environment_root.clone()),
                3=>f.edit_request(|r|r.selected_ids=vec!["Auto_Loot".into()]),
                4=>f.edit_receipt(|r|r.target_root=r.output_stage.clone()),
                5=>f.edit_receipt(|r|r.staged.get_mut("enshrouded.kfc").unwrap().sha256="0".repeat(64)),
                6=>{f.policy.preserved_loaders.clear();},
                7=>f.edit_request(|r|r.output_root=r.environment_root.clone()),
                8=>f.edit_receipt(|r|r.output_stage=r.target_root.join(".enshrouded.shroudforge-stage")),
                9=>{fs::write(f.root.join("enshrouded_999.dat"),b"unknown").unwrap();},
                _=>unreachable!(),
            }
            f.write();let before=f.asset_hashes();
            assert!(super::super::super::prepare(&f.root).is_err(),"case{case}");
            assert_eq!(f.asset_hashes(),before);assert_eq!(take_loader_demand(&f.root),0);
        }
    }
    #[test]
    fn selected_ids_refuse_malformed_and_autoloot_without_asset_writes() {
        let invalid:Vec<Vec<String>>=vec![
            vec![],
            vec!["".into()],
            vec![".".into()],
            vec!["..".into()],
            vec!["esm/stack-size".into()],
            vec![r"esm\stack-size".into()],
            vec!["a".repeat(81)],
            vec!["AUTO_LOOT".into()],
            vec!["esm.auto.loot".into()],
            vec!["esm.stack-size".into(),"esm.stack-size".into()],
        ];
        for (case,selected_ids) in invalid.into_iter().enumerate() {
            let mut f=Fixture::new();f.edit_request(|request|request.selected_ids=selected_ids);f.write();
            let before=f.asset_hashes();
            assert!(super::super::super::prepare(&f.root).is_err(),"case{case}");
            assert_eq!(f.asset_hashes(),before);
            assert_eq!(take_loader_demand(&f.root),0,"case{case}");
            // A rejected preparation leaves no zoom handoff; an absent handoff reads as unavailable.
            assert_eq!(zoom_demand::take(&f.root),2,"case{case}");
        }
    }
}

#[cfg(test)]
mod external_registry_manifest_tests {
    use super::*;
    #[test]
    fn external_registry_prepared_receipt_requires_matching_explicit_provenance() {
        let pin=crate::EvidencePin{path:"evidence/manifest.json".into(),bytes:7,sha256:"a".repeat(64)};
        let input=crate::ExternalRegistryInput{manifest:pin.clone(),producer_use_approval:crate::EvidencePin{path:"evidence/producer-use.json".into(),..pin}};
        let request=PreparedRequest{schema:2,target_root:PathBuf::from("source"),asset_input_root:PathBuf::from("source"),environment_root:PathBuf::from("environment"),output_root:PathBuf::from("output"),selected_ids:vec!["fixture".into()],inputs:BTreeMap::new(),environment_files:BTreeMap::new(),external_registry:Some(input.clone())};
        let membership=BTreeMap::new();
        let mut receipt=PreparedReceipt{schema:3,source_closure:"b".repeat(64),request_sha256:"c".repeat(64),target_root:PathBuf::from("source"),output_stage:PathBuf::from("stage"),staged:BTreeMap::new(),prepared_marker:FileStamp{bytes:7,sha256:"d".repeat(64)},source_dat_membership:Some(membership),source_dat_membership_sha256:Some(digest(&serde_json::to_vec(&BTreeMap::<String,FileStamp>::new()).unwrap())),external_registry:Some(input)};
        assert!(validate_receipt_schema(&request,&receipt,true).is_ok());
        assert!(validate_receipt_schema(&request,&receipt,false).is_err());
        receipt.schema=2;assert!(validate_receipt_schema(&request,&receipt,true).is_err());
        receipt.schema=3;receipt.external_registry.as_mut().unwrap().producer_use_approval.sha256="e".repeat(64);
        assert!(validate_receipt_schema(&request,&receipt,true).is_err());
        receipt.external_registry=None;assert!(validate_receipt_schema(&request,&receipt,true).is_err());
    }
}
