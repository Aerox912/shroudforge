//! Small portable, fictional evidence. No captured registry, local path or production approval.
use super::*;
use serde_json::{Value, json};
use shroudforge_api::{EvidencePin, ExternalRegistryInput, SourceStamp};
fn digest(v: &Value) -> String {
    fn norm(v: &Value) -> Value {
        match v {
            Value::Object(o) => serde_json::to_value(
                o.iter()
                    .map(|(k, v)| (k, norm(v)))
                    .collect::<BTreeMap<_, _>>(),
            )
            .unwrap(),
            Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
            _ => v.clone(),
        }
    }
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&norm(v)).unwrap())
    )
}
struct EvidenceFixture {
    root: PathBuf,
    input: ExternalRegistryInput,
    sources: BTreeMap<String, SourceStamp>,
    docs: BTreeMap<String, Value>,
    pins: BTreeMap<String, EvidencePin>,
    corrupt_complete: bool,
}
impl EvidenceFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "external-registry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let empty = EvidencePin {
            path: String::new(),
            bytes: 0,
            sha256: String::new(),
        };
        let mut f = Self {
            root,
            input: ExternalRegistryInput {
                manifest: empty.clone(),
                producer_use_approval: empty,
            },
            sources: BTreeMap::new(),
            docs: BTreeMap::new(),
            pins: BTreeMap::new(),
            corrupt_complete: false,
        };
        for n in [
            "enshrouded.exe",
            "enshrouded.kfc",
            "enshrouded.kfc_resources",
        ] {
            f.sources.insert(
                n.into(),
                SourceStamp {
                    bytes: 7,
                    sha256: "c".repeat(64),
                },
            );
        }
        f.docs.insert("registry".into(),json!({"version":"fictional-generation","types":[{"index":0,"name":"U32","qualifiedName":"fixture::U32","impactName":"U32","size":4,"alignment":4,"elementAlignment":4,"fieldCount":0,"primitiveType":"UINT32","flags":"","nameHash":kfc::hash::fnv("U32"),"qualifiedHash":kfc::hash::fnv("fixture::U32"),"impactHash":kfc::hash::fnv("U32"),"internalHash":1,"attributes":{"note":{"name":"note","namespace":["fixture"],"type":0,"value":"later"}}}]}));
        f.docs.insert("identity".into(),json!({"schemaVersion":2,"reflectionExtractor":"sf177-static-reflection-strict-v1","sourceClosure":"d".repeat(64),"executableSha256":"c".repeat(64),"pairedKfcSha256":"c".repeat(64),"pairedKfcGeneration":"fictional-generation"}));
        let prev = json!({"name":"note","namespace":["fixture"],"type":0,"value":"earlier"});
        let next = json!({"name":"note","namespace":["fixture"],"type":0,"value":"later"});
        f.docs.insert("report".into(),json!({"qualificationReady":false,"unresolvedNonzeroOptionalTypeReferences":0,"referencePresenceIssues":[],"attributeNameCollisionRecords":[{"typeIndex":0,"map":"type.attributes","previous":prev,"current":next}]}));
        f.docs.insert("matrix".into(),json!({"records":[{"recordOrdinal":1,"typeIndex":0,"qualifiedTypeName":"fixture::U32","map":"type.attributes","key":"note","previous":prev,"current":next,"classification":"same_reflected_identity_different_string_value","currentRegistryWinner":next,"currentTypeCanonicalSha256":digest(&f.docs["registry"]["types"][0]),"embeddedOldSerializedTypeFingerprint":digest(&f.docs["registry"]["types"][0])}]}));
        f.docs.insert("ledger".into(),json!({"records":[{"auditRecordOrdinal":1,"owner":{"typeIndex":0,"qualifiedTypeName":"fixture::U32","attributeMap":"type.attributes","mapKey":"note"},"earlier":prev,"later":next,"replacementSequence":[prev,next],"classification":"same_reflected_identity_different_string_value","retainedRegistryWinner":next}]}));
        f.docs.insert("actual_refusal".into(),json!({"status":"VERIFIED_TERMINAL_REFUSAL_UNSUPPORTED_SCHEMA_PRESENCE","childExit":2,"wrapperExit":3}));
        for n in [
            "source_evidence",
            "diagnosis_decision",
            "diagnosis_retention",
            "adjudication_retention",
            "diagnosis_assessment",
        ] {
            f.docs.insert(n.into(), json!({"fictional":true}));
        }
        f.docs.insert("receipt".into(),json!({"schemaVersion":1,"status":"CONDITIONAL_EXACT_DATA_ADJUDICATION_REQUIRES_EXTERNAL_ACCEPTANCE","adjudicationId":"fixture-exact-case","derivationIdentity":{"reflectionExtractor":"sf177-static-reflection-strict-v1","sourceClosure":"d".repeat(64),"candidateIndex":{"bytes":1,"sha256":"1".repeat(64)},"sourceManifest":{"bytes":1,"sha256":"2".repeat(64)},"contract":{"bytes":1,"sha256":"3".repeat(64)},"artifact":{"bytes":1,"sha256":"4".repeat(64)}},"newContractScope":{"pairedKfcGeneration":"fictional-generation"},"historicalRefusalPreserved":{"childExit":2,"wrapperExit":3,"qualificationReady":false},"collisionAdmission":{"exactCapturedCaseOnly":true,"generalOrWildcardCollisionAcceptance":false,"ownerMaps":1,"ownerTypes":1,"sameReflectedIdentity":1,"sameIdentityDifferentString":1,"exactRepresentedDuplicates":0,"affectedRepresentedTypeFingerprints":[{"typeIndex":0,"currentRepresentedTypeCanonicalSha256":digest(&f.docs["registry"]["types"][0]),"oldSerializedTypeFingerprint":digest(&f.docs["registry"]["types"][0]),"oldRawEqualityProven":false}]}}));
        f.docs.insert("canonical_acceptance".into(),json!({"adjudicationId":"fixture-exact-case","status":"CANONICAL_ACCEPTED_EXACT_PACKET_PENDING_MAIN_FINAL_ACTIVATION","canonicalPacketAcceptance":true}));
        f.docs.insert("main_acceptance".into(),json!({"adjudicationId":"fixture-exact-case","status":"MAIN_ACCEPTED_EXACT_SDK_REPRESENTED_DATA_ONLY","representedDataAccepted":true}));
        f.refresh();
        f
    }
    fn save(&mut self, role: &str, bytes: Vec<u8>) -> EvidencePin {
        let pin = EvidencePin {
            path: format!("{role}.data"),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        fs::write(self.root.join(&pin.path), bytes).unwrap();
        self.pins.insert(role.into(), pin.clone());
        pin
    }
    fn save_doc(&mut self, role: &str) -> EvidencePin {
        self.save(role, serde_json::to_vec(&self.docs[role]).unwrap())
    }
    fn reference(&self, role: &str) -> Value {
        serde_json::to_value(&self.pins[role]).unwrap()
    }
    fn refresh(&mut self) {
        self.pins.clear();
        self.save_doc("registry");
        let h = self.pins["registry"].sha256.clone();
        self.docs.get_mut("identity").unwrap()["registrySha256"] = json!(h);
        self.save_doc("identity");
        self.save_doc("report");
        let tuple: Vec<_> = self
            .sources
            .iter()
            .map(|(n, s)| json!({"name":n,"bytes":s.bytes,"sha256":s.sha256}))
            .collect();
        self.docs.get_mut("report").unwrap()["sourceInputs"] = json!(tuple);
        self.docs.get_mut("report").unwrap()["sourceClosure"] = json!("d".repeat(64));
        self.docs.get_mut("report").unwrap()["contractSha256"] = json!("3".repeat(64));
        self.save_doc("report");
        let files:Vec<_>=[("types-enshrouded.json","registry"),("types-enshrouded.identity.json","identity"),("PHASE-A-REPORT.json","report")].into_iter().map(|(name,role)|json!({"name":name,"bytes":self.pins[role].bytes,"sha256":self.pins[role].sha256})).collect();
        let complete = json!({"qualificationReady":false,"sourceInputs":tuple,"files":files});
        self.docs.insert("complete".into(), complete);
        if self.corrupt_complete {
            self.docs.get_mut("complete").unwrap()["files"][0]["sha256"] = json!("0".repeat(64));
        }
        self.save_doc("complete");
        self.save_doc("matrix");
        let h = digest(&self.docs["ledger"]["records"]);
        let a = self.reference("matrix");
        let b = self.reference("report");
        let l = self.docs.get_mut("ledger").unwrap();
        l["representedReplacementSequenceCanonicalSha256"] = json!(h);
        l["sourceMatrix"] = a;
        l["originalReport"] = b;
        self.save_doc("ledger");
        let candidate = json!({"index":{"bytes":1,"sha256":"1".repeat(64)},"source":{"bytes":1,"sha256":"2".repeat(64)},"contract":{"bytes":1,"sha256":"3".repeat(64)},"artifact":{"bytes":1,"sha256":"4".repeat(64)}});
        self.docs.insert(
            "source_evidence".into(),
            json!({"actualIdentity":self.docs["identity"],"frozenCandidate":candidate}),
        );
        for (role, key) in [
            ("complete", "completeSha256"),
            ("registry", "registrySha256"),
            ("identity", "identitySha256"),
            ("report", "reportSha256"),
        ] {
            self.docs.get_mut("actual_refusal").unwrap()[key] = json!(self.pins[role].sha256);
        }
        for n in [
            "source_evidence",
            "diagnosis_decision",
            "diagnosis_retention",
            "actual_refusal",
        ] {
            self.save_doc(n);
        }
        for n in [
            "consumer_contract",
            "adjudication_report",
            "diagnosis_report",
        ] {
            self.save(
                n,
                b"Fictional portable evidence. No real acceptance.\n".to_vec(),
            );
        }
        let mut files = Vec::new();
        for (name, role) in [
            ("COLLISION-MATRIX.json", "matrix"),
            ("DECISION-BOUNDARY.json", "diagnosis_decision"),
            ("EVIDENCE.json", "source_evidence"),
            ("REPORT.md", "diagnosis_report"),
            ("RETENTION.json", "diagnosis_retention"),
        ] {
            let mut v = self.reference(role);
            v["path"] = json!(name);
            files.push(v);
        }
        self.docs
            .insert("diagnosis_manifest".into(), json!({"files":files}));
        self.save_doc("diagnosis_manifest");
        let assessment = json!({"status":"PASS_COMPLETE_DIAGNOSIS_CONDITIONAL_REPRESENTED_DATA_BOUNDARY","diagnosisManifestSha256":self.pins["diagnosis_manifest"].sha256,"matrixSha256":self.pins["matrix"].sha256,"sourceEvidenceSha256":self.pins["source_evidence"].sha256,"priorActualVerdict":self.reference("actual_refusal")});
        self.docs.insert("diagnosis_assessment".into(), assessment);
        self.save_doc("diagnosis_assessment");
        let mut loss = json!({});
        for (role, key) in [
            ("report", "originalPhaseAReport"),
            ("matrix", "full18Matrix"),
            ("ledger", "lossLedger"),
            ("source_evidence", "fullSourceEvidence"),
            ("diagnosis_manifest", "diagnosisManifest"),
            ("diagnosis_assessment", "canonicalDiagnosisAssessment"),
            ("actual_refusal", "canonicalActualRefusal"),
        ] {
            loss[key] = self.reference(role);
        }
        loss["representedReplacementSequenceCanonicalSha256"] =
            self.docs["ledger"]["representedReplacementSequenceCanonicalSha256"].clone();
        let captured: Vec<_> = ["registry", "identity", "report", "complete"]
            .into_iter()
            .map(|n| self.reference(n))
            .collect();
        let tuple: Vec<_> = self
            .sources
            .iter()
            .map(|(n, s)| json!({"name":n,"bytes":s.bytes,"sha256":s.sha256}))
            .collect();
        let receipt = self.docs.get_mut("receipt").unwrap();
        receipt["mandatoryLossEvidence"] = loss;
        receipt["exactCapturedOutputs"] = json!(captured);
        receipt["newContractScope"]["originalSourceTupleAtCapture"] = json!(tuple);
        self.save_doc("receipt");
        self.save_doc("adjudication_retention");
        let mut files = Vec::new();
        for (name, role) in [
            ("ADJUDICATION-RECEIPT.json", "receipt"),
            ("MANDATORY-LOSS-LEDGER.json", "ledger"),
            ("CONSUMER-ACCEPTANCE-CONTRACT.md", "consumer_contract"),
            ("REPORT.md", "adjudication_report"),
            ("REUSE-RETENTION.json", "adjudication_retention"),
        ] {
            let mut v = self.reference(role);
            v["path"] = json!(name);
            files.push(v);
        }
        self.docs
            .insert("adjudication_manifest".into(), json!({"files":files}));
        self.save_doc("adjudication_manifest");
        let packet = json!({"manifestSha256":self.pins["adjudication_manifest"].sha256,"receiptSha256":self.pins["receipt"].sha256,"lossLedgerSha256":self.pins["ledger"].sha256,"consumerContractSha256":self.pins["consumer_contract"].sha256});
        self.docs.get_mut("canonical_acceptance").unwrap()["reusedEvidence"] = json!([
            self.pins["actual_refusal"].sha256,
            self.pins["diagnosis_assessment"].sha256
        ]);
        self.docs.get_mut("canonical_acceptance").unwrap()["acceptedPacket"] = packet.clone();
        self.save_doc("canonical_acceptance");
        let canonical = self.reference("canonical_acceptance");
        let main = self.docs.get_mut("main_acceptance").unwrap();
        main["canonicalVerdict"] = canonical;
        main["packet"] = packet;
        self.save_doc("main_acceptance");
        let manifest = json!({"schema_version":1,"source_inputs":self.sources,"generation":"fictional-generation","files":self.pins});
        self.docs.insert("manifest".into(), manifest);
        self.input.manifest = self.save_doc("manifest");
        self.save_approval();
    }
    fn save_approval(&mut self) {
        let bytes = fs::read(std::env::current_exe().unwrap()).unwrap();
        let producer = json!({"source_closure":option_env!("HD_PREPARED_SOURCE_CLOSURE").unwrap(),"artifact_sha256":format!("{:x}",Sha256::digest(bytes))});
        self.docs.insert("producer_review".into(),json!({"schemaVersion":1,"status":"CANONICAL_ACCEPTED_COMMITTED_PRODUCER_SOURCE_ARTIFACT","producerCommit":"1".repeat(40),"producer":producer}));
        let review = self.save_doc("producer_review");
        let approval = json!({"schemaVersion":1,"status":"COMMITTED_REVIEWED_PRODUCER_EXTERNAL_REGISTRY_USE","sourceState":"committed","canonicalProducerReviewAccepted":true,"mainProducerUseAccepted":true,"producerCommit":"1".repeat(40),"producer":producer,"canonicalProducerReview":review,"manifestSha256":self.input.manifest.sha256,"mainDataAcceptanceSha256":self.pins["main_acceptance"].sha256});
        self.docs.insert("producer_use_approval".into(), approval);
        self.input.producer_use_approval = self.save_doc("producer_use_approval");
    }
    fn admit(&self) -> anyhow::Result<shroudforge_api::VerifiedExternalRegistry> {
        shroudforge_api::admit_external_registry(&self.root, &self.input, &self.sources)
    }
}
// Keep synthetic evidence for the task-owned retention inventory; no recursive cleanup helper.
#[test]
fn external_registry_public_admission_keeps_caches_absent_and_holds_evidence_readonly() {
    let f = EvidenceFixture::new();
    let admitted = f.admit().unwrap();
    assert!(!f.root.join("shroudforge/cache").exists());
    assert!(
        OpenOptions::new()
            .write(true)
            .open(f.root.join("registry.data"))
            .is_err()
    );
    assert!(
        OpenOptions::new()
            .write(true)
            .open(f.root.join("report.data"))
            .is_err()
    );
    drop(admitted);
    assert!(
        OpenOptions::new()
            .write(true)
            .open(f.root.join("registry.data"))
            .is_ok()
    );
}
#[test]
fn external_registry_missing_changed_extra_or_unbound_inputs_refuse() {
    for mode in [
        "missing",
        "tampered",
        "extra",
        "generation",
        "strict-id",
        "activation",
        "producer",
        "ledger-order",
        "different-namespace",
        "wildcard",
    ] {
        let mut f = EvidenceFixture::new();
        match mode {
            "missing" => {
                fs::remove_file(f.root.join("report.data")).unwrap();
            }
            "tampered" => {
                fs::write(f.root.join("report.data"), b"CHANGED").unwrap();
            }
            "extra" => {
                f.docs.get_mut("manifest").unwrap()["files"]["extra"] =
                    json!({"path":"extra.data","bytes":0,"sha256":"e".repeat(64)});
                f.input.manifest = f.save_doc("manifest");
                f.save_approval();
            }
            "generation" => {
                f.docs.get_mut("identity").unwrap()["pairedKfcGeneration"] = json!("different");
                f.refresh();
            }
            "strict-id" => {
                f.docs.get_mut("identity").unwrap()["reflectionExtractor"] =
                    json!("unapproved-extractor");
                f.refresh();
            }
            "activation" => {
                f.docs.get_mut("main_acceptance").unwrap()["representedDataAccepted"] =
                    json!(false);
                f.refresh();
            }
            "producer" => {
                f.docs.get_mut("producer_use_approval").unwrap()["producer"]["source_closure"] =
                    json!("0".repeat(64));
                f.input.producer_use_approval = f.save_doc("producer_use_approval");
            }
            "ledger-order" => {
                f.docs.get_mut("ledger").unwrap()["records"][0]["replacementSequence"]
                    .as_array_mut()
                    .unwrap()
                    .reverse();
                f.refresh();
            }
            "different-namespace" => {
                f.docs.get_mut("matrix").unwrap()["records"][0]["previous"]["namespace"] =
                    json!(["other"]);
                f.refresh();
            }
            "wildcard" => {
                f.docs.get_mut("receipt").unwrap()["collisionAdmission"]["generalOrWildcardCollisionAcceptance"] =
                    json!(true);
                f.refresh();
            }
            _ => unreachable!(),
        }
        assert!(f.admit().is_err(), "wrongly admitted {mode}");
        assert!(!f.root.join("shroudforge/cache").exists());
    }
}
#[test]
fn external_registry_rejected_canonical_status_cannot_be_recast_as_approval() {
    let mut f = EvidenceFixture::new();
    f.docs.get_mut("canonical_acceptance").unwrap()["status"] = json!("REJECTED");
    f.refresh();
    assert!(
        f.admit().is_err(),
        "canonical status was ignored despite all boolean claims"
    );
}
#[test]
fn external_registry_inconsistent_complete_reference_is_not_complete_evidence() {
    let mut f = EvidenceFixture::new();
    f.corrupt_complete = true;
    f.refresh();
    assert!(f.admit().is_err(), "COMPLETE membership was never checked");
}
#[test]
fn external_registry_prepared_entry_rejects_before_source_or_cache_access() {
    let f = EvidenceFixture::new();
    let env = shroudforge_package::ModEnvironment::load(f.root.to_str().unwrap()).unwrap();
    let mut bad = f.input.clone();
    bad.manifest.path = "../unapproved.json".into();
    let missing = f.root.join("MUST_NOT_ACCESS_SOURCE");
    let output = f.root.join("MUST_NOT_CREATE_STAGE");
    let args = shroudforge_api::RunArgs {
        file_name: "enshrouded".into(),
        options: shroudforge_api::RunOptions {
            current_asset_mods: Some(vec!["fixture".into()]),
            defer_current_commit: true,
            patch: true,
            phase: shroudforge_api::RuntimePhase::Pregame,
            ..Default::default()
        },
    };
    let before = inventory(&f.root).unwrap();
    let err = shroudforge_api::run_prepared_with_external_registry(
        &env, args, &missing, &output, &bad, &f.sources,
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("external registry admission refused"));
    assert_eq!(inventory(&f.root).unwrap(), before);
    assert!(!missing.exists() && !output.exists());
}

