use serde_json::json;
use std::path::Path;

pub(crate) const OPERATION_METADATA_JOB_ID: &str = "runtime.operation-metadata";
const REQUEST_NOT_CURRENT: &str = "operation-metadata-request-not-current";

pub(crate) fn is_operation_metadata_job_id(job_id: Option<&str>) -> bool {
    job_id == Some(OPERATION_METADATA_JOB_ID)
}

pub fn request(root: &Path, action: &str) -> Result<(), String> {
    request_job(root, action, None)
}

pub fn request_job(root: &Path, action: &str, job_id: Option<&str>) -> Result<(), String> {
    if !["start", "stop", "snapshot"].contains(&action) {
        return Err("invalid diagnostic action".into());
    }
    let config = shroudforge_loader_config::config::read_loader(root)?;
    let settings = &config["modules"]["runtimeDiagnostics"];
    if action != "stop" && settings["enabled"] == false {
        return Err("Runtime Diagnostics module is disabled in Settings. The diagnostics service is unavailable until it is enabled.".into());
    }
    let job = if action == "stop" {
        None
    } else {
        Some(crate::job_catalog::resolve(root, settings, job_id)?)
    };
    let operation_metadata_job = job
        .as_ref()
        .is_some_and(|job| is_operation_metadata_job_id(Some(job.id.as_str())));
    if action == "start" && operation_metadata_job {
        return Err("Runtime operation metadata supports Snapshot requests only".into());
    }
    shroudforge_loader_config::config::update_loader(root, |value| {
        let settings = &mut value["modules"]["runtimeDiagnostics"];
        settings["sessionActive"] = json!(action != "stop");
        if let Some(job) = job {
            settings["jobId"] = json!(job.id);
            settings["continuous"] = json!(action == "start");
            let request_id = settings["requestId"].as_u64().unwrap_or(0);
            let next_request_id = if operation_metadata_job {
                request_id
                    .checked_add(1)
                    .ok_or_else(|| "diagnostic request id is exhausted".to_owned())?
            } else {
                request_id.saturating_add(1)
            };
            settings["requestId"] = json!(next_request_id);
        }
        Ok(())
    })
}

pub(crate) fn claim_operation_metadata_snapshot(
    root: &Path,
    expected_request_id: u64,
) -> Result<bool, String> {
    set_operation_metadata_request_inactive(root, expected_request_id, true)
}

pub(crate) fn cancel_operation_metadata_request(
    root: &Path,
    expected_request_id: u64,
) -> Result<bool, String> {
    set_operation_metadata_request_inactive(root, expected_request_id, false)
}

fn set_operation_metadata_request_inactive(
    root: &Path,
    expected_request_id: u64,
    snapshot_only: bool,
) -> Result<bool, String> {
    let mut changed = false;
    let result = shroudforge_loader_config::config::update_loader(root, |value| {
        let settings = &mut value["modules"]["runtimeDiagnostics"];
        let current_request_matches = settings["requestId"].as_u64() == Some(expected_request_id);
        let current_job_matches = is_operation_metadata_job_id(settings["jobId"].as_str());
        let current_request_is_active = settings["sessionActive"] == true;
        let current_request_is_snapshot = settings["continuous"] == false;
        let diagnostics_enabled = settings["enabled"] == true;
        if !current_request_matches
            || !current_job_matches
            || !current_request_is_active
            || (snapshot_only && (!current_request_is_snapshot || !diagnostics_enabled))
        {
            return Err(REQUEST_NOT_CURRENT.into());
        }
        settings["sessionActive"] = false.into();
        changed = true;
        Ok(())
    });
    match result {
        Ok(()) => Ok(changed),
        Err(error) if error == REQUEST_NOT_CURRENT => Ok(false),
        Err(error) => Err(error),
    }
}
