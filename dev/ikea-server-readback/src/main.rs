//! Finite resource projection. No EXE, DAT, original resource file, Lua or game execution.
//! The reviewed outer selects an exact request and owns process/time/memory limits.
use anyhow::{Context, Result, ensure};
use kfc::{
    container::{KFCFile, KFCReader},
    reflection::{LookupKey, TypeIndex, TypeRegistry},
    resource::value::{ConversionOptions, Value as ResourceValue, VariantOptions},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
const PRIVATE: &str = "D:/GameServers/staging/ikea-server-assets-20261105-r1";
const SOURCE: &str = "D:/GameServers/staging/ikea-game-update-20261105-r1/official";
const STEM: &str = "enshrouded_server";
const TYPES: &[&str] = &[
    "keen::ItemInfo",
    "keen::ecs::TemplateResource",
    "keen::BuffTypeCollectionResource",
    "keen::SceneResource",
    "keen::RecipeRegistryResource",
    "keen::ImpactProgram",
    "keen::ItemRegistryResource",
];
const MAX_REPORT: u64 = 256 * 1024 * 1024;
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn prepared_stage_matches(value: &str, expected: &Path) -> bool {
    // canonicalize() records this fixed local drive with the Windows verbatim
    // prefix. Compare only its two spellings; do not resolve aliases or dots.
    let value = value
        .strip_prefix(r"\\?\")
        .unwrap_or(value)
        .replace('\\', "/");
    value.eq_ignore_ascii_case(&expected.to_string_lossy().replace('\\', "/"))
}
fn verify_prefix(file: &mut File, bytes: u64, expected: &str) -> Result<()> {
    ensure!(
        file.metadata()?.len() >= bytes,
        "generated resource prefix truncated"
    );
    file.seek(SeekFrom::Start(0))?;
    let mut remaining = bytes;
    let mut buffer = [0u8; 65536];
    let mut digest = Sha256::new();
    while remaining > 0 {
        let amount = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..amount])?;
        digest.update(&buffer[..amount]);
        remaining -= amount as u64;
    }
    ensure!(
        format!("{:x}", digest.finalize()) == expected,
        "generated resource prefix differs"
    );
    Ok(())
}
fn directories(path: &Path) -> Result<Vec<File>> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let mut held = Vec::new();
    for p in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let f = OpenOptions::new()
            .access_mode(0x80000080)
            .share_mode(3)
            .custom_flags(0x02200000)
            .open(p)?;
        let m = f.metadata()?;
        ensure!(
            m.is_dir() && m.file_attributes() & 0x400 == 0,
            "ordinary directory required"
        );
        held.push(f);
    }
    ensure!(held.len() <= 32, "directory depth");
    Ok(held)
}
fn identity(f: &File) -> Result<[u32; 13]> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(h: *mut std::ffi::c_void, out: *mut std::ffi::c_void) -> i32;
    }
    let mut v = [0u32; 13];
    ensure!(
        unsafe { GetFileInformationByHandle(f.as_raw_handle(), v.as_mut_ptr().cast()) } != 0,
        "file identity"
    );
    ensure!(
        v[0] & 0x410 == 0 && v[10] == 1,
        "ordinary single-link file required"
    );
    Ok(v)
}
struct Held {
    f: File,
    path: PathBuf,
    id: [u32; 13],
    bytes: u64,
    sha: String,
}
impl Held {
    fn open(p: &Path, bytes: u64, expected: &str) -> Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        ensure!(bytes <= 2 * 1024 * 1024 * 1024, "input extent");
        let f = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(p)?;
        let id = identity(&f)?;
        ensure!(f.metadata()?.len() == bytes, "input length");
        let mut h = Self {
            f,
            path: p.into(),
            id,
            bytes,
            sha: expected.into(),
        };
        h.verify()?;
        Ok(h)
    }
    fn verify(&mut self) -> Result<()> {
        ensure!(identity(&self.f)? == self.id, "input identity drift");
        self.f.seek(SeekFrom::Start(0))?;
        let mut n = self.bytes;
        let mut b = [0u8; 65536];
        let mut s = Sha256::new();
        while n > 0 {
            let len = n.min(b.len() as u64) as usize;
            self.f.read_exact(&mut b[..len])?;
            s.update(&b[..len]);
            n -= len as u64;
        }
        ensure!(
            format!("{:x}", s.finalize()) == self.sha && identity(&self.f)? == self.id,
            "input hash drift"
        );
        use std::os::windows::fs::OpenOptionsExt;
        let p = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(&self.path)?;
        ensure!(identity(&p)? == self.id, "path identity drift");
        Ok(())
    }
    fn json(&mut self) -> Result<Value> {
        ensure!(self.bytes <= 32 * 1024 * 1024, "JSON extent");
        self.f.seek(SeekFrom::Start(0))?;
        let mut b = vec![0; self.bytes as usize];
        self.f.read_exact(&mut b)?;
        Ok(serde_json::from_slice(&b)?)
    }
}
fn annotate(v: &mut Value, r: &TypeRegistry, depth: usize) -> Result<()> {
    ensure!(depth <= 128, "value depth");
    match v {
        Value::Array(a) => {
            for v in a {
                annotate(v, r, depth + 1)?
            }
        }
        Value::Object(o) => {
            if let Some(i) = o.get("$type") {
                let t = r
                    .get(TypeIndex::new(i.as_u64().context("variant index")? as usize))
                    .context("variant type")?;
                o.insert("$typeName".into(), json!(t.qualified_name));
            }
            for v in o.values_mut() {
                annotate(v, r, depth + 1)?
            }
        }
        _ => {}
    }
    Ok(())
}
fn project(ty: &str, mut v: Value) -> Value {
    let fields: &[&str] = match ty {
        "keen::ItemInfo" => &[
            "itemId",
            "debugName",
            "maxStackSize",
            "equipment",
            "impactValues",
            "name",
            "category",
        ],
        "keen::SceneResource" => &["ibl", "noBuildZones"],
        "keen::ecs::TemplateResource" => &["name", "components"],
        _ => return v,
    };
    let mut out = serde_json::Map::new();
    for key in fields {
        if let Some(value) = v.get_mut(*key) {
            out.insert((*key).into(), value.take());
        }
    }
    if ty == "keen::ecs::TemplateResource" {
        if let Some(Value::Array(a)) = out.get_mut("components") {
            for c in a {
                if let Some(name) = c["$typeName"].as_str() {
                    if ![
                        "keen::ecs::InventoryCraftingStock",
                        "keen::ecs::InventorySetup",
                        "keen::ecs::EnemyScaling",
                        "keen::ecs::DamageMod",
                        "keen::ecs::Health",
                        "keen::ecs::Faction",
                    ]
                    .contains(&name)
                    {
                        *c = json!({"$typeName":name});
                    }
                }
            }
        }
    }
    Value::Object(out)
}
fn emit(f: &mut File, value: &Value, total: &mut u64) -> Result<()> {
    let b = serde_json::to_vec(value)?;
    *total = total
        .checked_add(b.len() as u64 + 1)
        .context("output overflow")?;
    ensure!(*total <= MAX_REPORT, "report cap");
    f.write_all(&b)?;
    f.write_all(b"\n")?;
    Ok(())
}
fn preserve_index(
    old: &KFCFile,
    new: &KFCFile,
    outputs: &serde_json::Map<String, Value>,
) -> Result<Value> {
    ensure!(
        new.containers().len() >= old.containers().len(),
        "original containers removed"
    );
    for (a, b) in old.containers().iter().zip(new.containers()) {
        ensure!(
            a.size == b.size && a.count == b.count,
            "original container metadata changed"
        );
    }
    let mut contents = 0usize;
    for (id, a) in old.contents().iter() {
        let b = new.contents().get(id).context("original content removed")?;
        ensure!(
            a.offset == b.offset && a.flags == b.flags && a.container_index == b.container_index,
            "original content mapping changed"
        );
        contents += 1;
    }
    let mut resources = 0usize;
    for (id, _) in old.resources().iter() {
        ensure!(
            new.resources().get(id).is_some(),
            "original resource removed"
        );
        resources += 1;
    }
    for (_, entry) in new.contents().iter() {
        ensure!(
            entry.container_index < new.containers().len(),
            "content outside containers"
        );
        if entry.container_index >= old.containers().len() {
            let name = format!("{STEM}_{:03}.dat", entry.container_index);
            let pin = outputs.get(&name).context("new content container absent")?;
            ensure!(
                pin["bytes"].as_u64() == Some(new.containers()[entry.container_index].size),
                "new container extent differs"
            );
        }
    }
    Ok(
        json!({"originalContainers":old.containers().len(),"originalContentsPreserved":contents,"originalResourcesPreserved":resources,"contentBytesDecoded":false}),
    )
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(args.len() == 2, "expected REQUEST_BYTES REQUEST_SHA256");
    let bytes: u64 = args[0].parse()?;
    ensure!(bytes <= 65536, "request extent");
    let root = Path::new(PRIVATE);
    let out = root.join("readback-r1");
    let stage = root.join("generated/.enshrouded_server.shroudforge-stage");
    let source = Path::new(SOURCE);
    let mut guards = directories(&out)?;
    guards.extend(directories(&stage)?);
    guards.extend(directories(source)?);
    guards.extend(directories(&root.join("controls"))?);
    guards.extend(directories(&root.join("evidence"))?);
    guards.extend(directories(&root.join("operation-r1"))?);
    let mut req = Held::open(
        &root.join("controls/READBACK-REQUEST.json"),
        bytes,
        &args[1],
    )?;
    let request = req.json()?;
    ensure!(
        request["kind"] == "ikea_generated_resource_projection_v1" && request["schema"] == 1,
        "request kind"
    );
    let keys: BTreeSet<_> = request
        .as_object()
        .context("request object")?
        .keys()
        .map(String::as_str)
        .collect();
    ensure!(
        keys == BTreeSet::from(["kind", "schema", "preparedReceipt", "readerSha256"]),
        "request fields"
    );
    let self_bytes = fs::read(std::env::current_exe()?)?;
    ensure!(
        request["readerSha256"] == hash(&self_bytes),
        "reader artifact"
    );
    let p = &request["preparedReceipt"];
    let mut receipt = Held::open(
        &root.join("operation-r1/PREPARED-RECEIPT.json"),
        p["bytes"].as_u64().context("receipt bytes")?,
        p["sha256"].as_str().context("receipt hash")?,
    )?;
    let prepared = receipt.json()?;
    ensure!(
        prepared["status"] == "PRIVATE_SERVER_STAGE_PENDING_INDEPENDENT_READBACK",
        "prepared status"
    );
    ensure!(
        prepared_stage_matches(prepared["stage"].as_str().context("stage")?, &stage),
        "prepared stage"
    );
    let mut inputs = Vec::new();
    let outputs = prepared["outputs"].as_object().context("outputs")?;
    ensure!((2..=32).contains(&outputs.len()), "outputs count");
    let mut output_bytes = 0u64;
    let names: BTreeSet<_> = fs::read_dir(&stage)?
        .map(|x| x.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    ensure!(
        names == outputs.keys().cloned().collect(),
        "stage membership"
    );
    for (name, p) in outputs {
        ensure!(
            !name.contains(['/', '\\', ':']) && name != "." && name != "..",
            "output leaf"
        );
        let n = p["bytes"].as_u64().context("output bytes")?;
        output_bytes = output_bytes.checked_add(n).context("extent")?;
        ensure!(output_bytes <= 2 * 1024 * 1024 * 1024, "stage extent");
        inputs.push(Held::open(
            &stage.join(name),
            n,
            p["sha256"].as_str().context("output hash")?,
        )?);
    }
    inputs.push(Held::open(
        &source.join(format!("{STEM}.kfc")),
        4263936,
        "76da3e72911775d34aaf6a3403c173068f14c1b3906be343633db3cf24191ee0",
    )?);
    let mut registry = Held::open(
        &root.join("evidence/types-enshrouded_server.json"),
        9196439,
        "fe74476f286d01fbd677b97abc6472ebb886313a7358ce0149cf1c6670bce4f3",
    )?;
    let raw = registry.json()?;
    let r: TypeRegistry = serde_json::from_value(raw.clone())?;
    ensure!(serde_json::to_value(&r)? == raw, "registry roundtrip");
    inputs.push(registry);
    let old = KFCFile::from_path(source.join(format!("{STEM}.kfc")), false)?;
    let new = KFCFile::from_path(stage.join(format!("{STEM}.kfc")), false)?;
    ensure!(
        old.game_version() == new.game_version() && new.game_version() == r.version,
        "generation differs"
    );
    let preservation = preserve_index(&old, &new, outputs)?;
    let prefix_path = stage.join(format!("{STEM}.kfc_resources"));
    let prefix = inputs
        .iter_mut()
        .find(|h| h.path == prefix_path)
        .context("held generated resource file")?;
    verify_prefix(
        &mut prefix.f,
        144056320,
        "f54f82d47545abe12ce5178e4302c78c88bcc948942b7d88a8f23d40ec4b0767",
    )?;
    let mut baseline = KFCReader::new_with_resource_path(
        source,
        STEM,
        stage.join(format!("{STEM}.kfc_resources")),
    )?
    .into_cursor()?;
    let mut current = KFCReader::new(&stage, STEM)?.into_cursor()?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out.join("RESOURCES.jsonl"))?;
    let mut total = 0;
    let mut decoded = 0u64;
    let mut count = 0usize;
    let mut counts = BTreeMap::new();
    emit(
        &mut f,
        &json!({"kind":"header","requestSha256":args[1],"preparedReceipt":p,"registrySha256":"fe74476f286d01fbd677b97abc6472ebb886313a7358ce0149cf1c6670bce4f3","strictSchemaQualification":false,"schemaDecision":"673f8cf8178e6cc35c3bc2ee2e8d478cae119e2119ca695cad1c6039d4fe133c","projectionOnly":true,"independentGeneratedPrefix":{"bytes":144056320,"sha256":"f54f82d47545abe12ce5178e4302c78c88bcc948942b7d88a8f23d40ec4b0767","verified":true}}),
        &mut total,
    )?;
    let mut selected = BTreeMap::new();
    for name in TYPES {
        let ty = r
            .get_by_name(LookupKey::Qualified(name))
            .context("required resource type")?;
        let ids: BTreeSet<_> = old
            .resources_by_type(ty.qualified_hash)
            .chain(new.resources_by_type(ty.qualified_hash))
            .copied()
            .collect();
        ensure!(!ids.is_empty(), "required type has no resources");
        selected.insert((*name).to_string(), ids);
    }
    // Include all additional changed/new resource kinds without reading unchanged
    // unrelated resource bodies. The scope comes from the already held indexes.
    for (id, entry) in new.resources().iter() {
        let changed = old
            .resources()
            .get(id)
            .is_none_or(|old| old.offset != entry.offset || old.size != entry.size);
        if changed {
            let ty = r
                .get_by_hash(LookupKey::Qualified(id.type_hash()))
                .context("changed resource type absent")?;
            selected
                .entry(ty.qualified_name.to_string())
                .or_insert_with(BTreeSet::new)
                .insert(*id);
        }
    }
    ensure!(selected.len() <= 128, "resource type count");
    for (name, ids) in &selected {
        let ty = r
            .get_by_name(LookupKey::Qualified(name))
            .context("selected type")?;
        counts.insert(name.clone(), ids.len());
        for id in ids {
            count += 1;
            ensure!(count <= 100000, "resource count");
            let mut values = Vec::new();
            let mut hashes = Vec::new();
            for reader in [&mut baseline, &mut current] {
                if let Some(n) = reader.resource_size(&id) {
                    ensure!(n <= 16 * 1024 * 1024, "resource extent");
                    decoded = decoded.checked_add(n).context("decode extent")?;
                    ensure!(decoded <= 4 * 1024 * 1024 * 1024, "decoded byte cap");
                    let mut b = Vec::new();
                    ensure!(
                        reader.read_resource_into(&id, &mut b)? && b.len() as u64 == n,
                        "resource read"
                    );
                    hashes.push(Some(hash(&b)));
                    let mut options = ConversionOptions::HUMAN_READABLE;
                    options.variant = VariantOptions::COMPACT;
                    let mut v = serde_json::to_value(ResourceValue::from_bytes_with_options(
                        &r, ty, &b, options,
                    )?)?;
                    annotate(&mut v, &r, 0)?;
                    values.push(v);
                } else {
                    hashes.push(None);
                    values.push(Value::Null);
                }
            }
            if name == "keen::ImpactProgram"
                && hashes[0] == hashes[1]
                && id.guid().to_string() != "91715e7c-46b6-4dae-9058-09780481a9bb"
            {
                continue;
            }
            if name != "keen::ecs::TemplateResource" || hashes[0] == hashes[1] {
                values = values
                    .into_iter()
                    .map(|v| if v.is_null() { v } else { project(name, v) })
                    .collect();
            }
            emit(
                &mut f,
                &json!({"kind":"resource","type":name,"guid":id.guid().to_string(),"part":id.part_index(),"baselineSha256":hashes[0],"currentSha256":hashes[1],"baseline":values[0],"current":values[1]}),
                &mut total,
            )?;
        }
    }
    req.verify()?;
    receipt.verify()?;
    for h in &mut inputs {
        h.verify()?;
    }
    let final_names: BTreeSet<_> = fs::read_dir(&stage)?
        .map(|x| x.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    ensure!(final_names == names, "stage membership drift");
    emit(
        &mut f,
        &json!({"kind":"complete","status":"BOUNDED_RESOURCE_PROJECTION_PENDING_SEMANTIC_REVIEW","counts":counts,"resourcesVisited":count,"decodedResourceBytes":decoded,"indexPreservation":preservation,"originalInputFiles":["enshrouded_server.kfc"],"originalExeDatResourceReads":0,"liveChanges":false}),
        &mut total,
    )?;
    f.sync_all()?;
    println!(
        "{}",
        json!({"status":"RESOURCE_PROJECTION_COMPLETE_PENDING_SEMANTICS","bytes":total,"resourcesVisited":count,"liveChanges":false})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_stage_accepts_only_fixed_drive_spellings() {
        let expected = Path::new(PRIVATE).join("generated/.enshrouded_server.shroudforge-stage");
        let plain = expected.to_string_lossy().replace('/', "\\");
        for accepted in [
            plain.clone(),
            format!(r"\\?\{plain}"),
            plain.to_lowercase(),
            expected.to_string_lossy().into_owned(),
        ] {
            assert!(prepared_stage_matches(&accepted, &expected), "{accepted}");
        }
        for rejected in [
            format!(r"\\.\{plain}"),
            format!(r"\\?\UNC\{plain}"),
            format!("//?/{}", expected.display()),
            plain.replacen("D:", "E:", 1),
            plain.replacen(r"D:\", "D:", 1),
            plain.replacen(r"\generated\", r"\other\..\generated\", 1),
            format!("{plain}:data"),
            format!("{plain}\\"),
            format!("{plain} "),
            format!(r"\\?\GLOBALROOT\{plain}"),
        ] {
            assert!(!prepared_stage_matches(&rejected, &expected), "{rejected}");
        }
    }
    fn temp() -> PathBuf {
        let p = PathBuf::from("E:/Build/ikea-server-readback-20261105-r1/fixtures").join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn prefix_accepts_tail_rejects_changed_and_short() {
        let root = temp();
        for (name, bytes, accepted) in [
            ("tail", b"abcappended".as_slice(), true),
            ("changed", b"abdappended".as_slice(), false),
            ("short", b"ab".as_slice(), false),
        ] {
            let path = root.join(name);
            fs::write(&path, bytes).unwrap();
            let mut file = File::open(&path).unwrap();
            assert_eq!(
                verify_prefix(&mut file, 3, &hash(b"abc")).is_ok(),
                accepted,
                "{}",
                name
            );
            if accepted {
                assert_eq!(file.stream_position().unwrap(), 3);
                assert_eq!(fs::read(path).unwrap(), bytes);
            }
        }
    }
    #[test]
    fn projection_preserves_false_zero_and_grapple_values() {
        let v = json!({"maxStackSize":0,"equipment":{"allowPlacementBelowFog":false},"impactValues":{"simple":[1,2]},"unrelated":7});
        let p = project("keen::ItemInfo", v);
        assert_eq!(
            p,
            json!({"maxStackSize":0,"equipment":{"allowPlacementBelowFog":false},"impactValues":{"simple":[1,2]}})
        );
    }
    #[test]
    fn template_projection_keeps_semantic_components() {
        let p = project(
            "keen::ecs::TemplateResource",
            json!({"name":"x","components":[{"$typeName":"keen::ecs::EnemyScaling","$value":{"health":2}},{"$typeName":"unrelated","$value":{"large":3}}]}),
        );
        assert_eq!(p["components"][0]["$value"]["health"], 2);
        assert!(p["components"][1].get("$value").is_none());
    }
    #[test]
    fn output_cap_refuses_before_write() {
        let p = temp().join("report");
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&p)
            .unwrap();
        let mut n = MAX_REPORT;
        assert!(emit(&mut f, &json!({"x":1}), &mut n).is_err());
        assert_eq!(f.metadata().unwrap().len(), 0);
    }
    #[test]
    fn held_files_deny_mutation_and_alias() {
        let p = temp().join("fixture");
        fs::write(&p, b"abc").unwrap();
        let mut h = Held::open(&p, 3, &hash(b"abc")).unwrap();
        assert!(OpenOptions::new().write(true).open(&p).is_err());
        assert!(fs::rename(&p, p.with_extension("moved")).is_err());
        h.verify().unwrap();
        drop(h);
        fs::hard_link(&p, p.with_extension("alias")).unwrap();
        assert!(Held::open(&p, 3, &hash(b"abc")).is_err());
    }
    #[test]
    fn invalid_variant_or_depth_refuses() {
        let r = TypeRegistry::default();
        assert!(annotate(&mut json!({"$type":0,"$value":{}}), &r, 0).is_err());
        assert!(annotate(&mut Value::Null, &r, 129).is_err());
    }
    #[test]
    fn content_dedup_preserved_but_remapping_rejected() {
        use kfc::{container::StaticMapBuilder, guid::ContentHash};
        fn one<T: Default>(_: &[T]) -> Vec<T> {
            vec![T::default()]
        }
        let key = ContentHash::new(10, 1, 2, 3);
        let make = |offset| {
            let mut f = KFCFile::default();
            let mut rows = one(f.contents().values());
            rows[0].offset = offset;
            let mut map = StaticMapBuilder::default();
            map.insert(key, rows.pop().unwrap());
            let mut containers = one(f.containers());
            containers[0].size = 100;
            containers[0].count = 1;
            f.set_containers(containers);
            f.set_contents(map.build());
            f
        };
        let old = make(0);
        let same = make(0);
        let outputs = serde_json::Map::new();
        assert!(preserve_index(&old, &same, &outputs).is_ok());
        assert!(preserve_index(&old, &make(10), &outputs).is_err());
        let mut changed = same.clone();
        let mut containers = changed.containers().to_vec();
        containers[0].size = 200;
        changed.set_containers(containers);
        assert!(preserve_index(&old, &changed, &outputs).is_err());
        assert!(preserve_index(&old, &KFCFile::default(), &outputs).is_err());
    }
}
