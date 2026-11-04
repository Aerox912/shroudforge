//! Explicit prepared-only external schema admission. Default cache loading never calls this module.
use anyhow::{Context, Result, ensure};
use kfc::reflection::TypeRegistry;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvidencePin {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExternalRegistryInput {
    pub manifest: EvidencePin,
    pub producer_use_approval: EvidencePin,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceStamp {
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    source_inputs: BTreeMap<String, SourceStamp>,
    generation: String,
    files: BTreeMap<String, EvidencePin>,
}
const ROLES: &[&str] = &[
    "registry",
    "identity",
    "report",
    "complete",
    "receipt",
    "adjudication_manifest",
    "ledger",
    "consumer_contract",
    "adjudication_report",
    "adjudication_retention",
    "diagnosis_manifest",
    "matrix",
    "source_evidence",
    "diagnosis_report",
    "diagnosis_decision",
    "diagnosis_retention",
    "actual_refusal",
    "diagnosis_assessment",
    "canonical_acceptance",
    "main_acceptance",
];
const MAX_TOTAL: u64 = 96 * 1024 * 1024;
const STRICT_ID: &str = "sf177-static-reflection-strict-v1";
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn digest_ok(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn normalize(v: &Value) -> Value {
    match v {
        Value::Object(o) => serde_json::to_value(
            o.iter()
                .map(|(k, v)| (k, normalize(v)))
                .collect::<BTreeMap<_, _>>(),
        )
        .unwrap(),
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        _ => v.clone(),
    }
}
fn canonical(v: &Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&normalize(v))?)
}

// Reject duplicate keys before serde/SDK maps can silently replace them.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Unique;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Unique, E> {
                Ok(Unique(json!(v)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = a.next_element::<Unique>()? {
                    out.push(v.0);
                }
                Ok(Unique(Value::Array(out)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if out.contains_key(&k) {
                        return Err(serde::de::Error::custom("duplicate external evidence key"));
                    }
                    out.insert(k, a.next_value::<Unique>()?.0);
                }
                Ok(Unique(Value::Object(out)))
            }
        }
        d.deserialize_any(Visitor)
    }
}
fn parse_json(raw: &[u8]) -> Result<Value> {
    let mut d = serde_json::Deserializer::from_slice(raw);
    let v = Unique::deserialize(&mut d)?.0;
    d.end()?;
    Ok(v)
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .context("external registry string missing")
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key).context("external registry field missing")
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    field(v, key)?
        .as_array()
        .context("external registry array missing")
}
fn ordinary(p: &Path, dir: bool) -> Result<()> {
    let m = fs::symlink_metadata(p)?;
    ensure!(!m.file_type().is_symlink(), "external linked path");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(m.file_attributes() & 0x400 == 0, "external reparse path");
    }
    ensure!(
        if dir { m.is_dir() } else { m.is_file() },
        "external path kind"
    );
    Ok(())
}
struct Bundle {
    handles: Vec<File>,
    total: u64,
    paths: BTreeSet<String>,
    files: BTreeMap<String, SourceStamp>,
}
impl Bundle {
    fn hold_dir(&mut self, p: &Path) -> Result<()> {
        ordinary(p, true)?;
        let mut o = OpenOptions::new();
        o.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.access_mode(0x80)
                .share_mode(1)
                .custom_flags(0x02000000 | 0x00200000);
        }
        let f = o.open(p)?;
        let held = f.metadata()?;
        ensure!(held.is_dir(), "external held directory kind");
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                held.file_attributes() & 0x400 == 0,
                "external held directory reparse"
            );
        }
        self.handles.push(f);
        ensure!(self.handles.len() <= 512, "external handle bound");
        Ok(())
    }
    fn read(&mut self, root: &Path, pin: &EvidencePin) -> Result<Vec<u8>> {
        ensure!(
            pin.bytes <= 64 * 1024 * 1024 && digest_ok(&pin.sha256),
            "external evidence bound/digest"
        );
        self.total = self
            .total
            .checked_add(pin.bytes)
            .context("external byte overflow")?;
        ensure!(self.total <= MAX_TOTAL, "external aggregate bound");
        ensure!(
            !pin.path.is_empty() && !pin.path.contains(['\\', ':']) && !pin.path.starts_with('/'),
            "external relative path required"
        );
        let mut p = root.to_path_buf();
        let parts: Vec<_> = pin.path.split('/').collect();
        ensure!(
            parts.len() <= 16 && self.paths.insert(pin.path.to_ascii_lowercase()),
            "external path alias"
        );
        for (i, s) in parts.iter().enumerate() {
            ensure!(
                !s.is_empty()
                    && *s != "."
                    && *s != ".."
                    && !s.ends_with([' ', '.'])
                    && !s.chars().any(|c| c.is_control() || "<>\"|?*".contains(c)),
                "external path component"
            );
            let device = s.split('.').next().unwrap().to_ascii_uppercase();
            ensure!(
                !["CON", "PRN", "AUX", "NUL"].contains(&device.as_str())
                    && !(device.len() == 4
                        && (device.starts_with("COM") || device.starts_with("LPT"))
                        && matches!(device.as_bytes()[3], b'1'..=b'9')),
                "external device alias"
            );
            p.push(s);
            if i + 1 < parts.len() {
                self.hold_dir(&p)?;
            }
        }
        ordinary(&p, false)?;
        let mut o = OpenOptions::new();
        o.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.share_mode(1).custom_flags(0x00200000);
        }
        let mut f = o.open(&p)?;
        let m = f.metadata()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(m.file_attributes() & 0x400 == 0, "external held reparse");
        }
        ensure!(m.is_file() && m.len() == pin.bytes, "external held length");
        let mut bytes = Vec::with_capacity(pin.bytes as usize);
        let mut remaining = pin.bytes;
        let mut b = [0u8; 65536];
        while remaining > 0 {
            let amount = remaining.min(b.len() as u64) as usize;
            let n = f.read(&mut b[..amount])?;
            ensure!(n == amount, "external short read");
            bytes.extend_from_slice(&b[..n]);
            remaining -= n as u64;
        }
        ensure!(
            f.metadata()?.len() == pin.bytes && hash(&bytes) == pin.sha256,
            "external evidence changed"
        );
        self.handles.push(f);
        self.files.insert(
            pin.path.clone(),
            SourceStamp {
                bytes: pin.bytes,
                sha256: pin.sha256.clone(),
            },
        );
        Ok(bytes)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ProducerIdentity {
    source_closure: String,
    artifact_sha256: String,
}
fn current_producer() -> Result<ProducerIdentity> {
    let closure =
        option_env!("HD_PREPARED_SOURCE_CLOSURE").context("external producer closure not built")?;
    ensure!(digest_ok(closure), "external producer closure invalid");
    let exe = std::env::current_exe()?;
    let metadata = fs::metadata(&exe)?;
    ensure!(
        metadata.len() <= 128 * 1024 * 1024,
        "external producer artifact bound"
    );
    let mut f = File::open(exe)?;
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(ProducerIdentity {
        source_closure: closure.into(),
        artifact_sha256: format!("{:x}", h.finalize()),
    })
}
pub struct VerifiedExternalRegistry {
    pub(crate) registry: std::rc::Rc<TypeRegistry>,
    _leases: Bundle,
    provenance: Value,
}
impl VerifiedExternalRegistry {
    pub fn provenance(&self) -> &Value {
        &self.provenance
    }
    /// Exact relative evidence files already verified and retained by admission,
    /// including its manifest, producer-use approval and producer review.
    pub fn evidence_files(&self) -> &BTreeMap<String, SourceStamp> {
        &self._leases.files
    }
}
/// Validate only explicitly pinned E/evidence inputs, before the prepared caller touches source/cache.
/// The caller must obtain manifest/approval pins from its separately authorized prepared request.
pub fn admit_external_registry(
    root: &Path,
    input: &ExternalRegistryInput,
    expected_sources: &BTreeMap<String, SourceStamp>,
) -> Result<VerifiedExternalRegistry> {
    #[cfg(not(windows))]
    {
        let _ = (root, input, expected_sources);
        anyhow::bail!("external admission requires Windows held sharing guards");
    }
    #[cfg(windows)]
    {
        load(root, input, expected_sources, &current_producer()?)
            .context("external registry admission refused")
    }
}
fn load(
    root: &Path,
    input: &ExternalRegistryInput,
    expected_sources: &BTreeMap<String, SourceStamp>,
    producer: &ProducerIdentity,
) -> Result<VerifiedExternalRegistry> {
    ensure!(
        root.is_absolute()
            && !root
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
        "external root must be explicit"
    );
    let mut b = Bundle {
        handles: Vec::new(),
        total: 0,
        paths: BTreeSet::new(),
        files: BTreeMap::new(),
    };
    for p in root.ancestors().collect::<Vec<_>>().into_iter().rev() {
        b.hold_dir(p)?;
    }
    let raw = b.read(root, &input.manifest)?;
    let m: Manifest = serde_json::from_value(parse_json(&raw)?)?;
    ensure!(m.schema_version == 1, "external manifest schema");
    ensure!(
        m.source_inputs == *expected_sources
            && m.source_inputs.len() == 3
            && [
                "enshrouded.exe",
                "enshrouded.kfc",
                "enshrouded.kfc_resources"
            ]
            .iter()
            .all(|n| m.source_inputs.contains_key(*n)),
        "external source tuple differs"
    );
    ensure!(
        m.source_inputs.values().all(|s| digest_ok(&s.sha256)) && !m.generation.is_empty(),
        "external source identity invalid"
    );
    ensure!(
        m.files.keys().map(String::as_str).collect::<BTreeSet<_>>()
            == ROLES.iter().copied().collect(),
        "external evidence membership differs"
    );
    let approval: Value = parse_json(&b.read(root, &input.producer_use_approval)?)?;
    ensure!(
        approval["schemaVersion"] == 1
            && approval["status"] == "COMMITTED_REVIEWED_PRODUCER_EXTERNAL_REGISTRY_USE"
            && approval["sourceState"] == "committed"
            && approval["canonicalProducerReviewAccepted"] == true
            && approval["mainProducerUseAccepted"] == true,
        "external final producer approval absent"
    );
    ensure!(
        serde_json::from_value::<ProducerIdentity>(field(&approval, "producer")?.clone())?
            == *producer,
        "external producer identity differs"
    );
    let commit = text(&approval, "producerCommit")?;
    ensure!(
        commit.len() == 40
            && commit
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "external committed producer missing"
    );
    ensure!(
        text(&approval, "manifestSha256")? == input.manifest.sha256
            && text(&approval, "mainDataAcceptanceSha256")? == m.files["main_acceptance"].sha256,
        "external producer approval not bound"
    );
    let review_pin: EvidencePin =
        serde_json::from_value(field(&approval, "canonicalProducerReview")?.clone())?;
    let review = parse_json(&b.read(root, &review_pin)?)?;
    ensure!(
        review["schemaVersion"] == 1
            && review["status"] == "CANONICAL_ACCEPTED_COMMITTED_PRODUCER_SOURCE_ARTIFACT",
        "external producer review not accepted"
    );
    ensure!(
        serde_json::from_value::<ProducerIdentity>(field(&review, "producer")?.clone())?
            == *producer
            && review["producerCommit"] == commit,
        "external reviewed producer identity differs"
    );
    let mut docs = BTreeMap::new();
    for role in ROLES {
        let pin = &m.files[*role];
        let bytes = b.read(root, pin)?;
        let value = if [
            "consumer_contract",
            "adjudication_report",
            "diagnosis_report",
        ]
        .contains(role)
        {
            Value::Null
        } else {
            parse_json(&bytes)?
        };
        docs.insert(*role, value);
    }
    let registry = validate(&m, &docs)?;
    let provenance = json!({"schemaVersion":1,"mode":"explicit_external_registry_read_only","manifest":input.manifest,"producerUseApproval":input.producer_use_approval,"producer":producer,"derivation":docs["receipt"]["derivationIdentity"],"lossLedger":m.files["ledger"],"mainAcceptance":m.files["main_acceptance"],"canonicalAcceptance":m.files["canonical_acceptance"],"generation":m.generation,"sourceInputs":m.source_inputs,"historicalToolQualificationReady":false});
    Ok(VerifiedExternalRegistry {
        registry: std::rc::Rc::new(registry),
        _leases: b,
        provenance,
    })
}
fn matches_ref(m: &Manifest, role: &str, v: &Value) -> Result<()> {
    let pin = &m.files[role];
    ensure!(
        v["sha256"] == pin.sha256 && v["bytes"] == pin.bytes,
        "external referenced identity differs"
    );
    Ok(())
}
fn same_hash(m: &Manifest, role: &str, v: &Value, key: &str) -> Result<()> {
    ensure!(
        v[key] == m.files[role].sha256,
        "external provenance hash differs"
    );
    Ok(())
}

