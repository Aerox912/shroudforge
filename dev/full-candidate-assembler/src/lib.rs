//! E-only assembly of a reviewable, noninstallable ShroudForge full-runtime candidate.
//! This crate never reads the target root and never performs an install transaction.
use anyhow::{Context, Result, bail, ensure};
use hdmap_prepared_generator::{Receipt as AssetReceipt, Request as AssetRequest, Stamp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sf_xhl_tree_adapter::{ApprovedPlan, PreparedReceipt as XhlReceipt};
use sha2::{Digest, Sha256};
use shroudforge_api::{
    API_VERSION,
    current_assets::full_runtime::{Contract, NativeFile},
};
use shroudforge_package::{ModEnvironment, dll_manager, parse_manifest};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_FILES: usize = 10_000;
const MAX_FILE_BYTES: u64 = 1_073_741_824;
const MAX_TOTAL_BYTES: u64 = 1_610_612_736;
const MAX_CONTRACT_FILES: usize = 132;
const MAX_RUNTIME_MODS: usize = 256;
const MAX_SCRIPTS: usize = 4096;
const REQUIRED_FEATURES: usize = 20;
const REQUIRED_ASSET_PROVIDERS: usize = 13;
const MAX_DEPLOYMENT_UNITS: usize = 64;
const PRESERVED_PATCH_CONFIG_PROVIDERS: [(&str, u8); 2] =
    [("xhl-rested-enhanced", 13), ("xhl-workshop-speed-20x", 14)];
const PRESERVED_PATCH_CONFIG_FIELDS: [&str; 3] = [
    "all original config bytes and actual file identity",
    "effective values equal immutable generation",
    "exact leaf inventory preserved; no action",
];
const PRESERVED_PATCH_CONFIG_DISPOSITION: &str = "Preserve observed target config unchanged; requiredPoststate is a desired installation condition, not an observed after-state. No payload write or runtime-contract binding. Controller must freshly validate exact leaf before/after and reject every action touching it.";
use shroudforge_api::current_assets::greeneye_profiles;
#[cfg(test)] const GREENEYE_PROFILE_ID: &str = greeneye_profiles::LEGACY.id;
#[cfg(test)] const GREENEYE_PROFILE_VERSION: u32 = greeneye_profiles::LEGACY.version;
#[cfg(test)] const GREENEYE_PROFILE_SHA256: &str = greeneye_profiles::LEGACY.definition_sha256;
#[cfg(test)] const GREENEYE_HOST_BYTES: u64 = greeneye_profiles::LEGACY.host_bytes;
#[cfg(test)] const GREENEYE_HOST_SHA256: &str = greeneye_profiles::LEGACY.host_sha256;
const XHL_CORE_PATH: &str = "mods/xhl-core/xhl-core.dll";
const XHL_CHAIN_PATH: &str = "mods/xhl-core/dinput8-chain.ini";
const XHL_SCAN_PREFIX: &str = "mods/xhl-core/mods/";
const POLICY_NAME: &str = "shroudforge-full-runtime.json";
const CACHE_IDENTITY_RELATIVE: &str = "shroudforge/cache/types-enshrouded.identity.json";
const CACHE_REGISTRY_RELATIVE: &str = "shroudforge/cache/types-enshrouded.json";
const CACHE_IDENTITY_BYTES: u64 = 187;
const CACHE_IDENTITY_SHA256: &str =
    "1beb89917f807c5183a0bb3cb1f5a61a4bb932e9cdf176d938fe8ede97b9a39d";
const CACHE_REGISTRY_BYTES: u64 = 22_570_549;
const CACHE_REGISTRY_SHA256: &str =
    "dcbc2c1374ddc528a51c1d424219047180f380f8ea61fe3db11e6e738f35c852";
const CACHE_BINDING_SHA256: &str =
    "e1c08ae2a1bdf01154225456cf9587c8cdb755c48690996a64bea1e611dc47d7";
const CACHE_VERDICT_SHA256: &str =
    "590a1fa95c547c6cb77cd9236aa31267c2ba976b187d6348b65848317927074c";
const CACHE_EXE_SHA256: &str = "0403981fd2e8ad18bd8c5698dff03c8d505b5a46677832c496db9c645513f2a6";
const CACHE_GENERATOR_EXE_SHA256: &str =
    "d7043ab668c5bdeeda6c5ffdd2913d863a8003331ae9516aa7ed9a8fa27b8e94";
const CACHE_EXTRACTOR_ID: &str = "kfc-a404b15cde40fc7bd43d812bb460ce40d2a9d3b1";

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn stamp(path: &Path) -> Result<Stamp> {
    let metadata = ordinary(path, false)?;
    ensure!(
        metadata.len() <= MAX_FILE_BYTES,
        "source file exceeds the 1 GiB bound: {}",
        path.display()
    );
    let mut file = File::open(path).with_context(|| format!("open source {}", path.display()))?;
    stamp_reader(&mut file)
}
fn stamp_reader(reader: &mut impl Read) -> Result<Stamp> {
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .context("file size overflow")?;
        ensure!(bytes <= MAX_FILE_BYTES, "file exceeds the 1 GiB bound");
        hash.update(&buffer[..count]);
    }
    Ok(Stamp {
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}
fn ordinary(path: &Path, directory: bool) -> Result<fs::Metadata> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "reparse/link source is forbidden: {}",
        path.display()
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "reparse source is forbidden: {}",
            path.display()
        );
    }
    ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "wrong source kind: {}",
        path.display()
    );
    Ok(metadata)
}
fn is_e_path(path: &Path) -> bool {
    path.is_absolute() && is_e_text(&path.to_string_lossy())
}
fn is_e_text(value: &str) -> bool {
    let raw = value.replace('/', "\\");
    let text = raw.strip_prefix("\\\\?\\").unwrap_or(&raw);
    text.len() >= 3 && text.as_bytes()[0].eq_ignore_ascii_case(&b'E') && &text[1..3] == ":\\"
}
fn validate_e_path(path: &Path, existing: bool, directory: bool) -> Result<()> {
    ensure!(
        is_e_path(path),
        "all source and output paths must be on E: {}",
        path.display()
    );
    ensure!(
        !path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
        "noncanonical path: {}",
        path.display()
    );
    #[cfg(windows)]
    {
        let mut components = path.components();
        let prefix = components.next().context("E: path has no drive prefix")?;
        ensure!(
            matches!(prefix, Component::Prefix(_)),
            "E: path does not start with a drive prefix: {}",
            path.display()
        );
        let root = components.next().context("E: path has no drive root")?;
        ensure!(
            matches!(root, Component::RootDir),
            "E: path is not rooted: {}",
            path.display()
        );

        // A Windows drive prefix (`E:` or `\\?\E:`) is syntax, not a
        // filesystem ancestor. Probe the complete drive root before walking
        // each existing directory below it, so reparse ancestors are still
        // rejected without treating `\\?\E:` as a missing path.
        let mut cursor = PathBuf::new();
        cursor.push(prefix.as_os_str());
        cursor.push(root.as_os_str());
        ensure!(
            cursor.is_absolute() && is_e_path(&cursor),
            "invalid E: drive root: {}",
            cursor.display()
        );
        match fs::symlink_metadata(&cursor) {
            Ok(_) => {
                ordinary(&cursor, true)
                    .with_context(|| format!("unsafe E: path component {}", cursor.display()))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ensure!(
                    !existing,
                    "required E: path is missing: {}",
                    cursor.display()
                );
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect E: drive root {}", cursor.display()));
            }
        }

        for component in components {
            ensure!(
                matches!(component, Component::Normal(_)),
                "noncanonical E: path component in {}",
                path.display()
            );
            cursor.push(component.as_os_str());
            match fs::symlink_metadata(&cursor) {
                Ok(_) => {
                    let is_leaf = cursor == path;
                    ordinary(&cursor, if is_leaf { directory } else { true }).with_context(
                        || format!("unsafe E: path component {}", cursor.display()),
                    )?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    ensure!(
                        !existing,
                        "required E: path is missing: {}",
                        cursor.display()
                    );
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("inspect E: path component {}", cursor.display())
                    });
                }
            }
        }
    }
    #[cfg(not(windows))]
    bail!("E: path validation is only supported on Windows");
    if existing {
        ordinary(path, directory)?;
    }
    Ok(())
}

