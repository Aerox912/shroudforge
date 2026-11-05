//! Distinct private source/control-sealed preparation protocol. Legacy
//! request/receipt schemas are not reinterpreted as this protocol.
use super::*;
use prepared_source_admission::{ReadLimits, SourceFinalization, SourceLease, SourcePlan};
use serde_json::Value;
use std::io::Write;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveRequest {
    pub schema: u32,
    pub kind: String,
    pub base: Request,
    pub sources: SourcePlan,
    /// Exact non-package evidence closure, joined to the opaque verified
    /// registry token before source admission. Loader control is separate.
    pub registry_evidence_files: BTreeMap<String, prepared_source_admission::Stamp>,
    pub source_read_limits: ReadLimits,
    pub expected_load_order: Vec<String>,
    /// Exact full KFC game_version, including build, branch and timestamp when present.
    pub expected_generation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePin {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveUseApproval {
    pub schema: u32,
    pub kind: String,
    pub request_sha256: String,
    pub source_closure: EvidencePin,
    pub normal_artifact: Stamp,
    pub source_verdict: EvidencePin,
    pub operation_verdict: EvidencePin,
    pub base_candidate: EvidencePin,
    pub successor: EvidencePin,
    pub successor_verdict: EvidencePin,
    pub operation_directory: PathBuf,
}

/// Created only by the exact evidence join. Caller-created booleans cannot
/// activate this route. Held evidence survives receipt serialization.
pub struct VerifiedEffectiveUse {
    request_sha256: String,
    closure_sha256: String,
    artifact_sha256: String,
    operation_verdict_sha256: String,
    base_candidate_sha256: String,
    successor_sha256: String,
    operation_directory: PathBuf,
    _evidence: Vec<File>,
    _operation_directory_handle: File,
}

#[derive(Serialize)]
pub struct EffectiveReceipt {
    pub schema: u32,
    pub kind: &'static str,
    pub status: &'static str,
    pub request_sha256: String,
    pub source_closure_sha256: String,
    pub artifact_sha256: String,
    pub operation_verdict_sha256: String,
    pub base_candidate_sha256: String,
    pub successor_sha256: String,
    pub raw_inventory_sha256: String,
    pub effective_inventory_sha256: String,
    pub planned_ids: Vec<String>,
    pub generation: String,
    pub stage: Receipt,
    pub source_finalization: SourceFinalization,
    pub independent_postimage_accepted: bool,
    pub current_native_accepted: bool,
    pub installation_approved: bool,
}

pub struct EffectivePreparedOutput {
    receipt: EffectiveReceipt,
    _sources: SourceLease,
    _asset_handles: Vec<File>,
    approval: VerifiedEffectiveUse,
}

fn e_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir)),
        "plain absolute E path required"
    );
    let text = path.to_str().context("nonUTF8 evidence path")?;
    ensure!(
        text.len() >= 3
            && text.as_bytes()[0].eq_ignore_ascii_case(&b'e')
            && text.as_bytes()[1] == b':'
            && !text.starts_with("\\\\"),
        "E-only evidence/output"
    );
    for component in path.components() {
        if let Component::Normal(part) = component {
            let part = part.to_str().context("nonUTF8 path component")?;
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            ensure!(
                !part.ends_with([' ', '.'])
                    && !part
                        .chars()
                        .any(|c| c.is_control() || ":<>\"|?*".contains(c))
                    && !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                    && !(stem.len() == 4
                        && (stem.starts_with("COM") || stem.starts_with("LPT"))
                        && matches!(stem.as_bytes()[3], b'1'..=b'9')),
                "noncanonical evidence component"
            );
        }
    }
    Ok(())
}

