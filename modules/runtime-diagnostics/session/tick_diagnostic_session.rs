use crate::Session;
use std::time::{Duration, Instant};

impl Session {
    pub fn tick(&mut self) {
        self.tick_with_host(&mut crate::run_diagnostic_job::UnavailableHost);
    }

    pub fn tick_with_host(&mut self, host: &mut dyn crate::DiagnosticHost) {
        self.refresh();
        crate::collectors::operation_timings::set_enabled(self.operation_timings_enabled());
        let now = Instant::now();
        let was_active = self.active;
        if self.active && self.started.elapsed().as_secs() < self.limit() {
            match self.advance_job_execution(host) {
                Ok(true) => self.finish_diagnostic_job("job-complete"),
                Err(error) => {
                    tracing::warn!(target:"shroudforge::diagnostics", %error, "Diagnostic job execution failed");
                    self.stop_job_execution("job-failed");
                    if !self.job_report.is_object() {
                        self.job_report = serde_json::json!({});
                    }
                    self.job_report["hostError"] = serde_json::json!(error);
                    self.finish_diagnostic_job("job-failed");
                }
                Ok(false) => {}
            }
        }
        if was_active && !self.active && matches!(self.reason, "job-complete" | "job-failed") {
            crate::collectors::operation_timings::set_enabled(self.operation_timings_enabled());
            self.publish();
            return;
        }
        if now < self.next_sample
            && !(self.active && self.started.elapsed().as_secs() >= self.limit())
        {
            return;
        }
        self.next_sample = now
            + Duration::from_millis(
                self.settings["intervalMilliseconds"]
                    .as_u64()
                    .unwrap_or(2000)
                    .clamp(500, 60000),
            );
        if self.active {
            let operation_metadata_job =
                crate::request_diagnostic_action::is_operation_metadata_job_id(
                    self.job.as_ref().map(|job| job.id.as_str()),
                );
            let may_sample = if operation_metadata_job {
                match self.settings["requestId"].as_u64() {
                    Some(request_id) => {
                        match crate::request_diagnostic_action::claim_operation_metadata_snapshot(
                            &self.root, request_id,
                        ) {
                            Ok(true) => true,
                            Ok(false) => {
                                if let Err(error) = crate::request_diagnostic_action::cancel_operation_metadata_request(
                                    &self.root,
                                    request_id,
                                ) {
                                    tracing::warn!(
                                        target:"shroudforge::diagnostics",
                                        %error,
                                        "Could not cancel the unclaimed operation metadata Snapshot"
                                    );
                                }
                                false
                            }
                            Err(error) => {
                                tracing::warn!(
                                    target:"shroudforge::diagnostics",
                                    %error,
                                    "Could not claim the operation metadata Snapshot"
                                );
                                false
                            }
                        }
                    }
                    None => false,
                }
            } else {
                true
            };
            if may_sample {
                self.sample();
                if self.started.elapsed().as_secs() >= self.limit()
                    || (self.settings["continuous"] != true
                        && self.job.as_ref().is_none_or(|job| job.execution.is_none()))
                {
                    self.complete_diagnostic_job();
                }
            } else if operation_metadata_job {
                self.active = false;
                self.reason = "stopped";
            }
        }
        crate::collectors::operation_timings::set_enabled(self.operation_timings_enabled());
        self.publish();
    }
}
