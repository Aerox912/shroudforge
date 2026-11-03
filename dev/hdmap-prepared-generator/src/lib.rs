//! One scoped caller of the accepted AL-SF28 prepared API. It cannot publish.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, fs::{self, File, OpenOptions}, io::Read, path::{Component, Path, PathBuf}};

pub const SOURCE_CLOSURE: Option<&str> = option_env!("HD_PREPARED_SOURCE_CLOSURE");
const STEM: &str = "enshrouded";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Stamp { pub bytes: u64, pub sha256: String }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u32,
    pub target_root: PathBuf,
    pub asset_input_root: PathBuf,
    pub environment_root: PathBuf,
    pub output_root: PathBuf,
    pub selected_ids: Vec<String>,
    /// Target-relative base assets and selected package/settings identities.
    pub inputs: BTreeMap<String, Stamp>,
    /// Complete disposable environment, including operation-local schema cache.
    pub environment_files: BTreeMap<String, Stamp>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema: u32,
    pub source_closure: String,
    pub request_sha256: String,
    pub target_root: PathBuf,
    pub output_stage: PathBuf,
    pub staged: BTreeMap<String, Stamp>,
    pub prepared_marker: Stamp,
    /// Canonical source DAT names and content identities captured before generation.
    /// Consumers must recheck this membership before using the staged output.
    pub source_dat_membership: BTreeMap<String, Stamp>,
    pub source_dat_membership_sha256: String,
}

pub fn ordinary(path: &Path, directory: bool) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    ensure!(!m.file_type().is_symlink(), "linked path");
    #[cfg(windows)] { use std::os::windows::fs::MetadataExt; ensure!(m.file_attributes() & 0x400 == 0, "reparse path"); }
    ensure!(if directory { m.is_dir() } else { m.is_file() }, "wrong path kind");
    Ok(())
}

fn root(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)), "noncanonical root");
    for parent in path.ancestors() { ordinary(parent, true)?; }
    Ok(fs::canonicalize(path)?)
}

fn checked(base: &Path, name: &str) -> Result<PathBuf> {
    ensure!(!name.is_empty() && !name.starts_with('/') && !name.contains(['\\', ':']), "unsafe relative path");
    let mut path = base.to_path_buf();
    let parts: Vec<_> = name.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        ensure!(!part.is_empty() && *part != "." && *part != ".." && !part.ends_with([' ', '.']) && !part.chars().any(|c| c.is_control() || "<>\"|?*".contains(c)), "unsafe relative component");
        let device = part.split('.').next().unwrap().to_ascii_uppercase();
        ensure!(!["CON","PRN","AUX","NUL"].contains(&device.as_str()) && !(device.len()==4 && (device.starts_with("COM") || device.starts_with("LPT")) && matches!(device.as_bytes()[3],b'1'..=b'9')), "device alias");
        path.push(part);
        ordinary(&path, i+1<parts.len())?;
    }
    Ok(path)
}

pub fn stamp(path: &Path) -> Result<Stamp> {
    ordinary(path, false)?;
    stamp_file(&mut File::open(path)?)
}

fn stamp_file(file: &mut File) -> Result<Stamp> {
    let mut h = Sha256::new(); let mut bytes = 0; let mut buffer = [0;65536];
    loop { let n = file.read(&mut buffer)?; if n==0 { break; } h.update(&buffer[..n]); bytes += n as u64; }
    Ok(Stamp { bytes, sha256: format!("{:x}",h.finalize()) })
}

fn valid_digest(s: &str) -> bool { s.len()==64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) }
fn source_closure() -> Result<&'static str> {
    let closure = SOURCE_CLOSURE.context("Delivery must compute HD_PREPARED_SOURCE_CLOSURE from the committed source closure")?;
    ensure!(valid_digest(closure), "invalid Delivery-computed source closure");
    Ok(closure)
}
fn asset(name: &str) -> bool {
    name=="enshrouded.kfc" || name=="enshrouded.kfc_resources" || dat_asset(name)
}

fn dat_asset(name: &str) -> bool {
    name.strip_prefix("enshrouded_").and_then(|n|n.strip_suffix(".dat")).is_some_and(|n|!n.is_empty() && n.bytes().all(|c|c.is_ascii_digit()))
}

fn source_dat_inventory(source: &Path) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| anyhow::anyhow!("nonUTF8 source name"))?;
        let folded = name.to_ascii_lowercase();
        let Some(digits) = folded.strip_prefix("enshrouded_").and_then(|value| value.strip_suffix(".dat")) else { continue; };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) { continue; }
        ensure!(name == folded, "source DAT name uses noncanonical case");
        let index: u32 = digits.parse().context("source DAT index is not representable")?;
        ensure!(index <= u16::MAX as u32, "source DAT index exceeds KFC slot range");
        ensure!(name == format!("enshrouded_{index:03}.dat"), "source DAT name is not canonically padded");
        ordinary(&entry.path(), false).context("source DAT is not an ordinary file")?;
        ensure!(names.insert(name), "duplicate source DAT name");
    }
    Ok(names)
}

