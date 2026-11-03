use super::*;
use kfc::{container::{KFCFile,KFCReader,KFCWriter},guid::{ContentHash,Guid,ResourceId},reflection::TypeRegistry};

struct Fixture { root:PathBuf, request:Request, resource:ResourceId, original_content:ContentHash, registry:TypeRegistry }
impl Fixture {
    fn new(lua:&str)->Self {
        Self::new_with_id(lua,"writer")
    }
    fn new_with_id(lua:&str,id:&str)->Self {
        let parent=PathBuf::from("E:/Scratch/hdmap-prepared-origin-tests");
        fs::create_dir_all(&parent).unwrap();
        let root=parent.join(format!("case-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let source=root.join("source");let environment=root.join("environment");let output=root.join("output");
        for p in [&source,&environment,&output] {fs::create_dir_all(p).unwrap();}
        let source=fs::canonicalize(source).unwrap();let environment=fs::canonicalize(environment).unwrap();let output=fs::canonicalize(output).unwrap();
        fs::write(source.join("enshrouded.exe"),b"synthetic fixture; never executable").unwrap();
        fs::write(source.join("enshrouded.kfc"),[]).unwrap();fs::write(source.join("enshrouded.kfc_resources"),[]).unwrap();
        let qualified="test::U32";
        let registry:TypeRegistry=serde_json::from_value(serde_json::json!({"version":"hd-prepared-fixture","types":[{
            "index":0,"name":"U32","qualifiedName":qualified,"impactName":"U32","size":4,"alignment":4,"elementAlignment":4,"fieldCount":0,"primitiveType":"UINT32","innerType":null,"flags":"","nameHash":kfc::hash::fnv("U32"),"qualifiedHash":kfc::hash::fnv(qualified),"impactHash":kfc::hash::fnv("U32"),"internalHash":99,"defaultValue":[0,0,0,0]
        }]})).unwrap();
        let resource=ResourceId::new(Guid::parse("00000000-0000-0000-0000-000000000001").unwrap(),kfc::hash::fnv(qualified),0);
        let original_content=ContentHash::from_data(b"retained source content");
        let mut writer=KFCWriter::<KFCFile,_>::new(&source,STEM,&registry,&registry.version).unwrap();
        writer.write_resource(&resource,&7u32.to_le_bytes()).unwrap();
        writer.write_content(&original_content,b"retained source content").unwrap();writer.finalize().unwrap();
        let package=environment.join(format!("mods/{id}"));fs::create_dir_all(package.join("src")).unwrap();
        fs::write(package.join("mod.json"),serde_json::to_vec(&serde_json::json!({"id":id,"name":id,"version":"1.0.0","dependencies":[],"capabilities":["patch"]})).unwrap()).unwrap();
        fs::write(package.join("extended.mod.json"),br#"{"schemaVersion":1,"enabled":true,"targets":["client"]}"#).unwrap();
        fs::write(package.join("src/mod.lua"),lua).unwrap();
        let cache=environment.join("shroudforge/cache");fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("types-enshrouded.json"),serde_json::to_vec(&registry).unwrap()).unwrap();
        fs::write(cache.join("types-enshrouded.identity.json"),serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"executableSha256":stamp(&source.join("enshrouded.exe")).unwrap().sha256,"reflectionExtractor":"kfc-a404b15cde40fc7bd43d812bb460ce40d2a9d3b1"})).unwrap()).unwrap();
        fs::write(source.join("personal-settings.keep"),b"unchanged").unwrap();
        let environment_files=inventory(&environment).unwrap();
        let mut inputs=inventory(&source).unwrap();inputs.remove("personal-settings.keep");
        inputs.extend(environment_files.iter().filter(|(n,_)|n.starts_with("mods/")).map(|(n,s)|(n.clone(),s.clone())));
        Self {root,request:Request{schema:1,target_root:source.clone(),asset_input_root:source,environment_root:environment,output_root:output,selected_ids:vec![id.into()],inputs,environment_files},resource,original_content,registry}
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let (Ok(path),Ok(parent))=(fs::canonicalize(&self.root),fs::canonicalize("E:/Scratch/hdmap-prepared-origin-tests")) {
            if path.parent()==Some(parent.as_path()) && ordinary(&path,true).is_ok() {let _=fs::remove_dir_all(path);}
        }
    }
}
const EDIT:&str=r#"local r=game.assets.get_resource('00000000-0000-0000-0000-000000000001','test::U32'); assert(r and r.data==7); r.data=29; return {}"#;