fn pinned_json(pin: &EvidencePin, held: &mut Vec<File>) -> Result<Value> {
    e_path(&pin.path)?;
    ensure!(
        pin.bytes <= 1024 * 1024 && valid_digest(&pin.sha256),
        "bounded evidence identity"
    );
    for parent in pin.path.ancestors().skip(1) {
        ordinary(parent, true)?;
    }
    ordinary(&pin.path, false)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let mut file = options.open(&pin.path)?;
    ensure!(file.metadata()?.len() == pin.bytes, "evidence length");
    let mut bytes = vec![0; pin.bytes as usize];
    file.read_exact(&mut bytes)?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == pin.sha256,
        "evidence hash"
    );
    let value = serde_json::from_slice(&bytes)?;
    held.push(file);
    Ok(value)
}

pub fn read_protocol_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    e_path(path)?;
    for parent in path.ancestors().skip(1) {
        ordinary(parent, true)?;
    }
    ordinary(path, false)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let mut file = options.open(path)?;
    let size = file.metadata()?.len();
    ensure!(size <= 256 * 1024, "oversized effective protocol input");
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn limited_json(value: &impl Serialize) -> Result<Vec<u8>> {
    struct Limited(Vec<u8>);
    impl Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > 1024 * 1024 - self.0.len() {
                return Err(std::io::Error::other("effective metadata cap"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = Limited(Vec::new());
    serde_json::to_writer(&mut bytes, value)?;
    Ok(bytes.0)
}

fn record_refusal(
    directory: &Path,
    primary: &str,
    finalization: &SourceFinalization,
) -> Result<()> {
    let value = serde_json::json!({"kind":"effective_preparation_refused","primary":primary,"sourceFinalization":finalization,"installationApproved":false});
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("REFUSAL.json"))?
        .write_all(&limited_json(&value)?)?;
    Ok(())
}

fn admit_sources_with_refusal(
    plan: SourcePlan,
    limits: ReadLimits,
    directory: &Path,
) -> Result<SourceLease> {
    match SourceLease::admit(plan, limits) {
        Ok(lease) => Ok(lease),
        Err(primary) => {
            let failure =
                primary.downcast_ref::<prepared_source_admission::SourceAdmissionFailure>();
            let value = serde_json::json!({"kind":"effective_preparation_refused","phase":"source_admission",
                "primary":format!("{primary:#}"),
                "sourceAdmissionState":if failure.is_some(){"failed_lease_retained"}else{"not_admitted"},
                "sourceFinalization":failure.map(|f|&f.finalization),"installationApproved":false});
            // A typed failure owns its finalized partial lease throughout bounded
            // serialization and create-new IO. Early pre-admission errors carry
            // null finalization rather than invented complete counters.
            let recording = (|| -> Result<()> {
                let bytes = limited_json(&value)?;
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(directory.join("REFUSAL.json"))?
                    .write_all(&bytes)?;
                Ok(())
            })();
            match recording {
                Ok(()) => Err(primary),
                Err(recording) => Err(primary.context(format!(
                    "additionally failed to record source admission refusal: {recording:#}"
                ))),
            }
        }
    }
}

impl EffectiveRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 3 && self.kind == "sf178_prepared_effective_source_request_v1",
            "wrong effective request"
        );
        ensure!(
            self.base.schema == 2 && self.base.external_registry.is_some(),
            "effective preparation requires genuine external registry admission"
        );
        for name in [
            "enshrouded.exe",
            "enshrouded.kfc",
            "enshrouded.kfc_resources",
        ] {
            ensure!(
                self.base
                    .inputs
                    .get(name)
                    .is_some_and(|s| s.bytes > 0 && valid_digest(&s.sha256)),
                "missing current base source identity"
            );
        }
        self.sources.validate()?;
        self.validate_control_closure()?;
        ensure!(
            self.sources.environment_root == self.base.environment_root,
            "source/environment root differs"
        );
        ensure!(
            self.sources.raw_files.len() == self.base.environment_files.len()
                && self.sources.raw_files.iter().all(|(name, s)| self
                    .base
                    .environment_files
                    .get(name)
                    .is_some_and(|raw| raw.bytes == s.bytes && raw.sha256 == s.sha256)),
            "complete raw environment differs"
        );
        let ids: BTreeSet<_> = self.base.selected_ids.iter().collect();
        let order: BTreeSet<_> = self.expected_load_order.iter().collect();
        ensure!(
            !ids.is_empty()
                && ids.len() == self.base.selected_ids.len()
                && ids == order
                && order.len() == self.expected_load_order.len(),
            "selected/planned identity"
        );
        ensure!(
            self.base
                .selected_ids
                .iter()
                .all(|id| valid_selected_mod_id(id)),
            "excluded selected id"
        );
        ensure!(
            !self.expected_generation.is_empty()
                && self.expected_generation.len() <= 128
                && self.expected_generation.is_ascii()
                && !self.expected_generation.chars().any(char::is_control),
            "generation identity"
        );
        Ok(())
    }
    pub fn canonical_sha256(&self) -> Result<String> {
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(self)?)))
    }
    fn validate_control_closure(&self) -> Result<()> {
        let external = self
            .base
            .external_registry
            .as_ref()
            .context("explicit registry input required")?;
        ensure!(
            !self.registry_evidence_files.is_empty() && self.registry_evidence_files.len() <= 64,
            "explicit registry evidence closure bound"
        );
        for pin in [&external.manifest, &external.producer_use_approval] {
            ensure!(
                self.registry_evidence_files
                    .get(&pin.path)
                    .is_some_and(|s| s.bytes == pin.bytes && s.sha256 == pin.sha256),
                "registry input missing from explicit evidence closure"
            );
        }
        for (name, stamp) in &self.registry_evidence_files {
            ensure!(
                !name.starts_with("mods/")
                    && name != "shroudforge/config/modloader-config.json"
                    && self.sources.raw_files.get(name) == Some(stamp),
                "registry evidence/raw control identity differs"
            );
        }
        ensure!(
            self.sources
                .raw_files
                .keys()
                .all(|name| name.starts_with("mods/")
                    || name == "shroudforge/config/modloader-config.json"
                    || self.registry_evidence_files.contains_key(name)),
            "unrelated source outside package/control/evidence closure"
        );
        ensure!(
            self.sources
                .replacements
                .iter()
                .all(|r| r.logical_path.starts_with("mods/")),
            "D1 replacements must belong to the selected package projection"
        );
        Ok(())
    }
}

