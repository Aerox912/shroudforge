use crate::Session;
use serde_json::json;
use std::time::Instant;

impl Session {
    pub(super) fn stop_diagnostic_job(&mut self) {
        self.stop_job_execution("stopped");
        if self.is_operation_metadata_job() {
            cancel_operation_metadata_request(self);
        } else {
            self.sample();
        }
        self.active = false;
        self.reason = "stopped";
        self.next_sample = Instant::now();
        tracing::info!(target:"shroudforge::diagnostics","Diagnostic session stopped");
    }

    pub(super) fn complete_diagnostic_job(&mut self) {
        let reason = if self.settings["continuous"] == true
            || self.job.as_ref().is_some_and(|job| job.execution.is_some())
        {
            "duration-complete"
        } else {
            "snapshot-complete"
        };
        self.finish_diagnostic_job(reason);
    }

    pub(crate) fn finish_diagnostic_job(&mut self, reason: &'static str) {
        self.stop_job_execution(reason);
        let operation_metadata_job = self.is_operation_metadata_job();
        if operation_metadata_job {
            cancel_operation_metadata_request(self);
        } else {
            self.sample();
        }
        self.active = false;
        self.reason = reason;
        self.next_sample = Instant::now();
        tracing::info!(target:"shroudforge::diagnostics",reason=self.reason,"Diagnostic session completed");
        // Keep the same persisted source of truth; do not overwrite a newer UI request.
        let request = self.settings["requestId"].clone();
        if let Err(error) = shroudforge_loader_config::config::update_loader(&self.root, |value| {
            let settings = &mut value["modules"]["runtimeDiagnostics"];
            let request_matches = settings.get("requestId").cloned().unwrap_or(json!(0)) == request;
            let job_matches = !operation_metadata_job
                || crate::request_diagnostic_action::is_operation_metadata_job_id(
                    settings["jobId"].as_str(),
                );
            if request_matches && job_matches {
                settings["sessionActive"] = false.into();
            }
            Ok(())
        }) {
            tracing::error!(target:"shroudforge::diagnostics","Could not persist completed session: {error}");
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop_job_execution("process-stopped");
        if self.active {
            if self.is_operation_metadata_job() {
                cancel_operation_metadata_request(self);
            } else {
                self.sample();
            }
        }
        if self.job.is_none() {
            cancel_unobserved_operation_metadata_request(
                &self.root,
                self.settings["requestId"].as_u64(),
            );
        }
        self.active = false;
        self.reason = "process-stopped";
        self.publish();
    }
}

fn cancel_operation_metadata_request(session: &Session) {
    let Some(request_id) = session.settings["requestId"].as_u64() else {
        return;
    };
    if let Err(error) = crate::request_diagnostic_action::cancel_operation_metadata_request(
        &session.root,
        request_id,
    ) {
        tracing::warn!(
            target:"shroudforge::diagnostics",
            %error,
            "Could not cancel the operation metadata request"
        );
    }
}

fn cancel_unobserved_operation_metadata_request(
    root: &std::path::Path,
    observed_request_id: Option<u64>,
) {
    let config = match shroudforge_loader_config::config::read_loader(root) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(
                target:"shroudforge::diagnostics",
                %error,
                "Could not inspect an unobserved operation metadata request during shutdown"
            );
            return;
        }
    };
    let settings = &config["modules"]["runtimeDiagnostics"];
    if !crate::request_diagnostic_action::is_operation_metadata_job_id(settings["jobId"].as_str())
        || settings["sessionActive"] != true
    {
        return;
    }
    let Some(request_id) = settings["requestId"].as_u64() else {
        return;
    };
    if observed_request_id == Some(request_id) {
        return;
    }
    if let Err(error) =
        crate::request_diagnostic_action::cancel_operation_metadata_request(root, request_id)
    {
        tracing::warn!(
            target:"shroudforge::diagnostics",
            %error,
            "Could not cancel an unobserved operation metadata request during shutdown"
        );
    }
}

impl Session {
    fn is_operation_metadata_job(&self) -> bool {
        crate::request_diagnostic_action::is_operation_metadata_job_id(
            self.job.as_ref().map(|job| job.id.as_str()),
        )
    }
}
