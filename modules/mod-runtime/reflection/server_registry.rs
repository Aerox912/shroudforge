//! Exact captured dedicated-server schema. No extraction, cache or client fallback.
//! Original strict refusal remains false; this admits its explicitly reviewed projection.
use super::*;

pub const RECIPE: &str = "3bd7950844b905ddccd64907bc179584c91b22bd2a1b446304d10cd101e4b1c6";
const GENERATION: &str = "1287369|^/game38/branches/release_1.0|2026-11-04T11:17:54.051725Z";
const MANIFEST: &str = "20f9e014ab4ad2f220134a6d59d40537c9620152b537c397b29b38957bc9fecd";
const LEDGER: &str = "d1188659f5c5db0417dceefddcb0ca3bcfc47a4159473d9239314584e2a8870a";
const FILES: &[(&str, u64, &str)] = &[
    ("types-enshrouded_server.json",9196439,"fe74476f286d01fbd677b97abc6472ebb886313a7358ce0149cf1c6670bce4f3"),
    ("types-enshrouded_server.identity.json",635,"fbfbac8bfb9ff36944ac80ad1771273fc18677a2a8588931eaab2f86dce6d03d"),
    ("PHASE-A-REPORT.json",2561137,"98656f3b5f82b46e74dd5b7552c94a946476e1a707dc6cebcaa59325efec62b7"),
    ("COMPLETE.json",892,"bcdabf31eac66e0b85a83e14210ea8d1e90924eb75325a485ec02cc77334fcfb"),
    ("LOSS-LEDGER.json",20891,LEDGER),
    ("INPUTS.json",1690,"b03405fd2f4671b1ac208edbf266830364561b6b2e28c856a3980e59d663ba5a"),
    ("CONSUMER-CONTRACT.md",3999,"c550afa619f743ab4de0bc722981c08a2349b0cf6d608f1b905eb8c74e478f1f"),
    ("CONSUMER-SOURCE.json",1412,"7b72df641588243046c65f14ac3b703f40eee9184ff360233c91c016ea2c9613"),
    ("NEGATIVE-OBSERVATION.json",10637,"9edb4d53447cf44bbdb20124047bea8e7885c99ef2efbc2943df67a49237d631"),
    ("RECIPE.json",40333,RECIPE),
    ("PLAN-CHECK.json",26830,"2cb6fd76e469962c0b1e08d16fd8838e3e99670bdf6a11f481f66a7e003868fb"),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerRegistryInput {
    /// Every evidence file is a flat leaf in a separately staged read-only root.
    pub manifest: EvidencePin,
    pub canonical_review: EvidencePin,
    pub main_acceptance: EvidencePin,
}

fn check_decision(review: &Value, main: &Value, input: &ServerRegistryInput) -> Result<()> {
    ensure!(review["status"] == "CANONICAL_ACCEPTED_IKEA_EXACT_REPRESENTED_DATA_DECISION_ONLY",
        "server data review missing");
    ensure!(main["status"] == "MAIN_ACCEPTED_IKEA_SERVER_REPRESENTED_DATA_ONLY",
        "server data acceptance missing");
    ensure!(review["manifest"]["sha256"] == MANIFEST && review["lossLedger"]["sha256"] == LEDGER
        && review["recipeSha256"] == RECIPE && review["representedDataDecisionAccepted"] == true,
        "server reviewed scope differs");
    ensure!(main["manifestSha256"] == MANIFEST && main["ledgerSha256"] == LEDGER
        && main["recipeSha256"] == RECIPE, "server accepted scope differs");
    ensure!(main["canonicalReviewSha256"] == input.canonical_review.sha256,
        "server acceptance binds another review");
    Ok(())
}

fn validate_registry(docs: &BTreeMap<String, Value>) -> Result<TypeRegistry> {
    let raw = &docs["types-enshrouded_server.json"];
    let report = &docs["PHASE-A-REPORT.json"];
    let identity = &docs["types-enshrouded_server.identity.json"];
    let complete = &docs["COMPLETE.json"];
    let ledger = &docs["LOSS-LEDGER.json"];
    ensure!(raw["version"] == GENERATION && identity["pairedKfcGeneration"] == GENERATION,
        "server generation differs");
    ensure!(report["qualificationReady"] == false && complete["qualificationReady"] == false
        && report["status"] == "unsupported_schema_presence"
        && report["unresolvedNonzeroOptionalTypeReferences"] == 0
        && report["referencePresenceIssues"] == json!([]), "server refusal changed");
    ensure!(report["sourceInputs"] == complete["sourceInputs"]
        && report["sourceInputs"] == ledger["sourceInputs"], "server source tuple differs");
    let registry: TypeRegistry = serde_json::from_value(raw.clone())?;
    // SDK deserialization may not silently ignore an unknown or unsupported field.
    ensure!(serde_json::to_value(&registry)? == *raw, "server SDK roundtrip loses represented data");
    let types = array(raw, "types")?;
    let rows = array(ledger, "records")?;
    let collisions = array(report, "attributeNameCollisionRecords")?;
    ensure!(types.len() == 11184 && rows.len() == 14 && collisions.len() == 14,
        "server represented counts differ");
    let mut owners = BTreeMap::new();
    for (ordinal, (row, collision)) in rows.iter().zip(collisions).enumerate() {
        let index = collision["typeIndex"].as_u64().context("server owner index")? as usize;
        let ty = types.get(index).context("server owner absent")?;
        let map = text(collision, "map")?;
        let before = field(collision, "previous")?;
        let after = field(collision, "current")?;
        let name = text(after, "name")?;
        let expected = json!({"auditRecordOrdinal":ordinal+1,
            "owner":{"typeIndex":index,"qualifiedTypeName":ty["qualifiedName"],"attributeMap":map,"mapKey":name},
            "earlier":before,"later":after,"replacementSequence":[before,after],
            "classification":if before == after {"identical_represented_duplicate"}else{"different_string_value_only"},
            "retainedRegistryWinner":after});
        ensure!(*row == expected, "server loss ledger mismatch");
        let attributes = if map == "type.attributes" { &ty["attributes"] } else {
            let field_name = map.strip_prefix("structFields.").and_then(|s|s.strip_suffix(".attributes"))
                .context("server collision map")?;
            &ty["structFields"][field_name]["attributes"]
        };
        ensure!(attributes.get(name) == Some(after), "server projection winner differs");
        owners.insert(index.to_string(), json!({"qualifiedName":ty["qualifiedName"],"sha256":hash(&canonical(ty)?)}));
    }
    ensure!(owners.len() == 11 && serde_json::to_value(owners)? == ledger["affectedTypeFingerprints"],
        "server affected fingerprints differ");
    ensure!(hash(&canonical(&Value::Array(rows.clone()))?) == ledger["representedReplacementSequenceCanonicalSha256"],
        "server loss sequence differs");
    Ok(registry)
}

/// Data admission only. The caller must separately enforce the exact producer,
/// source tuple, recipe, one-shot operation, root policy and output readback.
pub fn admit_ikea_server_registry(root: &Path, input: &ServerRegistryInput) -> Result<VerifiedExternalRegistry> {
    #[cfg(not(windows))]
    anyhow::bail!("server registry admission requires Windows file leases");
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        ensure!(root.is_absolute() && !root.components().any(|c| matches!(c, Component::CurDir|Component::ParentDir)),
            "explicit server evidence root required");
        let mut b = Bundle {handles:Vec::new(), total:0, paths:BTreeSet::new(), files:BTreeMap::new()};
        for directory in root.ancestors().collect::<Vec<_>>().into_iter().rev() {
            ordinary(directory, true)?;
            let held = OpenOptions::new().access_mode(0x80000080).share_mode(3)
                .custom_flags(0x02200000).open(directory)?;
            use std::os::windows::fs::MetadataExt;
            let metadata = held.metadata()?;
            ensure!(metadata.is_dir() && metadata.file_attributes() & 0x400 == 0, "server evidence directory changed");
            b.handles.push(held);
        }
        for pin in [&input.manifest, &input.canonical_review, &input.main_acceptance] {
            ensure!(!pin.path.contains('/'), "flat server evidence required");
        }
        ensure!(input.manifest.sha256 == MANIFEST, "server contract manifest differs");
        ensure!(input.canonical_review.bytes == 3896 && input.canonical_review.sha256 ==
            "673f8cf8178e6cc35c3bc2ee2e8d478cae119e2119ca695cad1c6039d4fe133c", "exact server data review required");
        let _manifest = parse_json(&b.read(root, &input.manifest)?)?;
        let review = parse_json(&b.read(root, &input.canonical_review)?)?;
        let main = parse_json(&b.read(root, &input.main_acceptance)?)?;
        check_decision(&review, &main, input)?;
        let mut docs = BTreeMap::new();
        for (name, bytes, sha256) in FILES {
            let raw = b.read(root, &EvidencePin {path:(*name).into(), bytes:*bytes, sha256:(*sha256).into()})?;
            if name.ends_with(".json") { docs.insert((*name).into(), parse_json(&raw)?); }
        }
        let registry = validate_registry(&docs)?;
        let provenance = json!({"mode":"exact_ikea_server_represented_data_only", "input":input,
            "recipeSha256":RECIPE,"generation":GENERATION,"sourceInputs":docs["PHASE-A-REPORT.json"]["sourceInputs"],
            "recipeFiles":docs["RECIPE.json"]["effectiveFiles"],"plannedIds":docs["PLAN-CHECK.json"]["plannedIds"],
            "datNamespace":docs["PHASE-A-REPORT.json"]["DATNamespace"],
            "originalQualificationReady":false,"originalExit":2,"lossLedgerSha256":LEDGER});
        Ok(VerifiedExternalRegistry {registry:std::rc::Rc::new(registry),_leases:b,provenance})
    }
}