/// Validate the actual review's declared format without rewriting its history.
/// Acceptance here concerns refusal evidence, not schema or producer readiness.
fn validate_actual_refusal(m: &Manifest, refusal: &Value) -> Result<()> {
    match text(refusal, "status")? {
        "VERIFIED_TERMINAL_REFUSAL_UNSUPPORTED_SCHEMA_PRESENCE" => {
            ensure!(
                refusal["wrapperExit"] == 3 && refusal["childExit"] == 2,
                "external actual refusal identity"
            );
            for (role, key) in [
                ("complete", "completeSha256"),
                ("registry", "registrySha256"),
                ("identity", "identitySha256"),
                ("report", "reportSha256"),
            ] {
                same_hash(m, role, refusal, key)?;
            }
        }
        "VERIFIED_TERMINAL_CURRENT5D_PHASE_A_REFUSAL_UNSUPPORTED_SCHEMA_PRESENCE" => {
            ensure!(
                refusal["actualRefusalEvidenceAccepted"] == true
                    && refusal["derivativeIntegrityAccepted"] == true
                    && refusal["qualificationReady"] == false
                    && refusal["schemaPresenceQualified"] == false
                    && refusal["historicalRegistryActivated"] == false
                    && refusal["producerUseApproved"] == false
                    && refusal["nativeProfilesQualified"] == false
                    && refusal["operationSpent"] == true
                    && refusal["retryApproved"] == false
                    && refusal["execution"]["processExit"].as_i64() == Some(2)
                    && refusal["execution"]["invocations"].as_u64() == Some(1),
                "external current refusal identity or qualification differs"
            );
            for role in ["complete", "registry", "identity", "report"] {
                matches_ref(m, role, field(refusal, role)?)?;
            }
        }
        _ => anyhow::bail!("external actual refusal format is unsupported"),
    }
    Ok(())
}