/// Read a CLI input only after validating its entire E: ancestry.
pub fn read_input_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    validate_e_path(path, true, false)?;
    ensure!(
        fs::metadata(path)?.len() <= limit,
        "input exceeds size bound"
    );
    fs::read(path).with_context(|| format!("read {}", path.display()))
}
fn safe_relative(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && !value.contains(['\\', ':', '\0']),
        "unsafe relative path: {value}"
    );
    ensure!(
        !value.starts_with('/') && !value.ends_with('/'),
        "unsafe relative path: {value}"
    );
    let components: Vec<_> = value.split('/').collect();
    ensure!(
        components.iter().all(|part| !part.is_empty()
            && *part != "."
            && *part != ".."
            && !part.ends_with([' ', '.'])
            && !part
                .chars()
                .any(|c| c.is_control() || "<>\"|?*".contains(c))),
        "unsafe relative path: {value}"
    );
    ensure!(
        Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "unsafe relative path: {value}"
    );
    Ok(())
}
fn target_path(root: &Path, relative: &str) -> Result<PathBuf> {
    safe_relative(relative)?;
    ensure!(
        root.is_absolute()
            && !root
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
        "target root must be a canonical absolute path"
    );
    let path = root.join(relative.replace('/', "\\"));
    ensure!(
        !path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
        "target-relative path escapes target root"
    );
    Ok(path)
}
fn target_root(value: &Path) -> Result<()> {
    normalize_absolute(value).context("target root must be a canonical native drive path")?;
    // Deliberately lexical only: target roots may be the live game path and must not be probed.
    Ok(())
}
fn check_stamp(actual: &Stamp, expected: &Stamp, what: &str) -> Result<()> {
    ensure!(
        valid_digest(&expected.sha256) && actual == expected,
        "content stamp differs for {what}"
    );
    Ok(())
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ordinary(path, false)?;
    let size = fs::metadata(path)?.len();
    ensure!(size <= limit, "input too large: {}", path.display());
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() as u64 == size,
        "input changed while reading: {}",
        path.display()
    );
    Ok(bytes)
}
fn check_raw_evidence(path: &Path, expected: &str, limit: u64) -> Result<Vec<u8>> {
    validate_e_path(path, true, false)?;
    ensure!(valid_digest(expected), "invalid evidence digest");
    let bytes = read_bounded(path, limit)?;
    ensure!(
        digest(&bytes) == expected,
        "evidence digest differs: {}",
        path.display()
    );
    Ok(bytes)
}
fn safe_id(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn excluded_identity(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase().replace(['-', '_', '.'], "");
    normalized.contains("autoloot")
        || normalized.contains("shroudtopia")
        || normalized.contains("standaloneflight")
        || normalized == "unlimitedflight"
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidencePin {
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DerivedEnvironmentEvidence {
    pub bridge_manifest: EvidencePin,
    pub tester_verdict: EvidencePin,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentKind {
    AssetProvider,
    RuntimeOnly,
    RuntimeService,
    NativeOnly,
    XhlNative,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectedFeature {
    pub row: u8,
    pub name: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectedComponent {
    pub feature_rows: Vec<u8>,
    pub dependencies: Vec<String>,
    pub id: String,
    pub version: String,
    pub kind: ComponentKind,
    pub enabled: bool,
    pub package_directory: Option<String>,
    pub service_id: Option<String>,
    pub xhl_paths: Vec<String>,
    pub requires_greeneye: bool,
    pub target: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExclusionKind {
    Autoloot,
    Shroudtopia,
    StandaloneFlight,
    DuplicateFlight,
    OldMinimapHost,
    ServerOnly,
    LegacyFallback,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Exclusion {
    pub kind: ExclusionKind,
    pub ids: Vec<String>,
    pub paths: Vec<String>,
    pub reason: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PackageOrigin {
    AssetEnvironment,
    ApprovedE,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntendedOverlay {
    pub path: String,
    pub source: PathBuf,
    pub stamp: Stamp,
    pub replaces: bool,
    pub evidence: Vec<EvidencePin>,
    pub fields: Vec<String>,
    pub disposition: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageInput {
    pub component_id: String,
    pub package_directory: String,
    pub origin: PackageOrigin,
    pub source_root: Option<PathBuf>,
    pub source_files: BTreeMap<String, Stamp>,
    pub overlays: Vec<IntendedOverlay>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootFileInput {
    pub component_id: String,
    pub source: PathBuf,
    pub target: String,
    pub stamp: Stamp,
    pub runtime_binding: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeSelection {
    pub boot_services: Vec<String>,
    pub process_role: String,
    pub dedicated_guest_qualified: bool,
    pub allow_unverified_cursor_gate: bool,
    pub flight_transport_enabled: bool,
    pub vein_selected: bool,
    pub vein_settings_path: Option<String>,
    pub overlay: bool,
    pub first_person: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreservationEntry {
    pub component_id: String,
    pub target: String,
    pub evidence_sha256: String,
    pub fields: Vec<String>,
    pub disposition: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreservedRuntimeFile {
    pub component_id: String,
    pub target: String,
    pub stamp: Stamp,
    pub evidence: EvidencePin,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchFileIdentity {
    pub volume: u64,
    pub id: [u8; 16],
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchFilePin {
    pub bytes: u64,
    pub sha256: String,
    pub identity: PatchFileIdentity,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestedPatchEffectiveValues {
    pub enabled: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkshopPatchEffectiveValues {
    pub enabled: bool,
    pub multiplier: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PreservedPatchEffectiveValues {
    Rested(RestedPatchEffectiveValues),
    Workshop(WorkshopPatchEffectiveValues),
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreservedPatchConfig {
    /// Selected deployment-unit ID, resolved to its pinned package below.
    pub component_id: String,
    /// Case-insensitive target path. Actual directory spelling comes from the target evidence.
    pub target: String,
    /// Exact generated source-file stamp from the complete package inventory.
    pub source_stamp: Stamp,
    /// Finite selected semantic values, checked against the source and target projections.
    pub effective_values: PreservedPatchEffectiveValues,
    /// Actual target state observed before installation, including full FileIdInfo.
    pub observed_target_before: PatchFilePin,
    /// Required post-install state, equal to the before pin; this is a desired postcondition,
    /// not evidence of an observed post-install read.
    pub required_poststate: PatchFilePin,
    pub evidence: EvidencePin,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchConfigQualification {
    schema_version: u32,
    target_root: PathBuf,
    rows: Vec<PatchConfigQualificationRow>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchConfigQualificationRow {
    component_id: String,
    relative_path: String,
    source: PatchConfigSourceProjection,
    selected: PatchConfigValueProjection,
    target: PatchConfigTargetProjection,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchConfigSourceProjection {
    stamp: Stamp,
    effective_values: PreservedPatchEffectiveValues,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchConfigValueProjection {
    effective_values: PreservedPatchEffectiveValues,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchConfigTargetProjection {
    observed_before: PatchFilePin,
    effective_values: PreservedPatchEffectiveValues,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissingRequirement {
    pub id: String,
    pub detail: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputManifest {
    pub schema_version: u32,
    pub candidate_id: String,
    pub target_root: PathBuf,
    pub output_root: PathBuf,
    pub request: EvidencePin,
    pub asset_receipt: EvidencePin,
    #[serde(default)]
    pub derived_environment_evidence: Option<DerivedEnvironmentEvidence>,
    pub prepared_producer_closure: String,
    pub runtime_source_closure: String,
    pub selected_manifest: EvidencePin,
    pub selected_features: Vec<SelectedFeature>,
    pub components: Vec<SelectedComponent>,
    pub asset_provider_ids: Vec<String>,
    pub exclusions: Vec<Exclusion>,
    pub packages: Vec<PackageInput>,
    pub root_files: Vec<RootFileInput>,
    pub contract_file_paths: Vec<String>,
    pub greeneye_paths: Vec<String>,
    pub greeneye_route: Option<GreeneyeRouteEvidence>,
    pub extended_zoom: bool,
    pub selection: RuntimeSelection,
    pub xhl_plan: EvidencePin,
    pub preservation_evidence: Vec<EvidencePin>,
    pub preservation: Vec<PreservationEntry>,
    #[serde(default)]
    pub preserved_runtime_files: Vec<PreservedRuntimeFile>,
    #[serde(default)]
    pub preserved_patch_configs: Vec<PreservedPatchConfig>,
    pub missing_requirements: Vec<MissingRequirement>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GreeneyeInputBinding {
    pub target: String,
    pub stamp: Stamp,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GreeneyeArtifactBinding {
    pub source: PathBuf,
    pub target: String,
    pub stamp: Stamp,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GreeneyeRouteEvidence {
    pub profile_id: String,
    pub profile_version: u32,
    pub profile_definition: EvidencePin,
    pub runtime_source_closure: String,
    pub runtime_artifact: GreeneyeArtifactBinding,
    pub bootstrap_artifact: GreeneyeArtifactBinding,
    pub host: GreeneyeInputBinding,
    pub scan_inputs: Vec<GreeneyeInputBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileBinding {
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetPostimage {
    pub source: PathBuf,
    pub stamp: Stamp,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Template {
    pub schema_version: u32,
    pub status: String,
    pub candidate_id: String,
    pub manifest_sha256: String,
    pub target_root: PathBuf,
    pub payload_root: PathBuf,
    pub prepared_producer_closure: String,
    pub runtime_source_closure: String,
    pub request_json: String,
    pub request_raw_sha256: String,
    pub request_canonical_sha256: String,
    pub receipt_json: String,
    pub receipt_raw_sha256: String,
    pub receipt_canonical_sha256: String,
    pub source_dat_membership_sha256: String,
    pub prepared_stage: PathBuf,
    pub asset_postimages: BTreeMap<String, AssetPostimage>,
    pub selected_features: Vec<SelectedFeature>,
    pub selected_components: Vec<SelectedComponent>,
    pub asset_provider_ids: Vec<String>,
    pub asset_request_selected_ids: Vec<String>,
    pub canonical_runtime_mod_order: Vec<String>,
    pub contract: Option<Contract>,
    pub contract_file_paths: Vec<String>,
    pub greeneye_paths: Vec<String>,
    pub greeneye_route: Option<GreeneyeRouteEvidence>,
    pub extended_zoom: bool,
    pub missing_requirements: Vec<MissingRequirement>,
    pub xhl_plan_sha256: String,
    pub xhl_transaction: String,
    pub xhl_target: PathBuf,
    pub xhl_files: BTreeMap<String, FileBinding>,
    pub preservation: Vec<PreservationEntry>,
    #[serde(default)]
    pub preserved_runtime_files: Vec<PreservedRuntimeFile>,
    #[serde(default)]
    pub preserved_patch_configs: Vec<PreservedPatchConfig>,
    pub payload_files: BTreeMap<String, FileBinding>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewManifest {
    pub schema_version: u32,
    pub status: String,
    pub candidate_id: String,
    pub installable: bool,
    pub target_root: PathBuf,
    pub payload_root: PathBuf,
    pub manifest_sha256: String,
    pub selected_features: Vec<SelectedFeature>,
    pub selected_components: Vec<SelectedComponent>,
    pub asset_provider_ids: Vec<String>,
    pub asset_request_selected_ids: Vec<String>,
    pub canonical_runtime_mod_order: Vec<String>,
    pub greeneye_route: Option<GreeneyeRouteEvidence>,
    pub missing_requirements: Vec<MissingRequirement>,
    pub preservation: Vec<PreservationEntry>,
    #[serde(default)]
    pub preserved_runtime_files: Vec<PreservedRuntimeFile>,
    #[serde(default)]
    pub preserved_patch_configs: Vec<PreservedPatchConfig>,
    pub asset_postimages: BTreeMap<String, AssetPostimage>,
    pub xhl_plan_sha256: String,
    pub xhl_transaction: String,
    pub payload_files: BTreeMap<String, FileBinding>,
    pub gates: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransactionIntent {
    pub schema_version: u32,
    pub transaction_id: String,
    pub status: String,
    pub target_root: PathBuf,
    pub asset_request_raw_sha256: String,
    pub asset_receipt_raw_sha256: String,
    pub xhl_approved_plan_sha256: String,
    pub xhl_transaction: String,
    pub expected_xhl_target: PathBuf,
    pub target_freshness_required: bool,
    pub install_performed: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalizationReceipt {
    pub schema_version: u32,
    pub status: String,
    pub candidate_id: String,
    pub template_sha256: String,
    pub approved_plan_sha256: String,
    pub xhl_prepared_receipt_raw_sha256: String,
    pub xhl_prepared_receipt_canonical_sha256: String,
    pub xhl_transaction: String,
    pub target_root: PathBuf,
    pub policy_sha256: String,
    pub deployment_target_recheck_required: bool,
    pub install_performed: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedPolicy {
    schema_version: u32,
    owner: String,
    root: PathBuf,
    source_closure: String,
    request_json: String,
    request_sha256: String,
    receipt_json: String,
    receipt_sha256: String,
    extended_zoom: bool,
    preserved_loaders: BTreeMap<String, String>,
    runtime: Contract,
}

fn inventory(root: &Path) -> Result<BTreeMap<String, Stamp>> {
    fn visit(
        root: &Path,
        dir: &Path,
        out: &mut BTreeMap<String, Stamp>,
        total: &mut u64,
    ) -> Result<()> {
        ordinary(dir, true)?;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "linked package member: {}",
                path.display()
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    fs::symlink_metadata(&path)?.file_attributes() & 0x400 == 0,
                    "reparse package member: {}",
                    path.display()
                );
            }
            if kind.is_dir() {
                visit(root, &path, out, total)?;
            } else {
                ensure!(
                    kind.is_file(),
                    "unsupported package member: {}",
                    path.display()
                );
                let relative = path
                    .strip_prefix(root)?
                    .to_str()
                    .context("non-UTF8 package member")?
                    .replace('\\', "/");
                safe_relative(&relative)?;
                let value = stamp(&path)?;
                *total = total
                    .checked_add(value.bytes)
                    .context("package size overflow")?;
                ensure!(
                    *total <= MAX_TOTAL_BYTES,
                    "package source exceeds total size bound"
                );
                ensure!(
                    out.insert(relative, value).is_none(),
                    "duplicate package member"
                );
                ensure!(out.len() <= MAX_FILES, "package has too many files");
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    let mut total = 0u64;
    visit(root, root, &mut result, &mut total)?;
    Ok(result)
}
fn raw_request_receipt(
    manifest: &InputManifest,
) -> Result<(Vec<u8>, Vec<u8>, AssetRequest, AssetReceipt)> {
    let request_bytes = check_raw_evidence(
        &manifest.request.path,
        &manifest.request.sha256,
        16 * 1024 * 1024,
    )?;
    let receipt_bytes = check_raw_evidence(
        &manifest.asset_receipt.path,
        &manifest.asset_receipt.sha256,
        32 * 1024 * 1024,
    )?;
    let request: AssetRequest =
        serde_json::from_slice(&request_bytes).context("parse prepared asset request")?;
    let receipt: AssetReceipt =
        serde_json::from_slice(&receipt_bytes).context("parse prepared asset receipt")?;
    ensure!(
        request.schema == 1 && receipt.schema == 2,
        "only prepared request schema 1 / DAT-bound receipt schema 2 is accepted"
    );
    ensure!(
        manifest.prepared_producer_closure == receipt.source_closure
            && valid_digest(&receipt.source_closure),
        "prepared producer closure differs from the pinned receipt"
    );
    ensure!(
        receipt.request_sha256 == digest(&serde_json::to_vec(&request)?),
        "receipt canonical request digest differs"
    );
    ensure!(
        receipt.source_dat_membership_sha256
            == digest(&serde_json::to_vec(&receipt.source_dat_membership)?),
        "prepared source DAT membership digest differs"
    );
    let request_dats: BTreeMap<_, _> = request
        .inputs
        .iter()
        .filter(|(name, _)| name.starts_with("enshrouded_") && name.ends_with(".dat"))
        .map(|(name, stamp)| (name.clone(), stamp.clone()))
        .collect();
    ensure!(
        request_dats == receipt.source_dat_membership,
        "prepared receipt DAT membership differs from typed request inputs"
    );
    let target = normalize_absolute(&manifest.target_root)?;
    ensure!(
        normalize_absolute(&request.target_root)? == target
            && normalize_absolute(&request.asset_input_root)? == target
            && normalize_absolute(&receipt.target_root)? == target,
        "asset request/receipt target identity differs"
    );
    target_root(&request.target_root)?;
    validate_e_path(&request.environment_root, true, true)?;
    validate_e_path(&request.output_root, true, true)?;
    let mut expected_environment = request.environment_files.clone();
    for (name, value) in
        validate_derived_environment_evidence(manifest, &request_bytes, &request, &receipt)?
    {
        ensure!(
            expected_environment.insert(name.clone(), value).is_none(),
            "derived cache output was already part of the immutable request: {name}"
        );
    }
    ensure!(
        inventory(&request.environment_root)? == expected_environment,
        "prepared request environment closure changed"
    );
    for name in request
        .environment_files
        .keys()
        .filter(|name| name.starts_with("mods/"))
    {
        ensure!(
            request.inputs.get(name) == request.environment_files.get(name),
            "prepared request is missing an immutable mod input: {name}"
        );
    }
    let expected_stage = request.output_root.join(".enshrouded.shroudforge-stage");
    // The prepared generator owns the exact stage name; compare against its actual receipt output.
    ensure!(
        normalize_absolute(&receipt.output_stage)? == normalize_absolute(&expected_stage)?,
        "prepared receipt stage path differs from request output root"
    );
    validate_e_path(&receipt.output_stage, true, true)?;
    ensure!(
        receipt.staged.len() <= MAX_FILES && !receipt.staged.is_empty(),
        "prepared asset stage is empty or oversized"
    );
    let mut total = 0u64;
    for (name, expected) in &receipt.staged {
        let is_dat = name
            .strip_prefix("enshrouded_")
            .and_then(|value| value.strip_suffix(".dat"))
            .is_some_and(|digits| {
                !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
            });
        ensure!(
            name == "enshrouded.kfc"
                || name == "enshrouded.kfc_resources"
                || name == "generated-inputs.json"
                || is_dat,
            "unexpected prepared postimage member: {name}"
        );
        safe_relative(name)?;
        ensure!(
            valid_digest(&expected.sha256),
            "invalid prepared postimage digest"
        );
        total = total
            .checked_add(expected.bytes)
            .context("postimage size overflow")?;
        ensure!(
            total <= MAX_TOTAL_BYTES,
            "prepared postimages exceed size bound"
        );
        let source = receipt.output_stage.join(name.replace('/', "\\"));
        check_stamp(&stamp(&source)?, expected, name)?;
    }
    check_stamp(
        &stamp(&receipt.output_stage.join("prepared-only.json"))?,
        &receipt.prepared_marker,
        "prepared-only.json",
    )?;
    ensure!(
        request.selected_ids.len() == REQUIRED_ASSET_PROVIDERS,
        "asset request must bind exactly 13 selected providers"
    );
    let mut request_ids = BTreeSet::new();
    for id in &request.selected_ids {
        ensure!(
            safe_id(id) && !excluded_identity(id) && request_ids.insert(id.to_ascii_lowercase()),
            "invalid, duplicate, or excluded asset provider ID: {id}"
        );
    }
    ensure!(
        request.inputs.contains_key("enshrouded.exe")
            && request.inputs.contains_key("enshrouded.kfc")
            && request.inputs.contains_key("enshrouded.kfc_resources"),
        "prepared request lacks required base identity inputs"
    );
    Ok((request_bytes, receipt_bytes, request, receipt))
}

fn required_json_string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("cache evidence is missing string field {key}"))
}

fn required_json_u64(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("cache evidence is missing integer field {key}"))
}

fn required_json_bool(value: &Value, key: &str) -> Result<bool> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .with_context(|| format!("cache evidence is missing Boolean field {key}"))
}

fn validate_derived_environment_evidence(
    manifest: &InputManifest,
    request_bytes: &[u8],
    request: &AssetRequest,
    receipt: &AssetReceipt,
) -> Result<BTreeMap<String, Stamp>> {
    let Some(evidence) = manifest.derived_environment_evidence.as_ref() else {
        return Ok(BTreeMap::new());
    };
    ensure!(
        evidence.bridge_manifest.sha256 == CACHE_BINDING_SHA256
            && evidence.tester_verdict.sha256 == CACHE_VERDICT_SHA256,
        "derived cache evidence is not the accepted narrow binding and verdict"
    );
    let bridge_bytes = check_raw_evidence(
        &evidence.bridge_manifest.path,
        CACHE_BINDING_SHA256,
        1024 * 1024,
    )?;
    let verdict_bytes = check_raw_evidence(
        &evidence.tester_verdict.path,
        CACHE_VERDICT_SHA256,
        1024 * 1024,
    )?;
    let bridge: Value = serde_json::from_slice(&bridge_bytes).context("parse cache binding")?;
    let verdict: Value = serde_json::from_slice(&verdict_bytes).context("parse cache verdict")?;
    let canonical_request_sha256 = digest(&serde_json::to_vec(request)?);
    ensure!(
        bridge.get("schema_version").and_then(Value::as_u64) == Some(1)
            && required_json_string(&bridge, "task_id")? == "RB82"
            && required_json_string(&bridge, "status")?
                == "read_only_provenance_and_serialization_binding",
        "cache binding schema or status differs"
    );
    let bridge_request = bridge
        .get("request")
        .context("cache binding request identity is missing")?;
    ensure!(
        normalize_absolute(Path::new(required_json_string(bridge_request, "path")?))?
            == normalize_absolute(&manifest.request.path)?
            && required_json_string(bridge_request, "raw_sha256")? == digest(request_bytes)
            && required_json_string(bridge_request, "raw_sha256")? == manifest.request.sha256
            && required_json_string(bridge_request, "canonical_request_sha256")?
                == canonical_request_sha256
            && required_json_string(bridge_request, "canonical_request_sha256")?
                == receipt.request_sha256
            && required_json_string(bridge_request, "source_dat_membership_sha256")?
                == receipt.source_dat_membership_sha256
            && required_json_u64(bridge_request, "environment_files")? == 75
            && required_json_u64(bridge_request, "selected_ids")?
                == REQUIRED_ASSET_PROVIDERS as u64
            && required_json_u64(bridge_request, "inputs_total")? == request.inputs.len() as u64,
        "cache binding does not match the exact immutable request and producer receipt"
    );
    ensure!(
        request.environment_files.len() == 75
            && request
                .inputs
                .get("enshrouded.exe")
                .is_some_and(|stamp| { stamp.sha256 == CACHE_EXE_SHA256 }),
        "cache binding requires the exact 75-file environment and requested executable"
    );
    for relative in [CACHE_IDENTITY_RELATIVE, CACHE_REGISTRY_RELATIVE] {
        ensure!(
            !request
                .environment_files
                .keys()
                .any(|name| name.eq_ignore_ascii_case(relative))
                && !request
                    .inputs
                    .keys()
                    .any(|name| name.eq_ignore_ascii_case(relative)),
            "derived cache output is present in the frozen request: {relative}"
        );
    }
    let frozen_source = bridge
        .get("frozen_source")
        .context("cache binding frozen source identity is missing")?;
    ensure!(
        required_json_string(frozen_source, "source_closure_sha256")? == receipt.source_closure
            && required_json_string(frozen_source, "executable_sha256")?
                == CACHE_GENERATOR_EXE_SHA256,
        "cache binding producer or executable identity differs"
    );
    let bridge_outputs = bridge
        .get("derived_schema_cache_outputs")
        .and_then(Value::as_array)
        .context("cache binding outputs are missing")?;
    let verdict_outputs = verdict
        .get("derivedSchemaOutputs")
        .and_then(Value::as_array)
        .context("cache verdict outputs are missing")?;
    ensure!(
        bridge_outputs.len() == 2 && verdict_outputs == bridge_outputs,
        "cache evidence must bind the same exact two derived outputs"
    );
    ensure!(
        required_json_string(&verdict, "status")?
            == "PASS_NARROW_FROZEN_CACHE_PROVENANCE_AND_CANONICAL_SERIALIZATION_SUPPLEMENT"
            && required_json_string(&verdict, "bridgeManifestSha256")? == CACHE_BINDING_SHA256
            && required_json_string(&verdict, "canonicalRequestSha256")?
                == canonical_request_sha256
            && required_json_string(&verdict, "datMembershipSha256")?
                == receipt.source_dat_membership_sha256
            && required_json_bool(&verdict, "typeRegistryParsed")? == false
            && required_json_bool(&verdict, "semanticsAccepted")? == false
            && required_json_bool(&verdict, "installationApproved")? == false
            && required_json_string(&verdict, "publication")? == "HOLD",
        "cache verdict differs from its accepted read-only scope"
    );

    // This helper binds provenance and bytes only; it intentionally does not parse the registry.
    let expected = [
        (
            "executable/extractor identity",
            CACHE_IDENTITY_RELATIVE,
            CACHE_IDENTITY_BYTES,
            CACHE_IDENTITY_SHA256,
        ),
        (
            "type registry",
            CACHE_REGISTRY_RELATIVE,
            CACHE_REGISTRY_BYTES,
            CACHE_REGISTRY_SHA256,
        ),
    ];
    let mut outputs = BTreeMap::new();
    for (row, (role, relative, bytes, sha256)) in bridge_outputs.iter().zip(expected) {
        ensure!(
            required_json_string(row, "role")? == role
                && normalize_absolute(Path::new(required_json_string(row, "path")?))?
                    == normalize_absolute(&request.environment_root.join(relative))?
                && required_json_u64(row, "bytes")? == bytes
                && required_json_string(row, "sha256")? == sha256,
            "derived cache output differs from its fixed path or content pin: {relative}"
        );
        if relative == CACHE_IDENTITY_RELATIVE {
            ensure!(
                row.get("identity_schema").and_then(Value::as_u64) == Some(2)
                    && required_json_string(row, "executable_sha256")? == CACHE_EXE_SHA256
                    && required_json_string(row, "reflection_extractor")? == CACHE_EXTRACTOR_ID
                    && required_json_bool(row, "executable_matches_request")?
                    && required_json_bool(row, "parsed_content")?,
                "executable/extractor cache identity differs"
            );
        } else {
            ensure!(
                !required_json_bool(row, "parsed_content")?,
                "type registry content must remain unparsed by this helper"
            );
        }
        outputs.insert(
            relative.to_string(),
            Stamp {
                bytes,
                sha256: sha256.to_string(),
            },
        );
    }
    Ok(outputs)
}

#[derive(Clone)]
struct StagedSource {
    source: PathBuf,
    target: String,
    stamp: Stamp,
    runtime_binding: bool,
}
fn path_key(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}
fn same_path(a: &Path, b: &Path) -> bool {
    match (normalize_absolute(a), normalize_absolute(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
fn package_environment_files(
    request: &AssetRequest,
    directory: &str,
) -> Result<BTreeMap<String, Stamp>> {
    let prefix = format!("mods/{directory}/");
    let mut result = BTreeMap::new();
    for (path, value) in &request.environment_files {
        if let Some(relative) = path.strip_prefix(&prefix) {
            safe_relative(relative)?;
            result.insert(relative.to_owned(), value.clone());
        }
    }
    Ok(result)
}
fn is_forbidden_target(path: &str, exclusions: &[Exclusion]) -> bool {
    let key = path_key(path);
    exclusions
        .iter()
        .any(|exclusion| exclusion.paths.iter().any(|banned| path_key(banned) == key))
}
fn inventory_stamp_matches(
    actual: &BTreeMap<String, Stamp>,
    expected: &BTreeMap<String, Stamp>,
    label: &str,
) -> Result<()> {
    ensure!(
        actual == expected,
        "{label} source inventory differs from its pinned manifest"
    );
    ensure!(
        actual.len() <= MAX_FILES,
        "{label} source has too many files"
    );
    Ok(())
}
fn base_contract_binding(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    name == "mod.json"
        || name == "extended.mod.json"
        || name.ends_with(".dll")
        || name.ends_with(".ini")
}
fn root_path_is_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "enshrouded.kfc"
        || lower == "enshrouded.kfc_resources"
        || (lower.starts_with("enshrouded_") && lower.ends_with(".dat"))
        || lower == "enshrouded.exe"
}
fn ensure_required_exclusions(exclusions: &[Exclusion]) -> Result<()> {
    let required = [
        ExclusionKind::Autoloot,
        ExclusionKind::Shroudtopia,
        ExclusionKind::StandaloneFlight,
        ExclusionKind::DuplicateFlight,
        ExclusionKind::OldMinimapHost,
        ExclusionKind::ServerOnly,
        ExclusionKind::LegacyFallback,
    ];
    let mut seen = BTreeSet::new();
    for item in exclusions {
        ensure!(
            !item.reason.trim().is_empty(),
            "an exclusion needs a concrete reason"
        );
        ensure!(
            !item.ids.is_empty() || !item.paths.is_empty(),
            "an exclusion must identify an ID or path"
        );
        ensure!(
            seen.insert(format!("{:?}", item.kind)),
            "duplicate exclusion category"
        );
        for id in &item.ids {
            ensure!(safe_id(id), "invalid excluded component ID");
        }
        for path in &item.paths {
            safe_relative(path)?;
        }
    }
    ensure!(
        required
            .iter()
            .all(|kind| seen.contains(&format!("{kind:?}"))),
        "the manifest must explicitly bind all seven selected-scope exclusions"
    );
    Ok(())
}
fn check_exclusions(manifest: &InputManifest) -> Result<()> {
    ensure_required_exclusions(&manifest.exclusions)?;
    let selected: BTreeSet<_> = manifest
        .components
        .iter()
        .map(|component| component.id.to_ascii_lowercase())
        .collect();
    for component in &manifest.components {
        ensure!(
            !excluded_identity(&component.id),
            "excluded component entered selected scope: {}",
            component.id
        );
    }
    for item in &manifest.exclusions {
        for id in &item.ids {
            ensure!(
                !selected.contains(&id.to_ascii_lowercase()),
                "excluded component is also selected: {id}"
            );
        }
    }
    Ok(())
}

fn validate_feature_unit_mapping(manifest: &InputManifest) -> Result<()> {
    ensure!(
        manifest.selected_features.len() == REQUIRED_FEATURES,
        "typed selection must contain exactly 20 feature rows"
    );
    ensure!(
        !manifest.components.is_empty() && manifest.components.len() <= MAX_DEPLOYMENT_UNITS,
        "deployment unit count is empty or exceeds the bound"
    );

    let mut features = BTreeMap::new();
    for feature in &manifest.selected_features {
        ensure!(
            (1..=REQUIRED_FEATURES as u8).contains(&feature.row) && !feature.name.trim().is_empty(),
            "selected feature row is invalid"
        );
        ensure!(
            features.insert(feature.row, feature).is_none(),
            "duplicate selected feature row"
        );
    }
    ensure!(
        (1..=REQUIRED_FEATURES as u8).all(|row| features.contains_key(&row)),
        "typed selection must bind each of the 20 selected feature rows"
    );

    let mut units = BTreeMap::new();
    for unit in &manifest.components {
        ensure!(
            safe_id(&unit.id) && !unit.version.trim().is_empty(),
            "invalid deployment unit"
        );
        ensure!(
            unit.target == "client",
            "selected candidate must be client-only: {}",
            unit.id
        );
        ensure!(
            units.insert(unit.id.to_ascii_lowercase(), unit).is_none(),
            "duplicate deployment unit ID: {}",
            unit.id
        );
        let mut mapped_rows = BTreeSet::new();
        for row in &unit.feature_rows {
            let feature = features
                .get(row)
                .with_context(|| format!("deployment unit references unknown feature row {row}"))?;
            ensure!(
                mapped_rows.insert(*row),
                "deployment unit maps a feature row more than once: {}",
                unit.id
            );
            ensure!(
                !unit.enabled || feature.enabled,
                "enabled deployment unit maps to a disabled feature: {}",
                unit.id
            );
        }
        let mut dependencies = BTreeSet::new();
        for dependency in &unit.dependencies {
            ensure!(safe_id(dependency), "invalid dependency ID");
            ensure!(
                dependency.to_ascii_lowercase() != unit.id.to_ascii_lowercase()
                    && dependencies.insert(dependency.to_ascii_lowercase()),
                "self or duplicate deployment dependency: {}",
                unit.id
            );
        }
        if let Some(directory) = &unit.package_directory {
            safe_relative(directory)?;
            ensure!(
                directory.split('/').count() == 1,
                "package directory must be one folder"
            );
        }
        match unit.kind {
            ComponentKind::RuntimeService => ensure!(
                unit.package_directory.is_none()
                    && unit.service_id.as_deref().is_some_and(safe_id)
                    && unit.xhl_paths.is_empty(),
                "runtime service must have a service ID and no package or XHL paths"
            ),
            ComponentKind::XhlNative => ensure!(
                unit.package_directory.is_none()
                    && unit.service_id.is_none()
                    && !unit.xhl_paths.is_empty(),
                "XHL native unit needs owned XHL paths and cannot be a package or service"
            ),
            _ => ensure!(
                unit.service_id.is_none() && unit.xhl_paths.is_empty(),
                "package unit cannot declare a service ID or XHL paths"
            ),
        }
        let mut xhl_paths = BTreeSet::new();
        for path in &unit.xhl_paths {
            safe_relative(path)?;
            ensure!(
                xhl_paths.insert(path.to_ascii_lowercase()),
                "XHL unit repeats an owned path: {}",
                unit.id
            );
        }
    }

    for unit in &manifest.components {
        for dependency in &unit.dependencies {
            let target = units
                .get(&dependency.to_ascii_lowercase())
                .with_context(|| format!("deployment dependency is missing: {dependency}"))?;
            ensure!(
                !unit.enabled || target.enabled,
                "enabled deployment unit depends on a disabled unit: {} -> {}",
                unit.id,
                target.id
            );
        }
    }

    fn visit_dependency(
        id: &str,
        units: &BTreeMap<String, &SelectedComponent>,
        active: &mut BTreeSet<String>,
        complete: &mut BTreeSet<String>,
    ) -> Result<()> {
        let key = id.to_ascii_lowercase();
        if complete.contains(&key) {
            return Ok(());
        }
        ensure!(
            active.insert(key.clone()),
            "deployment dependency cycle at {id}"
        );
        let unit = units.get(&key).context("missing deployment unit")?;
        for dependency in &unit.dependencies {
            visit_dependency(dependency, units, active, complete)?;
        }
        active.remove(&key);
        complete.insert(key);
        Ok(())
    }
    let mut active = BTreeSet::new();
    let mut complete = BTreeSet::new();
    for unit in &manifest.components {
        visit_dependency(&unit.id, &units, &mut active, &mut complete)?;
    }

    let mut owned = BTreeSet::new();
    for feature in manifest
        .selected_features
        .iter()
        .filter(|item| item.enabled)
    {
        let providers: Vec<_> = manifest
            .components
            .iter()
            .filter(|unit| unit.enabled && unit.feature_rows.contains(&feature.row))
            .collect();
        ensure!(
            !providers.is_empty(),
            "enabled selected feature has no enabled deployment unit: row {} ({})",
            feature.row,
            feature.name
        );
        for provider in providers {
            owned.insert(provider.id.to_ascii_lowercase());
            for dependency in &provider.dependencies {
                owned.insert(dependency.to_ascii_lowercase());
            }
        }
    }
    // Close ownership through dependency chains from feature-owned units.
    let mut pending: Vec<_> = owned.iter().cloned().collect();
    while let Some(id) = pending.pop() {
        let unit = units.get(&id).context("owned deployment unit is missing")?;
        for dependency in &unit.dependencies {
            let key = dependency.to_ascii_lowercase();
            if owned.insert(key.clone()) {
                pending.push(key);
            }
        }
    }
    ensure!(
        manifest
            .components
            .iter()
            .all(|unit| owned.contains(&unit.id.to_ascii_lowercase())),
        "deployment manifest contains an unowned unit"
    );
    for unit in &manifest.components {
        if unit.kind == ComponentKind::NativeOnly && unit.package_directory.is_none() {
            ensure!(
                manifest
                    .root_files
                    .iter()
                    .any(|file| file.component_id.eq_ignore_ascii_case(&unit.id)),
                "native-only unit without a package must own at least one root file: {}",
                unit.id
            );
        }
    }
    Ok(())
}

fn validate_xhl_unit_ownership(
    components: &[SelectedComponent],
    plan: &ApprovedPlan,
) -> Result<()> {
    let mut owners = BTreeMap::new();
    for unit in components
        .iter()
        .filter(|unit| unit.kind == ComponentKind::XhlNative)
    {
        for path in &unit.xhl_paths {
            ensure!(
                owners
                    .insert(path.to_ascii_lowercase(), unit.id.to_ascii_lowercase())
                    .is_none(),
                "XHL plan path has multiple deployment unit owners: {path}"
            );
        }
    }
    let plan_paths: BTreeSet<_> = plan
        .files
        .keys()
        .map(|path| path.to_ascii_lowercase())
        .collect();
    ensure!(
        owners.keys().cloned().collect::<BTreeSet<_>>() == plan_paths,
        "XHL deployment-unit ownership must exactly cover the approved plan files"
    );
    Ok(())
}

fn same_stamp(left: &Stamp, right: &Stamp) -> bool {
    left.bytes == right.bytes && left.sha256.eq_ignore_ascii_case(&right.sha256)
}

fn validate_greeneye_dll_plan(
    identity: &Value,
    target_root: &Path,
    host_target: &str,
) -> Result<()> {
    let expected = path_key(&target_path(target_root, host_target)?.to_string_lossy());
    let entries = identity
        .as_array()
        .context("Greeneye DLL load identity is not an array")?;
    let selected: Vec<_> = entries
        .iter()
        .filter(|entry| {
            entry
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path_key(path) == expected)
        })
        .collect();
    ensure!(
        selected.len() == 1 && selected[0].get("mode").and_then(Value::as_str) == Some("library"),
        "Greeneye host must have exactly one selected ordinary Library load entry"
    );
    Ok(())
}

fn validate_greeneye_route(manifest: &InputManifest, route: &GreeneyeRouteEvidence) -> Result<()> {
    let profile = greeneye_profiles::for_route(&route.profile_id, route.profile_version, &route.profile_definition.sha256)
        .context("unsupported exact Greeneye route profile")?;
    ensure!(
        route.profile_id == profile.id
            && route.profile_version == profile.version
            && route.profile_definition.sha256 == profile.definition_sha256,
        "Greeneye route profile identity is not the accepted versioned continuation"
    );
    check_raw_evidence(
        &route.profile_definition.path,
        profile.definition_sha256,
        1024 * 1024,
    )?;
    ensure!(
        route.runtime_source_closure == manifest.runtime_source_closure
            && valid_digest(&route.runtime_source_closure),
        "Greeneye profile is bound to a different full-runtime source closure"
    );

    let runtime_target = "shroudforge/shroudforge-runtime.dll";
    let bootstrap_target = "winmm.dll";
    for (artifact, expected_target) in [
        (&route.runtime_artifact, runtime_target),
        (&route.bootstrap_artifact, bootstrap_target),
    ] {
        ensure!(
            artifact.target.eq_ignore_ascii_case(expected_target),
            "Greeneye profile names an unexpected runtime or bootstrap artifact"
        );
        validate_e_path(&artifact.source, true, false)?;
        let input = manifest
            .root_files
            .iter()
            .find(|item| item.target.eq_ignore_ascii_case(expected_target))
            .with_context(|| {
                format!("Greeneye artifact is not a declared root input: {expected_target}")
            })?;
        ensure!(
            input.runtime_binding && same_stamp(&input.stamp, &artifact.stamp),
            "Greeneye artifact stamp differs from its declared runtime binding: {expected_target}"
        );
        ensure!(
            normalize_absolute(&input.source)? == normalize_absolute(&artifact.source)?,
            "Greeneye artifact path differs from the declared source: {expected_target}"
        );
        check_stamp(&stamp(&artifact.source)?, &artifact.stamp, expected_target)?;
    }

    let required_host = manifest
        .components
        .iter()
        .filter(|unit| unit.enabled && unit.requires_greeneye)
        .collect::<Vec<_>>();
    ensure!(
        required_host.len() == 1,
        "Greeneye continuation requires exactly one enabled host unit"
    );
    let host_unit = required_host[0];
    ensure!(
        host_unit.kind == ComponentKind::NativeOnly,
        "Greeneye host must be an explicitly selected native-only unit"
    );
    let package_directory = host_unit
        .package_directory
        .as_deref()
        .context("Greeneye host unit lacks a package directory")?;
    let expected_host_target = format!("mods/{package_directory}/dbgcore.dll");
    ensure!(
        route
            .host
            .target
            .eq_ignore_ascii_case(&expected_host_target)
            && manifest.greeneye_paths.len() == 1
            && manifest.greeneye_paths[0].eq_ignore_ascii_case(&expected_host_target),
        "Greeneye host path differs from the exact selected ordinary Library route"
    );
    ensure!(
        route.host.stamp.bytes == profile.host_bytes
            && route.host.stamp.sha256 == profile.host_sha256,
        "Greeneye host image differs from the accepted versioned profile"
    );
    let package = manifest
        .packages
        .iter()
        .find(|package| package.component_id.eq_ignore_ascii_case(&host_unit.id))
        .context("Greeneye host package source is missing")?;
    ensure!(
        package.origin == PackageOrigin::ApprovedE
            && package
                .package_directory
                .eq_ignore_ascii_case(package_directory),
        "Greeneye host package must come from its exact approved E source"
    );
    let host_overlay = package
        .overlays
        .iter()
        .find(|item| item.path.eq_ignore_ascii_case("dbgcore.dll"));
    let (host_source, host_stamp): (PathBuf, &Stamp) = if let Some(overlay) = host_overlay {
        (overlay.source.clone(), &overlay.stamp)
    } else {
        let file_stamp = package
            .source_files
            .get("dbgcore.dll")
            .context("Greeneye package inventory lacks dbgcore.dll")?;
        let source = package
            .source_root
            .as_deref()
            .context("Greeneye package source root is missing")?
            .join("dbgcore.dll");
        (source, file_stamp)
    };
    ensure!(
        same_stamp(host_stamp, &route.host.stamp),
        "Greeneye package host stamp differs from the accepted profile"
    );
    check_stamp(
        &stamp(&host_source)?,
        &route.host.stamp,
        &expected_host_target,
    )?;

    ensure!(
        route.scan_inputs.len() == profile.scan_inputs.len(),
        "Greeneye profile requires exactly four root scan inputs"
    );
    let mut expected_scan_paths = BTreeSet::new();
    for (index, (target, bytes, sha256)) in profile.scan_inputs.iter().enumerate() {
        let binding = &route.scan_inputs[index];
        ensure!(
            binding.target == *target
                && binding.stamp.bytes == *bytes
                && binding.stamp.sha256 == *sha256,
            "Greeneye scan input differs from the accepted profile: {target}"
        );
        expected_scan_paths.insert(target.to_ascii_lowercase());
        let input = manifest
            .root_files
            .iter()
            .find(|item| item.target.eq_ignore_ascii_case(target))
            .with_context(|| {
                format!("Greeneye scan input is not a declared root file: {target}")
            })?;
        ensure!(
            input.runtime_binding && same_stamp(&input.stamp, &binding.stamp),
            "Greeneye scan input is not bound to the runtime contract: {target}"
        );
        check_stamp(&stamp(&input.source)?, &binding.stamp, target)?;
        ensure!(
            manifest
                .contract_file_paths
                .iter()
                .any(|path| path.eq_ignore_ascii_case(target)),
            "Greeneye scan input is omitted from contractFilePaths: {target}"
        );
    }
    let actual_scan_paths: BTreeSet<_> = manifest
        .root_files
        .iter()
        .filter_map(|input| {
            let lower = input.target.to_ascii_lowercase();
            lower.starts_with("greeneye/").then_some(lower)
        })
        .collect();
    ensure!(
        actual_scan_paths == expected_scan_paths,
        "Greeneye namespace contains an unqualified scan input"
    );
    ensure!(
        !manifest
            .root_files
            .iter()
            .any(|input| input.target.eq_ignore_ascii_case("dbgcore.dll")),
        "legacy root Greeneye host cannot be bound as a candidate input"
    );
    ensure!(
        !manifest.root_files.iter().any(|input| input
            .target
            .eq_ignore_ascii_case("greeneye/hotreload.enable")),
        "Greeneye hotreload marker is not allowed in the selected namespace"
    );
    Ok(())
}

fn validate_greeneye_template(template: &Template, contract: &Contract) -> Result<()> {
    let selected: Vec<_> = template
        .selected_components
        .iter()
        .filter(|unit| unit.enabled && unit.requires_greeneye)
        .collect();
    if selected.is_empty() {
        ensure!(
            template.greeneye_route.is_none() && template.greeneye_paths.is_empty(),
            "Greeneye route evidence exists without a selected host"
        );
        return Ok(());
    }
    ensure!(
        selected.len() == 1,
        "Greeneye continuation requires exactly one selected host unit"
    );
    let route = template
        .greeneye_route
        .as_ref()
        .context("Greeneye template lacks its typed route evidence")?;
    let profile = greeneye_profiles::for_route(&route.profile_id, route.profile_version, &route.profile_definition.sha256)
        .context("unsupported exact Greeneye template profile")?;
    ensure!(
        route.profile_id == profile.id
            && route.profile_version == profile.version
            && route.profile_definition.sha256 == profile.definition_sha256
            && route.runtime_source_closure == template.runtime_source_closure,
        "Greeneye template profile or runtime source identity differs"
    );
    check_raw_evidence(
        &route.profile_definition.path,
        profile.definition_sha256,
        1024 * 1024,
    )?;
    let package_directory = selected[0]
        .package_directory
        .as_deref()
        .context("Greeneye host package directory is missing")?;
    let host_target = format!("mods/{package_directory}/dbgcore.dll");
    ensure!(
        route.host.target.eq_ignore_ascii_case(&host_target)
            && route.host.stamp.bytes == profile.host_bytes
            && route.host.stamp.sha256 == profile.host_sha256
            && template.greeneye_paths.len() == 1
            && template.greeneye_paths[0].eq_ignore_ascii_case(&host_target)
            && contract.greeneye_paths.len() == 1
            && contract.greeneye_paths[0].eq_ignore_ascii_case(&host_target),
        "Greeneye template host profile differs"
    );
    validate_greeneye_dll_plan(&contract.dll_plan, &template.target_root, &host_target)?;

    let host_payload = template
        .payload_files
        .get(&host_target)
        .context("Greeneye host is missing from the template payload")?;
    let host_contract = contract
        .files
        .get(&host_target)
        .context("Greeneye host is missing from the runtime contract")?;
    ensure!(
        host_payload.bytes == route.host.stamp.bytes
            && host_payload.sha256 == route.host.stamp.sha256
            && host_contract.bytes == route.host.stamp.bytes
            && host_contract.sha256 == route.host.stamp.sha256,
        "Greeneye host payload or contract stamp differs from the accepted profile"
    );
    check_stamp(
        &stamp(&target_path(&template.payload_root, &host_target)?)?,
        &route.host.stamp,
        &host_target,
    )?;

    ensure!(
        route.scan_inputs.len() == profile.scan_inputs.len(),
        "Greeneye template lacks the exact four scan inputs"
    );
    let mut expected_scan_paths = BTreeSet::new();
    for (index, (target, bytes, sha256)) in profile.scan_inputs.iter().enumerate() {
        let binding = &route.scan_inputs[index];
        ensure!(
            binding.target == *target
                && binding.stamp.bytes == *bytes
                && binding.stamp.sha256 == *sha256,
            "Greeneye template scan input differs from the accepted profile: {target}"
        );
        expected_scan_paths.insert(target.to_ascii_lowercase());
        ensure!(
            template
                .contract_file_paths
                .iter()
                .any(|path| path.eq_ignore_ascii_case(target)),
            "Greeneye template scan input is not an explicit contract file: {target}"
        );
        let payload = template
            .payload_files
            .get(*target)
            .with_context(|| format!("Greeneye scan input missing from payload: {target}"))?;
        let contract_file = contract
            .files
            .get(*target)
            .with_context(|| format!("Greeneye scan input missing from contract: {target}"))?;
        ensure!(
            payload.bytes == binding.stamp.bytes
                && payload.sha256 == binding.stamp.sha256
                && contract_file.bytes == binding.stamp.bytes
                && contract_file.sha256 == binding.stamp.sha256,
            "Greeneye scan input payload or contract stamp differs: {target}"
        );
        check_stamp(
            &stamp(&target_path(&template.payload_root, target)?)?,
            &binding.stamp,
            target,
        )?;
    }
    let actual_scan_paths: BTreeSet<_> = template
        .payload_files
        .keys()
        .filter_map(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with("greeneye/").then_some(lower)
        })
        .collect();
    ensure!(
        actual_scan_paths == expected_scan_paths,
        "Greeneye payload contains an unqualified scan input"
    );
    ensure!(
        !template
            .payload_files
            .keys()
            .any(|path| path.eq_ignore_ascii_case("dbgcore.dll")),
        "legacy root Greeneye host cannot be finalized"
    );

    for (artifact, target) in [
        (
            &route.runtime_artifact,
            "shroudforge/shroudforge-runtime.dll",
        ),
        (&route.bootstrap_artifact, "winmm.dll"),
    ] {
        ensure!(
            artifact.target.eq_ignore_ascii_case(target),
            "Greeneye artifact target differs in finalized template"
        );
        let payload = template
            .payload_files
            .get(target)
            .with_context(|| format!("Greeneye artifact missing from payload: {target}"))?;
        let contract_file = contract
            .files
            .get(target)
            .with_context(|| format!("Greeneye artifact missing from contract: {target}"))?;
        ensure!(
            payload.bytes == artifact.stamp.bytes
                && payload.sha256 == artifact.stamp.sha256
                && contract_file.bytes == artifact.stamp.bytes
                && contract_file.sha256 == artifact.stamp.sha256,
            "Greeneye runtime/bootstrap artifact identity differs: {target}"
        );
        check_stamp(
            &stamp(&target_path(&template.payload_root, target)?)?,
            &artifact.stamp,
            target,
        )?;
        validate_e_path(&artifact.source, true, false)?;
        check_stamp(&stamp(&artifact.source)?, &artifact.stamp, target)?;
    }
    Ok(())
}

fn collect_staged_sources(
    manifest: &InputManifest,
    request: &AssetRequest,
    receipt: &AssetReceipt,
    xhl_files: &BTreeMap<String, FileBinding>,
) -> Result<Vec<StagedSource>> {
    validate_feature_unit_mapping(manifest)?;
    ensure!(
        manifest.asset_provider_ids.len() == REQUIRED_ASSET_PROVIDERS,
        "asset provider list must contain exactly 13 rows"
    );
    ensure!(
        manifest.packages.len() <= MAX_DEPLOYMENT_UNITS,
        "package source count exceeds the deployment unit bound"
    );
    ensure!(
        manifest.root_files.len() <= MAX_FILES,
        "too many root files"
    );
    let mut component_ids = BTreeSet::new();
    let mut component_dirs = BTreeSet::new();
    for row in &manifest.components {
        ensure!(
            component_ids.insert(row.id.to_ascii_lowercase()),
            "duplicate deployment unit ID: {}",
            row.id
        );
        if let Some(directory) = &row.package_directory {
            safe_relative(directory)?;
            ensure!(
                directory.split('/').count() == 1,
                "package directory must be one folder"
            );
            ensure!(
                component_dirs.insert(directory.to_ascii_lowercase()),
                "duplicate selected package directory: {directory}"
            );
        }
    }
    check_exclusions(manifest)?;
    let _provider_by_component = validate_preserved_patch_config_inputs(manifest, request)?;
    let preserved_patch_targets: BTreeSet<_> = manifest
        .preserved_patch_configs
        .iter()
        .map(|entry| normalized_contract_path(&entry.target))
        .collect();
    for entry in &manifest.preserved_patch_configs {
        let target = normalized_contract_path(&entry.target);
        ensure!(
            !receipt
                .staged
                .keys()
                .any(|path| normalized_contract_path(path) == target),
            "preserved patch config collides with a generated asset postimage: {}",
            entry.target
        );
        ensure!(
            !xhl_files
                .keys()
                .any(|path| normalized_contract_path(path) == target),
            "preserved patch config collides with an XHL-owned target: {}",
            entry.target
        );
    }
    let expected_asset_ids: Vec<_> = manifest
        .components
        .iter()
        .filter(|row| row.kind == ComponentKind::AssetProvider)
        .map(|row| row.id.clone())
        .collect();
    ensure!(
        expected_asset_ids == manifest.asset_provider_ids
            && manifest.asset_provider_ids == request.selected_ids,
        "asset-provider units, typed provider order, and immutable request order must match exactly"
    );
    ensure!(
        manifest.asset_provider_ids.len() == REQUIRED_ASSET_PROVIDERS
            && manifest
                .asset_provider_ids
                .iter()
                .map(|id| id.to_ascii_lowercase())
                .collect::<BTreeSet<_>>()
                .len()
                == REQUIRED_ASSET_PROVIDERS,
        "asset provider list must contain exactly 13 unique ordered IDs"
    );
    let mut packages_seen = BTreeSet::new();
    let mut targets: BTreeMap<String, StagedSource> = BTreeMap::new();
    for package in &manifest.packages {
        ensure!(
            component_ids.contains(&package.component_id.to_ascii_lowercase()),
            "package source belongs to an unselected component: {}",
            package.component_id
        );
        let component = manifest
            .components
            .iter()
            .find(|row| row.id.eq_ignore_ascii_case(&package.component_id))
            .context("missing selected package row")?;
        ensure!(
            component.package_directory.as_deref() == Some(package.package_directory.as_str()),
            "package directory differs from selected row"
        );
        safe_relative(&package.package_directory)?;
        ensure!(
            package.package_directory.split('/').count() == 1
                && packages_seen.insert(package.package_directory.to_ascii_lowercase()),
            "duplicate or invalid package directory"
        );
        ensure!(
            !excluded_identity(&package.package_directory),
            "excluded package directory entered candidate: {}",
            package.package_directory
        );
        match (component.kind, package.origin) {
            (ComponentKind::AssetProvider, PackageOrigin::AssetEnvironment) => ensure!(
                package.source_root.is_none(),
                "asset package source is derived from the pinned request environment"
            ),
            (ComponentKind::RuntimeOnly | ComponentKind::NativeOnly, PackageOrigin::ApprovedE) => {
                let root = package
                    .source_root
                    .as_deref()
                    .context("approved E package source root is missing")?;
                validate_e_path(root, true, true)?;
            }
            _ => bail!("component/package origin conflict for {}", component.id),
        }
        ensure!(
            !package.source_files.is_empty() && package.source_files.len() <= MAX_FILES,
            "package source inventory is empty or oversized"
        );
        let source_root = match package.origin {
            PackageOrigin::AssetEnvironment => request
                .environment_root
                .join("mods")
                .join(&package.package_directory),
            PackageOrigin::ApprovedE => package.source_root.clone().unwrap(),
        };
        validate_e_path(&source_root, true, true)?;
        let actual = inventory(&source_root)?;
        inventory_stamp_matches(&actual, &package.source_files, &package.package_directory)?;
        if package.origin == PackageOrigin::AssetEnvironment {
            let request_inventory = package_environment_files(request, &package.package_directory)?;
            ensure!(
                request_inventory == package.source_files,
                "asset package differs from the immutable prepared environment: {}",
                package.package_directory
            );
        }
        if component.kind != ComponentKind::AssetProvider {
            ensure!(
                !request
                    .environment_files
                    .keys()
                    .any(|name| name.starts_with(&format!("mods/{}/", package.package_directory))),
                "runtime-only package is already inside the immutable asset environment: {}",
                package.package_directory
            );
        }
        ensure!(
            package.overlays.len() <= 128,
            "too many intended package overlays"
        );
        let mut overlay_paths = BTreeSet::new();
        for overlay in &package.overlays {
            safe_relative(&overlay.path)?;
            ensure!(
                overlay_paths.insert(overlay.path.to_ascii_lowercase()),
                "duplicate package overlay path"
            );
            ensure!(
                !overlay.fields.is_empty()
                    && overlay.fields.iter().all(|field| !field.trim().is_empty()),
                "intended config overlay needs named fields"
            );
            ensure!(
                !overlay.disposition.trim().is_empty(),
                "intended config overlay needs a disposition"
            );
            ensure!(
                !overlay.evidence.is_empty(),
                "intended config overlay lacks its selected-value provenance"
            );
            validate_e_path(&overlay.source, true, false)?;
            check_stamp(&stamp(&overlay.source)?, &overlay.stamp, &overlay.path)?;
            for evidence in &overlay.evidence {
                check_raw_evidence(&evidence.path, &evidence.sha256, 32 * 1024 * 1024)?;
            }
        }
        let prefix = format!("mods/{}/", package.package_directory);
        for (relative, file_stamp) in &package.source_files {
            safe_relative(relative)?;
            if overlay_paths.contains(&relative.to_ascii_lowercase()) {
                continue;
            }
            let source = source_root.join(relative.replace('/', "\\"));
            let target = format!("{prefix}{relative}");
            if preserved_patch_targets.contains(&normalized_contract_path(&target)) {
                ensure!(
                    relative == "config.txt"
                        && package.origin == PackageOrigin::AssetEnvironment
                        && component.kind == ComponentKind::AssetProvider,
                    "only the selected patch config.txt leaf can be omitted from the write payload"
                );
                ensure!(
                    package.source_files.get("config.txt") == Some(file_stamp),
                    "preserved patch config is not bound by the complete source inventory"
                );
                continue;
            }
            ensure!(
                !is_forbidden_target(&target, &manifest.exclusions),
                "excluded package path staged: {target}"
            );
            let key = target.to_ascii_lowercase();
            ensure!(
                targets
                    .insert(
                        key,
                        StagedSource {
                            source,
                            target: target.clone(),
                            stamp: file_stamp.clone(),
                            runtime_binding: base_contract_binding(&target)
                        }
                    )
                    .is_none(),
                "duplicate staged path: {target}"
            );
        }
        for overlay in &package.overlays {
            let present = package
                .source_files
                .keys()
                .any(|name| name.eq_ignore_ascii_case(&overlay.path));
            ensure!(
                overlay.replaces == present,
                "overlay replacement/addition declaration differs for {}",
                overlay.path
            );
            let target = format!("{prefix}{}", overlay.path);
            ensure!(
                !is_forbidden_target(&target, &manifest.exclusions),
                "excluded overlay path staged: {target}"
            );
            let key = target.to_ascii_lowercase();
            ensure!(
                targets
                    .insert(
                        key,
                        StagedSource {
                            source: overlay.source.clone(),
                            target: target.clone(),
                            stamp: overlay.stamp.clone(),
                            runtime_binding: base_contract_binding(&target)
                        }
                    )
                    .is_none(),
                "duplicate staged path: {target}"
            );
        }
    }
    for component in &manifest.components {
        if component.kind == ComponentKind::AssetProvider {
            ensure!(
                manifest
                    .packages
                    .iter()
                    .any(|package| package.component_id.eq_ignore_ascii_case(&component.id)),
                "selected asset provider lacks a complete package pin: {}",
                component.id
            );
        }
        if component.kind == ComponentKind::RuntimeOnly && component.enabled {
            let directory = component
                .package_directory
                .as_deref()
                .context("enabled runtime-only component lacks a package directory")?;
            ensure!(
                manifest.packages.iter().any(|package| {
                    package.component_id.eq_ignore_ascii_case(&component.id)
                        && package.package_directory.eq_ignore_ascii_case(directory)
                        && package.origin == PackageOrigin::ApprovedE
                }),
                "enabled runtime-only component lacks an approved E package input: {}",
                component.id
            );
        }
        if component.kind == ComponentKind::NativeOnly && component.package_directory.is_some() {
            ensure!(
                manifest.packages.iter().any(|package| {
                    package.component_id.eq_ignore_ascii_case(&component.id)
                        && package.origin == PackageOrigin::ApprovedE
                }),
                "native-only package unit lacks an approved E package input: {}",
                component.id
            );
        }
        ensure!(
            !matches!(
                component.kind,
                ComponentKind::RuntimeService | ComponentKind::XhlNative
            ) || !manifest
                .packages
                .iter()
                .any(|package| package.component_id.eq_ignore_ascii_case(&component.id)),
            "service or XHL unit cannot also declare a package input: {}",
            component.id
        );
    }
    for root_file in &manifest.root_files {
        safe_relative(&root_file.target)?;
        ensure!(
            !root_file.target.starts_with("mods/")
                || (root_file.target.split('/').count() == 2
                    && root_file.target.to_ascii_lowercase().ends_with(".ini")),
            "package files must be declared through a package inventory except the canonical loose Vein INI"
        );
        ensure!(
            root_file.target != "shroudforge/config/current-assets.json",
            "the full-runtime policy is emitted only by the finalizer"
        );
        ensure!(
            !is_forbidden_target(&root_file.target, &manifest.exclusions),
            "excluded root path staged: {}",
            root_file.target
        );
        ensure!(
            component_ids.contains(&root_file.component_id.to_ascii_lowercase()),
            "root file belongs to an unknown deployment unit: {}",
            root_file.component_id
        );
        validate_e_path(&root_file.source, true, false)?;
        check_stamp(
            &stamp(&root_file.source)?,
            &root_file.stamp,
            &root_file.target,
        )?;
        ensure!(
            targets
                .insert(
                    root_file.target.to_ascii_lowercase(),
                    StagedSource {
                        source: root_file.source.clone(),
                        target: root_file.target.clone(),
                        stamp: root_file.stamp.clone(),
                        runtime_binding: root_file.runtime_binding
                    }
                )
                .is_none(),
            "duplicate staged path: {}",
            root_file.target
        );
    }
    ensure!(
        !targets.is_empty() && targets.len() <= MAX_FILES,
        "candidate payload is empty or too large"
    );
    let mut bytes = 0u64;
    for file in targets.values() {
        bytes = bytes
            .checked_add(file.stamp.bytes)
            .context("candidate size overflow")?;
    }
    ensure!(
        bytes <= MAX_TOTAL_BYTES,
        "candidate payload exceeds size bound"
    );
    for path in &manifest.greeneye_paths {
        ensure!(
            targets.contains_key(&path.to_ascii_lowercase()),
            "Greeneye path is not supplied by an approved package or root-file input: {path}"
        );
    }
    Ok(targets.into_values().collect())
}

fn normalize_absolute(path: &Path) -> Result<String> {
    ensure!(
        path.is_absolute(),
        "expected an absolute path: {}",
        path.display()
    );
    let text = path
        .to_str()
        .context("non-UTF8 absolute path")?
        .replace('/', "\\");
    let drive_path = text.strip_prefix("\\\\?\\").unwrap_or(&text);
    ensure!(
        drive_path.len() >= 3
            && drive_path.as_bytes()[0].is_ascii_alphabetic()
            && drive_path.as_bytes()[1] == b':'
            && drive_path.as_bytes()[2] == b'\\',
        "absolute path must use a native drive spelling; UNC and device paths are refused"
    );
    let tail = drive_path[3..].trim_end_matches('\\');
    ensure!(
        !tail.contains(':')
            && tail.split('\\').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && !part.ends_with(['.', ' '])
                    && !part.chars().any(char::is_control)
            }),
        "absolute path contains a traversal, alias, or invalid component"
    );
    if tail.is_empty() {
        Ok(format!(
            "{}:\\",
            (drive_path.as_bytes()[0] as char).to_ascii_lowercase()
        ))
    } else {
        Ok(format!(
            "{}:\\{}",
            (drive_path.as_bytes()[0] as char).to_ascii_lowercase(),
            tail.to_ascii_lowercase()
        ))
    }
}
fn relative_native_path(path: &Path, root: &Path) -> Result<String> {
    let path_key = normalize_absolute(path)?;
    let root_key = normalize_absolute(root)?;
    let prefix = format!("{}\\", root_key.trim_end_matches('\\'));
    ensure!(
        path_key.starts_with(&prefix),
        "path escapes its native drive root"
    );
    let raw_path = path
        .to_str()
        .context("non-UTF8 absolute path")?
        .replace('/', "\\");
    let raw_root = root
        .to_str()
        .context("non-UTF8 absolute root")?
        .replace('/', "\\");
    let raw_path = raw_path.strip_prefix("\\\\?\\").unwrap_or(&raw_path);
    let raw_root = raw_root.strip_prefix("\\\\?\\").unwrap_or(&raw_root);
    let raw_prefix = format!("{}\\", raw_root.trim_end_matches('\\'));
    ensure!(
        raw_path
            .to_ascii_lowercase()
            .starts_with(&raw_prefix.to_ascii_lowercase()),
        "path spelling does not match its native drive root"
    );
    let relative = raw_path[raw_prefix.len()..].replace('\\', "/");
    safe_relative(&relative)?;
    Ok(relative)
}
fn package_relative_path(path: &Path, package_root: &Path) -> Result<Option<String>> {
    let relative = if path.is_absolute() {
        let path_key = normalize_absolute(path)?;
        let root_key = normalize_absolute(package_root)?;
        if path_key == root_key {
            return Ok(None);
        }
        relative_native_path(path, package_root)?
    } else {
        let relative = path
            .to_str()
            .context("non-UTF8 package member path")?
            .replace('\\', "/");
        if relative.is_empty() {
            return Ok(None);
        }
        relative
    };
    safe_relative(&relative)?;
    Ok(Some(relative))
}
fn is_reserved_xhl_output(name: &str) -> bool {
    name.eq_ignore_ascii_case("XHL-Native-Plugin-Host.log")
        || name.to_ascii_lowercase().starts_with("xhl/config/")
}

fn xhl_target_files(
    plan: &ApprovedPlan,
    raw_plan: &[u8],
    target: &Path,
) -> Result<BTreeMap<String, FileBinding>> {
    ensure!(
        plan.schema_version == 1 && !plan.transaction.is_empty(),
        "unsupported XHL approved plan"
    );
    validate_e_path(&plan.source_root, true, true)?;
    let expected_parent = target.join("mods");
    ensure!(
        normalize_absolute(&plan.target_parent)? == normalize_absolute(&expected_parent)?,
        "XHL plan targets a different mods directory"
    );
    ensure!(
        plan.target_parent
            .file_name()
            .is_some_and(|name| name == "mods"),
        "XHL target parent must be the target mods directory"
    );
    ensure!(
        plan.files.len() >= 5 && plan.files.len() <= 128,
        "XHL plan has invalid file count"
    );
    let mut aliases = BTreeSet::new();
    let mut total = 0u64;
    let mut files = BTreeMap::new();
    for (name, input) in &plan.files {
        safe_relative(name)?;
        ensure!(
            aliases.insert(name.to_ascii_lowercase()),
            "XHL plan contains path aliases"
        );
        ensure!(
            valid_digest(&input.sha256) && input.bytes <= MAX_FILE_BYTES,
            "XHL plan has invalid source stamp"
        );
        total = total
            .checked_add(input.bytes)
            .context("XHL source size overflow")?;
        ensure!(
            total <= 64 * 1024 * 1024,
            "XHL approved plan exceeds its 64 MiB bound"
        );
        if name == "dinput8-chain.ini" {
            ensure!(
                input.source.is_none() && input.source_identity.is_none(),
                "disabled chain is generated, not copied"
            );
            ensure!(
                input.bytes == sf_xhl_tree_adapter::tree::DISABLED_CHAIN.len() as u64
                    && input.sha256 == digest(sf_xhl_tree_adapter::tree::DISABLED_CHAIN),
                "XHL chain is not the exact disabled chain"
            );
        } else {
            let relative = input
                .source
                .as_deref()
                .context("XHL input source path is missing")?;
            safe_relative(relative)?;
            ensure!(
                input.source_identity.is_some(),
                "XHL source file identity pin is missing"
            );
            let source = plan.source_root.join(relative.replace('/', "\\"));
            // RB65 owns source identity and bytes at prepare time; this template binds the approved plan and its prepared receipt.
            validate_e_path(&source, false, false)?;
        }
        let target_name = if name.starts_with("mods/") {
            format!("{XHL_SCAN_PREFIX}{}", &name[5..])
        } else {
            format!("mods/xhl-core/{name}")
        };
        ensure!(
            files
                .insert(
                    target_name,
                    FileBinding {
                        bytes: input.bytes,
                        sha256: input.sha256.clone()
                    }
                )
                .is_none(),
            "duplicate XHL contract path"
        );
    }
    let core = plan
        .files
        .get("xhl-core.dll")
        .context("XHL plan lacks Core")?;
    ensure!(
        core.bytes == 588_800 && core.sha256 == sf_xhl_tree_adapter::pins::CORE_SHA,
        "XHL Core image differs from the approved pin"
    );
    ensure!(
        plan.files.contains_key("dinput8-chain.ini"),
        "XHL plan lacks disabled chain"
    );
    for (plugin, pin) in sf_xhl_tree_adapter::pins::PLUGINS {
        let key = format!("mods/{plugin}");
        let row = plan
            .files
            .get(&key)
            .with_context(|| format!("XHL plan lacks selected plugin {plugin}"))?;
        ensure!(
            row.sha256 == pin,
            "XHL plugin differs from the approved pin: {plugin}"
        );
        if plugin == sf_xhl_tree_adapter::pins::GIFTING_PATH {
            ensure!(
                row.bytes == sf_xhl_tree_adapter::pins::GIFTING_BYTES,
                "XHL Gifting image size differs"
            );
        }
        let parent = plugin
            .rsplit_once('/')
            .context("invalid XHL plugin path")?
            .0;
        ensure!(
            plan.files
                .contains_key(&format!("mods/{parent}/native-plugin.ini")),
            "XHL declaration missing for {plugin}"
        );
    }
    for (name, row) in &plan.files {
        if name.to_ascii_lowercase().ends_with(".dll") {
            ensure!(
                name == "xhl-core.dll"
                    || sf_xhl_tree_adapter::pins::PLUGINS
                        .iter()
                        .any(|(path, _)| name == &format!("mods/{path}")),
                "unselected XHL DLL in plan: {name}"
            );
        }
        if is_reserved_xhl_output(name) {
            bail!("XHL writable output cannot be bound as input: {name}");
        }
        ensure!(row.bytes <= MAX_FILE_BYTES, "XHL file exceeds size bound");
    }
    let _ = raw_plan;
    Ok(files)
}
fn selected_matrix_rows(bytes: &[u8]) -> Result<BTreeMap<u8, String>> {
    let text = std::str::from_utf8(bytes).context("selected component manifest is not UTF-8")?;
    let header = text
        .lines()
        .position(|line| {
            line.split('|')
                .any(|cell| cell.trim().eq_ignore_ascii_case("selected feature"))
        })
        .context("selected component manifest lacks the Selected feature table")?;
    let mut rows = BTreeMap::new();
    for line in text.lines().skip(header + 1) {
        let line = line.trim();
        if !line.starts_with('|') {
            if !rows.is_empty() {
                break;
            }
            continue;
        }
        let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 2 {
            continue;
        }
        let Ok(row) = cells[0].parse::<u8>() else {
            continue;
        };
        ensure!(
            (1..=REQUIRED_FEATURES as u8).contains(&row),
            "selected component manifest has an out-of-range row"
        );
        let name = cells[1].strip_prefix('\u{60}').unwrap_or(cells[1]);
        let name = name.strip_suffix('\u{60}').unwrap_or(name).trim();
        ensure!(
            !name.is_empty(),
            "selected component row has no feature name"
        );
        ensure!(
            rows.insert(row, name.to_string()).is_none(),
            "selected component manifest has duplicate row {row}"
        );
    }
    ensure!(
        rows.len() == REQUIRED_FEATURES
            && (1..=REQUIRED_FEATURES as u8).all(|row| rows.contains_key(&row)),
        "selected component manifest must contain the exact 20 selected feature rows"
    );
    Ok(rows)
}

fn check_selected_feature_rows(manifest: &InputManifest, bytes: &[u8]) -> Result<()> {
    let source_rows = selected_matrix_rows(bytes)?;
    let mut bound_rows = BTreeMap::new();
    for feature in &manifest.selected_features {
        ensure!(
            (1..=REQUIRED_FEATURES as u8).contains(&feature.row) && !feature.name.trim().is_empty(),
            "selected feature row binding is invalid"
        );
        ensure!(
            feature.enabled,
            "the pinned selected-feature matrix cannot contain a disabled feature row"
        );
        ensure!(
            bound_rows
                .insert(feature.row, feature.name.trim().to_string())
                .is_none(),
            "duplicate selected feature row binding"
        );
    }
    ensure!(
        source_rows == bound_rows,
        "typed selected feature rows do not match the pinned 20-row selection manifest"
    );
    Ok(())
}

fn validate_runtime_service_rows(manifest: &InputManifest) -> Result<()> {
    let mut boot_services = BTreeSet::new();
    for service in &manifest.selection.boot_services {
        ensure!(safe_id(service), "invalid selected boot service ID");
        ensure!(
            boot_services.insert(service.to_ascii_lowercase()),
            "duplicate selected boot service ID"
        );
    }
    let mut declared_services = BTreeSet::new();
    for component in &manifest.components {
        if component.kind == ComponentKind::RuntimeService {
            let service = component
                .service_id
                .as_deref()
                .context("runtime service row lacks a service ID")?;
            ensure!(
                component.package_directory.is_none(),
                "nonpackage runtime service row cannot name a package directory"
            );
            ensure!(safe_id(service), "invalid runtime service ID");
            ensure!(
                declared_services.insert(service.to_ascii_lowercase()),
                "duplicate runtime service row"
            );
            ensure!(
                component.enabled == boot_services.contains(&service.to_ascii_lowercase()),
                "runtime service row enablement differs from the selected boot-service plan"
            );
        } else {
            ensure!(
                component.service_id.is_none(),
                "package component cannot declare a nonpackage runtime service ID"
            );
        }
    }
    ensure!(
        declared_services == boot_services,
        "runtime-service deployment units must exactly cover the selected boot-service IDs"
    );
    Ok(())
}

fn validate_enabled_runtime_plan(manifest: &InputManifest, runtime_ids: &[String]) -> Result<()> {
    let planned: BTreeSet<_> = runtime_ids
        .iter()
        .map(|id| id.to_ascii_lowercase())
        .collect();
    for component in &manifest.components {
        if component.kind == ComponentKind::RuntimeOnly && component.enabled {
            ensure!(
                planned.contains(&component.id.to_ascii_lowercase()),
                "enabled runtime-only unit is absent from the canonical runtime plan: {}",
                component.id
            );
        }
    }
    Ok(())
}

fn check_manifest_evidence(manifest: &InputManifest) -> Result<()> {
    ensure!(
        manifest.schema_version == 1 && safe_id(&manifest.candidate_id),
        "unsupported candidate input manifest"
    );
    ensure!(
        valid_digest(&manifest.prepared_producer_closure)
            && valid_digest(&manifest.runtime_source_closure),
        "source closure pin is invalid"
    );
    ensure!(
        valid_digest(&manifest.selected_manifest.sha256),
        "selected manifest digest is invalid"
    );
    let selected_manifest_bytes = check_raw_evidence(
        &manifest.selected_manifest.path,
        &manifest.selected_manifest.sha256,
        32 * 1024 * 1024,
    )?;
    check_selected_feature_rows(manifest, &selected_manifest_bytes)?;
    validate_feature_unit_mapping(manifest)?;
    validate_runtime_service_rows(manifest)?;
    let plan_bytes = check_raw_evidence(
        &manifest.xhl_plan.path,
        &manifest.xhl_plan.sha256,
        1024 * 1024,
    )?;
    let plan: ApprovedPlan =
        serde_json::from_slice(&plan_bytes).context("parse XHL approved plan")?;
    validate_xhl_unit_ownership(&manifest.components, &plan)?;
    let xhl_files = xhl_target_files(&plan, &plan_bytes, &manifest.target_root)?;
    ensure!(
        plan.transaction.len() >= 16 && plan.transaction.len() <= 128,
        "invalid XHL transaction ID"
    );
    ensure!(
        manifest.preservation_evidence.len() <= 64 && manifest.preservation.len() <= MAX_FILES,
        "preservation evidence is oversized"
    );
    let mut evidence_hashes = BTreeSet::new();
    for evidence in &manifest.preservation_evidence {
        ensure!(
            valid_digest(&evidence.sha256),
            "invalid preservation evidence digest"
        );
        check_raw_evidence(&evidence.path, &evidence.sha256, 32 * 1024 * 1024)?;
        evidence_hashes.insert(evidence.sha256.clone());
    }
    let mut preserved_targets = BTreeSet::new();
    for item in &manifest.preservation {
        ensure!(
            component_known(&manifest.components, &item.component_id),
            "preservation entry references an unselected component"
        );
        safe_relative(&item.target)?;
        ensure!(
            !item.fields.is_empty() && item.fields.iter().all(|field| !field.trim().is_empty()),
            "preservation entry needs named fields"
        );
        ensure!(
            !item.disposition.trim().is_empty(),
            "preservation entry needs an explicit disposition"
        );
        ensure!(
            evidence_hashes.contains(&item.evidence_sha256),
            "preservation entry is not bound to a checked evidence file"
        );
        ensure!(
            preserved_targets.insert(item.target.to_ascii_lowercase()),
            "duplicate preservation target"
        );
    }
    for package in &manifest.packages {
        for overlay in &package.overlays {
            let target = format!("mods/{}/{}", package.package_directory, overlay.path);
            ensure!(
                preserved_targets.contains(&target.to_ascii_lowercase()),
                "intended configuration overlay lacks a preservation record: {target}"
            );
        }
    }
    let mut greeneye = BTreeSet::new();
    for path in &manifest.greeneye_paths {
        safe_relative(path)?;
        ensure!(
            greeneye.insert(path.to_ascii_lowercase()),
            "duplicate Greeneye path"
        );
    }
    let greeneye_selected = manifest
        .components
        .iter()
        .any(|component| component.enabled && component.requires_greeneye);
    ensure!(
        greeneye_selected == !greeneye.is_empty(),
        "selected Greeneye components must bind their exact greeneyePaths, and unselected routes cannot declare them"
    );
    match (&manifest.greeneye_route, greeneye_selected) {
        (Some(route), true) => validate_greeneye_route(manifest, route)?,
        (None, true) => {}
        (None, false) => {}
        (Some(_), false) => bail!("Greeneye route evidence is present without a selected host"),
    }
    ensure!(
        manifest.selection.process_role == "client",
        "selected full-runtime contract must be client-only"
    );
    ensure!(
        manifest.contract_file_paths.len() <= MAX_CONTRACT_FILES,
        "too many runtime-bound files"
    );
    let mut bound = BTreeSet::new();
    for path in &manifest.contract_file_paths {
        safe_relative(path)?;
        ensure!(
            bound.insert(path.to_ascii_lowercase()),
            "duplicate runtime file binding"
        );
    }
    let mut missing_ids = BTreeSet::new();
    ensure!(
        manifest.missing_requirements.iter().all(|item| {
            !item.id.trim().is_empty()
                && !item.detail.trim().is_empty()
                && item.id != "greeneye-route-evidence"
                && missing_ids.insert(item.id.to_ascii_lowercase())
        }),
        "missing requirements need unique nonreserved IDs and details"
    );
    ensure!(
        manifest.missing_requirements.len() <= 128,
        "too many missing requirements"
    );
    let expected_target = manifest.target_root.join("mods/xhl-core");
    let _ = xhl_files;
    target_root(&expected_target)?;
    Ok(())
}
fn component_known(components: &[SelectedComponent], id: &str) -> bool {
    components
        .iter()
        .any(|component| component.id.eq_ignore_ascii_case(id))
}
fn create_parent_dirs(root: &Path, relative: &str) -> Result<PathBuf> {
    safe_relative(relative)?;
    let parts: Vec<_> = relative.split('/').collect();
    let mut current = root.to_path_buf();
    for part in &parts[..parts.len().saturating_sub(1)] {
        current.push(part);
        if current.exists() {
            ordinary(&current, true)?;
        } else {
            fs::create_dir(&current)
                .with_context(|| format!("create payload directory {}", current.display()))?;
            ordinary(&current, true)?;
        }
    }
    Ok(current)
}
fn write_new_file(root: &Path, file: &StagedSource) -> Result<Stamp> {
    validate_e_path(&file.source, true, false)?;
    check_stamp(&stamp(&file.source)?, &file.stamp, &file.target)?;
    let parent = create_parent_dirs(root, &file.target)?;
    let leaf = file
        .target
        .rsplit('/')
        .next()
        .context("empty output name")?;
    let destination = parent.join(leaf);
    let mut input = File::open(&file.source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .with_context(|| format!("create payload file {}", destination.display()))?;
    let actual = stamp_reader(&mut input)?;
    check_stamp(&actual, &file.stamp, &file.target)?;
    input.seek_to_start()?;
    let copied = std::io::copy(&mut input, &mut output)?;
    output.flush()?;
    ensure!(
        copied == file.stamp.bytes,
        "source size changed during copy: {}",
        file.target
    );
    drop(output);
    check_stamp(&stamp(&destination)?, &file.stamp, &file.target)?;
    Ok(file.stamp.clone())
}
trait SeekToStart {
    fn seek_to_start(&mut self) -> Result<()>;
}
impl SeekToStart for File {
    fn seek_to_start(&mut self) -> Result<()> {
        use std::io::Seek;
        self.seek(std::io::SeekFrom::Start(0))?;
        Ok(())
    }
}
fn copy_payload(payload: &Path, sources: &[StagedSource]) -> Result<BTreeMap<String, FileBinding>> {
    fs::create_dir(payload)
        .with_context(|| format!("create fresh payload root {}", payload.display()))?;
    ordinary(payload, true)?;
    let mut result = BTreeMap::new();
    let mut total = 0u64;
    for file in sources {
        let value = write_new_file(payload, file)?;
        total = total
            .checked_add(value.bytes)
            .context("payload size overflow")?;
        ensure!(total <= MAX_TOTAL_BYTES, "payload exceeds total size bound");
        ensure!(
            result
                .insert(
                    file.target.clone(),
                    FileBinding {
                        bytes: value.bytes,
                        sha256: value.sha256
                    }
                )
                .is_none(),
            "duplicate payload binding"
        );
    }
    ensure!(
        inventory(payload)?.len() == result.len(),
        "payload inventory has missing or unexpected files"
    );
    Ok(result)
}
fn read_payload_json<T: for<'de> Deserialize<'de>>(root: &Path, relative: &str) -> Result<T> {
    safe_relative(relative)?;
    let path = root.join(relative.replace('/', "\\"));
    validate_e_path(&path, true, false)?;
    serde_json::from_slice(&read_bounded(&path, 16 * 1024 * 1024)?)
        .with_context(|| format!("parse payload {relative}"))
}
fn verify_component_manifests(
    root: &Path,
    manifest: &InputManifest,
    env: &ModEnvironment,
) -> Result<()> {
    for component in &manifest.components {
        let Some(directory) = &component.package_directory else {
            continue;
        };
        let path = root.join("mods").join(directory);
        if !path.join("mod.json").exists() {
            ensure!(
                component.kind == ComponentKind::NativeOnly,
                "selected package manifest is missing: {}",
                component.id
            );
            ensure!(
                env.mod_registry().get(&component.id).is_none(),
                "native-only package unexpectedly parsed as a Lua mod: {}",
                component.id
            );
            continue;
        }
        let info = env
            .mod_registry()
            .get(&component.id)
            .with_context(|| format!("selected manifest ID was not admitted: {}", component.id))?
            .info();
        ensure!(
            info.version.to_string() == component.version,
            "selected version differs for {}",
            component.id
        );
        ensure!(
            info.enabled == component.enabled,
            "selected enablement differs for {}",
            component.id
        );
        ensure!(
            info.target.supports_process(false),
            "selected package cannot run in the client process: {}",
            component.id
        );
    }
    for package in &manifest.packages {
        let has_manifest = package.source_files.contains_key("mod.json")
            || package.overlays.iter().any(|item| item.path == "mod.json");
        if package.origin == PackageOrigin::ApprovedE && package.component_id != "xhl-core" {
            let component = manifest
                .components
                .iter()
                .find(|row| row.id.eq_ignore_ascii_case(&package.component_id))
                .unwrap();
            if component.kind == ComponentKind::RuntimeOnly {
                ensure!(
                    has_manifest,
                    "runtime-only component lacks its mod manifest: {}",
                    component.id
                );
            }
        }
    }
    Ok(())
}
fn effective_scripts(
    env: &ModEnvironment,
    runtime_ids: &[String],
) -> Result<BTreeMap<String, NativeFile>> {
    let mut result = BTreeMap::new();
    for id in runtime_ids {
        let package = env
            .mod_registry()
            .get(id)
            .with_context(|| format!("runtime package missing from candidate: {id}"))?;
        let mut fs = package.fs();
        let package_root = PathBuf::from(fs.root().as_str());
        let mut paths = Vec::new();
        for entry in fs.files() {
            let path = entry?;
            let Some(text) = package_relative_path(Path::new(path.as_str()), &package_root)? else {
                continue;
            };
            if !fs.is_file(&text) {
                continue;
            }
            if text.starts_with("src/") && text.to_ascii_lowercase().ends_with(".lua") {
                paths.push(text);
            }
        }
        paths.sort();
        for path in paths {
            let mut reader = fs
                .read_file(&path)
                .with_context(|| format!("read effective runtime script {id}:{path}"))?;
            let value = stamp_reader(&mut reader)?;
            let key = format!("{id}:{path}");
            ensure!(
                result
                    .insert(
                        key,
                        NativeFile {
                            bytes: value.bytes,
                            sha256: value.sha256
                        }
                    )
                    .is_none(),
                "duplicate effective runtime script"
            );
            ensure!(
                result.len() <= MAX_SCRIPTS,
                "too many effective runtime scripts"
            );
        }
        ensure!(
            result.contains_key(&format!("{id}:src/mod.lua")),
            "runtime entrypoint is missing for {id}"
        );
    }
    Ok(result)
}

fn xhl_profile_files(
    plan: &ApprovedPlan,
) -> Result<Vec<(String, shroudforge_package::dll_manager::DllLoadMode)>> {
    ensure!(
        plan.files.contains_key("xhl-core.dll"),
        "XHL DLL plan is missing Core"
    );
    let mut files = vec![(
        "xhl-core.dll".to_string(),
        shroudforge_package::dll_manager::DllLoadMode::Dinput8,
    )];
    for (plugin, _) in sf_xhl_tree_adapter::pins::PLUGINS {
        let relative = format!("mods/{plugin}");
        ensure!(
            plan.files.contains_key(&relative),
            "XHL DLL plan is missing selected plugin {plugin}"
        );
        files.push((
            relative,
            shroudforge_package::dll_manager::DllLoadMode::Library,
        ));
    }
    Ok(files)
}

fn check_xhl_core_selection(
    root: &Path,
    manifest: &InputManifest,
    plan: &ApprovedPlan,
) -> Result<()> {
    use shroudforge_package::dll_manager::{DllLoadMode, DllProfileTarget};
    let core_units: Vec<_> = manifest
        .components
        .iter()
        .filter(|unit| unit.id.eq_ignore_ascii_case("xhl-core"))
        .collect();
    ensure!(
        core_units.len() == 1
            && core_units[0].kind == ComponentKind::XhlNative
            && core_units[0].enabled
            && core_units[0].target.eq_ignore_ascii_case("client")
            && plan.files.contains_key("xhl-core.dll"),
        "selected client XHL Core unit or approved Core image is missing"
    );
    let selections = dll_manager::read_selections(root);
    let core_profiles: Vec<_> = selections
        .iter()
        .filter(|profile| profile.package_directory.eq_ignore_ascii_case("xhl-core"))
        .collect();
    ensure!(
        dll_manager::enabled(root) && core_profiles.len() == 1,
        "XHL Core requires exactly one enabled DLL Manager selection"
    );
    let selection = core_profiles[0];
    let expected_files = xhl_profile_files(plan)?;
    ensure!(
        selection.id.eq_ignore_ascii_case("xhl-core")
            && selection.enabled
            && selection.approved
            && matches!(selection.target, DllProfileTarget::Client)
            && selection
                .compatibility_profile
                .as_deref()
                .is_none_or(|id| id.is_empty() || id == "none")
            && selection.files.len() == expected_files.len()
            && selection
                .files
                .iter()
                .zip(expected_files.iter())
                .all(|(actual, (path, mode))| {
                    actual.path.eq_ignore_ascii_case(path) && actual.mode == *mode
                })
            && selection.files.first().is_some_and(|file| {
                file.path.eq_ignore_ascii_case("xhl-core.dll") && file.mode == DllLoadMode::Dinput8
            }),
        "XHL Core DLL Manager selection differs from the approved client plan"
    );
    let legacy_gifting: Vec<_> = selections
        .iter()
        .filter(|profile| {
            profile
                .package_directory
                .eq_ignore_ascii_case("XHL-Unlimited-Gifting-Range")
        })
        .collect();
    ensure!(
        legacy_gifting.len() == 1
            && !legacy_gifting[0].enabled
            && legacy_gifting[0].approved
            && matches!(legacy_gifting[0].target, DllProfileTarget::Client)
            && legacy_gifting[0].files.is_empty()
            && legacy_gifting[0]
                .compatibility_profile
                .as_deref()
                .is_none_or(|id| id.is_empty() || id == "none"),
        "legacy Gifting directory must have one explicit disabled client DLL selection"
    );
    Ok(())
}

fn planning_target(name: &str) -> Result<String> {
    safe_relative(name)?;
    Ok(format!("mods/xhl-core/{name}"))
}

fn write_planning_bytes(root: &Path, target: &str, bytes: &[u8], expected: &Stamp) -> Result<()> {
    let parent = create_parent_dirs(root, target)?;
    let leaf = target
        .rsplit('/')
        .next()
        .context("empty planning-view output name")?;
    let destination = parent.join(leaf);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .with_context(|| format!("create planning-view file {}", destination.display()))?;
    output.write_all(bytes)?;
    output.flush()?;
    drop(output);
    check_stamp(&stamp(&destination)?, expected, target)
}

fn copy_open_source_to_planning_view(
    source_root: &Path,
    root: &Path,
    target: &str,
    input: &sf_xhl_tree_adapter::InputFile,
    identities: &mut BTreeSet<(u64, [u8; 16])>,
) -> Result<()> {
    let source_relative = input
        .source
        .as_deref()
        .context("approved XHL source path is missing")?;
    safe_relative(source_relative)?;
    let source = source_root.join(source_relative.replace('/', "\\"));
    validate_e_path(&source, true, false)?;
    let mut input_file = File::open(&source)
        .with_context(|| format!("open approved XHL source {}", source.display()))?;
    let (volume, id) = sf_xhl_tree_adapter::tree::held_file_identity(&input_file)
        .with_context(|| format!("read approved XHL file identity {}", source.display()))?;
    let expected_identity = input
        .source_identity
        .as_ref()
        .context("approved XHL source file identity is missing")?;
    ensure!(
        expected_identity.volume == volume && expected_identity.id == id,
        "approved XHL source identity drifted: {source_relative}"
    );
    ensure!(
        identities.insert((volume, id)),
        "approved XHL source file identity aliases another input: {source_relative}"
    );
    let expected = Stamp {
        bytes: input.bytes,
        sha256: input.sha256.clone(),
    };
    check_stamp(&stamp_reader(&mut input_file)?, &expected, source_relative)?;
    input_file.seek_to_start()?;
    let parent = create_parent_dirs(root, target)?;
    let leaf = target
        .rsplit('/')
        .next()
        .context("empty planning-view output name")?;
    let destination = parent.join(leaf);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .with_context(|| format!("create planning-view file {}", destination.display()))?;
    let copied = std::io::copy(&mut input_file, &mut output)?;
    output.flush()?;
    drop(output);
    ensure!(
        copied == expected.bytes,
        "approved XHL source size changed while copying: {source_relative}"
    );
    check_stamp(&stamp(&destination)?, &expected, target)
}

fn materialize_xhl_planning_view(
    payload_root: &Path,
    view_root: &Path,
    plan: &ApprovedPlan,
    xhl_files: &BTreeMap<String, FileBinding>,
) -> Result<()> {
    ensure!(
        fs::symlink_metadata(view_root)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "DLL planning view must be fresh"
    );
    fs::create_dir(view_root)
        .with_context(|| format!("create E-only DLL planning view {}", view_root.display()))?;
    ordinary(view_root, true)?;
    let source_key = normalize_absolute(&plan.source_root)?;
    let output_key = normalize_absolute(
        view_root
            .parent()
            .context("DLL planning view has no owned output parent")?,
    )?;
    let is_same_or_child = |parent: &str, candidate: &str| {
        candidate == parent
            || candidate
                .strip_prefix(parent)
                .is_some_and(|suffix| suffix.starts_with('\\'))
    };
    ensure!(
        !is_same_or_child(&source_key, &output_key) && !is_same_or_child(&output_key, &source_key),
        "approved XHL sources overlap the candidate output tree"
    );
    let mut expected = BTreeMap::new();
    let mut aliases = BTreeSet::new();
    for (name, value) in inventory(payload_root)? {
        ensure!(
            !name.to_ascii_lowercase().starts_with("mods/xhl-core/"),
            "ordinary payload overlaps the XHL-owned package tree: {name}"
        );
        ensure!(
            aliases.insert(path_key(&name)),
            "ordinary payload contains a planning-view path alias: {name}"
        );
        let file = StagedSource {
            source: payload_root.join(name.replace('/', "\\")),
            target: name.clone(),
            stamp: value.clone(),
            runtime_binding: false,
        };
        write_new_file(view_root, &file)?;
        expected.insert(name, value);
    }

    validate_e_path(&plan.source_root, true, true)?;
    let source_chain = sf_xhl_tree_adapter::tree::pin_directory_chain(&plan.source_root)
        .context("pin approved XHL source root for planning")?;
    let source_identity = source_chain
        .first()
        .context("approved XHL source root identity is missing")?
        .identity();
    ensure!(
        plan.source_root_identity.volume == source_identity.volume
            && plan.source_root_identity.id == source_identity.id,
        "approved XHL source-root identity drifted"
    );
    let mut source_identities = BTreeSet::new();
    for (name, input) in &plan.files {
        let target = planning_target(name)?;
        let expected_stamp = Stamp {
            bytes: input.bytes,
            sha256: input.sha256.clone(),
        };
        let binding = xhl_files
            .get(&target)
            .with_context(|| format!("approved XHL target is missing from file map: {target}"))?;
        ensure!(
            binding.bytes == expected_stamp.bytes && binding.sha256 == expected_stamp.sha256,
            "approved XHL target stamp differs from plan: {target}"
        );
        ensure!(
            aliases.insert(path_key(&target)),
            "approved XHL path conflicts with ordinary payload: {target}"
        );
        if name == "dinput8-chain.ini" {
            ensure!(
                input.source.is_none() && input.source_identity.is_none(),
                "disabled XHL chain must be generated from the canonical bytes"
            );
            let chain_stamp = Stamp {
                bytes: sf_xhl_tree_adapter::tree::DISABLED_CHAIN.len() as u64,
                sha256: digest(sf_xhl_tree_adapter::tree::DISABLED_CHAIN),
            };
            check_stamp(&expected_stamp, &chain_stamp, name)?;
            write_planning_bytes(
                view_root,
                &target,
                sf_xhl_tree_adapter::tree::DISABLED_CHAIN,
                &chain_stamp,
            )?;
        } else {
            copy_open_source_to_planning_view(
                &plan.source_root,
                view_root,
                &target,
                input,
                &mut source_identities,
            )?;
        }
        expected.insert(target, expected_stamp);
    }
    ensure!(
        xhl_files.len() == plan.files.len() && inventory(view_root)? == expected,
        "DLL planning view membership differs from exact payload plus approved XHL inputs"
    );
    Ok(())
}

fn remove_planning_view(view_root: &Path, output_root: &Path) -> Result<()> {
    ensure!(
        view_root.parent() == Some(output_root),
        "DLL planning view is outside its owned candidate output"
    );
    ordinary(view_root, true)?;
    let _ = inventory(view_root)?;
    fs::remove_dir_all(view_root)
        .with_context(|| format!("remove E-only DLL planning view {}", view_root.display()))?;
    ensure!(
        fs::symlink_metadata(view_root)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "DLL planning view remained after cleanup"
    );
    Ok(())
}

fn resolve_xhl_dll_plan(
    payload_root: &Path,
    manifest: &InputManifest,
    plan: &ApprovedPlan,
    xhl_files: &BTreeMap<String, FileBinding>,
) -> Result<Value> {
    ensure!(
        !manifest
            .contract_file_paths
            .iter()
            .any(|name| name.to_ascii_lowercase().starts_with("mods/xhl-core/")),
        "XHL file bindings remain deferred until the exact prepared tree receipt"
    );
    let output_root = manifest.output_root.as_path();
    let view_root = output_root.join(".dll-planning-view");
    let result = (|| {
        materialize_xhl_planning_view(payload_root, &view_root, plan, xhl_files)?;
        check_xhl_core_selection(&view_root, manifest, plan)?;
        let expected_files = inventory(&view_root)?;
        let mut dll_plan = dll_manager::resolve_load_plan(&view_root, false);
        ensure!(
            inventory(&view_root)? == expected_files,
            "canonical DLL resolver changed the E-only planning view"
        );
        ensure!(
            dll_plan.issues.is_empty(),
            "native DLL plan has issues: {}",
            serde_json::to_string(&dll_plan.issues)?
        );
        let expected_xhl = xhl_profile_files(plan)?;
        let mut actual_xhl = BTreeMap::new();
        let mut identity = dll_manager::load_plan_identity(&dll_plan);
        let entries = identity
            .as_array_mut()
            .context("canonical DLL identity is not an array")?;
        ensure!(
            entries.len() == dll_plan.entries.len(),
            "canonical DLL identity entry count differs"
        );
        let payload_inventory = inventory(payload_root)?;
        for (value, entry) in entries.iter_mut().zip(dll_plan.entries.iter_mut()) {
            let name = relative_native_path(&entry.path, &view_root).with_context(|| {
                format!(
                    "DLL plan escapes E-only planning view: {}",
                    entry.path.display()
                )
            })?;
            let actual = expected_files
                .get(&name)
                .with_context(|| format!("DLL plan target is absent from planning view: {name}"))?;
            if entry.package_directory.eq_ignore_ascii_case("xhl-core") {
                let profile_name = name
                    .strip_prefix("mods/xhl-core/")
                    .context("XHL resolver path escaped its package directory")?;
                let expected = expected_xhl
                    .iter()
                    .find(|(path, _)| path.eq_ignore_ascii_case(profile_name))
                    .with_context(|| format!("XHL resolver selected an undeclared DLL: {name}"))?;
                ensure!(
                    entry.mode == expected.1,
                    "XHL DLL mode differs from its selected profile: {name}"
                );
                let target = planning_target(profile_name)?;
                let binding = xhl_files
                    .get(&target)
                    .context("XHL resolver selected a DLL without an approved plan input")?;
                ensure!(
                    actual.bytes == binding.bytes && actual.sha256 == binding.sha256,
                    "XHL resolver DLL bytes differ from the approved plan: {name}"
                );
                ensure!(
                    actual_xhl
                        .insert(profile_name.to_ascii_lowercase(), entry.mode)
                        .is_none(),
                    "XHL resolver selected a DLL more than once: {name}"
                );
            } else {
                ensure!(
                    !name.to_ascii_lowercase().starts_with("mods/xhl-core/")
                        && payload_inventory.contains_key(&name),
                    "DLL plan target is outside the selected payload or XHL package: {name}"
                );
                let payload_stamp = &payload_inventory[&name];
                ensure!(
                    actual == payload_stamp,
                    "ordinary DLL bytes differ from the exact candidate payload: {name}"
                );
            }
            let rebound = target_path(&manifest.target_root, &name)?;
            value["path"] = json!(rebound);
            entry.path = rebound;
        }
        let expected_xhl: BTreeMap<_, _> = expected_xhl
            .into_iter()
            .map(|(path, mode)| (path.to_ascii_lowercase(), mode))
            .collect();
        ensure!(
            actual_xhl == expected_xhl
                && actual_xhl
                    .iter()
                    .filter(|(path, _)| path.eq_ignore_ascii_case("xhl-core.dll"))
                    .count()
                    == 1,
            "canonical DLL plan does not contain every selected XHL DLL exactly once"
        );
        Ok(identity)
    })();
    let cleanup = if fs::symlink_metadata(&view_root).is_ok() {
        remove_planning_view(&view_root, output_root)
    } else {
        Ok(())
    };
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(error), Err(cleanup_error)) => Err(error.context(format!(
            "DLL planning-view cleanup also failed: {cleanup_error:#}"
        ))),
    }
}

fn runtime_plan(
    root: &Path,
    manifest: &InputManifest,
    xhl_plan: &ApprovedPlan,
    xhl_files: &BTreeMap<String, FileBinding>,
) -> Result<(Vec<String>, Value, BTreeMap<String, NativeFile>)> {
    let text = root
        .to_str()
        .context("candidate payload path is not UTF-8")?;
    let env = ModEnvironment::load(text)
        .map_err(|error| anyhow::anyhow!("candidate mod environment: {error:?}"))?;
    verify_component_manifests(root, manifest, &env)?;
    let (mut runtime, runtime_issues) = env.runtime_plan_report_detailed(false, API_VERSION);
    ensure!(
        runtime_issues.is_empty(),
        "runtime dependency plan has issues: {}",
        serde_json::to_string(&runtime_issues)?
    );
    let loader =
        shroudforge_loader_config::config::read_loader(root).map_err(anyhow::Error::msg)?;
    if loader
        .pointer("/modules/worldEditor/enabled")
        .and_then(Value::as_bool)
        == Some(false)
    {
        runtime.retain(|package| package.info().id != "world-editor");
    }
    let selected_ids: BTreeSet<_> = manifest
        .components
        .iter()
        .filter(|component| {
            matches!(
                component.kind,
                ComponentKind::AssetProvider | ComponentKind::RuntimeOnly
            )
        })
        .map(|component| component.id.as_str())
        .collect();
    ensure!(
        runtime
            .iter()
            .all(|package| selected_ids.contains(package.info().id.as_str())),
        "runtime plan contains a package outside the selected deployment-unit manifest"
    );
    let runtime_ids: Vec<_> = runtime
        .iter()
        .map(|package| package.info().id.clone())
        .collect();
    validate_enabled_runtime_plan(manifest, &runtime_ids)?;
    ensure!(
        runtime_ids.len() <= MAX_RUNTIME_MODS,
        "runtime plan exceeds maximum package count"
    );
    let (asset_plan, asset_issues) = env.plan_report_detailed(false, API_VERSION);
    ensure!(
        asset_issues.is_empty(),
        "native DLL activation plan has package issues: {}",
        serde_json::to_string(&asset_issues)?
    );
    let identity = resolve_xhl_dll_plan(root, manifest, xhl_plan, xhl_files)?;
    let scripts = effective_scripts(&env, &runtime_ids)?;
    // Force the exact public manager plan to remain the one represented in the contract.
    let _ = asset_plan;
    // Keep the canonical plan object local; only its serialized identity escapes.
    Ok((runtime_ids, identity, scripts))
}
fn flight_services_match(contract: &Contract, env: &ModEnvironment) -> Result<()> {
    use shroudforge_api::current_assets::full_runtime::selected_services as services;
    let plan = services::plan(contract)?;
    if let Some(package) = env.mod_registry().get("sf-unlimited-flight") {
        let info = package.info();
        if info.enabled
            && info.target.supports_process(false)
            && info
                .capabilities
                .iter()
                .any(|capability| capability.requires_runtime())
        {
            let (generic, flight) = services::flight_settings(&info.setting_values)?;
            ensure!(
                (plan.services & services::GENERIC_HOOKS != 0) == generic,
                "shared-speed startup service differs from selected Flight settings"
            );
            ensure!(
                (plan.services & services::FLIGHT_SCALAR != 0) == flight,
                "Flight scalar startup service differs from selected F6 setting"
            );
        } else {
            ensure!(
                plan.services & (services::GENERIC_HOOKS | services::FLIGHT_SCALAR) == 0,
                "Flight services selected while the bundled package is disabled or unavailable"
            );
        }
    } else {
        ensure!(
            plan.services & (services::GENERIC_HOOKS | services::FLIGHT_SCALAR) == 0,
            "Flight services selected without the selected bundled package"
        );
    }
    Ok(())
}
fn validate_preserved_runtime_files(
    entries: &[PreservedRuntimeFile],
    target: &Path,
    selected_vein_path: Option<&str>,
    preservation: &[PreservationEntry],
    payload_files: &BTreeMap<String, FileBinding>,
) -> Result<()> {
    ensure!(
        entries.len() <= 1,
        "only the selected preserved Vein INI is supported"
    );
    for item in entries {
        safe_relative(&item.target)?;
        ensure!(
            selected_vein_path == Some(item.target.as_str()) && item.target.ends_with(".ini"),
            "preserved runtime binding is not the selected Vein INI"
        );
        ensure!(
            !payload_files
                .keys()
                .any(|path| path.eq_ignore_ascii_case(&item.target)),
            "preserved runtime file must not be in the write payload"
        );
        ensure!(
            valid_digest(&item.stamp.sha256),
            "invalid preserved runtime digest"
        );
        ensure!(
            preservation.iter().any(|entry| entry.target == item.target
                && entry.component_id == item.component_id
                && entry.evidence_sha256 == item.evidence.sha256),
            "preserved runtime input lacks its matching preservation record"
        );
        let bytes =
            check_raw_evidence(&item.evidence.path, &item.evidence.sha256, 32 * 1024 * 1024)?;
        let evidence: Value = serde_json::from_slice(&bytes)?;
        let evidence_root = evidence
            .get("sourceRoot")
            .and_then(Value::as_str)
            .context("preserved runtime evidence lacks sourceRoot")?;
        ensure!(
            same_path(Path::new(evidence_root), target),
            "preserved runtime evidence targets another installation"
        );
        let rows = evidence
            .get("files")
            .and_then(Value::as_array)
            .context("preserved runtime evidence lacks files")?;
        let matching: Vec<_> = rows
            .iter()
            .filter(|row| {
                row.get("relativePath")
                    .and_then(Value::as_str)
                    .is_some_and(|path| path.eq_ignore_ascii_case(&item.target))
            })
            .collect();
        ensure!(
            matching.len() == 1,
            "preserved runtime evidence row is missing or ambiguous"
        );
        let row = matching[0];
        ensure!(
            row.get("relativePath").and_then(Value::as_str) == Some(item.target.as_str())
                && row.get("status").and_then(Value::as_str) == Some("PRESENT_PRESERVE_BYTES")
                && row.get("bytes").and_then(Value::as_u64) == Some(item.stamp.bytes)
                && row.get("sha256").and_then(Value::as_str) == Some(item.stamp.sha256.as_str())
                && row.get("postSha256").and_then(Value::as_str)
                    == Some(item.stamp.sha256.as_str()),
            "preserved runtime evidence differs from the exact unchanged file stamp"
        );
    }
    Ok(())
}

fn patch_config_provider_feature(provider_id: &str) -> Result<u8> {
    PRESERVED_PATCH_CONFIG_PROVIDERS
        .iter()
        .find_map(|(id, row)| (*id == provider_id).then_some(*row))
        .context("patch config provider is outside the two selected preserve-only modules")
}

fn validate_patch_config_values(
    provider_id: &str,
    values: &PreservedPatchEffectiveValues,
) -> Result<()> {
    match (provider_id, values) {
        ("xhl-rested-enhanced", PreservedPatchEffectiveValues::Rested(values)) => ensure!(
            values.enabled,
            "selected Rested config must preserve enabled=true"
        ),
        ("xhl-workshop-speed-20x", PreservedPatchEffectiveValues::Workshop(values)) => ensure!(
            values.enabled && values.multiplier == 20,
            "selected Workshop config must preserve enabled=true and integer multiplier=20"
        ),
        _ => bail!("patch config effective-value shape/provider mismatch"),
    }
    Ok(())
}

fn validate_patch_file_pin(pin: &PatchFilePin) -> Result<()> {
    ensure!(
        pin.bytes > 0,
        "preserved patch config must be a nonempty file"
    );
    ensure!(
        valid_digest(&pin.sha256),
        "invalid preserved patch config digest"
    );
    ensure!(
        pin.identity.volume > 0 && pin.identity.id.iter().any(|byte| *byte != 0),
        "preserved patch config requires full nonzero FileIdInfo"
    );
    Ok(())
}

fn package_manifest_id(source_root: &Path) -> Result<String> {
    validate_e_path(&source_root, true, true)?;
    let manifest_path = source_root.join("mod.json");
    validate_e_path(&manifest_path, true, false)?;
    let bytes = fs::read(&manifest_path)
        .with_context(|| format!("read pinned package manifest {}", manifest_path.display()))?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "package manifest is oversized"
    );
    let value: Value = serde_json::from_slice(&bytes).context("parse selected package manifest")?;
    let parsed = parse_manifest(&source_root, value).map_err(anyhow::Error::msg)?;
    Ok(parsed.id)
}

fn validate_package_manifest_id(
    source_root: &Path,
    expected_stamp: &Stamp,
    target: &str,
) -> Result<String> {
    let manifest_path = source_root.join("mod.json");
    check_stamp(&stamp(&manifest_path)?, expected_stamp, target)?;
    package_manifest_id(source_root)
}

fn validate_preserved_patch_config_evidence(
    entries: &[PreservedPatchConfig],
    provider_by_component: &BTreeMap<String, String>,
    target_root: &Path,
    preservation: &[PreservationEntry],
) -> Result<()> {
    ensure!(
        entries.len() == PRESERVED_PATCH_CONFIG_PROVIDERS.len(),
        "both selected patch config leaves require explicit preserve-only bindings"
    );
    let mut component_ids = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let mut providers = BTreeSet::new();
    let first_evidence = entries
        .first()
        .context("preserved patch config bindings are missing")?
        .evidence
        .clone();
    for entry in entries {
        ensure!(
            component_ids.insert(entry.component_id.clone()),
            "duplicate preserved patch config component"
        );
        ensure!(
            targets.insert(entry.target.to_ascii_lowercase()),
            "case-insensitive preserved patch config target collision"
        );
        let provider_id = provider_by_component
            .get(&entry.component_id)
            .context("preserved patch config does not resolve to a selected package provider")?;
        ensure!(
            providers.insert(provider_id.clone()),
            "duplicate preserved patch config provider"
        );
        patch_config_provider_feature(provider_id)?;
        validate_patch_config_values(provider_id, &entry.effective_values)?;
        ensure!(
            valid_digest(&entry.source_stamp.sha256) && entry.source_stamp.bytes > 0,
            "preserved patch source stamp is invalid"
        );
        validate_patch_file_pin(&entry.observed_target_before)?;
        ensure!(
            entry.required_poststate == entry.observed_target_before,
            "preserved patch config poststate must equal its observed before pin"
        );
        safe_relative(&entry.target)?;
        ensure!(
            entry.target.to_ascii_lowercase().ends_with("/config.txt"),
            "preserved patch target must be the selected config.txt leaf"
        );
        ensure!(
            entry.evidence.path == first_evidence.path
                && entry.evidence.sha256 == first_evidence.sha256,
            "both patch config rows must share one pinned qualification receipt"
        );
        let records: Vec<_> = preservation
            .iter()
            .filter(|item| {
                item.component_id == entry.component_id
                    && item.target.eq_ignore_ascii_case(&entry.target)
            })
            .collect();
        ensure!(
            records.len() == 1
                && records[0].evidence_sha256 == entry.evidence.sha256
                && records[0]
                    .fields
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    == PRESERVED_PATCH_CONFIG_FIELDS
                && records[0].disposition == PRESERVED_PATCH_CONFIG_DISPOSITION,
            "patch config lacks its exact selected-value preservation record"
        );
        ensure!(
            entries
                .iter()
                .filter(|candidate| candidate.evidence.path == entry.evidence.path)
                .count()
                == entries.len(),
            "patch config evidence reference is inconsistent"
        );
    }
    let expected_providers: BTreeSet<_> = PRESERVED_PATCH_CONFIG_PROVIDERS
        .iter()
        .map(|(id, _)| (*id).to_owned())
        .collect();
    ensure!(
        providers == expected_providers,
        "preserved patch config provider set differs from the two selected providers"
    );
    let bytes = check_raw_evidence(
        &first_evidence.path,
        &first_evidence.sha256,
        4 * 1024 * 1024,
    )?;
    let qualification: PatchConfigQualification = serde_json::from_slice(&bytes)
        .context("parse strict patch config qualification evidence")?;
    ensure!(
        qualification.schema_version == 1 && same_path(&qualification.target_root, target_root),
        "patch config qualification receipt has a different schema or target root"
    );
    ensure!(
        qualification.rows.len() == entries.len(),
        "patch config qualification must contain exactly the selected rows"
    );
    let mut evidence_components = BTreeSet::new();
    let mut evidence_targets = BTreeSet::new();
    for row in &qualification.rows {
        ensure!(
            evidence_components.insert(row.component_id.clone())
                && evidence_targets.insert(row.relative_path.to_ascii_lowercase()),
            "qualification receipt has duplicate component or target rows"
        );
    }
    for entry in entries {
        let provider_id = provider_by_component
            .get(&entry.component_id)
            .context("patch config provider mapping disappeared")?;
        let rows: Vec<_> = qualification
            .rows
            .iter()
            .filter(|row| row.component_id == entry.component_id)
            .collect();
        ensure!(
            rows.len() == 1,
            "patch config qualification row is missing or ambiguous"
        );
        let row = rows[0];
        ensure!(
            row.relative_path.eq_ignore_ascii_case(&entry.target)
                && row.source.stamp == entry.source_stamp
                && row.source.effective_values == entry.effective_values
                && row.selected.effective_values == entry.effective_values
                && row.target.effective_values == entry.effective_values
                && row.target.observed_before == entry.observed_target_before,
            "source, selected, and target patch config projections or pins differ"
        );
        validate_patch_config_values(provider_id, &row.source.effective_values)?;
        validate_patch_config_values(provider_id, &row.selected.effective_values)?;
        validate_patch_config_values(provider_id, &row.target.effective_values)?;
    }
    Ok(())
}

fn validate_preserved_patch_config_inputs(
    manifest: &InputManifest,
    request: &AssetRequest,
) -> Result<BTreeMap<String, String>> {
    ensure!(
        manifest.preserved_patch_configs.len() == PRESERVED_PATCH_CONFIG_PROVIDERS.len(),
        "both selected patch config leaves require explicit preserve-only bindings"
    );
    let mut provider_by_component = BTreeMap::new();
    let mut targets = BTreeSet::new();
    for entry in &manifest.preserved_patch_configs {
        let component = manifest
            .components
            .iter()
            .find(|component| component.id == entry.component_id)
            .context("preserved patch config component is not selected")?;
        ensure!(
            component.kind == ComponentKind::AssetProvider
                && component.enabled
                && component.package_directory.is_some(),
            "preserved patch config must belong to an enabled asset-provider unit"
        );
        let package = manifest
            .packages
            .iter()
            .find(|package| package.component_id == entry.component_id)
            .context("preserved patch config component has no exact package input")?;
        ensure!(
            package.origin == PackageOrigin::AssetEnvironment
                && package.package_directory == component.package_directory.as_deref().unwrap(),
            "preserved patch config package does not match its selected unit"
        );
        let source_root = request
            .environment_root
            .join("mods")
            .join(&package.package_directory);
        let provider_id = validate_package_manifest_id(
            &source_root,
            package
                .source_files
                .get("mod.json")
                .context("preserved patch package lacks mod.json in its source inventory")?,
            "mod.json",
        )?;
        let expected_feature = patch_config_provider_feature(&provider_id)?;
        ensure!(
            component.feature_rows.contains(&expected_feature)
                && request
                    .selected_ids
                    .iter()
                    .filter(|id| **id == provider_id)
                    .count()
                    == 1
                && manifest
                    .asset_provider_ids
                    .iter()
                    .filter(|id| **id == provider_id)
                    .count()
                    == 1,
            "preserved patch config provider is not mapped to the exact selected feature and request"
        );
        let directory = component.package_directory.as_deref().unwrap();
        let target = format!("mods/{directory}/config.txt");
        safe_relative(&target)?;
        ensure!(
            target.eq_ignore_ascii_case(&entry.target),
            "preserved patch config target differs from its selected package"
        );
        let source_stamp = package
            .source_files
            .get("config.txt")
            .context("preserved patch config is missing from the complete source inventory")?;
        ensure!(
            source_stamp == &entry.source_stamp,
            "preserved patch config source stamp differs from the full package inventory"
        );
        ensure!(
            targets.insert(target.to_ascii_lowercase()),
            "selected patch config target is duplicated"
        );
        let component_key = entry.component_id.clone();
        ensure!(
            provider_by_component
                .insert(component_key, provider_id)
                .is_none(),
            "duplicate preserved patch config component"
        );
        for other in [
            manifest
                .root_files
                .iter()
                .any(|item| item.target.eq_ignore_ascii_case(&target)),
            manifest
                .contract_file_paths
                .iter()
                .any(|path| path.eq_ignore_ascii_case(&target)),
            manifest
                .greeneye_paths
                .iter()
                .any(|path| path.eq_ignore_ascii_case(&target)),
            manifest
                .preserved_runtime_files
                .iter()
                .any(|item| item.target.eq_ignore_ascii_case(&target)),
            manifest.packages.iter().any(|item| {
                item.overlays.iter().any(|overlay| {
                    format!("mods/{}/{}", item.package_directory, overlay.path)
                        .eq_ignore_ascii_case(&target)
                })
            }),
        ] {
            ensure!(
                !other,
                "preserved patch config target collides with another target declaration"
            );
        }
    }
    ensure!(
        manifest.preserved_patch_configs.len() == provider_by_component.len(),
        "preserved patch config component rows are ambiguous"
    );
    for entry in &manifest.preserved_patch_configs {
        ensure!(
            manifest
                .preservation_evidence
                .iter()
                .filter(|pin| pin.path == entry.evidence.path && pin.sha256 == entry.evidence.sha256)
                .count()
                == 1,
            "patch config qualification receipt must appear exactly once in preservationEvidence"
        );
    }
    validate_preserved_patch_config_evidence(
        &manifest.preserved_patch_configs,
        &provider_by_component,
        &manifest.target_root,
        &manifest.preservation,
    )?;
    Ok(provider_by_component)
}

fn validate_preserved_patch_config_template(
    template: &Template,
    request: &AssetRequest,
) -> Result<()> {
    ensure!(
        template.preserved_patch_configs.len() == PRESERVED_PATCH_CONFIG_PROVIDERS.len(),
        "template must bind both selected patch config leaves"
    );
    let mut provider_by_component = BTreeMap::new();
    let mut targets = BTreeSet::new();
    for entry in &template.preserved_patch_configs {
        let component = template
            .selected_components
            .iter()
            .find(|component| component.id == entry.component_id)
            .context("preserved patch config component is absent from template selection")?;
        ensure!(
            component.kind == ComponentKind::AssetProvider
                && component.enabled
                && component.target == "client",
            "preserved patch config must belong to an enabled selected client asset provider"
        );
        let directory = component
            .package_directory
            .as_deref()
            .context("preserved patch config selected package directory is missing")?;
        safe_relative(directory)?;
        ensure!(
            directory.split('/').count() == 1,
            "preserved patch config package directory is not one segment"
        );
        let prefix = format!("mods/{directory}/");
        let manifest_rel = format!("{prefix}mod.json");
        let manifest_stamp = request
            .environment_files
            .iter()
            .filter(|(path, _)| path.eq_ignore_ascii_case(&manifest_rel))
            .map(|(_, stamp)| stamp)
            .collect::<Vec<_>>();
        ensure!(
            manifest_stamp.len() == 1,
            "preserved patch package manifest is absent or aliased in the immutable environment"
        );
        let source_root = request.environment_root.join("mods").join(directory);
        let provider_id =
            validate_package_manifest_id(&source_root, manifest_stamp[0], "mod.json")?;
        let feature_row = patch_config_provider_feature(&provider_id)?;
        ensure!(
            component.feature_rows.contains(&feature_row)
                && request
                    .selected_ids
                    .iter()
                    .filter(|id| **id == provider_id)
                    .count()
                    == 1
                && template
                    .asset_provider_ids
                    .iter()
                    .filter(|id| **id == provider_id)
                    .count()
                    == 1
                && template
                    .asset_request_selected_ids
                    .iter()
                    .filter(|id| **id == provider_id)
                    .count()
                    == 1,
            "preserved patch provider does not resolve through the selected feature, unit, and immutable request"
        );
        let expected_target = format!("{prefix}config.txt");
        ensure!(
            entry.target.eq_ignore_ascii_case(&expected_target)
                && targets.insert(entry.target.to_ascii_lowercase()),
            "preserved patch target differs from its selected package or is duplicated"
        );
        let source_rows = request
            .environment_files
            .iter()
            .filter(|(path, _)| path.eq_ignore_ascii_case(&expected_target))
            .map(|(_, stamp)| stamp)
            .collect::<Vec<_>>();
        ensure!(
            source_rows.len() == 1 && source_rows[0] == &entry.source_stamp,
            "preserved patch source stamp is absent or differs from the immutable request"
        );
        let source_path = source_root.join("config.txt");
        validate_e_path(&source_path, true, false)?;
        check_stamp(&stamp(&source_path)?, &entry.source_stamp, &expected_target)?;
        ensure!(
            provider_by_component
                .insert(entry.component_id.clone(), provider_id)
                .is_none(),
            "duplicate preserved patch config component"
        );
        let target = normalized_contract_path(&entry.target);
        ensure!(
            !template
                .payload_files
                .keys()
                .any(|path| normalized_contract_path(path) == target)
                && !template
                    .asset_postimages
                    .keys()
                    .any(|path| normalized_contract_path(path) == target)
                && !template
                    .contract_file_paths
                    .iter()
                    .any(|path| normalized_contract_path(path) == target)
                && !template
                    .greeneye_paths
                    .iter()
                    .any(|path| normalized_contract_path(path) == target)
                && !template
                    .xhl_files
                    .keys()
                    .any(|path| normalized_contract_path(path) == target)
                && !template
                    .preserved_runtime_files
                    .iter()
                    .any(|item| normalized_contract_path(&item.target) == target)
                && template.contract.as_ref().is_none_or(|contract| {
                    contract
                        .files
                        .keys()
                        .all(|path| normalized_contract_path(path) != target)
                }),
            "preserved patch config entered another payload, contract, or action-owned input map"
        );
    }
    let provider_set: BTreeSet<_> = provider_by_component.values().cloned().collect();
    ensure!(
        provider_set
            == PRESERVED_PATCH_CONFIG_PROVIDERS
                .iter()
                .map(|(id, _)| (*id).to_owned())
                .collect(),
        "preserved patch config provider set differs from the two selected providers"
    );
    validate_preserved_patch_config_evidence(
        &template.preserved_patch_configs,
        &provider_by_component,
        &template.target_root,
        &template.preservation,
    )
}

fn file_map_for_contract(
    root: &Path,
    manifest: &InputManifest,
    payload_files: &BTreeMap<String, FileBinding>,
    dll_identity: &Value,
    runtime_ids: &[String],
    xhl_files: &BTreeMap<String, FileBinding>,
) -> Result<(BTreeMap<String, NativeFile>, Vec<MissingRequirement>)> {
    validate_preserved_runtime_files(
        &manifest.preserved_runtime_files,
        &manifest.target_root,
        manifest.selection.vein_settings_path.as_deref(),
        &manifest.preservation,
        payload_files,
    )?;
    let mut paths: BTreeSet<String> = manifest.contract_file_paths.iter().cloned().collect();
    for name in [
        "winmm.dll",
        "shroudforge/shroudforge-runtime.dll",
        "shroudforge/kfc-runtime.dll",
        "shroudforge/config/modloader-config.json",
        "dinput8.dll",
    ] {
        paths.insert(name.into());
    }
    for (name, _) in payload_files {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("mods/")
            && (lower.ends_with(".dll")
                || lower.ends_with(".ini")
                || lower.ends_with("/mod.json")
                || lower.ends_with("/extended.mod.json"))
        {
            paths.insert(name.clone());
        }
    }
    for id in runtime_ids {
        let directory = manifest
            .components
            .iter()
            .find(|component| component.id.eq_ignore_ascii_case(id))
            .and_then(|component| component.package_directory.as_ref())
            .with_context(|| format!("runtime package has no selected directory: {id}"))?;
        paths.insert(format!("mods/{directory}/mod.json"));
        if payload_files.contains_key(&format!("mods/{directory}/extended.mod.json")) {
            paths.insert(format!("mods/{directory}/extended.mod.json"));
        }
    }
    for entry in dll_identity
        .as_array()
        .context("DLL plan identity is not an array")?
    {
        let path = PathBuf::from(
            entry
                .get("path")
                .and_then(Value::as_str)
                .context("DLL identity path missing")?,
        );
        let name = path
            .strip_prefix(&manifest.target_root)
            .context("rebound DLL path is outside selected target root")?
            .to_str()
            .context("non-UTF8 rebound DLL path")?
            .replace('\\', "/");
        if xhl_files.contains_key(&name) {
            continue;
        }
        paths.insert(name.clone());
        if let Some(parent) = Path::new(&name).parent().and_then(Path::to_str) {
            let declaration = format!("{parent}/native-plugin.ini");
            if payload_files.contains_key(&declaration) {
                paths.insert(declaration);
            }
        }
    }
    paths.extend(manifest.greeneye_paths.iter().cloned());
    if let Some(path) = &manifest.selection.vein_settings_path {
        paths.insert(path.clone());
    }
    let mut missing = Vec::new();
    let mut files = BTreeMap::new();
    for name in paths {
        safe_relative(&name)?;
        if let Some(preserved) = manifest
            .preserved_runtime_files
            .iter()
            .find(|item| item.target == name)
        {
            files.insert(
                name,
                NativeFile {
                    bytes: preserved.stamp.bytes,
                    sha256: preserved.stamp.sha256.clone(),
                },
            );
            continue;
        }
        let Some(binding) = payload_files.get(&name) else {
            missing.push(MissingRequirement {
                id: format!("runtime-file:{name}"),
                detail: "required runtime file is not present in the selected E-only payload"
                    .into(),
            });
            continue;
        };
        let actual = stamp(&root.join(name.replace('/', "\\")))?;
        ensure!(
            actual.bytes == binding.bytes && actual.sha256 == binding.sha256,
            "payload runtime binding changed: {name}"
        );
        ensure!(
            files
                .insert(
                    name.clone(),
                    NativeFile {
                        bytes: actual.bytes,
                        sha256: actual.sha256
                    }
                )
                .is_none(),
            "duplicate runtime file binding"
        );
    }
    ensure!(
        files.len() <= MAX_CONTRACT_FILES,
        "runtime contract file map exceeds schema 6 bound"
    );
    for name in &manifest.greeneye_paths {
        ensure!(
            files.contains_key(name),
            "Greeneye route is not bound: {name}"
        );
    }
    Ok((files, missing))
}
fn build_contract(
    payload_root: &Path,
    manifest: &InputManifest,
    request_raw: &[u8],
    receipt_raw: &[u8],
    payload_files: &BTreeMap<String, FileBinding>,
    xhl_plan: &ApprovedPlan,
    xhl_files: &BTreeMap<String, FileBinding>,
) -> Result<(Option<Contract>, Vec<MissingRequirement>)> {
    let (runtime_ids, dll_plan, scripts) =
        runtime_plan(payload_root, manifest, xhl_plan, xhl_files)?;
    let (files, mut missing) = file_map_for_contract(
        payload_root,
        manifest,
        payload_files,
        &dll_plan,
        &runtime_ids,
        xhl_files,
    )?;
    let mut selection = json!({
        "xhl": null,
        "processRole": manifest.selection.process_role,
        "dedicatedGuestQualified": manifest.selection.dedicated_guest_qualified,
        "allowUnverifiedCursorGate": manifest.selection.allow_unverified_cursor_gate,
        "flightTransportEnabled": manifest.selection.flight_transport_enabled,
        "veinSelected": manifest.selection.vein_selected,
        "veinSettingsPath": manifest.selection.vein_settings_path,
    });
    let _ = &mut selection;
    let value = json!({
        "schemaVersion": 3,
        "host": "sf-winmm-selected-services-v3",
        "runtimeSourceClosure": manifest.runtime_source_closure,
        "preparedRequestSha256": digest(request_raw),
        "preparedReceiptSha256": digest(receipt_raw),
        "bootServices": manifest.selection.boot_services,
        "overlay": manifest.selection.overlay,
        "firstPerson": manifest.selection.first_person,
        "selection": selection,
        "runtimeMods": runtime_ids,
        "scripts": scripts,
        "dllPlan": dll_plan,
        "files": files,
        "greeneyePaths": manifest.greeneye_paths,
    });
    let contract: Contract =
        serde_json::from_value(value).context("construct full-runtime schema 3 contract")?;
    let service_plan =
        shroudforge_api::current_assets::full_runtime::selected_services::plan(&contract)?;
    ensure!(
        service_plan.process_role == 1,
        "full-runtime candidate is not client-only"
    );
    let env = ModEnvironment::load(payload_root.to_str().context("non-UTF8 payload path")?)
        .map_err(|error| anyhow::anyhow!("candidate mod environment: {error:?}"))?;
    flight_services_match(&contract, &env)?;
    if manifest.extended_zoom {
        let package = env
            .mod_registry()
            .get("esm-real-map")
            .context("ExtendedZoom selected without HD Map package")?;
        ensure!(
            package.info().enabled,
            "HD Map package must remain enabled for ExtendedZoom"
        );
        let mut package_fs = package.fs();
        let extended = package_fs
            .read_file("extended.mod.json")
            .context("HD Map extended.mod.json missing")?;
        let value: Value = serde_json::from_slice(&read_all(extended, 16 * 1024 * 1024)?)?;
        let configured = value
            .pointer("/settings/ExtendedZoom/value")
            .and_then(Value::as_bool)
            .or_else(|| {
                value
                    .pointer("/settings/ExtendedZoom")
                    .and_then(Value::as_bool)
            })
            .context("HD Map ExtendedZoom setting missing")?;
        ensure!(
            configured,
            "HD Map ExtendedZoom does not preserve the selected true value"
        );
    } else if let Some(package) = env.mod_registry().get("esm-real-map") {
        if let Ok(file) = package.fs().read_file("extended.mod.json") {
            let value: Value = serde_json::from_slice(&read_all(file, 16 * 1024 * 1024)?)?;
            let configured = value
                .pointer("/settings/ExtendedZoom/value")
                .and_then(Value::as_bool)
                .or_else(|| {
                    value
                        .pointer("/settings/ExtendedZoom")
                        .and_then(Value::as_bool)
                })
                .unwrap_or(false);
            ensure!(
                !configured,
                "HD Map ExtendedZoom differs from selected false value"
            );
        }
    }
    if !missing.is_empty() {
        return Ok((None, missing));
    }
    ensure!(
        contract.files.len() >= 5,
        "full-runtime contract lacks required base/native inputs"
    );
    Ok((Some(contract), std::mem::take(&mut missing)))
}
fn read_all(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "package metadata exceeds bound"
    );
    Ok(bytes)
}

fn write_json_new<T: Serialize>(path: &Path, value: &T) -> Result<String> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create evidence file {}", path.display()))?;
    file.write_all(&bytes)?;
    file.flush()?;
    drop(file);
    Ok(digest(&bytes))
}
fn postimages(receipt: &AssetReceipt) -> BTreeMap<String, AssetPostimage> {
    receipt
        .staged
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                AssetPostimage {
                    source: receipt.output_stage.join(name.replace('/', "\\")),
                    stamp: value.clone(),
                },
            )
        })
        .collect()
}
fn normalized_contract_path(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}

/// Validate and write a fresh E-only candidate template. This never reads the target root.
pub fn prepare_template(manifest_bytes: &[u8], expected_manifest_sha256: &str) -> Result<Template> {
    ensure!(
        valid_digest(expected_manifest_sha256)
            && digest(manifest_bytes) == expected_manifest_sha256,
        "input manifest digest differs"
    );
    let manifest: InputManifest =
        serde_json::from_slice(manifest_bytes).context("parse strict candidate input manifest")?;
    check_manifest_evidence(&manifest)?;
    target_root(&manifest.target_root)?;
    validate_e_path(&manifest.output_root, false, true)?;
    ensure!(
        !manifest.output_root.exists(),
        "candidate output root must be fresh; existing output is never overwritten"
    );
    let parent = manifest
        .output_root
        .parent()
        .context("candidate output parent is missing")?;
    validate_e_path(parent, true, true)?;
    let (request_bytes, receipt_bytes, request, receipt) = raw_request_receipt(&manifest)?;
    let plan_bytes = check_raw_evidence(
        &manifest.xhl_plan.path,
        &manifest.xhl_plan.sha256,
        1024 * 1024,
    )?;
    let plan: ApprovedPlan = serde_json::from_slice(&plan_bytes).context("parse XHL plan")?;
    let xhl_files = xhl_target_files(&plan, &plan_bytes, &manifest.target_root)?;
    ensure!(
        manifest.xhl_plan.sha256 == digest(&plan_bytes),
        "XHL plan raw digest differs"
    );
    ensure!(
        manifest.runtime_source_closure.len() == 64,
        "runtime source closure is malformed"
    );
    let sources = collect_staged_sources(&manifest, &request, &receipt, &xhl_files)?;
    let payload_root = manifest.output_root.join("payload");
    fs::create_dir(&manifest.output_root).with_context(|| {
        format!(
            "create candidate output root {}",
            manifest.output_root.display()
        )
    })?;
    ordinary(&manifest.output_root, true)?;
    let payload_files = copy_payload(&payload_root, &sources)?;
    let mut missing = manifest.missing_requirements.clone();
    let greeneye_selected = manifest
        .components
        .iter()
        .any(|component| component.enabled && component.requires_greeneye);
    if greeneye_selected && manifest.greeneye_route.is_none() {
        missing.push(MissingRequirement {
            id: "greeneye-route-evidence".into(),
            detail: "bind the accepted versioned continuation profile, exact host and four scan inputs, and matching runtime/bootstrap artifact identities".into(),
        });
    }
    let contract = if missing.is_empty() {
        let (contract, additional) = build_contract(
            &payload_root,
            &manifest,
            &request_bytes,
            &receipt_bytes,
            &payload_files,
            &plan,
            &xhl_files,
        )?;
        missing.extend(additional);
        if missing.is_empty() { contract } else { None }
    } else {
        None
    };
    if let Some(contract) = &contract {
        ensure!(
            contract.schema_version == 3 && contract.host == "sf-winmm-selected-services-v3",
            "template runtime contract is not schema 3"
        );
    }
    let canonical_runtime_mod_order = contract
        .as_ref()
        .map(|value| value.runtime_mods.clone())
        .unwrap_or_default();
    let template = Template {
        schema_version: 1,
        status: if missing.is_empty() {
            "NONINSTALLABLE_TEMPLATE".into()
        } else {
            "BLOCKED_MISSING_REQUIREMENTS".into()
        },
        candidate_id: manifest.candidate_id.clone(),
        manifest_sha256: expected_manifest_sha256.into(),
        target_root: manifest.target_root.clone(),
        payload_root: payload_root.clone(),
        prepared_producer_closure: receipt.source_closure.clone(),
        runtime_source_closure: manifest.runtime_source_closure.clone(),
        request_json: String::from_utf8(request_bytes.clone())
            .context("prepared request is not UTF-8 JSON")?,
        request_raw_sha256: digest(&request_bytes),
        request_canonical_sha256: receipt.request_sha256.clone(),
        receipt_json: String::from_utf8(receipt_bytes.clone())
            .context("prepared receipt is not UTF-8 JSON")?,
        receipt_raw_sha256: digest(&receipt_bytes),
        receipt_canonical_sha256: digest(&serde_json::to_vec(&receipt)?),
        source_dat_membership_sha256: receipt.source_dat_membership_sha256.clone(),
        prepared_stage: receipt.output_stage.clone(),
        asset_postimages: postimages(&receipt),
        selected_features: manifest.selected_features.clone(),
        selected_components: manifest.components.clone(),
        asset_provider_ids: manifest.asset_provider_ids.clone(),
        asset_request_selected_ids: request.selected_ids.clone(),
        canonical_runtime_mod_order,
        contract,
        contract_file_paths: manifest.contract_file_paths.clone(),
        greeneye_paths: manifest.greeneye_paths.clone(),
        greeneye_route: manifest.greeneye_route.clone(),
        extended_zoom: manifest.extended_zoom,
        missing_requirements: missing.clone(),
        xhl_plan_sha256: manifest.xhl_plan.sha256.clone(),
        xhl_transaction: plan.transaction.clone(),
        xhl_target: manifest.target_root.join("mods/xhl-core"),
        xhl_files,
        preservation: manifest.preservation.clone(),
        preserved_runtime_files: manifest.preserved_runtime_files.clone(),
        preserved_patch_configs: manifest.preserved_patch_configs.clone(),
        payload_files: payload_files.clone(),
    };
    validate_preserved_patch_config_template(&template, &request)?;
    if let Some(contract) = &template.contract {
        validate_greeneye_template(&template, contract)?;
    }
    let review = ReviewManifest {
        schema_version: 1,
        status: template.status.clone(),
        candidate_id: template.candidate_id.clone(),
        installable: false,
        target_root: template.target_root.clone(),
        payload_root: template.payload_root.clone(),
        manifest_sha256: template.manifest_sha256.clone(),
        selected_features: template.selected_features.clone(),
        selected_components: template.selected_components.clone(),
        asset_provider_ids: template.asset_provider_ids.clone(),
        asset_request_selected_ids: template.asset_request_selected_ids.clone(),
        canonical_runtime_mod_order: template.canonical_runtime_mod_order.clone(),
        greeneye_route: template.greeneye_route.clone(),
        missing_requirements: template.missing_requirements.clone(),
        preservation: template.preservation.clone(),
        preserved_runtime_files: template.preserved_runtime_files.clone(),
        preserved_patch_configs: template.preserved_patch_configs.clone(),
        asset_postimages: template.asset_postimages.clone(),
        xhl_plan_sha256: template.xhl_plan_sha256.clone(),
        xhl_transaction: template.xhl_transaction.clone(),
        payload_files: template.payload_files.clone(),
        gates: vec![
            "Deployment must recheck the selected target and all hash-only preservation inputs before any installation transaction.".into(),
            "The XHL prepared receipt must match this exact approved plan, transaction, and target.".into(),
            "Prepared asset postimages remain references to the E: output stage and must be revalidated before deployment.".into(),
            "Asset request selectedIds order is bound exactly to assetProviderIds; canonical runtime order is recorded independently from ModEnvironment::runtime_plan_report_detailed.".into(),
            "A Greeneye continuation profile binds its actual host, four scan inputs, and runtime/bootstrap hashes; it does not assert plugin scan or gameplay readiness.".into(),
            "This assembler did not read the target, install files, change ACLs, or launch processes.".into(),
        ],
    };
    let intent = TransactionIntent {
        schema_version: 1,
        transaction_id: digest(
            format!(
                "{}:{}:{}",
                expected_manifest_sha256, template.request_raw_sha256, template.xhl_plan_sha256
            )
            .as_bytes(),
        ),
        status: "PLANNED_ONLY".into(),
        target_root: template.target_root.clone(),
        asset_request_raw_sha256: template.request_raw_sha256.clone(),
        asset_receipt_raw_sha256: template.receipt_raw_sha256.clone(),
        xhl_approved_plan_sha256: template.xhl_plan_sha256.clone(),
        xhl_transaction: template.xhl_transaction.clone(),
        expected_xhl_target: template.xhl_target.clone(),
        target_freshness_required: true,
        install_performed: false,
    };
    write_json_new(&manifest.output_root.join("review-manifest.json"), &review)?;
    write_json_new(
        &manifest.output_root.join("transaction-intent.json"),
        &intent,
    )?;
    write_json_new(&manifest.output_root.join("template.json"), &template)?;
    Ok(template)
}
fn expected_scan_directories(plan: &ApprovedPlan) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for name in plan.files.keys() {
        let Some(file) = name.strip_prefix("mods/") else {
            continue;
        };
        safe_relative(file)?;
        result.insert(String::new());
        let mut parent = Path::new(file).parent();
        while let Some(path) = parent {
            if path.as_os_str().is_empty() {
                break;
            }
            let relative = path
                .to_str()
                .context("non-UTF8 XHL directory")?
                .replace('\\', "/");
            safe_relative(&relative)?;
            result.insert(relative);
            parent = path.parent();
        }
    }
    Ok(result)
}
fn validate_xhl_receipt(
    plan: &ApprovedPlan,
    receipt: &XhlReceipt,
    plan_sha: &str,
    target: &Path,
) -> Result<()> {
    ensure!(
        receipt.schema_version == 1 && receipt.transaction == plan.transaction,
        "XHL receipt transaction differs"
    );
    ensure!(
        receipt.approved_plan_sha256 == plan_sha,
        "XHL prepared receipt does not bind the approved plan bytes"
    );
    ensure!(
        normalize_absolute(&receipt.target)? == normalize_absolute(&target.join("mods/xhl-core"))?,
        "XHL receipt target differs from selected target"
    );
    ensure!(
        receipt.container.relative.is_empty(),
        "XHL container identity is not the target Core directory"
    );
    let expected_dirs = expected_scan_directories(plan)?;
    let mut actual_dirs = BTreeSet::new();
    let mut identities = BTreeSet::new();
    identities.insert((receipt.container.volume, receipt.container.id));
    for directory in &receipt.directories {
        if !directory.relative.is_empty() {
            safe_relative(&directory.relative)?;
        }
        ensure!(
            actual_dirs.insert(directory.relative.clone()),
            "duplicate XHL scan directory identity"
        );
        ensure!(
            identities.insert((directory.volume, directory.id)),
            "aliased XHL directory identity"
        );
    }
    ensure!(
        actual_dirs == expected_dirs,
        "XHL scan directory IDs differ from the approved plan closure"
    );
    let mut expected_files = BTreeMap::new();
    let mut expected_identity_keys = BTreeSet::new();
    for (name, input) in &plan.files {
        let key = format!("mods/xhl-core/{name}");
        expected_files.insert(key, (input.bytes, input.sha256.clone()));
    }
    ensure!(
        receipt.files.len() == expected_files.len(),
        "XHL receipt file map is incomplete or oversized"
    );
    for (name, expected) in &expected_files {
        let pin = receipt
            .files
            .get(name)
            .with_context(|| format!("XHL receipt lacks {name}"))?;
        ensure!(
            (pin.bytes, pin.sha256.clone()) == *expected,
            "XHL prepared file stamp differs: {name}"
        );
        ensure!(
            expected_identity_keys.insert((pin.identity.volume, pin.identity.id)),
            "XHL receipt aliases two prepared files"
        );
        ensure!(
            identities.insert((pin.identity.volume, pin.identity.id)),
            "XHL prepared file aliases a directory identity"
        );
    }
    Ok(())
}
fn contract_with_xhl(
    template: &Template,
    plan: &ApprovedPlan,
    receipt: &XhlReceipt,
) -> Result<Contract> {
    let mut document = serde_json::to_value(
        template
            .contract
            .as_ref()
            .context("template has no complete runtime contract")?,
    )?;
    ensure!(
        document["schemaVersion"] == 3 && document["host"] == "sf-winmm-selected-services-v3",
        "template runtime contract schema differs"
    );
    ensure!(
        document.pointer("/selection/xhl").is_none()
            || document.pointer("/selection/xhl") == Some(&Value::Null),
        "template unexpectedly contains XHL selection IDs"
    );
    let files = document["files"]
        .as_object_mut()
        .context("runtime file map is not an object")?;
    for (name, pin) in &receipt.files {
        let expected = template
            .xhl_files
            .get(name)
            .context("XHL file is outside the template-approved file map")?;
        ensure!(
            expected.bytes == pin.bytes && expected.sha256 == pin.sha256,
            "XHL prepared file differs from template stamp: {name}"
        );
        ensure!(
            files
                .insert(name.clone(), json!({"bytes":pin.bytes,"sha256":pin.sha256}))
                .is_none(),
            "XHL path conflicts with another runtime binding: {name}"
        );
    }
    ensure!(
        files.len() <= MAX_CONTRACT_FILES,
        "final XHL runtime contract exceeds schema 6 file bound"
    );
    let scan_files: Vec<String> = plan
        .files
        .keys()
        .filter_map(|name| name.strip_prefix("mods/").map(str::to_owned))
        .collect();
    let selection = json!({
        "service": "xhl-core-registration-v1",
        "container": receipt.container,
        "directories": receipt.directories,
        "files": scan_files,
    });
    document["selection"]["xhl"] = selection;
    let contract: Contract = serde_json::from_value(document)
        .context("construct finalized schema 3 runtime contract")?;
    let native = shroudforge_api::current_assets::full_runtime::selected_services::plan(&contract)?;
    ensure!(
        native.process_role == 1,
        "finalized runtime plan is not client-only"
    );
    Ok(contract)
}

/// Bind the exact XHL ApprovedPlan and RB65 readback receipt to a schema 6 policy.
/// This writes only E: candidate evidence and never calls the adapter Install API.
pub fn finalize_policy(
    template_path: &Path,
    expected_template_sha256: &str,
    plan_path: &Path,
    receipt_path: &Path,
    expected_receipt_sha256: &str,
) -> Result<FinalizationReceipt> {
    let template_bytes =
        check_raw_evidence(template_path, expected_template_sha256, 64 * 1024 * 1024)?;
    let template: Template =
        serde_json::from_slice(&template_bytes).context("parse strict candidate template")?;
    ensure!(
        template.schema_version == 1 && template.status == "NONINSTALLABLE_TEMPLATE",
        "candidate template is blocked or unsupported"
    );
    ensure!(
        template.missing_requirements.is_empty() && template.contract.is_some(),
        "candidate template has unresolved required inputs"
    );
    target_root(&template.target_root)?;
    validate_e_path(&template.payload_root, true, true)?;
    ensure!(
        same_path(
            template_path.parent().unwrap_or(Path::new("")),
            template.payload_root.parent().unwrap_or(Path::new(""))
        ),
        "template is not beside its pinned payload root"
    );
    let template_contract = serde_json::to_value(
        template
            .contract
            .as_ref()
            .context("template has no complete runtime contract")?,
    )?;
    let request: AssetRequest =
        serde_json::from_str(&template.request_json).context("template request JSON is invalid")?;
    ensure!(
        digest(template.request_json.as_bytes()) == template.request_raw_sha256,
        "template raw request digest differs"
    );
    ensure!(
        digest(&serde_json::to_vec(&request)?) == template.request_canonical_sha256,
        "template canonical request digest differs"
    );
    validate_preserved_patch_config_template(&template, &request)?;
    validate_preserved_runtime_files(
        &template.preserved_runtime_files,
        &template.target_root,
        template_contract
            .pointer("/selection/veinSettingsPath")
            .and_then(Value::as_str),
        &template.preservation,
        &template.payload_files,
    )?;
    for item in &template.preserved_runtime_files {
        let bound = template
            .contract
            .as_ref()
            .unwrap()
            .files
            .get(&item.target)
            .context("preserved runtime input is absent from the final contract")?;
        ensure!(
            bound.bytes == item.stamp.bytes && bound.sha256 == item.stamp.sha256,
            "preserved runtime contract stamp differs"
        );
    }
    validate_greeneye_template(
        &template,
        template
            .contract
            .as_ref()
            .context("template has no complete runtime contract")?,
    )?;
    ensure!(
        template_contract.pointer("/selection/xhl").is_none()
            || template_contract.pointer("/selection/xhl") == Some(&Value::Null),
        "template contains XHL prepared IDs"
    );
    let plan_bytes = check_raw_evidence(plan_path, &template.xhl_plan_sha256, 1024 * 1024)?;
    let plan: ApprovedPlan =
        serde_json::from_slice(&plan_bytes).context("parse final approved XHL plan")?;
    ensure!(
        plan.transaction == template.xhl_transaction,
        "XHL transaction differs from template"
    );
    let derived_files = xhl_target_files(&plan, &plan_bytes, &template.target_root)?;
    ensure!(
        derived_files == template.xhl_files,
        "approved XHL file map differs from template"
    );
    let receipt_bytes =
        check_raw_evidence(receipt_path, expected_receipt_sha256, 32 * 1024 * 1024)?;
    let receipt: XhlReceipt =
        serde_json::from_slice(&receipt_bytes).context("parse RB65 XHL prepared receipt")?;
    validate_xhl_receipt(
        &plan,
        &receipt,
        &template.xhl_plan_sha256,
        &template.target_root,
    )?;
    let mut contract = contract_with_xhl(&template, &plan, &receipt)?;
    for entry in &template.preserved_patch_configs {
        ensure!(
            contract
                .files
                .keys()
                .all(|path| !path.eq_ignore_ascii_case(&entry.target)),
            "preserved patch config must stay outside the runtime contract"
        );
    }
    ensure!(
        contract.files.contains_key("dinput8.dll"),
        "matching ShroudForge DirectInput host is not runtime-bound"
    );
    ensure!(
        contract.files.contains_key(XHL_CORE_PATH) && contract.files.contains_key(XHL_CHAIN_PATH),
        "XHL Core or disabled chain is not runtime-bound"
    );
    // Re-parse immutable generation evidence; keep raw and typed digests distinct.
    let asset_receipt: AssetReceipt = serde_json::from_str(&template.receipt_json)
        .context("template asset receipt JSON is invalid")?;
    ensure!(
        asset_receipt.request_sha256 == template.request_canonical_sha256,
        "prepared asset receipt binds another canonical request"
    );
    ensure!(
        digest(template.receipt_json.as_bytes()) == template.receipt_raw_sha256,
        "template raw asset receipt digest differs"
    );
    ensure!(
        digest(&serde_json::to_vec(&asset_receipt)?) == template.receipt_canonical_sha256,
        "template canonical asset receipt digest differs"
    );
    ensure!(
        asset_receipt.source_dat_membership_sha256 == template.source_dat_membership_sha256,
        "prepared DAT membership binding differs"
    );
    let policy = PreparedPolicy {
        schema_version: 6,
        owner: "shroudforge-full-prepared".into(),
        root: template.target_root.clone(),
        source_closure: template.prepared_producer_closure.clone(),
        request_json: template.request_json.clone(),
        request_sha256: template.request_raw_sha256.clone(),
        receipt_json: template.receipt_json.clone(),
        receipt_sha256: template.receipt_raw_sha256.clone(),
        extended_zoom: template.extended_zoom,
        preserved_loaders: BTreeMap::new(),
        runtime: contract,
    };
    let policy_path = template
        .payload_root
        .join("shroudforge/config/current-assets.json");
    let policy_hash = write_json_new(&policy_path, &policy)?;
    let finalization = FinalizationReceipt {
        schema_version: 1,
        status: "FINALIZED_POLICY_NONINSTALLABLE_UNTIL_DEPLOYMENT".into(),
        candidate_id: template.candidate_id.clone(),
        template_sha256: expected_template_sha256.into(),
        approved_plan_sha256: template.xhl_plan_sha256.clone(),
        xhl_prepared_receipt_raw_sha256: expected_receipt_sha256.into(),
        xhl_prepared_receipt_canonical_sha256: receipt.digest(),
        xhl_transaction: receipt.transaction.clone(),
        target_root: template.target_root.clone(),
        policy_sha256: policy_hash,
        deployment_target_recheck_required: true,
        install_performed: false,
    };
    write_json_new(
        &template_path
            .parent()
            .context("template output parent missing")?
            .join("finalization.json"),
        &finalization,
    )?;
    Ok(finalization)
}

#[cfg(test)]
mod tests;