#[test]
fn external_registry_duplicate_evidence_role_is_not_silently_overwritten() {
    let mut f = EvidenceFixture::new();
    let original = fs::read_to_string(f.root.join(&f.input.manifest.path)).unwrap();
    let member = format!(
        "\"registry\":{}",
        serde_json::to_string(&f.pins["registry"]).unwrap()
    );
    assert!(original.contains(&member));
    let duplicated = original.replacen(&member, &format!("{member},{member}"), 1);
    f.input.manifest = f.save("manifest", duplicated.into_bytes());
    f.save_approval();
    assert!(
        f.admit().is_err(),
        "duplicate role was silently collapsed by map deserialization"
    );
}
#[test]
fn external_registry_path_escape_and_reparse_are_not_followed() {
    let mut f = EvidenceFixture::new();
    f.input.manifest.path = "../manifest.data".into();
    assert!(f.admit().is_err());
    let outside = f.root.with_extension("outside");
    fs::create_dir(&outside).unwrap();
    let link = f.root.join("linked-evidence");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "synthetic junction creation failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    f.input.manifest.path = "linked-evidence/manifest.data".into();
    assert!(f.admit().is_err());
    fs::remove_dir(link).unwrap(); // remove only the test-created junction, never recursively follow it
}

#[test]
fn external_registry_producer_use_requires_exact_artifact_approval_and_tuple() {
    for mode in [
        "missing-approval",
        "wrong-artifact",
        "wrong-tuple",
        "unknown-schema",
        "extra-ledger",
    ] {
        let mut f = EvidenceFixture::new();
        match mode {
            "missing-approval" => {
                fs::remove_file(f.root.join(&f.input.producer_use_approval.path)).unwrap()
            }
            "wrong-artifact" => {
                f.docs.get_mut("producer_use_approval").unwrap()["producer"]["artifact_sha256"] =
                    json!("0".repeat(64));
                f.input.producer_use_approval = f.save_doc("producer_use_approval");
            }
            "wrong-tuple" => {
                f.sources.get_mut("enshrouded.exe").unwrap().sha256 = "0".repeat(64);
            }
            "unknown-schema" => {
                f.docs.get_mut("manifest").unwrap()["schema_version"] = json!(999);
                f.input.manifest = f.save_doc("manifest");
                f.save_approval();
            }
            "extra-ledger" => {
                let extra = f.docs["ledger"]["records"][0].clone();
                f.docs.get_mut("ledger").unwrap()["records"]
                    .as_array_mut()
                    .unwrap()
                    .push(extra);
                f.refresh();
            }
            _ => unreachable!(),
        }
        assert!(f.admit().is_err(), "admitted {mode}");
    }
}
// A real prepared call on ONE fictional U32 resource; a corrupt conventional cache must be ignored.
#[test]
fn external_registry_prepared_positive_uses_verified_registry_without_cache_fallback() {
    use kfc::{
        container::{KFCFile, KFCReader, KFCWriter},
        guid::{Guid, ResourceId},
        reflection::TypeRegistry,
    };
    let mut f = EvidenceFixture::new();
    let source = f.root.with_extension("source");
    let output = f.root.with_extension("output");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(
        source.join("enshrouded.exe"),
        b"FICTIONAL STATIC BYTES; NOT EXECUTABLE",
    )
    .unwrap();
    fs::write(source.join("enshrouded.kfc"), []).unwrap();
    fs::write(source.join("enshrouded.kfc_resources"), []).unwrap();
    let registry: TypeRegistry = serde_json::from_value(f.docs["registry"].clone()).unwrap();
    let resource = ResourceId::new(
        Guid::parse("00000000-0000-0000-0000-000000000001").unwrap(),
        kfc::hash::fnv("fixture::U32"),
        0,
    );
    let mut writer =
        KFCWriter::<KFCFile, _>::new(&source, "enshrouded", &registry, "fictional-generation")
            .unwrap();
    writer
        .write_resource(&resource, &7u32.to_le_bytes())
        .unwrap();
    writer.finalize().unwrap();
    let package = f.root.join("mods/fixture");
    fs::create_dir_all(package.join("src")).unwrap();
    fs::write(package.join("mod.json"),br#"{"id":"fixture","name":"Fixture","version":"1.0.0","dependencies":[],"capabilities":["patch"]}"#).unwrap();
    fs::write(
        package.join("extended.mod.json"),
        br#"{"schemaVersion":1,"enabled":true,"targets":["client"]}"#,
    )
    .unwrap();
    fs::write(package.join("src/mod.lua"),br#"local r=game.assets.get_resource('00000000-0000-0000-0000-000000000001','fixture::U32'); assert(r.data==7); r.data=29; return {}"#).unwrap();
    let cache = f.root.join("shroudforge/cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(
        cache.join("types-enshrouded.json"),
        b"CORRUPT_CACHE_MUST_REMAIN_UNREAD",
    )
    .unwrap();
    fs::write(
        cache.join("types-enshrouded.identity.json"),
        b"CORRUPT_CACHE_IDENTITY",
    )
    .unwrap();
    for name in [
        "enshrouded.exe",
        "enshrouded.kfc",
        "enshrouded.kfc_resources",
    ] {
        let stamp = stamp(&source.join(name)).unwrap();
        f.sources.insert(
            name.into(),
            SourceStamp {
                bytes: stamp.bytes,
                sha256: stamp.sha256,
            },
        );
    }
    f.docs.get_mut("identity").unwrap()["executableSha256"] =
        json!(f.sources["enshrouded.exe"].sha256);
    f.docs.get_mut("identity").unwrap()["pairedKfcSha256"] =
        json!(f.sources["enshrouded.kfc"].sha256);
    f.refresh();
    let before_source = inventory(&source).unwrap();
    let before_environment = inventory(&f.root).unwrap();
    let mut inputs = before_source.clone();
    inputs.extend(
        before_environment
            .iter()
            .filter(|(n, _)| n.starts_with("mods/"))
            .map(|(n, s)| (n.clone(), s.clone())),
    );
    let request = Request {
        schema: 2,
        target_root: source.clone(),
        asset_input_root: source.clone(),
        environment_root: f.root.clone(),
        output_root: output,
        selected_ids: vec!["fixture".into()],
        inputs,
        environment_files: before_environment.clone(),
        external_registry: Some(f.input.clone()),
    };
    let receipt = prepare(&request).unwrap();
    assert_eq!(receipt.schema, 3);
    assert_eq!(receipt.external_registry, Some(f.input.clone()));
    assert_eq!(inventory(&source).unwrap(), before_source);
    assert_eq!(inventory(&f.root).unwrap(), before_environment);
    let mut reader = KFCReader::new(&receipt.output_stage, "enshrouded")
        .unwrap()
        .into_cursor()
        .unwrap();
    let bytes = reader.read_resource(&resource).unwrap().unwrap();
    assert_eq!(&bytes[..4], &29u32.to_le_bytes());
}

#[test]
fn external_registry_boolean_review_without_bound_producer_review_is_not_approval() {
    let mut f = EvidenceFixture::new();
    f.docs
        .get_mut("producer_use_approval")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("canonicalProducerReview");
    f.input.producer_use_approval = f.save_doc("producer_use_approval");
    assert!(
        f.admit().is_err(),
        "producer review booleans were accepted without a bound review record"
    );
}
