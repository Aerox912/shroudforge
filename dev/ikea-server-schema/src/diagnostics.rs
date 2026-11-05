use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
static STAGE: AtomicU8 = AtomicU8::new(0);
static READ_OBSERVED: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicU64 = AtomicU64::new(0);
static RETURNED: AtomicU64 = AtomicU64::new(0);
static CALLS: AtomicU64 = AtomicU64::new(0);
static TERMINAL: AtomicBool = AtomicBool::new(false);
pub(crate) fn source_read_counts(requested: u64, returned: u64, calls: u64, terminal: bool) {
    REQUESTED.store(requested, Ordering::Relaxed);
    RETURNED.store(returned, Ordering::Relaxed);
    CALLS.store(calls, Ordering::Relaxed);
    TERMINAL.store(terminal, Ordering::Relaxed);
    READ_OBSERVED.store(true, Ordering::Relaxed);
}
pub fn set(n: u8) {
    STAGE.store(n, Ordering::Relaxed)
}
pub fn refusal() -> serde_json::Value {
    let stages = [
        "arguments",
        "containment",
        "source_guards",
        "namespace_metadata",
        "static_extraction",
        "schema_invariants",
        "kfc_metadata",
        "guard_recheck",
        "output_write",
    ];
    let reads = if READ_OBSERVED.load(Ordering::Relaxed) {
        serde_json::json!({"requestedBytes":REQUESTED.load(Ordering::Relaxed),"returnedBytes":RETURNED.load(Ordering::Relaxed),"readCalls":CALLS.load(Ordering::Relaxed),"terminal":TERMINAL.load(Ordering::Relaxed),"measurement":"application requests/returned counts; error internal transfer UNKNOWN"})
    } else {
        serde_json::json!("UNKNOWN")
    };
    serde_json::json!({"sourceReadAccounting":reads,"status":"refused","kind":"phase_a_contract_or_budget","stage":stages.get(STAGE.load(Ordering::Relaxed)as usize).unwrap_or(&"UNKNOWN"),"sourceProgress":"UNKNOWN","rawErrorIncluded":false})
}
