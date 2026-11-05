use crate::{extractor, guard, metadata, schema};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::Path;
pub const ROOT: &str = r"D:\GameServers\staging\ikea-game-update-20261105-r1\official";
pub const OUTPUT: &str = "D:/GameServers/staging/ikea-server-schema-observation-20261105-r1";
pub const PINS: [(&str, u64, &str); 3] = [
    (
        "enshrouded_server.exe",
        22786048,
        "f0d824be7fcd48ebfe2458ff5c28da26060c2074e3ec1fcac587e5a1d508aeb2",
    ),
    (
        "enshrouded_server.kfc",
        4263936,
        "76da3e72911775d34aaf6a3403c173068f14c1b3906be343633db3cf24191ee0",
    ),
    (
        "enshrouded_server.kfc_resources",
        144056320,
        "f54f82d47545abe12ce5178e4302c78c88bcc948942b7d88a8f23d40ec4b0767",
    ),
];
pub fn model(
    exe: &[u8],
    kfc: &[u8],
    namespace: Value,
    baseline: Option<&Value>,
) -> Result<(Value, Value, Value)> {
    crate::diagnostics::set(4);
    let types = extractor::extract_reflection_data(exe)?;
    let unresolved = extractor::unresolved_optional_count();
    let collisions = extractor::attribute_collisions();
    let mut registry: kfc::reflection::TypeRegistry =
        serde_json::from_value(json!({"version":"","types":types}))?;
    crate::diagnostics::set(5);
    let invariants = schema::validate(&registry)?;
    crate::diagnostics::set(6);
    let file = metadata::parse(kfc)?;
    ensure!(!file.game_version().is_empty(), "generation absent");
    registry.version = file.game_version().into();
    let compare = match baseline { Some(value) => schema::compare(&registry, value)?, None => json!({"status":"NO_PRIOR_SERVER_SCHEMA_BASELINE","wholeSchemaEqual":null,"newBaselineCompatibilityProven":false}) };
    let meta = metadata::summary(&file, &registry)?;
    for (i, c) in file.containers().iter().enumerate() {
        let rows = namespace["members"].as_array().context("namespace rows")?;
        let member = rows
            .iter()
            .find(|r| r["index"] == i)
            .context("KFC referenced container missing namespace member")?;
        ensure!(
            member["bytes"] == c.size,
            "KFC container metadata size mismatch"
        );
    }
    let ready = unresolved == 0 && collisions.is_empty();
    let report = json!({"status":if ready{"phase_a_metadata_complete"}else{"unsupported_schema_presence"},"qualificationReady":ready,"extractorId":"sf177-static-reflection-strict-v1","schemaComparisonScope":"all SDK-serialized extracted metadata, including defaults/attributes; pinned SDK rawvariant_type pointer slot remains unconsumed as in parent; no universal rawPE/native compatibility proof","contractSha256":schema::sha(crate::CONTRACT_BYTES),"sourceClosure":option_env!("IKEA_SERVER_SCHEMA_SOURCE_CLOSURE").unwrap_or("UNPINNED"),"unresolvedNonzeroOptionalTypeReferences":unresolved,"referencePresenceIssues":extractor::reference_issues(),"attributeNameCollisionRecords":collisions,"schemaInvariants":invariants,"schemaComparison":compare,"originalKfcMetadata":meta,"DATNamespace":namespace,"providerPayloadsRead":false,"DATContentRead":false,"assetPreparationReady":false,"gameNativeCompatibilityProven":false,"requestReceiptOrStageRebound":false});
    Ok((
        serde_json::to_value(registry)?,
        json!({"schemaVersion":2,"executableSha256":PINS[0].2,"reflectionExtractor":"sf177-static-reflection-strict-v1","parentExtractor":"kfc-a404b15cde40fc7bd43d812bb460ce40d2a9d3b1","pairedKfcSha256":PINS[1].2,"pairedKfcGeneration":file.game_version(),"registryVersionSource":"paired_KFC_not_independent_EXE_version"}),
        report,
    ))
}
pub fn run() -> Result<Value> {
    run_at(Path::new(ROOT), Path::new(OUTPUT), &PINS, None)
}
fn run_at(
    root: &Path,
    parent: &Path,
    pins: &[(&str, u64, &str); 3],
    baseline: Option<&Value>,
) -> Result<Value> {
    ensure!(
        pins.iter().map(|p| p.0).eq(PINS.iter().map(|p| p.0)),
        "fixed3names"
    );
    ensure!(
        pins.iter().map(|p| p.1).sum::<u64>() <= 171106304,
        "fixed3bytes"
    );
    let output_parent = guard::DirectoryGuard::open(parent, true)?;
    ensure!(
        !output_parent
            .path
            .join("current-acquisition-v1")
            .try_exists()?,
        "output exists"
    );
    crate::diagnostics::set(2);
    let source = guard::DirectoryGuard::open(root, false)?;
    let mut reads = crate::read_budget::SourceReads::new(pins.map(|p| p.1));
    let mut held = Vec::new();
    for (slot, (n, b, h)) in pins.iter().enumerate() {
        held.push(guard::Guard::open(&source, n, *b, h, slot, &mut reads)?);
    }
    crate::diagnostics::set(3);
    let before = guard::namespace(&source)?;
    let exe = held[0].bytes(PINS[0].1, &mut reads)?;
    let kfc = held[1].bytes(PINS[1].1, &mut reads)?;
    let (registry, mut identity, mut report) = model(&exe, &kfc, before.clone(), baseline)?;
    identity["executableSha256"] = json!(pins[0].2);
    identity["pairedKfcSha256"] = json!(pins[1].2);
    let registry_raw = serde_json::to_vec(&registry)?;
    let registry_sha = schema::sha(&registry_raw);
    identity["registrySha256"] = json!(registry_sha);
    identity["sourceClosure"] =
        json!(option_env!("IKEA_SERVER_SCHEMA_SOURCE_CLOSURE").unwrap_or("UNPINNED"));
    report["heldSourceObservations"] =
        json!(held.iter().map(|g| g.observation()).collect::<Vec<_>>());
    report["sourceInputs"] = json!(
        pins.iter()
            .map(|(n, b, h)| json!({"name":n,"bytes":b,"sha256":h}))
            .collect::<Vec<_>>()
    );
    crate::diagnostics::set(7);
    for g in &mut held {
        g.verify(&mut reads)?;
    }
    ensure!(
        guard::namespace(&source)? == before,
        "namespace metadata changed"
    );
    source.verify()?;
    report["sourceReadAccounting"] = reads.complete()?;
    report["requestedContentReadBytes"] = report["sourceReadAccounting"]["requestedBytes"].clone();
    report["returnedContentReadBytes"] = report["sourceReadAccounting"]["returnedBytes"].clone();
    crate::diagnostics::set(8);
    let output = guard::output(parent, "current-acquisition-v1")?;
    let identity_raw = serde_json::to_vec(&identity)?;
    let report_raw = serde_json::to_vec(&report)?;
    ensure!(
        registry_raw.len() <= 67108864
            && identity_raw.len() <= 2097152
            && report_raw.len() <= 16777216,
        "output bounds"
    );
    guard::write_new(&output, "types-enshrouded_server.json", &registry_raw, 67108864)?;
    guard::write_new(
        &output,
        "types-enshrouded_server.identity.json",
        &identity_raw,
        2097152,
    )?;
    guard::write_new(&output, "PHASE-A-REPORT.json", &report_raw, 16777216)?;
    output.verify()?;
    let manifest = json!({"status":"complete_outputs_not_compatibility_acceptance","qualificationReady":report["qualificationReady"],"sourceInputs":report["sourceInputs"],"files":[{"name":"types-enshrouded_server.json","bytes":registry_raw.len(),"sha256":registry_sha},{"name":"types-enshrouded_server.identity.json","bytes":identity_raw.len(),"sha256":schema::sha(&identity_raw)},{"name":"PHASE-A-REPORT.json","bytes":report_raw.len(),"sha256":schema::sha(&report_raw)}]});
    let manifest_raw = serde_json::to_vec(&manifest)?;
    ensure!(
        identity_raw.len() + manifest_raw.len() <= 2097152,
        "identity+manifest output cap"
    );
    guard::write_new(&output, "COMPLETE.json", &manifest_raw, 2097152)?;
    ensure!(
        report["qualificationReady"] == true,
        "schema presence unsupported"
    );
    Ok(
        json!({"status":"phase_a_metadata_complete","output":output.path,"providerPayloadsRead":false,"DATContentRead":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn baseline(exe: &[u8]) -> Value {
        let types = extractor::extract_reflection_data(exe).unwrap();
        let r: kfc::reflection::TypeRegistry =
            serde_json::from_value(json!({"version":"","types":types})).unwrap();
        let j = serde_json::to_value(r).unwrap();
        json!({"types":j["types"].as_array().unwrap().iter().map(|v|json!({"sha256":schema::sha(&serde_json::to_vec(v).unwrap())})).collect::<Vec<_>>()})
    }
    #[test]
    fn phase_a_production_model_full_positive_and_optional_unresolved_not_qualified() {
        let mut exe = crate::extractor::tests::fixture();
        let baseline = baseline(&exe);
        let kfc = crate::metadata::fixture();
        let ns = json!({"members":[],"DATContentReads":false});
        let (_, _, proof) = model(&exe, &kfc, ns.clone(), Some(&baseline)).unwrap();
        assert_eq!(proof["qualificationReady"], true);
        assert_eq!(proof["schemaComparison"]["wholeSchemaEqual"], true);
        let (_, _, without_baseline) = model(&exe, &kfc, ns.clone(), None).unwrap();
        assert_eq!(without_baseline["schemaComparison"]["status"], "NO_PRIOR_SERVER_SCHEMA_BASELINE");
        assert!(without_baseline["schemaComparison"]["wholeSchemaEqual"].is_null());
        assert_eq!(without_baseline["assetPreparationReady"], false);
        exe[656..664].copy_from_slice(&(0x140000000u64 + 3000).to_le_bytes());
        let (_, _, proof) = model(&exe, &kfc, ns, Some(&baseline)).unwrap();
        assert_eq!(proof["qualificationReady"], false);
        assert_eq!(proof["unresolvedNonzeroOptionalTypeReferences"], 1);
    }
}

#[cfg(test)]
mod orchestration_tests {
    use super::*;
    use std::fs;
    #[test]
    fn read_a1_same_production_acquisition_creates_bound_complete_outputs_without_dat_read() {
        let base = std::path::PathBuf::from(format!(
            "E:/Build/ikea-server-schema-tests-20261105-r1/synthetic-acquisition-{}",
            std::process::id()
        ));
        fs::create_dir(&base).unwrap();
        let input = base.join("input");
        let output = base.join("output");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&output).unwrap();
        let exe = crate::extractor::tests::fixture();
        let kfc = crate::metadata::fixture();
        let resources = b"SYNTHETIC_RESOURCES_HASH_ONLY";
        let hashes = [schema::sha(&exe), schema::sha(&kfc), schema::sha(resources)];
        let pins = [
            ("enshrouded_server.exe", exe.len() as u64, hashes[0].as_str()),
            ("enshrouded_server.kfc", kfc.len() as u64, hashes[1].as_str()),
            (
                "enshrouded_server.kfc_resources",
                resources.len() as u64,
                hashes[2].as_str(),
            ),
        ];
        fs::write(input.join(pins[0].0), &exe).unwrap();
        fs::write(input.join(pins[1].0), &kfc).unwrap();
        fs::write(input.join(pins[2].0), resources).unwrap();
        fs::write(input.join("enshrouded_server_007.dat"), b"DAT_BODY_MUST_NOT_READ").unwrap();
        let types = extractor::extract_reflection_data(&exe).unwrap();
        let registry: kfc::reflection::TypeRegistry =
            serde_json::from_value(json!({"version":"","types":types})).unwrap();
        let j = serde_json::to_value(registry).unwrap();
        let baseline = json!({"types":j["types"].as_array().unwrap().iter().map(|v|json!({"sha256":schema::sha(&serde_json::to_vec(v).unwrap())})).collect::<Vec<_>>()});
        let result = run_at(&input, &output, &pins, None).unwrap();
        assert_eq!(result["status"], "phase_a_metadata_complete");
        let p = output.join("current-acquisition-v1");
        let report: Value =
            serde_json::from_slice(&fs::read(p.join("PHASE-A-REPORT.json")).unwrap()).unwrap();
        assert_eq!(report["DATContentRead"], false);
        assert_eq!(report["schemaComparison"]["status"], "NO_PRIOR_SERVER_SCHEMA_BASELINE");
        assert!(report["schemaComparison"]["wholeSchemaEqual"].is_null());
        assert!(report["sourceReadAccounting"]["phases"].as_array().unwrap().iter()
            .all(|row| row["file"].as_str().unwrap().starts_with("enshrouded_server.")));
        let total = 2 * pins.iter().map(|p| p.1).sum::<u64>() + pins[0].1 + pins[1].1;
        assert_eq!(report["requestedContentReadBytes"], total);
        assert_eq!(report["returnedContentReadBytes"], total);
        assert_eq!(
            report["sourceReadAccounting"]["requestsDebitedBeforeIO"],
            true
        );
        assert_eq!(report["sourceReadAccounting"]["EOFProbe"], false);
        assert_eq!(report["sourceReadAccounting"]["readCalls"], 8);
        assert_eq!(
            report["DATNamespace"]["members"][0]["contentSha256"],
            "UNOBSERVED"
        );
        assert!(!report.to_string().contains("DAT_BODY_MUST_NOT_READ"));
        let complete: Value =
            serde_json::from_slice(&fs::read(p.join("COMPLETE.json")).unwrap()).unwrap();
        for r in complete["files"].as_array().unwrap() {
            let b = fs::read(p.join(r["name"].as_str().unwrap())).unwrap();
            assert_eq!(schema::sha(&b), r["sha256"].as_str().unwrap());
        }
        assert!(run_at(&input, &output, &pins, Some(&baseline)).is_err());
        let wrong = [
            ("enshrouded_server.exe", exe.len() as u64, "bad"),
            pins[1],
            pins[2],
        ];
        let fresh = base.join("refusal-output");
        fs::create_dir(&fresh).unwrap();
        assert!(run_at(&input, &fresh, &wrong, Some(&baseline)).is_err());
        assert!(!fresh.join("current-acquisition-v1/COMPLETE.json").exists());
        let client_names = [("enshrouded.exe", pins[0].1, pins[0].2), pins[1], pins[2]];
        assert!(run_at(&input, &fresh, &client_names, None).is_err());
    }
}
