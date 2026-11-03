//! Explicit private diagnostic selection, never installed-package discovery.
use super::early_probe_contract as contract;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

pub(super) const ENVIRONMENT: &str = "SHROUDFORGE_EARLY_OWNER_PROBE_ADMISSION";
const SOURCE_SHA256: &str = "4b6e581c8af041dc1b9bbfba5eed6aeb96d40399f92f0fe0e47e8d7f8152e0c3";
const PAYLOAD_SHA256: &str = "1c92c5193acea574cb94ab35c198b73a3745ddc72aff1520aeb026af377944f7";
// Immutable rejected predecessor. Changing a selection/receipt cannot promote it.
const REJECTED_DLL_SHA256: &str =
    "73e372841dcd5ed763bacec7d6561a10d69cda5eb407646130844bcad2c4be67";
const MAX_ADMISSION: u64 = 16 * 1024;
const MAX_REVIEW: u64 = 256 * 1024;
const MAX_DLL: u64 = 16 * 1024 * 1024;

struct StrictJson(Value);
impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = StrictJson;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("JSON without duplicate fields")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictJson(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(StrictJson(value.into()))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(StrictJson(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|n| StrictJson(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StrictJson(value.into()))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(StrictJson(value.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictJson(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = input.next_element::<StrictJson>()? {
                    values.push(value.0);
                }
                Ok(StrictJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = input.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("duplicate diagnostic field"));
                    }
                    values.insert(key, input.next_value::<StrictJson>()?.0);
                }
                Ok(StrictJson(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}
fn parse_json(bytes: &[u8]) -> Result<Value, String> {
    let mut input = serde_json::Deserializer::from_slice(bytes);
    let value = StrictJson::deserialize(&mut input)
        .map_err(|e| e.to_string())?
        .0;
    input.end().map_err(|e| e.to_string())?;
    Ok(value)
}

#[derive(Debug)]
pub(super) struct Selection {
    pub dll: PathBuf,
    pub profile: PathBuf,
    pub review: PathBuf,
    pub report: PathBuf,
    pub review_sha256: String,
}

fn exact_keys(value: &Value, keys: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("diagnostic document must be an object")?;
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err("unknown diagnostic field".into());
    }
    Ok(())
}
fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .ok_or_else(|| format!("missing diagnostic string: {field}"))
}
fn absolute(value: &Value, field: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(text(value, field)?);
    if !path.is_absolute() {
        return Err(format!("{field} must be an explicit absolute path"));
    }
    Ok(path)
}
fn digest_text(value: &Value, field: &str) -> Result<String, String> {
    let digest = text(value, field)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!("invalid lowercase SHA-256: {field}"));
    }
    Ok(digest.into())
}

pub(super) fn parse_selection(bytes: &[u8]) -> Result<Option<Selection>, String> {
    if bytes.len() as u64 > MAX_ADMISSION {
        return Err("diagnostic selection exceeds limit".into());
    }
    let value = parse_json(bytes)?;
    exact_keys(
        &value,
        &[
            "schemaVersion",
            "kind",
            "enabled",
            "packageId",
            "dll",
            "profile",
            "dllSha256",
            "profileSha256",
            "abiSha256",
            "executableSha256",
            "sourceManifestSha256",
            "payloadManifestSha256",
            "reviewReceiptPath",
            "reviewReceiptSha256",
            "reviewReportPath",
        ],
    )?;
    if value["schemaVersion"].as_u64() != Some(1) || text(&value, "kind")? != contract::KIND {
        return Err("unsupported diagnostic selection schema/kind".into());
    }
    match value["enabled"].as_bool() {
        Some(false) => return Ok(None),
        Some(true) => {}
        None => return Err("explicit boolean enabled is required".into()),
    }
    for (field, expected) in [
        ("packageId", contract::PACKAGE_ID),
        ("dllSha256", contract::DLL_SHA256),
        ("profileSha256", contract::PROFILE_SHA256),
        ("abiSha256", contract::ABI_SHA256),
        ("executableSha256", contract::EXE_SHA256),
        ("sourceManifestSha256", SOURCE_SHA256),
        ("payloadManifestSha256", PAYLOAD_SHA256),
    ] {
        if text(&value, field)? != expected {
            return Err(format!("unadmitted diagnostic identity: {field}"));
        }
    }
    Ok(Some(Selection {
        dll: absolute(&value, "dll")?,
        profile: absolute(&value, "profile")?,
        review: absolute(&value, "reviewReceiptPath")?,
        report: absolute(&value, "reviewReportPath")?,
        review_sha256: digest_text(&value, "reviewReceiptSha256")?,
    }))
}

pub(super) fn verify_review(bytes: &[u8], report: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_REVIEW {
        return Err("review receipt exceeds limit".into());
    }
    let value = parse_json(bytes)?;
    exact_keys(
        &value,
        &[
            "schemaVersion",
            "taskId",
            "verdict",
            "scope",
            "sourceManifestSha256",
            "payloadManifestSha256",
            "dllSha256",
            "profileSha256",
            "abiSha256",
            "reportSha256",
            "independentChecks",
            "liveExecutionAuthorized",
            "transferCapability",
        ],
    )?;
    if value["schemaVersion"].as_u64() != Some(1)
        || text(&value, "taskId")? != "AL120-10"
        || text(&value, "verdict")? != "PASS"
        || text(&value, "scope")? != "offline-probe-abi"
        || value["liveExecutionAuthorized"].as_bool() != Some(false)
        || value["transferCapability"].as_u64() != Some(0)
    {
        return Err("independent offline review is not an exact observation-only PASS".into());
    }
    for (field, expected) in [
        ("sourceManifestSha256", SOURCE_SHA256),
        ("payloadManifestSha256", PAYLOAD_SHA256),
        ("dllSha256", contract::DLL_SHA256),
        ("profileSha256", contract::PROFILE_SHA256),
        ("abiSha256", contract::ABI_SHA256),
    ] {
        if text(&value, field)? != expected {
            return Err(format!("review candidate mismatch: {field}"));
        }
    }
    let checks = &value["independentChecks"];
    exact_keys(checks, &["executed", "result", "limits"])?;
    let limits = checks["limits"]
        .as_array()
        .ok_or("review limits must be explicit")?;
    if checks["executed"].as_bool() != Some(true)
        || text(checks, "result")? != "PASS"
        || limits.is_empty()
        || limits
            .iter()
            .any(|item| item.as_str().is_none_or(str::is_empty))
    {
        return Err("review execution/result/limits are incomplete".into());
    }
    if digest_text(&value, "reportSha256")? != hex_digest(report) {
        return Err("independent report identity mismatch".into());
    }
    if contract::DLL_SHA256 == REJECTED_DLL_SHA256 {
        return Err("the curated predecessor failed AL120-10; explicit successor pins and review are required".into());
    }
    Ok(())
}

pub(super) fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) struct HeldFile {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    _file: File,
    #[cfg(windows)]
    identity: (u32, u64),
}
impl HeldFile {
    pub fn same_identity(&self, other: &Self) -> bool {
        #[cfg(windows)]
        {
            self.identity == other.identity
        }
        #[cfg(not(windows))]
        {
            self.path == other.path
        }
    }
}
#[cfg(windows)]
#[repr(C)]
struct ByHandleFileInformation {
    attributes:u32,creation:[u32;2],access:[u32;2],write:[u32;2],volume:u32,
    size_high:u32,size_low:u32,links:u32,index_high:u32,index_low:u32,
}
#[cfg(windows)]
fn file_identity(file:&File)->Result<(u32,u64),String>{
    use std::os::windows::io::AsRawHandle;
    #[link(name="kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle:*mut std::ffi::c_void,output:*mut ByHandleFileInformation)->i32;
    }
    let mut info=std::mem::MaybeUninit::<ByHandleFileInformation>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(),info.as_mut_ptr()) }==0 {
        return Err("diagnostic file handle identity unavailable".into());
    }
    let info=unsafe { info.assume_init() };
    Ok((info.volume,((info.index_high as u64)<<32)|info.index_low as u64))
}
pub(super) fn hold_file(path: &Path, limit: u64) -> Result<HeldFile, String> {
    if !path.is_absolute() {
        return Err("diagnostic path must be absolute".into());
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(path.components().next(),Some(Component::Prefix(prefix))
            if matches!(prefix.kind(),Prefix::Disk(_)|Prefix::VerbatimDisk(_)))
        {
            return Err("diagnostic artifacts must use a local drive path".into());
        }
    }
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|error| error.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err("diagnostic reparse paths are refused".into());
            }
        }
        if metadata.file_type().is_symlink() {
            return Err("diagnostic symlink paths are refused".into());
        }
    }
    let canonical = path.canonicalize().map_err(|error| error.to_string())?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        options.share_mode(FILE_SHARE_READ);
    }
    let mut file = options
        .open(&canonical)
        .map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    #[cfg(windows)]
    let identity=file_identity(&file)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err("diagnostic file type/size refused".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 != metadata.len() {
        return Err("diagnostic file changed while reading".into());
    }
    Ok(HeldFile {
        path: canonical,
        bytes,
        _file: file,
        #[cfg(windows)]
        identity,
    })
}

