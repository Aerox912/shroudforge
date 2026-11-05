//! Operation controls live in one private namespace, never in the game root.
use crate::prepare::{PRIVATE, Request, directory_chain, write_new};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    schema: u32,
    status: String,
    selected_unix_ms: u64,
    expires_unix_ms: u64,
    request: Pin,
    source_closure: Pin,
    source_review: Pin,
    operation_review: Pin,
    artifact_bytes: u64,
    artifact_sha256: String,
}
fn digest(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

struct Controls {
    files: Vec<File>,
    names: BTreeSet<String>,
}
impl Controls {
    fn read(&mut self, pin: &Pin) -> Result<Vec<u8>> {
        ensure!(
            pin.bytes > 0 && pin.bytes <= 2 * 1024 * 1024 && valid_hash(&pin.sha256),
            "bounded pinned control required"
        );
        ensure!(
            matches!(
                pin.name.as_str(),
                "SELECTION.json"
                    | "REQUEST.json"
                    | "SOURCE-CLOSURE.json"
                    | "SOURCE-REVIEW.json"
                    | "OPERATION-REVIEW.json"
            ) && self.names.insert(pin.name.clone()),
            "unknown or repeated control"
        );
        let path = Path::new(PRIVATE).join("controls").join(&pin.name);
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        let mut file = options.open(&path)?;
        let metadata = file.metadata()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(metadata.file_attributes() & 0x400 == 0, "reparse control");
        }
        ensure!(
            metadata.is_file() && metadata.len() == pin.bytes,
            "control extent"
        );
        let mut raw = vec![0; pin.bytes as usize];
        file.read_exact(&mut raw)?;
        ensure!(digest(&raw) == pin.sha256, "control hash");
        self.files.push(file);
        Ok(raw)
    }
}
fn check_time(s: &Selection) -> Result<()> {
    let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    check_window(s.selected_unix_ms, s.expires_unix_ms, now)
}
fn check_window(selected: u64, expires: u64, now: u64) -> Result<()> {
    ensure!(
        selected <= now
            && now < expires
            && expires.checked_sub(selected).is_some_and(|t| t <= 900000),
        "selection time window"
    );
    Ok(())
}

/// Retains all control leases through result serialization. This is a new exact
/// operation contract, not a reinterpretation of any client receipt.
pub struct Approved {
    pub request: Request,
    _controls: Controls,
    _directories: Vec<File>,
    _artifact: File,
    pub selection_sha256: String,
}
pub fn admit(selection_bytes: u64, selection_sha256: &str) -> Result<Approved> {
    ensure!(selection_bytes <= 65536, "selection bound");
    let mut directories = directory_chain(&Path::new(PRIVATE).join("controls"))?;
    directories.extend(directory_chain(&Path::new(PRIVATE).join("operation-r1"))?);
    let mut controls = Controls {
        files: Vec::new(),
        names: BTreeSet::new(),
    };
    let raw = controls.read(&Pin {
        name: "SELECTION.json".into(),
        bytes: selection_bytes,
        sha256: selection_sha256.into(),
    })?;
    let selection: Selection = serde_json::from_slice(&raw)?;
    ensure!(
        selection.schema == 1 && selection.status == "SELECTED_IKEA_SERVER_ASSET_PREPARATION_ONCE",
        "selection kind"
    );
    check_time(&selection)?;
    ensure!(
        selection.request.name == "REQUEST.json"
            && selection.source_closure.name == "SOURCE-CLOSURE.json"
            && selection.source_review.name == "SOURCE-REVIEW.json"
            && selection.operation_review.name == "OPERATION-REVIEW.json",
        "control roles"
    );
    let request_raw = controls.read(&selection.request)?;
    let request: Request = serde_json::from_slice(&request_raw)?;
    let closure = controls.read(&selection.source_closure)?;
    let compiled = option_env!("IKEA_SERVER_PRODUCER_SOURCE_CLOSURE")
        .context("producer closure not compiled")?;
    ensure!(
        valid_hash(compiled) && digest(&closure) == compiled,
        "producer source closure differs"
    );
    let source: Value = serde_json::from_slice(&controls.read(&selection.source_review)?)?;
    let operation: Value = serde_json::from_slice(&controls.read(&selection.operation_review)?)?;
    ensure!(
        source["status"] == "CANONICAL_ACCEPTED_IKEA_SERVER_PREPARED_PRODUCER_SOURCE_ARTIFACT_ONLY"
            && source["sourceClosureSha256"] == compiled
            && source["artifactSha256"] == selection.artifact_sha256
            && source["artifactBytes"] == selection.artifact_bytes,
        "producer review differs"
    );
    ensure!(
        operation["status"] == "CANONICAL_ACCEPTED_IKEA_SERVER_PREPARED_OPERATION_ONLY"
            && operation["requestSha256"] == selection.request.sha256
            && operation["sourceClosureSha256"] == compiled
            && operation["artifactSha256"] == selection.artifact_sha256
            && operation["sourceReviewSha256"] == selection.source_review.sha256
            && operation["noBackups"] == true
            && operation["liveChangesAllowed"] == false,
        "operation review differs"
    );
    ensure!(
        valid_hash(&selection.artifact_sha256) && selection.artifact_bytes <= 64 * 1024 * 1024,
        "artifact extent"
    );
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let mut artifact = options.open(std::env::current_exe()?)?;
    ensure!(
        artifact.metadata()?.len() == selection.artifact_bytes,
        "artifact length"
    );
    let mut h = Sha256::new();
    let mut remaining = selection.artifact_bytes;
    let mut block = [0; 65536];
    while remaining > 0 {
        let n = remaining.min(block.len() as u64) as usize;
        artifact.read_exact(&mut block[..n])?;
        h.update(&block[..n]);
        remaining -= n as u64;
    }
    ensure!(
        format!("{:x}", h.finalize()) == selection.artifact_sha256,
        "artifact hash"
    );
    artifact.seek(SeekFrom::Start(0))?;
    check_time(&selection)?;
    write_new(
        &Path::new(PRIVATE).join("operation-r1/CONSUMED.json"),
        &json!({"status":"SERVER_ASSET_PREPARATION_SELECTED_ATTEMPT_SPENT",
        "selectionSha256":selection_sha256,"requestSha256":selection.request.sha256,"sourceClosure":compiled,
        "artifactSha256":selection.artifact_sha256,"replayAllowed":false,"liveChangesAllowed":false}),
    )?;
    Ok(Approved {
        request,
        _controls: controls,
        _directories: directories,
        _artifact: artifact,
        selection_sha256: selection_sha256.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_expiry_is_exclusive_and_duration_bounded() {
        assert!(check_window(100, 900100, 100).is_ok());
        assert!(check_window(100, 900100, 900099).is_ok());
        for (a, b, n) in [
            (100, 900100, 900100),
            (100, 900101, 101),
            (100, 99, 100),
            (100, 100, 100),
            (100, 900100, 99),
        ] {
            assert!(check_window(a, b, n).is_err());
        }
    }
    #[test]
    fn unknown_control_names_refuse_before_filesystem_access() {
        let mut controls = Controls {
            files: Vec::new(),
            names: BTreeSet::new(),
        };
        for name in [
            "../enshrouded_server.exe",
            "enshrouded_server.exe",
            "C:/secret",
            "SOURCE-REVIEW.json:stream",
        ] {
            assert!(
                controls
                    .read(&Pin {
                        name: name.into(),
                        bytes: 1,
                        sha256: "a".repeat(64)
                    })
                    .is_err()
            );
        }
        assert!(controls.files.is_empty());
    }
}