fn validate_refusal_contract(
    m: &Manifest, refusal: &Value, report: &Value, complete: &Value, receipt: &Value,
) -> Result<()> {
    validate_actual_refusal(m, refusal)?;
    ensure!(
        report["qualificationReady"] == false
            && complete["qualificationReady"] == false
            && receipt["historicalRefusalPreserved"]["childExit"] == 2
            && receipt["historicalRefusalPreserved"]["qualificationReady"] == false,
        "external historical refusal changed"
    );
    Ok(())
}
fn manifest_members(m: &Manifest, doc: &Value, pairs: &[(&str, &str)]) -> Result<()> {
    let files = array(doc, "files")?;
    ensure!(files.len() == pairs.len(), "external member count");
    let mut names = BTreeSet::new();
    for entry in files {
        let name = text(entry, "path")?;
        ensure!(names.insert(name), "external duplicate member");
        let role = pairs
            .iter()
            .find(|(n, _)| *n == name)
            .context("external extra member")?
            .1;
        matches_ref(m, role, entry)?;
    }
    Ok(())
}
fn validate(m: &Manifest, d: &BTreeMap<&str, Value>) -> Result<TypeRegistry> {
    let r = &d["receipt"];
    let main = &d["main_acceptance"];
    let ca = &d["canonical_acceptance"];
    ensure!(
        r["schemaVersion"] == 1
            && r["status"] == "CONDITIONAL_EXACT_DATA_ADJUDICATION_REQUIRES_EXTERNAL_ACCEPTANCE",
        "external receipt schema/status"
    );
    let id = text(r, "adjudicationId")?;
    ensure!(
        main["adjudicationId"] == id && ca["adjudicationId"] == id,
        "external adjudication identity differs"
    );
    ensure!(
        main["status"] == "MAIN_ACCEPTED_EXACT_SDK_REPRESENTED_DATA_ONLY"
            && main["representedDataAccepted"] == true
            && ca["canonicalPacketAcceptance"] == true
            && ca["status"] == "CANONICAL_ACCEPTED_EXACT_PACKET_PENDING_MAIN_FINAL_ACTIVATION",
        "external activation missing"
    );
    same_hash(
        m,
        "canonical_acceptance",
        &main["canonicalVerdict"],
        "sha256",
    )?;
    for (role, key) in [
        ("adjudication_manifest", "manifestSha256"),
        ("receipt", "receiptSha256"),
        ("ledger", "lossLedgerSha256"),
        ("consumer_contract", "consumerContractSha256"),
    ] {
        same_hash(m, role, &main["packet"], key)?;
        same_hash(m, role, &ca["acceptedPacket"], key)?;
    }
    manifest_members(
        m,
        &d["adjudication_manifest"],
        &[
            ("ADJUDICATION-RECEIPT.json", "receipt"),
            ("MANDATORY-LOSS-LEDGER.json", "ledger"),
            ("CONSUMER-ACCEPTANCE-CONTRACT.md", "consumer_contract"),
            ("REPORT.md", "adjudication_report"),
            ("REUSE-RETENTION.json", "adjudication_retention"),
        ],
    )?;
    manifest_members(
        m,
        &d["diagnosis_manifest"],
        &[
            ("COLLISION-MATRIX.json", "matrix"),
            ("DECISION-BOUNDARY.json", "diagnosis_decision"),
            ("EVIDENCE.json", "source_evidence"),
            ("REPORT.md", "diagnosis_report"),
            ("RETENTION.json", "diagnosis_retention"),
        ],
    )?;
    let loss = field(r, "mandatoryLossEvidence")?;
    for (role, key) in [
        ("report", "originalPhaseAReport"),
        ("matrix", "full18Matrix"),
        ("ledger", "lossLedger"),
        ("source_evidence", "fullSourceEvidence"),
        ("diagnosis_manifest", "diagnosisManifest"),
        ("diagnosis_assessment", "canonicalDiagnosisAssessment"),
        ("actual_refusal", "canonicalActualRefusal"),
    ] {
        matches_ref(m, role, field(loss, key)?)?;
    }
    let cap = array(r, "exactCapturedOutputs")?;
    ensure!(cap.len() == 4, "external captured set");
    let mut outputs = BTreeSet::new();
    for v in cap {
        let role = ["registry", "identity", "report", "complete"]
            .into_iter()
            .find(|role| m.files[*role].sha256 == v["sha256"])
            .context("external unknown captured output")?;
        ensure!(outputs.insert(role), "external duplicate captured output");
        matches_ref(m, role, v)?;
    }
    let complete_files = array(&d["complete"], "files")?;
    ensure!(complete_files.len() == 3, "external COMPLETE set differs");
    let mut complete_names = BTreeSet::new();
    for row in complete_files {
        let name = text(row, "name")?;
        let role = match name {
            "types-enshrouded.json" => "registry",
            "types-enshrouded.identity.json" => "identity",
            "PHASE-A-REPORT.json" => "report",
            _ => anyhow::bail!("external COMPLETE extra member"),
        };
        ensure!(complete_names.insert(name), "external COMPLETE duplicate");
        matches_ref(m, role, row)?;
    }
    validate_refusal_contract(m, &d["actual_refusal"], &d["report"], &d["complete"], r)?;
    ensure!(
        d["diagnosis_assessment"]["status"]
            == "PASS_COMPLETE_DIAGNOSIS_CONDITIONAL_REPRESENTED_DATA_BOUNDARY",
        "external diagnosis assessment not accepted"
    );
    for (role, key) in [
        ("diagnosis_manifest", "diagnosisManifestSha256"),
        ("matrix", "matrixSha256"),
        ("source_evidence", "sourceEvidenceSha256"),
    ] {
        same_hash(m, role, &d["diagnosis_assessment"], key)?;
    }
    same_hash(
        m,
        "actual_refusal",
        &d["diagnosis_assessment"]["priorActualVerdict"],
        "sha256",
    )?;
    let reused = array(ca, "reusedEvidence")?;
    ensure!(
        reused.len() == 2
            && reused
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
                == [
                    m.files["actual_refusal"].sha256.as_str(),
                    m.files["diagnosis_assessment"].sha256.as_str()
                ]
                .into_iter()
                .collect(),
        "external canonical provenance differs"
    );
    let derived = field(r, "derivationIdentity")?;
    ensure!(
        derived["reflectionExtractor"] == STRICT_ID
            && d["identity"]["reflectionExtractor"] == STRICT_ID,
        "external extractor identity differs"
    );
    ensure!(
        derived["sourceClosure"] == d["identity"]["sourceClosure"]
            && d["identity"]["registrySha256"] == m.files["registry"].sha256,
        "external derivation link differs"
    );
    ensure!(
        digest_ok(text(derived, "sourceClosure")?)
            && d["identity"]["schemaVersion"] == 2
            && d["identity"]["executableSha256"] == m.source_inputs["enshrouded.exe"].sha256
            && d["identity"]["pairedKfcSha256"] == m.source_inputs["enshrouded.kfc"].sha256,
        "external derived source differs"
    );
    ensure!(
        d["identity"]["pairedKfcGeneration"] == m.generation
            && d["registry"]["version"] == m.generation
            && r["newContractScope"]["pairedKfcGeneration"] == m.generation,
        "external generation differs"
    );
    source_rows(
        array(&r["newContractScope"], "originalSourceTupleAtCapture")?,
        &m.source_inputs,
    )?;
    source_rows(array(&d["complete"], "sourceInputs")?, &m.source_inputs)?;
    source_rows(array(&d["report"], "sourceInputs")?, &m.source_inputs)?;
    ensure!(
        d["report"]["sourceClosure"] == derived["sourceClosure"]
            && d["report"]["contractSha256"] == derived["contract"]["sha256"],
        "external report derivation differs"
    );
    ensure!(
        d["source_evidence"]["actualIdentity"] == d["identity"],
        "external source evidence identity differs"
    );
    for (key, origin) in [
        ("index", "candidateIndex"),
        ("source", "sourceManifest"),
        ("contract", "contract"),
        ("artifact", "artifact"),
    ] {
        let a = &d["source_evidence"]["frozenCandidate"][key];
        let b = &derived[origin];
        ensure!(
            a["sha256"] == b["sha256"]
                && a["bytes"] == b["bytes"]
                && a["sha256"].as_str().is_some_and(digest_ok),
            "external source artifact provenance differs"
        );
    }
    ensure!(
        d["report"]["unresolvedNonzeroOptionalTypeReferences"] == 0
            && array(&d["report"], "referencePresenceIssues")?.is_empty(),
        "external unresolved references"
    );
    validate_collisions(m, d)?;
    Ok(serde_json::from_value(d["registry"].clone())?)
}
fn source_rows(rows: &[Value], expected: &BTreeMap<String, SourceStamp>) -> Result<()> {
    ensure!(rows.len() == 3, "external source row count");
    let mut seen = BTreeSet::new();
    for row in rows {
        let name = text(row, "name")?;
        let stamp = expected.get(name).context("external extra source row")?;
        ensure!(
            seen.insert(name) && row["bytes"] == stamp.bytes && row["sha256"] == stamp.sha256,
            "external source row differs"
        );
    }
    Ok(())
}
fn validate_collisions(m: &Manifest, d: &BTreeMap<&str, Value>) -> Result<()> {
    let receipt = &d["receipt"];
    let admission = field(receipt, "collisionAdmission")?;
    ensure!(
        admission["exactCapturedCaseOnly"] == true
            && admission["generalOrWildcardCollisionAcceptance"] == false,
        "external wildcard acceptance"
    );
    let rows = array(&d["matrix"], "records")?;
    let ledger = array(&d["ledger"], "records")?;
    let audit = array(&d["report"], "attributeNameCollisionRecords")?;
    ensure!(
        !rows.is_empty()
            && rows.len() <= 262144
            && ledger.len() == rows.len()
            && audit.len() == rows.len(),
        "external collision set incomplete"
    );
    ensure!(
        d["ledger"]["representedReplacementSequenceCanonicalSha256"]
            == hash(&canonical(&Value::Array(ledger.clone()))?)
            && receipt["mandatoryLossEvidence"]["representedReplacementSequenceCanonicalSha256"]
                == d["ledger"]["representedReplacementSequenceCanonicalSha256"],
        "external ledger order/digest differs"
    );
    matches_ref(m, "matrix", &d["ledger"]["sourceMatrix"])?;
    matches_ref(m, "report", &d["ledger"]["originalReport"])?;
    let types = array(&d["registry"], "types")?;
    let mut maps = BTreeSet::new();
    let mut owners = BTreeMap::new();
    let mut identical = 0;
    for (i, ((c, l), a)) in rows.iter().zip(ledger).zip(audit).enumerate() {
        let index = c["typeIndex"]
            .as_u64()
            .context("external collision type index")? as usize;
        let ty = types.get(index).context("external owner type absent")?;
        let prev = field(c, "previous")?;
        let next = field(c, "current")?;
        let name = text(next, "name")?;
        let map = text(c, "map")?;
        ensure!(
            prev["name"] == next["name"]
                && prev.get("namespace") == next.get("namespace")
                && prev.get("type") == next.get("type"),
            "external namespace/type collision"
        );
        for attr in [prev, next] {
            let obj = attr.as_object().context("external attribute shape")?;
            ensure!(
                obj.keys()
                    .all(|k| ["name", "namespace", "type", "value"].contains(&k.as_str())),
                "external unknown attribute field"
            );
        }
        ensure!(
            c["key"] == name && c["qualifiedTypeName"] == ty["qualifiedName"],
            "external named owner differs"
        );
        let exact = prev == next;
        let class = if exact {
            identical += 1;
            "identical_represented_record_duplicate"
        } else {
            "same_reflected_identity_different_string_value"
        };
        if !exact {
            let mut p = prev.clone();
            let mut n = next.clone();
            ensure!(
                p.as_object_mut()
                    .context("external attribute shape")?
                    .remove("value")
                    .is_some()
                    && n.as_object_mut()
                        .context("external attribute shape")?
                        .remove("value")
                        .is_some()
                    && p == n
                    && prev["value"].is_string()
                    && next["value"].is_string(),
                "external unknown differing value"
            );
        }
        ensure!(
            c["recordOrdinal"] == (i + 1)
                && c["classification"] == class
                && maps.insert((index, map)),
            "external collision order/owner"
        );
        ensure!(
            a == &json!({"typeIndex":index,"map":map,"previous":prev,"current":next}),
            "external audit differs"
        );
        let expected = json!({"auditRecordOrdinal":i+1,"owner":{"typeIndex":index,"qualifiedTypeName":ty["qualifiedName"],"attributeMap":map,"mapKey":name},"earlier":prev,"later":next,"replacementSequence":[prev,next],"classification":class,"retainedRegistryWinner":next});
        ensure!(l == &expected, "external loss ledger differs");
        let attrs = if map == "type.attributes" {
            field(ty, "attributes")?
        } else {
            let key = map
                .strip_prefix("structFields.")
                .and_then(|x| x.strip_suffix(".attributes"))
                .context("external owner map")?;
            &ty["structFields"][key]["attributes"]
        };
        ensure!(
            attrs.get(name) == Some(next) && c["currentRegistryWinner"] == *next,
            "external retained winner differs"
        );
        let h = hash(&canonical(ty)?);
        ensure!(
            c["currentTypeCanonicalSha256"] == h && c["embeddedOldSerializedTypeFingerprint"] == h,
            "external affected fingerprint differs"
        );
        owners.insert(index, h);
    }
    let fps = array(admission, "affectedRepresentedTypeFingerprints")?;
    ensure!(
        fps.len() == owners.len(),
        "external fingerprint set differs"
    );
    let mut seen = BTreeSet::new();
    for fp in fps {
        let index = fp["typeIndex"]
            .as_u64()
            .context("external fingerprint index")? as usize;
        let h = owners.get(&index).context("external extra fingerprint")?;
        ensure!(
            seen.insert(index)
                && fp["currentRepresentedTypeCanonicalSha256"] == *h
                && fp["oldSerializedTypeFingerprint"] == *h
                && fp["oldRawEqualityProven"] == false,
            "external fingerprint identity differs"
        );
    }
    ensure!(
        admission["ownerMaps"] == maps.len()
            && admission["ownerTypes"] == owners.len()
            && admission["sameReflectedIdentity"] == rows.len()
            && admission["sameIdentityDifferentString"] == rows.len() - identical
            && admission["exactRepresentedDuplicates"] == identical,
        "external classified counts differ"
    );
    Ok(())
}

