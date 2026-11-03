use std::{collections::BTreeMap,fs};
use kfc::{container::{KFCFile,KFCReader,KFCWriter},guid::ContentHash,reflection::TypeRegistry};
use std::path::PathBuf;
struct Fixture{root:PathBuf,source:PathBuf,output:PathBuf,registry:TypeRegistry,reference:KFCFile,old:ContentHash}
impl Fixture{
 fn new()->Self{let parent=PathBuf::from("E:/Scratch/generated-dat-reservation-tests");fs::create_dir_all(&parent).unwrap();let root=parent.join(format!("case-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));let source=root.join("source");let output=root.join("output");fs::create_dir_all(&source).unwrap();fs::create_dir(&output).unwrap();fs::write(source.join("game.kfc"),[]).unwrap();fs::write(source.join("game.kfc_resources"),[]).unwrap();let registry=TypeRegistry::default();let old=ContentHash::from_data(b"original content");let mut writer=KFCWriter::<KFCFile,_>::new(&source,"game",&registry,"fixture").unwrap();writer.write_content(&old,b"original content").unwrap();writer.finalize().unwrap();let reference=KFCFile::from_path(source.join("game.kfc"),false).unwrap();Self{root,source,output,registry,reference,old}}
 fn snapshot(&self)->BTreeMap<String,Vec<u8>>{fs::read_dir(&self.source).unwrap().map(|e|{let e=e.unwrap();(e.file_name().into_string().unwrap(),fs::read(e.path()).unwrap())}).collect()}
}
impl Drop for Fixture{fn drop(&mut self){let parent=fs::canonicalize("E:/Scratch/generated-dat-reservation-tests").unwrap();if let Ok(p)=fs::canonicalize(&self.root){if p.parent()==Some(parent.as_path()){let _=fs::remove_dir_all(p);}}}}
#[test]
fn generated_content_skips_all_retained_source_dats_and_preserves_maps_and_bytes(){
 let f=Fixture::new();for index in 1..8{fs::write(f.source.join(format!("game_{index:03}.dat")),format!("retained orphan {index}")).unwrap();}let before=f.snapshot();let bytes=b"new unique generated content";let id=ContentHash::from_data(bytes);
 let old=f.reference.contents().get(&f.old).unwrap();let old_map=(old.offset,old.flags,old.container_index);let old_info=&f.reference.containers()[0];let old_container=(old_info.size,old_info.count);
 let mut w=KFCWriter::new_generated_incremental(&f.source,&f.output,"game",&f.reference,&f.registry).unwrap();w.write_content(&id,bytes).unwrap();w.finalize().unwrap();
 assert_eq!(f.snapshot(),before);let result=KFCFile::from_path(f.output.join("game.kfc"),false).unwrap();assert_eq!(result.contents().get(&id).unwrap().container_index,8);let old=result.contents().get(&f.old).unwrap();assert_eq!((old.offset,old.flags,old.container_index),old_map);assert_eq!((result.containers()[0].size,result.containers()[0].count),old_container);assert_eq!(result.containers().len(),16);
 for index in 0..8{assert!(!f.output.join(format!("game_{index:03}.dat")).exists());}for index in 8..16{assert!(f.output.join(format!("game_{index:03}.dat")).is_file());}
 let mut reader=KFCReader::new(&f.output,"game").unwrap().into_cursor().unwrap();assert_eq!(reader.read_content(&id).unwrap().unwrap(),bytes);let mut source=KFCReader::new(&f.source,"game").unwrap().into_cursor().unwrap();assert_eq!(source.read_content(&f.old).unwrap().unwrap(),b"original content");
}
#[test]
fn sparse_reserved_namespace_emits_only_unoccupied_padding(){
 let f=Fixture::new();fs::write(f.source.join("game_007.dat"),b"occupied-seven").unwrap();let before=f.snapshot();let id=ContentHash::from_data(b"generated");let mut w=KFCWriter::new_generated_incremental(&f.source,&f.output,"game",&f.reference,&f.registry).unwrap();w.write_content(&id,b"generated").unwrap();w.finalize().unwrap();assert_eq!(f.snapshot(),before);assert!(!f.output.join("game_000.dat").exists()&&!f.output.join("game_007.dat").exists());for index in 1..7{assert_eq!(fs::metadata(f.output.join(format!("game_{index:03}.dat"))).unwrap().len(),0);}let result=KFCFile::from_path(f.output.join("game.kfc"),false).unwrap();assert_eq!(result.containers()[7].size,14);assert_eq!(result.contents().get(&id).unwrap().container_index,8);
}
#[test]
fn noncanonical_missing_nonfile_or_exhausted_source_names_refuse_without_clobber(){
 for mode in 0..5{let f=Fixture::new();match mode{0=>fs::write(f.source.join("game_01.dat"),b"alias").unwrap(),1=>fs::remove_file(f.source.join("game_000.dat")).unwrap(),2=>fs::create_dir(f.source.join("game_001.dat")).unwrap(),3=>fs::write(f.source.join("game_65536.dat"),[]).unwrap(),4=>fs::write(f.source.join("game_65535.dat"),[]).unwrap(),_=>unreachable!()};
  if mode==4{let mut w=KFCWriter::new_generated_incremental(&f.source,&f.output,"game",&f.reference,&f.registry).unwrap();let id=ContentHash::from_data(b"new");assert!(w.write_content(&id,b"new").is_err());assert!(!f.output.join("game_65536.dat").exists());}else{assert!(KFCWriter::new_generated_incremental(&f.source,&f.output,"game",&f.reference,&f.registry).is_err());assert_eq!(fs::read_dir(&f.output).unwrap().count(),0);}
 }
}

#[test]
fn late_source_dat_at_reserved_slot_rejects_prepared_stage(){
 let f=super::Fixture::new(super::EDIT);
 let (source,environment,output,_handles,source_dat_membership)=crate::qualify(&f.request).unwrap();
 let late_slot="enshrouded_001.dat";
 assert!(source_dat_membership.contains_key("enshrouded_000.dat"));
 assert!(!source_dat_membership.contains_key(late_slot));
 let source_before=crate::inventory(&source).unwrap();
 let stage=shroudforge_game_assets::transaction::begin_generated_prepared(&source,&output,"enshrouded").unwrap();
 let reference=KFCFile::from_path(source.join("enshrouded.kfc"),false).unwrap();
 let mut writer=KFCWriter::new_generated_incremental(&source,&stage,"enshrouded",&reference,&f.registry).unwrap();

 // The writer has reserved its physical slots; add the canonical file before generated content is written.
 fs::write(source.join(late_slot),b"late source slot one").unwrap();
 let bytes=b"generated content for reserved slot one";
 let id=ContentHash::from_data(bytes);
 writer.write_content(&id,bytes).unwrap();
 writer.finalize().unwrap();

 let staged=crate::inventory(&stage).unwrap();
 assert!(staged.contains_key(late_slot),"fixture must reproduce the generated/source DAT collision");
 let accepted=crate::accept_prepared_stage(&f.request,&source,&environment,&output,&stage,&source_dat_membership);
 assert!(accepted.is_err(),"a source membership change must prevent Receipt acceptance");
 assert!(stage.join("prepared-only.json").is_file());
 assert!(shroudforge_game_assets::transaction::commit_generated(&output,"enshrouded").is_err());
 assert!(shroudforge_game_assets::transaction::recover(&output,"enshrouded").is_err());
 assert!(shroudforge_game_assets::transaction::abort(&output,"enshrouded").is_err());

 let mut expected_source=source_before;
 expected_source.insert(late_slot.into(),crate::stamp(&source.join(late_slot)).unwrap());
 assert_eq!(crate::inventory(&source).unwrap(),expected_source,"no backup or other source write is allowed");
}

#[test]
fn populated_registry_and_source_dat_overlay_resolve_generated_maps(){
 let f=super::Fixture::new(super::EDIT);
 fs::write(f.request.asset_input_root.join("enshrouded_001.dat"),b"retained occupied slot one").unwrap();
 fs::write(f.request.asset_input_root.join("enshrouded_007.dat"),b"retained occupied slot seven").unwrap();
 let before: BTreeMap<_,_>=fs::read_dir(&f.request.asset_input_root).unwrap().map(|entry|{let entry=entry.unwrap();(entry.file_name().into_string().unwrap(),fs::read(entry.path()).unwrap())}).collect();
 let reference=KFCFile::from_path(f.request.asset_input_root.join("enshrouded.kfc"),false).unwrap();
 let old_mapping=reference.contents().get(&f.original_content).unwrap();
 let old_map=(old_mapping.offset,old_mapping.flags,old_mapping.container_index);
 let new_bytes=b"generated overlay content";
 let new_content=ContentHash::from_data(new_bytes);
 let mut writer=KFCWriter::new_generated_incremental(&f.request.asset_input_root,&f.request.output_root,"enshrouded",&reference,&f.registry).unwrap();
 writer.write_resource(&f.resource,&29u32.to_le_bytes()).unwrap();
 writer.write_content(&new_content,new_bytes).unwrap();
 writer.finalize().unwrap();

 let generated=KFCFile::from_path(f.request.output_root.join("enshrouded.kfc"),false).unwrap();
 assert_eq!((generated.contents().get(&f.original_content).unwrap().offset,generated.contents().get(&f.original_content).unwrap().flags,generated.contents().get(&f.original_content).unwrap().container_index),old_map);
 assert!(generated.contents().contains_key(&new_content));
 assert!(generated.resources().contains_key(&f.resource));
 assert!(generated.resources_by_type(f.resource.type_hash()).any(|id|*id==f.resource));
 assert_eq!(before,fs::read_dir(&f.request.asset_input_root).unwrap().map(|entry|{let entry=entry.unwrap();(entry.file_name().into_string().unwrap(),fs::read(entry.path()).unwrap())}).collect());
 assert!(!f.request.output_root.join("enshrouded_001.dat").exists());
 assert!(!f.request.output_root.join("enshrouded_007.dat").exists());

 let overlay=f.root.join("overlay");fs::create_dir(&overlay).unwrap();
 for entry in fs::read_dir(&f.request.output_root).unwrap(){let entry=entry.unwrap();let name=entry.file_name().into_string().unwrap();if super::asset(&name){fs::copy(entry.path(),overlay.join(name)).unwrap();}}
 for name in super::source_dat_inventory(&f.request.asset_input_root).unwrap(){assert!(!overlay.join(&name).exists());fs::copy(f.request.asset_input_root.join(&name),overlay.join(name)).unwrap();}
 let mut composite=KFCReader::new(&overlay,"enshrouded").unwrap().into_cursor().unwrap();
 assert_eq!(composite.read_content(&f.original_content).unwrap().unwrap(),b"retained source content");
 assert_eq!(composite.read_content(&new_content).unwrap().unwrap(),new_bytes);
 assert_eq!(composite.read_resource(&f.resource).unwrap().unwrap(),29u32.to_le_bytes());
}
