//! Source-only phase of private prepared admission. This is not a producer,
//! operation approval, current-asset admission, installer or publication API.
//! A future distinct prepared request must bind this exact plan and artifacts.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shroudforge_package::{
    PreparedManifestPolicy,
    prepared_package_view::{
        PreparedPackageEntry, PreparedPackageView, PreparedReadBudget, PreparedSource,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

mod held_identity;
use held_identity::{Identity, PinnedDirectory};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replacement {
    pub logical_path: String,
    pub raw: Stamp,
    pub path: PathBuf,
    pub replacement: Stamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfilePolicy {
    RejectAmbientProfiles,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePlan {
    pub schema: u32,
    pub kind: String,
    pub environment_root: PathBuf,
    /// Complete raw file membership, including every admitted control file.
    pub raw_files: BTreeMap<String, Stamp>,
    /// Captured traversal order, directories included and empty root first.
    pub entries: Vec<String>,
    pub package_roots: Vec<String>,
    pub replacements: Vec<Replacement>,
    pub profile_policy: ProfilePolicy,
    pub allow_unknown_manifest_fields: bool,
    pub raw_inventory_sha256: String,
    pub effective_inventory_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
struct InventoryRow<'a> {
    ordinal: usize,
    path: &'a str,
    directory: bool,
    origin: String,
    stamp: Option<&'a Stamp>,
}

fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn relative(value: &str, root: bool) -> Result<()> {
    if value.is_empty() && root {
        return Ok(());
    }
    ensure!(
        !value.is_empty() && value.is_ascii() && !value.contains(['\\', ':']),
        "noncanonical relative name"
    );
    for part in value.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with([' ', '.'])
                && !part
                    .chars()
                    .any(|c| c.is_control() || "<>\"|?*".contains(c)),
            "unsafe component"
        );
        ensure!(
            !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "device alias"
        );
    }
    Ok(())
}

impl SourcePlan {
    /// Pure structural checks. This does not grant operation authority.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1 && self.kind == "prepared_effective_source_plan_v1",
            "wrong source plan"
        );
        ensure!(
            self.entries.first().is_some_and(String::is_empty)
                && self.entries.len() <= 20_000
                && !self.raw_files.is_empty()
                && self.raw_files.len() <= 10_000,
            "source namespace bound"
        );
        ensure!(
            !self.package_roots.is_empty()
                && self.package_roots.len() <= 256
                && self.replacements.len() <= 16,
            "source declaration bound"
        );
        let mut seen = BTreeSet::new();
        let mut prior = BTreeSet::new();
        for (i, name) in self.entries.iter().enumerate() {
            relative(name, i == 0)?;
            ensure!(seen.insert(name.to_ascii_lowercase()), "namespace alias");
            if i != 0 {
                let parent = name.rsplit_once('/').map_or("", |(parent, _)| parent);
                ensure!(
                    prior.contains(parent) && !self.raw_files.contains_key(parent),
                    "source parent missing/not directory"
                );
            }
            prior.insert(name.as_str());
        }
        for (name, stamp) in &self.raw_files {
            relative(name, false)?;
            ensure!(
                !name.ends_with(".lua") || stamp.bytes <= 8 * 1024 * 1024,
                "prepared Lua source exceeds8MiB"
            );
            ensure!(
                prior.contains(name.as_str()) && valid_digest(&stamp.sha256),
                "raw member not admitted"
            );
        }
        let mut packages = BTreeSet::new();
        for package in &self.package_roots {
            relative(package, false)?;
            ensure!(
                package.starts_with("mods/")
                    && package.split('/').count() == 2
                    && prior.contains(package.as_str())
                    && !self.raw_files.contains_key(package),
                "package root"
            );
            ensure!(
                packages.insert(package.to_ascii_lowercase())
                    && self.raw_files.contains_key(&format!("{package}/mod.json")),
                "package alias/manifest"
            );
        }
        for name in self
            .raw_files
            .keys()
            .filter(|name| name.starts_with("mods/"))
        {
            ensure!(
                self.package_roots
                    .iter()
                    .any(|root| name.starts_with(&format!("{root}/"))),
                "unselected package member"
            );
        }
        let mut replaced = BTreeSet::new();
        for replacement in &self.replacements {
            relative(&replacement.logical_path, false)?;
            ensure!(
                !replacement.logical_path.ends_with(".lua")
                    || replacement.replacement.bytes <= 8 * 1024 * 1024,
                "prepared Lua override exceeds8MiB"
            );
            ensure!(
                replacement.logical_path.starts_with("mods/")
                    && self.raw_files.get(&replacement.logical_path) == Some(&replacement.raw),
                "replacement raw origin"
            );
            ensure!(
                replaced.insert(&replacement.logical_path)
                    && valid_digest(&replacement.replacement.sha256),
                "replacement identity"
            );
        }
        ensure!(
            valid_digest(&self.raw_inventory_sha256)
                && valid_digest(&self.effective_inventory_sha256),
            "inventory digest syntax"
        );
        ensure!(
            self.inventory_digest(false)? == self.raw_inventory_sha256
                && self.inventory_digest(true)? == self.effective_inventory_sha256,
            "inventory seal"
        );
        Ok(())
    }
    pub fn inventory_digest(&self, effective: bool) -> Result<String> {
        let rows = self
            .entries
            .iter()
            .enumerate()
            .map(|(ordinal, path)| {
                let replacement = if effective {
                    self.replacements.iter().find(|r| r.logical_path == *path)
                } else {
                    None
                };
                let stamp = replacement
                    .map(|r| &r.replacement)
                    .or_else(|| self.raw_files.get(path));
                InventoryRow {
                    ordinal,
                    path,
                    directory: stamp.is_none(),
                    origin: replacement
                        .map(|r| format!("override:{}", r.path.to_string_lossy()))
                        .unwrap_or_else(|| format!("base:{path}")),
                    stamp,
                }
            })
            .collect::<Vec<_>>();
        digest(&(
            "prepared_source_inventory_v1",
            self.environment_root.to_str().context("nonUTF8 root")?,
            rows,
        ))
    }
    pub fn required_final_bytes(&self) -> Result<u64> {
        self.raw_files
            .values()
            .chain(self.replacements.iter().map(|r| &r.replacement))
            .try_fold(0u64, |sum, s| {
                sum.checked_add(s.bytes).context("source byte sum overflow")
            })
    }
    pub fn required_final_calls(&self) -> Result<u64> {
        self.raw_files
            .values()
            .chain(self.replacements.iter().map(|r| &r.replacement))
            .try_fold(0u64, |sum, s| {
                sum.checked_add(s.bytes / 65536 + u64::from(s.bytes % 65536 != 0))
                    .context("source call sum overflow")
            })
    }
}