#[cfg(test)]
mod refusal_format_tests {
    use super::*;

    fn current() -> (Manifest, Value) {
        let verdict: Value = serde_json::from_str(include_str!("fixtures/current5d-refusal.json")).unwrap();
        let files = ["registry", "identity", "report", "complete"]
            .into_iter()
            .map(|role| (role.to_owned(), EvidencePin {
                path: format!("evidence/{role}.json"),
                bytes: verdict[role]["bytes"].as_u64().unwrap(),
                sha256: verdict[role]["sha256"].as_str().unwrap().to_owned(),
            }))
            .collect();
        (Manifest { schema_version: 1, source_inputs: BTreeMap::new(), generation: String::new(), files }, verdict)
    }

    #[test]
    fn current_retained_refusal_format_accepts_only_bound_unqualified_evidence() {
        let (manifest, verdict) = current();
        validate_actual_refusal(&manifest, &verdict).unwrap();
        for key in ["qualificationReady", "schemaPresenceQualified", "historicalRegistryActivated",
                    "producerUseApproved", "nativeProfilesQualified", "retryApproved"] {
            let mut changed = verdict.clone();
            changed[key] = json!(true);
            assert!(validate_actual_refusal(&manifest, &changed).is_err(), "accepted {key}");
        }
        for key in ["actualRefusalEvidenceAccepted", "derivativeIntegrityAccepted", "operationSpent"] {
            let mut changed = verdict.clone();
            changed.as_object_mut().unwrap().remove(key);
            assert!(validate_actual_refusal(&manifest, &changed).is_err(), "missing {key}");
        }
        for (key, value) in [("processExit", json!(0)), ("processExit", json!(2.0)), ("invocations", json!(2))] {
            let mut changed = verdict.clone(); changed["execution"][key] = value;
            assert!(validate_actual_refusal(&manifest, &changed).is_err());
        }
    }