fn inventory(base: &Path) -> Result<BTreeMap<String, Stamp>> {
    fn visit(base: &Path, dir: &Path, out: &mut BTreeMap<String, Stamp>) -> Result<()> {
        ordinary(dir,true)?;
        for entry in fs::read_dir(dir)? {
            let p=entry?.path();let m=fs::symlink_metadata(&p)?;
            if m.is_dir() { ordinary(&p,true)?; visit(base,&p,out)?; }
            else { ordinary(&p,false)?;let name=p.strip_prefix(base)?.to_str().context("non UTF8 member")?.replace('\\',"/"); checked(base,&name)?;out.insert(name,stamp(&p)?); }
            ensure!(out.len()<=10000,"oversized environment");
        } Ok(())
    }
    let mut out=BTreeMap::new();visit(base,base,&mut out)?;Ok(out)
}

fn qualify(request: &Request) -> Result<(PathBuf, PathBuf, PathBuf, Vec<File>, BTreeMap<String, Stamp>)> {
    source_closure()?;
    ensure!(request.schema==1,"unsupported prepared identity");
    let source=root(&request.asset_input_root)?;let environment=root(&request.environment_root)?;let output=root(&request.output_root)?;
    ensure!(root(&request.target_root)?==source,"preparation must read its actual target, not retarget a foreign receipt");
    let development=root(Path::new("E:/"))?;
    ensure!(environment.starts_with(&development) && output.starts_with(&development),"writable roots must be on E");
    shroudforge_game_assets::transaction::validate_prepared_roots(&environment,&source,&output,STEM)?;
    ensure!(fs::read_dir(&output)?.next().is_none(),"output root must be fresh and empty");
    ensure!(!shroudforge_game_assets::transaction::directory(&source,STEM).exists(),"source transaction exists");
    ensure!(!request.inputs.is_empty() && request.inputs.len()<=10000 && request.environment_files.len()<=10000,"invalid input size");
    let mut ids=BTreeSet::new();
    for id in &request.selected_ids {
        ensure!(!id.is_empty() && id.len()<=128 && id.bytes().all(|c|c.is_ascii_alphanumeric() || b"_-".contains(&c)),"invalid selected id");
        ensure!(!id.to_ascii_lowercase().replace(['-','_'],"").contains("autoloot"),"AutoLoot excluded");
        ensure!(ids.insert(id),"duplicate selected id");
    }
    ensure!(!ids.is_empty(),"empty selection");
    ensure!(inventory(&environment)?==request.environment_files,"environment closure differs");
    let mut aliases=BTreeSet::new();let mut handles=Vec::new();let mut expected_assets=BTreeSet::new();
    for (name,value) in &request.inputs {
        ensure!(aliases.insert(name.to_ascii_lowercase()) && valid_digest(&value.sha256),"invalid or aliased input");
        if asset(name) || name=="enshrouded.exe" {
            let p=checked(&source,name)?;
            let mut options=OpenOptions::new();options.read(true);
            #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt; options.share_mode(1);}
            let mut file=options.open(&p)?;
            ensure!(stamp_file(&mut file)?==*value,"source input differs: {name}");
            handles.push(file);
            if asset(name) { expected_assets.insert(name.clone()); }
        } else {
            ensure!(name.starts_with("mods/") && request.environment_files.get(name)==Some(value),"target mod input is not generation-pinned");
            checked(&environment,name)?;
        }
    }
    for name in ["enshrouded.exe","enshrouded.kfc","enshrouded.kfc_resources"] {ensure!(request.inputs.contains_key(name),"missing base identity");}
    let source_dat_membership: BTreeMap<_, _> = request.inputs.iter()
        .filter(|(name, _)| dat_asset(name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let actual_dats = source_dat_inventory(&source)?;
    let expected_dat_names: BTreeSet<_> = source_dat_membership.keys().cloned().collect();
    ensure!(actual_dats==expected_dat_names,"source DAT membership differs");
    let actual_assets: BTreeSet<_> = actual_dats
        .into_iter()
        .chain(["enshrouded.kfc".to_owned(), "enshrouded.kfc_resources".to_owned()])
        .collect();
    ensure!(actual_assets==expected_assets,"source asset membership differs");
    for name in request.environment_files.keys().filter(|n|n.starts_with("mods/")) {
        ensure!(request.inputs.get(name)==request.environment_files.get(name),"unbound staged package member");
    }
    Ok((source,environment,output,handles,source_dat_membership))
}

fn validate_source_dat_output(
    source: &Path,
    expected: &BTreeMap<String, Stamp>,
    staged: &BTreeMap<String, Stamp>,
) -> Result<()> {
    let actual_names = source_dat_inventory(source)?;
    let expected_names: BTreeSet<_> = expected.keys().cloned().collect();
    ensure!(actual_names==expected_names,"source DAT membership changed during generation");
    let mut actual = BTreeMap::new();
    for name in actual_names {
        actual.insert(name.clone(),stamp(&checked(source,&name)?)?);
    }
    ensure!(&actual==expected,"source DAT identity changed during generation");
    for name in staged.keys().filter(|name| dat_asset(name)) {
        ensure!(!actual.contains_key(name),"generated DAT overlaps current source");
    }
    Ok(())
}

fn accept_prepared_stage(
    request: &Request,
    source: &Path,
    environment: &Path,
    output: &Path,
    stage: &Path,
    source_dat_membership: &BTreeMap<String, Stamp>,
) -> Result<Receipt> {
    let expected_stage=shroudforge_game_assets::transaction::directory(output,STEM);
    ensure!(stage==expected_stage.as_path(),"unexpected output stage");
    let all=inventory(stage)?;let mut staged=BTreeMap::new();
    for (name,value) in &all {
        if name=="prepared-only.json" {continue;}
        ensure!(asset(name) || name=="generated-inputs.json","unexpected generated member");
        ensure!(!dat_asset(name) || !source_dat_membership.contains_key(name),"generated DAT overlaps source");
        staged.insert(name.clone(),value.clone());
    }
    for name in ["enshrouded.kfc","enshrouded.kfc_resources","generated-inputs.json"] {ensure!(staged.contains_key(name),"incomplete prepared output");}
    let marker:(u32,String,String)=serde_json::from_slice(&fs::read(stage.join("prepared-only.json"))?)?;
    ensure!(marker==(1,source.to_string_lossy().into_owned(),output.to_string_lossy().into_owned()),"prepared marker identity differs");
    let inputs:(u32,String,(u64,String),(u64,String))=serde_json::from_slice(&fs::read(stage.join("generated-inputs.json"))?)?;
    let k=&request.inputs["enshrouded.kfc"];let r=&request.inputs["enshrouded.kfc_resources"];
    ensure!(inputs==(1,source.to_string_lossy().into_owned(),(k.bytes,k.sha256.clone()),(r.bytes,r.sha256.clone())),"generated preimage differs");
    for (name,value) in &request.inputs {
        if dat_asset(name) {continue;}
        let base=if asset(name)||name=="enshrouded.exe" {source} else {environment};
        ensure!(stamp(&checked(base,name)?)?==*value,"input changed during preparation");
    }
    validate_source_dat_output(source,source_dat_membership,&staged)?;
    let source_dat_membership_sha256=format!("{:x}",Sha256::digest(serde_json::to_vec(source_dat_membership)?));
    Ok(Receipt {
        schema:2,
        source_closure:source_closure()?.into(),
        request_sha256:format!("{:x}",Sha256::digest(serde_json::to_vec(request)?)),
        target_root:source.to_path_buf(),
        output_stage:stage.to_path_buf(),
        staged,
        prepared_marker:all["prepared-only.json"].clone(),
        source_dat_membership:source_dat_membership.clone(),
        source_dat_membership_sha256,
    })
}

pub fn prepare(request: &Request) -> Result<Receipt> {
    let (source,environment,output,_input_handles,source_dat_membership)=qualify(request)?;
    let env=shroudforge_package::ModEnvironment::load(environment.to_str().context("nonUTF8 environment")?).map_err(|e|anyhow::anyhow!("environment: {e}"))?;
    ensure!(root(env.mods_dir().as_std_path())?==environment.join("mods"),"configured mods escape environment");
    ensure!(root(env.cache_dir().as_std_path())?.starts_with(&environment),"configured cache escapes environment");
    let planned:BTreeSet<_>=env.plan(false,shroudforge_api::API_VERSION).iter().filter(|m|m.info().requires_pregame()).map(|m|m.info().id.clone()).collect();
    ensure!(planned==request.selected_ids.iter().cloned().collect(),"planned pregame selection differs");
    let stage=shroudforge_api::run_prepared_with_local_schema(&env,shroudforge_api::RunArgs {file_name:STEM.into(),options:shroudforge_api::RunOptions {current_asset_mods:Some(request.selected_ids.clone()),defer_current_commit:true,patch:true,skip_cache:true,force_patch:true,is_server:Some(false),phase:shroudforge_api::RuntimePhase::Pregame,..Default::default()}},&source,&output)?;
    accept_prepared_stage(request,&source,&environment,&output,&stage,&source_dat_membership)
}

#[cfg(test)]
mod tests;