fn inventory(root: &Path, limit: usize) -> Result<Vec<String>> {
    fn visit(
        root: &Path,
        path: &Path,
        out: &mut Vec<String>,
        limit: usize,
        depth: usize,
    ) -> Result<()> {
        ensure!(depth <= 64, "source directory depth");
        for entry in fs::read_dir(path)? {
            ensure!(out.len() < limit, "source membership cap");
            let path = entry?.path();
            let meta = fs::symlink_metadata(&path)?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(meta.file_attributes() & 0x400 == 0, "reparse source member");
            }
            ensure!(
                !meta.file_type().is_symlink() && (meta.is_dir() || meta.is_file()),
                "nonordinary source member"
            );
            let name = path
                .strip_prefix(root)?
                .to_str()
                .context("nonUTF8 source name")?
                .replace('\\', "/");
            relative(&name, false)?;
            out.push(name);
            if meta.is_dir() {
                visit(root, &path, out, limit, depth + 1)?;
            }
        }
        Ok(())
    }
    let mut out = vec![String::new()];
    visit(root, root, &mut out, limit, 0)?;
    Ok(out)
}

struct Held {
    source: PreparedSource,
    file: Arc<File>,
    identity: Identity,
    path: PathBuf,
    stamp: Stamp,
}
impl Held {
    fn open(path: &Path, expected: &Stamp) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        let file = Arc::new(options.open(path)?);
        let identity = held_identity::snapshot(&file, path, false)?;
        ensure!(identity.bytes == expected.bytes, "held source length");
        let source = PreparedSource::held(file.clone(), expected.bytes)?;
        Ok(Self {
            source,
            file,
            identity,
            path: path.to_owned(),
            stamp: expected.clone(),
        })
    }
    fn verify(&self, budget: PreparedReadBudget, finalizing: bool) -> Result<()> {
        ensure!(
            held_identity::snapshot(&self.file, &self.path, false)? == self.identity,
            "held source identity drift"
        );
        let mut reader = self.source.reader(budget, finalizing);
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        let mut remaining = self.stamp.bytes;
        while remaining != 0 {
            let n = remaining.min(65536) as usize;
            reader.read_exact(&mut buffer[..n])?;
            hash.update(&buffer[..n]);
            remaining -= n as u64;
        }
        ensure!(
            format!("{:x}", hash.finalize()) == self.stamp.sha256,
            "held source hash"
        );
        ensure!(
            held_identity::snapshot(&self.file, &self.path, false)? == self.identity,
            "held source identity drift"
        );
        Ok(())
    }
}