    #[test]
    fn current_refusal_rejects_changed_or_missing_output_bindings() {
        let (manifest, verdict) = current();
        for role in ["registry", "identity", "report", "complete"] {
            for key in ["bytes", "sha256"] {
                let mut changed = verdict.clone();
                changed[role].as_object_mut().unwrap().remove(key);
                assert!(validate_actual_refusal(&manifest, &changed).is_err(), "missing {role}.{key}");
            }
            let mut changed = verdict.clone(); changed[role]["sha256"] = json!("0".repeat(64));
            assert!(validate_actual_refusal(&manifest, &changed).is_err(), "changed {role}");
        }
        let mut changed = verdict; changed["status"] = json!("UNKNOWN_REFUSAL_FORMAT");
        assert!(validate_actual_refusal(&manifest, &changed).is_err());
    }

    #[test]
    fn public_refusal_contract_uses_current_exit_without_flat_field_and_retains_refusals() {
        let (manifest, verdict) = current();
        assert!(verdict.get("childExit").is_none());
        let report = json!({"qualificationReady":false});
        let complete = report.clone();
        let receipt = json!({"historicalRefusalPreserved":{"childExit":2,"qualificationReady":false}});
        validate_refusal_contract(&manifest, &verdict, &report, &complete, &receipt).unwrap();
        let mut promoted = report.clone(); promoted["qualificationReady"] = json!(true);
        assert!(validate_refusal_contract(&manifest, &verdict, &promoted, &complete, &receipt).is_err());
        assert!(validate_refusal_contract(&manifest, &verdict, &report, &promoted, &receipt).is_err());
        for (key, value) in [("childExit", json!(0)), ("qualificationReady", json!(true))] {
            let mut changed = receipt.clone(); changed["historicalRefusalPreserved"][key] = value;
            assert!(validate_refusal_contract(&manifest, &verdict, &report, &complete, &changed).is_err());
        }
    }

    #[test]
    fn legacy_refusal_flat_hashes_and_wrapper_exit_remain_supported() {
        let (manifest, _) = current();
        let mut legacy = json!({"status":"VERIFIED_TERMINAL_REFUSAL_UNSUPPORTED_SCHEMA_PRESENCE", "wrapperExit":3, "childExit":2});
        for (role, key) in [("registry", "registrySha256"), ("identity", "identitySha256"),
                            ("report", "reportSha256"), ("complete", "completeSha256")] {
            legacy[key] = json!(manifest.files[role].sha256);
        }
        validate_actual_refusal(&manifest, &legacy).unwrap();
        legacy["wrapperExit"] = json!(2);
        assert!(validate_actual_refusal(&manifest, &legacy).is_err());
        legacy["wrapperExit"] = json!(3); legacy["childExit"] = json!(0);
        assert!(validate_actual_refusal(&manifest, &legacy).is_err());
        legacy["childExit"] = json!(2); legacy["reportSha256"] = json!("0".repeat(64));
        assert!(validate_actual_refusal(&manifest, &legacy).is_err());
    }
}
