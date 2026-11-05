//! Private output only. No publication/recovery/backup or process-control calls.
use anyhow::{Context, Result, ensure};
use prepared_source_admission::{ReadLimits, SourceLease, SourcePlan, Stamp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const STEM: &str = "enshrouded_server";
const SOURCE: &str = "D:/GameServers/staging/ikea-game-update-20261105-r1/official";
pub(crate) const PRIVATE: &str = "D:/GameServers/staging/ikea-server-assets-20261105-r1";
const LOADER: &[u8] = br#"{"schemaVersion":1,"modules":{"modDetection":{"enabled":false}}}"#;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u32,
    pub kind: String,
    pub source_root: PathBuf,
    pub evidence_root: PathBuf,
    pub output_root: PathBuf,
    pub operation_root: PathBuf,
    pub sources: SourcePlan,
    pub source_read_limits: ReadLimits,
    pub registry: shroudforge_api::ServerRegistryInput,
}

fn hash(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}

fn fixed(path: &Path, expected: &str) -> Result<()> {
    ensure!(
        path.to_str()
            .context("path UTF8")?
            .replace('\\', "/")
            .eq_ignore_ascii_case(expected),
        "fixed private server path differs"
    );
    Ok(())
}

pub(crate) fn directory_chain(path: &Path) -> Result<Vec<File>> {
    #[cfg(not(windows))]
    {
        let _ = path;
        anyhow::bail!("Windows leases required");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let mut held = Vec::new();
        for p in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            let file = OpenOptions::new()
                .access_mode(0x80000080)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(p)?;
            let m = file.metadata()?;
            ensure!(
                m.is_dir() && m.file_attributes() & 0x400 == 0,
                "ordinary directory required"
            );
            held.push(file);
            ensure!(held.len() <= 32, "ancestor bound");
        }
        Ok(held)
    }
}

fn identity(file: &File) -> Result<Value> {
    #[cfg(not(windows))]
    {
        let _ = file;
        anyhow::bail!("Windows identity required");
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandle(
                h: *mut std::ffi::c_void,
                out: *mut std::ffi::c_void,
            ) -> i32;
        }
        let mut raw = [0u32; 13];
        ensure!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), raw.as_mut_ptr().cast()) }
                != 0,
            "file identity"
        );
        let m = file.metadata()?;
        ensure!(
            m.is_file() && m.file_attributes() & 0x400 == 0 && raw[10] == 1,
            "ordinary single-link input required"
        );
        Ok(
            json!({"bytes":m.len(),"fileIdentity":{"volumeSerial":raw[7],"fileIndexHigh":raw[11],"fileIndexLow":raw[12]},
            "observedMetadata":{"attributes":m.file_attributes(),"creationTime":m.creation_time(),"lastWriteTime":m.last_write_time()}}),
        )
    }
}

struct HeldFile {
    file: File,
    path: PathBuf,
    identity: Value,
    stamp: Stamp,
}
impl HeldFile {
    fn open(
        path: &Path,
        expected_bytes: u64,
        expected_hash: Option<&str>,
        expected_metadata: Option<&Value>,
    ) -> Result<Self> {
        let mut o = OpenOptions::new();
        o.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.share_mode(1).custom_flags(0x00200000);
        }
        let mut file = o.open(path)?;
        let id = identity(&file)?;
        ensure!(id["bytes"] == expected_bytes, "held input length differs");
        if let Some(expected) = expected_metadata {
            ensure!(
                id["fileIdentity"] == expected["fileIdentity"]
                    && id["observedMetadata"] == expected["observedMetadata"],
                "captured server file metadata differs"
            );
        }
        let stamp = Self::hash_file(&mut file, expected_bytes)?;
        if let Some(expected) = expected_hash {
            ensure!(stamp.sha256 == expected, "held input hash differs");
        }
        ensure!(identity(&file)? == id, "input changed while hashing");
        Ok(Self {
            file,
            path: path.into(),
            identity: id,
            stamp,
        })
    }
    fn hash_file(file: &mut File, bytes: u64) -> Result<Stamp> {
        file.seek(SeekFrom::Start(0))?;
        let mut h = Sha256::new();
        let mut remaining = bytes;
        let mut chunk = [0u8; 65536];
        while remaining > 0 {
            let amount = remaining.min(chunk.len() as u64) as usize;
            let n = file.read(&mut chunk[..amount])?;
            ensure!(n == amount, "short bounded source hash read");
            h.update(&chunk[..n]);
            remaining -= n as u64;
        }
        Ok(Stamp {
            bytes,
            sha256: format!("{:x}", h.finalize()),
        })
    }
    fn verify(&mut self) -> Result<()> {
        ensure!(
            identity(&self.file)? == self.identity,
            "held source identity changed"
        );
        let mut o = OpenOptions::new();
        o.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.share_mode(1).custom_flags(0x00200000);
        }
        ensure!(
            identity(&o.open(&self.path)?)? == self.identity,
            "source path changed"
        );
        ensure!(
            Self::hash_file(&mut self.file, self.stamp.bytes)? == self.stamp,
            "source content changed"
        );
        Ok(())
    }
}