fn stamp_matches(value: &Value, bytes: u64, sha: &str) -> bool {
    value["bytes"].as_u64() == Some(bytes) && value["sha256"].as_str() == Some(sha)
}

fn validate_committed_source(source: &Value) -> Result<()> {
    let git_id = |value: &Value| {
        value.as_str().is_some_and(|s| {
            s.len() == 40
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    };
    ensure!(
        source["schema_version"] == 1
            && source["source_root"]["repository"] == "shroudforge"
            && git_id(&source["source_root"]["commit"])
            && git_id(&source["source_root"]["tree"]),
        "normal committed schema1 source receipt required"
    );
    let gitlinks = source["source_root"]["gitlinks"]
        .as_array()
        .context("committed gitlinks")?;
    let paths = [
        "kfc-parser",
        "modules/game-runtime/native/json",
        "modules/hotkeys/SDL",
    ];
    ensure!(
        gitlinks.len() == paths.len()
            && gitlinks
                .iter()
                .zip(paths)
                .all(|(v, path)| v["path"] == path && git_id(&v["commit"]) && git_id(&v["tree"])),
        "committed gitlink inventory differs"
    );
    let locks = source["lockfiles"]
        .as_array()
        .context("committed lockfiles")?;
    let paths = ["Cargo.lock", "dev/hdmap-prepared-generator/Cargo.lock"];
    ensure!(
        locks.len() == paths.len()
            && locks.iter().zip(paths).all(
                |(v, path)| v["path"] == path && v["sha256"].as_str().is_some_and(valid_digest)
            ),
        "committed lockfile inventory differs"
    );
    Ok(())
}

fn overlap(left: &Path, right: &Path) -> Result<bool> {
    let normalize = |path: &Path| -> Result<String> {
        Ok(path
            .to_str()
            .context("nonUTF8 root")?
            .trim_start_matches("\\\\?\\")
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase())
    };
    let left = normalize(left)?;
    let right = normalize(right)?;
    Ok(left == right
        || left.starts_with(&format!("{right}\\"))
        || right.starts_with(&format!("{left}\\")))
}

fn validate_package_projection(
    request: &EffectiveRequest,
    base: &Value,
    successor: &Value,
) -> Result<()> {
    let origin = base["environmentRoot"]
        .as_str()
        .context("candidate package origin root")?;
    e_path(Path::new(origin))?;
    let package_files: BTreeMap<_, _> = request
        .sources
        .raw_files
        .iter()
        .filter(|(name, _)| name.starts_with("mods/"))
        .collect();
    let raw = base["environmentFiles"]
        .as_object()
        .context("base environment")?;
    let effective = successor["effectiveEnvironmentFiles"]
        .as_object()
        .context("successor environment")?;
    ensure!(
        raw.len() == package_files.len()
            && effective.len() == raw.len()
            && successor["overrideCount"].as_u64()
                == Some(request.sources.replacements.len() as u64),
        "selected package projection membership differs"
    );
    for (name, stamp) in package_files {
        ensure!(
            raw.get(name)
                .is_some_and(|value| stamp_matches(value, stamp.bytes, &stamp.sha256)),
            "raw source differs from candidate"
        );
        let value = effective.get(name).context("missing effective member")?;
        if let Some(replacement) = request
            .sources
            .replacements
            .iter()
            .find(|r| &r.logical_path == name)
        {
            ensure!(
                value["kind"] == "OVERRIDE"
                    && value["path"].as_str().map(Path::new) == Some(replacement.path.as_path())
                    && stamp_matches(
                        value,
                        replacement.replacement.bytes,
                        &replacement.replacement.sha256
                    ),
                "override not D1-selected"
            );
        } else {
            ensure!(
                value["kind"] == "UNCHANGED_BASE_REFERENCE"
                    && value["relativePath"] == *name
                    && value["root"].as_str().map(Path::new)
                        == base["environmentRoot"].as_str().map(Path::new)
                    && stamp_matches(value, stamp.bytes, &stamp.sha256),
                "effective base reference differs"
            );
        }
    }
    Ok(())
}

pub(crate) fn validate_verified_registry_files(
    expected: &BTreeMap<String, prepared_source_admission::Stamp>,
    registry: &shroudforge_api::VerifiedExternalRegistry,
) -> Result<()> {
    ensure!(
        registry.evidence_files().len() == expected.len()
            && registry.evidence_files().iter().all(|(name, s)| expected
                .get(name)
                .is_some_and(|r| r.bytes == s.bytes && r.sha256 == s.sha256)),
        "verified registry differs from complete admitted evidence closure"
    );
    Ok(())
}

/// Join normal committed source/artifact acceptance, exact frozen candidate and
/// D1 successor, and a separately accepted operation IO/storage policy. None of
/// the test-only snapshot/old EMPTY/index/component receipts meet this gate.
pub fn admit_effective_use(
    request: &EffectiveRequest,
    approval: EffectiveUseApproval,
) -> Result<VerifiedEffectiveUse> {
    request.validate()?;
    ensure!(
        approval.schema == 1 && approval.kind == "sf178_effective_producer_use_v1",
        "wrong use approval"
    );
    let request_sha256 = request.canonical_sha256()?;
    ensure!(
        approval.request_sha256 == request_sha256,
        "use approval request differs"
    );
    let closure = source_closure()?;
    ensure!(
        approval.source_closure.sha256 == closure,
        "compiled closure differs"
    );
    let mut held = Vec::new();
    let source = pinned_json(&approval.source_closure, &mut held)?;
    validate_committed_source(&source)?;
    let executable = std::env::current_exe()?;
    e_path(&executable)?;
    ordinary(&executable, false)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let mut binary = options.open(&executable)?;
    ensure!(
        stamp_file(&mut binary)? == approval.normal_artifact,
        "normal executable differs"
    );
    held.push(binary);
    let review = pinned_json(&approval.source_verdict, &mut held)?;
    ensure!(
        review["status"] == "CANONICAL_ACCEPTED_EFFECTIVE_PREPARED_PRODUCER_SOURCE_ARTIFACT_ONLY"
            && review["sourceClosureSha256"] == closure
            && stamp_matches(
                &review["artifact"],
                approval.normal_artifact.bytes,
                &approval.normal_artifact.sha256
            ),
        "normal source/artifact canonical differs"
    );
    let operation = pinned_json(&approval.operation_verdict, &mut held)?;
    ensure!(
        operation["status"] == "CANONICAL_ACCEPTED_EFFECTIVE_PREPARED_OPERATION_IO_STORAGE_ONLY"
            && operation["requestSha256"] == request_sha256
            && operation["sourceClosureSha256"] == closure
            && operation["artifactSha256"] == approval.normal_artifact.sha256
            && operation["baseCandidateSha256"] == approval.base_candidate.sha256
            && operation["successorSha256"] == approval.successor.sha256
            && operation["noBackups"] == true
            && operation["mainWritesAllowed"] == false
            && operation["autoLootIncluded"] == false
            && operation["independentPostimageRequired"] == true
            && operation["operationDirectory"].as_str().map(Path::new)
                == Some(approval.operation_directory.as_path())
            && operation["generation"] == request.expected_generation
            && operation["generatedResourcePrefixBytes"].as_u64()
                == request
                    .base
                    .inputs
                    .get("enshrouded.kfc_resources")
                    .map(|s| s.bytes),
        "operation policy/candidate differs"
    );
    let base = pinned_json(&approval.base_candidate, &mut held)?;
    let successor = pinned_json(&approval.successor, &mut held)?;
    let canonical = pinned_json(&approval.successor_verdict, &mut held)?;
    ensure!(
        canonical["status"] == "CANONICAL_ACCEPTED_HD_D1_EXACT_SOURCE_SUCCESSOR_ONLY",
        "D1 source acceptance required"
    );
    ensure!(
        canonical["selectionIdentity"]["sha256"] == approval.successor.sha256
            && canonical["selectionIdentity"]["bytes"].as_u64() == Some(approval.successor.bytes),
        "D1 canonical binds a different successor"
    );
    ensure!(
        successor["originalCandidate"]["sha256"] == approval.base_candidate.sha256
            && serde_json::from_value::<Vec<String>>(base["selectedIds"].clone())?
                == request.base.selected_ids
            && base["selectedIds"] == successor["selectedIds"],
        "base/successor selected ids"
    );
    validate_package_projection(request, &base, &successor)?;
    e_path(&approval.operation_directory)?;
    let parent = approval
        .operation_directory
        .parent()
        .context("operation evidence parent")?;
    let canonical_parent = super::root(parent)?;
    let canonical_operation = canonical_parent.join(
        approval
            .operation_directory
            .file_name()
            .context("operation directory name")?,
    );
    for root in [
        &request.base.environment_root,
        &request.base.output_root,
        &request.base.asset_input_root,
    ] {
        ensure!(
            !overlap(&canonical_operation, &super::root(root)?)?,
            "operation evidence root overlaps inputs/output"
        );
    }
    ensure!(
        approval
            .operation_directory
            .parent()
            .is_some_and(Path::is_dir),
        "operation evidence parent"
    );
    fs::create_dir(&approval.operation_directory)?;
    let mut directory_options = OpenOptions::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        directory_options
            .access_mode(0x80)
            .share_mode(3)
            .custom_flags(0x02200000);
    }
    #[cfg(not(windows))]
    {
        directory_options.read(true);
    }
    let directory_handle = directory_options.open(&approval.operation_directory)?;
    ordinary(&approval.operation_directory, true)?;
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(approval.operation_directory.join("CONSUMED"))?;
    marker.write_all(request_sha256.as_bytes())?;
    Ok(VerifiedEffectiveUse {
        request_sha256,
        closure_sha256: closure.into(),
        artifact_sha256: approval.normal_artifact.sha256,
        operation_verdict_sha256: approval.operation_verdict.sha256,
        base_candidate_sha256: approval.base_candidate.sha256,
        successor_sha256: approval.successor.sha256,
        operation_directory: approval.operation_directory,
        _evidence: held,
        _operation_directory_handle: directory_handle,
    })
}

fn prepared_mod_environment(
    environment: &Path,
    sources: &SourceLease,
) -> Result<shroudforge_package::ModEnvironment> {
    shroudforge_package::ModEnvironment::from_prepared_packages(
        environment.to_str().context("environment UTF8")?.into(),
        environment.join("shroudforge/cache").to_str().context("cache UTF8")?.into(),
        environment.join("mods").to_str().context("mods UTF8")?.into(),
        sources.package_views()?,
        sources.manifest_policy(),
    ).map_err(|report| {
        let mut details = Vec::new();
        if let Some(error) = report.error { details.push(error.to_string()); }
        details.extend(report.mods.iter().map(|error| format!("{}: {}", error.path, error.error)));
        anyhow::anyhow!("prepared environment: {}", details.join("; "))
    })
}

/// Only stages on E. This does not accept a postimage, native profile, install
/// or launch. The returned object retains both source graphs until serialized.
pub fn prepare_effective(
    request: &EffectiveRequest,
    approval: VerifiedEffectiveUse,
) -> Result<EffectivePreparedOutput> {
    request.validate()?;
    ensure!(
        request.canonical_sha256()? == approval.request_sha256,
        "request changed after use admission"
    );
    let external = request
        .base
        .external_registry
        .as_ref()
        .context("external registry")?;
    let stamps = request
        .base
        .inputs
        .iter()
        .filter(|(name, _)| {
            [
                "enshrouded.exe",
                "enshrouded.kfc",
                "enshrouded.kfc_resources",
            ]
            .contains(&name.as_str())
        })
        .map(|(name, s)| {
            (
                name.clone(),
                shroudforge_api::SourceStamp {
                    bytes: s.bytes,
                    sha256: s.sha256.clone(),
                },
            )
        })
        .collect();
    let registry = shroudforge_api::admit_external_registry(
        &request.base.environment_root,
        external,
        &stamps,
    )?;
    validate_verified_registry_files(&request.registry_evidence_files, &registry)?;
    let mut sources = admit_sources_with_refusal(
        request.sources.clone(),
        request.source_read_limits,
        &approval.operation_directory,
    )?;
    let mut asset_handles = Vec::new();
    let result = (|| -> Result<Receipt> {
        let loader =
            sources.read_control("shroudforge/config/modloader-config.json", 1024 * 1024)?;
        let controls = shroudforge_api::PreparedControls::from_admitted_bytes(loader.as_deref())
            .and_then(|controls| controls.with_generation(&request.expected_generation))
            .map_err(anyhow::Error::msg)?;
        ensure!(
            controls.manifest_policy() == sources.manifest_policy(),
            "manifest/control detection policy differs"
        );
        // Held views retain the sealed request's path spelling. Windows
        // canonicalization adds a verbatim prefix, which is not lexically
        // equal to those roots. Resolve the prepared package plan before any
        // original hashing, using that same admitted source root throughout.
        let env = prepared_mod_environment(&request.sources.environment_root, &sources)?;
        let (planned, issues) = env.plan_report(false, shroudforge_api::API_VERSION);
        ensure!(
            issues.is_empty(),
            "effective plan issues: {}",
            issues.join("; ")
        );
        let ids: Vec<_> = planned
            .into_iter()
            .filter(|m| m.info().requires_pregame())
            .map(|m| m.info().id.clone())
            .collect();
        ensure!(
            ids == request.expected_load_order,
            "effective ordered plan differs"
        );
        let (source, environment, output, handles, dats) = qualify(&request.base)?;
        asset_handles = handles;
        let args = shroudforge_api::RunArgs {
            file_name: STEM.into(),
            options: shroudforge_api::RunOptions {
                current_asset_mods: Some(request.base.selected_ids.clone()),
                defer_current_commit: true,
                patch: true,
                skip_cache: true,
                force_patch: true,
                is_server: Some(false),
                phase: shroudforge_api::RuntimePhase::Pregame,
                ..Default::default()
            },
        };
        let stage = shroudforge_api::run_prepared_effective_sources(
            &env,
            args,
            &source,
            &output,
            registry,
            controls,
            &request.expected_generation,
        )?;
        accept_prepared_stage(&request.base, &source, &environment, &output, &stage, &dats)
    })();
    let finalization = sources.finalize()?;
    match result {
        Err(primary) => {
            if let Err(recording) = record_refusal(
                &approval.operation_directory,
                &format!("{primary:#}"),
                &finalization,
            ) {
                return Err(primary.context(format!(
                    "additionally failed to record refusal: {recording:#}"
                )));
            }
            Err(primary)
        }
        Ok(stage) => {
            if !finalization.verified {
                record_refusal(
                    &approval.operation_directory,
                    "complete source finalization failed",
                    &finalization,
                )
                .context(
                    "complete source finalization failed; additionally failed to record refusal",
                )?;
                anyhow::bail!("complete source finalization failed");
            }
            let receipt = EffectiveReceipt {
                schema: 4,
                kind: "sf178_prepared_effective_source_receipt_v1",
                status: "SOURCE_STAGE_ONLY_PENDING_INDEPENDENT_ACCEPTANCE",
                request_sha256: approval.request_sha256.clone(),
                source_closure_sha256: approval.closure_sha256.clone(),
                artifact_sha256: approval.artifact_sha256.clone(),
                operation_verdict_sha256: approval.operation_verdict_sha256.clone(),
                base_candidate_sha256: approval.base_candidate_sha256.clone(),
                successor_sha256: approval.successor_sha256.clone(),
                raw_inventory_sha256: request.sources.raw_inventory_sha256.clone(),
                effective_inventory_sha256: request.sources.effective_inventory_sha256.clone(),
                planned_ids: request.expected_load_order.clone(),
                generation: request.expected_generation.clone(),
                stage,
                source_finalization: finalization,
                independent_postimage_accepted: false,
                current_native_accepted: false,
                installation_approved: false,
            };
            Ok(EffectivePreparedOutput {
                receipt,
                _sources: sources,
                _asset_handles: asset_handles,
                approval,
            })
        }
    }
}

impl EffectivePreparedOutput {
    /// Bound serialization occurs while immutable source/asset/evidence leases
    /// are still owned. Never return a legacy receipt with invented provenance.
    pub fn write_receipt(self) -> Result<EvidencePin> {
        let bytes = limited_json(&self.receipt)?;
        let path = self
            .approval
            .operation_directory
            .join("EFFECTIVE-RECEIPT.json");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?
            .write_all(&bytes)?;
        Ok(EvidencePin {
            path,
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        })
    }
}

#[cfg(all(test, windows))]
#[path = "effective_tests.rs"]
mod tests;