pub struct SourceLease {
    plan: SourcePlan,
    raw: BTreeMap<String, Held>,
    overrides: BTreeMap<String, Held>,
    directories: Vec<PinnedDirectory>,
    budget: PreparedReadBudget,
    finalized: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadLimits {
    pub max_bytes: u64,
    pub max_calls: u64,
    pub final_reserve_bytes: u64,
    pub final_reserve_calls: u64,
}

#[derive(Debug, Serialize)]
pub struct FileFinalization {
    pub name: String,
    pub identity_matched: Option<bool>,
    pub hash_matched: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SourceFinalization {
    pub verified: bool,
    pub files: Vec<FileFinalization>,
    pub directories_matched: bool,
    pub membership_matched: Option<bool>,
    pub phase_error: Option<String>,
    pub metadata_errors: Vec<String>,
    pub charged_bytes: u64,
    pub returned_bytes: u64,
    pub calls: u64,
    pub terminal_io: bool,
}

pub struct SourceAdmissionFailure {
    pub primary: String,
    pub finalization: SourceFinalization,
    // Keep every eligible handle alive until the caller records the failure.
    // Drop never performs IO and does not retry a failed admission.
    _lease: SourceLease,
}
impl std::fmt::Debug for SourceAdmissionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceAdmissionFailure")
            .field("primary", &self.primary)
            .field("finalization", &self.finalization)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for SourceAdmissionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "source admission refused: {}", self.primary)
    }
}
impl std::error::Error for SourceAdmissionFailure {}