fn assemble_overlay(source:&Path,stage:&Path,overlay:&Path)->Result<()> {
    fs::create_dir(overlay)?;
    for entry in fs::read_dir(stage)? {
        let entry=entry?;let name=entry.file_name().into_string().map_err(|_|anyhow::anyhow!("nonUTF8 stage member"))?;
        if asset(&name) { fs::copy(entry.path(),overlay.join(name))?; }
    }
    for name in source_dat_inventory(source)? {
        ensure!(!overlay.join(&name).exists(),"generated output collides with retained source DAT");
        fs::copy(source.join(&name),overlay.join(name))?;
    }
    Ok(())
}

#[test]
fn selected_dotted_id_preparation_respects_generated_dat_reservation(){
    let mut f=Fixture::new_with_id(EDIT,"esm.stack-size");
    fs::write(f.request.asset_input_root.join("enshrouded_001.dat"),b"retained old owned bytes").unwrap();
    f.request.inputs.insert("enshrouded_001.dat".into(),stamp(&f.request.asset_input_root.join("enshrouded_001.dat")).unwrap());
    let before=inventory(&f.request.asset_input_root).unwrap();let receipt=prepare(&f.request).unwrap();
    let expected_dat_membership:BTreeMap<_,_>=f.request.inputs.iter().filter(|(name,_)|dat_asset(name)).map(|(name,value)|(name.clone(),value.clone())).collect();
    assert_eq!(receipt.schema,2);
    assert_eq!(receipt.source_dat_membership,expected_dat_membership);
    assert_eq!(receipt.source_dat_membership_sha256,format!("{:x}",Sha256::digest(serde_json::to_vec(&receipt.source_dat_membership).unwrap())));
    assert_eq!(inventory(&f.request.asset_input_root).unwrap(),before);
    assert!(!receipt.staged.contains_key("enshrouded_001.dat"));
    let generated=KFCFile::from_path(receipt.output_stage.join("enshrouded.kfc"),false).unwrap();assert_eq!(generated.containers().len(),2);
    assert!(generated.resources().contains_key(&f.resource));
    assert!(generated.resources_by_type(f.resource.type_hash()).any(|id| *id==f.resource));
    let overlay=f.root.join("overlay");assemble_overlay(&f.request.asset_input_root,&receipt.output_stage,&overlay).unwrap();
    let mut composite=KFCReader::new(&overlay,STEM).unwrap().into_cursor().unwrap();
    assert_eq!(composite.read_content(&f.original_content).unwrap().unwrap(),b"retained source content");
    assert_eq!(composite.read_resource(&f.resource).unwrap().unwrap(),29u32.to_le_bytes());
    let mut reader=KFCReader::new(&receipt.output_stage,STEM).unwrap().into_cursor().unwrap();assert_eq!(reader.read_resource(&f.resource).unwrap().unwrap(),29u32.to_le_bytes());
    assert!(shroudforge_game_assets::transaction::commit_generated(&f.request.output_root,STEM).is_err());
}

#[test]
fn selected_ids_follow_manifest_rules_and_keep_autoloot_excluded() {
    let invalid=vec![
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
        let mut f=Fixture::new(EDIT);f.request.selected_ids=selected_ids;
        let before=inventory(&f.request.asset_input_root).unwrap();
        assert!(prepare(&f.request).is_err(),"case{case}");
        assert_eq!(inventory(&f.request.asset_input_root).unwrap(),before);
        assert_eq!(fs::read_dir(&f.request.output_root).unwrap().count(),0);
    }
}

