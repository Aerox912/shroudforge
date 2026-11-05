//! Pure consumer of the genuine effective-source envelope. No file IO or approval.
use super::{ReadLimits, SourceFinalization, SourcePlan, Stamp, digest, valid_digest};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, BTreeSet};

// Field order is part of the producer's canonical JSON digest contract.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveRequest<B> {
    pub schema: u32,
    pub kind: String,
    pub base: B,
    pub sources: SourcePlan,
    pub registry_evidence_files: BTreeMap<String, Stamp>,
    pub source_read_limits: ReadLimits,
    pub expected_load_order: Vec<String>,
    pub expected_generation: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveReceipt<S> {
    pub schema: u32,
    pub kind: String,
    pub status: String,
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
    pub stage: S,
    pub source_finalization: SourceFinalization,
    pub independent_postimage_accepted: bool,
    pub current_native_accepted: bool,
    pub installation_approved: bool,
}

pub struct EffectivePair<B, S> {
    pub request: EffectiveRequest<B>,
    pub receipt: EffectiveReceipt<S>,
    pub request_canonical_sha256: String,
    pub receipt_canonical_sha256: String,
    pub base_request_canonical_sha256: String,
}

impl<B, S> EffectivePair<B, S> {
    /// The build closure is distinct from the outer source-closure evidence hash.
    pub fn validate_stage_binding(
        &self,
        producer: &str,
        stage_source_closure: &str,
        stage_request_sha256: &str,
    ) -> Result<()> {
        ensure!(valid_digest(producer) && stage_source_closure == producer,
            "effective nested producer differs");
        ensure!(stage_request_sha256 == self.base_request_canonical_sha256,
            "effective nested base request digest differs");
        Ok(())
    }
}

pub fn parse_effective<B: DeserializeOwned + Serialize, S: DeserializeOwned + Serialize>(
    request_json: &str,
    receipt_json: &str,
) -> Result<EffectivePair<B, S>> {
    let request: EffectiveRequest<B> = serde_json::from_str(request_json)?;
    let receipt: EffectiveReceipt<S> = serde_json::from_str(receipt_json)?;
    ensure!(request.schema == 3 && request.kind == "sf178_prepared_effective_source_request_v1"
        && receipt.schema == 4 && receipt.kind == "sf178_prepared_effective_source_receipt_v1",
        "effective envelope schema/kind differs");
    ensure!(receipt.status == "SOURCE_STAGE_ONLY_PENDING_INDEPENDENT_ACCEPTANCE"
        && !receipt.independent_postimage_accepted && !receipt.current_native_accepted
        && !receipt.installation_approved, "effective receipt claimed later acceptance");
    let request_canonical_sha256 = digest(&request)?;
    ensure!(receipt.request_sha256 == request_canonical_sha256,
        "effective outer request digest differs");
    for hash in [&receipt.source_closure_sha256, &receipt.artifact_sha256,
        &receipt.operation_verdict_sha256, &receipt.base_candidate_sha256,
        &receipt.successor_sha256] {
        ensure!(valid_digest(hash), "effective provenance digest invalid");
    }
    request.sources.validate()?;
    ensure!(receipt.raw_inventory_sha256 == request.sources.raw_inventory_sha256
        && receipt.effective_inventory_sha256 == request.sources.effective_inventory_sha256,
        "effective inventory provenance differs");
    let ids: BTreeSet<_> = request.expected_load_order.iter().collect();
    ensure!(!ids.is_empty() && ids.len() == request.expected_load_order.len()
        && receipt.planned_ids == request.expected_load_order
        && ids.iter().all(|id| shroudforge_package::config::valid_id(id)
            && !id.to_ascii_lowercase().replace(['-', '_', '.'], "").contains("autoloot")),
        "effective planned order differs");
    let generation = &request.expected_generation;
    ensure!(!generation.is_empty() && generation.len() <= 128 && generation.is_ascii()
        && !generation.contains('|') && !generation.chars().any(char::is_control)
        && &receipt.generation == generation, "effective generation differs");
    ensure!(!request.registry_evidence_files.is_empty() && request.registry_evidence_files.len() <= 64,
        "effective registry evidence bound");
    for (name, stamp) in &request.registry_evidence_files {
        ensure!(!name.starts_with("mods/") && name != "shroudforge/config/modloader-config.json"
            && request.sources.raw_files.get(name) == Some(stamp),
            "effective registry evidence/raw control differs");
    }
    ensure!(request.sources.raw_files.keys().all(|name| name.starts_with("mods/")
        || name == "shroudforge/config/modloader-config.json"
        || request.registry_evidence_files.contains_key(name)), "unrelated effective source control");
    let finalization = &receipt.source_finalization;
    let expected_names: Vec<_> = request.sources.raw_files.keys().cloned()
        .chain(request.sources.replacements.iter().map(|r| format!("override:{}", r.logical_path)))
        .collect();
    ensure!(finalization.verified && finalization.directories_matched
        && finalization.membership_matched == Some(true) && finalization.phase_error.is_none()
        && finalization.metadata_errors.is_empty() && !finalization.terminal_io
        && finalization.files.iter().map(|row| &row.name).eq(expected_names.iter())
        && finalization.files.iter().all(|row| row.identity_matched == Some(true)
            && row.hash_matched == Some(true) && row.error.is_none()),
        "effective source finalization incomplete");
    let limits = request.source_read_limits;
    let final_bytes = request.sources.required_final_bytes()?;
    let final_calls = request.sources.required_final_calls()?;
    ensure!(limits.final_reserve_bytes >= final_bytes && limits.final_reserve_calls >= final_calls
        && limits.max_bytes >= limits.final_reserve_bytes && limits.max_calls >= limits.final_reserve_calls
        && finalization.charged_bytes == finalization.returned_bytes
        && finalization.charged_bytes >= final_bytes && finalization.calls >= final_calls
        && finalization.charged_bytes <= limits.max_bytes && finalization.calls <= limits.max_calls,
        "effective finalization IO accounting differs");
    let base_request_canonical_sha256 = digest(&request.base)?;
    let receipt_canonical_sha256 = digest(&receipt)?;
    Ok(EffectivePair { request, receipt, request_canonical_sha256,
        receipt_canonical_sha256, base_request_canonical_sha256 })
}