fn dat_names(root: &Path) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for (i, entry) in fs::read_dir(root)?.enumerate() {
        ensure!(i < 128, "source namespace bound");
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("source name UTF8"))?;
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("enshrouded_server_") && lower.ends_with(".dat") {
            ensure!(
                lower == name && names.insert(name),
                "ambiguous source DAT name"
            );
        }
    }
    Ok(names)
}

pub(crate) fn write_new(path: &Path, value: &Value) -> Result<()> {
    let raw = serde_json::to_vec_pretty(value)?;
    ensure!(raw.len() <= 2 * 1024 * 1024, "receipt extent");
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&raw)?;
    file.sync_all()?;
    Ok(())
}

/// Called only after independent producer/operation admission in the CLI broker.
/// This function never accepts its own generated stage for installation.
pub fn prepare(request: &Request) -> Result<Value> {
    ensure!(
        request.schema == 1 && request.kind == "ikea_private_server_preparation_v1",
        "server protocol"
    );
    fixed(&request.source_root, SOURCE)?;
    fixed(
        &request.sources.environment_root,
        &format!("{PRIVATE}/environment"),
    )?;
    fixed(&request.evidence_root, &format!("{PRIVATE}/evidence"))?;
    fixed(&request.output_root, &format!("{PRIVATE}/generated"))?;
    fixed(&request.operation_root, &format!("{PRIVATE}/operation-r1"))?;
    ensure!(
        request.registry.main_acceptance.path == "MAIN-ACCEPTANCE.json"
            && request.registry.main_acceptance.bytes == 743
            && request.registry.main_acceptance.sha256
                == "08fb8baf49cc2033a214daa39bca0f35e1c8a6bc753f993f362fe1b51981efe5",
        "exact Main data acceptance required"
    );
    ensure!(
        request.registry.manifest.path == "MANIFEST.json"
            && request.registry.canonical_review.path == "CANONICAL-REVIEW.json",
        "fixed data control roles"
    );
    let mut directories = Vec::new();
    for path in [
        &request.source_root,
        &request.sources.environment_root,
        &request.evidence_root,
        &request.output_root,
        &request.operation_root,
    ] {
        directories.extend(directory_chain(path)?);
    }
    ensure!(
        fs::read_dir(&request.output_root)?.next().is_none(),
        "fresh empty private output required"
    );
    ensure!(
        !shroudforge_game_assets::transaction::directory(&request.source_root, STEM).exists(),
        "source transaction present"
    );
    // Hold every evidence leaf even after the SDK consumes and drops its opaque token.
    let expected_evidence: BTreeSet<String> = [
        "types-enshrouded_server.json",
        "types-enshrouded_server.identity.json",
        "PHASE-A-REPORT.json",
        "COMPLETE.json",
        "LOSS-LEDGER.json",
        "INPUTS.json",
        "CONSUMER-CONTRACT.md",
        "CONSUMER-SOURCE.json",
        "NEGATIVE-OBSERVATION.json",
        "RECIPE.json",
        "PLAN-CHECK.json",
        "MANIFEST.json",
        "MAIN-ACCEPTANCE.json",
        "CANONICAL-REVIEW.json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let actual_evidence: BTreeSet<_> = fs::read_dir(&request.evidence_root)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| std::io::Error::other("evidence name UTF8"))
        })
        .collect::<std::io::Result<_>>()?;
    ensure!(
        actual_evidence == expected_evidence,
        "evidence membership differs"
    );
    let mut evidence = Vec::new();
    let mut evidence_bytes = 0u64;
    for (i, entry) in fs::read_dir(&request.evidence_root)?.enumerate() {
        ensure!(i < 20, "evidence membership bound");
        let path = entry?.path();
        let bytes = fs::symlink_metadata(&path)?.len();
        evidence_bytes = evidence_bytes
            .checked_add(bytes)
            .context("evidence overflow")?;
        ensure!(evidence_bytes <= 32 * 1024 * 1024, "evidence extent");
        evidence.push(HeldFile::open(&path, bytes, None, None)?);
    }
    let registry =
        shroudforge_api::admit_ikea_server_registry(&request.evidence_root, &request.registry)?;
    let provenance = registry.provenance().clone();
    let recipe: BTreeMap<String, Stamp> = provenance["recipeFiles"]
        .as_object()
        .context("recipe files")?
        .iter()
        .map(|(name, p)| {
            Ok((
                name.clone(),
                Stamp {
                    bytes: p["bytes"].as_u64().context("recipe bytes")?,
                    sha256: p["sha256"].as_str().context("recipe hash")?.into(),
                },
            ))
        })
        .collect::<Result<_>>()?;
    ensure!(
        request.sources.raw_files == recipe
            && request.sources.replacements.is_empty()
            && !request.sources.allow_unknown_manifest_fields,
        "exact63 server recipe required"
    );
    ensure!(
        request.source_read_limits.max_bytes <= 32 * 1024 * 1024
            && request.source_read_limits.max_calls <= 2000,
        "source budget ceiling"
    );
    let mut sources =
        SourceLease::admit_ikea_server(request.sources.clone(), request.source_read_limits)?;
    let root = &request.sources.environment_root;
    let env = shroudforge_package::ModEnvironment::from_prepared_packages(
        root.to_str().context("root UTF8")?.into(),
        root.join("shroudforge/cache")
            .to_str()
            .context("cache UTF8")?
            .into(),
        root.join("mods").to_str().context("mods UTF8")?.into(),
        sources.package_views()?,
        sources.manifest_policy(),
    )
    .map_err(|e| anyhow::anyhow!("server package plan: {e:?}"))?;
    let (mods, issues) = env.plan_report(true, shroudforge_api::API_VERSION);
    let ids: Vec<String> = mods
        .iter()
        .filter(|m| m.info().requires_pregame())
        .map(|m| m.info().id.clone())
        .collect();
    ensure!(
        issues.is_empty() && serde_json::to_value(&ids)? == provenance["plannedIds"],
        "server ordered plan differs"
    );
    let generation = provenance["generation"]
        .as_str()
        .context("server generation")?;
    let controls = shroudforge_api::PreparedControls::from_admitted_bytes(Some(LOADER))
        .map_err(anyhow::Error::msg)?;
    ensure!(
        controls.manifest_policy() == sources.manifest_policy(),
        "prepared manifest policy differs"
    );
    let dats = provenance["datNamespace"]["members"]
        .as_array()
        .context("captured server namespace")?;
    ensure!(dats.len() == 10, "server DAT count");
    let names: BTreeSet<_> = dats
        .iter()
        .map(|r| r["name"].as_str().context("DAT name").map(str::to_owned))
        .collect::<Result<_>>()?;
    ensure!(
        dat_names(&request.source_root)? == names,
        "source namespace differs"
    );
    let mut assets = Vec::new();
    for row in provenance["sourceInputs"]
        .as_array()
        .context("source pins")?
    {
        let name = row["name"].as_str().context("source name")?;
        ensure!(
            [
                "enshrouded_server.exe",
                "enshrouded_server.kfc",
                "enshrouded_server.kfc_resources"
            ]
            .contains(&name),
            "unexpected source input"
        );
        assets.push(HeldFile::open(
            &request.source_root.join(name),
            row["bytes"].as_u64().context("source bytes")?,
            Some(row["sha256"].as_str().context("source hash")?),
            None,
        )?);
    }
    for row in dats {
        assets.push(HeldFile::open(
            &request.source_root.join(row["name"].as_str().unwrap()),
            row["bytes"].as_u64().context("DAT bytes")?,
            None,
            Some(row),
        )?);
    }
    let original_hash_bytes: u64 = assets.iter().map(|f| f.stamp.bytes).sum();
    ensure!(
        original_hash_bytes <= 9 * 1024 * 1024 * 1024,
        "source hash extent"
    );
    write_new(
        &request.operation_root.join("INPUTS-HELD.json"),
        &json!({"sourceTuple":provenance["sourceInputs"],
        "initialSourceHashes":assets.iter().map(|f|json!({"path":f.path,"identity":f.identity,"stamp":f.stamp})).collect::<Vec<_>>(),
        "datHashesFirstObservedHere":true,"sourceHashPassBytes":original_hash_bytes,"recipeSha256":provenance["recipeSha256"]}),
    )?;
    let args = shroudforge_api::RunArgs {
        file_name: STEM.into(),
        options: shroudforge_api::RunOptions {
            current_asset_mods: Some(ids.clone()),
            defer_current_commit: true,
            patch: true,
            skip_cache: true,
            force_patch: true,
            is_server: Some(true),
            phase: shroudforge_api::RuntimePhase::Pregame,
            ..Default::default()
        },
    };
    let generated = shroudforge_api::run_prepared_effective_sources(
        &env,
        args,
        &request.source_root,
        &request.output_root,
        registry,
        controls,
        generation,
    );
    let finalization = sources.finalize()?;
    let mut drift = Vec::new();
    for f in assets.iter_mut().chain(evidence.iter_mut()) {
        if let Err(e) = f.verify() {
            drift.push(format!("{}: {e:#}", f.path.display()));
        }
    }
    if dat_names(&request.source_root)? != names {
        drift.push("source DAT namespace changed".into());
    }
    let stage = match generated {
        Ok(stage) if finalization.verified && drift.is_empty() => stage,
        other => {
            write_new(
                &request.operation_root.join("REFUSAL.json"),
                &json!({"status":"SERVER_PREPARATION_REFUSED","generationError":other.err().map(|e|format!("{e:#}")),"sourceFinalization":finalization,"drift":drift,"liveChanges":false}),
            )?;
            anyhow::bail!("server preparation refused; retained evidence");
        }
    };
    shroudforge_game_assets::transaction::validate_generated_prepared(
        &request.source_root,
        &request.output_root,
        STEM,
    )?;
    let mut outputs = BTreeMap::new();
    let mut total = 0u64;
    for (i, entry) in fs::read_dir(&stage)?.enumerate() {
        ensure!(i < 32, "generated member bound");
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("generated UTF8"))?;
        let length = entry.metadata()?.len();
        total = total.checked_add(length).context("generated overflow")?;
        ensure!(total <= 2 * 1024 * 1024 * 1024, "generated extent");
        let held = HeldFile::open(&entry.path(), length, None, None)?;
        ensure!(!names.contains(&name), "generated DAT overlaps source");
        outputs.insert(name, held.stamp);
    }
    let receipt = json!({"status":"PRIVATE_SERVER_STAGE_PENDING_INDEPENDENT_READBACK","generation":generation,"plannedIds":ids,
        "sourceFinalization":finalization,"sourceHashPassBytes":original_hash_bytes,"originalSourceHashPasses":2,
        "assetRuntimeIO":"Not counted by hash-pass counter; separate reviewed producer/outer limits apply",
        "stage":stage,"outputs":outputs,"registryProvenance":provenance,"requestSha256":hash(&serde_json::to_vec(request)?),
        "nativeCompatibilityAccepted":false,"independentEffectsAccepted":false,"installationApproved":false,"liveChanges":false,"backupsCreated":false});
    write_new(
        &request.operation_root.join("PREPARED-RECEIPT.json"),
        &receipt,
    )?;
    Ok(receipt)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn fixture() -> PathBuf {
        let root =
            PathBuf::from("E:/Build/ikea-server-producer-fixtures-20261105-r1").join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn fixed_paths_refuse_live_roots_aliases_and_traversal() {
        assert!(fixed(Path::new(SOURCE), SOURCE).is_ok());
        for path in [
            "D:/GameServers/IKEA",
            "D:/GameServers/staging/ikea-game-update-20261105-r1/official/../official",
            "D:/Games/SteamLibrary/steamapps/common/Enshrouded",
            "//server/share/official",
        ] {
            assert!(fixed(Path::new(path), SOURCE).is_err());
        }
    }
    #[test]
    fn held_input_denies_writes_rename_and_checks_hash_before_use() {
        let root = fixture();
        let path = root.join("fixture.bin");
        fs::write(&path, b"known synthetic input").unwrap();
        assert!(HeldFile::open(&path, 21, Some(&"0".repeat(64)), None).is_err());
        let mut held =
            HeldFile::open(&path, 21, Some(&hash(b"known synthetic input")), None).unwrap();
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        assert!(fs::rename(&path, root.join("moved.bin")).is_err());
        held.verify().unwrap();
        drop(held);
        fs::rename(&path, root.join("moved.bin")).unwrap();
    }
    #[test]
    fn hardlinked_inputs_and_changed_captured_metadata_are_refused() {
        let root = fixture();
        let path = root.join("fixture.bin");
        fs::write(&path, b"abc").unwrap();
        let held = HeldFile::open(&path, 3, None, None).unwrap();
        let mut expected = held.identity.clone();
        drop(held);
        expected["observedMetadata"]["lastWriteTime"] = json!(0);
        assert!(HeldFile::open(&path, 3, None, Some(&expected)).is_err());
        fs::hard_link(&path, root.join("alias.bin")).unwrap();
        assert!(HeldFile::open(&path, 3, None, None).is_err());
    }
    #[test]
    fn directory_guard_blocks_rename_and_new_receipt_does_not_overwrite() {
        let root = fixture();
        let child = root.join("held");
        fs::create_dir(&child).unwrap();
        let held = directory_chain(&child).unwrap();
        assert!(fs::rename(&child, root.join("moved")).is_err());
        let receipt = child.join("RESULT.json");
        write_new(&receipt, &json!({"a":1})).unwrap();
        assert!(write_new(&receipt, &json!({"a":2})).is_err());
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(receipt).unwrap()).unwrap(),
            json!({"a":1})
        );
        drop(held);
        fs::rename(&child, root.join("moved")).unwrap();
    }
}