pub(super) struct ValidatedFiles {
    pub _admission: HeldFile,
    pub admission_sha256: String,
    pub dll: HeldFile,
    pub _profile: HeldFile,
    pub _review: HeldFile,
    pub _report: HeldFile,
}
pub(super) fn read_selected(path: &Path) -> Result<Option<ValidatedFiles>, String> {
    let admission = hold_file(path, MAX_ADMISSION)?;
    let admission_sha256 = hex_digest(&admission.bytes);
    let Some(selection) = parse_selection(&admission.bytes)? else {
        return Ok(None);
    };
    // Review rejection precedes DLL reading/mapping and any generation grant.
    let review = hold_file(&selection.review, MAX_REVIEW)?;
    if hex_digest(&review.bytes) != selection.review_sha256 {
        return Err("selected review receipt hash mismatch".into());
    }
    let report = hold_file(&selection.report, MAX_REVIEW)?;
    verify_review(&review.bytes, &report.bytes)?;
    let profile = hold_file(&selection.profile, MAX_REVIEW)?;
    if hex_digest(&profile.bytes) != contract::PROFILE_SHA256 {
        return Err("diagnostic profile hash mismatch".into());
    }
    let dll = hold_file(&selection.dll, MAX_DLL)?;
    if hex_digest(&dll.bytes) != contract::DLL_SHA256 {
        return Err("diagnostic DLL hash mismatch".into());
    }
    Ok(Some(ValidatedFiles {
        _admission: admission,
        admission_sha256,
        dll,
        _profile: profile,
        _review: review,
        _report: report,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_schema_binds_only_curated_absolute_diagnostic_inputs() {
        let value = serde_json::json!({"schemaVersion":1,"kind":contract::KIND,"enabled":true,
            "packageId":contract::PACKAGE_ID,"dll":"E:/fixture/probe.dll","profile":"E:/fixture/profile.json",
            "dllSha256":contract::DLL_SHA256,"profileSha256":contract::PROFILE_SHA256,
            "abiSha256":contract::ABI_SHA256,"executableSha256":contract::EXE_SHA256,
            "sourceManifestSha256":SOURCE_SHA256,"payloadManifestSha256":PAYLOAD_SHA256,
            "reviewReceiptPath":"E:/fixture/VERDICT.json","reviewReceiptSha256":"a".repeat(64),
            "reviewReportPath":"E:/fixture/REPORT.md"});
        assert!(
            parse_selection(&serde_json::to_vec(&value).unwrap())
                .unwrap()
                .is_some()
        );
        for field in [
            "packageId",
            "dllSha256",
            "profileSha256",
            "abiSha256",
            "executableSha256",
            "sourceManifestSha256",
            "payloadManifestSha256",
        ] {
            let mut changed = value.clone();
            changed[field] = "unadmitted".into();
            assert!(
                parse_selection(&serde_json::to_vec(&changed).unwrap()).is_err(),
                "{field}"
            );
        }
        for field in ["dll", "profile", "reviewReceiptPath", "reviewReportPath"] {
            let mut changed = value.clone();
            changed[field] = "relative/file".into();
            assert!(
                parse_selection(&serde_json::to_vec(&changed).unwrap()).is_err(),
                "{field}"
            );
        }
        let mut changed = value;
        changed["reviewReceiptSha256"] = "A".repeat(64).into();
        assert!(parse_selection(&serde_json::to_vec(&changed).unwrap()).is_err());
        assert!(hold_file(Path::new(r"\\unadmitted-host\share\probe.dll"), 1).is_err());
    }
    #[test]
    fn explicit_disabled_record_never_requires_artifact_paths() {
        assert!(
            parse_selection(
                br#"{"schemaVersion":1,"kind":"autoloot-owner-probe-v1","enabled":false}"#
            )
            .unwrap()
            .is_none()
        );
        for bytes in [br#"{"schemaVersion":1,"kind":"autoloot-owner-probe-v1"}"#.as_slice(),
            br#"{"schemaVersion":1,"kind":"autoloot-owner-probe-v1","enabled":false,"surprise":true}"#.as_slice(),
            br#"{"schemaVersion":2,"kind":"autoloot-owner-probe-v1","enabled":false}"#.as_slice()] { assert!(parse_selection(bytes).is_err()); }
        assert!(parse_selection(br#"{"schemaVersion":1,"kind":"autoloot-owner-probe-v1","enabled":true,"enabled":false}"#).is_err());
    }
    #[test]
    fn actual_failed_predecessor_cannot_be_promoted_by_selection() {
        // Synthetic data in the actual flat AL120-10 receipt schema. The real
        // failed receipt is additionally checked by the task-owned harness.
        let report = b"synthetic offline review report";
        let mut altered = serde_json::json!({"schemaVersion":1,"taskId":"AL120-10","verdict":"FAIL",
            "scope":"offline-probe-abi","sourceManifestSha256":SOURCE_SHA256,"payloadManifestSha256":PAYLOAD_SHA256,
            "dllSha256":contract::DLL_SHA256,"profileSha256":contract::PROFILE_SHA256,"abiSha256":contract::ABI_SHA256,
            "reportSha256":hex_digest(report),"independentChecks":{"executed":true,"result":"FAIL","limits":["synthetic only"]},
            "liveExecutionAuthorized":false,"transferCapability":0});
        assert!(verify_review(&serde_json::to_vec(&altered).unwrap(), report).is_err());
        altered["verdict"] = "PASS".into();
        altered["independentChecks"]["result"] = "PASS".into();
        assert!(verify_review(&serde_json::to_vec(&altered).unwrap(), report).is_err());
        altered["liveExecutionAuthorized"] = true.into();
        assert!(verify_review(&serde_json::to_vec(&altered).unwrap(), report).is_err());
    }
}