impl SourceLease {
    /// E-only environment sources. Caller must separately bind exact plan,
    /// source/artifact/registry/current assets and operation authority.
    pub fn admit(plan: SourcePlan, limits: ReadLimits) -> Result<Self> {
        plan.validate()?;
        let bytes = plan.required_final_bytes()?;
        let calls = plan.required_final_calls()?;
        ensure!(
            limits.final_reserve_bytes >= bytes && limits.final_reserve_calls >= calls,
            "incomplete final reserve"
        );
        ensure!(
            limits
                .max_bytes
                .checked_sub(limits.final_reserve_bytes)
                .is_some_and(|available| available >= bytes)
                && limits
                    .max_calls
                    .checked_sub(limits.final_reserve_calls)
                    .is_some_and(|available| available >= calls),
            "initial source hash budget"
        );
        let budget = PreparedReadBudget::new(
            limits.max_bytes,
            limits.max_calls,
            limits.final_reserve_bytes,
            limits.final_reserve_calls,
        )?;
        let directories = held_identity::pin_chain(&plan.environment_root)?;
        let mut lease = Self {
            plan,
            raw: BTreeMap::new(),
            overrides: BTreeMap::new(),
            directories,
            budget,
            finalized: false,
        };
        if let Err(error) = lease.admit_members() {
            let finalization = lease.finalize()?;
            return Err(SourceAdmissionFailure {
                primary: format!("{error:#}"),
                finalization,
                _lease: lease,
            }
            .into());
        }
        Ok(lease)
    }
    fn admit_members(&mut self) -> Result<()> {
        let plan = &self.plan;
        ensure!(
            inventory(&plan.environment_root, 20_000)? == plan.entries,
            "initial source membership/order"
        );
        for name in plan
            .entries
            .iter()
            .filter(|name| !name.is_empty() && !plan.raw_files.contains_key(*name))
        {
            self.directories
                .push(PinnedDirectory::open(&plan.environment_root.join(name))?);
        }
        let mut identities = BTreeSet::new();
        for (name, stamp) in &plan.raw_files {
            let held = Held::open(&plan.environment_root.join(name), stamp)?;
            ensure!(
                identities.insert(held.identity.key()),
                "physical source alias"
            );
            self.raw.insert(name.clone(), held);
            self.raw[name].verify(self.budget.clone(), false)?;
        }
        for replacement in &plan.replacements {
            ensure!(
                !replacement.path.starts_with(&plan.environment_root),
                "override inside raw environment"
            );
            self.directories.extend(held_identity::pin_chain(
                replacement.path.parent().context("override parent")?,
            )?);
            let held = Held::open(&replacement.path, &replacement.replacement)?;
            ensure!(
                identities.insert(held.identity.key()),
                "physical override alias"
            );
            self.overrides
                .insert(replacement.logical_path.clone(), held);
            self.overrides[&replacement.logical_path].verify(self.budget.clone(), false)?;
        }
        Ok(())
    }
    pub fn manifest_policy(&self) -> PreparedManifestPolicy {
        PreparedManifestPolicy {
            allow_unknown_manifest_fields: self.plan.allow_unknown_manifest_fields,
        }
    }
    /// Only this admitted raw graph is consulted. Absent files are known absent
    /// in the captured complete namespace, never an IO/parse-error fallback.
    pub fn read_control(&self, name: &str, cap: usize) -> Result<Option<Vec<u8>>> {
        ensure!(!self.finalized, "source lease finalized");
        relative(name, false)?;
        ensure!(
            !name.starts_with("mods/"),
            "package is not a control document"
        );
        for (index, _) in name.match_indices('/') {
            ensure!(
                !self.plan.raw_files.contains_key(&name[..index]),
                "control parent is a file"
            );
        }
        let Some(held) = self.raw.get(name) else {
            ensure!(
                !self.plan.entries.iter().any(|entry| entry == name),
                "control path is a directory"
            );
            return Ok(None);
        };
        let size = usize::try_from(held.stamp.bytes)?;
        ensure!(size <= cap, "control byte cap");
        let mut raw = vec![0; size];
        held.source
            .reader(self.budget.clone(), false)
            .read_exact(&mut raw)?;
        Ok(Some(raw))
    }
    pub fn package_views(&self) -> Result<Vec<PreparedPackageView>> {
        ensure!(!self.finalized, "source lease finalized");
        self.plan
            .package_roots
            .iter()
            .map(|root| {
                let prefix = format!("{root}/");
                let entries = self
                    .plan
                    .entries
                    .iter()
                    .filter(|name| *name == root || name.starts_with(&prefix))
                    .map(|name| {
                        let path = if name == root {
                            ""
                        } else {
                            &name[prefix.len()..]
                        };
                        let source = self
                            .overrides
                            .get(name)
                            .or_else(|| self.raw.get(name))
                            .map(|held| held.source.clone());
                        PreparedPackageEntry {
                            path: path.into(),
                            source,
                        }
                    })
                    .collect();
                Ok(PreparedPackageView::new(
                    camino::Utf8PathBuf::from_path_buf(self.plan.environment_root.join(root))
                        .map_err(|_| anyhow::anyhow!("nonUTF8 package root"))?,
                    entries,
                    self.budget.clone(),
                )?)
            })
            .collect()
    }
    /// Exactly once, before accepting any stage. No IO occurs in Drop. On any
    /// error this lease and all outputs remain unaccepted; no automatic retry.
    pub fn finalize(&mut self) -> Result<SourceFinalization> {
        ensure!(!self.finalized, "source finalization already consumed");
        self.finalized = true;
        let phase_error = self
            .budget
            .begin_finalization()
            .err()
            .map(|e| e.to_string());
        let mut files = Vec::new();
        for (name, held) in self
            .plan
            .raw_files
            .keys()
            .map(|name| (name.clone(), self.raw.get(name)))
            .chain(self.plan.replacements.iter().map(|r| {
                (
                    format!("override:{}", r.logical_path),
                    self.overrides.get(&r.logical_path),
                )
            }))
        {
            let mut row = FileFinalization {
                name,
                identity_matched: None,
                hash_matched: None,
                error: None,
            };
            if let Some(held) = held {
                match held_identity::snapshot(&held.file, &held.path, false) {
                    Ok(identity) => row.identity_matched = Some(identity == held.identity),
                    Err(error) => row.error = Some(format!("{error:#}")),
                }
                if phase_error.is_none()
                    && row.identity_matched == Some(true)
                    && !self.budget.counts().terminal_io
                {
                    match held.verify(self.budget.clone(), true) {
                        Ok(()) => row.hash_matched = Some(true),
                        Err(error) => {
                            row.hash_matched = Some(false);
                            row.error = Some(format!("{error:#}"));
                        }
                    }
                }
            } else {
                row.error = Some("not admitted before primary failure".into());
            }
            files.push(row);
        }
        let mut metadata_errors = Vec::new();
        for directory in &self.directories {
            if let Err(error) = directory.verify() {
                metadata_errors.push(format!("{error:#}"));
            }
        }
        let directories_matched = metadata_errors.is_empty();
        let membership_matched = match inventory(&self.plan.environment_root, 20_000) {
            Ok(actual) => Some(
                actual.into_iter().collect::<BTreeSet<_>>()
                    == self.plan.entries.iter().cloned().collect(),
            ),
            Err(error) => {
                metadata_errors.push(format!("{error:#}"));
                None
            }
        };
        let counts = self.budget.counts();
        Ok(SourceFinalization {
            verified: phase_error.is_none()
                && directories_matched
                && membership_matched == Some(true)
                && files.iter().all(|row| {
                    row.identity_matched == Some(true)
                        && row.hash_matched == Some(true)
                        && row.error.is_none()
                }),
            files,
            directories_matched,
            membership_matched,
            phase_error,
            metadata_errors,
            charged_bytes: counts.charged_bytes,
            returned_bytes: counts.returned_bytes,
            calls: counts.calls,
            terminal_io: counts.terminal_io,
        })
    }
}

#[cfg(all(test, windows))]
mod tests;