#[test]
fn actual_lua_output_is_separate_and_cannot_use_normal_publication() {
    let f=Fixture::new(EDIT);let before=inventory(&f.request.asset_input_root).unwrap();
    let receipt=prepare(&f.request).unwrap();
    assert_eq!(inventory(&f.request.asset_input_root).unwrap(),before);
    assert_eq!(receipt.target_root,f.request.target_root);
    assert_eq!(receipt.source_closure,source_closure().unwrap());
    let mut reader=KFCReader::new(&receipt.output_stage,STEM).unwrap().into_cursor().unwrap();
    assert_eq!(reader.read_resource(&f.resource).unwrap().unwrap(),29u32.to_le_bytes());
    let mut source=KFCReader::new(&f.request.asset_input_root,STEM).unwrap().into_cursor().unwrap();
    assert_eq!(source.read_resource(&f.resource).unwrap().unwrap(),7u32.to_le_bytes());
    assert!(shroudforge_game_assets::transaction::commit_generated(&f.request.output_root,STEM).is_err());
    assert!(shroudforge_game_assets::transaction::recover(&f.request.output_root,STEM).is_err());
    assert!(shroudforge_game_assets::transaction::abort(&f.request.output_root,STEM).is_err());
    assert!(prepare(&f.request).is_err());
}

#[test]
fn refusals_happen_before_any_output_or_source_change() {
    for mutation in 0..7 {
        let mut f=Fixture::new(EDIT);let before=inventory(&f.request.asset_input_root).unwrap();
        match mutation {
            0=>f.request.selected_ids=vec!["AUTO_LOOT".into()],
            1=>f.request.output_root=f.request.asset_input_root.clone(),
            2=>f.request.selected_ids.push("writer".into()),
            3=>f.request.inputs.get_mut("enshrouded.exe").unwrap().sha256="0".repeat(64),
            4=>{f.request.environment_files.remove("mods/writer/src/mod.lua");},
            5=>f.request.target_root=f.request.environment_root.clone(),
            6=>{f.request.inputs.insert("../escape".into(),Stamp{bytes:0,sha256:"0".repeat(64)});},
            _=>unreachable!(),
        }
        assert!(prepare(&f.request).is_err(),"case{mutation}");
        assert_eq!(inventory(&f.request.asset_input_root).unwrap(),before);
        assert_eq!(fs::read_dir(f.root.join("output")).unwrap().count(),0);
    }
}

#[test]
fn lua_failure_retains_only_unpublishable_e_stage() {
    let f=Fixture::new("error('intentional selected Lua failure')");let before=inventory(&f.request.asset_input_root).unwrap();
    assert!(prepare(&f.request).is_err());
    assert_eq!(inventory(&f.request.asset_input_root).unwrap(),before);
    let stage=shroudforge_game_assets::transaction::directory(&f.request.output_root,STEM);
    assert!(stage.join("prepared-only.json").is_file());
    assert!(prepare(&f.request).is_err());
    assert!(shroudforge_game_assets::transaction::recover(&f.request.output_root,STEM).is_err());
}

#[test]
fn source_dat_aliases_are_rejected_before_prepared_stage_creation() {
    for name in ["Enshrouded_001.DAT", "enshrouded_1.dat", "enshrouded_0001.dat"] {
        let f = Fixture::new(EDIT);
        fs::write(f.request.asset_input_root.join(name), b"occupied source slot").unwrap();
        assert!(source_dat_inventory(&f.request.asset_input_root).is_err(), "{name}");
        assert_eq!(fs::read_dir(&f.request.output_root).unwrap().count(), 0);
        assert!(!shroudforge_game_assets::transaction::directory(
            &f.request.output_root,
            STEM
        )
        .exists());
    }
}

#[cfg(windows)]
#[test]
fn input_handles_refuse_concurrent_source_writes_and_delete() {
    let f=Fixture::new(EDIT);let (_,_,_,handles,_)=qualify(&f.request).unwrap();
    assert!(OpenOptions::new().write(true).open(f.request.asset_input_root.join("enshrouded.kfc")).is_err());
    assert!(fs::remove_file(f.request.asset_input_root.join("enshrouded.kfc")).is_err());
    drop(handles);
    assert!(OpenOptions::new().write(true).open(f.request.asset_input_root.join("enshrouded.kfc")).is_ok());
}

#[path="dat_reservation_tests.rs"]
mod dat_reservation;
